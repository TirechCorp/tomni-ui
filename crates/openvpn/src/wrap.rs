//! A proteção extra dos pacotes de controle: `tls-auth`, `tls-crypt` e `tls-crypt-v2`.
//!
//! Sem proteção, qualquer um que descubra a porta consegue começar uma negociação TLS com o
//! servidor. Com a chave estática de 2048 bits, o servidor descarta de graça tudo o que não vem
//! de quem a conhece. Só o canal de controle é protegido; os pacotes de dados nunca passam por
//! aqui.
//!
//! # `tls-auth`
//!
//! Um HMAC (o `auth` do `.ovpn`, SHA-1 por padrão) na frente do corpo. O pacote na rede:
//!
//! ```text
//! op|key_id (1) · session_id (8) · HMAC (tamanho do digest)
//! · packet_id (4) · tempo (4) · corpo (do n_acks em diante)
//! ```
//!
//! O HMAC cobre `packet_id · tempo · op|key_id · session_id · corpo`, nessa ordem, e usa os
//! primeiros bytes da parte de HMAC da metade da chave.
//!
//! # `tls-crypt`
//!
//! O corpo é cifrado (AES-256 em modo contador) e autenticado (HMAC-SHA-256):
//!
//! ```text
//! op|key_id (1) · session_id (8) · packet_id (4) · tempo (4) · etiqueta (32) · corpo cifrado
//! ```
//!
//! A etiqueta é `HMAC-SHA-256(op|key_id · session_id · packet_id · tempo · corpo em claro)` e os
//! 16 primeiros bytes dela são o IV da cifra. A chave de cifra são os 32 primeiros bytes da
//! parte de cifra da metade, e a de HMAC os 32 primeiros da parte de HMAC.
//!
//! # `tls-crypt-v2`
//!
//! Igual ao `tls-crypt`, com a chave `Kc` do cliente no lugar da chave estática. O primeiro
//! pacote (`P_CONTROL_HARD_RESET_CLIENT_V3`) leva, depois do pacote protegido, a chave `Kc`
//! embrulhada para o servidor (o `WKc`, sem cifra), que termina com o próprio tamanho.
//!
//! # O contador
//!
//! O `packet_id` começa em 1 e cresce a cada pacote enviado; o "tempo" é o instante (segundos
//! desde 1970) em que o remetente começou a sequência e não muda dentro dela. Ele é gerado na
//! hora de enviar, então uma retransmissão ganha um contador novo.

use std::fmt;

use aes::Aes256;
use ctr::Ctr128BE;
use ctr::cipher::{KeyIvInit, StreamCipher};
use hmac::{Hmac, KeyInit, Mac};
use sha1::Sha1;
use sha2::{Sha256, Sha384, Sha512};
use zeroize::Zeroizing;

use crate::config::AuthDigest;
use crate::packet::{ControlPacket, Opcode, PacketError, SESSION_ID_LEN, SessionId, classify};
use crate::replay::{ReplayError, ReplayWindow};
use crate::static_key::{Direction, KeyHalf, StaticKey};

/// O tamanho do contador mais o tempo, no pacote (formato do protocolo).
pub const REPLAY_ID_LEN: usize = 8;

/// O tamanho da etiqueta do `tls-crypt` (HMAC-SHA-256, formato do protocolo).
pub const TLS_CRYPT_TAG_LEN: usize = 32;

/// O tamanho do IV da cifra do `tls-crypt` (um bloco do AES).
const IV_LEN: usize = 16;

/// O tamanho da chave de cifra e da de HMAC do `tls-crypt` (AES-256 e SHA-256).
const TLS_CRYPT_KEY_LEN: usize = 32;

/// De que lado da conexão o wrapper está. O servidor usa as metades da chave ao contrário do
/// cliente.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    /// O cliente (o app).
    Client,
    /// O servidor (o servidor de teste).
    Server,
}

