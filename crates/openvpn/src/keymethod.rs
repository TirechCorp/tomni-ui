//! A troca de chaves "key-method 2": a primeira mensagem de controle depois do TLS.
//!
//! Com o TLS de pé, cada lado manda uma mensagem binária por dentro dele. É aqui que o cliente
//! entrega usuário e senha, as opções que acha que a conexão tem e as suas capacidades
//! (`peer info`), e que os dois lados trocam os números aleatórios da PRF (veja [`crate::prf`]).
//!
//! # Formato
//!
//! ```text
//! cliente:  u32 0 · u8 2 · pre_master (48) · random1 (32) · random2 (32)
//!           · opções · usuário · senha · peer info
//! servidor: u32 0 · u8 2 · random1 (32) · random2 (32)
//!           · opções · usuário · senha · [peer info]
//! ```
//!
//! Cada texto é um `u16` com o tamanho, **contando o NUL final**, e os bytes terminados em NUL.
//! Um texto ausente é só o tamanho 0. O cliente manda o `pre_master` sempre, mesmo quando as
//! chaves do canal de dados saem do exportador do TLS, porque essa escolha só chega depois,
//! no `PUSH_REPLY`. O servidor de várias conexões não manda `peer info`.

use std::fmt;

use zeroize::Zeroizing;

use crate::prf::{PRE_MASTER_LEN, RANDOM_LEN};

/// O método de troca de chaves (o único que existe hoje).
pub const KEY_METHOD: u8 = 2;

/// O maior tamanho do texto de opções (formato do protocolo).
pub const MAX_OPTIONS_LEN: usize = 512;

/// Os bits do `IV_PROTO` do `peer info`: o que este cliente sabe fazer.
pub mod proto_flags {
    /// Cabeçalho de dados `P_DATA_V2`, com `peer-id`.
    pub const DATA_V2: u32 = 1 << 1;
    /// O cliente manda `PUSH_REQUEST` sozinho, sem esperar o servidor.
    pub const REQUEST_PUSH: u32 = 1 << 2;
    /// Entende as chaves exportadas do TLS (`key-derivation tls-ekm`).
    pub const TLS_KEY_EXPORT: u32 = 1 << 3;
    /// Entende `AUTH_PENDING` com a palavra-chave `timeout`.
    pub const AUTH_PENDING_KW: u32 = 1 << 4;
    /// Entende a opção `dns` do `PUSH_REPLY` (a versão 2).
    pub const DNS_OPTION_V2: u32 = 1 << 11;
    /// Manda e entende o aviso de saída (`explicit-exit-notify`).
    pub const CC_EXIT_NOTIFY: u32 = 1 << 7;
    /// Entende `AUTH_FAILED,TEMP`.
    pub const AUTH_FAIL_TEMP: u32 = 1 << 8;
}

/// O que pode dar errado ao ler a mensagem.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeyMethodError {
    /// A mensagem ainda não chegou inteira.
    Incomplete,
    /// O começo não é `u32 0`.
    BadHeader,
    /// O método não é o 2.
    BadMethod(u8),
    /// Um dos textos não é UTF-8 ou não termina em NUL.
    BadString,
    /// O texto de opções passa do limite do protocolo.
    OptionsTooLong(usize),
}

impl fmt::Display for KeyMethodError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Incomplete => f.write_str("a troca de chaves ainda está incompleta"),
            Self::BadHeader => {
                f.write_str("a troca de chaves do servidor tem um cabeçalho inválido")
            }
            Self::BadMethod(m) => write!(f, "o servidor usa o key-method {m}; só o 2 é suportado"),
            Self::BadString => f.write_str("a troca de chaves tem um texto inválido"),
            Self::OptionsTooLong(n) => write!(
                f,
                "o texto de opções tem {n} bytes, mais que o limite de {MAX_OPTIONS_LEN} do protocolo"
            ),
        }
    }
}

impl std::error::Error for KeyMethodError {}

fn put_string(out: &mut Vec<u8>, text: Option<&str>) {
    match text {
        None => out.extend_from_slice(&0u16.to_be_bytes()),
        Some(t) => {
            // O texto do usuário e da senha cabe nos 16 bits do protocolo; o que passar disso é cortado
            // pelo chamador (que valida) e aqui vira erro de programação.
            let len = u16::try_from(t.len() + 1).expect("texto maior que 65535 bytes");
            out.extend_from_slice(&len.to_be_bytes());
            out.extend_from_slice(t.as_bytes());
            out.push(0);
        }
    }
}

