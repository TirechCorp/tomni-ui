//! Um servidor FTP de teste, em processo, que imita o vsftpd, o proftpd e o pure-ftpd.
//!
//! Sobe em `127.0.0.1:0`, uma thread por cliente, sobre uma pasta de verdade (`/` é a pasta).
//! Aceita o usuário "ana" com a senha "senha", o anônimo e o usuário "conta" (que pede o
//! `ACCT` "projeto"). Fala FTP simples, FTPS explícito (`AUTH TLS`) e implícito, com o TLS do
//! rustls e um certificado do rcgen, e confere a retomada da sessão TLS nas conexões de dados
//! como o vsftpd (`require_ssl_reuse`) e o proftpd fazem. As injeções (`Opcoes`) imitam o que
//! os servidores de verdade fazem de errado: `421`, IP privado no `227`, `EPSV` recusado,
//! nomes em Latin-1, só TLS 1.2, recusa de retomada no TLS 1.3.

#![allow(dead_code)]

use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::net::{IpAddr, Ipv4Addr, Shutdown, SocketAddr, TcpListener, TcpStream};
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use rustls::crypto::ring;
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use rustls::server::NoServerSessionStorage;
use rustls::{HandshakeKind, ServerConfig, ServerConnection, StreamOwned, version};

/// O que o servidor imita.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Perfil {
    Vsftpd,
    Proftpd,
    PureFtpd,
}

/// Como o TLS é oferecido.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tls {
    Nenhum,
    Explicito,
    Implicito,
}

/// As versões do TLS que o servidor fala.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Versoes {
    Todas,
    So12,
    So13,
}

#[derive(Clone)]
pub struct Opcoes {
    pub perfil: Perfil,
    pub tls: Tls,
    pub versoes: Versoes,
    /// Recusa a conexão de dados que não retoma a sessão (o padrão do vsftpd e do proftpd).
    pub exige_retomada: bool,
    /// Não dá tíquetes no TLS 1.3 (não há como retomar ali; o 1.2 retoma).
    pub sem_tiquetes_13: bool,
    pub epsv_recusado: bool,
    pub eprt_recusado: bool,
    /// O IP que o `227` anuncia no lugar do real.
    pub ip_pasv: Option<Ipv4Addr>,
    /// Nomes em Latin-1 e sem `UTF8` no `FEAT`.
    pub latin1: bool,
    /// Quantas conexões de controle ao mesmo tempo (a que passa de `421`).
    pub max_conexoes: Option<usize>,
    /// Devolve os dados devagar (para dar tempo de cancelar).
    pub lento: bool,
    /// Usa outro certificado nas conexões de dados.
    pub cert_dados_diferente: bool,
    /// Minutos que o relógio do servidor está a leste do UTC (só o `LIST` usa).
    pub fuso_min: i32,
    /// O servidor não conhece o `REST STREAM` (o envio retomado vai por `APPE`).
    pub sem_rest_stream: bool,
}

impl Opcoes {
    pub fn novo(perfil: Perfil) -> Opcoes {
        Opcoes {
            perfil,
            tls: Tls::Nenhum,
            versoes: Versoes::Todas,
            exige_retomada: perfil != Perfil::PureFtpd,
            sem_tiquetes_13: false,
            epsv_recusado: false,
            eprt_recusado: false,
            ip_pasv: None,
            latin1: false,
            max_conexoes: None,
            lento: false,
            cert_dados_diferente: false,
            fuso_min: 0,
            sem_rest_stream: false,
        }
    }

    pub fn vsftpd() -> Opcoes {
        Opcoes::novo(Perfil::Vsftpd)
    }

    pub fn proftpd() -> Opcoes {
        Opcoes::novo(Perfil::Proftpd)
    }

    pub fn pureftpd() -> Opcoes {
        Opcoes::novo(Perfil::PureFtpd)
    }

    pub fn tls(mut self, tls: Tls) -> Opcoes {
        self.tls = tls;
        self
    }

    pub fn versoes(mut self, versoes: Versoes) -> Opcoes {
        self.versoes = versoes;
        self
    }
}

/// O que o servidor e os testes dividem.
#[derive(Default)]
pub struct Estado {
    /// Os comandos recebidos, em ordem (a senha vira `***`).
    pub comandos: Mutex<Vec<String>>,
    /// Conexões de controle aceitas.
    pub controles: AtomicUsize,
    /// Conexões de controle abertas agora.
    pub abertas: AtomicUsize,
    /// Conexões de dados com TLS que retomaram a sessão.
    pub dados_retomados: AtomicUsize,
    /// Conexões de dados com TLS que **não** retomaram.
    pub dados_completos: AtomicUsize,
    /// Conexões de dados sem TLS.
    pub dados_claros: AtomicUsize,
    /// Dados recusados por falta de retomada.
    pub recusados: AtomicUsize,
    /// O próximo comando recebe `421` e a conexão cai.
    derrubar_proximo: AtomicBool,
    /// As versões do TLS negociadas (`"1.2"` ou `"1.3"`), uma por conexão com TLS.
    pub versoes: Mutex<Vec<String>>,
    /// As conexões abertas, para derrubar.
    conexoes: Mutex<Vec<TcpStream>>,
}

impl Estado {
    pub fn comandos(&self) -> Vec<String> {
        self.comandos.lock().unwrap().clone()
    }

    /// Quantas vezes o comando (a primeira palavra) foi recebido.
    pub fn contar(&self, comando: &str) -> usize {
        self.comandos
            .lock()
            .unwrap()
            .iter()
            .filter(|linha| {
                linha
                    .split(' ')
                    .next()
                    .is_some_and(|c| c.eq_ignore_ascii_case(comando))
            })
            .count()
    }

    pub fn limpar(&self) {
        self.comandos.lock().unwrap().clear();
    }
}

pub struct Ftpd {
    pub porta: u16,
    pub raiz: PathBuf,
    pub estado: Arc<Estado>,
    parar: Arc<AtomicBool>,
}

