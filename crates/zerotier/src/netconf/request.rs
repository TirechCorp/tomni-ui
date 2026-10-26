//! O pedido de configuração ao controlador e a remontagem da resposta, que vem em pedaços
//! assinados (`Network::requestConfiguration` e `Network::handleConfigChunk`).

use crate::dictionary::Dictionary;
use crate::identity::SIGNATURE_LEN;
use crate::peer::RemoteVersion;
use crate::{Address, Identity, NetworkId};

use super::{CONFIG_VERSION, rules};

/// O maior dicionário de configuração que aceitamos montar.
pub const MAX_CONFIG_LEN: usize = 262_144;
/// Quantas atualizações montamos ao mesmo tempo (`ZT_NETWORK_MAX_INCOMING_UPDATES`).
const MAX_INCOMING_UPDATES: usize = 3;
/// Máximo de capabilities, regras de capability e tags que anunciamos ao controlador.
const MAX_CAPABILITIES: u64 = 128;
const MAX_CAPABILITY_RULES: u64 = 64;
const MAX_TAGS: u64 = 128;
/// O fabricante do nó: ZeroTier.
const VENDOR_ZEROTIER: u64 = 1;
/// A revisão do motor de regras que implementamos.
const RULES_ENGINE_REVISION: u64 = 1;

/// O corpo do `NETWORK_CONFIG_REQUEST`: `nwid ‖ tamanho ‖ metadados ‖ revisão ‖ carimbo`. O par
/// `atual` é a revisão e o carimbo da configuração que já temos (zeros se não temos).
pub fn request_payload(
    rede: NetworkId,
    versao: RemoteVersion,
    sistema: &str,
    atual: Option<(u64, u64)>,
) -> Vec<u8> {
    let mut md = Dictionary::new();
    md.add_u64("v", CONFIG_VERSION);
    md.add_u64("vend", VENDOR_ZEROTIER);
    md.add_u64("pv", u64::from(versao.protocol));
    md.add_u64("majv", u64::from(versao.major));
    md.add_u64("minv", u64::from(versao.minor));
    md.add_u64("revv", u64::from(versao.revision));
    md.add_u64("mr", rules::MAX_RULES as u64);
    md.add_u64("mc", MAX_CAPABILITIES);
    md.add_u64("mcr", MAX_CAPABILITY_RULES);
    md.add_u64("mt", MAX_TAGS);
    md.add_u64("f", 0);
    md.add_u64("revr", RULES_ENGINE_REVISION);
    md.add_str("o", sistema);
    let md = md.to_bytes();
    let mut saida = Vec::with_capacity(8 + 2 + md.len() + 16);
    saida.extend_from_slice(&rede.as_u64().to_be_bytes());
    saida.extend_from_slice(&(md.len() as u16).to_be_bytes());
    saida.extend_from_slice(&md);
    let (revisao, carimbo) = atual.unwrap_or((0, 0));
    saida.extend_from_slice(&revisao.to_be_bytes());
    saida.extend_from_slice(&carimbo.to_be_bytes());
    saida
}

/// Lê o pedido do lado do controlador: a rede, os metadados e a revisão e o carimbo que o
/// membro já tem. `None` se está truncado.
pub fn parse_request(payload: &[u8]) -> Option<(NetworkId, Dictionary, (u64, u64))> {
    let rede = NetworkId::new(u64::from_be_bytes(payload.get(..8)?.try_into().ok()?));
    let n = usize::from(u16::from_be_bytes(payload.get(8..10)?.try_into().ok()?));
    let md = Dictionary::parse(payload.get(10..10 + n)?);
    let resto = payload.get(10 + n..).unwrap_or(&[]);
    let atual = if resto.len() >= 16 {
        (
            u64::from_be_bytes(resto[..8].try_into().ok()?),
            u64::from_be_bytes(resto[8..16].try_into().ok()?),
        )
    } else {
        (0, 0)
    };
    Some((rede, md, atual))
}