/// Um leitor que sabe distinguir "faltam bytes" de "está errado".
struct Reader<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], KeyMethodError> {
        let fim = self.pos.checked_add(n).ok_or(KeyMethodError::Incomplete)?;
        let parte = self
            .bytes
            .get(self.pos..fim)
            .ok_or(KeyMethodError::Incomplete)?;
        self.pos = fim;
        Ok(parte)
    }

    fn array<const N: usize>(&mut self) -> Result<[u8; N], KeyMethodError> {
        let mut a = [0u8; N];
        a.copy_from_slice(self.take(N)?);
        Ok(a)
    }

    /// `Ok(None)` para o texto ausente (tamanho 0).
    fn string(&mut self) -> Result<Option<Zeroizing<String>>, KeyMethodError> {
        let len = usize::from(u16::from_be_bytes(self.array()?));
        if len == 0 {
            return Ok(None);
        }
        let bruto = self.take(len)?;
        let (&ultimo, texto) = bruto.split_last().ok_or(KeyMethodError::BadString)?;
        if ultimo != 0 {
            return Err(KeyMethodError::BadString);
        }
        let texto = std::str::from_utf8(texto).map_err(|_| KeyMethodError::BadString)?;
        Ok(Some(Zeroizing::new(texto.to_owned())))
    }

    fn header(&mut self) -> Result<(), KeyMethodError> {
        if self.array::<4>()? != [0; 4] {
            return Err(KeyMethodError::BadHeader);
        }
        let [metodo] = self.array::<1>()?;
        if metodo != KEY_METHOD {
            return Err(KeyMethodError::BadMethod(metodo));
        }
        Ok(())
    }
}

fn texto_ou_vazio(t: Option<Zeroizing<String>>) -> Zeroizing<String> {
    t.unwrap_or_default()
}

/// A mensagem do cliente.
#[derive(Clone)]
pub struct ClientKeyExchange {
    /// O segredo de 48 bytes da PRF.
    pub pre_master: Zeroizing<[u8; PRE_MASTER_LEN]>,
    /// O primeiro aleatório.
    pub random1: [u8; RANDOM_LEN],
    /// O segundo aleatório.
    pub random2: [u8; RANDOM_LEN],
    /// O texto de opções (veja [`options_string`]).
    pub options: String,
    /// O usuário (ou o usuário do `auth-token`).
    pub username: Option<Zeroizing<String>>,
    /// A senha (ou o `auth-token`).
    pub password: Option<Zeroizing<String>>,
    /// As capacidades do cliente (veja [`PeerInfo`]).
    pub peer_info: String,
}

impl fmt::Debug for ClientKeyExchange {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ClientKeyExchange")
            .field("options", &self.options)
            .field("peer_info", &self.peer_info)
            .finish_non_exhaustive()
    }
}

impl ClientKeyExchange {
    /// Sorteia os números aleatórios e monta a mensagem.
    pub fn random(
        options: String,
        username: Option<Zeroizing<String>>,
        password: Option<Zeroizing<String>>,
        peer_info: String,
    ) -> ClientKeyExchange {
        let mut pre_master = Zeroizing::new([0u8; PRE_MASTER_LEN]);
        let mut random1 = [0u8; RANDOM_LEN];
        let mut random2 = [0u8; RANDOM_LEN];
        // Sem aleatoriedade do sistema não há como negociar com segurança.
        getrandom::fill(&mut pre_master[..]).expect("o sistema não forneceu números aleatórios");
        getrandom::fill(&mut random1).expect("o sistema não forneceu números aleatórios");
        getrandom::fill(&mut random2).expect("o sistema não forneceu números aleatórios");
        ClientKeyExchange {
            pre_master,
            random1,
            random2,
            options,
            username,
            password,
            peer_info,
        }
    }

