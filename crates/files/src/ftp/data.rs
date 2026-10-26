//! As conexões de dados: passivas e ativas, com TLS (retomando a sessão do controle), e as
//! transferências (`Download` e `Upload`) sobre elas.

use std::io::{self, Read, Write};
use std::net::{IpAddr, SocketAddr, TcpListener, TcpStream};
use std::time::{Duration, Instant};

use tomni_ui_net::CONNECT_TIMEOUT;

use super::control::{Reply, reply_code, reply_error};
use super::tls::{Stream, TlsVersion, is_handshake_error};
use super::{Ftp, FtpMode};
use crate::fs::{Download, Upload};

/// Quanto esperar uma resposta que "pode" vir logo (o erro de uma conexão de dados sem retomada).
const ESPERA_RAPIDA: Duration = Duration::from_millis(250);
/// Quanto esperar a resposta do controle depois que o servidor fechou os dados.
const ESPERA_RECUSA: Duration = Duration::from_secs(1);
/// Quanto esperar a resposta do controle quando os dados seguem abertos.
const ESPERA_CURTA: Duration = Duration::from_millis(30);

/// Quanto esperar as respostas do `ABOR`.
const ESPERA_ABOR: Duration = Duration::from_secs(5);

/// O endereço do `PASV`, `(h1,h2,h3,h4,p1,p2)`.
pub(super) fn parse_pasv(text: &str) -> Option<(std::net::Ipv4Addr, u16)> {
    // Depois do parêntese, ou depois do código da resposta (`227 …`).
    let start = match text.find('(') {
        Some(open) => open + 1,
        None => {
            let skip = if text
                .as_bytes()
                .get(..3)
                .is_some_and(|b| b.iter().all(u8::is_ascii_digit))
            {
                3
            } else {
                0
            };
            skip + text[skip..].find(|c: char| c.is_ascii_digit())?
        }
    };
    let numbers: Vec<u16> = text[start..]
        .split(|c: char| !c.is_ascii_digit())
        .filter(|part| !part.is_empty())
        .take(6)
        .map(|part| part.parse().ok())
        .collect::<Option<_>>()?;
    if numbers.len() != 6 || numbers.iter().any(|&n| n > 255) {
        return None;
    }
    let ip = std::net::Ipv4Addr::new(
        numbers[0] as u8,
        numbers[1] as u8,
        numbers[2] as u8,
        numbers[3] as u8,
    );
    Some((ip, numbers[4] * 256 + numbers[5]))
}

/// A porta do `EPSV`, `(|||porta|)` (o separador pode ser qualquer caractere).
pub(super) fn parse_epsv(text: &str) -> Option<u16> {
    let inner = &text[text.find('(')? + 1..];
    let delimiter = inner.chars().next()?;
    let mut parts = inner.split(delimiter);
    parts.next()?; // vazio antes do primeiro separador
    parts.next()?; // protocolo
    parts.next()?; // endereço
    parts.next()?.parse().ok()
}

/// `true` no endereço que não adianta para quem está fora da rede do servidor.
fn is_private(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            v4.is_private() || v4.is_loopback() || v4.is_unspecified() || v4.is_link_local()
        }
        IpAddr::V6(v6) => v6.is_loopback() || v6.is_unspecified(),
    }
}

/// O que falhou ao abrir a conexão de dados, e se vale tentar de novo sem o TLS 1.3.
pub(super) struct DataFail {
    pub error: io::Error,
    pub suspect: bool,
}

impl From<io::Error> for DataFail {
    fn from(error: io::Error) -> Self {
        Self {
            error,
            suspect: false,
        }
    }
}

enum Pending {
    Connected(TcpStream),
    Listening(TcpListener),
}

