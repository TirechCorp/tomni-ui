//! O mundo dos testes do motor: um servidor de teste numa thread própria, ligado ao cliente por
//! enlaces em memória (`MemoryLink`), com uma pilha de rede de verdade do outro lado (eco na
//! porta 7000 e um DNS na 53). O cliente é a `OpenVpn` inteira, usada como o app usa: pela ponte
//! SOCKS5.
#![allow(dead_code)]

use std::collections::VecDeque;
use std::future::pending;
use std::io::{self, Read, Write};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::{Notify, mpsc};
use tokio::task::LocalSet;
use tokio::time::sleep_until;
use tomni_ui_net::Proxy;
use tomni_ui_netstack::NetStack;
use tomni_ui_netstack::pipe::{PipeOptions, PipeTunnel, RawEnd};
use tomni_ui_openvpn::channel::Clock;
use tomni_ui_openvpn::config::Config;
use tomni_ui_openvpn::test_server::{Attempt, Decision, ServerEvent, ServerSession};
use tomni_ui_openvpn::transport::{Connecting, Connector, Link, MemoryLink, Target};
use tomni_ui_openvpn::{OpenVpn, OpenVpnBuilder, State};

use super::agora;
use super::rede::{empurra, perfil, servidor};

/// O endereço do servidor dentro da VPN: a pilha dele responde aqui.
pub const SERVIDOR: &str = "10.9.0.1";

pub type Regra = Arc<dyn Fn(&Attempt) -> Decision + Send + Sync>;
pub type Empurra = Arc<dyn Fn(usize) -> String + Send + Sync>;

/// Como o servidor se comporta.
#[derive(Clone)]
pub struct Cenario {
    pub tcp: bool,
    /// O `PUSH_REPLY` de cada conexão (recebe o número da conexão, de 1 em diante).
    pub push: Empurra,
    /// A decisão sobre cada tentativa de autenticação.
    pub regra: Regra,
    /// Os nomes que o DNS do servidor conhece.
    pub dns: Vec<(&'static str, [u8; 4])>,
}

impl Cenario {
    pub fn novo() -> Cenario {
        Cenario {
            tcp: false,
            push: Arc::new(|_| empurra("10.8.0.2", "")),
            regra: Arc::new(|_| Decision::Accept { token: None }),
            dns: Vec::new(),
        }
    }

    pub fn tcp(mut self) -> Cenario {
        self.tcp = true;
        self
    }

    pub fn push(mut self, f: impl Fn(usize) -> String + Send + Sync + 'static) -> Cenario {
        self.push = Arc::new(f);
        self
    }

    pub fn regra(mut self, f: impl Fn(&Attempt) -> Decision + Send + Sync + 'static) -> Cenario {
        self.regra = Arc::new(f);
        self
    }

    pub fn dns(mut self, nome: &'static str, ip: [u8; 4]) -> Cenario {
        self.dns.push((nome, ip));
        self
    }
}

enum Pedido {
    Texto(String),
    Ip(Vec<u8>),
    Resolve(Decision),
}

/// O que o servidor conta ao teste.
#[derive(Default)]
pub struct Estat {
    /// O servidor finge que a rede sumiu (não ouve nem fala).
    pub mudo: AtomicBool,
    /// Perde um datagrama a cada tantos, nos dois sentidos (0: nenhum).
    pub perda_cada: AtomicUsize,
    pub autenticados: AtomicUsize,
    pub chaves: AtomicUsize,
    pub saidas: AtomicU64,
    pub pings: AtomicU64,
    /// Os pacotes IP que chegaram do cliente.
    pub ip_do_cliente: AtomicU64,
    /// Os usuários e senhas de cada tentativa.
    pub tentativas: Mutex<Vec<(Option<String>, Option<String>)>>,
    pedidos: Mutex<VecDeque<Pedido>>,
    acorda: Notify,
}

impl Estat {
    pub fn n(&self, campo: &AtomicUsize) -> usize {
        campo.load(Ordering::SeqCst)
    }

