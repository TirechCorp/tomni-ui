//! O canal de dados: cifra e decifra os pacotes IP, controla o `packet-id`, a janela de replay, o
//! enquadramento da compressão, o ping e o `OCC`.
//!
//! Tudo é sem E/S. As chaves vêm do canal de controle ([`crate::control::Event::DataKeys`]) e a
//! cada renegociação chega uma geração nova: a anterior continua valendo só pelo `tran-window`.
//!
//! ## O pacote
//!
//! Com AEAD (AES-128-GCM, AES-256-GCM e ChaCha20-Poly1305):
//!
//! ```text
//! P_DATA_V2:  op|key_id (1) · peer_id (3) · packet_id (4) · etiqueta (16) · texto cifrado
//! P_DATA_V1:  op|key_id (1) ·               packet_id (4) · etiqueta (16) · texto cifrado
//! ```
//!
//! O nonce de 12 bytes é o `packet_id` seguido dos 8 primeiros bytes da chave "de HMAC" da
//! direção (o IV implícito). O dado associado é o cabeçalho inteiro até o `packet_id`
//! (`op|key_id`, `peer_id` e `packet_id`) no V2; no V1, só o `packet_id` (**confira**: o V1 só
//! aparece em servidores sem `peer-id`, e nenhum servidor atual o usa com AEAD; não foi provado
//! contra um servidor real).
//!
//! O texto em claro é a carga com o enquadramento da compressão na frente (quando o servidor
//! empurrou `compress` ou `comp-lzo`): o pacote IP, o ping (16 bytes mágicos) ou o `OCC`.

use std::fmt;
use std::time::{Duration, Instant};

use ring::aead::{
    AES_128_GCM, AES_256_GCM, Aad, CHACHA20_POLY1305, LessSafeKey, NONCE_LEN, Nonce, UnboundKey,
};
use zeroize::Zeroizing;

use crate::config::Compress;
use crate::packet::{MAX_KEY_ID, Opcode, PEER_ID_UNDEFINED, classify, first_byte};
use crate::prf::{DataKeys, DirectionKeys};
use crate::replay::{ReplayError, ReplayWindow};

/// O ping do OpenVPN: um pacote de dados cuja carga são estes 16 bytes.
pub const PING_STRING: [u8; 16] = [
    0x2a, 0x18, 0x7b, 0xf3, 0x64, 0x1e, 0xb4, 0xcb, 0x07, 0xed, 0x2d, 0x0a, 0x98, 0x1f, 0xc7, 0x48,
];

/// O começo das mensagens `OCC` (a conferência de opções), também dentro de um pacote de dados.
pub const OCC_MAGIC: [u8; 16] = [
    0x28, 0x7f, 0x34, 0x6b, 0xd4, 0xef, 0x7a, 0x81, 0x2d, 0x56, 0xb8, 0xd3, 0xaf, 0xc5, 0x45, 0x9c,
];

/// O código do `OCC` que avisa a saída do cliente (depois dos 16 bytes mágicos).
pub const OCC_EXIT: u8 = 6;

/// O byte que marca "sem compressão" no enquadramento do `comp-lzo no`.
const NO_COMPRESS: u8 = 0xFA;
/// O byte que marca "sem compressão" no enquadramento com o primeiro byte levado para o fim.
const NO_COMPRESS_SWAP: u8 = 0xFB;
/// A marca de LZO no enquadramento antigo.
const LZO_BYTE: u8 = 0x66;
/// A marca de LZ4 no enquadramento antigo.
const LZ4_BYTE: u8 = 0x69;
/// O byte que anuncia o enquadramento novo (`stub-v2`, `lz4-v2`).
const V2_INDICATOR: u8 = 0x50;
/// No enquadramento novo, "sem compressão".
const V2_UNCOMPRESSED: u8 = 0x00;

/// O `packet-id` a partir do qual o motor pede uma renegociação (a sequência não pode dar a
/// volta com a mesma chave).
pub const RENEGOTIATE_AT_ID: u32 = 0xFF00_0000;

/// O tamanho da etiqueta dos AEAD.
const TAG_LEN: usize = 16;
/// O tamanho do `packet-id` (só o contador, sem tempo).
const ID_LEN: usize = 4;

/// A cifra do canal de dados.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DataCipher {
    /// `AES-128-GCM`.
    Aes128Gcm,
    /// `AES-256-GCM`.
    Aes256Gcm,
    /// `CHACHA20-POLY1305`.
    ChaCha20Poly1305,
}

