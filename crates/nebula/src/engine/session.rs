//! Os túneis e os apertos de mão: o `hostmap` e o `handshake_manager` do Nebula.

use std::collections::VecDeque;
use std::net::{IpAddr, SocketAddr};

use tokio::time::Instant;

use super::{Cached, Engine, Host, Key, MAX_CACHED_PACKETS, Pending, Via, now_nanos};
use crate::handshake::{Completed, Machine};
use crate::header::{self, Header, MessageType};
use crate::tunnel::Tunnel;
use crate::underlay::Underlay;

/// Por que um túnel novo não entrou.
enum Rejected {
    /// O mesmo primeiro pacote já foi atendido: o par repetiu; a resposta é reenviada.
    AlreadySeen(Vec<u8>),
    /// Já há um túnel mais novo com o par.
    TooOld,
    /// Outro motivo (índice repetido, certificado sem endereço).
    Other,
}

impl<U: Underlay> Engine<U> {
    /// Um índice local aleatório que ninguém usa (nunca zero). Vem do protocolo: o Nebula tenta
    /// 32 vezes antes de desistir.
    fn alloc_index(&self) -> u32 {
        let mut escolhido = 1;
        for _ in 0..32 {
            let mut bytes = [0u8; 4];
            let _ = getrandom::fill(&mut bytes);
            escolhido = u32::from_be_bytes(bytes).max(1);
            if escolhido != 0
                && !self.hosts.contains_key(&escolhido)
                && !self.pending_by_index.contains_key(&escolhido)
            {
                break;
            }
        }
        escolhido
    }

    /// Os endereços UDP para tentar com `vpn`.
    pub(super) fn candidates(&self, vpn: IpAddr) -> Vec<SocketAddr> {
        self.remotes
            .get(&vpn)
            .map(|lista| lista.candidates(&self.my_nets))
            .unwrap_or_default()
    }

    /// Começa o aperto de mão com `vpn` (se já não há um) e pergunta aos lighthouses onde ele
    /// está.
    pub(super) fn start_handshake(&mut self, vpn: IpAddr) {
        if self.pending.contains_key(&vpn) || self.my_addrs.contains(&vpn) {
            return;
        }
        let indice = self.alloc_index();
        let mut maquina = Machine::new(self.creds.clone(), true, indice, None);
        let stage0 = match maquina.initiate(now_nanos()) {
            Ok(pacote) => pacote,
            Err(erro) => {
                self.unreachable(vpn, format!("{vpn} recusou o handshake: {}", erro.reason));
                return;
            }
        };
        // Com endereço estático conhecido a primeira tentativa sai já; senão espera o
        // lighthouse responder (ou o primeiro intervalo).
        let tem_estatico = self
            .remotes
            .get(&vpn)
            .is_some_and(|r| r.resolving || !r.statics.is_empty());
        let espera = if tem_estatico {
            std::time::Duration::ZERO
        } else {
            self.cfg.handshakes.try_interval
        };
        let quando = Instant::now() + espera;
        self.pending.insert(
            vpn,
            Pending {
                machine: maquina,
                local_index: indice,
                stage0,
                counter: 0,
                next_due: quando,
                cache: VecDeque::new(),
                last_remotes: Vec::new(),
                last_error: None,
                sent_any: false,
            },
        );
        self.pending_by_index.insert(indice, vpn);
        self.timers.add(quando, Key::Handshake(vpn));
        self.query_lighthouses(vpn);
    }

    /// O temporizador de uma tentativa venceu. Um disparo velho (que não é o esperado) é
    /// ignorado.
    pub(super) fn handshake_tick(&mut self, vpn: IpAddr, quando: Instant) {
        match self.pending.get(&vpn) {
            Some(p) if p.next_due == quando => self.attempt(vpn),
            _ => {}
        }
    }