/// Por que um pacote não passou.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WrapError {
    /// O pacote é curto demais para o formato.
    Short,
    /// O HMAC não confere: o pacote foi alterado ou a chave é outra.
    BadHmac,
    /// O contador foi recusado.
    Replay(ReplayError),
    /// O pacote não é um pacote de controle válido.
    Packet(PacketError),
    /// O `tls-auth` precisa de um `auth` diferente de `none`.
    NoDigest,
    /// O contador de pacotes acabou; é preciso renegociar.
    Exhausted,
    /// O `WKc` é grande demais ou está mal formado.
    BadWkc,
}

impl fmt::Display for WrapError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Short => f.write_str("pacote curto demais para a proteção do canal de controle"),
            Self::BadHmac => f.write_str(
                "a autenticação do pacote falhou (a chave tls-auth/tls-crypt não é a do servidor?)",
            ),
            Self::Replay(e) => write!(f, "{e}"),
            Self::Packet(e) => write!(f, "{e}"),
            Self::NoDigest => f.write_str("o tls-auth não funciona com `auth none`"),
            Self::Exhausted => f.write_str("o contador de pacotes do canal de controle acabou"),
            Self::BadWkc => f.write_str("o WKc do tls-crypt-v2 está mal formado"),
        }
    }
}

impl std::error::Error for WrapError {}

impl From<PacketError> for WrapError {
    fn from(e: PacketError) -> Self {
        WrapError::Packet(e)
    }
}

impl From<ReplayError> for WrapError {
    fn from(e: ReplayError) -> Self {
        WrapError::Replay(e)
    }
}

/// As chaves de uma direção do `tls-crypt`.
struct CryptKeys {
    cipher: Zeroizing<[u8; TLS_CRYPT_KEY_LEN]>,
    hmac: Zeroizing<[u8; TLS_CRYPT_KEY_LEN]>,
}

impl CryptKeys {
    fn from_half(half: &KeyHalf) -> CryptKeys {
        let mut cipher = Zeroizing::new([0u8; TLS_CRYPT_KEY_LEN]);
        let mut hmac = Zeroizing::new([0u8; TLS_CRYPT_KEY_LEN]);
        cipher.copy_from_slice(&half.cipher[..TLS_CRYPT_KEY_LEN]);
        hmac.copy_from_slice(&half.hmac[..TLS_CRYPT_KEY_LEN]);
        CryptKeys { cipher, hmac }
    }
}

enum Kind {
    None,
    Auth {
        digest: AuthDigest,
        out_key: Zeroizing<Vec<u8>>,
        in_key: Zeroizing<Vec<u8>>,
    },
    Crypt {
        out: CryptKeys,
        inn: CryptKeys,
        wkc: Vec<u8>,
    },
}

/// Protege e abre os pacotes de controle de uma conexão.
pub struct Wrapper {
    kind: Kind,
    /// O último contador enviado.
    send_id: u32,
    /// O instante em que a sequência de envio começou (segundos desde 1970).
    start_time: u32,
    replay: ReplayWindow,
}

impl fmt::Debug for Wrapper {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let nome = match self.kind {
            Kind::None => "nenhuma",
            Kind::Auth { .. } => "tls-auth",
            Kind::Crypt { ref wkc, .. } if wkc.is_empty() => "tls-crypt",
            Kind::Crypt { .. } => "tls-crypt-v2",
        };
        write!(f, "Wrapper({nome})")
    }
}

/// Os HMACs são calculados sobre partes soltas, sem juntar tudo numa cópia.
macro_rules! com_hmac {
    ($digest:expr, $chave:expr, $mac:ident => $corpo:expr) => {
        match $digest {
            AuthDigest::Sha1 => {
                let mut $mac = <Hmac<Sha1> as KeyInit>::new_from_slice($chave)
                    .expect("HMAC aceita qualquer chave");
                $corpo
            }
            AuthDigest::Sha256 => {
                let mut $mac = <Hmac<Sha256> as KeyInit>::new_from_slice($chave)
                    .expect("HMAC aceita qualquer chave");
                $corpo
            }
            AuthDigest::Sha384 => {
                let mut $mac = <Hmac<Sha384> as KeyInit>::new_from_slice($chave)
                    .expect("HMAC aceita qualquer chave");
                $corpo
            }
            AuthDigest::Sha512 => {
                let mut $mac = <Hmac<Sha512> as KeyInit>::new_from_slice($chave)
                    .expect("HMAC aceita qualquer chave");
                $corpo
            }
            AuthDigest::None => unreachable!("o construtor recusa `auth none`"),
        }
    };
}