impl DataCipher {
    /// A cifra de um nome do OpenVPN (sem diferenciar maiúsculas). `None` para as que o motor
    /// não implementa.
    pub fn from_name(name: &str) -> Option<DataCipher> {
        match name.to_ascii_uppercase().as_str() {
            "AES-128-GCM" => Some(DataCipher::Aes128Gcm),
            "AES-256-GCM" => Some(DataCipher::Aes256Gcm),
            "CHACHA20-POLY1305" => Some(DataCipher::ChaCha20Poly1305),
            _ => None,
        }
    }

    /// O nome do OpenVPN.
    pub fn name(self) -> &'static str {
        match self {
            DataCipher::Aes128Gcm => "AES-128-GCM",
            DataCipher::Aes256Gcm => "AES-256-GCM",
            DataCipher::ChaCha20Poly1305 => "CHACHA20-POLY1305",
        }
    }

    /// O tamanho da chave, em bytes.
    pub fn key_len(self) -> usize {
        match self {
            DataCipher::Aes128Gcm => 16,
            DataCipher::Aes256Gcm | DataCipher::ChaCha20Poly1305 => 32,
        }
    }

    fn algorithm(self) -> &'static ring::aead::Algorithm {
        match self {
            DataCipher::Aes128Gcm => &AES_128_GCM,
            DataCipher::Aes256Gcm => &AES_256_GCM,
            DataCipher::ChaCha20Poly1305 => &CHACHA20_POLY1305,
        }
    }
}

/// O que dá errado ao cifrar ou decifrar um pacote.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DataError {
    /// Ainda não há chave (o canal de controle não terminou).
    NoKeys,
    /// O pacote é de um `key_id` que não está vivo.
    UnknownKey(u8),
    /// O pacote é curto demais ou tem outro formato.
    Malformed,
    /// A etiqueta não confere (chave errada ou pacote adulterado).
    Authentication,
    /// O pacote foi recusado pela janela de replay.
    Replay(ReplayError),
    /// A carga veio comprimida com um algoritmo que o app não suporta.
    Compressed(&'static str),
    /// O `packet-id` acabou; só uma renegociação resolve.
    Exhausted,
}

impl fmt::Display for DataError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DataError::NoKeys => f.write_str("ainda não há chave do canal de dados"),
            DataError::UnknownKey(k) => {
                write!(f, "pacote de dados com o key_id {k}, que não está vivo")
            }
            DataError::Malformed => f.write_str("pacote de dados malformado"),
            DataError::Authentication => f.write_str("a etiqueta do pacote de dados não confere"),
            DataError::Replay(e) => write!(f, "{e}"),
            DataError::Compressed(a) => write!(
                f,
                "o servidor comprime com {a}, que o app não suporta; peça ao administrador allow-compression no ou compress stub-v2"
            ),
            DataError::Exhausted => f.write_str("o packet-id do canal de dados acabou"),
        }
    }
}

impl std::error::Error for DataError {}

/// O que um pacote de dados decifrado era.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Opened {
    /// Um pacote IP, para a pilha.
    Ip(Vec<u8>),
    /// O ping do servidor: só prova que ele está vivo.
    Ping,
    /// Uma mensagem `OCC` (a conferência de opções); o motor a ignora.
    Occ(Vec<u8>),
}

/// Os ajustes do canal de dados (vêm do `PUSH_REPLY` e do `.ovpn`).
#[derive(Debug, Clone)]
pub struct DataOptions {
    /// A cifra.
    pub cipher: DataCipher,
    /// O `peer-id` empurrado: com ele, o formato é `P_DATA_V2`; sem ele, `P_DATA_V1`.
    pub peer_id: Option<u32>,
    /// O enquadramento da compressão.
    pub compress: Option<Compress>,
    /// O `replay-window`, em pacotes.
    pub replay_window: u32,
    /// Por quanto tempo a geração anterior continua valendo.
    pub tran_window: Duration,
    /// O `reneg-bytes`.
    pub reneg_bytes: Option<u64>,
    /// O `reneg-pkts`.
    pub reneg_pkts: Option<u64>,
}

struct Generation {
    key_id: u8,
    seal: LessSafeKey,
    open: LessSafeKey,
    send_iv: Zeroizing<[u8; 8]>,
    recv_iv: Zeroizing<[u8; 8]>,
    next_id: u32,
    replay: ReplayWindow,
    retire_at: Option<Instant>,
    bytes: u64,
    packets: u64,
}

impl Generation {
    fn new(
        key_id: u8,
        cipher: DataCipher,
        keys: &DataKeys,
        replay_window: u32,
    ) -> Result<Generation, DataError> {
        let key = |d: &DirectionKeys| {
            UnboundKey::new(cipher.algorithm(), &d.cipher[..cipher.key_len()])
                .map(LessSafeKey::new)
                .map_err(|_| DataError::Malformed)
        };
        let iv = |d: &DirectionKeys| {
            let mut iv = Zeroizing::new([0u8; 8]);
            iv.copy_from_slice(&d.hmac[..8]);
            iv
        };
        Ok(Generation {
            key_id,
            seal: key(&keys.send)?,
            open: key(&keys.recv)?,
            send_iv: iv(&keys.send),
            recv_iv: iv(&keys.recv),
            next_id: 1,
            replay: ReplayWindow::new(replay_window),
            retire_at: None,
            bytes: 0,
            packets: 0,
        })
    }