/// Os certificados (o do controle e outro, para a injeção) e a chave, uma vez por processo.
fn certificado(indice: usize) -> &'static (Vec<CertificateDer<'static>>, Vec<u8>) {
    static CERTS: OnceLock<Vec<(Vec<CertificateDer<'static>>, Vec<u8>)>> = OnceLock::new();
    &CERTS.get_or_init(|| {
        (0..2)
            .map(|_| {
                let certificado = rcgen::generate_simple_self_signed(vec![
                    "localhost".to_owned(),
                    "127.0.0.1".to_owned(),
                ])
                .unwrap();
                (
                    vec![certificado.cert.der().clone()],
                    certificado.signing_key.serialize_der(),
                )
            })
            .collect()
    })[indice]
}

/// A impressão SHA-256 (`AB:CD:…`) do certificado do servidor.
pub fn impressao_do_certificado() -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(&certificado(0).0[0])
        .iter()
        .map(|b| format!("{b:02X}"))
        .collect::<Vec<_>>()
        .join(":")
}

fn config_tls(opcoes: &Opcoes, indice: usize) -> Arc<ServerConfig> {
    let (cadeia, chave) = certificado(indice);
    let provider = Arc::new(ring::default_provider());
    let versoes: &[&'static rustls::SupportedProtocolVersion] = match opcoes.versoes {
        Versoes::Todas => &[&version::TLS13, &version::TLS12],
        Versoes::So12 => &[&version::TLS12],
        Versoes::So13 => &[&version::TLS13],
    };
    let mut config = ServerConfig::builder_with_provider(provider)
        .with_protocol_versions(versoes)
        .unwrap()
        .with_no_client_auth()
        .with_single_cert(
            cadeia.clone(),
            PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(chave.clone())),
        )
        .unwrap();
    if opcoes.sem_tiquetes_13 {
        // Sem armazenamento e sem tíquetes no 1.3; o 1.2 retoma pelo número da sessão.
        config.send_tls13_tickets = 0;
    } else {
        config.ticketer = ring::Ticketer::new().unwrap();
    }
    let _ = NoServerSessionStorage {};
    Arc::new(config)
}

impl Ftpd {
    /// Sobe o servidor com a raiz em `pasta/raiz`.
    pub fn novo(pasta: &Path, opcoes: Opcoes) -> Ftpd {
        let raiz = pasta.join("raiz");
        fs::create_dir_all(&raiz).unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let porta = listener.local_addr().unwrap().port();
        let estado = Arc::new(Estado::default());
        let parar = Arc::new(AtomicBool::new(false));
        let tls_controle = (opcoes.tls != Tls::Nenhum).then(|| config_tls(&opcoes, 0));
        let tls_dados = match (&tls_controle, opcoes.cert_dados_diferente) {
            (None, _) => None,
            // O mesmo `ServerConfig` (e a mesma chave dos tíquetes) é o que deixa retomar.
            (Some(config), false) => Some(config.clone()),
            (Some(_), true) => Some(config_tls(&opcoes, 1)),
        };
        let comum = Arc::new(Comum {
            opcoes,
            raiz: raiz.clone(),
            estado: estado.clone(),
            tls_controle,
            tls_dados,
        });
        {
            let parar = parar.clone();
            thread::spawn(move || {
                for conexao in listener.incoming() {
                    if parar.load(Ordering::SeqCst) {
                        break;
                    }
                    let Ok(conexao) = conexao else { continue };
                    let comum = comum.clone();
                    thread::spawn(move || {
                        let _ = atender(comum, conexao);
                    });
                }
            });
        }
        Ftpd {
            porta,
            raiz,
            estado,
            parar,
        }
    }

    /// O próximo comando de qualquer cliente recebe `421` e a conexão cai.
    pub fn derrubar_proximo(&self) {
        self.estado.derrubar_proximo.store(true, Ordering::SeqCst);
    }

    /// Derruba todas as conexões abertas.
    pub fn derrubar_todas(&self) {
        for conexao in self.estado.conexoes.lock().unwrap().drain(..) {
            let _ = conexao.shutdown(Shutdown::Both);
        }
    }

    pub fn escrever(&self, nome: &str, dados: &[u8]) -> PathBuf {
        let caminho = self.raiz.join(nome.trim_start_matches('/'));
        fs::create_dir_all(caminho.parent().unwrap()).unwrap();
        fs::write(&caminho, dados).unwrap();
        caminho
    }

    pub fn ler(&self, nome: &str) -> Vec<u8> {
        fs::read(self.raiz.join(nome.trim_start_matches('/'))).unwrap()
    }
}

impl Drop for Ftpd {
    fn drop(&mut self) {
        self.parar.store(true, Ordering::SeqCst);
        let _ = TcpStream::connect(("127.0.0.1", self.porta));
        self.derrubar_todas();
    }
}

struct Comum {
    opcoes: Opcoes,
    raiz: PathBuf,
    estado: Arc<Estado>,
    tls_controle: Option<Arc<ServerConfig>>,
    tls_dados: Option<Arc<ServerConfig>>,
}

/// Uma conexão, em claro ou com TLS.
enum Conn {
    Plain(TcpStream),
    Tls(Box<StreamOwned<ServerConnection, TcpStream>>),
    Vazio,
}

impl Conn {
    fn tcp(&self) -> Option<&TcpStream> {
        match self {
            Conn::Plain(t) => Some(t),
            Conn::Tls(t) => Some(&t.sock),
            Conn::Vazio => None,
        }
    }

    /// Faz o handshake TLS (servidor) e solta os tíquetes.
    fn tls(tcp: TcpStream, config: Arc<ServerConfig>) -> io::Result<Conn> {
        let mut tcp = tcp;
        let mut conn = ServerConnection::new(config).map_err(io::Error::other)?;
        while conn.is_handshaking() {
            conn.complete_io(&mut tcp)?;
        }
        while conn.wants_write() {
            conn.write_tls(&mut tcp)?;
        }
        Ok(Conn::Tls(Box::new(StreamOwned::new(conn, tcp))))
    }

    fn retomou(&self) -> bool {
        match self {
            Conn::Tls(t) => t.conn.handshake_kind() == Some(HandshakeKind::Resumed),
            _ => false,
        }
    }