fn hmac_of(digest: AuthDigest, key: &[u8], parts: &[&[u8]]) -> Vec<u8> {
    com_hmac!(digest, key, mac => {
        for p in parts {
            mac.update(p);
        }
        mac.finalize().into_bytes().to_vec()
    })
}

fn hmac_check(digest: AuthDigest, key: &[u8], parts: &[&[u8]], expected: &[u8]) -> bool {
    com_hmac!(digest, key, mac => {
        for p in parts {
            mac.update(p);
        }
        mac.verify_slice(expected).is_ok()
    })
}

fn apply_ctr(key: &[u8; TLS_CRYPT_KEY_LEN], iv: &[u8], data: &mut [u8]) {
    // A chave tem 32 bytes e o IV 16, então os tamanhos sempre servem.
    let mut cipher =
        Ctr128BE::<Aes256>::new_from_slices(key, iv).expect("tamanhos de chave e IV fixos");
    cipher.apply_keystream(data);
}

impl Wrapper {
    /// Sem proteção: o pacote vai como está.
    pub fn none() -> Wrapper {
        Wrapper::new(Kind::None, 0, 1)
    }

    fn new(kind: Kind, start_time: u32, replay_window: u32) -> Wrapper {
        Wrapper {
            kind,
            send_id: 0,
            start_time,
            replay: ReplayWindow::new(replay_window),
        }
    }

    /// `tls-auth`. `direction` é o `key-direction` visto pelo cliente; o servidor usa o inverso.
    /// `start_time` é o relógio (segundos desde 1970) e `replay_window` o `replay-window`.
    pub fn tls_auth(
        key: &StaticKey,
        direction: Direction,
        digest: AuthDigest,
        role: Role,
        start_time: u32,
        replay_window: u32,
    ) -> Result<Wrapper, WrapError> {
        if digest == AuthDigest::None {
            return Err(WrapError::NoDigest);
        }
        let (client_out, client_in) = key.tls_auth_halves(direction);
        let (out, inn) = match role {
            Role::Client => (client_out, client_in),
            Role::Server => (client_in, client_out),
        };
        let corta = |half: &KeyHalf| Zeroizing::new(half.hmac[..digest.len()].to_vec());
        let kind = Kind::Auth {
            digest,
            out_key: corta(&out),
            in_key: corta(&inn),
        };
        Ok(Wrapper::new(kind, start_time, replay_window))
    }

    /// `tls-crypt`.
    pub fn tls_crypt(key: &StaticKey, role: Role, start_time: u32, replay_window: u32) -> Wrapper {
        Wrapper::crypt(key, Vec::new(), role, start_time, replay_window)
    }

    /// `tls-crypt-v2` do lado do cliente: `kc` é a chave do cliente (256 bytes, no mesmo formato
    /// da chave estática) e `wkc` a chave embrulhada que vai no primeiro pacote.
    pub fn tls_crypt_v2(
        kc: &StaticKey,
        wkc: Vec<u8>,
        start_time: u32,
        replay_window: u32,
    ) -> Result<Wrapper, WrapError> {
        if wkc.len() < 2 {
            return Err(WrapError::BadWkc);
        }
        Ok(Wrapper::crypt(
            kc,
            wkc,
            Role::Client,
            start_time,
            replay_window,
        ))
    }

    fn crypt(key: &StaticKey, wkc: Vec<u8>, role: Role, start_time: u32, window: u32) -> Wrapper {
        let (client_out, client_in) = key.tls_crypt_halves();
        let (out, inn) = match role {
            Role::Client => (client_out, client_in),
            Role::Server => (client_in, client_out),
        };
        let kind = Kind::Crypt {
            out: CryptKeys::from_half(&out),
            inn: CryptKeys::from_half(&inn),
            wkc,
        };
        Wrapper::new(kind, start_time, window)
    }