    /// Os bytes que vão por dentro do TLS.
    pub fn encode(&self) -> Result<Zeroizing<Vec<u8>>, KeyMethodError> {
        if self.options.len() + 1 > MAX_OPTIONS_LEN {
            return Err(KeyMethodError::OptionsTooLong(self.options.len() + 1));
        }
        let mut out = Zeroizing::new(Vec::with_capacity(256));
        out.extend_from_slice(&[0, 0, 0, 0, KEY_METHOD]);
        out.extend_from_slice(&self.pre_master[..]);
        out.extend_from_slice(&self.random1);
        out.extend_from_slice(&self.random2);
        put_string(&mut out, Some(&self.options));
        put_string(&mut out, self.username.as_ref().map(|s| s.as_str()));
        put_string(&mut out, self.password.as_ref().map(|s| s.as_str()));
        put_string(&mut out, Some(&self.peer_info));
        Ok(out)
    }

    /// Lê a mensagem do cliente (o servidor de teste usa). Devolve também quantos bytes usou.
    pub fn parse(bytes: &[u8]) -> Result<(ClientKeyExchange, usize), KeyMethodError> {
        let mut r = Reader { bytes, pos: 0 };
        r.header()?;
        let pre_master = Zeroizing::new(r.array::<PRE_MASTER_LEN>()?);
        let random1 = r.array()?;
        let random2 = r.array()?;
        let options = texto_ou_vazio(r.string()?).to_string();
        let username = r.string()?;
        let password = r.string()?;
        let peer_info = texto_ou_vazio(r.string()?).to_string();
        Ok((
            ClientKeyExchange {
                pre_master,
                random1,
                random2,
                options,
                username,
                password,
                peer_info,
            },
            r.pos,
        ))
    }
}

/// A mensagem do servidor.
#[derive(Clone)]
pub struct ServerKeyExchange {
    /// O primeiro aleatório.
    pub random1: [u8; RANDOM_LEN],
    /// O segundo aleatório.
    pub random2: [u8; RANDOM_LEN],
    /// O texto de opções do servidor.
    pub options: String,
    /// O `peer info`, se o servidor mandou.
    pub peer_info: Option<String>,
}

impl fmt::Debug for ServerKeyExchange {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ServerKeyExchange")
            .field("options", &self.options)
            .field("peer_info", &self.peer_info)
            .finish_non_exhaustive()
    }
}

impl ServerKeyExchange {
    /// Sorteia os números aleatórios e monta a mensagem.
    pub fn random(options: String, peer_info: Option<String>) -> ServerKeyExchange {
        let mut random1 = [0u8; RANDOM_LEN];
        let mut random2 = [0u8; RANDOM_LEN];
        getrandom::fill(&mut random1).expect("o sistema não forneceu números aleatórios");
        getrandom::fill(&mut random2).expect("o sistema não forneceu números aleatórios");
        ServerKeyExchange {
            random1,
            random2,
            options,
            peer_info,
        }
    }

    /// Os bytes que vão por dentro do TLS.
    pub fn encode(&self) -> Result<Vec<u8>, KeyMethodError> {
        if self.options.len() + 1 > MAX_OPTIONS_LEN {
            return Err(KeyMethodError::OptionsTooLong(self.options.len() + 1));
        }
        let mut out = Vec::with_capacity(160);
        out.extend_from_slice(&[0, 0, 0, 0, KEY_METHOD]);
        out.extend_from_slice(&self.random1);
        out.extend_from_slice(&self.random2);
        put_string(&mut out, Some(&self.options));
        put_string(&mut out, None);
        put_string(&mut out, None);
        if let Some(info) = &self.peer_info {
            put_string(&mut out, Some(info));
        }
        Ok(out)
    }

    /// Lê a mensagem do servidor e devolve também quantos bytes usou.
    ///
    /// O `peer info` é opcional e a mensagem não diz se ele vem. Como o que sobra depois dele
    /// pode ser a próxima mensagem de controle (um texto terminado em NUL, nunca um `u16`
    /// seguido de `IV_...`), só se reconhece como `peer info` um texto inteiro que comece com
    /// `IV_` ou `UV_`.
    pub fn parse(bytes: &[u8]) -> Result<(ServerKeyExchange, usize), KeyMethodError> {
        let mut r = Reader { bytes, pos: 0 };
        r.header()?;
        let random1 = r.array()?;
        let random2 = r.array()?;
        let options = texto_ou_vazio(r.string()?).to_string();
        // Usuário e senha existem no formato, mas o servidor não os usa.
        r.string()?;
        r.string()?;
        let mut peer_info = None;
        let antes = r.pos;
        if let Ok(Some(texto)) = r.string()
            && (texto.starts_with("IV_") || texto.starts_with("UV_"))
        {
            peer_info = Some(texto.to_string());
        } else {
            r.pos = antes;
        }
        Ok((
            ServerKeyExchange {
                random1,
                random2,
                options,
                peer_info,
            },
            r.pos,
        ))
    }
}