    /// Uma tentativa: manda o primeiro pacote para todos os endereços conhecidos.
    fn attempt(&mut self, vpn: IpAddr) {
        let intervalo = self.cfg.handshakes.try_interval;
        let tentativas = self.cfg.handshakes.retries;
        let candidatos = self.candidates(vpn);
        let resolvendo = self.remotes.get(&vpn).is_some_and(|r| r.resolving);
        let Some(p) = self.pending.get_mut(&vpn) else {
            return;
        };
        if resolvendo && candidatos.is_empty() {
            // Falta resolver um nome: esperar não conta como tentativa.
            p.next_due = Instant::now() + intervalo;
            let quando = p.next_due;
            self.timers.add(quando, Key::Handshake(vpn));
            return;
        }
        if p.counter >= tentativas {
            self.handshake_timeout(vpn);
            return;
        }
        p.counter += 1;
        let contador = p.counter;
        // A espera cresce linearmente: 1, 2, 3... vezes o intervalo.
        p.next_due = Instant::now() + intervalo * contador;
        let quando = p.next_due;
        self.timers.add(quando, Key::Handshake(vpn));
        // Com pouca pista, pergunta de novo aos lighthouses na quinta tentativa.
        let perguntar = contador == 5 && candidatos.len() <= 1 && !self.lighthouses.contains(&vpn);
        self.send_stage0(vpn, &candidatos);
        self.start_relays(vpn);
        if perguntar {
            self.query_lighthouses(vpn);
        }
    }

    fn send_stage0(&mut self, vpn: IpAddr, candidatos: &[SocketAddr]) {
        let furar = self.cfg.punchy.punch;
        let Some(p) = self.pending.get_mut(&vpn) else {
            return;
        };
        p.last_remotes = candidatos.to_vec();
        if !candidatos.is_empty() {
            p.sent_any = true;
        }
        let pacote = p.stage0.clone();
        for destino in candidatos {
            if furar {
                self.raw_send(&[1], *destino);
            }
            self.raw_send(&pacote, *destino);
        }
    }

    /// Chegou um endereço novo (de um lighthouse ou do DNS): tenta já, sem mexer no
    /// temporizador, se o conjunto mudou.
    pub(super) fn retrigger(&mut self, vpn: IpAddr) {
        let candidatos = self.candidates(vpn);
        let Some(p) = self.pending.get(&vpn) else {
            return;
        };
        if p.last_remotes == candidatos || candidatos.is_empty() {
            return;
        }
        self.send_stage0(vpn, &candidatos);
    }

    fn handshake_timeout(&mut self, vpn: IpAddr) {
        let Some(p) = self.pending.remove(&vpn) else {
            return;
        };
        self.pending_by_index.remove(&p.local_index);
        if self.primary.contains_key(&vpn) {
            // O par chegou primeiro por outro caminho (ele nos procurou): já há túnel.
            return;
        }
        let motivo = if let Some(erro) = &p.last_error {
            format!("{vpn} não completou o handshake: {erro}")
        } else if p.sent_any {
            format!("{vpn} não respondeu ao handshake")
        } else {
            format!("não foi possível chegar a {vpn}: sem caminho direto e sem relay")
        };
        if self.lighthouses.contains(&vpn) {
            self.lighthouse_failed(vpn, motivo);
        } else if p.cache.iter().any(|c| matches!(c, Cached::Ip(_))) {
            self.unreachable(vpn, motivo);
        }
    }

    /// Recomeça o aperto de mão de `vpn` com outro índice (o do par errado respondeu, ou a
    /// resposta foi recusada). Os pacotes guardados ficam.
    fn restart_pending(&mut self, vpn: IpAddr) {
        let indice = self.alloc_index();
        let mut maquina = Machine::new(self.creds.clone(), true, indice, None);
        let Ok(stage0) = maquina.initiate(now_nanos()) else {
            return;
        };
        let Some(p) = self.pending.get_mut(&vpn) else {
            return;
        };
        self.pending_by_index.remove(&p.local_index);
        self.pending_by_index.insert(indice, vpn);
        p.machine = maquina;
        p.local_index = indice;
        p.stage0 = stage0;
    }

    /// Um pacote do tipo "aperto de mão".
    pub(super) fn on_handshake(&mut self, pacote: &[u8], via: Via, h: Header) {
        if h.subtype != header::HANDSHAKE_IX_PSK0 {
            return;
        }
        if h.counter == 1 {
            self.begin_handshake(pacote, via, h);
        } else {
            self.continue_handshake(pacote, via, h);
        }
    }

    /// Quem responde: chegou o primeiro pacote de um par.
    fn begin_handshake(&mut self, pacote: &[u8], via: Via, h: Header) {
        if h.remote_index != 0 {
            return;
        }
        let indice = self.alloc_index();
        let mut maquina = Machine::new(self.creds.clone(), false, indice, None);
        let Ok(passo) = maquina.process(pacote, now_nanos()) else {
            return;
        };
        let (Some(resposta), Some(pronto)) = (passo.reply, passo.done) else {
            return;
        };
        match self.install(pronto, via, Some(pacote.to_vec()), Some(resposta.clone())) {
            Ok(indice) => {
                self.reply(via, &resposta);
                if let Some(vpn) = self.hosts.get(&indice).map(|h| h.vpn) {
                    self.tunnel_opened(vpn, indice);
                }
            }
            Err(Rejected::AlreadySeen(antiga)) => self.reply(via, &antiga),
            Err(Rejected::TooOld | Rejected::Other) => {}
        }
    }