/// Parte `dicionario` em pedaços assinados pelo `controlador`, prontos para ir na carga do
/// `OK(NETWORK_CONFIG_REQUEST)` ou do `NETWORK_CONFIG`. Cada pedaço leva no máximo `tamanho`
/// bytes do dicionário. `None` se o controlador não tem a parte secreta.
pub fn build_chunks(
    controlador: &Identity,
    rede: NetworkId,
    dicionario: &[u8],
    update_id: u64,
    tamanho: usize,
) -> Option<Vec<Vec<u8>>> {
    let total = dicionario.len();
    let mut pedacos = Vec::new();
    let mut indice = 0;
    while indice < total || (total == 0 && pedacos.is_empty()) {
        let fim = (indice + tamanho).min(total);
        let dados = &dicionario[indice..fim];
        let mut p = Vec::with_capacity(dados.len() + 40 + SIGNATURE_LEN);
        p.extend_from_slice(&rede.as_u64().to_be_bytes());
        p.extend_from_slice(&(dados.len() as u16).to_be_bytes());
        p.extend_from_slice(dados);
        p.push(0);
        p.extend_from_slice(&update_id.to_be_bytes());
        p.extend_from_slice(&(total as u32).to_be_bytes());
        p.extend_from_slice(&(indice as u32).to_be_bytes());
        let assinatura = controlador.sign(&p)?;
        p.push(1);
        p.extend_from_slice(&(SIGNATURE_LEN as u16).to_be_bytes());
        p.extend_from_slice(&assinatura);
        pedacos.push(p);
        indice = fim;
        if total == 0 {
            break;
        }
    }
    Some(pedacos)
}

/// Uma atualização em montagem.
struct Update {
    id: u64,
    /// O contador de "mais novo" (maior = mais recente).
    age: u64,
    ids: Vec<u64>,
    bytes: usize,
    data: Vec<u8>,
    /// Os pedaços assinados como chegaram (`u32` com o tamanho + o pedaço), para guardar.
    raw: Vec<u8>,
    signed: bool,
}

/// O que `ConfigAssembler::add` devolve quando uma atualização fica completa.
#[derive(Debug)]
pub struct Assembled {
    /// O id da atualização (para o `OK` que a confirma).
    pub update_id: u64,
    /// O dicionário montado.
    pub dictionary: Dictionary,
    /// Os pedaços assinados, juntos (`u32` com o tamanho + o pedaço, em sequência), para o app
    /// guardar e conferir de novo com [`split_stored`]. Vazio no formato antigo, sem assinatura.
    pub signed: Vec<u8>,
}

/// Separa o que [`Assembled::signed`] juntou nos pedaços de antes.
pub fn split_stored(guardado: &[u8]) -> Option<Vec<&[u8]>> {
    let mut pedacos = Vec::new();
    let mut p = 0;
    while p < guardado.len() {
        let n = u32::from_be_bytes(guardado.get(p..p + 4)?.try_into().ok()?) as usize;
        pedacos.push(guardado.get(p + 4..p + 4 + n)?);
        p += 4 + n;
    }
    Some(pedacos)
}

/// Remonta a configuração a partir dos pedaços, conferindo a assinatura de cada um.
#[derive(Default)]
pub struct ConfigAssembler {
    updates: Vec<Update>,
    clock: u64,
}

impl ConfigAssembler {
    /// Um montador vazio.
    pub fn new() -> Self {
        Self::default()
    }

    /// Esquece o que está em montagem.
    pub fn clear(&mut self) {
        self.updates.clear();
    }