    /// Quantos bytes a proteção acrescenta a cada pacote (sem contar o `WKc` do primeiro).
    pub fn overhead(&self) -> usize {
        match &self.kind {
            Kind::None => 0,
            Kind::Auth { digest, .. } => digest.len() + REPLAY_ID_LEN,
            Kind::Crypt { .. } => TLS_CRYPT_TAG_LEN + REPLAY_ID_LEN,
        }
    }

    /// Se a proteção cifra o corpo (só o `tls-crypt` e o `tls-crypt-v2`).
    pub fn encrypts(&self) -> bool {
        matches!(self.kind, Kind::Crypt { .. })
    }

    fn next_replay_id(&mut self) -> Result<[u8; REPLAY_ID_LEN], WrapError> {
        self.send_id = self.send_id.checked_add(1).ok_or(WrapError::Exhausted)?;
        let mut out = [0u8; REPLAY_ID_LEN];
        out[..4].copy_from_slice(&self.send_id.to_be_bytes());
        out[4..].copy_from_slice(&self.start_time.to_be_bytes());
        Ok(out)
    }

    /// Protege o pacote e devolve os bytes que vão para a rede (sem o prefixo do TCP).
    pub fn seal(&mut self, packet: &ControlPacket) -> Result<Vec<u8>, WrapError> {
        let header = packet.header();
        let mut body = packet.body()?;
        if matches!(self.kind, Kind::None) {
            let mut out = header.to_vec();
            out.append(&mut body);
            return Ok(out);
        }
        let replay_id = self.next_replay_id()?;
        match &self.kind {
            Kind::None => unreachable!("tratado acima"),
            Kind::Auth {
                digest, out_key, ..
            } => {
                let tag = hmac_of(
                    *digest,
                    out_key,
                    &[&replay_id, &header[..1], &header[1..], &body],
                );
                let mut out =
                    Vec::with_capacity(header.len() + tag.len() + REPLAY_ID_LEN + body.len());
                out.extend_from_slice(&header);
                out.extend_from_slice(&tag);
                out.extend_from_slice(&replay_id);
                out.append(&mut body);
                Ok(out)
            }
            Kind::Crypt { out, wkc, .. } => {
                let tag = hmac_of(
                    AuthDigest::Sha256,
                    &out.hmac[..],
                    &[&header[..1], &header[1..], &replay_id, &body],
                );
                apply_ctr(&out.cipher, &tag[..IV_LEN], &mut body);
                let mut wire = Vec::with_capacity(
                    header.len() + REPLAY_ID_LEN + tag.len() + body.len() + wkc.len(),
                );
                wire.extend_from_slice(&header);
                wire.extend_from_slice(&replay_id);
                wire.extend_from_slice(&tag);
                wire.append(&mut body);
                if packet.opcode == Opcode::HardResetClientV3 {
                    wire.extend_from_slice(wkc);
                }
                Ok(wire)
            }
        }
    }