    fn is_retired(&self, now: Instant) -> bool {
        self.retire_at.is_some_and(|t| now >= t)
    }
}

fn nonce(id: u32, iv: &[u8; 8]) -> Nonce {
    let mut n = [0u8; NONCE_LEN];
    n[..4].copy_from_slice(&id.to_be_bytes());
    n[4..].copy_from_slice(iv);
    Nonce::assume_unique_for_key(n)
}

/// O canal de dados de uma sessão.
pub struct DataChannel {
    options: DataOptions,
    generations: Vec<Generation>,
    /// O `key_id` usado para enviar.
    current: Option<u8>,
    /// Quantos pacotes foram recusados na recepção.
    rejected: u64,
}

impl fmt::Debug for DataChannel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DataChannel")
            .field("cipher", &self.options.cipher)
            .field("generations", &self.generations.len())
            .field("current", &self.current)
            .finish_non_exhaustive()
    }
}

impl DataChannel {
    /// Um canal sem chaves; [`DataChannel::install`] põe a primeira.
    pub fn new(options: DataOptions) -> DataChannel {
        DataChannel {
            options,
            generations: Vec::new(),
            current: None,
            rejected: 0,
        }
    }

    /// Os ajustes em uso.
    pub fn options(&self) -> &DataOptions {
        &self.options
    }

    /// Aceita a geração `key_id` e passa a enviar com ela. As anteriores valem até `agora +
    /// tran-window`. Se já havia uma geração com o mesmo `key_id`, ela é trocada.
    ///
    /// # Errors
    ///
    /// Se a chave não serve para a cifra (não acontece com as do canal de controle).
    pub fn install(&mut self, key_id: u8, keys: &DataKeys, now: Instant) -> Result<(), DataError> {
        let generation = Generation::new(
            key_id & MAX_KEY_ID,
            self.options.cipher,
            keys,
            self.options.replay_window,
        )?;
        let tran = self.options.tran_window;
        for old in &mut self.generations {
            if old.retire_at.is_none() {
                old.retire_at = Some(now + tran);
            }
        }
        self.generations.retain(|g| g.key_id != generation.key_id);
        self.current = Some(generation.key_id);
        self.generations.push(generation);
        Ok(())
    }

    /// Descarta as gerações cujo prazo acabou.
    pub fn expire(&mut self, now: Instant) {
        self.generations.retain(|g| !g.is_retired(now));
    }

    /// Quando a próxima geração vence, se houver.
    pub fn next_expiry(&self) -> Option<Instant> {
        self.generations.iter().filter_map(|g| g.retire_at).min()
    }

    /// Se já há chave para enviar.
    pub fn ready(&self) -> bool {
        self.current.is_some()
    }

    /// Se a geração atual pede uma renegociação (`packet-id` perto do fim, `reneg-bytes`,
    /// `reneg-pkts`).
    pub fn needs_renegotiation(&self) -> bool {
        let Some(g) = self.current_generation() else {
            return false;
        };
        g.next_id >= RENEGOTIATE_AT_ID
            || self.options.reneg_bytes.is_some_and(|b| g.bytes >= b)
            || self.options.reneg_pkts.is_some_and(|p| g.packets >= p)
    }

    /// Quantos pacotes recebidos foram recusados (etiqueta, replay, chave desconhecida).
    pub fn rejected(&self) -> u64 {
        self.rejected
    }

    /// O `key_id` com que se envia agora.
    pub fn current_key(&self) -> Option<u8> {
        self.current
    }

    fn current_generation(&self) -> Option<&Generation> {
        let id = self.current?;
        self.generations.iter().find(|g| g.key_id == id)
    }

    /// Quanto o formato soma ao pacote IP: o cabeçalho, a etiqueta e o enquadramento da
    /// compressão. Serve para calcular o MTU.
    pub fn overhead(&self) -> usize {
        header_len(self.options.peer_id) + TAG_LEN + framing_overhead(self.options.compress)
    }

    /// Cifra `payload` (um pacote IP) com a geração atual e devolve o datagrama pronto.
    ///
    /// # Errors
    ///
    /// [`DataError::NoKeys`] sem chave, ou [`DataError::Exhausted`] com o `packet-id` no fim.
    pub fn seal(&mut self, payload: &[u8]) -> Result<Vec<u8>, DataError> {
        let framed = frame(self.options.compress, payload);
        self.seal_unframed(&framed)
    }