    /// Acrescenta um pedaço. `payload` começa no id da rede; `origem` é quem mandou; `controlador`
    /// é a identidade do controlador da rede; `packet_id` é o id do pacote (id do pedaço no
    /// formato antigo, sem assinatura, que só vale vindo do próprio controlador).
    pub fn add(
        &mut self,
        payload: &[u8],
        origem: Address,
        controlador: &Identity,
        packet_id: u64,
    ) -> Option<Assembled> {
        let tam_pedaco = usize::from(u16::from_be_bytes(payload.get(8..10)?.try_into().ok()?));
        let dados = payload.get(10..10 + tam_pedaco)?;
        let resto = payload.get(10 + tam_pedaco..).unwrap_or(&[]);

        let (update_id, total, indice, chunk_id) = if resto.is_empty() {
            // Formato antigo, sem assinatura: só do controlador.
            if origem != controlador.address() || tam_pedaco >= MAX_CONFIG_LEN {
                return None;
            }
            (packet_id, tam_pedaco, 0usize, packet_id)
        } else {
            let cab = resto.get(..17)?;
            let update_id = u64::from_be_bytes(cab[1..9].try_into().ok()?);
            let total = u32::from_be_bytes(cab[9..13].try_into().ok()?) as usize;
            let indice = u32::from_be_bytes(cab[13..17].try_into().ok()?) as usize;
            if indice + tam_pedaco > total || total >= MAX_CONFIG_LEN {
                return None;
            }
            let assin = resto.get(17..)?;
            if *assin.first()? != 1
                || u16::from_be_bytes(assin.get(1..3)?.try_into().ok()?) as usize != SIGNATURE_LEN
            {
                return None;
            }
            let assinatura = assin.get(3..3 + SIGNATURE_LEN)?;
            let assinado = &payload[..10 + tam_pedaco + 17];
            if !controlador.verify(assinado, assinatura) {
                return None;
            }
            let mut id = [0u8; 8];
            for (i, b) in assinatura.iter().take(16).enumerate() {
                id[i & 7] ^= b;
            }
            (update_id, total, indice, u64::from_be_bytes(id))
        };

        self.clock += 1;
        let pos = match self.updates.iter().position(|u| u.id == update_id) {
            Some(i) => i,
            None => {
                if self.updates.len() >= MAX_INCOMING_UPDATES {
                    // Troca a mais antiga.
                    let velho = self
                        .updates
                        .iter()
                        .enumerate()
                        .min_by_key(|(_, u)| u.age)
                        .map(|(i, _)| i)?;
                    self.updates.remove(velho);
                }
                self.updates.push(Update {
                    id: update_id,
                    age: 0,
                    ids: vec![],
                    bytes: 0,
                    data: vec![0; total],
                    raw: Vec::new(),
                    signed: !resto.is_empty(),
                });
                self.updates.len() - 1
            }
        };
        let u = &mut self.updates[pos];
        if u.ids.contains(&chunk_id) || u.data.len() != total {
            return None;
        }
        u.age = self.clock;
        u.ids.push(chunk_id);
        u.data[indice..indice + tam_pedaco].copy_from_slice(dados);
        u.bytes += tam_pedaco;
        if u.signed {
            u.raw
                .extend_from_slice(&(payload.len() as u32).to_be_bytes());
            u.raw.extend_from_slice(payload);
        }
        if u.bytes != total {
            return None;
        }
        let pronto = self.updates.remove(pos);
        Some(Assembled {
            update_id,
            dictionary: Dictionary::parse(&pronto.data),
            signed: pronto.raw,
        })
    }
}

#[cfg(test)]
mod testes {
    use super::*;

    fn identidade(n: u8) -> Identity {
        let texto = std::fs::read_to_string(format!(
            "{}/tests/fixtures/identidades/id{n}.secret",
            env!("CARGO_MANIFEST_DIR")
        ))
        .unwrap();
        Identity::from_secret_str(&texto).unwrap()
    }

    fn rede(ctl: &Identity) -> NetworkId {
        NetworkId::new((ctl.address().as_u64() << 24) | 1)
    }

    fn dicionario_grande() -> Vec<u8> {
        let mut d = Dictionary::new();
        d.add("R", &vec![0x41; 3000]);
        d.add_str("n", "grande");
        d.to_bytes()
    }

