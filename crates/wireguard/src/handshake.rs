//! O handshake `Noise_IKpsk2_25519_ChaChaPoly_BLAKE2s` do WireGuard (artigo, seção 5.4).
//!
//! Está escrito à mão, e não sobre uma biblioteca de Noise, por um motivo: no WireGuard quem
//! responde só descobre qual par está do outro lado (e, portanto, qual a chave
//! pré-compartilhada dele) depois de abrir a primeira mensagem. As bibliotecas de Noise
//! fixam a PSK antes do handshake. A conferência de que está certo é o teste de
//! interoperabilidade com o `boringtun`.
//!
//! Aqui ficam só as contas (funções puras sobre bytes); o estado de cada par, os relógios e as
//! filas ficam em `peer.rs`.

use std::net::SocketAddr;

use subtle::ConstantTimeEq;
use zeroize::Zeroizing;

use crate::crypto::{aead_open, aead_seal, hash, kdf1, kdf2, kdf3, mac16, xaead_open, xaead_seal};
use crate::key::{KeyError, PresharedKey, PrivateKey, PublicKey, random_bytes};

/// O nome da construção do Noise (artigo, seção 5.4).
const CONSTRUCAO: &[u8] = b"Noise_IKpsk2_25519_ChaChaPoly_BLAKE2s";
/// O identificador do protocolo.
const IDENTIFICADOR: &[u8] = b"WireGuard v1 zx2c4 Jason@zx2c4.com";
/// O rótulo da chave do `mac1`.
const ROTULO_MAC1: &[u8] = b"mac1----";
/// O rótulo da chave do cookie.
const ROTULO_COOKIE: &[u8] = b"cookie--";

/// O tipo da mensagem de iniciação.
pub(crate) const TIPO_INICIACAO: u32 = 1;
/// O tipo da mensagem de resposta.
pub(crate) const TIPO_RESPOSTA: u32 = 2;
/// O tipo da resposta de cookie.
pub(crate) const TIPO_COOKIE: u32 = 3;
/// O tipo da mensagem de transporte.
pub(crate) const TIPO_TRANSPORTE: u32 = 4;

/// O tamanho da iniciação.
pub const TAMANHO_INICIACAO: usize = 148;
/// O tamanho da resposta.
pub const TAMANHO_RESPOSTA: usize = 92;
/// O tamanho da resposta de cookie.
pub const TAMANHO_COOKIE: usize = 64;

/// O estado inicial do Noise: `(Ci, Hi)`.
fn estado_inicial() -> ([u8; 32], [u8; 32]) {
    let ci = hash(&[CONSTRUCAO]);
    let hi = hash(&[&ci, IDENTIFICADOR]);
    (ci, hi)
}

/// A identidade deste ponto: a chave privada e o que se deriva só da pública.
///
/// Compartilhada (por `Arc`) entre o [`Device`](crate::Device), o `Responder` e os pares.
pub struct Identity {
    private: PrivateKey,
    public: PublicKey,
    /// A chave com que o `mac1` das mensagens que chegam para nós é conferido.
    mac1_key: [u8; 32],
    /// A chave com que o cookie que mandamos é cifrado.
    cookie_key: [u8; 32],
}

impl Identity {
    /// Monta a identidade a partir da chave privada.
    pub fn new(private: PrivateKey) -> Self {
        let public = private.public_key();
        Identity {
            mac1_key: hash(&[ROTULO_MAC1, public.as_bytes()]),
            cookie_key: hash(&[ROTULO_COOKIE, public.as_bytes()]),
            private,
            public,
        }
    }

    /// A chave pública que corresponde à privada.
    pub fn public(&self) -> PublicKey {
        self.public
    }
}

/// O que se calcula uma vez por par: a PSK e o Diffie-Hellman estático.
pub(crate) struct PeerKeys {
    public: PublicKey,
    psk: Zeroizing<[u8; 32]>,
    static_static: Zeroizing<[u8; 32]>,
    /// A chave com que o `mac1` das mensagens que mandamos a este par é feito.
    mac1_key: [u8; 32],
    /// A chave com que o cookie vindo deste par é aberto.
    cookie_key: [u8; 32],
}