    /// O ping pronto para enviar.
    ///
    /// # Errors
    ///
    /// Os mesmos de [`DataChannel::seal`].
    pub fn seal_ping(&mut self) -> Result<Vec<u8>, DataError> {
        self.seal(&PING_STRING)
    }

    /// O aviso de saída (`OCC_EXIT`) pronto para enviar.
    ///
    /// # Errors
    ///
    /// Os mesmos de [`DataChannel::seal`].
    pub fn seal_exit(&mut self) -> Result<Vec<u8>, DataError> {
        let mut occ = OCC_MAGIC.to_vec();
        occ.push(OCC_EXIT);
        self.seal(&occ)
    }

    /// Cifra `framed` como está, sem pôr o enquadramento da compressão (para os testes forjarem
    /// pacotes que o app nunca gera).
    ///
    /// # Errors
    ///
    /// Os mesmos de [`DataChannel::seal`].
    pub fn seal_raw(&mut self, framed: &[u8]) -> Result<Vec<u8>, DataError> {
        self.seal_unframed(framed)
    }

    fn seal_unframed(&mut self, plain: &[u8]) -> Result<Vec<u8>, DataError> {
        let peer_id = self.options.peer_id;
        let key_id = self.current.ok_or(DataError::NoKeys)?;
        let g = self
            .generations
            .iter_mut()
            .find(|g| g.key_id == key_id)
            .ok_or(DataError::NoKeys)?;
        if g.next_id == u32::MAX {
            return Err(DataError::Exhausted);
        }
        let id = g.next_id;
        g.next_id += 1;

        let mut head = Vec::with_capacity(8);
        match peer_id {
            Some(peer) => {
                head.push(first_byte(Opcode::DataV2, key_id));
                head.extend_from_slice(&peer.to_be_bytes()[1..]);
            }
            None => head.push(first_byte(Opcode::DataV1, key_id)),
        }
        head.extend_from_slice(&id.to_be_bytes());
        let ad_start = if peer_id.is_some() {
            0
        } else {
            head.len() - ID_LEN
        };

        let mut body = plain.to_vec();
        let tag = g
            .seal
            .seal_in_place_separate_tag(
                nonce(id, &g.send_iv),
                Aad::from(&head[ad_start..]),
                &mut body,
            )
            .map_err(|_| DataError::Malformed)?;
        g.bytes += plain.len() as u64;
        g.packets += 1;

        let mut out = head;
        out.reserve(TAG_LEN + body.len());
        out.extend_from_slice(tag.as_ref());
        out.extend_from_slice(&body);
        Ok(out)
    }

    /// Decifra um datagrama de dados. `now` vale para o prazo das gerações.
    ///
    /// # Errors
    ///
    /// Qualquer [`DataError`]; o motor descarta o pacote e segue.
    pub fn open(&mut self, packet: &[u8], now: Instant) -> Result<Opened, DataError> {
        let result = self.open_inner(packet, now);
        if result.is_err() {
            self.rejected += 1;
        }
        result
    }

    fn open_inner(&mut self, packet: &[u8], now: Instant) -> Result<Opened, DataError> {
        let (opcode, key_id) = classify(packet).map_err(|_| DataError::Malformed)?;
        let head_len = match opcode {
            Opcode::DataV2 => header_len(Some(0)),
            Opcode::DataV1 => header_len(None),
            _ => return Err(DataError::Malformed),
        };
        if packet.len() < head_len + TAG_LEN {
            return Err(DataError::Malformed);
        }
        let is_v2 = opcode == Opcode::DataV2;
        let g = self
            .generations
            .iter_mut()
            .find(|g| g.key_id == key_id && !g.is_retired(now))
            .ok_or(DataError::UnknownKey(key_id))?;
        let id_at = head_len - ID_LEN;
        let id = u32::from_be_bytes([
            packet[id_at],
            packet[id_at + 1],
            packet[id_at + 2],
            packet[id_at + 3],
        ]);
        g.replay.check(id, 0).map_err(DataError::Replay)?;
        let ad = if is_v2 {
            &packet[..head_len]
        } else {
            &packet[id_at..head_len]
        };
        // O `ring` quer o texto cifrado seguido da etiqueta.
        let (tag, cipher) = packet[head_len..].split_at(TAG_LEN);
        let mut buffer = Vec::with_capacity(cipher.len() + TAG_LEN);
        buffer.extend_from_slice(cipher);
        buffer.extend_from_slice(tag);
        let plain = g
            .open
            .open_in_place(nonce(id, &g.recv_iv), Aad::from(ad), &mut buffer)
            .map_err(|_| DataError::Authentication)?;
        g.replay.accept(id, 0);
        let plain = unframe(self.options.compress, plain)?;
        classify_payload(plain)
    }
}