    #[test]
    fn o_pedido_vai_e_volta() {
        let versao = RemoteVersion {
            protocol: 11,
            major: 1,
            minor: 14,
            revision: 2,
        };
        let r = NetworkId::new(0x1122_3344_5500_0001);
        let p = request_payload(r, versao, "linux-x86_64", Some((7, 99)));
        let (rede, md, atual) = parse_request(&p).unwrap();
        assert_eq!(rede, r);
        assert_eq!(atual, (7, 99));
        assert_eq!(md.get_u64("pv"), Some(11));
        assert_eq!(md.get_u64("v"), Some(CONFIG_VERSION));
        assert_eq!(md.get_str("o"), Some("linux-x86_64"));
        let (_, _, zeros) = parse_request(&request_payload(r, versao, "x", None)).unwrap();
        assert_eq!(zeros, (0, 0));
    }

    #[test]
    fn os_pedacos_se_juntam_fora_de_ordem() {
        let ctl = identidade(1);
        let dicio = dicionario_grande();
        let pedacos = build_chunks(&ctl, rede(&ctl), &dicio, 42, 1000).unwrap();
        assert!(pedacos.len() >= 4);
        let mut m = ConfigAssembler::new();
        let mut pronto = None;
        for p in pedacos.iter().rev() {
            if let Some(a) = m.add(p, ctl.address(), &ctl, 1) {
                pronto = Some(a);
            }
        }
        let a = pronto.expect("montou");
        assert_eq!(a.update_id, 42);
        assert_eq!(a.dictionary.get_str("n"), Some("grande"));
        assert_eq!(a.dictionary.get("R").unwrap().len(), 3000);
    }

    #[test]
    fn pedaco_repetido_ou_adulterado_nao_conta() {
        let ctl = identidade(1);
        let dicio = dicionario_grande();
        let pedacos = build_chunks(&ctl, rede(&ctl), &dicio, 1, 1000).unwrap();
        let mut m = ConfigAssembler::new();
        assert!(m.add(&pedacos[0], ctl.address(), &ctl, 1).is_none());
        assert!(m.add(&pedacos[0], ctl.address(), &ctl, 1).is_none());
        let mut ruim = pedacos[1].clone();
        ruim[12] ^= 1;
        assert!(m.add(&ruim, ctl.address(), &ctl, 1).is_none());
        // Assinado por outra identidade.
        let falso = build_chunks(&identidade(2), rede(&ctl), &dicio, 1, 1000).unwrap();
        assert!(m.add(&falso[1], ctl.address(), &ctl, 1).is_none());
        // Nada disso atrapalhou os pedaços bons.
        let mut feito = None;
        for p in &pedacos[1..] {
            feito = m.add(p, ctl.address(), &ctl, 1);
        }
        assert!(feito.is_some());
    }

    #[test]
    fn o_formato_antigo_so_vale_do_controlador() {
        let ctl = identidade(1);
        let mut dicio = Dictionary::new();
        dicio.add_str("n", "antigo");
        let dados = dicio.to_bytes();
        let mut p = rede(&ctl).as_u64().to_be_bytes().to_vec();
        p.extend_from_slice(&(dados.len() as u16).to_be_bytes());
        p.extend_from_slice(&dados);
        let mut m = ConfigAssembler::new();
        assert!(m.add(&p, identidade(2).address(), &ctl, 9).is_none());
        let a = m.add(&p, ctl.address(), &ctl, 9).unwrap();
        assert_eq!(a.dictionary.get_str("n"), Some("antigo"));
    }

    #[test]
    fn total_grande_demais_e_recusado() {
        let ctl = identidade(1);
        let grande = vec![b'x'; MAX_CONFIG_LEN];
        let pedacos = build_chunks(&ctl, rede(&ctl), &grande, 1, 1000).unwrap();
        let mut m = ConfigAssembler::new();
        assert!(m.add(&pedacos[0], ctl.address(), &ctl, 1).is_none());
        assert!(m.updates.is_empty());
    }
}