    pub fn tentativas(&self) -> Vec<(Option<String>, Option<String>)> {
        self.tentativas
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn pedir(&self, pedido: Pedido) {
        self.pedidos
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push_back(pedido);
        self.acorda.notify_one();
    }

    /// Manda uma mensagem de controle ao cliente (`RESTART`, `HALT`...).
    pub fn mensagem(&self, texto: &str) {
        self.pedir(Pedido::Texto(texto.to_owned()));
    }

    /// Manda um pacote IP ao cliente.
    pub fn ip(&self, pacote: Vec<u8>) {
        self.pedir(Pedido::Ip(pacote));
    }

    pub fn resolve(&self, decisao: Decision) {
        self.pedir(Pedido::Resolve(decisao));
    }
}

/// O conector que entrega ao motor um enlace em memória e a outra ponta ao servidor.
pub struct ConectorTeste {
    tx: mpsc::UnboundedSender<MemoryLink>,
    pub conexoes: Arc<AtomicUsize>,
    /// Quantas aberturas ainda falham antes de uma funcionar.
    pub falhas: Arc<AtomicUsize>,
    /// Os `remote` que o motor tentou, na ordem.
    pub alvos: Arc<Mutex<Vec<String>>>,
}

impl Connector for ConectorTeste {
    fn connect<'a>(&'a mut self, target: Target<'a>) -> Connecting<'a> {
        Box::pin(async move {
            self.alvos
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push(format!("{}:{}", target.remote.host, target.remote.port));
            if self
                .falhas
                .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |n| n.checked_sub(1))
                .is_ok()
            {
                return Err(io::Error::new(
                    io::ErrorKind::ConnectionRefused,
                    "conexão recusada",
                ));
            }
            self.conexoes.fetch_add(1, Ordering::SeqCst);
            let (cliente, servidor) = MemoryLink::pair();
            let _ = self.tx.send(servidor);
            Ok(Box::new(cliente) as Box<dyn Link>)
        })
    }
}

pub struct Mundo {
    pub estat: Arc<Estat>,
    pub conexoes: Arc<AtomicUsize>,
    pub falhas: Arc<AtomicUsize>,
    pub alvos: Arc<Mutex<Vec<String>>>,
    tx: Option<mpsc::UnboundedSender<MemoryLink>>,
}

impl Mundo {
    pub fn novo(cenario: Cenario) -> Mundo {
        let estat = Arc::new(Estat::default());
        let (tx, mut rx) = mpsc::unbounded_channel::<MemoryLink>();
        let dentro = estat.clone();
        std::thread::Builder::new()
            .name("servidor-de-teste".into())
            .spawn(move || {
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .expect("runtime do servidor");
                let local = LocalSet::new();
                runtime.block_on(local.run_until(async move {
                    // A pilha do servidor dura o mundo todo (como o servidor de verdade): as
                    // reconexões do cliente chegam sempre à mesma.
                    let (tunel, mut cru) = PipeTunnel::raw(
                        &format!("{SERVIDOR}/8"),
                        PipeOptions {
                            mtu: 1500,
                            ..PipeOptions::default()
                        },
                    );
                    let pilha = NetStack::spawn(tunel);
                    eco(&pilha).await;
                    dns(&pilha, &cenario.dns).await;
                    let mut n = 0;
                    while let Some(link) = rx.recv().await {
                        n += 1;
                        servir(link, &cenario, n, &dentro, &mut cru).await;
                    }
                    pilha.shutdown().await;
                }));
            })
            .expect("thread do servidor");
        Mundo {
            estat,
            conexoes: Arc::default(),
            falhas: Arc::default(),
            alvos: Arc::default(),
            tx: Some(tx),
        }
    }

    pub fn conector(&self) -> ConectorTeste {
        ConectorTeste {
            tx: self.tx.clone().expect("mundo vivo"),
            conexoes: self.conexoes.clone(),
            falhas: self.falhas.clone(),
            alvos: self.alvos.clone(),
        }
    }

    pub fn conexoes(&self) -> usize {
        self.conexoes.load(Ordering::SeqCst)
    }

    /// O cliente: o perfil de teste mais `extra`, já ligado ao mundo.
    pub fn abrir(&self, extra: &str) -> OpenVpn {
        self.abrir_com(perfil(extra), |b| b)
    }

