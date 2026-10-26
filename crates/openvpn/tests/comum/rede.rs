//! A rede de teste em memória: um cliente (`Session`) e um servidor de teste (`ServerSession`)
//! ligados por datagramas que o teste controla (perda, corte, TCP em pedaços).

use std::time::{Duration, Instant};

use tomni_ui_openvpn::auth::AuthState;
use tomni_ui_openvpn::channel::Clock;
use tomni_ui_openvpn::config::{AuthDigest, Config, Proto};
use tomni_ui_openvpn::control::ControlOptions;
use tomni_ui_openvpn::packet::{TcpDeframer, tcp_frame};
use tomni_ui_openvpn::pki::{Pki, PrivateKey, Wrap, parse_certificates};
use tomni_ui_openvpn::session::{Session, SessionEvent, SessionSettings};
use tomni_ui_openvpn::static_key::StaticKey;
use tomni_ui_openvpn::test_server::{Attempt, Decision, ServerEvent, ServerOptions, ServerSession};

use super::{agora, ler, pasta};

pub fn certs(nome: &str) -> Vec<rustls::pki_types::CertificateDer<'static>> {
    parse_certificates(&ler(nome)).unwrap_or_else(|e| panic!("{nome}: {e}"))
}

pub fn chave(nome: &str) -> PrivateKey {
    PrivateKey::from_pem(&ler(nome)).unwrap_or_else(|e| panic!("{nome}: {e}"))
}

/// O perfil de teste mais as linhas de `extra`.
pub fn perfil(extra: &str) -> Config {
    let texto = format!("{}\n{extra}\n", ler("tudo-embutido.ovpn"));
    Config::from_ovpn(&texto, Some(&pasta())).expect("perfil").0
}

/// As opções do cliente: o perfil de teste com `tls-auth`, `auth SHA256`, e as linhas de `extra`.
pub fn cliente(extra: &str, tcp: bool, auth: AuthState) -> (ControlOptions, SessionSettings) {
    let config = perfil(extra);
    let transporte = if tcp { Proto::Tcp } else { Proto::Udp };
    let controle =
        ControlOptions::from_config(&config, transporte, false, auth, Clock::fixed(agora()))
            .expect("opções do cliente");
    (controle, SessionSettings::from_config(&config, transporte))
}

/// O `PUSH_REPLY` padrão do servidor de teste, trocando o endereço do cliente e somando `extra`.
pub fn empurra(endereco: &str, extra: &str) -> String {
    let mut s = format!(
        "PUSH_REPLY,route 10.9.0.0 255.255.255.0,route-gateway 10.8.0.1,topology subnet,ifconfig {endereco} 255.255.255.0,peer-id 0,cipher AES-256-GCM,ping 10,ping-restart 60,key-derivation tls-ekm"
    );
    if !extra.is_empty() {
        s.push(',');
        s.push_str(extra);
    }
    s
}

/// As opções do servidor de teste (TLS-auth com `auth SHA256`, chaves pelo exportador).
pub fn servidor(tcp: bool, push: String) -> ServerOptions {
    let mut o = ServerOptions::new(
        certs("servidor.crt"),
        chave("servidor.key"),
        certs("ca.crt"),
        Clock::fixed(agora()),
    );
    o.auth_digest = AuthDigest::Sha256;
    let chave_estatica = StaticKey::from_pem(&ler("ta.key")).expect("ta.key");
    o.wrap = Pki {
        wrap: Wrap::TlsAuth,
        static_key: Some(chave_estatica),
        key_direction: Some(1),
        ..Pki::default()
    };
    if tcp {
        o.transport = Proto::Tcp;
    }
    o.ekm = true;
    o.push = vec![push];
    o
}

pub fn aceita(_: &Attempt) -> Decision {
    Decision::Accept { token: None }
}

type Perda = Box<dyn FnMut(bool, u64) -> bool>;

/// O cliente e o servidor de teste, ligados.
pub struct ParSessao {
    pub cliente: Session,
    pub servidor: ServerSession,
    pub t0: Instant,
    pub t: Instant,
    tcp: bool,
    pub eventos: Vec<SessionEvent>,
    pub eventos_s: Vec<ServerEvent>,
    /// Os pacotes IP que chegaram ao cliente.
    pub ip_cliente: Vec<Vec<u8>>,
    /// Os pacotes IP que chegaram ao servidor.
    pub ip_servidor: Vec<Vec<u8>>,
    /// Decide se um datagrama se perde: recebe "é do cliente" e quantos esse lado já enviou.
    pub perda: Option<Perda>,
    /// Corta a rede nos dois sentidos.
    pub corte: bool,
    contagem: [u64; 2],
    desquadrador_cliente: TcpDeframer,
    desquadrador_servidor: TcpDeframer,
    /// Quantos datagramas de dados (`P_DATA`) o cliente enviou.
    pub dados_enviados: u64,
}

impl ParSessao {
    pub fn novo(
        cliente: (ControlOptions, SessionSettings),
        servidor: ServerOptions,
        autenticar: impl FnMut(&Attempt) -> Decision + 'static,
    ) -> ParSessao {
        let t0 = Instant::now();
        let tcp = cliente.0.transport == Proto::Tcp;
        let cliente = Session::new(cliente.0, cliente.1, t0).expect("sessão do cliente");
        let servidor = ServerSession::new(servidor, autenticar).expect("servidor");
        ParSessao {
            cliente,
            servidor,
            t0,
            t: t0,
            tcp,
            eventos: Vec::new(),
            eventos_s: Vec::new(),
            ip_cliente: Vec::new(),
            ip_servidor: Vec::new(),
            perda: None,
            corte: false,
            contagem: [0, 0],
            desquadrador_cliente: TcpDeframer::new(),
            desquadrador_servidor: TcpDeframer::new(),
            dados_enviados: 0,
        }
    }