    fn versao(&self) -> Option<&'static str> {
        match self {
            Conn::Tls(t) => t.conn.protocol_version().map(|v| match v {
                rustls::ProtocolVersion::TLSv1_3 => "1.3",
                _ => "1.2",
            }),
            _ => None,
        }
    }

    fn fechar(self) {
        match self {
            Conn::Tls(mut t) => {
                t.conn.send_close_notify();
                let _ = t.flush();
                let _ = t.sock.shutdown(Shutdown::Write);
                let _ = t.sock.set_read_timeout(Some(Duration::from_millis(500)));
                let mut resto = [0u8; 256];
                while matches!(t.read(&mut resto), Ok(n) if n > 0) {}
            }
            Conn::Plain(t) => {
                let _ = t.shutdown(Shutdown::Both);
            }
            Conn::Vazio => {}
        }
    }
}

impl Read for Conn {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        match self {
            Conn::Plain(t) => t.read(buf),
            Conn::Tls(t) => match t.read(buf) {
                Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => Ok(0),
                outro => outro,
            },
            Conn::Vazio => Ok(0),
        }
    }
}

impl Write for Conn {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        match self {
            Conn::Plain(t) => t.write(buf),
            Conn::Tls(t) => t.write(buf),
            Conn::Vazio => Err(io::ErrorKind::BrokenPipe.into()),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        match self {
            Conn::Plain(t) => t.flush(),
            Conn::Tls(t) => t.flush(),
            Conn::Vazio => Ok(()),
        }
    }
}

struct Sessao {
    comum: Arc<Comum>,
    ctrl: Conn,
    entrada: Vec<u8>,
    par: SocketAddr,
    usuario: Option<String>,
    autenticado: bool,
    cwd: String,
    protegido: bool,
    rest: u64,
    pasv: Option<TcpListener>,
    ativo: Option<SocketAddr>,
    renomear: Option<String>,
    /// Um `426` já foi mandado e o próximo `ABOR` responde `226`.
    abortou: bool,
}

fn atender(comum: Arc<Comum>, tcp: TcpStream) -> io::Result<()> {
    let estado = comum.estado.clone();
    if let Ok(copia) = tcp.try_clone() {
        estado.conexoes.lock().unwrap().push(copia);
    }
    estado.controles.fetch_add(1, Ordering::SeqCst);
    let abertas = estado.abertas.fetch_add(1, Ordering::SeqCst) + 1;
    let resultado = sessao(comum.clone(), tcp, abertas);
    estado.abertas.fetch_sub(1, Ordering::SeqCst);
    resultado
}

fn sessao(comum: Arc<Comum>, tcp: TcpStream, abertas: usize) -> io::Result<()> {
    let _ = tcp.set_nodelay(true);
    tcp.set_read_timeout(Some(Duration::from_secs(60)))?;
    let par = tcp.peer_addr()?;
    let ctrl = if comum.opcoes.tls == Tls::Implicito {
        let config = comum.tls_controle.clone().unwrap();
        match Conn::tls(tcp, config) {
            Ok(conn) => conn,
            Err(_) => return Ok(()),
        }
    } else {
        Conn::Plain(tcp)
    };
    if let Some(versao) = ctrl.versao() {
        comum.estado.versoes.lock().unwrap().push(versao.to_owned());
    }
    let mut s = Sessao {
        comum: comum.clone(),
        ctrl,
        entrada: Vec::new(),
        par,
        usuario: None,
        autenticado: false,
        cwd: "/".to_owned(),
        protegido: false,
        rest: 0,
        pasv: None,
        ativo: None,
        renomear: None,
        abortou: false,
    };
    if comum.opcoes.max_conexoes.is_some_and(|max| abertas > max) {
        s.resp(421, "Too many connections from your IP address.")?;
        return Ok(());
    }
    let saudacao = match comum.opcoes.perfil {
        Perfil::Vsftpd => "(vsFTPd 3.0.5)",
        Perfil::Proftpd => "ProFTPD Server (teste) [127.0.0.1]",
        Perfil::PureFtpd => "---------- Welcome to Pure-FTPd ----------",
    };
    s.resp(220, saudacao)?;
    while let Some((comando, arg)) = s.ler_comando()? {
        if s.executar(&comando, &arg)? {
            break;
        }
    }
    let ctrl = std::mem::replace(&mut s.ctrl, Conn::Vazio);
    ctrl.fechar();
    Ok(())
}

fn fmt_data(segundos: i64) -> String {
    let dias = segundos.div_euclid(86_400);
    let resto = segundos.rem_euclid(86_400);
    let (ano, mes, dia) = civil(dias);
    format!(
        "{ano:04}{mes:02}{dia:02}{:02}{:02}{:02}",
        resto / 3600,
        resto % 3600 / 60,
        resto % 60
    )
}

fn civil(dias: i64) -> (i64, u32, u32) {
    let z = dias + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let dia = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let mes = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (yoe + era * 400 + i64::from(mes <= 2), mes, dia)
}

fn dias_de(ano: i64, mes: u32, dia: u32) -> i64 {
    let y = if mes <= 2 { ano - 1 } else { ano };
    let era = y.div_euclid(400);
    let yoe = y.rem_euclid(400);
    let m = i64::from(mes);
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + i64::from(dia) - 1;
    era * 146_097 + yoe * 365 + yoe / 4 - yoe / 100 + doy - 719_468
}

/// `aaaammddhhmmss` → segundos desde 1970 (UTC).
fn ler_data(texto: &str) -> Option<i64> {
    let texto = texto.split('.').next()?;
    if texto.len() != 14 || !texto.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let n = |r: std::ops::Range<usize>| texto[r].parse::<u32>().ok();
    Some(
        dias_de(i64::from(n(0..4)?), n(4..6)?, n(6..8)?) * 86_400
            + i64::from(n(8..10)? * 3600 + n(10..12)? * 60 + n(12..14)?),
    )
}