    pub fn abrir_com(
        &self,
        config: Config,
        ajusta: impl FnOnce(OpenVpnBuilder) -> OpenVpnBuilder,
    ) -> OpenVpn {
        let builder = OpenVpnBuilder::new(config)
            .name("teste")
            .connector(self.conector())
            .clock(Clock::fixed(agora()));
        ajusta(builder).start().expect("a VPN abre")
    }
}

impl Drop for Mundo {
    fn drop(&mut self) {
        self.tx = None;
    }
}

async fn servir(
    mut link: MemoryLink,
    cenario: &Cenario,
    n: usize,
    estat: &Arc<Estat>,
    cru: &mut RawEnd,
) {
    let regra = cenario.regra.clone();
    let registro = estat.clone();
    let estat = estat.clone();
    let mut sessao = ServerSession::new(
        servidor(cenario.tcp, (cenario.push)(n)),
        move |a: &Attempt| {
            registro
                .tentativas
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push((a.username.clone(), a.password.clone()));
            regra(a)
        },
    )
    .expect("servidor de teste");
    let mut contagem = [0u64; 2];
    let mut perde = |estat: &Estat, lado: usize| {
        contagem[lado] += 1;
        let cada = estat.perda_cada.load(Ordering::SeqCst) as u64;
        estat.mudo.load(Ordering::SeqCst) || (cada > 0 && contagem[lado] % cada == 0)
    };
    loop {
        let agora = Instant::now();
        while let Some(datagrama) = sessao.poll_transmit(agora) {
            if !perde(&estat, 1) {
                link.push(datagrama);
            }
        }
        while let Some(ip) = sessao.poll_ip() {
            estat.ip_do_cliente.fetch_add(1, Ordering::SeqCst);
            cru.send(ip);
        }
        while let Some(evento) = sessao.poll_event() {
            match evento {
                ServerEvent::Authenticated { .. } => {
                    estat.autenticados.fetch_add(1, Ordering::SeqCst);
                }
                ServerEvent::DataKeys { .. } => {
                    estat.chaves.fetch_add(1, Ordering::SeqCst);
                }
                ServerEvent::Exit => {
                    estat.saidas.fetch_add(1, Ordering::SeqCst);
                }
                _ => {}
            }
        }
        estat.pings.store(sessao.pings_received(), Ordering::SeqCst);
        let proximo = sessao.next_timeout();
        tokio::select! {
            datagrama = link.recv() => match datagrama {
                Some(d) => {
                    if !perde(&estat, 0) {
                        sessao.handle_datagram(&d, Instant::now());
                    }
                }
                None => break,
            },
            ip = cru.recv() => match ip {
                Some(ip) => sessao.send_ip(&ip),
                None => break,
            },
            () = async {
                match proximo {
                    Some(at) => sleep_until(tokio::time::Instant::from_std(at)).await,
                    None => pending().await,
                }
            } => sessao.handle_timeout(Instant::now()),
            () = estat.acorda.notified() => {
                loop {
                    let pedido = estat
                        .pedidos
                        .lock()
                        .unwrap_or_else(PoisonError::into_inner)
                        .pop_front();
                    match pedido {
                        Some(Pedido::Texto(t)) => sessao.send_message(&t),
                        Some(Pedido::Ip(p)) => sessao.send_ip(&p),
                        Some(Pedido::Resolve(d)) => sessao.resolve_pending(d),
                        None => break,
                    }
                }
            }
        }
    }
}

/// O eco na porta 7000.
async fn eco(pilha: &NetStack) {
    let mut escuta = pilha.listen(7000).await.expect("escuta 7000");
    tokio::task::spawn_local(async move {
        while let Ok(mut conexao) = escuta.accept().await {
            tokio::task::spawn_local(async move {
                let (mut l, mut e) = tokio::io::split(&mut conexao);
                let _ = tokio::io::copy(&mut l, &mut e).await;
            });
        }
    });
}

/// Um DNS sobre TCP na porta 53, com a tabela dada (A apenas; o resto é NXDOMAIN).
async fn dns(pilha: &NetStack, tabela: &[(&'static str, [u8; 4])]) {
    let tabela: Vec<(String, [u8; 4])> = tabela
        .iter()
        .map(|(n, ip)| ((*n).to_owned(), *ip))
        .collect();
    let mut escuta = pilha.listen(53).await.expect("escuta 53");
    tokio::task::spawn_local(async move {
        while let Ok(mut conexao) = escuta.accept().await {
            let tabela = tabela.clone();
            tokio::task::spawn_local(async move {
                let mut tamanho = [0u8; 2];
                conexao.read_exact(&mut tamanho).await.ok()?;
                let mut consulta = vec![0u8; usize::from(u16::from_be_bytes(tamanho))];
                conexao.read_exact(&mut consulta).await.ok()?;
                let (mut pos, mut nome) = (12, Vec::new());
                while consulta[pos] != 0 {
                    let n = usize::from(consulta[pos]);
                    nome.push(
                        String::from_utf8_lossy(&consulta[pos + 1..pos + 1 + n]).into_owned(),
                    );
                    pos += 1 + n;
                }
                let fim = pos + 5;
                let nome = nome.join(".");
                let mut resposta = consulta[0..2].to_vec();
                match tabela.iter().find(|(n, _)| *n == nome) {
                    Some((_, ip)) => {
                        resposta.extend_from_slice(&[0x81, 0x80, 0, 1, 0, 1, 0, 0, 0, 0]);
                        resposta.extend_from_slice(&consulta[12..fim]);
                        resposta.extend_from_slice(&[0xC0, 12, 0, 1, 0, 1, 0, 0, 0, 60, 0, 4]);
                        resposta.extend_from_slice(ip);
                    }
                    None => {
                        resposta.extend_from_slice(&[0x81, 0x83, 0, 1, 0, 0, 0, 0, 0, 0]);
                        resposta.extend_from_slice(&consulta[12..fim]);
                    }
                }
                let mut quadro = (resposta.len() as u16).to_be_bytes().to_vec();
                quadro.extend_from_slice(&resposta);
                conexao.write_all(&quadro).await.ok()?;
                let _ = conexao.shutdown().await;
                Some(())
            });
        }
    });
}

// ---- o lado do cliente ----

/// Espera o estado satisfazer `serve` (até `segundos`); em falha, mostra o último.
pub fn esperar(vpn: &OpenVpn, segundos: u64, serve: impl Fn(&State) -> bool) -> State {
    let fim = Instant::now() + Duration::from_secs(segundos);
    loop {
        let estado = vpn.state();
        if serve(&estado) {
            return estado;
        }
        assert!(
            Instant::now() < fim,
            "o estado esperado não veio em {segundos} s; último: {estado:?}"
        );
        std::thread::sleep(Duration::from_millis(15));
    }
}

pub fn conectado(estado: &State) -> bool {
    matches!(estado, State::Connected { .. })
}

/// Espera até a condição valer.
pub fn ate(segundos: u64, o_que: &str, serve: impl Fn() -> bool) {
    let fim = Instant::now() + Duration::from_secs(segundos);
    while !serve() {
        assert!(
            Instant::now() < fim,
            "não aconteceu em {segundos} s: {o_que}"
        );
        std::thread::sleep(Duration::from_millis(15));
    }
}

/// Abre uma conexão ao `host:porta` de dentro da VPN, pela ponte SOCKS5.
pub fn abrir_pela_ponte(proxy: &Proxy, host: &str, porta: u16) -> io::Result<std::net::TcpStream> {
    tomni_ui_net::connect(host, porta, Some(proxy), Duration::from_secs(10))
}

/// Manda `dados` ao eco e devolve o que voltou.
pub fn eco_de(proxy: &Proxy, host: &str, porta: u16, dados: &[u8]) -> io::Result<Vec<u8>> {
    let mut s = abrir_pela_ponte(proxy, host, porta)?;
    s.set_read_timeout(Some(Duration::from_secs(30)))?;
    let mut escrita = s.try_clone()?;
    let enviado = dados.to_vec();
    let escritor = std::thread::spawn(move || escrita.write_all(&enviado));
    let mut volta = vec![0u8; dados.len()];
    s.read_exact(&mut volta)?;
    escritor.join().expect("escritor")?;
    Ok(volta)
}

/// Bytes de teste que não se repetem em blocos curtos.
pub fn bytes_de_teste(n: usize) -> Vec<u8> {
    let mut x: u32 = 0x1234_5678;
    (0..n)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            (x >> 24) as u8
        })
        .collect()
}

pub fn sha(dados: &[u8]) -> Vec<u8> {
    ring::digest::digest(&ring::digest::SHA256, dados)
        .as_ref()
        .to_vec()
}