    /// Confere e abre um pacote que veio da rede. Só um pacote autêntico e novo é aceito, e só
    /// ele atualiza a janela de replay.
    pub fn open(&mut self, wire: &[u8]) -> Result<ControlPacket, WrapError> {
        if matches!(self.kind, Kind::None) {
            return Ok(ControlPacket::parse(wire)?);
        }
        let (opcode, key_id) = classify(wire)?;
        if !opcode.is_control() {
            return Err(PacketError::NotControl.into());
        }
        let header_len = 1 + SESSION_ID_LEN;
        let header = wire.get(..header_len).ok_or(WrapError::Short)?;
        let mut session_id = SessionId::default();
        session_id.0.copy_from_slice(&header[1..]);
        let rest = &wire[header_len..];
        match &self.kind {
            Kind::None => unreachable!("tratado acima"),
            Kind::Auth { digest, in_key, .. } => {
                let dlen = digest.len();
                if rest.len() < dlen + REPLAY_ID_LEN {
                    return Err(WrapError::Short);
                }
                let (tag, rest) = rest.split_at(dlen);
                let (replay_id, body) = rest.split_at(REPLAY_ID_LEN);
                if !hmac_check(
                    *digest,
                    in_key,
                    &[replay_id, &header[..1], &header[1..], body],
                    tag,
                ) {
                    return Err(WrapError::BadHmac);
                }
                let (id, time) = split_replay_id(replay_id);
                self.replay.check_and_accept(id, time)?;
                Ok(ControlPacket::parse_body(opcode, key_id, session_id, body)?)
            }
            Kind::Crypt { inn, .. } => {
                if rest.len() < REPLAY_ID_LEN + TLS_CRYPT_TAG_LEN {
                    return Err(WrapError::Short);
                }
                let (replay_id, rest) = rest.split_at(REPLAY_ID_LEN);
                let (tag, cipher_body) = rest.split_at(TLS_CRYPT_TAG_LEN);
                let mut body = cipher_body.to_vec();
                apply_ctr(&inn.cipher, &tag[..IV_LEN], &mut body);
                if !hmac_check(
                    AuthDigest::Sha256,
                    &inn.hmac[..],
                    &[&header[..1], &header[1..], replay_id, &body],
                    tag,
                ) {
                    return Err(WrapError::BadHmac);
                }
                let (id, time) = split_replay_id(replay_id);
                self.replay.check_and_accept(id, time)?;
                Ok(ControlPacket::parse_body(
                    opcode, key_id, session_id, &body,
                )?)
            }
        }
    }
}

fn split_replay_id(bytes: &[u8]) -> (u32, u32) {
    let id = u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
    let time = u32::from_be_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]);
    (id, time)
}

#[cfg(test)]
mod testes {
    use super::*;

    fn chave(semente: u8) -> StaticKey {
        let mut b = [0u8; 256];
        for (i, x) in b.iter_mut().enumerate() {
            *x = (i as u8).wrapping_mul(7).wrapping_add(semente);
        }
        StaticKey::from_bytes(b)
    }

    fn pacote(n: u32) -> ControlPacket {
        ControlPacket {
            opcode: Opcode::ControlV1,
            key_id: 2,
            session_id: SessionId([1, 2, 3, 4, 5, 6, 7, 8]),
            acks: vec![n, n + 1],
            remote_session_id: Some(SessionId([8, 7, 6, 5, 4, 3, 2, 1])),
            message_id: Some(n),
            payload: (0..40).map(|i| i as u8 ^ n as u8).collect(),
        }
    }

    const T0: u32 = 1_800_000_000;

    fn todos() -> Vec<(&'static str, Wrapper, Wrapper)> {
        let k = chave(1);
        let mut v = vec![];
        for (nome, dir) in [
            ("tls-auth bidirecional", Direction::Bidirectional),
            ("tls-auth direção 1", Direction::Inverse),
        ] {
            v.push((
                nome,
                Wrapper::tls_auth(&k, dir, AuthDigest::Sha1, Role::Client, T0, 64).unwrap(),
                Wrapper::tls_auth(&k, dir, AuthDigest::Sha1, Role::Server, T0, 64).unwrap(),
            ));
        }
        v.push((
            "tls-auth sha512",
            Wrapper::tls_auth(
                &k,
                Direction::Normal,
                AuthDigest::Sha512,
                Role::Client,
                T0,
                64,
            )
            .unwrap(),
            Wrapper::tls_auth(
                &k,
                Direction::Normal,
                AuthDigest::Sha512,
                Role::Server,
                T0,
                64,
            )
            .unwrap(),
        ));
        v.push((
            "tls-crypt",
            Wrapper::tls_crypt(&k, Role::Client, T0, 64),
            Wrapper::tls_crypt(&k, Role::Server, T0, 64),
        ));
        v.push(("nenhuma", Wrapper::none(), Wrapper::none()));
        v
    }

    #[test]
    fn ida_e_volta_nos_dois_sentidos() {
        for (nome, mut cliente, mut servidor) in todos() {
            for n in 0..5 {
                let p = pacote(n);
                let wire = cliente.seal(&p).unwrap();
                assert_eq!(
                    servidor.open(&wire).unwrap(),
                    p,
                    "{nome}, cliente -> servidor"
                );
                let wire = servidor.seal(&p).unwrap();
                assert_eq!(
                    cliente.open(&wire).unwrap(),
                    p,
                    "{nome}, servidor -> cliente"
                );
            }
        }
    }

