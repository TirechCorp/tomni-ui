//! As conexões com as regiões do DERP: a de casa fica sempre aberta; as outras abrem quando há
//! um par cuja casa é nelas e fecham depois de um tempo sem uso.

use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

use tokio::sync::mpsc;

use super::{Client, DerpTarget, Event};
use crate::key::{NodeKey, NodePublic};
use crate::tailcfg::{DerpMap, DerpRegionId};
use crate::upgrade::Roots;

/// Depois de quanto tempo sem uso uma região que não é a de casa é fechada.
const OCIOSA: Duration = Duration::from_secs(60);

struct Entrada {
    cliente: Client,
    ultimo_uso: Instant,
}

/// As conexões com o DERP de um nó.
pub struct Regions {
    chave: NodeKey,
    raizes: Roots,
    mapa: DerpMap,
    casa: DerpRegionId,
    abertas: HashMap<DerpRegionId, Entrada>,
    prontas: HashSet<DerpRegionId>,
    eventos: mpsc::UnboundedSender<(DerpRegionId, Event)>,
}

impl Regions {
    /// Sem nenhuma conexão ainda.
    pub fn new(
        chave: NodeKey,
        raizes: Roots,
        eventos: mpsc::UnboundedSender<(DerpRegionId, Event)>,
    ) -> Self {
        Regions {
            chave,
            raizes,
            mapa: DerpMap::default(),
            casa: 0,
            abertas: HashMap::new(),
            prontas: HashSet::new(),
            eventos,
        }
    }

    /// O alvo de uma região: o primeiro servidor que atende DERP.
    pub fn target(mapa: &DerpMap, regiao: DerpRegionId) -> Option<DerpTarget> {
        let r = mapa.regions.get(&regiao)?.as_ref()?;
        r.nodes.iter().find_map(DerpTarget::from_node)
    }

    /// Troca o mapa: fecha as regiões que sumiram ou cujo servidor mudou.
    pub fn set_map(&mut self, mapa: DerpMap) {
        self.mapa = mapa;
        let caidas: Vec<DerpRegionId> = self
            .abertas
            .iter()
            .filter(|(id, e)| Self::target(&self.mapa, **id).as_ref() != Some(e.cliente.target()))
            .map(|(id, _)| *id)
            .collect();
        for id in caidas {
            self.abertas.remove(&id);
            self.prontas.remove(&id);
        }
        // A de casa reabre de imediato se ainda existe.
        if self.casa != 0 {
            let c = self.casa;
            self.garantir(c, Instant::now());
        }
    }

    /// O mapa que está valendo.
    pub fn map(&self) -> &DerpMap {
        &self.mapa
    }

    /// Define a região de casa e abre a conexão.
    pub fn set_home(&mut self, regiao: DerpRegionId) {
        if regiao == self.casa {
            return;
        }
        if let Some(e) = self.abertas.get(&self.casa) {
            e.cliente.note_preferred(false);
        }
        self.casa = regiao;
        if regiao != 0 {
            self.garantir(regiao, Instant::now());
            if let Some(e) = self.abertas.get(&regiao) {
                e.cliente.note_preferred(true);
            }
        }
    }

    /// A região de casa (0 = nenhuma).
    pub fn home(&self) -> DerpRegionId {
        self.casa
    }

    fn garantir(&mut self, regiao: DerpRegionId, agora: Instant) -> bool {
        if let Some(e) = self.abertas.get_mut(&regiao) {
            e.ultimo_uso = agora;
            return true;
        }
        let Some(alvo) = Self::target(&self.mapa, regiao) else {
            return false;
        };
        let cliente = Client::spawn(
            alvo,
            self.chave.clone(),
            self.raizes.clone(),
            regiao == self.casa,
            self.eventos.clone(),
        );
        self.abertas.insert(
            regiao,
            Entrada {
                cliente,
                ultimo_uso: agora,
            },
        );
        true
    }

    /// Manda um pacote ao nó `para` pelo DERP da região `regiao`, abrindo a conexão se preciso.
    /// `false` se a região não existe ou a fila está cheia.
    pub fn send(
        &mut self,
        regiao: DerpRegionId,
        para: NodePublic,
        pacote: Vec<u8>,
        agora: Instant,
    ) -> bool {
        if !self.garantir(regiao, agora) {
            return false;
        }
        self.abertas
            .get(&regiao)
            .is_some_and(|e| e.cliente.send(para, pacote))
    }

    /// Anota um evento: guarda quais regiões estão prontas.
    pub fn note_event(&mut self, regiao: DerpRegionId, evento: &Event) {
        match evento {
            Event::Connected => {
                self.prontas.insert(regiao);
            }
            Event::Disconnected(_) => {
                self.prontas.remove(&regiao);
            }
            _ => {}
        }
    }

    /// Se a conexão com a região está pronta.
    pub fn is_ready(&self, regiao: DerpRegionId) -> bool {
        self.prontas.contains(&regiao)
    }

    /// Se a região de casa está pronta.
    pub fn home_ready(&self) -> bool {
        self.casa != 0 && self.is_ready(self.casa)
    }

    /// Fecha as regiões (que não são a de casa) sem uso há um minuto.
    pub fn tick(&mut self, agora: Instant) {
        let casa = self.casa;
        let velhas: Vec<DerpRegionId> = self
            .abertas
            .iter()
            .filter(|(id, e)| **id != casa && agora.duration_since(e.ultimo_uso) > OCIOSA)
            .map(|(id, _)| *id)
            .collect();
        for id in velhas {
            self.abertas.remove(&id);
            self.prontas.remove(&id);
        }
    }

    /// Quantas regiões estão abertas (conectando ou prontas).
    pub fn open_count(&self) -> usize {
        self.abertas.len()
    }

    /// Fecha tudo.
    pub fn close(&mut self) {
        self.abertas.clear();
        self.prontas.clear();
    }
}

/// Escolhe a região de casa: a de menor tempo de conexão TCP entre as que não pedem para ser
/// evitadas. `None` se nenhuma responde. (O Tailscale mede com STUN; o TCP serve ao mesmo
/// fim e não precisa de UDP, que pode estar bloqueado.)
pub async fn pick_home(mapa: &DerpMap) -> Option<DerpRegionId> {
    let mut sondas = tokio::task::JoinSet::new();
    for (id, r) in &mapa.regions {
        let Some(r) = r else { continue };
        if r.avoid || r.no_measure_no_home {
            continue;
        }
        let Some(alvo) = Regions::target(mapa, *id) else {
            continue;
        };
        let id = *id;
        sondas.spawn(async move {
            let inicio = Instant::now();
            let destino = match alvo.ip {
                Some(ip) => tokio::net::TcpStream::connect((ip, alvo.port)).await,
                None => tokio::net::TcpStream::connect((alvo.host.as_str(), alvo.port)).await,
            };
            destino.ok().map(|_| (inicio.elapsed(), id))
        });
    }
    let mut melhor: Option<(Duration, DerpRegionId)> = None;
    let limite = tokio::time::sleep(Duration::from_secs(4));
    tokio::pin!(limite);
    loop {
        tokio::select! {
            r = sondas.join_next() => {
                match r {
                    None => break,
                    Some(Ok(Some(c))) => {
                        if melhor.is_none_or(|m| c < m) {
                            melhor = Some(c);
                        }
                    }
                    Some(_) => {}
                }
            }
            _ = &mut limite => break,
        }
    }
    melhor.map(|(_, id)| id)
}