impl PeerKeys {
    pub(crate) fn new(
        id: &Identity,
        public: PublicKey,
        psk: Option<&PresharedKey>,
    ) -> Result<Self, KeyError> {
        let static_static = id.private.diffie_hellman(&public)?;
        Ok(PeerKeys {
            psk: Zeroizing::new(psk.map(|p| *p.as_bytes()).unwrap_or([0; 32])),
            static_static,
            mac1_key: hash(&[ROTULO_MAC1, public.as_bytes()]),
            cookie_key: hash(&[ROTULO_COOKIE, public.as_bytes()]),
            public,
        })
    }
}

/// As chaves de transporte que saem de um handshake.
pub(crate) struct SessionKeys {
    pub(crate) send: Zeroizing<[u8; 32]>,
    pub(crate) recv: Zeroizing<[u8; 32]>,
}

fn le32(b: &[u8]) -> u32 {
    u32::from_le_bytes([b[0], b[1], b[2], b[3]])
}

fn pub_de(b: &[u8]) -> PublicKey {
    let mut k = [0u8; 32];
    k.copy_from_slice(&b[..32]);
    PublicKey::from_bytes(k)
}

/// Preenche o `mac1` e o `mac2` no fim da mensagem. Devolve o `mac1` (que o cookie de volta
/// usa como dado associado).
fn colocar_macs(msg: &mut [u8], mac1_key: &[u8; 32], cookie: Option<&[u8; 16]>) -> [u8; 16] {
    let n = msg.len();
    let mac1 = mac16(mac1_key, &[&msg[..n - 32]]);
    msg[n - 32..n - 16].copy_from_slice(&mac1);
    let mac2 = match cookie {
        Some(c) => mac16(c, &[&msg[..n - 16]]),
        None => [0; 16],
    };
    msg[n - 16..].copy_from_slice(&mac2);
    mac1
}

/// Confere o `mac1` de uma mensagem que chegou para nós (iniciação ou resposta).
pub(crate) fn mac1_confere(id: &Identity, msg: &[u8]) -> bool {
    let n = msg.len();
    if n < 32 {
        return false;
    }
    let esperado = mac16(&id.mac1_key, &[&msg[..n - 32]]);
    esperado.ct_eq(&msg[n - 32..n - 16]).into()
}

/// Confere o `mac2` de uma mensagem contra o cookie que demos a quem a mandou.
pub(crate) fn mac2_confere(msg: &[u8], cookie: &[u8; 16]) -> bool {
    let n = msg.len();
    if n < 32 {
        return false;
    }
    let esperado = mac16(cookie, &[&msg[..n - 16]]);
    esperado.ct_eq(&msg[n - 16..]).into()
}

/// O `mac1` que a mensagem traz.
pub(crate) fn mac1_da_mensagem(msg: &[u8]) -> [u8; 16] {
    let n = msg.len();
    let mut m = [0u8; 16];
    m.copy_from_slice(&msg[n - 32..n - 16]);
    m
}

/// O cookie de quem manda de `origem`: `Mac(R, ip ‖ porta)`.
pub(crate) fn cookie_de(segredo: &[u8; 32], origem: &SocketAddr) -> [u8; 16] {
    let porta = origem.port().to_be_bytes();
    match origem.ip() {
        std::net::IpAddr::V4(ip) => mac16(segredo, &[&ip.octets(), &porta]),
        std::net::IpAddr::V6(ip) => mac16(segredo, &[&ip.octets(), &porta]),
    }
}

/// Monta a resposta de cookie para uma mensagem cujo `mac1` era `mac1` e cujo remetente usou o
/// índice `receptor`.
pub(crate) fn criar_resposta_de_cookie(
    id: &Identity,
    receptor: u32,
    mac1: &[u8; 16],
    cookie: &[u8; 16],
) -> Result<[u8; TAMANHO_COOKIE], KeyError> {
    let nonce = random_bytes::<24>()?;
    let cifrado = xaead_seal(&id.cookie_key, &nonce, cookie, mac1);
    let mut msg = [0u8; TAMANHO_COOKIE];
    msg[0..4].copy_from_slice(&TIPO_COOKIE.to_le_bytes());
    msg[4..8].copy_from_slice(&receptor.to_le_bytes());
    msg[8..32].copy_from_slice(&nonce);
    msg[32..64].copy_from_slice(&cifrado);
    Ok(msg)
}