impl Ftp {
    /// O endereço da conexão de dados no modo passivo: `EPSV` e, se o servidor recusar, `PASV`.
    fn passive_endpoint(&mut self) -> io::Result<(String, u16)> {
        if self.state.use_epsv {
            let reply = self.cmd("EPSV")?;
            if reply.code == 229
                && let Some(port) = parse_epsv(&reply.lines.join(" "))
            {
                return Ok((self.state.server.clone(), port));
            }
            if reply.code >= 500 || reply.code == 229 {
                self.state.use_epsv = false;
            } else {
                return Err(reply_error(&reply, "abrir os dados com", ""));
            }
        }
        if self.state.server_ip.is_some_and(|ip| ip.is_ipv6()) {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "o servidor recusou o EPSV e o PASV não funciona em IPv6",
            ));
        }
        let reply = self.cmd("PASV")?;
        if reply.code != 227 {
            return Err(reply_error(&reply, "abrir os dados com", ""));
        }
        let Some((ip, port)) = parse_pasv(&reply.lines.join(" ")) else {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "o servidor respondeu o PASV fora do formato",
            ));
        };
        // Atrás de NAT o servidor conta o próprio endereço de dentro: vale o do controle.
        let host = if is_private(IpAddr::V4(ip)) {
            self.state.server.clone()
        } else {
            ip.to_string()
        };
        Ok((host, port))
    }

    fn data_connect(&self, host: &str, port: u16) -> io::Result<TcpStream> {
        let tcp = tomni_ui_net::connect(host, port, self.cfg.proxy.as_ref(), CONNECT_TIMEOUT)
            .map_err(|error| {
                io::Error::new(error.kind(), tomni_ui_net::connect_error_message(&error))
            })?;
        let _ = tcp.set_nodelay(true);
        tcp.set_read_timeout(Some(self.cfg.timeout))?;
        tcp.set_write_timeout(Some(self.cfg.timeout))?;
        Ok(tcp)
    }

    /// Prepara o modo ativo: escuta numa porta e a anuncia com `EPRT` (ou `PORT`).
    fn active_listen(&mut self) -> io::Result<TcpListener> {
        if self.cfg.proxy.is_some() {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "o modo ativo não funciona por proxy; use o passivo",
            ));
        }
        let local = self
            .ctrl
            .as_ref()
            .ok_or_else(|| io::Error::from(io::ErrorKind::ConnectionReset))?
            .stream
            .local_addr()?
            .ip();
        let announce = self.cfg.active_address.unwrap_or(local);
        let bind_ip = if self.cfg.active_address.is_some() {
            match local {
                IpAddr::V4(_) => IpAddr::from([0, 0, 0, 0]),
                IpAddr::V6(_) => IpAddr::from([0u16; 8]),
            }
        } else {
            local
        };
        let listener = match &self.cfg.active_ports {
            Some(range) => range
                .clone()
                .find_map(|port| TcpListener::bind(SocketAddr::new(bind_ip, port)).ok())
                .ok_or_else(|| {
                    io::Error::new(
                        io::ErrorKind::AddrInUse,
                        "nenhuma porta livre na faixa do modo ativo",
                    )
                })?,
            None => TcpListener::bind(SocketAddr::new(bind_ip, 0))?,
        };
        let port = listener.local_addr()?.port();
        if self.state.use_eprt {
            let family = if announce.is_ipv4() { 1 } else { 2 };
            let reply = self.cmd(&format!("EPRT |{family}|{announce}|{port}|"))?;
            if reply.ok() {
                return Ok(listener);
            }
            if reply.code < 500 {
                return Err(reply_error(&reply, "abrir os dados com", ""));
            }
            self.state.use_eprt = false;
        }
        let IpAddr::V4(v4) = announce else {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "o servidor não aceitou o EPRT e o PORT não funciona em IPv6",
            ));
        };
        let [a, b, c, d] = v4.octets();
        let reply = self.cmd(&format!(
            "PORT {a},{b},{c},{d},{},{}",
            port / 256,
            port % 256
        ))?;
        if reply.ok() {
            Ok(listener)
        } else {
            Err(reply_error(&reply, "abrir os dados com", ""))
        }
    }

    /// Aceita a conexão do servidor (só a do IP dele) por até o tempo-limite.
    fn active_accept(&self, listener: &TcpListener) -> io::Result<TcpStream> {
        listener.set_nonblocking(true)?;
        let deadline = Instant::now() + self.cfg.timeout;
        loop {
            match listener.accept() {
                Ok((tcp, from)) => {
                    if self
                        .state
                        .server_ip
                        .is_some_and(|server| server != from.ip())
                    {
                        continue;
                    }
                    tcp.set_nonblocking(false)?;
                    let _ = tcp.set_nodelay(true);
                    tcp.set_read_timeout(Some(self.cfg.timeout))?;
                    tcp.set_write_timeout(Some(self.cfg.timeout))?;
                    return Ok(tcp);
                }
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                    if Instant::now() >= deadline {
                        return Err(io::Error::new(
                            io::ErrorKind::TimedOut,
                            "o servidor não abriu a conexão de dados (modo ativo)",
                        ));
                    }
                    std::thread::sleep(Duration::from_millis(10));
                }
                Err(error) => return Err(error),
            }
        }
    }

    /// Uma tentativa de abrir a conexão de dados para `command`.
    fn open_stream_once(
        &mut self,
        command: &str,
        offset: u64,
        action: &str,
        target: &str,
    ) -> Result<Stream, DataFail> {
        let pending = match self.cfg.mode {
            FtpMode::Passive => {
                let (host, port) = self.passive_endpoint()?;
                Pending::Connected(self.data_connect(&host, port)?)
            }
            FtpMode::Active => Pending::Listening(self.active_listen()?),
        };
        if offset > 0 {
            let reply = self.cmd(&format!("REST {offset}"))?;
            if reply.code != 350 {
                return Err(reply_error(&reply, "retomar", target).into());
            }
        }
        let reply = self.cmd(command)?;
        if !reply.preliminary() {
            let text = reply.text().to_lowercase();
            let suspect = matches!(reply.code, 425 | 522 | 534)
                || (reply.code == 450
                    && ["reuse", "session", "resum"]
                        .iter()
                        .any(|k| text.contains(k)));
            return Err(DataFail {
                error: reply_error(&reply, action, target),
                suspect,
            });
        }
        let tcp = match pending {
            Pending::Connected(tcp) => tcp,
            Pending::Listening(listener) => self.active_accept(&listener)?,
        };
        if !self.state.secured {
            return Ok(Stream::Plain(tcp));
        }
        let mut stream = self.tls()?.connect(tcp).map_err(|error| DataFail {
            suspect: is_handshake_error(&error),
            error,
        })?;
        if !stream.resumed() && stream.is_tls13() {
            // Um servidor que exige a retomada recusa logo depois do handshake: uns respondem
            // `425` no controle, outros (o proftpd) só fecham os dados e respondem depois. Uma
            // resposta que chega agora e não é erro fica guardada para o fim da transferência.
            let closed = stream.closed_within(ESPERA_RAPIDA);
            let wait = if closed { ESPERA_RECUSA } else { ESPERA_CURTA };
            if let Some(reply) = self.peek_reply(wait) {
                if reply.code >= 400 {
                    return Err(DataFail {
                        error: reply_error(&reply, action, target),
                        suspect: true,
                    });
                }
                self.stash_reply(reply);
            } else if closed {
                return Err(DataFail {
                    error: io::Error::new(
                        io::ErrorKind::ConnectionReset,
                        format!("o servidor fechou a conexão de dados ({action} {target})"),
                    ),
                    suspect: true,
                });
            }
        }
        Ok(stream)
    }

    /// Abre a conexão de dados; se o servidor não retoma a sessão no TLS 1.3, refaz a conexão
    /// de controle em TLS 1.2 e tenta de novo.
    pub(super) fn open_stream(
        &mut self,
        command: &str,
        offset: u64,
        action: &str,
        target: &str,
    ) -> io::Result<Stream> {
        match self.open_stream_once(command, offset, action, target) {
            Err(fail) if fail.suspect && self.can_fall_back() => {
                self.fall_back()?;
                self.open_stream_once(command, offset, action, target)
                    .map_err(|fail| fail.error)
            }
            other => other.map_err(|fail| fail.error),
        }
    }

    fn can_fall_back(&self) -> bool {
        self.state.secured
            && !self.state.fell_back
            && self.cfg.tls_version == TlsVersion::Auto
            && self
                .ctrl
                .as_ref()
                .is_some_and(|ctrl| ctrl.stream.is_tls13())
    }

    fn fall_back(&mut self) -> io::Result<()> {
        self.state.fell_back = true;
        self.state.tls_version = TlsVersion::Only12;
        self.state.notice =
            Some("o servidor não retoma sessões TLS 1.3; usando TLS 1.2".to_owned());
        if let Some(mut ctrl) = self.ctrl.take() {
            let _ = ctrl.stream.set_timeout(Some(Duration::from_secs(1)));
            let _ = ctrl.send("QUIT");
            ctrl.stream.close(false);
        }
        self.tls = None;
        self.open()
    }

    /// Lê uma resposta se ela chegar em `wait`.
    fn peek_reply(&mut self, wait: Duration) -> Option<Reply> {
        let ctrl = self.ctrl.as_mut()?;
        let _ = ctrl.stream.set_timeout(Some(wait));
        let reply = ctrl.read_reply();
        let _ = ctrl.stream.set_timeout(Some(self.cfg.timeout));
        match reply {
            Ok(reply) => Some(reply),
            Err(error) if error.kind() == io::ErrorKind::TimedOut => None,
            Err(_) => {
                self.ctrl = None;
                None
            }
        }
    }

    fn stash_reply(&mut self, reply: Reply) {
        if let Some(ctrl) = self.ctrl.as_mut() {
            ctrl.stash(reply);
        }
    }

    /// Lê a resposta final de uma transferência: `226` ou `250`.
    pub(super) fn finish_transfer(&mut self, action: &str) -> io::Result<()> {
        let reply = loop {
            let reply = self.read_reply()?;
            if !reply.preliminary() {
                break reply;
            }
        };
        if reply.code == 226 || reply.code == 250 {
            Ok(())
        } else {
            Err(reply_error(&reply, action, ""))
        }
    }

    /// Roda um comando que devolve os dados numa conexão (`LIST`, `MLSD`) e junta tudo.
    pub(super) fn fetch(
        &mut self,
        command: &str,
        action: &str,
        target: &str,
    ) -> io::Result<Vec<u8>> {
        let mut retried = false;
        loop {
            let mut stream = self.open_stream(command, 0, action, target)?;
            let mut bytes = Vec::new();
            let read = stream.read_to_end(&mut bytes);
            stream.close(false);
            let done = match read {
                Ok(_) => self.finish_transfer(action),
                Err(error) => {
                    // Lê a resposta do servidor (o motivo real) antes de desistir.
                    let _ = self.finish_transfer(action);
                    Err(error)
                }
            };
            match done {
                Ok(()) => return Ok(bytes),
                Err(error)
                    if !retried
                        && matches!(reply_code(&error), Some(425 | 522 | 534))
                        && self.can_fall_back() =>
                {
                    retried = true;
                    self.fall_back()?;
                }
                Err(error) => return Err(error),
            }
        }
    }

    /// Cancela uma transferência em andamento: fecha os dados, manda `ABOR` e lê as respostas.
    /// Se o servidor não responder, a conexão de controle é descartada (a próxima operação
    /// reabre).
    pub(super) fn abort(&mut self, data: Stream) {
        data.tcp().shutdown(std::net::Shutdown::Both).ok();
        drop(data);
        let Some(ctrl) = self.ctrl.as_mut() else {
            return;
        };
        let _ = ctrl.stream.set_timeout(Some(ESPERA_ABOR));
        let mut clean = ctrl.send("ABOR").is_ok();
        // O servidor responde a transferência (426, ou 226 se já tinha acabado) e o ABOR (226).
        for _ in 0..2 {
            if !clean {
                break;
            }
            clean = ctrl.read_reply().is_ok();
        }
        if clean {
            let _ = ctrl.stream.set_timeout(Some(self.cfg.timeout));
        } else {
            self.ctrl = None;
        }
    }
}