fn segundos(tempo: SystemTime) -> i64 {
    tempo
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

const MESES: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

impl Sessao {
    fn opcoes(&self) -> &Opcoes {
        &self.comum.opcoes
    }

    fn perfil(&self) -> Perfil {
        self.comum.opcoes.perfil
    }

    fn registrar(&self, linha: &str) {
        self.comum
            .estado
            .comandos
            .lock()
            .unwrap()
            .push(linha.to_owned());
    }

    fn bytes(&self, texto: &str) -> Vec<u8> {
        if self.opcoes().latin1 {
            texto.chars().map(|c| c as u32 as u8).collect()
        } else {
            texto.as_bytes().to_vec()
        }
    }

    fn texto(&self, bytes: &[u8]) -> String {
        if self.opcoes().latin1 {
            bytes.iter().map(|&b| char::from(b)).collect()
        } else {
            String::from_utf8_lossy(bytes).into_owned()
        }
    }

    fn resp(&mut self, codigo: u16, texto: &str) -> io::Result<()> {
        self.ctrl
            .write_all(format!("{codigo} {texto}\r\n").as_bytes())?;
        self.ctrl.flush()
    }

    fn resp_linhas(
        &mut self,
        codigo: u16,
        cabecalho: &str,
        linhas: &[String],
        fim: &str,
    ) -> io::Result<()> {
        let mut saida = format!("{codigo}-{cabecalho}\r\n");
        for linha in linhas {
            saida.push(' ');
            saida.push_str(linha);
            saida.push_str("\r\n");
        }
        saida.push_str(&format!("{codigo} {fim}\r\n"));
        self.ctrl.write_all(&self.bytes(&saida))?;
        self.ctrl.flush()
    }

    fn ler_comando(&mut self) -> io::Result<Option<(String, String)>> {
        loop {
            if let Some(fim) = self.entrada.iter().position(|&b| b == b'\n') {
                let mut linha: Vec<u8> = self.entrada.drain(..=fim).collect();
                linha.pop();
                if linha.last() == Some(&b'\r') {
                    linha.pop();
                }
                let linha = self.texto(&linha);
                let (comando, arg) = match linha.split_once(' ') {
                    Some((c, a)) => (c.to_ascii_uppercase(), a.to_owned()),
                    None => (linha.to_ascii_uppercase(), String::new()),
                };
                return Ok(Some((comando, arg)));
            }
            let mut pedaco = [0u8; 4096];
            match self.ctrl.read(&mut pedaco) {
                Ok(0) => return Ok(None),
                Ok(n) => self.entrada.extend_from_slice(&pedaco[..n]),
                Err(e)
                    if matches!(
                        e.kind(),
                        io::ErrorKind::ConnectionReset
                            | io::ErrorKind::TimedOut
                            | io::ErrorKind::WouldBlock
                    ) =>
                {
                    return Ok(None);
                }
                Err(e) => return Err(e),
            }
        }
    }

    /// O caminho no disco; `None` se tentar sair da raiz.
    fn disco(&self, caminho: &str) -> Option<PathBuf> {
        let juntado = if caminho.starts_with('/') {
            caminho.to_owned()
        } else {
            format!("{}/{caminho}", self.cwd.trim_end_matches('/'))
        };
        let mut partes: Vec<&str> = Vec::new();
        for parte in juntado.split('/') {
            match parte {
                "" | "." => {}
                ".." => {
                    partes.pop()?;
                }
                outra => partes.push(outra),
            }
        }
        let mut saida = self.comum.raiz.clone();
        saida.extend(partes);
        Some(saida)
    }

    fn virtual_de(&self, caminho: &str) -> String {
        let disco = self.disco(caminho).unwrap_or_default();
        let relativo = disco
            .strip_prefix(&self.comum.raiz)
            .unwrap_or(Path::new(""));
        format!("/{}", relativo.to_string_lossy())
    }

    fn tem_mlst(&self) -> bool {
        self.perfil() != Perfil::Vsftpd
    }

    fn feat(&self) -> Vec<String> {
        let mut f = Vec::new();
        if self.opcoes().tls != Tls::Nenhum {
            f.push("AUTH TLS".to_owned());
        }
        f.push("EPRT".to_owned());
        if !self.opcoes().epsv_recusado {
            f.push("EPSV".to_owned());
        }
        f.push("MDTM".to_owned());
        if self.perfil() != Perfil::Vsftpd {
            f.push("MFMT".to_owned());
            f.push("MLST type*;size*;modify*;UNIX.mode*;UNIX.owner*;UNIX.group*;".to_owned());
        }
        f.push("PASV".to_owned());
        if self.opcoes().tls != Tls::Nenhum {
            f.push("PBSZ".to_owned());
            f.push("PROT".to_owned());
        }
        if !self.opcoes().sem_rest_stream {
            f.push("REST STREAM".to_owned());
        }
        f.push("SIZE".to_owned());
        f.push("TVFS".to_owned());
        if !self.opcoes().latin1 {
            f.push("UTF8".to_owned());
        }
        f
    }

    /// Executa um comando; `true` quando a sessão acabou.
    fn executar(&mut self, comando: &str, arg: &str) -> io::Result<bool> {
        let linha = if comando == "PASS" {
            "PASS ***".to_owned()
        } else {
            format!("{comando} {arg}").trim_end().to_owned()
        };
        self.registrar(&linha);
        if self
            .comum
            .estado
            .derrubar_proximo
            .swap(false, Ordering::SeqCst)
        {
            self.resp(421, "Timeout. Closing control connection.")?;
            return Ok(true);
        }
        let livre = matches!(
            comando,
            "USER"
                | "PASS"
                | "ACCT"
                | "AUTH"
                | "PBSZ"
                | "PROT"
                | "FEAT"
                | "OPTS"
                | "QUIT"
                | "NOOP"
                | "SYST"
        );
        if !self.autenticado && !livre {
            self.resp(530, "Please login with USER and PASS.")?;
            return Ok(false);
        }
        match comando {
            "USER" => {
                self.usuario = Some(arg.to_owned());
                self.autenticado = false;
                self.resp(331, "Please specify the password.")?;
            }
            "PASS" => {
                let usuario = self.usuario.clone().unwrap_or_default();
                match usuario.as_str() {
                    "ana" if arg == "senha" => {
                        self.autenticado = true;
                        self.resp(230, "Login successful.")?;
                    }
                    "anonymous" | "ftp" => {
                        self.autenticado = true;
                        self.resp(230, "Anonymous access granted.")?;
                    }
                    "conta" if arg == "senha" => self.resp(332, "Need account for login.")?,
                    _ => self.resp(530, "Login incorrect.")?,
                }
            }
            "ACCT" => {
                if arg == "projeto" {
                    self.autenticado = true;
                    self.resp(230, "Account accepted, login successful.")?;
                } else {
                    self.resp(530, "Account incorrect.")?;
                }
            }
            "AUTH" => {
                let pedido = arg.to_ascii_uppercase();
                if self.opcoes().tls == Tls::Explicito
                    && matches!(pedido.as_str(), "TLS" | "SSL" | "TLS-C")
                    && !matches!(self.ctrl, Conn::Tls(_))
                {
                    self.resp(234, &format!("Proceed with negotiation ({pedido})."))?;
                    let tcp = match std::mem::replace(&mut self.ctrl, Conn::Vazio) {
                        Conn::Plain(tcp) => tcp,
                        _ => return Ok(true),
                    };
                    let config = self.comum.tls_controle.clone().unwrap();
                    match Conn::tls(tcp, config) {
                        Ok(conn) => {
                            if let Some(v) = conn.versao() {
                                self.comum.estado.versoes.lock().unwrap().push(v.to_owned());
                            }
                            self.ctrl = conn;
                        }
                        Err(_) => return Ok(true),
                    }
                } else {
                    self.resp(502, "Security mechanism not implemented.")?;
                }
            }
            "PBSZ" => self.resp(200, "PBSZ set to 0.")?,
            "PROT" => match arg.to_ascii_uppercase().as_str() {
                "P" if matches!(self.ctrl, Conn::Tls(_)) => {
                    self.protegido = true;
                    self.resp(200, "PROT now Private.")?;
                }
                "C" => {
                    self.protegido = false;
                    self.resp(200, "PROT now Clear.")?;
                }
                _ => self.resp(536, "Requested PROT level not supported.")?,
            },
            "FEAT" => {
                let linhas = self.feat();
                self.resp_linhas(211, "Features:", &linhas, "End")?;
            }
            "OPTS" => {
                if arg.to_ascii_uppercase().starts_with("UTF8") && !self.opcoes().latin1 {
                    self.resp(200, "Always in UTF8 mode.")?;
                } else {
                    self.resp(501, "Option not understood.")?;
                }
            }
            "SYST" => self.resp(215, "UNIX Type: L8")?,
            "TYPE" => self.resp(200, "Switching to Binary mode.")?,
            "NOOP" => self.resp(200, "NOOP ok.")?,
            "PWD" | "XPWD" => {
                let cwd = self.cwd.clone();
                self.resp(
                    257,
                    &format!("\"{}\" is the current directory", cwd.replace('"', "\"\"")),
                )?;
            }
            "CWD" | "CDUP" => {
                let alvo = if comando == "CDUP" { ".." } else { arg };
                match self.disco(alvo) {
                    Some(d) if d.is_dir() => {
                        self.cwd = self.virtual_de(alvo);
                        self.resp(250, "Directory successfully changed.")?;
                    }
                    _ => self.resp(550, "Failed to change directory.")?,
                }
            }
            "QUIT" => {
                self.resp(221, "Goodbye.")?;
                return Ok(true);
            }
            "ABOR" => {
                if self.abortou {
                    self.abortou = false;
                    self.resp(226, "Abort successful")?;
                } else {
                    self.resp(226, "No transfer to ABOR.")?;
                }
            }
            "EPSV" => {
                if self.opcoes().epsv_recusado {
                    self.resp(500, "Unknown command.")?;
                } else {
                    let listener = TcpListener::bind("127.0.0.1:0")?;
                    let porta = listener.local_addr()?.port();
                    self.pasv = Some(listener);
                    self.ativo = None;
                    self.resp(
                        229,
                        &format!("Entering Extended Passive Mode (|||{porta}|)."),
                    )?;
                }
            }
            "PASV" => {
                let listener = TcpListener::bind("127.0.0.1:0")?;
                let porta = listener.local_addr()?.port();
                self.pasv = Some(listener);
                self.ativo = None;
                let ip = self.opcoes().ip_pasv.unwrap_or(Ipv4Addr::LOCALHOST);
                let [a, b, c, d] = ip.octets();
                self.resp(
                    227,
                    &format!(
                        "Entering Passive Mode ({a},{b},{c},{d},{},{}).",
                        porta / 256,
                        porta % 256
                    ),
                )?;
            }
            "EPRT" => {
                let campos: Vec<&str> = arg.split('|').collect();
                let endereco = campos
                    .get(2)
                    .and_then(|ip| ip.parse::<IpAddr>().ok())
                    .zip(campos.get(3).and_then(|p| p.parse::<u16>().ok()));
                if self.opcoes().eprt_recusado {
                    self.resp(500, "Unknown command.")?;
                } else if let Some((ip, porta)) = endereco {
                    self.ativo = Some(SocketAddr::new(ip, porta));
                    self.pasv = None;
                    self.resp(200, "EPRT command successful.")?;
                } else {
                    self.resp(501, "Illegal EPRT command.")?;
                }
            }
            "PORT" => {
                let n: Vec<u16> = arg
                    .split(',')
                    .filter_map(|p| p.trim().parse().ok())
                    .collect();
                if n.len() == 6 {
                    let ip = Ipv4Addr::new(n[0] as u8, n[1] as u8, n[2] as u8, n[3] as u8);
                    self.ativo = Some(SocketAddr::new(IpAddr::V4(ip), n[4] * 256 + n[5]));
                    self.pasv = None;
                    self.resp(200, "PORT command successful.")?;
                } else {
                    self.resp(501, "Illegal PORT command.")?;
                }
            }
            "REST" => match arg.parse::<u64>() {
                Ok(n) if !self.opcoes().sem_rest_stream => {
                    self.rest = n;
                    self.resp(350, &format!("Restart position accepted ({n})."))?;
                }
                _ => self.resp(501, "REST requires a value greater than or equal to 0.")?,
            },
            "LIST" | "NLST" => self.listar(arg)?,
            "MLSD" if self.tem_mlst() => self.mlsd(arg)?,
            "MLST" if self.tem_mlst() => self.mlst(arg)?,
            "RETR" => self.baixar(arg)?,
            "STOR" => self.enviar(arg, false)?,
            "APPE" => self.enviar(arg, true)?,
            "SIZE" => match self.disco(arg).map(|d| (d.clone(), fs::metadata(d))) {
                Some((_, Ok(m))) if m.is_file() => self.resp(213, &m.len().to_string())?,
                Some((_, Ok(_))) => self.resp(550, "Could not get file size.")?,
                _ => self.resp(550, "Could not get file size.")?,
            },
            "MDTM" => self.mdtm(arg)?,
            "MFMT" if self.tem_mlst() => match arg.split_once(' ') {
                Some((data, caminho)) => self.definir_data(data, caminho, 213, "Modify=")?,
                None => self.resp(501, "Invalid MFMT syntax.")?,
            },
            "MKD" | "XMKD" => match self.disco(arg) {
                Some(d) if fs::create_dir(&d).is_ok() => {
                    let v = self.virtual_de(arg);
                    self.resp(257, &format!("\"{v}\" created"))?;
                }
                _ => self.resp(550, "Create directory operation failed.")?,
            },
            "RMD" | "XRMD" => match self.disco(arg) {
                Some(d) if fs::remove_dir(&d).is_ok() => {
                    self.resp(250, "Remove directory operation successful.")?
                }
                _ => self.resp(550, "Remove directory operation failed.")?,
            },
            "DELE" => match self.disco(arg) {
                Some(d) if d.symlink_metadata().is_ok() && fs::remove_file(&d).is_ok() => {
                    self.resp(250, "Delete operation successful.")?;
                }
                _ => self.resp(550, "Delete operation failed.")?,
            },
            "RNFR" => match self.disco(arg) {
                Some(d) if d.symlink_metadata().is_ok() => {
                    self.renomear = Some(arg.to_owned());
                    self.resp(350, "Ready for RNTO.")?;
                }
                _ => self.resp(550, "RNFR command failed.")?,
            },
            "RNTO" => {
                let origem = self.renomear.take().and_then(|o| self.disco(&o));
                match (origem, self.disco(arg)) {
                    (Some(o), Some(d)) if fs::rename(&o, &d).is_ok() => {
                        self.resp(250, "Rename successful.")?
                    }
                    (None, _) => self.resp(503, "RNFR required first.")?,
                    _ => self.resp(550, "Rename failed.")?,
                }
            }
            "SITE" => self.site(arg)?,
            "HELP" => self.resp(214, "Help OK.")?,
            _ => self.resp(500, "Unknown command.")?,
        }
        Ok(false)
    }

    fn definir_data(
        &mut self,
        data: &str,
        caminho: &str,
        codigo: u16,
        prefixo: &str,
    ) -> io::Result<()> {
        let (Some(tempo), Some(disco)) = (ler_data(data), self.disco(caminho)) else {
            return self.resp(501, "Invalid time.");
        };
        let Ok(arquivo) = OpenOptions::new().read(true).open(&disco) else {
            return self.resp(550, "No such file or directory.");
        };
        let tempo = UNIX_EPOCH + Duration::from_secs(tempo as u64);
        if arquivo.set_modified(tempo).is_ok() {
            let texto = if prefixo.is_empty() {
                "File modification time set.".to_owned()
            } else {
                format!("{prefixo}{data}; {caminho}")
            };
            self.resp(codigo, &texto)
        } else {
            self.resp(550, "Could not set time.")
        }
    }

    fn mdtm(&mut self, arg: &str) -> io::Result<()> {
        // `MDTM aaaammddhhmmss caminho`: o vsftpd muda a data; os outros leem um arquivo de
        // nome esquisito (que não existe).
        if let Some((data, caminho)) = arg.split_once(' ')
            && ler_data(data).is_some()
            && self.perfil() == Perfil::Vsftpd
        {
            return self.definir_data(data, caminho, 213, "");
        }
        match self.disco(arg).and_then(|d| fs::metadata(d).ok()) {
            Some(m) => {
                let t = m.modified().map(segundos).unwrap_or(0);
                self.resp(213, &fmt_data(t))
            }
            None => self.resp(550, "Could not get file modification time."),
        }
    }

    fn site(&mut self, arg: &str) -> io::Result<()> {
        let (sub, resto) = arg.split_once(' ').unwrap_or((arg, ""));
        match (sub.to_ascii_uppercase().as_str(), self.perfil()) {
            ("HELP", Perfil::Vsftpd) => self.resp(214, "CHMOD UMASK HELP"),
            ("HELP", Perfil::Proftpd) => {
                let linhas = ["CHMOD", "SYMLINK", "UTIME", "HELP"].map(str::to_owned);
                self.resp_linhas(
                    214,
                    "The following SITE commands are recognized (* =>'s unimplemented):",
                    &linhas,
                    "Direct comments to root@localhost",
                )
            }
            ("HELP", Perfil::PureFtpd) => self.resp(214, "CHMOD IDLE"),
            ("CHMOD", _) => {
                let Some((modo, caminho)) = resto.split_once(' ') else {
                    return self.resp(501, "SITE CHMOD needs 2 arguments.");
                };
                let (Ok(modo), Some(disco)) = (u32::from_str_radix(modo, 8), self.disco(caminho))
                else {
                    return self.resp(501, "SITE CHMOD needs a numeric mode.");
                };
                if fs::set_permissions(&disco, fs::Permissions::from_mode(modo)).is_ok() {
                    self.resp(200, "SITE CHMOD command ok.")
                } else {
                    self.resp(550, "Could not chmod file.")
                }
            }
            ("UTIME", Perfil::Proftpd) => match resto.split_once(' ') {
                Some((data, caminho)) => {
                    self.definir_data(data, caminho, 200, "SITE UTIME command successful")
                }
                None => self.resp(501, "Invalid SITE UTIME syntax."),
            },
            ("SYMLINK", Perfil::Proftpd) => {
                let Some((alvo, link)) = resto.split_once(' ') else {
                    return self.resp(501, "SITE SYMLINK needs 2 arguments.");
                };
                match self.disco(link) {
                    Some(d) if std::os::unix::fs::symlink(alvo, &d).is_ok() => {
                        self.resp(200, "SITE SYMLINK command successful.")
                    }
                    _ => self.resp(550, "SITE SYMLINK command failed."),
                }
            }
            _ => self.resp(500, "Unknown SITE command."),
        }
    }

    // ----- listagens -----

    fn entradas(&self, dir: &Path) -> Option<Vec<(String, PathBuf)>> {
        let mut saida: Vec<(String, PathBuf)> = fs::read_dir(dir)
            .ok()?
            .filter_map(Result::ok)
            .map(|e| (e.file_name().to_string_lossy().into_owned(), e.path()))
            .collect();
        saida.sort();
        Some(saida)
    }

    fn linha_list(&self, nome: &str, caminho: &Path) -> Option<String> {
        let m = fs::symlink_metadata(caminho).ok()?;
        let modo = m.mode();
        let tipo = if m.is_dir() {
            'd'
        } else if m.file_type().is_symlink() {
            'l'
        } else {
            '-'
        };
        let mut perms = String::new();
        for (i, c) in "rwxrwxrwx".chars().enumerate() {
            perms.push(if modo & (1 << (8 - i)) != 0 { c } else { '-' });
        }
        let agora = segundos(SystemTime::now());
        let t = m.mtime() + i64::from(self.opcoes().fuso_min) * 60;
        let (ano, mes, dia) = civil(t.div_euclid(86_400));
        let resto = t.rem_euclid(86_400);
        let data = if agora - m.mtime() > 180 * 86_400 || m.mtime() > agora + 86_400 {
            format!("{} {dia:2}  {ano}", MESES[(mes - 1) as usize])
        } else {
            format!(
                "{} {dia:2} {:02}:{:02}",
                MESES[(mes - 1) as usize],
                resto / 3600,
                resto % 3600 / 60
            )
        };
        let alvo = if tipo == 'l' {
            format!(" -> {}", fs::read_link(caminho).ok()?.to_string_lossy())
        } else {
            String::new()
        };
        Some(format!(
            "{tipo}{perms}    1 ftp      ftp      {:>8} {data} {nome}{alvo}\r\n",
            m.len()
        ))
    }

    fn linha_mlsd(&self, nome: &str, caminho: &Path, completa: bool) -> Option<String> {
        let m = fs::symlink_metadata(caminho).ok()?;
        let tipo = if m.is_dir() {
            "dir".to_owned()
        } else if m.file_type().is_symlink() {
            format!(
                "OS.unix=slink:{}",
                fs::read_link(caminho).ok()?.to_string_lossy()
            )
        } else {
            "file".to_owned()
        };
        let _ = completa;
        Some(format!(
            "type={tipo};size={};modify={};UNIX.mode=0{:o};UNIX.owner={};UNIX.group={}; {nome}\r\n",
            m.len(),
            fmt_data(m.mtime()),
            m.mode() & 0o7777,
            m.uid(),
            m.gid()
        ))
    }

    fn mlsd(&mut self, arg: &str) -> io::Result<()> {
        let alvo = if arg.is_empty() {
            self.cwd.clone()
        } else {
            arg.to_owned()
        };
        let Some(dir) = self.disco(&alvo).filter(|d| d.is_dir()) else {
            return self.resp(550, "No such file or directory.");
        };
        let mut texto = String::new();
        texto.push_str(
            &self
                .linha_mlsd(".", &dir, true)
                .unwrap_or_default()
                .replace("type=dir", "type=cdir"),
        );
        for (nome, caminho) in self.entradas(&dir).unwrap_or_default() {
            if let Some(l) = self.linha_mlsd(&nome, &caminho, false) {
                texto.push_str(&l);
            }
        }
        self.mandar_dados(self.bytes(&texto))
    }

    fn mlst(&mut self, arg: &str) -> io::Result<()> {
        let alvo = if arg.is_empty() {
            self.cwd.clone()
        } else {
            arg.to_owned()
        };
        let Some(caminho) = self.disco(&alvo).filter(|d| d.symlink_metadata().is_ok()) else {
            return self.resp(550, "No such file or directory.");
        };
        let nome = self.virtual_de(&alvo);
        let linha = self.linha_mlsd(&nome, &caminho, true).unwrap_or_default();
        self.resp_linhas(
            250,
            &format!("Start of list for {nome}"),
            &[linha.trim_end().to_owned()],
            "End of list",
        )
    }

    fn listar(&mut self, arg: &str) -> io::Result<()> {
        let mut alvo = arg;
        // O vsftpd e o pure-ftpd tratam `-a`; um servidor "sem -a" não entra aqui.
        while let Some(resto) = alvo.strip_prefix('-') {
            alvo = resto.split_once(' ').map_or("", |(_, depois)| depois);
        }
        let alvo = if alvo.is_empty() {
            self.cwd.clone()
        } else {
            alvo.to_owned()
        };
        let Some(caminho) = self.disco(&alvo) else {
            return self.resp(550, "Failed to open directory.");
        };
        let mut texto = String::new();
        if caminho.is_dir() {
            let lista = self.entradas(&caminho).unwrap_or_default();
            texto.push_str(&format!("total {}\r\n", lista.len()));
            for (nome, c) in lista {
                if let Some(l) = self.linha_list(&nome, &c) {
                    texto.push_str(&l);
                }
            }
        } else if caminho.exists() {
            let nome = caminho.file_name().unwrap().to_string_lossy().into_owned();
            texto.push_str(&self.linha_list(&nome, &caminho).unwrap_or_default());
        } else {
            return self.resp(550, "Failed to open directory.");
        }
        self.mandar_dados(self.bytes(&texto))
    }

    // ----- dados -----

    /// Abre a conexão de dados (depois do `150`): aceita a passiva ou conecta na ativa, com TLS
    /// se for o caso. `None` se não deu (a resposta já foi mandada).
    fn abrir_dados(&mut self) -> io::Result<Option<Conn>> {
        let tcp = if let Some(listener) = self.pasv.take() {
            listener.set_nonblocking(true)?;
            let limite = std::time::Instant::now() + Duration::from_secs(10);
            loop {
                match listener.accept() {
                    Ok((tcp, _)) => break Some(tcp),
                    Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                        if std::time::Instant::now() > limite {
                            break None;
                        }
                        thread::sleep(Duration::from_millis(5));
                    }
                    Err(_) => break None,
                }
            }
        } else if let Some(endereco) = self.ativo.take() {
            TcpStream::connect_timeout(&endereco, Duration::from_secs(5)).ok()
        } else {
            self.resp(425, "Use PORT or PASV first.")?;
            return Ok(None);
        };
        let Some(tcp) = tcp else {
            self.resp(425, "Failed to establish connection.")?;
            return Ok(None);
        };
        let _ = tcp.set_nodelay(true);
        tcp.set_read_timeout(Some(Duration::from_secs(30)))?;
        let estado = self.comum.estado.clone();
        if !self.protegido {
            estado.dados_claros.fetch_add(1, Ordering::SeqCst);
            return Ok(Some(Conn::Plain(tcp)));
        }
        let config = self.comum.tls_dados.clone().unwrap();
        let conn = match Conn::tls(tcp, config) {
            Ok(conn) => conn,
            Err(_) => {
                self.resp(
                    425,
                    "Unable to build data connection: TLS handshake failed.",
                )?;
                return Ok(None);
            }
        };
        if let Some(v) = conn.versao() {
            estado.versoes.lock().unwrap().push(v.to_owned());
        }
        if conn.retomou() {
            estado.dados_retomados.fetch_add(1, Ordering::SeqCst);
        } else {
            estado.dados_completos.fetch_add(1, Ordering::SeqCst);
            if self.opcoes().exige_retomada {
                estado.recusados.fetch_add(1, Ordering::SeqCst);
                match self.perfil() {
                    Perfil::Vsftpd => {
                        self.resp(522, "SSL connection failed: session reuse required")?
                    }
                    _ => self.resp(
                        425,
                        "Unable to build data connection: Operation not permitted",
                    )?,
                }
                conn.fechar();
                return Ok(None);
            }
        }
        Ok(Some(conn))
    }

    fn mandar_dados(&mut self, dados: Vec<u8>) -> io::Result<()> {
        self.resp(150, "Here comes the directory listing.")?;
        let Some(mut conn) = self.abrir_dados()? else {
            return Ok(());
        };
        let ok = conn.write_all(&dados).and_then(|()| conn.flush()).is_ok();
        conn.fechar();
        if ok {
            self.resp(226, "Directory send OK.")
        } else {
            self.resp(426, "Failure writing network stream.")
        }
    }

    fn baixar(&mut self, arg: &str) -> io::Result<()> {
        let rest = std::mem::take(&mut self.rest);
        let mut arquivo = match self
            .disco(arg)
            .filter(|d| d.is_file())
            .and_then(|d| File::open(d).ok())
        {
            Some(a) => a,
            None => {
                return match self.perfil() {
                    Perfil::Vsftpd => self.resp(550, "Failed to open file."),
                    Perfil::Proftpd => self.resp(550, &format!("{arg}: No such file or directory")),
                    Perfil::PureFtpd => {
                        self.resp(550, &format!("Can't open {arg}: No such file or directory"))
                    }
                };
            }
        };
        arquivo.seek(SeekFrom::Start(rest))?;
        self.resp(150, "Opening BINARY mode data connection.")?;
        let Some(mut conn) = self.abrir_dados()? else {
            return Ok(());
        };
        let lento = self.opcoes().lento;
        let mut bloco = vec![0u8; 16 * 1024];
        let mut falhou = false;
        loop {
            let n = arquivo.read(&mut bloco)?;
            if n == 0 {
                break;
            }
            if conn.write_all(&bloco[..n]).is_err() {
                falhou = true;
                break;
            }
            if lento {
                thread::sleep(Duration::from_millis(2));
            }
        }
        if !falhou {
            falhou = conn.flush().is_err();
        }
        conn.fechar();
        if falhou {
            self.abortou = true;
            self.resp(426, "Failure writing network stream.")
        } else {
            self.resp(226, "Transfer complete.")
        }
    }

    fn enviar(&mut self, arg: &str, anexar: bool) -> io::Result<()> {
        let rest = std::mem::take(&mut self.rest);
        let Some(disco) = self.disco(arg) else {
            return self.resp(553, "Could not create file.");
        };
        let aberto = if anexar {
            OpenOptions::new().create(true).append(true).open(&disco)
        } else if rest > 0 {
            OpenOptions::new()
                .create(true)
                .write(true)
                .truncate(false)
                .open(&disco)
        } else {
            OpenOptions::new()
                .create(true)
                .write(true)
                .truncate(true)
                .open(&disco)
        };
        let mut arquivo = match aberto {
            Ok(a) => a,
            Err(_) => return self.resp(553, "Could not create file."),
        };
        if !anexar {
            arquivo.seek(SeekFrom::Start(rest))?;
        }
        self.resp(150, "Ok to send data.")?;
        let Some(mut conn) = self.abrir_dados()? else {
            return Ok(());
        };
        let mut bloco = vec![0u8; 64 * 1024];
        let mut falhou = false;
        loop {
            match conn.read(&mut bloco) {
                Ok(0) => break,
                Ok(n) => arquivo.write_all(&bloco[..n])?,
                Err(_) => {
                    falhou = true;
                    break;
                }
            }
        }
        conn.fechar();
        if falhou {
            self.abortou = true;
            self.resp(426, "Failure reading network stream.")
        } else {
            self.resp(226, "Transfer complete.")
        }
    }
}
