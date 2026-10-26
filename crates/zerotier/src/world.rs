//! O mundo: a lista de raízes que o nó usa para se achar na rede. O **planet** é o mundo
//! oficial, assinado por quem mantém a rede; uma **moon** é um mundo extra que um nó "orbita"
//! para ganhar raízes próprias (`World.hpp`).
//!
//! A forma binária é a do arquivo `planet` e das moons (`000000<id>.moon`): tipo (1 byte,
//! 1 = planet, 127 = moon), id e data (8 bytes cada, big-endian), a chave pública que precisa
//! assinar a próxima atualização (64 bytes), a assinatura (96), a contagem de raízes e, para cada
//! uma, a identidade, a contagem de endereços estáveis e os endereços. A moon termina com um
//! campo de 16 bits para extensões (zero).

use std::net::SocketAddr;

use crate::identity::{PUBLIC_LEN, SIGNATURE_LEN, verify_with_public};
use crate::{Error, Identity, Result, inet};

/// O tipo do planet na forma binária (`World::Type::TYPE_PLANET`).
const TIPO_PLANET: u8 = 1;
/// O tipo da moon (`World::Type::TYPE_MOON`).
const TIPO_MOON: u8 = 127;

/// O id do planet oficial, "Terra" (`ZT_WORLD_ID_EARTH`: a distância da Terra ao Sol em
/// quilômetros).
pub const ID_TERRA: u64 = 149_604_618;

/// Máximo de raízes num mundo (`ZT_WORLD_MAX_ROOTS`).
const MAX_RAIZES: usize = 4;
/// Máximo de endereços estáveis por raiz (`ZT_WORLD_MAX_STABLE_ENDPOINTS_PER_ROOT`).
const MAX_ENDERECOS: usize = 32;

/// Marcas que cercam os dados que se assinam (`World::serialize(forSign)`).
const MARCA_INICIO: u64 = 0x7f7f_7f7f_7f7f_7f7f;
const MARCA_FIM: u64 = 0xf7f7_f7f7_f7f7_f7f7;

/// Lê o mundo aos pedaços, sem passar do fim.
struct Leitor<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> Leitor<'a> {
    fn pegar(&mut self, n: usize) -> Result<&'a [u8]> {
        let trecho = self
            .bytes
            .get(self.pos..self.pos + n)
            .ok_or_else(|| Error::Packet("mundo curto demais".into()))?;
        self.pos += n;
        Ok(trecho)
    }

    fn resto(&self) -> &'a [u8] {
        &self.bytes[self.pos..]
    }
}

/// O tipo de um mundo.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WorldKind {
    /// O mundo oficial.
    Planet,
    /// Um mundo extra, orbitado por escolha de quem usa.
    Moon,
}

/// Uma raiz: a identidade e onde achá-la.
#[derive(Clone, Debug)]
pub struct Root {
    /// A identidade (só a parte pública).
    pub identity: Identity,
    /// Os endereços fixos da raiz, na ordem do mundo.
    pub endpoints: Vec<SocketAddr>,
}

/// Um planet ou uma moon.
#[derive(Clone, Debug)]
pub struct World {
    kind: WorldKind,
    id: u64,
    timestamp: u64,
    updates_signed_by: [u8; PUBLIC_LEN],
    signature: [u8; SIGNATURE_LEN],
    roots: Vec<Root>,
}

impl World {
    /// O planet oficial do ZeroTier ("Terra"), embutido no crate.
    ///
    /// # Panics
    ///
    /// Nunca, a não ser que o arquivo embutido esteja corrompido (um teste confere).
    pub fn terra() -> Self {
        Self::parse(include_bytes!("../planet/terra.planet")).expect("o planet embutido é válido")
    }

