//! O Noise IX do Nebula (`Noise_IX_25519_{AESGCM|ChaChaPoly}_SHA256`, sem chave pré-compartilhada
//! e com o prólogo vazio) e a cifra dos pacotes de dados.
//!
//! A especificação é a do Noise (noiseprotocol.org, revisão 34). Escrito aqui, e não sobre uma
//! biblioteca de Noise, por duas razões do Nebula: o pacote de dados autentica o cabeçalho como
//! dado associado (o `TransportState` das bibliotecas não aceita), e o aperto de mão precisa
//! voltar atrás quando chega lixo no lugar da resposta (o estado é `Clone`, a leitura que falha
//! não deixa marca). Conferido contra os vetores de teste do `cacophony` (veja os testes).
//!
//! Só a curva 25519: o P-256 fica para depois (M48).

use std::fmt;

use aes_gcm::Aes256Gcm;
use aes_gcm::KeyInit;
use aes_gcm::aead::{Aead, Payload};
use chacha20poly1305::ChaCha20Poly1305;
use sha2::{Digest, Sha256};
use zeroize::Zeroizing;

use crate::config::Cipher;

/// O tamanho da chave, da chave pública e do segredo da curva 25519 (do protocolo).
pub(crate) const KEY_LEN: usize = 32;
/// O tamanho da etiqueta de autenticação do AES-GCM e do ChaCha20-Poly1305 (do protocolo).
pub(crate) const TAG_LEN: usize = 16;
/// O tamanho do hash SHA-256 (do protocolo).
const HASH_LEN: usize = 32;
/// O tamanho do bloco do SHA-256, que o HMAC usa (do protocolo).
const BLOCK_LEN: usize = 64;

/// Por que um aperto de mão Noise falhou.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum NoiseError {
    /// A mensagem é menor do que o padrão exige.
    TooShort,
    /// A troca de chaves deu zero (chave pública inválida).
    BadPublicKey,
    /// A autenticação falhou (chave errada, mensagem alterada ou de outra rede).
    Authentication,
    /// A mensagem não é a que o estado espera.
    OutOfOrder,
}

impl fmt::Display for NoiseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            NoiseError::TooShort => "a mensagem do aperto de mão é curta demais",
            NoiseError::BadPublicKey => "a chave pública do aperto de mão é inválida",
            NoiseError::Authentication => "a autenticação do aperto de mão falhou",
            NoiseError::OutOfOrder => "a mensagem do aperto de mão veio fora de ordem",
        })
    }
}

impl std::error::Error for NoiseError {}

/// O nome do protocolo, que entra no início do hash.
fn protocol_name(cipher: Cipher) -> &'static str {
    match cipher {
        Cipher::Aes => "Noise_IX_25519_AESGCM_SHA256",
        Cipher::ChaChaPoly => "Noise_IX_25519_ChaChaPoly_SHA256",
    }
}

/// O HMAC-SHA256 (RFC 2104) sobre os pedaços de `data` em sequência.
fn hmac(key: &[u8], data: &[&[u8]]) -> [u8; HASH_LEN] {
    let mut block = [0u8; BLOCK_LEN];
    if key.len() > BLOCK_LEN {
        block[..HASH_LEN].copy_from_slice(&Sha256::digest(key));
    } else {
        block[..key.len()].copy_from_slice(key);
    }
    let mut inner = Sha256::new();
    inner.update(block.map(|b| b ^ 0x36));
    for pedaco in data {
        inner.update(pedaco);
    }
    let inner = inner.finalize();
    let mut outer = Sha256::new();
    outer.update(block.map(|b| b ^ 0x5c));
    outer.update(inner);
    outer.finalize().into()
}

/// O HKDF do Noise com duas saídas (`HKDF(chaining_key, input, 2)`).
fn hkdf2(
    chaining_key: &[u8; HASH_LEN],
    input: &[u8],
) -> (Zeroizing<[u8; HASH_LEN]>, Zeroizing<[u8; HASH_LEN]>) {
    let temp = Zeroizing::new(hmac(chaining_key, &[input]));
    let out1 = hmac(temp.as_slice(), &[&[1]]);
    let out2 = hmac(temp.as_slice(), &[&out1, &[2]]);
    (Zeroizing::new(out1), Zeroizing::new(out2))
}