/// Abre uma resposta de cookie vinda do par. `mac1` é o `mac1` da última mensagem que mandamos
/// a ele. Devolve `(índice a que a resposta se refere, cookie)`.
pub(crate) fn abrir_resposta_de_cookie(
    par: &PeerKeys,
    msg: &[u8],
    mac1: &[u8; 16],
) -> Option<(u32, [u8; 16])> {
    if msg.len() != TAMANHO_COOKIE {
        return None;
    }
    let mut nonce = [0u8; 24];
    nonce.copy_from_slice(&msg[8..32]);
    let claro = xaead_open(&par.cookie_key, &nonce, &msg[32..64], mac1)?;
    if claro.len() != 16 {
        return None;
    }
    let mut cookie = [0u8; 16];
    cookie.copy_from_slice(&claro);
    Some((le32(&msg[4..8]), cookie))
}

/// O que o iniciador guarda entre mandar a iniciação e receber a resposta.
pub(crate) struct InitiatorState {
    pub(crate) index: u32,
    ephemeral: PrivateKey,
    chaining: Zeroizing<[u8; 32]>,
    hash: [u8; 32],
    /// O `mac1` da iniciação mandada.
    pub(crate) mac1: [u8; 16],
}

/// Cria a mensagem de iniciação (tipo 1).
pub(crate) fn criar_iniciacao(
    id: &Identity,
    par: &PeerKeys,
    index: u32,
    carimbo: &[u8; 12],
    cookie: Option<&[u8; 16]>,
) -> Result<([u8; TAMANHO_INICIACAO], InitiatorState), KeyError> {
    let efemera = PrivateKey::generate()?;
    let e_pub = efemera.public_key();
    let (ci, hi) = estado_inicial();
    let mut h = hash(&[&hi, par.public.as_bytes()]);
    let mut c = kdf1(&ci, e_pub.as_bytes());
    h = hash(&[&h, e_pub.as_bytes()]);
    let dh = efemera.diffie_hellman(&par.public)?;
    let (c2, k) = kdf2(&c, &*dh);
    c = c2;
    let estatica = aead_seal(&k, 0, id.public.as_bytes(), &h);
    h = hash(&[&h, &estatica]);
    let (c2, k) = kdf2(&c, &*par.static_static);
    c = c2;
    let tempo = aead_seal(&k, 0, carimbo, &h);
    h = hash(&[&h, &tempo]);

    let mut msg = [0u8; TAMANHO_INICIACAO];
    msg[0..4].copy_from_slice(&TIPO_INICIACAO.to_le_bytes());
    msg[4..8].copy_from_slice(&index.to_le_bytes());
    msg[8..40].copy_from_slice(e_pub.as_bytes());
    msg[40..88].copy_from_slice(&estatica);
    msg[88..116].copy_from_slice(&tempo);
    let mac1 = colocar_macs(&mut msg, &par.mac1_key, cookie);
    Ok((
        msg,
        InitiatorState {
            index,
            ephemeral: efemera,
            chaining: c,
            hash: h,
            mac1,
        },
    ))
}

/// Consome a resposta (tipo 2) a uma iniciação nossa. O `mac1` já foi conferido por quem chama.
pub(crate) fn consumir_resposta(
    id: &Identity,
    par: &PeerKeys,
    estado: &InitiatorState,
    msg: &[u8; TAMANHO_RESPOSTA],
) -> Option<(SessionKeys, u32)> {
    if le32(&msg[8..12]) != estado.index {
        return None;
    }
    let indice_remoto = le32(&msg[4..8]);
    let e_r = pub_de(&msg[12..44]);
    let mut c = kdf1(&estado.chaining, e_r.as_bytes());
    let mut h = hash(&[&estado.hash, e_r.as_bytes()]);
    let dh = estado.ephemeral.diffie_hellman(&e_r).ok()?;
    c = kdf1(&c, &*dh);
    let dh = id.private.diffie_hellman(&e_r).ok()?;
    c = kdf1(&c, &*dh);
    let (c2, tau, k) = kdf3(&c, &*par.psk);
    c = c2;
    h = hash(&[&h, &*tau]);
    let vazio = aead_open(&k, 0, &msg[44..60], &h)?;
    if !vazio.is_empty() {
        return None;
    }
    let (enviar, receber) = kdf2(&c, &[]);
    Some((
        SessionKeys {
            send: enviar,
            recv: receber,
        },
        indice_remoto,
    ))
}