    /// Lê a forma binária de um arquivo `planet` ou `.moon`. Não confere a assinatura: use
    /// [`World::verify`] ou [`World::should_be_replaced_by`].
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        let (mundo, _) = Self::parse_prefix(bytes)?;
        Ok(mundo)
    }

    /// Como [`World::parse`], devolvendo também quantos bytes usou (as moons chegam em sequência
    /// na resposta a um `WHOIS` de moon).
    pub fn parse_prefix(bytes: &[u8]) -> Result<(Self, usize)> {
        let mut leitor = Leitor { bytes, pos: 0 };
        let kind = match leitor.pegar(1)?[0] {
            TIPO_PLANET => WorldKind::Planet,
            TIPO_MOON => WorldKind::Moon,
            outro => return Err(Error::Packet(format!("tipo de mundo {outro} desconhecido"))),
        };
        let id = u64::from_be_bytes(leitor.pegar(8)?.try_into().expect("8 bytes"));
        let timestamp = u64::from_be_bytes(leitor.pegar(8)?.try_into().expect("8 bytes"));
        let updates_signed_by: [u8; PUBLIC_LEN] =
            leitor.pegar(PUBLIC_LEN)?.try_into().expect("64 bytes");
        let signature: [u8; SIGNATURE_LEN] =
            leitor.pegar(SIGNATURE_LEN)?.try_into().expect("96 bytes");
        let raizes = usize::from(leitor.pegar(1)?[0]);
        if raizes > MAX_RAIZES {
            return Err(Error::Packet("raízes demais no mundo".into()));
        }
        let mut roots = Vec::with_capacity(raizes);
        for _ in 0..raizes {
            let (identity, usado) = Identity::from_wire(leitor.resto())?;
            leitor.pos += usado;
            let quantos = usize::from(leitor.pegar(1)?[0]);
            if quantos > MAX_ENDERECOS {
                return Err(Error::Packet("endereços demais numa raiz".into()));
            }
            let mut endpoints = Vec::new();
            for _ in 0..quantos {
                let (endereco, usado) = inet::read(leitor.resto())?;
                leitor.pos += usado;
                endpoints.extend(endereco);
            }
            roots.push(Root {
                identity,
                endpoints,
            });
        }
        if kind == WorldKind::Moon {
            let extensao = usize::from(u16::from_be_bytes(
                leitor.pegar(2)?.try_into().expect("2 bytes"),
            ));
            leitor.pegar(extensao)?;
        }
        let pos = leitor.pos;
        Ok((
            Self {
                kind,
                id,
                timestamp,
                updates_signed_by,
                signature,
                roots,
            },
            pos,
        ))
    }

    /// Cria um mundo e o assina com `assinante` (a chave que assina também as atualizações
    /// seguintes, a menos que `proximo_assinante` diga outra).
    pub fn make(
        kind: WorldKind,
        id: u64,
        timestamp: u64,
        proximo_assinante: &Identity,
        roots: Vec<Root>,
        assinante: &Identity,
    ) -> Result<Self> {
        let mut mundo = Self {
            kind,
            id,
            timestamp,
            updates_signed_by: *proximo_assinante.public_bytes(),
            signature: [0; SIGNATURE_LEN],
            roots,
        };
        mundo.signature = assinante
            .sign(&mundo.signed_bytes())
            .ok_or_else(|| Error::Identity("o assinante precisa da chave secreta".into()))?;
        Ok(mundo)
    }

    fn body(&self, saida: &mut Vec<u8>) {
        saida.push(match self.kind {
            WorldKind::Planet => TIPO_PLANET,
            WorldKind::Moon => TIPO_MOON,
        });
        saida.extend_from_slice(&self.id.to_be_bytes());
        saida.extend_from_slice(&self.timestamp.to_be_bytes());
        saida.extend_from_slice(&self.updates_signed_by);
    }

    fn roots_bytes(&self, saida: &mut Vec<u8>) {
        saida.push(self.roots.len() as u8);
        for raiz in &self.roots {
            saida.extend_from_slice(&raiz.identity.to_wire());
            saida.push(raiz.endpoints.len() as u8);
            for endereco in &raiz.endpoints {
                inet::write(saida, Some(*endereco));
            }
        }
        if self.kind == WorldKind::Moon {
            saida.extend_from_slice(&0u16.to_be_bytes());
        }
    }

    /// O que a assinatura cobre (`World::serialize(forSign = true)`).
    fn signed_bytes(&self) -> Vec<u8> {
        let mut saida = MARCA_INICIO.to_be_bytes().to_vec();
        self.body(&mut saida);
        self.roots_bytes(&mut saida);
        saida.extend_from_slice(&MARCA_FIM.to_be_bytes());
        saida
    }

    /// A forma binária, a que [`World::parse`] lê e o arquivo `planet` guarda.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut saida = Vec::new();
        self.body(&mut saida);
        saida.extend_from_slice(&self.signature);
        self.roots_bytes(&mut saida);
        saida
    }

    /// Planet ou moon.
    pub fn kind(&self) -> WorldKind {
        self.kind
    }

    /// O id do mundo (o do planet oficial é [`ID_TERRA`]; o da moon é o endereço do dono).
    pub fn id(&self) -> u64 {
        self.id
    }

    /// A data de definição, em milissegundos, que ordena as versões do mesmo mundo.
    pub fn timestamp(&self) -> u64 {
        self.timestamp
    }

    /// As raízes, na ordem do mundo.
    pub fn roots(&self) -> &[Root] {
        &self.roots
    }

    /// A chave que precisa assinar a próxima atualização.
    pub fn updates_signed_by(&self) -> &[u8; PUBLIC_LEN] {
        &self.updates_signed_by
    }

    /// A assinatura confere com `chave`?
    pub fn verify(&self, chave: &[u8; PUBLIC_LEN]) -> bool {
        verify_with_public(chave, &self.signed_bytes(), &self.signature)
    }

    /// `atualizacao` deve substituir este mundo? Só se for do mesmo id e tipo, mais nova e
    /// assinada pela chave que este mundo diz (`World::shouldBeReplacedBy`).
    pub fn should_be_replaced_by(&self, atualizacao: &World) -> bool {
        self.id == atualizacao.id
            && self.kind == atualizacao.kind
            && self.timestamp < atualizacao.timestamp
            && atualizacao.verify(&self.updates_signed_by)
    }
}