/// A cifra AEAD de uma chave, com o nonce no formato do Noise: quatro bytes zero e o contador de
/// 64 bits, em big-endian no AES-GCM e em little-endian no ChaCha20-Poly1305.
pub(crate) enum DataCipher {
    /// AES-256-GCM.
    Aes(Box<Aes256Gcm>),
    /// ChaCha20-Poly1305.
    ChaCha(Box<ChaCha20Poly1305>),
}

impl DataCipher {
    /// A cifra de `cipher` com a `key`.
    pub(crate) fn new(cipher: Cipher, key: &[u8; KEY_LEN]) -> DataCipher {
        // O tamanho da chave está no tipo, então `new_from_slice` não falha.
        match cipher {
            Cipher::Aes => DataCipher::Aes(Box::new(
                Aes256Gcm::new_from_slice(key).unwrap_or_else(|_| unreachable!("32 bytes")),
            )),
            Cipher::ChaChaPoly => DataCipher::ChaCha(Box::new(
                ChaCha20Poly1305::new_from_slice(key).unwrap_or_else(|_| unreachable!("32 bytes")),
            )),
        }
    }

    /// Cifra `plaintext` com o contador `counter` e o dado associado `ad`; a etiqueta vai no fim.
    pub(crate) fn seal(&self, counter: u64, ad: &[u8], plaintext: &[u8]) -> Option<Vec<u8>> {
        let payload = Payload {
            msg: plaintext,
            aad: ad,
        };
        match self {
            DataCipher::Aes(c) => c.encrypt(&nonce(counter, true).into(), payload).ok(),
            DataCipher::ChaCha(c) => c.encrypt(&nonce(counter, false).into(), payload).ok(),
        }
    }

    /// Decifra e confere. `None` se a etiqueta não bate.
    pub(crate) fn open(&self, counter: u64, ad: &[u8], ciphertext: &[u8]) -> Option<Vec<u8>> {
        let payload = Payload {
            msg: ciphertext,
            aad: ad,
        };
        match self {
            DataCipher::Aes(c) => c.decrypt(&nonce(counter, true).into(), payload).ok(),
            DataCipher::ChaCha(c) => c.decrypt(&nonce(counter, false).into(), payload).ok(),
        }
    }
}

/// O nonce de 12 bytes de um contador.
fn nonce(counter: u64, big_endian: bool) -> [u8; 12] {
    let mut n = [0u8; 12];
    n[4..].copy_from_slice(&if big_endian {
        counter.to_be_bytes()
    } else {
        counter.to_le_bytes()
    });
    n
}

/// A troca de chaves X25519, recusando o resultado zero (ponto de ordem baixa).
fn dh(
    secret: &[u8; KEY_LEN],
    public: &[u8; KEY_LEN],
) -> Result<Zeroizing<[u8; KEY_LEN]>, NoiseError> {
    let out = Zeroizing::new(x25519_dalek::x25519(*secret, *public));
    if out.iter().all(|b| *b == 0) {
        return Err(NoiseError::BadPublicKey);
    }
    Ok(out)
}

/// O estado simétrico do Noise (`SymmetricState`).
#[derive(Clone)]
struct Symmetric {
    cipher: Cipher,
    chaining_key: [u8; HASH_LEN],
    hash: [u8; HASH_LEN],
    key: Option<Zeroizing<[u8; KEY_LEN]>>,
    nonce: u64,
}

impl Symmetric {
    fn new(cipher: Cipher, prologue: &[u8]) -> Symmetric {
        let name = protocol_name(cipher).as_bytes();
        let mut hash = [0u8; HASH_LEN];
        if name.len() <= HASH_LEN {
            hash[..name.len()].copy_from_slice(name);
        } else {
            hash = Sha256::digest(name).into();
        }
        let mut s = Symmetric {
            cipher,
            chaining_key: hash,
            hash,
            key: None,
            nonce: 0,
        };
        s.mix_hash(prologue);
        s
    }

    fn mix_hash(&mut self, data: &[u8]) {
        let mut sha = Sha256::new();
        sha.update(self.hash);
        sha.update(data);
        self.hash = sha.finalize().into();
    }

    fn mix_key(&mut self, input: &[u8]) {
        let (ck, key) = hkdf2(&self.chaining_key, input);
        self.chaining_key = *ck;
        self.key = Some(key);
        self.nonce = 0;
    }