/// A iniciação aberta até a identificação do par (falta ainda o carimbo de tempo).
pub(crate) struct PartialInitiation {
    pub(crate) peer: PublicKey,
    pub(crate) remote_index: u32,
    ephemeral: PublicKey,
    chaining: Zeroizing<[u8; 32]>,
    hash: [u8; 32],
    enc_timestamp: [u8; 28],
}

/// Abre a parte estática de uma iniciação (tipo 1). Os `mac`s já foram conferidos por quem chama.
pub(crate) fn abrir_iniciacao(
    id: &Identity,
    msg: &[u8; TAMANHO_INICIACAO],
) -> Option<PartialInitiation> {
    let (ci, hi) = estado_inicial();
    let mut h = hash(&[&hi, id.public.as_bytes()]);
    let e_i = pub_de(&msg[8..40]);
    let mut c = kdf1(&ci, e_i.as_bytes());
    h = hash(&[&h, e_i.as_bytes()]);
    let dh = id.private.diffie_hellman(&e_i).ok()?;
    let (c2, k) = kdf2(&c, &*dh);
    c = c2;
    let estatica = aead_open(&k, 0, &msg[40..88], &h)?;
    if estatica.len() != 32 {
        return None;
    }
    let par = pub_de(&estatica);
    h = hash(&[&h, &msg[40..88]]);
    let mut enc_timestamp = [0u8; 28];
    enc_timestamp.copy_from_slice(&msg[88..116]);
    Some(PartialInitiation {
        peer: par,
        remote_index: le32(&msg[4..8]),
        ephemeral: e_i,
        chaining: c,
        hash: h,
        enc_timestamp,
    })
}

/// O que o respondedor guarda entre abrir a iniciação e montar a resposta.
pub(crate) struct ResponderState {
    pub(crate) remote_index: u32,
    remote_ephemeral: PublicKey,
    chaining: Zeroizing<[u8; 32]>,
    hash: [u8; 32],
}

/// Termina de abrir a iniciação com as chaves do par: devolve o carimbo de tempo.
pub(crate) fn terminar_iniciacao(
    par: &PeerKeys,
    parcial: PartialInitiation,
) -> Option<([u8; 12], ResponderState)> {
    let (c, k) = kdf2(&parcial.chaining, &*par.static_static);
    let tempo = aead_open(&k, 0, &parcial.enc_timestamp, &parcial.hash)?;
    if tempo.len() != 12 {
        return None;
    }
    let mut carimbo = [0u8; 12];
    carimbo.copy_from_slice(&tempo);
    let h = hash(&[&parcial.hash, &parcial.enc_timestamp]);
    Some((
        carimbo,
        ResponderState {
            remote_index: parcial.remote_index,
            remote_ephemeral: parcial.ephemeral,
            chaining: c,
            hash: h,
        },
    ))
}

/// Cria a resposta (tipo 2) e as chaves de transporte do respondedor. Devolve também o `mac1`.
pub(crate) fn criar_resposta(
    par: &PeerKeys,
    estado: &ResponderState,
    index: u32,
    cookie: Option<&[u8; 16]>,
) -> Result<([u8; TAMANHO_RESPOSTA], SessionKeys, [u8; 16]), KeyError> {
    let efemera = PrivateKey::generate()?;
    let e_pub = efemera.public_key();
    let mut c = kdf1(&estado.chaining, e_pub.as_bytes());
    let mut h = hash(&[&estado.hash, e_pub.as_bytes()]);
    let dh = efemera.diffie_hellman(&estado.remote_ephemeral)?;
    c = kdf1(&c, &*dh);
    let dh = efemera.diffie_hellman(&par.public)?;
    c = kdf1(&c, &*dh);
    let (c2, tau, k) = kdf3(&c, &*par.psk);
    c = c2;
    h = hash(&[&h, &*tau]);
    let vazio = aead_seal(&k, 0, &[], &h);

    let mut msg = [0u8; TAMANHO_RESPOSTA];
    msg[0..4].copy_from_slice(&TIPO_RESPOSTA.to_le_bytes());
    msg[4..8].copy_from_slice(&index.to_le_bytes());
    msg[8..12].copy_from_slice(&estado.remote_index.to_le_bytes());
    msg[12..44].copy_from_slice(e_pub.as_bytes());
    msg[44..60].copy_from_slice(&vazio);
    let mac1 = colocar_macs(&mut msg, &par.mac1_key, cookie);
    let (receber, enviar) = kdf2(&c, &[]);
    Ok((
        msg,
        SessionKeys {
            send: enviar,
            recv: receber,
        },
        mac1,
    ))
}