    #[test]
    fn o_tamanho_na_rede_confere_com_o_overhead() {
        for (nome, mut cliente, _) in todos() {
            let p = pacote(1);
            let puro = p.encode().unwrap().len();
            assert_eq!(
                cliente.seal(&p).unwrap().len(),
                puro + cliente.overhead(),
                "{nome}"
            );
        }
    }

    #[test]
    fn alterar_qualquer_byte_descarta_o_pacote() {
        for (nome, mut cliente, mut servidor) in todos() {
            if nome == "nenhuma" {
                continue;
            }
            let wire = cliente.seal(&pacote(3)).unwrap();
            for i in 0..wire.len() {
                for mascara in [0x01u8, 0x80] {
                    let mut ruim = wire.clone();
                    ruim[i] ^= mascara;
                    assert!(
                        servidor.open(&ruim).is_err(),
                        "{nome}: byte {i} alterado passou"
                    );
                }
            }
            // O pacote bom ainda passa depois das tentativas ruins: elas não queimaram o contador.
            assert!(servidor.open(&wire).is_ok(), "{nome}");
        }
    }

    #[test]
    fn pacote_cortado_em_qualquer_ponto_da_erro() {
        for (nome, mut cliente, mut servidor) in todos() {
            let wire = cliente.seal(&pacote(3)).unwrap();
            for corte in 0..wire.len() {
                // O corte que cai exatamente no fim do corpo também é inválido, porque o
                // corpo perde bytes; só o pacote sem proteção pode cortar a carga sem erro.
                if nome == "nenhuma" && corte >= wire.len() - 40 {
                    continue;
                }
                assert!(
                    servidor.open(&wire[..corte]).is_err(),
                    "{nome}: corte em {corte}"
                );
            }
        }
    }

    #[test]
    fn replay_do_mesmo_pacote_e_recusado() {
        for (nome, mut cliente, mut servidor) in todos() {
            if nome == "nenhuma" {
                continue;
            }
            let wire = cliente.seal(&pacote(1)).unwrap();
            assert!(servidor.open(&wire).is_ok());
            assert_eq!(
                servidor.open(&wire),
                Err(WrapError::Replay(ReplayError::Duplicate)),
                "{nome}"
            );
        }
    }

    #[test]
    fn pacotes_fora_de_ordem_dentro_da_janela_passam() {
        for (nome, mut cliente, mut servidor) in todos() {
            if nome == "nenhuma" {
                continue;
            }
            let a = cliente.seal(&pacote(1)).unwrap();
            let b = cliente.seal(&pacote(2)).unwrap();
            let c = cliente.seal(&pacote(3)).unwrap();
            assert!(servidor.open(&c).is_ok(), "{nome}");
            assert!(servidor.open(&a).is_ok(), "{nome}");
            assert!(servidor.open(&b).is_ok(), "{nome}");
        }
    }

    #[test]
    fn tempo_antigo_e_recusado_e_tempo_novo_recomeca() {
        let k = chave(5);
        let mut velho = Wrapper::tls_crypt(&k, Role::Client, T0 - 100, 64);
        let mut novo = Wrapper::tls_crypt(&k, Role::Client, T0 + 100, 64);
        let mut servidor = Wrapper::tls_crypt(&k, Role::Server, T0, 64);
        let mut meio = Wrapper::tls_crypt(&k, Role::Client, T0, 64);
        assert!(servidor.open(&meio.seal(&pacote(1)).unwrap()).is_ok());
        assert_eq!(
            servidor.open(&velho.seal(&pacote(1)).unwrap()),
            Err(WrapError::Replay(ReplayError::OldTime))
        );
        // Uma sequência nova (o cliente reiniciou) recomeça do 1.
        assert!(servidor.open(&novo.seal(&pacote(1)).unwrap()).is_ok());
        assert!(servidor.open(&meio.seal(&pacote(2)).unwrap()).is_err());
    }