/// Os parâmetros que entram no texto de opções.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OptionsParams<'a> {
    /// `tun` ou `tap`.
    pub dev_type: &'a str,
    /// O `link-mtu`.
    pub link_mtu: u32,
    /// O `tun-mtu`.
    pub tun_mtu: u32,
    /// O protocolo no nome do OpenVPN: `UDPv4`, `UDPv6`, `TCPv4_CLIENT` ou `TCPv6_CLIENT`.
    pub proto: &'a str,
    /// A cifra do canal de dados, se há uma.
    pub cipher: Option<&'a str>,
    /// O digest do HMAC dos dados (`None` com AEAD).
    pub auth: Option<&'a str>,
    /// O tamanho da chave da cifra, em bits.
    pub keysize: Option<u32>,
}

/// O texto de opções que o cliente declara. O servidor só compara com as dele e avisa se
/// diferem; ele não recusa a conexão por isso.
pub fn options_string(p: &OptionsParams<'_>) -> String {
    let mut s = format!(
        "V4,dev-type {},link-mtu {},tun-mtu {},proto {}",
        p.dev_type, p.link_mtu, p.tun_mtu, p.proto
    );
    if let Some(c) = p.cipher {
        s.push_str(&format!(",cipher {c}"));
    }
    if let Some(a) = p.auth {
        s.push_str(&format!(",auth {a}"));
    }
    if let Some(k) = p.keysize {
        s.push_str(&format!(",keysize {k}"));
    }
    s.push_str(",key-method 2,tls-client");
    s
}

/// As capacidades que o cliente anuncia ao servidor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PeerInfo {
    /// A versão (`IV_VER`).
    pub version: String,
    /// O sistema: `linux`, `win`, `mac`...
    pub platform: String,
    /// Os bits de [`proto_flags`].
    pub proto: u32,
    /// As cifras do canal de dados que o cliente aceita, em ordem de preferência.
    pub ciphers: Vec<String>,
    /// O programa e a versão (`IV_GUI_VER`), se for informar.
    pub gui_version: Option<String>,
    /// O `tun-mtu` pretendido (`IV_MTU`), se o servidor puder usá-lo.
    pub mtu: Option<u32>,
}

impl PeerInfo {
    /// O texto: uma linha `CHAVE=valor` por capacidade, cada uma terminada em quebra de linha.
    pub fn to_text(&self) -> String {
        let mut s = String::new();
        s.push_str(&format!("IV_VER={}\n", self.version));
        s.push_str(&format!("IV_PLAT={}\n", self.platform));
        s.push_str("IV_TCPNL=1\n");
        s.push_str(&format!("IV_PROTO={}\n", self.proto));
        if !self.ciphers.is_empty() {
            s.push_str(&format!("IV_CIPHERS={}\n", self.ciphers.join(":")));
        }
        if let Some(g) = &self.gui_version {
            s.push_str(&format!("IV_GUI_VER={g}\n"));
        }
        if let Some(m) = self.mtu {
            s.push_str(&format!("IV_MTU={m}\n"));
        }
        s
    }
}

/// Lê um `peer info` em pares chave/valor, na ordem. Linhas sem `=` são ignoradas.
pub fn parse_peer_info(text: &str) -> Vec<(String, String)> {
    text.lines()
        .filter_map(|l| l.split_once('=').map(|(k, v)| (k.to_owned(), v.to_owned())))
        .collect()
}

#[cfg(test)]
mod testes {
    use super::*;

    fn cliente() -> ClientKeyExchange {
        ClientKeyExchange::random(
            options_string(&OptionsParams {
                dev_type: "tun",
                link_mtu: 1549,
                tun_mtu: 1500,
                proto: "UDPv4",
                cipher: Some("AES-256-GCM"),
                auth: None,
                keysize: Some(256),
            }),
            Some(Zeroizing::new("maria".into())),
            Some(Zeroizing::new("s3nh@ çom acento".into())),
            PeerInfo {
                version: "tomni-ui 0.1".into(),
                platform: "linux".into(),
                proto: proto_flags::DATA_V2 | proto_flags::TLS_KEY_EXPORT,
                ciphers: vec!["AES-256-GCM".into(), "CHACHA20-POLY1305".into()],
                gui_version: Some("Tirech Omni 1".into()),
                mtu: Some(1500),
            }
            .to_text(),
        )
    }