    fn entregar(&mut self, do_cliente: bool, pacote: Vec<u8>) {
        if do_cliente && pacote.first().is_some_and(|b| matches!(b >> 3, 6 | 9)) {
            self.dados_enviados += 1;
        }
        let lado = usize::from(do_cliente);
        self.contagem[lado] += 1;
        let n = self.contagem[lado];
        if self.corte {
            return;
        }
        if let Some(f) = self.perda.as_mut()
            && f(do_cliente, n)
        {
            return;
        }
        let chegaram = if self.tcp {
            let quadro = tcp_frame(&pacote).expect("quadro TCP");
            let d = if do_cliente {
                &mut self.desquadrador_servidor
            } else {
                &mut self.desquadrador_cliente
            };
            for pedaco in quadro.chunks(11) {
                d.push(pedaco);
            }
            let mut v = Vec::new();
            while let Some(p) = d.next_packet() {
                v.push(p);
            }
            v
        } else {
            vec![pacote]
        };
        for d in chegaram {
            if do_cliente {
                self.servidor.handle_datagram(&d, self.t);
            } else {
                self.cliente.handle_datagram(&d, self.t);
            }
        }
    }

    /// Passa tudo o que está pronto de um lado para o outro, até acalmar.
    pub fn drenar(&mut self) {
        for _ in 0..100_000 {
            let mut moveu = false;
            while let Some(p) = self.cliente.poll_transmit(self.t) {
                moveu = true;
                self.entregar(true, p);
            }
            while let Some(p) = self.servidor.poll_transmit(self.t) {
                moveu = true;
                self.entregar(false, p);
            }
            while let Some(e) = self.cliente.poll_event() {
                self.eventos.push(e);
            }
            while let Some(e) = self.servidor.poll_event() {
                self.eventos_s.push(e);
            }
            while let Some(p) = self.cliente.poll_ip() {
                self.ip_cliente.push(p);
            }
            while let Some(p) = self.servidor.poll_ip() {
                self.ip_servidor.push(p);
            }
            if !moveu {
                return;
            }
        }
        panic!("a drenagem não acalmou");
    }

    /// Roda até `parar` ser verdadeiro, o cliente acabar, ou passarem `limite_s` segundos.
    pub fn rodar(&mut self, limite_s: u64, parar: impl Fn(&ParSessao) -> bool) {
        let fim = self.t + Duration::from_secs(limite_s);
        for _ in 0..500_000 {
            self.drenar();
            if parar(self) || self.cliente.is_finished() {
                return;
            }
            let proximo = [self.cliente.next_timeout(), self.servidor.next_timeout()]
                .into_iter()
                .flatten()
                .min();
            match proximo {
                Some(p) if p <= fim => {
                    self.t = p.max(self.t);
                    self.cliente.handle_timeout(self.t);
                    self.servidor.handle_timeout(self.t);
                }
                _ => {
                    self.t = fim.max(self.t);
                    self.cliente.handle_timeout(self.t);
                    self.servidor.handle_timeout(self.t);
                    self.drenar();
                    return;
                }
            }
        }
        panic!("o laço de simulação não terminou");
    }

    /// Conecta: roda até o `Connected`.
    pub fn conectar(&mut self) {
        self.rodar(60, |p| p.conectado());
        assert!(
            self.conectado(),
            "não conectou: {:?} / servidor: {:?}",
            self.eventos,
            self.eventos_s
        );
    }

    pub fn conectado(&self) -> bool {
        self.eventos
            .iter()
            .any(|e| matches!(e, SessionEvent::Connected(_)))
    }

    pub fn conexao(&self) -> &tomni_ui_openvpn::session::Connected {
        self.eventos
            .iter()
            .find_map(|e| match e {
                SessionEvent::Connected(c) => Some(&**c),
                _ => None,
            })
            .expect("conectou")
    }

    pub fn trocas_de_chave(&self) -> usize {
        self.eventos
            .iter()
            .filter(|e| matches!(e, SessionEvent::Rekeyed))
            .count()
    }

    pub fn falha(&self) -> Option<&tomni_ui_openvpn::session::Failure> {
        self.eventos.iter().find_map(|e| match e {
            SessionEvent::Failed(f) => Some(f),
            _ => None,
        })
    }
}

/// Um pacote IPv4 qualquer (só o cabeçalho de versão e o tamanho valem para os testes da
/// sessão) com `n` bytes de carga que identificam o pacote.
pub fn pacote_ip(id: u32, n: usize) -> Vec<u8> {
    let total = 20 + n;
    let mut p = vec![0u8; total];
    p[0] = 0x45;
    p[2..4].copy_from_slice(&(total as u16).to_be_bytes());
    p[8] = 64;
    p[9] = 17;
    p[12..16].copy_from_slice(&[10, 8, 0, 2]);
    p[16..20].copy_from_slice(&[10, 9, 0, 5]);
    for (i, b) in p[20..].iter_mut().enumerate() {
        *b = (id as usize + i) as u8;
    }
    if n >= 4 {
        p[20..24].copy_from_slice(&id.to_be_bytes());
    }
    p
}

/// O id que `pacote_ip` gravou.
pub fn id_do_pacote(p: &[u8]) -> u32 {
    u32::from_be_bytes([p[20], p[21], p[22], p[23]])
}