    #[test]
    fn chave_errada_ou_direcao_errada_nao_passa() {
        let k = chave(1);
        let outra = chave(2);
        let mut cliente = Wrapper::tls_crypt(&k, Role::Client, T0, 64);
        let mut servidor_errado = Wrapper::tls_crypt(&outra, Role::Server, T0, 64);
        assert_eq!(
            servidor_errado.open(&cliente.seal(&pacote(1)).unwrap()),
            Err(WrapError::BadHmac)
        );
        // Dois clientes não se entendem (as duas pontas enviam com a mesma metade).
        let mut outro_cliente = Wrapper::tls_crypt(&k, Role::Client, T0, 64);
        assert_eq!(
            outro_cliente.open(&cliente.seal(&pacote(1)).unwrap()),
            Err(WrapError::BadHmac)
        );
        // No tls-auth, a direção do servidor precisa ser a inversa da do cliente.
        let mut c = Wrapper::tls_auth(
            &k,
            Direction::Normal,
            AuthDigest::Sha1,
            Role::Client,
            T0,
            64,
        )
        .unwrap();
        let mut s = Wrapper::tls_auth(
            &k,
            Direction::Inverse,
            AuthDigest::Sha1,
            Role::Client,
            T0,
            64,
        )
        .unwrap();
        // `s` é um cliente com direção 1: envia com a metade 1, que é o que o servidor do cliente
        // com direção 0 espera receber.
        assert!(c.open(&s.seal(&pacote(1)).unwrap()).is_ok());
    }

    #[test]
    fn cada_envio_tem_contador_novo_e_retransmissao_muda_os_bytes() {
        let k = chave(1);
        let mut cliente = Wrapper::tls_crypt(&k, Role::Client, T0, 64);
        let p = pacote(1);
        let a = cliente.seal(&p).unwrap();
        let b = cliente.seal(&p).unwrap();
        assert_ne!(a, b);
        // O contador começa em 1, depois do cabeçalho de 9 bytes.
        assert_eq!(&a[9..13], &1u32.to_be_bytes());
        assert_eq!(&b[9..13], &2u32.to_be_bytes());
        assert_eq!(&a[13..17], &T0.to_be_bytes());
        assert_eq!(&b[13..17], &T0.to_be_bytes());
    }

    #[test]
    fn o_corpo_do_tls_crypt_nao_aparece_em_claro() {
        let k = chave(1);
        let mut cliente = Wrapper::tls_crypt(&k, Role::Client, T0, 64);
        let mut p = pacote(1);
        p.payload = b"texto-bem-reconhecivel-no-fio".to_vec();
        let wire = cliente.seal(&p).unwrap();
        assert!(!wire.windows(10).any(|w| w == b"texto-bem-"));
        let mut tls_auth = Wrapper::tls_auth(
            &k,
            Direction::Bidirectional,
            AuthDigest::Sha1,
            Role::Client,
            T0,
            64,
        )
        .unwrap();
        let wire = tls_auth.seal(&p).unwrap();
        assert!(
            wire.windows(10).any(|w| w == b"texto-bem-"),
            "o tls-auth só autentica"
        );
    }

    #[test]
    fn tls_auth_recusa_auth_none() {
        assert!(matches!(
            Wrapper::tls_auth(
                &chave(1),
                Direction::Normal,
                AuthDigest::None,
                Role::Client,
                T0,
                64
            ),
            Err(WrapError::NoDigest)
        ));
    }

    #[test]
    fn o_contador_acaba_com_erro_e_nao_repete() {
        let mut c = Wrapper::tls_crypt(&chave(1), Role::Client, T0, 64);
        c.send_id = u32::MAX - 1;
        assert!(c.seal(&pacote(1)).is_ok());
        assert_eq!(c.seal(&pacote(1)), Err(WrapError::Exhausted));
    }