fn classify_payload(plain: Vec<u8>) -> Result<Opened, DataError> {
    if plain == PING_STRING {
        Ok(Opened::Ping)
    } else if plain.starts_with(&OCC_MAGIC) {
        Ok(Opened::Occ(plain))
    } else if plain.is_empty() {
        Err(DataError::Malformed)
    } else {
        Ok(Opened::Ip(plain))
    }
}

/// O tamanho do cabeçalho até o `packet-id`, inclusive.
fn header_len(peer_id: Option<u32>) -> usize {
    match peer_id {
        Some(_) => 1 + 3 + ID_LEN,
        None => 1 + ID_LEN,
    }
}

/// Quantos bytes o enquadramento soma no pior caso.
fn framing_overhead(compress: Option<Compress>) -> usize {
    match compress {
        None => 0,
        Some(Compress::StubV2) => 2,
        Some(_) => 1,
    }
}

/// Põe o enquadramento da compressão na frente da carga (o app nunca comprime).
fn frame(compress: Option<Compress>, payload: &[u8]) -> Vec<u8> {
    match compress {
        None => payload.to_vec(),
        Some(Compress::LzoOff) | Some(Compress::Lzo) => {
            let mut out = Vec::with_capacity(payload.len() + 1);
            out.push(NO_COMPRESS);
            out.extend_from_slice(payload);
            out
        }
        Some(Compress::StubV2) | Some(Compress::Lz4V2) => {
            // Só quando o primeiro byte se parece com a marca é que o enquadramento aparece.
            if payload.first() == Some(&V2_INDICATOR) {
                let mut out = Vec::with_capacity(payload.len() + 2);
                out.extend_from_slice(&[V2_INDICATOR, V2_UNCOMPRESSED]);
                out.extend_from_slice(payload);
                out
            } else {
                payload.to_vec()
            }
        }
        Some(Compress::Stub) | Some(Compress::Lz4) => {
            // A marca vai na frente e o primeiro byte da carga vai para o fim.
            let Some((&first, rest)) = payload.split_first() else {
                return vec![NO_COMPRESS_SWAP];
            };
            let mut out = Vec::with_capacity(payload.len() + 1);
            out.push(NO_COMPRESS_SWAP);
            out.extend_from_slice(rest);
            out.push(first);
            out
        }
    }
}

/// Tira o enquadramento e devolve a carga. A que veio comprimida de verdade é um erro.
fn unframe(compress: Option<Compress>, plain: &[u8]) -> Result<Vec<u8>, DataError> {
    let Some(mode) = compress else {
        return Ok(plain.to_vec());
    };
    match mode {
        Compress::StubV2 | Compress::Lz4V2 => {
            if plain.first() == Some(&V2_INDICATOR) {
                match plain.get(1) {
                    Some(&V2_UNCOMPRESSED) => Ok(plain[2..].to_vec()),
                    Some(1) => Err(DataError::Compressed("LZ4")),
                    Some(2) => Err(DataError::Compressed("LZO")),
                    _ => Err(DataError::Malformed),
                }
            } else {
                Ok(plain.to_vec())
            }
        }
        Compress::LzoOff | Compress::Lzo | Compress::Stub | Compress::Lz4 => {
            let Some((&flag, rest)) = plain.split_first() else {
                return Err(DataError::Malformed);
            };
            match flag {
                NO_COMPRESS => Ok(rest.to_vec()),
                NO_COMPRESS_SWAP => match rest.split_last() {
                    Some((&last, middle)) => {
                        let mut out = Vec::with_capacity(rest.len());
                        out.push(last);
                        out.extend_from_slice(middle);
                        Ok(out)
                    }
                    None => Ok(Vec::new()),
                },
                LZO_BYTE => Err(DataError::Compressed("LZO")),
                LZ4_BYTE => Err(DataError::Compressed("LZ4")),
                _ => Err(DataError::Malformed),
            }
        }
    }
}

/// O `peer_id` que significa "sem peer-id" no `P_DATA_V2`.
pub const NO_PEER_ID: u32 = PEER_ID_UNDEFINED;

#[cfg(test)]
mod testes {
    use super::*;
    use crate::prf::KEY_BLOCK_LEN;
    use crate::wrap::Role;

    fn chaves(papel: Role) -> DataKeys {
        let mut bloco = [0u8; KEY_BLOCK_LEN];
        for (i, b) in bloco.iter_mut().enumerate() {
            *b = (i * 7 + 3) as u8;
        }
        DataKeys::from_block(&bloco, papel)
    }

    fn opcoes(cifra: DataCipher, peer: Option<u32>, compress: Option<Compress>) -> DataOptions {
        DataOptions {
            cipher: cifra,
            peer_id: peer,
            compress,
            replay_window: 64,
            tran_window: Duration::from_secs(60),
            reneg_bytes: None,
            reneg_pkts: None,
        }
    }