    #[test]
    fn o_cliente_vai_e_volta() {
        let c = cliente();
        let bytes = c.encode().unwrap();
        let (volta, usados) = ClientKeyExchange::parse(&bytes).unwrap();
        assert_eq!(usados, bytes.len());
        assert_eq!(*volta.pre_master, *c.pre_master);
        assert_eq!(volta.random1, c.random1);
        assert_eq!(volta.random2, c.random2);
        assert_eq!(volta.options, c.options);
        assert_eq!(volta.username.as_deref().map(String::as_str), Some("maria"));
        assert_eq!(
            volta.password.as_deref().map(String::as_str),
            Some("s3nh@ çom acento")
        );
        assert_eq!(volta.peer_info, c.peer_info);
    }

    #[test]
    fn formato_dos_bytes_do_cliente() {
        let mut c = cliente();
        c.options = "O".into();
        c.username = None;
        c.password = None;
        c.peer_info = "IV_VER=1\n".into();
        let b = c.encode().unwrap();
        assert_eq!(&b[..5], &[0, 0, 0, 0, 2]);
        assert_eq!(&b[5..53], &c.pre_master[..]);
        assert_eq!(&b[53..85], &c.random1);
        assert_eq!(&b[85..117], &c.random2);
        // opções: tamanho 2 (com o NUL), "O", NUL; depois dois textos ausentes (tamanho 0).
        assert_eq!(&b[117..121], &[0, 2, b'O', 0]);
        assert_eq!(&b[121..123], &[0, 0]);
        assert_eq!(&b[123..125], &[0, 0]);
        // peer info: 9 bytes + NUL = 10.
        assert_eq!(&b[125..127], &[0, 10]);
        assert_eq!(&b[127..], b"IV_VER=1\n\0");
    }

    #[test]
    fn cliente_cortado_em_qualquer_ponto_e_incompleto() {
        let bytes = cliente().encode().unwrap();
        for corte in 0..bytes.len() {
            assert_eq!(
                ClientKeyExchange::parse(&bytes[..corte]).unwrap_err(),
                KeyMethodError::Incomplete,
                "corte em {corte}"
            );
        }
    }

    #[test]
    fn cabecalho_e_metodo_errados() {
        let mut b = cliente().encode().unwrap().to_vec();
        b[4] = 1;
        assert_eq!(
            ClientKeyExchange::parse(&b).unwrap_err(),
            KeyMethodError::BadMethod(1)
        );
        b[4] = 2;
        b[0] = 1;
        assert_eq!(
            ClientKeyExchange::parse(&b).unwrap_err(),
            KeyMethodError::BadHeader
        );
    }

    #[test]
    fn texto_sem_nul_ou_fora_do_utf8_e_recusado() {
        let mut s = ServerKeyExchange::random("V4".into(), None)
            .encode()
            .unwrap();
        // Estraga o NUL final do texto de opções (tamanho 3: "V4" + NUL).
        let pos = s.windows(3).position(|w| w == b"V4\0").unwrap() + 2;
        s[pos] = b'x';
        assert_eq!(
            ServerKeyExchange::parse(&s).unwrap_err(),
            KeyMethodError::BadString
        );
        let mut s = ServerKeyExchange::random("V4".into(), None)
            .encode()
            .unwrap();
        let pos = s.windows(3).position(|w| w == b"V4\0").unwrap();
        s[pos] = 0xFF;
        assert_eq!(
            ServerKeyExchange::parse(&s).unwrap_err(),
            KeyMethodError::BadString
        );
    }

    #[test]
    fn o_servidor_vai_e_volta_com_e_sem_peer_info() {
        for info in [None, Some("IV_VER=2.6\nIV_PLAT=linux\n".to_string())] {
            let s = ServerKeyExchange::random("V4,dev-type tun".into(), info.clone());
            let bytes = s.encode().unwrap();
            let (volta, usados) = ServerKeyExchange::parse(&bytes).unwrap();
            assert_eq!(usados, bytes.len());
            assert_eq!(volta.random1, s.random1);
            assert_eq!(volta.random2, s.random2);
            assert_eq!(volta.options, s.options);
            assert_eq!(volta.peer_info, info);
        }
    }