/// Um arquivo sendo baixado.
pub(super) struct FtpDownload<'a> {
    ftp: &'a mut Ftp,
    data: Option<Stream>,
    done: bool,
}

impl<'a> FtpDownload<'a> {
    pub(super) fn new(ftp: &'a mut Ftp, data: Stream) -> Self {
        Self {
            ftp,
            data: Some(data),
            done: false,
        }
    }
}

impl Read for FtpDownload<'_> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let Some(data) = self.data.as_mut() else {
            return Ok(0);
        };
        let read = data.read(buf)?;
        if read == 0 {
            self.done = true;
        }
        Ok(read)
    }
}

impl Download for FtpDownload<'_> {
    fn finish(mut self: Box<Self>) -> io::Result<()> {
        let Some(data) = self.data.take() else {
            return Ok(());
        };
        if self.done {
            data.close(false);
            self.ftp.finish_transfer("ler")
        } else {
            // Fechou antes do fim: é cancelar.
            self.ftp.abort(data);
            Ok(())
        }
    }
}

impl Drop for FtpDownload<'_> {
    fn drop(&mut self) {
        if let Some(data) = self.data.take() {
            self.ftp.abort(data);
        }
    }
}

/// Um arquivo sendo enviado.
pub(super) struct FtpUpload<'a> {
    ftp: &'a mut Ftp,
    data: Option<Stream>,
}