    fn encrypt_and_hash(&mut self, plaintext: &[u8], out: &mut Vec<u8>) -> Result<(), NoiseError> {
        let start = out.len();
        match &self.key {
            None => out.extend_from_slice(plaintext),
            Some(key) => {
                let sealed = DataCipher::new(self.cipher, key)
                    .seal(self.nonce, &self.hash, plaintext)
                    .ok_or(NoiseError::Authentication)?;
                self.nonce += 1;
                out.extend_from_slice(&sealed);
            }
        }
        let escrito = out[start..].to_vec();
        self.mix_hash(&escrito);
        Ok(())
    }

    fn decrypt_and_hash(&mut self, ciphertext: &[u8]) -> Result<Vec<u8>, NoiseError> {
        let plaintext = match &self.key {
            None => ciphertext.to_vec(),
            Some(key) => {
                let aberto = DataCipher::new(self.cipher, key)
                    .open(self.nonce, &self.hash, ciphertext)
                    .ok_or(NoiseError::Authentication)?;
                self.nonce += 1;
                aberto
            }
        };
        self.mix_hash(ciphertext);
        Ok(plaintext)
    }

    /// O `Split`: a chave do iniciador para o respondedor e a do respondedor para o iniciador.
    fn split(&self) -> (Zeroizing<[u8; KEY_LEN]>, Zeroizing<[u8; KEY_LEN]>) {
        hkdf2(&self.chaining_key, &[])
    }
}

/// As chaves de transporte que o aperto de mão entrega.
pub(crate) struct Split {
    /// Cifra o que o iniciador envia.
    pub(crate) initiator_to_responder: Zeroizing<[u8; KEY_LEN]>,
    /// Cifra o que o respondedor envia.
    pub(crate) responder_to_initiator: Zeroizing<[u8; KEY_LEN]>,
}

/// O aperto de mão Noise IX (duas mensagens):
///
/// ```text
/// -> e, s
/// <- e, ee, se, s, es
/// ```
#[derive(Clone)]
pub(crate) struct Ix {
    sym: Symmetric,
    initiator: bool,
    static_secret: Zeroizing<[u8; KEY_LEN]>,
    static_public: [u8; KEY_LEN],
    ephemeral: Option<(Zeroizing<[u8; KEY_LEN]>, [u8; KEY_LEN])>,
    remote_ephemeral: Option<[u8; KEY_LEN]>,
    remote_static: Option<[u8; KEY_LEN]>,
    /// Quantas mensagens já foram trocadas.
    messages: usize,
}

impl Ix {
    /// Um aperto de mão novo. `static_secret` e `static_public` são o par de chaves do nó.
    pub(crate) fn new(
        cipher: Cipher,
        initiator: bool,
        static_secret: [u8; KEY_LEN],
        static_public: [u8; KEY_LEN],
        prologue: &[u8],
    ) -> Ix {
        Ix {
            sym: Symmetric::new(cipher, prologue),
            initiator,
            static_secret: Zeroizing::new(static_secret),
            static_public,
            ephemeral: None,
            remote_ephemeral: None,
            remote_static: None,
            messages: 0,
        }
    }

    /// Usa esta chave efêmera em vez de sortear uma (só para os vetores de teste).
    #[cfg(test)]
    fn with_ephemeral(mut self, secret: [u8; KEY_LEN]) -> Ix {
        let public = x25519_dalek::x25519(secret, x25519_dalek::X25519_BASEPOINT_BYTES);
        self.ephemeral = Some((Zeroizing::new(secret), public));
        self
    }

    /// A chave pública do outro lado, depois de lida a mensagem que a traz.
    pub(crate) fn remote_static(&self) -> Option<[u8; KEY_LEN]> {
        self.remote_static
    }

    /// Quantas mensagens do aperto de mão já foram escritas ou lidas.
    pub(crate) fn message_index(&self) -> usize {
        self.messages
    }

    /// O hash do aperto de mão (só os testes leem).
    #[cfg(test)]
    fn handshake_hash(&self) -> [u8; HASH_LEN] {
        self.sym.hash
    }

    /// O aperto de mão terminou.
    pub(crate) fn is_finished(&self) -> bool {
        self.messages == 2
    }

    fn my_turn(&self) -> bool {
        self.messages % 2 == usize::from(!self.initiator)
    }

