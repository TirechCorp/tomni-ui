//! Quem recebe as iniciações: confere os `mac`s, faz o controle de carga com cookies e abre a
//! parte estática da mensagem para descobrir de qual par ela veio.
//!
//! Com o ponto sob carga ([`Responder::set_under_load`]), uma iniciação sem `mac2` válido não
//! gasta Diffie-Hellman: recebe uma resposta de cookie, e só quem provar que recebe pacotes no
//! endereço de origem (devolvendo o cookie no `mac2`) é atendido (artigo, seção 5.3).

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Instant;

use zeroize::Zeroizing;

use crate::handshake::{
    Identity, PartialInitiation, TAMANHO_INICIACAO, TIPO_INICIACAO, abrir_iniciacao, cookie_de,
    criar_resposta_de_cookie, mac1_confere, mac1_da_mensagem, mac2_confere,
};
use crate::key::{KeyError, PublicKey, random_bytes};
use crate::timers::COOKIE_LIFETIME;

/// Uma iniciação cujo `mac1` conferiu e cuja parte estática abriu: sabe-se de qual par é, falta
/// entregá-la ao [`Peer`](crate::Peer) (que confere o carimbo de tempo).
pub struct OpenedInitiation {
    pub(crate) parcial: PartialInitiation,
}

impl OpenedInitiation {
    /// A chave pública de quem iniciou.
    pub fn peer(&self) -> PublicKey {
        self.parcial.peer
    }
}

impl std::fmt::Debug for OpenedInitiation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "OpenedInitiation({})", self.parcial.peer.short())
    }
}

/// O que fazer com um datagrama de iniciação.
#[derive(Debug)]
pub enum Inbound {
    /// Entregar ao par indicado por [`OpenedInitiation::peer`].
    Initiation(OpenedInitiation),
    /// Mandar de volta para a origem esta resposta de cookie (o ponto está sob carga).
    Reply(Vec<u8>),
    /// Descartar em silêncio.
    Drop,
}

/// O recebedor de iniciações de um dispositivo.
pub struct Responder {
    identity: Arc<Identity>,
    sob_carga: bool,
    segredo: Zeroizing<[u8; 32]>,
    segredo_de: Instant,
}

impl Responder {
    /// Cria o recebedor. O segredo dos cookies é sorteado agora e trocado a cada
    /// `COOKIE_LIFETIME` (120 s).
    pub fn new(identity: Arc<Identity>, now: Instant) -> Result<Self, KeyError> {
        Ok(Responder {
            identity,
            sob_carga: false,
            segredo: Zeroizing::new(random_bytes::<32>()?),
            segredo_de: now,
        })
    }

    /// Liga ou desliga o controle de carga (as iniciações passam a exigir o cookie).
    pub fn set_under_load(&mut self, sob_carga: bool) {
        self.sob_carga = sob_carga;
    }

    /// Se o controle de carga está ligado.
    pub fn under_load(&self) -> bool {
        self.sob_carga
    }

    /// Processa um datagrama que chegou de `origem`. Só trata iniciações (tipo 1); qualquer
    /// outra coisa, ou algo malformado, ou com `mac1` errado, é [`Inbound::Drop`].
    pub fn open(&mut self, datagrama: &[u8], origem: SocketAddr, now: Instant) -> Inbound {
        let Ok(msg) = <&[u8; TAMANHO_INICIACAO]>::try_from(datagrama) else {
            return Inbound::Drop;
        };
        if u32::from_le_bytes([msg[0], msg[1], msg[2], msg[3]]) != TIPO_INICIACAO {
            return Inbound::Drop;
        }
        if !mac1_confere(&self.identity, msg) {
            return Inbound::Drop;
        }
        if self.sob_carga {
            if now.saturating_duration_since(self.segredo_de) >= COOKIE_LIFETIME {
                let Ok(novo) = random_bytes::<32>() else {
                    return Inbound::Drop;
                };
                self.segredo = Zeroizing::new(novo);
                self.segredo_de = now;
            }
            let cookie = cookie_de(&self.segredo, &origem);
            if !mac2_confere(msg, &cookie) {
                let receptor = u32::from_le_bytes([msg[4], msg[5], msg[6], msg[7]]);
                return match criar_resposta_de_cookie(
                    &self.identity,
                    receptor,
                    &mac1_da_mensagem(msg),
                    &cookie,
                ) {
                    Ok(r) => Inbound::Reply(r.to_vec()),
                    Err(_) => Inbound::Drop,
                };
            }
        }
        match abrir_iniciacao(&self.identity, msg) {
            Some(parcial) => Inbound::Initiation(OpenedInitiation { parcial }),
            None => Inbound::Drop,
        }
    }
}