/// O carimbo TAI64N de `unix_segundos` e `nanos`, com os 24 bits baixos dos nanossegundos
/// zerados (como o WireGuard do Linux faz, para não vazar a precisão do relógio).
pub(crate) fn carimbo_tai64n(unix_segundos: u64, nanos: u32) -> [u8; 12] {
    let mut c = [0u8; 12];
    c[0..8].copy_from_slice(&(unix_segundos + 0x4000_0000_0000_000a).to_be_bytes());
    c[8..12].copy_from_slice(&(nanos & !0x00ff_ffff).to_be_bytes());
    c
}

#[cfg(test)]
mod testes {
    use super::*;

    struct Par {
        a: Identity,
        b: Identity,
        a_vendo_b: PeerKeys,
        b_vendo_a: PeerKeys,
    }

    fn par(psk_a: Option<&PresharedKey>, psk_b: Option<&PresharedKey>) -> Par {
        let a = Identity::new(PrivateKey::generate().unwrap());
        let b = Identity::new(PrivateKey::generate().unwrap());
        let a_vendo_b = PeerKeys::new(&a, b.public(), psk_a).unwrap();
        let b_vendo_a = PeerKeys::new(&b, a.public(), psk_b).unwrap();
        Par {
            a,
            b,
            a_vendo_b,
            b_vendo_a,
        }
    }

    fn handshake(p: &Par) -> Option<(SessionKeys, SessionKeys)> {
        let carimbo = carimbo_tai64n(1_700_000_000, 0);
        let (ini, est_a) = criar_iniciacao(&p.a, &p.a_vendo_b, 11, &carimbo, None).unwrap();
        assert!(mac1_confere(&p.b, &ini));
        let parcial = abrir_iniciacao(&p.b, &ini)?;
        assert_eq!(parcial.peer, p.a.public());
        assert_eq!(parcial.remote_index, 11);
        let (carimbo_lido, est_b) = terminar_iniciacao(&p.b_vendo_a, parcial)?;
        assert_eq!(carimbo_lido, carimbo);
        let (resp, chaves_b, _) = criar_resposta(&p.b_vendo_a, &est_b, 22, None).unwrap();
        assert!(mac1_confere(&p.a, &resp));
        let (chaves_a, indice_de_b) = consumir_resposta(&p.a, &p.a_vendo_b, &est_a, &resp)?;
        assert_eq!(indice_de_b, 22);
        Some((chaves_a, chaves_b))
    }

    #[test]
    fn o_handshake_sem_psk_fecha_com_chaves_espelhadas() {
        let p = par(None, None);
        let (a, b) = handshake(&p).expect("handshake");
        assert_eq!(*a.send, *b.recv);
        assert_eq!(*a.recv, *b.send);
        assert_ne!(*a.send, *a.recv);
    }

    #[test]
    fn o_handshake_com_psk_fecha_e_psk_diferente_falha() {
        let psk = PresharedKey::generate().unwrap();
        let p = par(Some(&psk), Some(&psk));
        let (a, b) = handshake(&p).expect("handshake com PSK");
        assert_eq!(*a.send, *b.recv);

        let outra = PresharedKey::generate().unwrap();
        let ruim = par(Some(&psk), Some(&outra));
        assert!(handshake(&ruim).is_none());
        // E sem PSK de um lado só também não fecha.
        let ruim = par(Some(&psk), None);
        assert!(handshake(&ruim).is_none());
    }