    fn par(
        cifra: DataCipher,
        peer: Option<u32>,
        compress: Option<Compress>,
    ) -> (DataChannel, DataChannel, Instant) {
        let agora = Instant::now();
        let mut cliente = DataChannel::new(opcoes(cifra, peer, compress));
        let mut servidor = DataChannel::new(opcoes(cifra, peer, compress));
        cliente.install(0, &chaves(Role::Client), agora).unwrap();
        servidor.install(0, &chaves(Role::Server), agora).unwrap();
        (cliente, servidor, agora)
    }

    fn pacote_ip(n: usize) -> Vec<u8> {
        let mut p = vec![0x45, 0, 0, 0];
        p.extend((0..n).map(|i| (i % 251) as u8));
        p
    }

    const CIFRAS: [DataCipher; 3] = [
        DataCipher::Aes128Gcm,
        DataCipher::Aes256Gcm,
        DataCipher::ChaCha20Poly1305,
    ];

    #[test]
    fn ida_e_volta_nas_tres_cifras_em_v1_e_v2() {
        for cifra in CIFRAS {
            for peer in [Some(5), None] {
                let (mut c, mut s, agora) = par(cifra, peer, None);
                for tamanho in [0usize, 1, 40, 1400] {
                    let ip = pacote_ip(tamanho);
                    let dg = c.seal(&ip).unwrap();
                    assert_eq!(
                        s.open(&dg, agora).unwrap(),
                        Opened::Ip(ip.clone()),
                        "{cifra:?}"
                    );
                    let volta = s.seal(&ip).unwrap();
                    assert_eq!(c.open(&volta, agora).unwrap(), Opened::Ip(ip));
                }
            }
        }
    }

    #[test]
    fn formato_do_pacote_v2_e_v1() {
        let (mut c, _, _) = par(DataCipher::Aes256Gcm, Some(0x0001_0203), None);
        let dg = c.seal(&[0x45, 1, 2, 3]).unwrap();
        // op|key_id, peer_id (3), packet_id (4), etiqueta (16), 4 bytes cifrados.
        assert_eq!(dg.len(), 1 + 3 + 4 + 16 + 4);
        assert_eq!(dg[0], 9 << 3);
        assert_eq!(&dg[1..4], &[1, 2, 3]);
        assert_eq!(&dg[4..8], &[0, 0, 0, 1]);
        assert_eq!(c.overhead(), 24);
        let (mut c1, _, _) = par(DataCipher::Aes256Gcm, None, None);
        let dg1 = c1.seal(&[0x45]).unwrap();
        assert_eq!(dg1[0], 6 << 3);
        assert_eq!(&dg1[1..5], &[0, 0, 0, 1]);
        assert_eq!(dg1.len(), 1 + 4 + 16 + 1);
    }

    #[test]
    fn o_packet_id_cresce_de_um_em_um_a_partir_de_1() {
        let (mut c, _, _) = par(DataCipher::Aes128Gcm, Some(1), None);
        for esperado in 1u32..=5 {
            let dg = c.seal(&[0x45]).unwrap();
            assert_eq!(u32::from_be_bytes([dg[4], dg[5], dg[6], dg[7]]), esperado);
        }
    }

    #[test]
    fn nonce_e_dado_associado_conferidos_por_vetor_fixo() {
        // Cifra "à mão" com o `ring`: o nonce é packet_id ‖ 8 bytes do hmac da direção, e o
        // dado associado é o cabeçalho (op|key_id, peer_id, packet_id).
        let chave = chaves(Role::Client);
        let (mut c, _, _) = par(DataCipher::Aes256Gcm, Some(7), None);
        let dg = c.seal(&[0x45, 9, 9]).unwrap();
        let k = LessSafeKey::new(UnboundKey::new(&AES_256_GCM, &chave.send.cipher[..32]).unwrap());
        let mut n = [0u8; 12];
        n[..4].copy_from_slice(&1u32.to_be_bytes());
        n[4..].copy_from_slice(&chave.send.hmac[..8]);
        let mut buf = dg[24..].to_vec();
        buf.extend_from_slice(&dg[8..24]);
        let claro = k
            .open_in_place(
                Nonce::assume_unique_for_key(n),
                Aad::from(&dg[..8]),
                &mut buf,
            )
            .unwrap();
        assert_eq!(claro, &[0x45, 9, 9]);
    }

    #[test]
    fn um_byte_trocado_em_qualquer_posicao_e_recusado() {
        let (mut c, mut s, agora) = par(DataCipher::ChaCha20Poly1305, Some(3), None);
        let dg = c.seal(&pacote_ip(30)).unwrap();
        for i in 0..dg.len() {
            let mut ruim = dg.clone();
            ruim[i] ^= 0x01;
            assert!(s.open(&ruim, agora).is_err(), "byte {i}");
        }
        // O original ainda passa: os recusados não queimam o id.
        assert!(s.open(&dg, agora).is_ok());
    }