    fn ephemeral_pair(&mut self) -> Result<([u8; KEY_LEN], [u8; KEY_LEN]), NoiseError> {
        if self.ephemeral.is_none() {
            let mut secret = Zeroizing::new([0u8; KEY_LEN]);
            getrandom::fill(secret.as_mut_slice()).map_err(|_| NoiseError::Authentication)?;
            let public = x25519_dalek::x25519(*secret, x25519_dalek::X25519_BASEPOINT_BYTES);
            self.ephemeral = Some((secret, public));
        }
        let (secret, public) = self
            .ephemeral
            .as_ref()
            .unwrap_or_else(|| unreachable!("acabou de ser criada"));
        Ok((**secret, *public))
    }

    /// Escreve a próxima mensagem, com `payload` cifrado dentro dela. Se escreve a última, o
    /// aperto de mão termina e `split` vale.
    pub(crate) fn write_message(&mut self, payload: &[u8]) -> Result<Vec<u8>, NoiseError> {
        if !self.my_turn() || self.is_finished() {
            return Err(NoiseError::OutOfOrder);
        }
        let mut out = Vec::with_capacity(KEY_LEN * 2 + TAG_LEN * 2 + payload.len());
        let (e_secret, e_public) = self.ephemeral_pair()?;
        self.sym.mix_hash(&e_public);
        out.extend_from_slice(&e_public);
        if self.initiator {
            // -> e, s
            self.sym
                .encrypt_and_hash(&self.static_public.clone(), &mut out)?;
        } else {
            // <- e, ee, se, s, es
            let (re, rs) = self.remotes()?;
            self.sym.mix_key(dh(&e_secret, &re)?.as_slice());
            self.sym.mix_key(dh(&e_secret, &rs)?.as_slice());
            self.sym
                .encrypt_and_hash(&self.static_public.clone(), &mut out)?;
            let secret = *self.static_secret;
            self.sym.mix_key(dh(&secret, &re)?.as_slice());
        }
        self.sym.encrypt_and_hash(payload, &mut out)?;
        self.messages += 1;
        Ok(out)
    }

    /// Lê a próxima mensagem e devolve a carga. Se falha, o estado fica como estava (dá para
    /// tentar outra mensagem).
    pub(crate) fn read_message(&mut self, message: &[u8]) -> Result<Vec<u8>, NoiseError> {
        if self.my_turn() || self.is_finished() {
            return Err(NoiseError::OutOfOrder);
        }
        let mut tentativa = self.clone();
        let payload = tentativa.read_inner(message)?;
        *self = tentativa;
        Ok(payload)
    }

    fn remotes(&self) -> Result<([u8; KEY_LEN], [u8; KEY_LEN]), NoiseError> {
        match (self.remote_ephemeral, self.remote_static) {
            (Some(re), Some(rs)) => Ok((re, rs)),
            _ => Err(NoiseError::OutOfOrder),
        }
    }

    fn read_inner(&mut self, message: &[u8]) -> Result<Vec<u8>, NoiseError> {
        if message.len() < KEY_LEN {
            return Err(NoiseError::TooShort);
        }
        let re: [u8; KEY_LEN] = message[..KEY_LEN]
            .try_into()
            .unwrap_or_else(|_| unreachable!("32 bytes"));
        self.sym.mix_hash(&re);
        self.remote_ephemeral = Some(re);
        let mut resto = &message[KEY_LEN..];
        if self.initiator {
            // <- e, ee, se, s, es
            let (e_secret, _) = self.ephemeral_pair()?;
            self.sym.mix_key(dh(&e_secret, &re)?.as_slice());
            let secret = *self.static_secret;
            self.sym.mix_key(dh(&secret, &re)?.as_slice());
            if resto.len() < KEY_LEN + TAG_LEN {
                return Err(NoiseError::TooShort);
            }
            let (s, depois) = resto.split_at(KEY_LEN + TAG_LEN);
            let rs = self.sym.decrypt_and_hash(s)?;
            let rs: [u8; KEY_LEN] = rs
                .as_slice()
                .try_into()
                .map_err(|_| NoiseError::Authentication)?;
            self.remote_static = Some(rs);
            self.sym.mix_key(dh(&e_secret, &rs)?.as_slice());
            resto = depois;
        } else {
            // -> e, s (sem chave ainda: a estática vai às claras)
            if resto.len() < KEY_LEN {
                return Err(NoiseError::TooShort);
            }
            let (s, depois) = resto.split_at(KEY_LEN);
            let rs = self.sym.decrypt_and_hash(s)?;
            let rs: [u8; KEY_LEN] = rs
                .as_slice()
                .try_into()
                .map_err(|_| NoiseError::Authentication)?;
            self.remote_static = Some(rs);
            resto = depois;
        }
        let payload = self.sym.decrypt_and_hash(resto)?;
        self.messages += 1;
        Ok(payload)
    }