    /// Quem inicia: chegou a resposta.
    fn continue_handshake(&mut self, pacote: &[u8], via: Via, h: Header) {
        let Some(&vpn) = self.pending_by_index.get(&h.remote_index) else {
            return;
        };
        let agora = now_nanos();
        let Some(p) = self.pending.get_mut(&vpn) else {
            return;
        };
        let passo = match p.machine.process(pacote, agora) {
            Ok(passo) => passo,
            Err(erro) if !erro.fatal => {
                // Quase sempre é a cifra: a mensagem 1 vai em claro, então só ao abrir a resposta
                // se descobre que o outro lado usa outra `cipher`.
                p.last_error = Some(format!(
                    "a resposta não pôde ser aberta ({}); a `cipher` precisa ser a mesma em toda a rede",
                    erro.reason
                ));
                return;
            }
            Err(erro) => {
                p.last_error = Some(erro.reason);
                if let Via::Udp(de) = via {
                    self.remotes.entry(vpn).or_default().block(de);
                }
                self.restart_pending(vpn);
                return;
            }
        };
        let Some(pronto) = passo.done else {
            return;
        };
        // Quem respondeu precisa ser mesmo quem procuramos.
        let tem_endereco = pronto
            .remote_cert
            .networks()
            .iter()
            .any(|rede| rede.addr() == vpn);
        if !tem_endereco {
            if let Some(p) = self.pending.get_mut(&vpn) {
                p.last_error = Some(format!(
                    "quem respondeu em {via} é {}, e não {vpn}",
                    pronto.remote_cert.name()
                ));
            }
            if let Via::Udp(de) = via {
                self.remotes.entry(vpn).or_default().block(de);
            }
            self.restart_pending(vpn);
            return;
        }
        match self.install(pronto, via, None, None) {
            Ok(indice) => self.tunnel_opened(vpn, indice),
            Err(_) => {
                // Já há um túnel melhor: os pacotes guardados vão por ele.
                if let Some(&indice) = self.primary.get(&vpn) {
                    self.flush_cache(vpn, indice);
                }
            }
        }
        if let Some(p) = self.pending.remove(&vpn) {
            self.pending_by_index.remove(&p.local_index);
        }
    }

    /// Põe um túnel que acabou de abrir no mapa dos túneis.
    fn install(
        &mut self,
        pronto: Completed,
        via: Via,
        stage0: Option<Vec<u8>>,
        stage2: Option<Vec<u8>>,
    ) -> Result<u32, Rejected> {
        let enderecos: Vec<IpAddr> = pronto
            .remote_cert
            .networks()
            .iter()
            .map(|rede| rede.addr())
            .collect();
        let Some(&vpn) = enderecos.first() else {
            return Err(Rejected::Other);
        };
        // Só quem responde confere: o primeiro pacote repetido de um túnel que já temos, e o par
        // que insiste com um aperto de mão mais velho. Quem inicia sempre fica com o túnel que
        // acabou de abrir (no Nebula é o `Complete`, que não recusa): o outro lado já o
        // instalou ao responder, e recusar aqui deixaria os dois com túneis principais
        // diferentes, um deles desconhecido do par, quando os dois abrem ao mesmo tempo.
        if !pronto.initiator {
            if stage0.is_some()
                && let Some(antigo) = self
                    .hosts
                    .values()
                    .find(|h| h.vpn == vpn && h.stage0 == stage0)
            {
                return Err(Rejected::AlreadySeen(
                    antigo.stage2.clone().unwrap_or_default(),
                ));
            }
            if let Some(antigo) = self.primary.get(&vpn).and_then(|i| self.hosts.get(i))
                && antigo.time >= pronto.time
                && !antigo.initiator
            {
                return Err(Rejected::TooOld);
            }
        }
        if self.hosts.contains_key(&pronto.local_index) {
            return Err(Rejected::Other);
        }
        let indice = pronto.local_index;
        let agora = Instant::now();
        let host = Host {
            vpn,
            addrs: enderecos.clone(),
            unsafe_nets: pronto.remote_cert.unsafe_networks().to_vec(),
            local_index: indice,
            remote_index: pronto.remote_index,
            tunnel: Tunnel::new(pronto.send, pronto.recv, pronto.message_index),
            remote: None,
            last_roam: None,
            my_cert_version: pronto.my_cert.version(),
            peer_cert: pronto.remote_cert,
            initiator: pronto.initiator,
            time: pronto.time,
            in_traffic: false,
            out_traffic: false,
            pending_deletion: false,
            last_activity: agora,
            stage0,
            stage2,
            rehandshaked: false,
            relay_for: std::collections::HashMap::new(),
            relays: Vec::new(),
        };
        self.hosts.insert(indice, host);
        for addr in enderecos {
            self.primary.insert(addr, indice);
        }
        match via {
            Via::Udp(de) => self.set_remote(indice, de),
            Via::Relay { through, .. } => {
                if let Some(h) = self.hosts.get_mut(&indice) {
                    h.relays.push(through);
                }
            }
        }
        self.timers.add(
            agora + self.cfg.connection_alive_interval,
            Key::Traffic(indice),
        );
        self.remotes.entry(vpn).or_default().unblock_all();
        self.recompute_state();
        Ok(indice)
    }