impl<'a> FtpUpload<'a> {
    pub(super) fn new(ftp: &'a mut Ftp, data: Stream) -> Self {
        Self {
            ftp,
            data: Some(data),
        }
    }
}

impl Write for FtpUpload<'_> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        match self.data.as_mut() {
            Some(data) => data.write(buf),
            None => Err(io::Error::from(io::ErrorKind::BrokenPipe)),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        match self.data.as_mut() {
            Some(data) => data.flush(),
            None => Ok(()),
        }
    }
}

impl Upload for FtpUpload<'_> {
    fn finish(mut self: Box<Self>) -> io::Result<()> {
        let Some(mut data) = self.data.take() else {
            return Ok(());
        };
        let flushed = data.flush();
        // Lê até o `close_notify` do servidor: no TLS 1.3 chegam ali os tickets da próxima retomada.
        data.close(true);
        let reply = self.ftp.finish_transfer("gravar em");
        flushed?;
        reply
    }
}

impl Drop for FtpUpload<'_> {
    fn drop(&mut self) {
        if let Some(data) = self.data.take() {
            self.ftp.abort(data);
        }
    }
}

#[cfg(test)]
mod testes {
    use super::*;

    #[test]
    fn pasv_le_o_endereco() {
        let (ip, porta) = parse_pasv("Entering Passive Mode (192,168,0,7,195,80).").unwrap();
        assert_eq!(ip, std::net::Ipv4Addr::new(192, 168, 0, 7));
        assert_eq!(porta, 195 * 256 + 80);
        let (_, porta) = parse_pasv("227 Passive Mode 10,0,0,1,4,1").unwrap();
        assert_eq!(porta, 1025);
        assert!(parse_pasv("227 sem números").is_none());
        assert!(parse_pasv("227 (1,2,3,4,5)").is_none());
    }

    #[test]
    fn epsv_le_a_porta() {
        assert_eq!(
            parse_epsv("229 Entering Extended Passive Mode (|||6446|)"),
            Some(6446)
        );
        assert_eq!(parse_epsv("229 ok (!!!2121!)"), Some(2121));
        assert_eq!(parse_epsv("229 sem parênteses"), None);
    }

    #[test]
    fn enderecos_particulares() {
        assert!(is_private("10.0.0.1".parse().unwrap()));
        assert!(is_private("192.168.1.5".parse().unwrap()));
        assert!(is_private("127.0.0.1".parse().unwrap()));
        assert!(is_private("0.0.0.0".parse().unwrap()));
        assert!(!is_private("8.8.8.8".parse().unwrap()));
    }
}