    /// As chaves de transporte. Só vale com o aperto de mão terminado.
    pub(crate) fn split(&self) -> Option<Split> {
        if !self.is_finished() {
            return None;
        }
        let (i2r, r2i) = self.sym.split();
        Some(Split {
            initiator_to_responder: i2r,
            responder_to_initiator: r2i,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Vetor {
        nome: &'static str,
        prologo: &'static str,
        i_estatica: &'static str,
        i_efemera: &'static str,
        r_estatica: &'static str,
        r_efemera: &'static str,
        hash: &'static str,
        mensagens: [(&'static str, &'static str); 4],
    }

    /// Os vetores do `cacophony` (domínio público) para o IX com as duas cifras.
    const VETORES: &[Vetor] = &[
        Vetor {
            nome: "Noise_IX_25519_AESGCM_SHA256",
            prologo: "4a6f686e2047616c74",
            i_estatica: "e61ef9919cde45dd5f82166404bd08e38bceb5dfdfded0a34c8df7ed542214d1",
            i_efemera: "893e28b9dc6ca8d611ab664754b8ceb7bac5117349a4439a6b0569da977c464a",
            r_estatica: "4a3acbfdb163dec651dfa3194dece676d437029c62a408b4c5ea9114246e4893",
            r_efemera: "bbdb4cdbd309f1a1f2e1456967fe288cadd6f712d65dc7b7793d5e63da6b375b",
            hash: "a59954350219531eec9452c90afca10db17192bbd066a936c89f81490a77ba05",
            mensagens: [
                (
                    "4c756477696720766f6e204d69736573",
                    "ca35def5ae56cec33dc2036731ab14896bc4c75dbb07a61f879f8e3afa4c79446bc3822a2aa7f4e6981d6538692b3cdf3e6df9eea6ed269eb41d93c22757b75a4c756477696720766f6e204d69736573",
                ),
                (
                    "4d757272617920526f746862617264",
                    "95ebc60d2b1fa672c1f46a8aa265ef51bfe38e7ccb39ec5be34069f14480884381f5e7fd3177b25f05504a881c3d9abb4ceaa6e8ab72f7925bc1ecee7f533d4d83453c68b45114148f3d11c0281c89906e4c23267b3d038eeb789a7ade71edd08ef12236e741eda973e5baba4bc005",
                ),
                (
                    "462e20412e20486179656b",
                    "60faff0c912ae6003ff00295f240b28e8f68f3442e26661701cd7f",
                ),
                (
                    "4361726c204d656e676572",
                    "746cac398c37a680d8ee7836a7826a164383f562bcac347bbde807",
                ),
            ],
        },
        Vetor {
            nome: "Noise_IX_25519_ChaChaPoly_SHA256",
            prologo: "4a6f686e2047616c74",
            i_estatica: "e61ef9919cde45dd5f82166404bd08e38bceb5dfdfded0a34c8df7ed542214d1",
            i_efemera: "893e28b9dc6ca8d611ab664754b8ceb7bac5117349a4439a6b0569da977c464a",
            r_estatica: "4a3acbfdb163dec651dfa3194dece676d437029c62a408b4c5ea9114246e4893",
            r_efemera: "bbdb4cdbd309f1a1f2e1456967fe288cadd6f712d65dc7b7793d5e63da6b375b",
            hash: "c95696b7e335ad2ef3b5a35cb407b40c6376ee4f39c4619ffa37929b6dd8026d",
            mensagens: [
                (
                    "4c756477696720766f6e204d69736573",
                    "ca35def5ae56cec33dc2036731ab14896bc4c75dbb07a61f879f8e3afa4c79446bc3822a2aa7f4e6981d6538692b3cdf3e6df9eea6ed269eb41d93c22757b75a4c756477696720766f6e204d69736573",
                ),
                (
                    "4d757272617920526f746862617264",
                    "95ebc60d2b1fa672c1f46a8aa265ef51bfe38e7ccb39ec5be34069f144808843db451ba0cc81ba55f01e5aeb04e3748f337344ed2a494219a3fae8ef756f95054f06f10bbe3e8a27bdf263fc314e16c300bf822646c34d35641d9635ea993c4694966ab721281c5093bc5d3831bf0a",
                ),
                (
                    "462e20412e20486179656b",
                    "90a3ae2a6f1c0f3c2b7a81c5ddfb3a068376a18b9267745459497b",
                ),
                (
                    "4361726c204d656e676572",
                    "a54a54e469da6914ec8edeb1f2c1fc7434ab6a4834a0736b34fd9e",
                ),
            ],
        },
    ];

    fn chave(hex: &str) -> [u8; 32] {
        crate::util::unhex(hex).unwrap().try_into().unwrap()
    }

    fn par(hex: &str) -> ([u8; 32], [u8; 32]) {
        let s = chave(hex);
        (
            s,
            x25519_dalek::x25519(s, x25519_dalek::X25519_BASEPOINT_BYTES),
        )
    }

    #[test]
    fn hmac_confere_com_os_vetores_da_rfc_4231() {
        // Caso 1.
        let r = hmac(&[0x0b; 20], &[b"Hi There"]);
        assert_eq!(
            crate::util::hex(&r),
            "b0344c61d8db38535ca8afceaf0bf12b881dc200c9833da726e9376c2e32cff7"
        );
        // Caso 2 (a chave "Jefe").
        let r = hmac(b"Jefe", &[b"what do ya want ", b"for nothing?"]);
        assert_eq!(
            crate::util::hex(&r),
            "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843"
        );
        // Caso 6: chave maior que o bloco.
        let r = hmac(
            &[0xaa; 131],
            &[b"Test Using Larger Than Block-Size Key - Hash Key First"],
        );
        assert_eq!(
            crate::util::hex(&r),
            "60e431591ee0b67f0d8a26aacbf5b77f8e0bc6213728c5140546040f0ee37f54"
        );
    }

    #[test]
    fn confere_com_os_vetores_do_noise() {
        for v in VETORES {
            let cipher = if v.nome.contains("AESGCM") {
                Cipher::Aes
            } else {
                Cipher::ChaChaPoly
            };
            let prologo = crate::util::unhex(v.prologo).unwrap();
            let (is, ip) = par(v.i_estatica);
            let (rs, rp) = par(v.r_estatica);
            let mut i = Ix::new(cipher, true, is, ip, &prologo).with_ephemeral(chave(v.i_efemera));
            let mut r = Ix::new(cipher, false, rs, rp, &prologo).with_ephemeral(chave(v.r_efemera));

            let p0 = crate::util::unhex(v.mensagens[0].0).unwrap();
            let m0 = i.write_message(&p0).unwrap();
            assert_eq!(crate::util::hex(&m0), v.mensagens[0].1, "{} msg 0", v.nome);
            assert_eq!(r.read_message(&m0).unwrap(), p0);
            assert_eq!(r.remote_static(), Some(ip));

            let p1 = crate::util::unhex(v.mensagens[1].0).unwrap();
            let m1 = r.write_message(&p1).unwrap();
            assert_eq!(crate::util::hex(&m1), v.mensagens[1].1, "{} msg 1", v.nome);
            assert_eq!(i.read_message(&m1).unwrap(), p1);
            assert_eq!(i.remote_static(), Some(rp));

            assert_eq!(crate::util::hex(&i.handshake_hash()), v.hash);
            assert_eq!(crate::util::hex(&r.handshake_hash()), v.hash);

            // As duas mensagens seguintes já são de transporte (contador 0, sem dado associado).
            let si = i.split().unwrap();
            let sr = r.split().unwrap();
            let p2 = crate::util::unhex(v.mensagens[2].0).unwrap();
            let c = DataCipher::new(cipher, &si.initiator_to_responder);
            assert_eq!(
                crate::util::hex(&c.seal(0, &[], &p2).unwrap()),
                v.mensagens[2].1,
                "{} msg 2",
                v.nome
            );
            let p3 = crate::util::unhex(v.mensagens[3].0).unwrap();
            let c = DataCipher::new(cipher, &sr.responder_to_initiator);
            assert_eq!(
                crate::util::hex(&c.seal(0, &[], &p3).unwrap()),
                v.mensagens[3].1,
                "{} msg 3",
                v.nome
            );
        }
    }

    fn par_de_nos(cipher: Cipher) -> (Ix, Ix) {
        let (is, ip) = par(&"11".repeat(32));
        let (rs, rp) = par(&"22".repeat(32));
        (
            Ix::new(cipher, true, is, ip, &[]),
            Ix::new(cipher, false, rs, rp, &[]),
        )
    }

    #[test]
    fn as_duas_cifras_fecham_o_aperto_de_mao() {
        for cipher in [Cipher::Aes, Cipher::ChaChaPoly] {
            let (mut i, mut r) = par_de_nos(cipher);
            let m0 = i.write_message(b"ola").unwrap();
            assert_eq!(r.read_message(&m0).unwrap(), b"ola");
            let m1 = r.write_message(b"oi").unwrap();
            assert_eq!(i.read_message(&m1).unwrap(), b"oi");
            let (si, sr) = (i.split().unwrap(), r.split().unwrap());
            assert_eq!(*si.initiator_to_responder, *sr.initiator_to_responder);
            assert_eq!(*si.responder_to_initiator, *sr.responder_to_initiator);
            assert_ne!(*si.initiator_to_responder, *si.responder_to_initiator);
        }
    }

    #[test]
    fn leitura_que_falha_nao_deixa_marca() {
        let (mut i, mut r) = par_de_nos(Cipher::Aes);
        let m0 = i.write_message(b"ola").unwrap();
        r.read_message(&m0).unwrap();
        let m1 = r.write_message(b"oi").unwrap();
        // Lixo no lugar da resposta (e uma resposta adulterada).
        assert!(i.read_message(&[7u8; 120]).is_err());
        let mut ruim = m1.clone();
        let ultimo = ruim.len() - 1;
        ruim[ultimo] ^= 1;
        assert_eq!(i.read_message(&ruim), Err(NoiseError::Authentication));
        assert!(i.read_message(&[1, 2, 3]).is_err());
        assert_eq!(i.message_index(), 1);
        // A resposta de verdade ainda fecha.
        assert_eq!(i.read_message(&m1).unwrap(), b"oi");
        assert!(i.is_finished());
    }

    #[test]
    fn fora_de_ordem_e_recusado() {
        let (mut i, mut r) = par_de_nos(Cipher::Aes);
        assert_eq!(r.write_message(b"x"), Err(NoiseError::OutOfOrder));
        assert_eq!(i.read_message(&[0; 100]), Err(NoiseError::OutOfOrder));
        let m0 = i.write_message(b"x").unwrap();
        assert_eq!(i.write_message(b"x"), Err(NoiseError::OutOfOrder));
        r.read_message(&m0).unwrap();
        assert!(i.split().is_none());
    }

    #[test]
    fn chave_publica_de_ordem_baixa_e_recusada() {
        let (mut i, mut r) = par_de_nos(Cipher::Aes);
        let m0 = i.write_message(b"x").unwrap();
        r.read_message(&m0).unwrap();
        let mut m1 = r.write_message(b"y").unwrap();
        // A efêmera do respondedor vira o ponto zero.
        m1[..32].fill(0);
        assert_eq!(i.read_message(&m1), Err(NoiseError::BadPublicKey));
    }

    #[test]
    fn o_nonce_do_aes_e_big_endian_e_o_do_chacha_little_endian() {
        assert_eq!(nonce(1, true), [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1]);
        assert_eq!(nonce(1, false), [0, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0]);
        let chave = [9u8; 32];
        for cipher in [Cipher::Aes, Cipher::ChaChaPoly] {
            let c = DataCipher::new(cipher, &chave);
            let selado = c.seal(0x0102_0304_0506_0708, b"cab", b"dados").unwrap();
            assert_eq!(selado.len(), 5 + TAG_LEN);
            assert_eq!(
                c.open(0x0102_0304_0506_0708, b"cab", &selado).unwrap(),
                b"dados"
            );
            assert!(c.open(0x0102_0304_0506_0709, b"cab", &selado).is_none());
            assert!(c.open(0x0102_0304_0506_0708, b"CAB", &selado).is_none());
        }
    }
}