    /// O que se manda assim que um túnel abre: os pacotes que esperavam (vale para quem
    /// respondeu também: o par pode ter chegado primeiro que a nossa tentativa) e o aviso ao
    /// lighthouse. Fica separado do `install` porque quem responde precisa mandar a resposta do
    /// aperto de mão ANTES de qualquer dado: o par só conhece o índice depois de abrir a resposta,
    /// e um dado que chegasse antes dela viraria "índice desconhecido" (`RecvError`) e derrubaria
    /// o túnel dos dois lados.
    fn tunnel_opened(&mut self, vpn: IpAddr, indice: u32) {
        self.flush_cache(vpn, indice);
        if self.lighthouses.contains(&vpn) {
            self.lighthouse_up(vpn);
        }
    }

    /// Manda o que esperava o túnel de `vpn` abrir.
    fn flush_cache(&mut self, vpn: IpAddr, indice: u32) {
        let guardados = match self.pending.get_mut(&vpn) {
            Some(p) => std::mem::take(&mut p.cache),
            None => return,
        };
        for item in guardados {
            match item {
                Cached::Ip(pacote) => {
                    self.send_ip(indice, &pacote);
                }
                Cached::Message {
                    kind,
                    subtype,
                    payload,
                } => {
                    self.send_to_host(indice, kind, subtype, &payload);
                }
            }
        }
    }

    /// Guarda algo para quando o túnel com `vpn` abrir, e começa o aperto de mão.
    pub(super) fn cache_and_handshake(&mut self, vpn: IpAddr, item: Cached) {
        self.start_handshake(vpn);
        if let Some(p) = self.pending.get_mut(&vpn)
            && p.cache.len() < MAX_CACHED_PACKETS
        {
            p.cache.push_back(item);
        }
    }

    /// Anota de onde os pacotes do par chegam agora.
    pub(super) fn set_remote(&mut self, indice: u32, addr: SocketAddr) {
        let Some(host) = self.hosts.get_mut(&indice) else {
            return;
        };
        host.remote = Some(addr);
        let vpn = host.vpn;
        self.remotes.entry(vpn).or_default().learn(addr);
    }

    /// Tira um túnel do mapa. Se há outro túnel com o mesmo par, ele passa a ser o principal.
    pub(super) fn delete_host(&mut self, indice: u32) {
        let Some(host) = self.hosts.remove(&indice) else {
            return;
        };
        for addr in &host.addrs {
            if self.primary.get(addr) == Some(&indice) {
                self.primary.remove(addr);
            }
        }
        let outro = self
            .hosts
            .values()
            .filter(|o| o.vpn == host.vpn)
            .max_by_key(|o| o.time)
            .map(|o| o.local_index);
        if let Some(outro) = outro {
            for addr in &host.addrs {
                self.primary.entry(*addr).or_insert(outro);
            }
        }
        let relay_for = host.relay_for.into_values().collect();
        self.relays_after_delete(host.vpn, relay_for, &host.relays, outro);
        self.recompute_state();
    }

    /// Avisa o par que o túnel fecha (uma mensagem cifrada vazia), sem esperar resposta.
    pub(super) fn send_close(&mut self, indice: u32) {
        self.send_to_host(indice, MessageType::CloseTunnel, 0, &[]);
    }

    /// Fecha o túnel: avisa o par e esquece.
    pub(super) fn close_tunnel(&mut self, indice: u32) {
        self.send_close(indice);
        self.delete_host(indice);
    }
}