    #[test]
    fn tls_crypt_v2_leva_o_wkc_so_no_primeiro_pacote() {
        let kc = chave(9);
        let mut wkc = vec![0x77; 98];
        wkc.extend_from_slice(&100u16.to_be_bytes());
        let mut c = Wrapper::tls_crypt_v2(&kc, wkc.clone(), T0, 64).unwrap();
        let mut reset = pacote(0);
        reset.opcode = Opcode::HardResetClientV3;
        reset.acks.clear();
        reset.remote_session_id = None;
        reset.payload.clear();
        let wire = c.seal(&reset).unwrap();
        assert!(wire.ends_with(&wkc));
        let sem_wkc = &wire[..wire.len() - wkc.len()];
        // O resto é um pacote do tls-crypt comum, cifrado com Kc.
        let mut s = Wrapper::tls_crypt(&kc, Role::Server, T0, 64);
        assert_eq!(s.open(sem_wkc).unwrap(), reset);
        // Os seguintes não levam o WKc.
        let wire = c.seal(&pacote(2)).unwrap();
        assert_eq!(wire.len(), pacote(2).encode().unwrap().len() + c.overhead());
        assert!(matches!(
            Wrapper::tls_crypt_v2(&kc, vec![], T0, 64),
            Err(WrapError::BadWkc)
        ));
    }

    /// Valores calculados fora do crate com o `openssl` (HMAC e AES-256-CTR), para conferir a
    /// construção byte a byte.
    #[test]
    fn tls_crypt_confere_com_calculo_independente() {
        let k = chave(1);
        let mut cliente = Wrapper::tls_crypt(&k, Role::Client, T0, 64);
        let p = ControlPacket {
            opcode: Opcode::ControlV1,
            key_id: 0,
            session_id: SessionId([0x11; 8]),
            acks: vec![],
            remote_session_id: None,
            message_id: Some(0),
            payload: b"ola".to_vec(),
        };
        let wire = cliente.seal(&p).unwrap();
        let metade = k.half(1);
        let corpo = [&[0u8][..], &0u32.to_be_bytes(), b"ola"].concat();
        let replay_id = [&1u32.to_be_bytes()[..], &T0.to_be_bytes()].concat();
        let entrada = [&[0x20u8][..], &[0x11; 8], &replay_id, &corpo].concat();
        let tag = hmac_of(AuthDigest::Sha256, &metade.hmac[..32], &[&entrada]);
        assert_eq!(&wire[17..49], &tag[..]);
        let mut cifrado = corpo.clone();
        apply_ctr(
            &CryptKeys::from_half(&metade).cipher,
            &tag[..16],
            &mut cifrado,
        );
        assert_eq!(&wire[49..], &cifrado[..]);
        // Os mesmos bytes, calculados com o `openssl` (HMAC-SHA-256 e AES-256-CTR) e a chave
        // `(i * 7 + 1) mod 256`.
        let esperado_tag = "4e6d21d11c2d39ebf187044d317300c928a721f3a5f8005fd75b329988643bd9";
        let esperado_corpo = "d384c1cff89a1b8f";
        assert_eq!(crate::pki::hex(&wire[17..49]), esperado_tag);
        assert_eq!(crate::pki::hex(&wire[49..]), esperado_corpo);
    }

    /// O HMAC-SHA-1 calculado fora do crate (Python) para a mesma chave.
    #[test]
    fn tls_auth_confere_com_calculo_independente() {
        let k = chave(1);
        let mut c = Wrapper::tls_auth(
            &k,
            Direction::Bidirectional,
            AuthDigest::Sha1,
            Role::Client,
            T0,
            64,
        )
        .unwrap();
        let p = ControlPacket {
            opcode: Opcode::ControlV1,
            key_id: 0,
            session_id: SessionId([0x11; 8]),
            acks: vec![],
            remote_session_id: None,
            message_id: Some(0),
            payload: b"ola".to_vec(),
        };
        let wire = c.seal(&p).unwrap();
        // op, sessão, HMAC (20), contador (8), corpo.
        assert_eq!(
            crate::pki::hex(&wire[9..29]),
            "9d6b9e52ebad79143cc9fdcad9028fe5d4f9c92c"
        );
        assert_eq!(&wire[29..33], &1u32.to_be_bytes());
        assert_eq!(&wire[37..], &[0, 0, 0, 0, 0, b'o', b'l', b'a']);
    }
}