    #[test]
    fn a_mensagem_seguinte_depois_do_servidor_nao_vira_peer_info() {
        let s = ServerKeyExchange::random("V4".into(), None);
        let mut bytes = s.encode().unwrap();
        let tamanho = bytes.len();
        bytes.extend_from_slice(b"PUSH_REPLY,route 10.0.0.0\0");
        let (volta, usados) = ServerKeyExchange::parse(&bytes).unwrap();
        assert_eq!(volta.peer_info, None);
        assert_eq!(usados, tamanho);
    }

    #[test]
    fn servidor_sem_usuario_e_senha_no_fim_e_incompleto() {
        let bytes = ServerKeyExchange::random("V4".into(), None)
            .encode()
            .unwrap();
        for corte in 0..bytes.len() {
            assert_eq!(
                ServerKeyExchange::parse(&bytes[..corte]).unwrap_err(),
                KeyMethodError::Incomplete,
                "corte em {corte}"
            );
        }
    }

    #[test]
    fn opcoes_longas_demais_sao_recusadas() {
        let mut c = cliente();
        c.options = "x".repeat(MAX_OPTIONS_LEN);
        assert_eq!(
            c.encode().unwrap_err(),
            KeyMethodError::OptionsTooLong(MAX_OPTIONS_LEN + 1)
        );
        c.options = "x".repeat(MAX_OPTIONS_LEN - 1);
        assert!(c.encode().is_ok());
    }

    #[test]
    fn texto_de_opcoes() {
        let s = options_string(&OptionsParams {
            dev_type: "tun",
            link_mtu: 1549,
            tun_mtu: 1500,
            proto: "TCPv4_CLIENT",
            cipher: Some("AES-256-GCM"),
            auth: None,
            keysize: Some(256),
        });
        assert_eq!(
            s,
            "V4,dev-type tun,link-mtu 1549,tun-mtu 1500,proto TCPv4_CLIENT,cipher AES-256-GCM,keysize 256,key-method 2,tls-client"
        );
        let s = options_string(&OptionsParams {
            dev_type: "tun",
            link_mtu: 1,
            tun_mtu: 1,
            proto: "UDPv4",
            cipher: Some("AES-256-CBC"),
            auth: Some("SHA1"),
            keysize: Some(256),
        });
        assert!(s.contains(",cipher AES-256-CBC,auth SHA1,keysize 256,"));
    }

    #[test]
    fn peer_info_em_texto_e_de_volta() {
        let info = PeerInfo {
            version: "tomni-ui 0.1".into(),
            platform: "win".into(),
            proto: proto_flags::DATA_V2 | proto_flags::REQUEST_PUSH,
            ciphers: vec!["AES-128-GCM".into(), "CHACHA20-POLY1305".into()],
            gui_version: None,
            mtu: None,
        };
        let texto = info.to_text();
        assert_eq!(
            texto,
            "IV_VER=tomni-ui 0.1\nIV_PLAT=win\nIV_TCPNL=1\nIV_PROTO=6\nIV_CIPHERS=AES-128-GCM:CHACHA20-POLY1305\n"
        );
        let pares = parse_peer_info(&texto);
        assert_eq!(pares[0], ("IV_VER".into(), "tomni-ui 0.1".into()));
        assert_eq!(pares[3], ("IV_PROTO".into(), "6".into()));
        assert_eq!(pares.len(), 5);
    }

    #[test]
    fn bits_do_iv_proto() {
        assert_eq!(proto_flags::DATA_V2, 2);
        assert_eq!(proto_flags::REQUEST_PUSH, 4);
        assert_eq!(proto_flags::TLS_KEY_EXPORT, 8);
        assert_eq!(proto_flags::AUTH_PENDING_KW, 16);
        assert_eq!(proto_flags::CC_EXIT_NOTIFY, 128);
        assert_eq!(proto_flags::AUTH_FAIL_TEMP, 256);
        assert_eq!(proto_flags::DNS_OPTION_V2, 2048);
    }

    #[test]
    fn o_debug_nao_mostra_segredo() {
        let texto = format!("{:?}", cliente());
        assert!(!texto.contains("s3nh"));
        assert!(!texto.contains("maria"));
    }
}