    #[test]
    fn replay_repetido_atrasado_dentro_e_fora_da_janela() {
        let (mut c, mut s, agora) = par(DataCipher::Aes128Gcm, Some(1), None);
        let dgs: Vec<_> = (0..200).map(|_| c.seal(&[0x45, 0]).unwrap()).collect();
        // Fora de ordem dentro da janela: o 3º e depois o 2º e o 1º.
        assert!(s.open(&dgs[2], agora).is_ok());
        assert!(s.open(&dgs[1], agora).is_ok());
        assert!(s.open(&dgs[0], agora).is_ok());
        // Repetido.
        assert_eq!(
            s.open(&dgs[1], agora),
            Err(DataError::Replay(ReplayError::Duplicate))
        );
        // Avança bem além da janela de 64 e tenta um antigo.
        assert!(s.open(&dgs[150], agora).is_ok());
        assert_eq!(
            s.open(&dgs[10], agora),
            Err(DataError::Replay(ReplayError::TooOld))
        );
        assert!(s.rejected() >= 2);
    }

    #[test]
    fn ping_e_occ_nao_viram_pacote_ip() {
        let (mut c, mut s, agora) = par(DataCipher::Aes256Gcm, Some(1), None);
        let ping = c.seal_ping().unwrap();
        assert_eq!(s.open(&ping, agora).unwrap(), Opened::Ping);
        let saida = c.seal_exit().unwrap();
        match s.open(&saida, agora).unwrap() {
            Opened::Occ(p) => {
                assert!(p.starts_with(&OCC_MAGIC));
                assert_eq!(p[16], OCC_EXIT);
            }
            outro => panic!("{outro:?}"),
        }
    }

    #[test]
    fn enquadramento_lzo_desligado_poe_o_byte_fa() {
        let (mut c, mut s, agora) = par(DataCipher::Aes128Gcm, Some(1), Some(Compress::LzoOff));
        let ip = pacote_ip(20);
        let dg = c.seal(&ip).unwrap();
        assert_eq!(dg.len(), 8 + 16 + 1 + ip.len());
        assert_eq!(s.open(&dg, agora).unwrap(), Opened::Ip(ip));
    }

    #[test]
    fn enquadramento_stub_leva_o_primeiro_byte_para_o_fim() {
        let (mut c, mut s, agora) = par(DataCipher::Aes128Gcm, Some(1), Some(Compress::Stub));
        let ip = pacote_ip(20);
        assert_eq!(frame(Some(Compress::Stub), &[1, 2, 3]), vec![0xFB, 2, 3, 1]);
        assert_eq!(
            unframe(Some(Compress::Stub), &[0xFB, 2, 3, 1]).unwrap(),
            vec![1, 2, 3]
        );
        let dg = c.seal(&ip).unwrap();
        assert_eq!(s.open(&dg, agora).unwrap(), Opened::Ip(ip));
        // O ping também passa pelo enquadramento.
        let ping = c.seal_ping().unwrap();
        assert_eq!(s.open(&ping, agora).unwrap(), Opened::Ping);
    }

    #[test]
    fn enquadramento_stub_v2_so_aparece_na_colisao() {
        let (mut c, mut s, agora) = par(DataCipher::Aes128Gcm, Some(1), Some(Compress::StubV2));
        let ip = pacote_ip(20);
        let dg = c.seal(&ip).unwrap();
        assert_eq!(dg.len(), 8 + 16 + ip.len(), "sem colisão não soma nada");
        assert_eq!(s.open(&dg, agora).unwrap(), Opened::Ip(ip));
        // Um pacote que começa com 0x50 ganha o enquadramento de 2 bytes.
        assert_eq!(
            frame(Some(Compress::StubV2), &[0x50, 1]),
            vec![0x50, 0x00, 0x50, 1]
        );
        let colide = vec![0x50, 7, 7, 7];
        let dg = c.seal(&colide).unwrap();
        assert_eq!(dg.len(), 8 + 16 + 2 + colide.len());
        assert_eq!(s.open(&dg, agora).unwrap(), Opened::Ip(colide));
    }

    #[test]
    fn carga_comprimida_vira_o_erro_da_mensagem() {
        assert_eq!(
            unframe(Some(Compress::StubV2), &[0x50, 0x01, 9, 9]),
            Err(DataError::Compressed("LZ4"))
        );
        assert_eq!(
            unframe(Some(Compress::LzoOff), &[0x66, 9, 9]),
            Err(DataError::Compressed("LZO"))
        );
        assert_eq!(
            unframe(Some(Compress::Lz4), &[0x69, 9, 9]),
            Err(DataError::Compressed("LZ4"))
        );
        let msg = DataError::Compressed("LZO").to_string();
        assert!(
            msg.contains("comprime com LZO") && msg.contains("compress stub-v2"),
            "{msg}"
        );
    }