    #[test]
    fn os_tamanhos_das_mensagens_sao_os_do_artigo() {
        let p = par(None, None);
        let carimbo = carimbo_tai64n(1, 0);
        let (ini, est_a) = criar_iniciacao(&p.a, &p.a_vendo_b, 1, &carimbo, None).unwrap();
        assert_eq!(ini.len(), 148);
        assert_eq!(le32(&ini[0..4]), 1);
        let parcial = abrir_iniciacao(&p.b, &ini).unwrap();
        let (_, est_b) = terminar_iniciacao(&p.b_vendo_a, parcial).unwrap();
        let (resp, _, _) = criar_resposta(&p.b_vendo_a, &est_b, 2, None).unwrap();
        assert_eq!(resp.len(), 92);
        assert_eq!(le32(&resp[0..4]), 2);
        assert_eq!(est_a.index, 1);
    }

    #[test]
    fn mac1_errado_ou_corpo_adulterado_e_recusado() {
        let p = par(None, None);
        let carimbo = carimbo_tai64n(1, 0);
        let (mut ini, _) = criar_iniciacao(&p.a, &p.a_vendo_b, 1, &carimbo, None).unwrap();
        assert!(mac1_confere(&p.b, &ini));
        // O mac1 é de B; A não o aceita.
        assert!(!mac1_confere(&p.a, &ini));
        ini[20] ^= 1;
        assert!(!mac1_confere(&p.b, &ini));
    }

    #[test]
    fn iniciacao_de_estranho_nao_abre() {
        let p = par(None, None);
        let estranho = Identity::new(PrivateKey::generate().unwrap());
        let estranho_vendo_b = PeerKeys::new(&estranho, p.b.public(), None).unwrap();
        let carimbo = carimbo_tai64n(1, 0);
        let (ini, _) = criar_iniciacao(&estranho, &estranho_vendo_b, 1, &carimbo, None).unwrap();
        // Abre a parte estática (B não sabe quem é), mas o carimbo exige a chave estática do
        // estranho, que B calcularia com a chave pública dele: o par que B conhece é A.
        let parcial = abrir_iniciacao(&p.b, &ini).unwrap();
        assert_eq!(parcial.peer, estranho.public());
        assert!(terminar_iniciacao(&p.b_vendo_a, parcial).is_none());
    }

    #[test]
    fn o_carimbo_tai64n_segue_o_formato() {
        let c = carimbo_tai64n(0, 0);
        assert_eq!(&c[0..8], &0x4000_0000_0000_000au64.to_be_bytes());
        let c = carimbo_tai64n(10, 0x0123_4567);
        assert_eq!(&c[0..8], &0x4000_0000_0000_0014u64.to_be_bytes());
        // Os 24 bits baixos dos nanossegundos são zerados.
        assert_eq!(&c[8..12], &0x0100_0000u32.to_be_bytes());
    }

    #[test]
    fn o_cookie_vai_e_volta_e_o_mac2_confere() {
        let p = par(None, None);
        let segredo = [7u8; 32];
        let origem: SocketAddr = "192.0.2.10:51820".parse().unwrap();
        let cookie = cookie_de(&segredo, &origem);
        // Origem diferente, cookie diferente.
        let outra: SocketAddr = "192.0.2.10:51821".parse().unwrap();
        assert_ne!(cookie, cookie_de(&segredo, &outra));

        let carimbo = carimbo_tai64n(1, 0);
        let (sem, est) = criar_iniciacao(&p.a, &p.a_vendo_b, 5, &carimbo, None).unwrap();
        assert!(!mac2_confere(&sem, &cookie));
        let resposta = criar_resposta_de_cookie(&p.b, 5, &est.mac1, &cookie).unwrap();
        assert_eq!(resposta.len(), 64);
        let (para, lido) = abrir_resposta_de_cookie(&p.a_vendo_b, &resposta, &est.mac1).unwrap();
        assert_eq!(para, 5);
        assert_eq!(lido, cookie);
        // Com outro mac1 como dado associado, não abre.
        assert!(abrir_resposta_de_cookie(&p.a_vendo_b, &resposta, &[0; 16]).is_none());

        let (com, _) = criar_iniciacao(&p.a, &p.a_vendo_b, 6, &carimbo, Some(&lido)).unwrap();
        assert!(mac1_confere(&p.b, &com));
        assert!(mac2_confere(&com, &cookie));
        assert!(!mac2_confere(&com, &[0; 16]));
    }
}