    #[test]
    fn a_geracao_antiga_vale_so_ate_o_tran_window() {
        let (mut c, mut s, agora) = par(DataCipher::Aes256Gcm, Some(1), None);
        let velho = c.seal(&pacote_ip(10)).unwrap();
        // Chaves novas (key_id 1) dos dois lados.
        let mut bloco = [0u8; KEY_BLOCK_LEN];
        bloco
            .iter_mut()
            .enumerate()
            .for_each(|(i, b)| *b = (i * 13 + 1) as u8);
        c.install(1, &DataKeys::from_block(&bloco, Role::Client), agora)
            .unwrap();
        s.install(1, &DataKeys::from_block(&bloco, Role::Server), agora)
            .unwrap();
        assert_eq!(c.current_key(), Some(1));
        // A geração velha ainda decifra, a nova também.
        assert!(s.open(&velho, agora).is_ok());
        let novo = c.seal(&pacote_ip(10)).unwrap();
        assert_eq!(novo[0] & 7, 1);
        assert!(s.open(&novo, agora).is_ok());
        // Depois do prazo, a velha cai.
        let depois = agora + Duration::from_secs(61);
        assert_eq!(s.next_expiry(), Some(agora + Duration::from_secs(60)));
        s.expire(depois);
        let outro_velho = {
            let (mut c0, _, _) = par(DataCipher::Aes256Gcm, Some(1), None);
            c0.seal(&pacote_ip(10)).unwrap()
        };
        assert_eq!(s.open(&outro_velho, depois), Err(DataError::UnknownKey(0)));
    }

    #[test]
    fn sem_chave_nao_envia_e_a_chave_desconhecida_e_recusada() {
        let mut vazio = DataChannel::new(opcoes(DataCipher::Aes128Gcm, Some(1), None));
        assert_eq!(vazio.seal(&[0x45]), Err(DataError::NoKeys));
        assert!(!vazio.ready());
        let agora = Instant::now();
        assert_eq!(
            vazio.open(
                &[
                    9 << 3 | 2,
                    0,
                    0,
                    1,
                    0,
                    0,
                    0,
                    1,
                    0,
                    0,
                    0,
                    0,
                    0,
                    0,
                    0,
                    0,
                    0,
                    0,
                    0,
                    0,
                    0,
                    0,
                    0,
                    0,
                    0
                ],
                agora
            ),
            Err(DataError::UnknownKey(2))
        );
        assert_eq!(vazio.open(&[9 << 3], agora), Err(DataError::Malformed));
        assert_eq!(vazio.open(&[], agora), Err(DataError::Malformed));
    }

    #[test]
    fn renegocia_por_bytes_por_pacotes_e_perto_do_fim_do_id() {
        let agora = Instant::now();
        let mut o = opcoes(DataCipher::Aes128Gcm, Some(1), None);
        o.reneg_pkts = Some(3);
        let mut c = DataChannel::new(o);
        c.install(0, &chaves(Role::Client), agora).unwrap();
        for _ in 0..3 {
            assert!(!c.needs_renegotiation());
            c.seal(&[0x45]).unwrap();
        }
        assert!(c.needs_renegotiation());

        let mut o = opcoes(DataCipher::Aes128Gcm, Some(1), None);
        o.reneg_bytes = Some(100);
        let mut c = DataChannel::new(o);
        c.install(0, &chaves(Role::Client), agora).unwrap();
        c.seal(&pacote_ip(60)).unwrap();
        assert!(!c.needs_renegotiation());
        c.seal(&pacote_ip(60)).unwrap();
        assert!(c.needs_renegotiation());

        let mut c = DataChannel::new(opcoes(DataCipher::Aes128Gcm, Some(1), None));
        c.install(0, &chaves(Role::Client), agora).unwrap();
        c.generations[0].next_id = RENEGOTIATE_AT_ID;
        assert!(c.needs_renegotiation());
        c.generations[0].next_id = u32::MAX;
        assert_eq!(c.seal(&[0x45]), Err(DataError::Exhausted));
    }

    #[test]
    fn nomes_das_cifras() {
        assert_eq!(
            DataCipher::from_name("aes-256-gcm"),
            Some(DataCipher::Aes256Gcm)
        );
        assert_eq!(DataCipher::from_name("BF-CBC"), None);
        for c in CIFRAS {
            assert_eq!(DataCipher::from_name(c.name()), Some(c));
        }
    }
}
