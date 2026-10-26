//! Os quadros de uma janela: a política para quando a superfície falha, a rede de segurança
//! contra a janela que pinta sem parar ([`Pacer`]) e o rastro dos quadros
//! (`TOMNI_UI_TRACE_FRAMES`).

use std::fmt::Write as _;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

/// Falhas seguidas ao pegar a textura da superfície.
///
/// A superfície desatualizada ou perdida é refeita na hora, com uma nova tentativa. Se falhar
/// de novo, só a primeira falha da sequência pede outro redesenho: uma superfície que não volta
/// (um monitor desligado, um driver que insiste no erro) não vira um laço de quadros. O quadro
/// que não saiu fica pendente ([`SurfaceFailures::stale`]) e sai no próximo evento.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct SurfaceFailures {
    count: u32,
    stale: bool,
}

impl SurfaceFailures {
    /// O quadro não saiu. Retorna se vale pedir outro redesenho agora: só na primeira falha
    /// seguida.
    pub(crate) fn failed(&mut self) -> bool {
        self.stale = true;
        self.count = self.count.saturating_add(1);
        self.count == 1
    }

    /// O quadro ficou para depois sem ser erro (janela coberta ou GPU ocupada): não pede outro
    /// redesenho, só marca a tela como pendente.
    pub(crate) fn skipped(&mut self) {
        self.stale = true;
    }

    /// O quadro saiu: a sequência de falhas acaba.
    pub(crate) fn succeeded(&mut self) {
        self.count = 0;
        self.stale = false;
    }

    /// Se o último quadro pedido não chegou à tela.
    pub(crate) fn stale(&self) -> bool {
        self.stale
    }
}

/// Por que um quadro foi pintado.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Reason {
    /// Um evento (tecla, ponteiro, tamanho, foco) mudou a tela.
    Event,
    /// O prazo do `Handler` venceu: uma animação, a dica, o toque longo.
    Deadline,
    /// A fase do pisca do cursor.
    Blink,
    /// A superfície foi refeita e o quadro, pedido de novo.
    Surface,
    /// O sistema pediu (exposição da janela), sem pedido do runner.
    System,
}

impl Reason {
    const ALL: [Reason; 5] = [
        Reason::Event,
        Reason::Deadline,
        Reason::Blink,
        Reason::Surface,
        Reason::System,
    ];

    fn index(self) -> usize {
        self as usize
    }

    fn label(self) -> &'static str {
        match self {
            Reason::Event => "evento",
            Reason::Deadline => "prazo",
            Reason::Blink => "pisca",
            Reason::Surface => "superfície",
            Reason::System => "sistema",
        }
    }
}

/// Escreve em `line` os motivos com quadro e quantos foram, separados por vírgula
/// (`superfície 60, prazo 2`).
fn write_counts(line: &mut String, counts: &[u32; Reason::ALL.len()]) {
    let mut first = true;
    for (reason, count) in Reason::ALL.iter().zip(counts) {
        if *count > 0 {
            let sep = if first { "" } else { ", " };
            let _ = write!(line, "{sep}{} {count}", reason.label());
            first = false;
        }
    }
}

/// Se `TOMNI_UI_TRACE_FRAMES` está ligada (qualquer valor que não seja vazio nem `0`). Lida uma
/// vez por processo.
pub(crate) fn trace_enabled() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| {
        std::env::var_os("TOMNI_UI_TRACE_FRAMES").is_some_and(|v| !v.is_empty() && v != "0")
    })
}

/// Rastro dos quadros de uma janela: quantos foram pintados e por quê, numa linha por segundo
/// no stderr, só quando houve quadro. Desligado, cada chamada é um teste de `bool`.
#[derive(Debug)]
pub(crate) struct FrameTrace {
    on: bool,
    pending: Option<Reason>,
    counts: [u32; Reason::ALL.len()],
    presented: u32,
    /// O que o prazo do `Handler` esperava da última vez ([`crate::Handler::deadline_reason`]).
    deadline: Option<String>,
    since: Instant,
}

impl FrameTrace {
    /// Ligado conforme `TOMNI_UI_TRACE_FRAMES`.
    pub(crate) fn new(now: Instant) -> Self {
        Self::with(trace_enabled(), now)
    }

    pub(crate) fn with(on: bool, now: Instant) -> Self {
        Self {
            on,
            pending: None,
            counts: [0; Reason::ALL.len()],
            presented: 0,
            deadline: None,
            since: now,
        }
    }

    pub(crate) fn on(&self) -> bool {
        self.on
    }

    /// O runner pediu um redesenho por `reason`. Vale o primeiro pedido antes do quadro. O
    /// motivo é guardado mesmo com o rastro desligado: o [`Pacer`] também conta por motivo.
    pub(crate) fn request(&mut self, reason: Reason) {
        if self.pending.is_none() {
            self.pending = Some(reason);
        }
    }

    /// Tira o motivo do pedido pendente, que o [`Pacer`] segura para depois.
    pub(crate) fn take(&mut self) -> Option<Reason> {
        self.pending.take()
    }

    /// Guarda o que o prazo do `Handler` espera.
    pub(crate) fn note_deadline(&mut self, reason: impl FnOnce() -> Option<String>) {
        if self.on {
            self.deadline = reason();
        }
    }

    /// Um quadro foi pintado; `presented` diz se chegou à tela. Devolve o motivo dele (o do
    /// primeiro pedido, ou [`Reason::System`] sem pedido do runner).
    pub(crate) fn frame(&mut self, presented: bool) -> Reason {
        let reason = self.pending.take().unwrap_or(Reason::System);
        if self.on {
            self.counts[reason.index()] += 1;
            self.presented += u32::from(presented);
        }
        reason
    }

    /// A linha do último segundo (ou mais), se houve quadro, e recomeça a contar. Sem quadro, a
    /// contagem continua: um quadro solto sai com o tempo desde a última linha.
    pub(crate) fn flush(&mut self, now: Instant, window: &str) -> Option<String> {
        let total: u32 = self.counts.iter().sum();
        let elapsed = now.saturating_duration_since(self.since);
        if !self.on || total == 0 || elapsed < Duration::from_secs(1) {
            return None;
        }
        let seconds = format!("{:.1}", elapsed.as_secs_f64()).replace('.', ",");
        let mut line = format!(
            "tomni-ui: quadros de {window}: {total} em {seconds} s, {} na tela (",
            self.presented
        );
        write_counts(&mut line, &self.counts);
        line.push(')');
        if self.counts[Reason::Deadline.index()] > 0
            && let Some(deadline) = &self.deadline
        {
            let _ = write!(line, "; prazo de: {deadline}");
        }
        self.counts = [0; Reason::ALL.len()];
        self.presented = 0;
        self.since = now;
        Some(line)
    }
}

/// O intervalo mínimo entre dois quadros de uma janela limitada pelo [`Pacer`]: 100 ms, ou 10
/// quadros por segundo. Deixa legível a saída rápida de um terminal (`tail -f`, `htop`, uma
/// compilação) assistida sem tocar no mouse, a uma fração do custo de 60 quadros por segundo.
pub(crate) const LIMITED_INTERVAL: Duration = Duration::from_millis(100);

/// Acima de quantos quadros por segundo a janela conta como pintando sem parar.
const BUSY_RATE: u32 = 20;

/// Por quanto tempo a janela pinta sem parar, sem entrada, até o [`Pacer`] limitar.
const BUSY_FOR: Duration = Duration::from_secs(5);

/// A rede de segurança contra a janela que pinta sem parar.
///
/// Parada, uma janela não pinta nada; um laço de quadros sem ninguém mexendo nela é um defeito
/// (do runner, de um widget, do sistema). Quando a janela pinta mais de [`BUSY_RATE`] quadros
/// por segundo, em todos os segundos, por [`BUSY_FOR`] sem nenhuma entrada da pessoa, o
/// `Pacer` limita o ritmo dela a um quadro a cada [`LIMITED_INTERVAL`] e o runner escreve uma
/// linha no stderr com os motivos dos quadros ([`Episode::line`]), uma vez por episódio: o
/// limite esconde o custo, não o defeito.
///
/// A primeira entrada (tecla, ponteiro, roda, toque, IME, arquivos soltos, tamanho, foco)
/// devolve o ritmo normal na hora. A saída de um terminal, o `wake()` de outra thread, os
/// prazos e o pisca não contam como entrada. Limitado, o pedido de quadro que chega cedo
/// demais não se perde: fica retido ([`Pacer::hold`]) e sai no próximo horário
/// ([`Pacer::release`]), pelo `ControlFlow::WaitUntil` do loop, sem dormir a thread.
///
/// A conta é por segundo: cada segundo fechado com mais de [`BUSY_RATE`] quadros por segundo
/// continua a sequência; um abaixo disso, ou uma entrada, a recomeça.
#[derive(Debug)]
pub(crate) struct Pacer {
    /// Início do segundo em contagem.
    since: Instant,
    /// Quadros do segundo em contagem, por motivo.
    counts: [u32; Reason::ALL.len()],
    /// Os quadros do último segundo fechado, por motivo, e quanto ele durou.
    last: ([u32; Reason::ALL.len()], Duration),
    /// Desde quando a janela pinta sem parar, sem entrada.
    busy_since: Option<Instant>,
    /// O último quadro pintado.
    last_frame: Option<Instant>,
    /// O ritmo está limitado.
    limited: bool,
    /// O pedido de quadro retido até o próximo horário, com o seu motivo.
    held: Option<Reason>,
}

/// O começo de um episódio do [`Pacer`]: a janela pintou sem parar e passou a ser limitada.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Episode {
    /// Os quadros do último segundo, por motivo.
    counts: [u32; Reason::ALL.len()],
    /// Quanto durou o último segundo.
    second: Duration,
    /// Há quanto tempo a janela pintava sem parar.
    busy: Duration,
}

impl Episode {
    /// Se algum quadro do último segundo foi de prazo: a linha vai com os widgets que animam.
    pub(crate) fn has_deadline(&self) -> bool {
        self.counts[Reason::Deadline.index()] > 0
    }

    /// A linha do aviso: `tomni-ui: Janela(0) pintou 60 quadros/s por 5 s sem entrada;
    /// limitando a 10/s (motivos: superfície 60)`, com `; prazo de: …` se houver `deadline`.
    pub(crate) fn line(&self, window: &str, deadline: Option<&str>) -> String {
        // O segundo fechado dura um pouco mais que um segundo: as contas vão por segundo.
        let per_second =
            |count: u32| (f64::from(count) / self.second.as_secs_f64().max(0.001)).round();
        let rate = per_second(self.counts.iter().sum());
        let counts = self.counts.map(|count| per_second(count) as u32);
        let limit = Duration::from_secs(1).as_millis() / LIMITED_INTERVAL.as_millis();
        let mut line = format!(
            "tomni-ui: {window} pintou {rate} quadros/s por {} s sem entrada; \
             limitando a {limit}/s (motivos: ",
            self.busy.as_secs()
        );
        write_counts(&mut line, &counts);
        line.push(')');
        if let Some(deadline) = deadline {
            let _ = write!(line, "; prazo de: {deadline}");
        }
        line
    }
}

impl Pacer {
    /// Ritmo normal, começando a contar em `now`.
    pub(crate) fn new(now: Instant) -> Self {
        Self {
            since: now,
            counts: [0; Reason::ALL.len()],
            last: ([0; Reason::ALL.len()], Duration::ZERO),
            busy_since: None,
            last_frame: None,
            limited: false,
            held: None,
        }
    }

    /// Se o ritmo está limitado.
    #[cfg(test)]
    fn limited(&self) -> bool {
        self.limited
    }

    /// O próximo horário de quadro de uma janela limitada.
    fn slot(&self) -> Option<Instant> {
        if !self.limited {
            return None;
        }
        self.last_frame.map(|last| last + LIMITED_INTERVAL)
    }

    /// Se a janela pode pintar em `now`: sempre, fora do limite; limitada, só a partir do
    /// próximo horário.
    pub(crate) fn allow(&self, now: Instant) -> bool {
        self.slot().is_none_or(|slot| now >= slot)
    }

    /// Limitada e antes do horário: até quando a janela espera. O runner não roda o frame dela
    /// nesse meio-tempo, e o tempo que passou chega inteiro ao `Handler` no horário.
    pub(crate) fn wait(&self, now: Instant) -> Option<Instant> {
        self.slot().filter(|slot| now < *slot)
    }

    /// Adia o prazo `at` de uma janela limitada até o próximo horário dela.
    pub(crate) fn clamp(&self, at: Instant) -> Instant {
        self.slot().map_or(at, |slot| at.max(slot))
    }

    /// Segura o pedido de quadro que chegou antes do horário. Vale o motivo do primeiro.
    pub(crate) fn hold(&mut self, reason: Reason) {
        self.held.get_or_insert(reason);
    }

    /// O pedido retido, se já deu o horário dele.
    pub(crate) fn release(&mut self, now: Instant) -> Option<Reason> {
        if self.allow(now) {
            self.held.take()
        } else {
            None
        }
    }

    /// A pessoa mexeu na janela: o ritmo normal volta na hora e a contagem recomeça. Devolve o
    /// pedido retido, que sai agora, e se a janela estava limitada.
    pub(crate) fn input(&mut self, now: Instant) -> (Option<Reason>, bool) {
        let was = self.limited;
        self.since = now;
        self.counts = [0; Reason::ALL.len()];
        self.busy_since = None;
        self.limited = false;
        (self.held.take(), was)
    }

    /// A janela pintou um quadro por `reason` em `now`. Devolve o [`Episode`] quando o limite
    /// acabou de entrar.
    pub(crate) fn frame(&mut self, now: Instant, reason: Reason) -> Option<Episode> {
        self.last_frame = Some(now);
        if self.limited {
            return None;
        }
        self.counts[reason.index()] += 1;
        let second = now.saturating_duration_since(self.since);
        if second < Duration::from_secs(1) {
            return None;
        }
        let total: u32 = self.counts.iter().sum();
        // Mais de `BUSY_RATE` quadros por segundo, em milissegundos para não arredondar.
        let busy = u128::from(total) * 1000 > u128::from(BUSY_RATE) * second.as_millis();
        if busy {
            self.busy_since.get_or_insert(self.since);
            self.last = (self.counts, second);
        } else {
            self.busy_since = None;
        }
        self.since = now;
        self.counts = [0; Reason::ALL.len()];
        let busy = now.saturating_duration_since(self.busy_since?);
        if busy < BUSY_FOR {
            return None;
        }
        self.limited = true;
        self.busy_since = None;
        Some(Episode {
            counts: self.last.0,
            second: self.last.1,
            busy,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn superficie_que_nao_volta_nao_vira_laco_de_quadros() {
        let mut falhas = SurfaceFailures::default();
        let mut pedidos = 0;
        for _ in 0..100 {
            if falhas.failed() {
                pedidos += 1;
            }
        }
        assert_eq!(pedidos, 1, "só a primeira falha seguida pede outro quadro");
        assert!(falhas.stale(), "o quadro que não saiu fica pendente");
        falhas.succeeded();
        assert!(!falhas.stale());
        assert!(
            falhas.failed(),
            "depois de um quadro na tela, a próxima falha tenta de novo"
        );
    }

    #[test]
    fn janela_coberta_fica_pendente_sem_pedir_quadro() {
        let mut falhas = SurfaceFailures::default();
        falhas.skipped();
        assert!(falhas.stale());
        assert!(falhas.failed(), "pular não conta como falha");
    }

    #[test]
    fn rastro_conta_os_motivos_e_sai_uma_vez_por_segundo() {
        let t = Instant::now();
        let mut rastro = FrameTrace::with(true, t);
        rastro.request(Reason::Blink);
        rastro.request(Reason::Event);
        rastro.frame(true);
        rastro.frame(false);
        rastro.note_deadline(|| Some("ProgressRing".into()));
        rastro.request(Reason::Deadline);
        rastro.frame(true);
        assert_eq!(
            rastro.flush(t + Duration::from_millis(500), "Janela(1)"),
            None
        );
        assert_eq!(
            rastro
                .flush(t + Duration::from_secs(2), "Janela(1)")
                .as_deref(),
            Some(
                "tomni-ui: quadros de Janela(1): 3 em 2,0 s, 2 na tela \
                 (prazo 1, pisca 1, sistema 1); prazo de: ProgressRing"
            )
        );
        assert_eq!(
            rastro.flush(t + Duration::from_secs(4), "Janela(1)"),
            None,
            "sem quadro, sem linha"
        );
    }

    #[test]
    fn rastro_desligado_nao_conta() {
        let t = Instant::now();
        let mut rastro = FrameTrace::with(false, t);
        rastro.request(Reason::Event);
        rastro.frame(true);
        assert_eq!(rastro.flush(t + Duration::from_secs(5), "Janela(1)"), None);
    }

    /// Pinta `fps` quadros por segundo por `reason` de `from` até `until` (sem incluir),
    /// devolvendo o último instante e os episódios que começaram.
    fn pintar(
        pacer: &mut Pacer,
        from: Instant,
        until: Instant,
        fps: u32,
        reason: Reason,
    ) -> (Instant, Vec<(Instant, Episode)>) {
        let passo = Duration::from_secs(1) / fps;
        let mut t = from;
        let mut last = from;
        let mut episodios = Vec::new();
        while t < until {
            // Como o runner: o quadro só sai quando o `Pacer` deixa, e leva o retido junto.
            if pacer.allow(t) {
                pacer.release(t);
                if let Some(episodio) = pacer.frame(t, reason) {
                    episodios.push((t, episodio));
                }
                last = t;
            } else {
                pacer.hold(reason);
            }
            t += passo;
        }
        (last, episodios)
    }

    #[test]
    fn janela_que_pinta_sem_parar_e_limitada_depois_de_5_s_sem_entrada() {
        let t = Instant::now();
        let mut pacer = Pacer::new(t);
        let (_, episodios) = pintar(
            &mut pacer,
            t,
            t + Duration::from_millis(4900),
            60,
            Reason::Surface,
        );
        assert!(episodios.is_empty(), "antes dos 5 s o ritmo é o normal");
        assert!(!pacer.limited());
        let (last, episodios) = pintar(
            &mut pacer,
            t + Duration::from_millis(4900),
            t + Duration::from_secs(7),
            60,
            Reason::Surface,
        );
        assert_eq!(episodios.len(), 1, "o limite entra uma vez");
        let (quando, _) = episodios[0];
        assert!(
            quando >= t + BUSY_FOR && quando < t + Duration::from_secs(6),
            "entra aos 5 s: {:?}",
            quando - t
        );
        assert!(pacer.limited());
        assert!(!pacer.allow(last + Duration::from_millis(50)));
        assert!(pacer.allow(last + LIMITED_INTERVAL));
    }

    #[test]
    fn limitada_pinta_no_maximo_10_quadros_por_segundo() {
        let t = Instant::now();
        let mut pacer = Pacer::new(t);
        pintar(
            &mut pacer,
            t,
            t + Duration::from_secs(6),
            60,
            Reason::Deadline,
        );
        assert!(pacer.limited());
        let inicio = t + Duration::from_secs(10);
        let mut quadros = 0;
        let mut agora = inicio;
        // Um prazo a cada 16 ms, e o loop acorda no horário que o `Pacer` dá.
        while agora < inicio + Duration::from_secs(2) {
            if pacer.allow(agora) {
                pacer.frame(agora, Reason::Deadline);
                quadros += 1;
            }
            agora = pacer.clamp(agora + Duration::from_millis(16));
        }
        assert_eq!(quadros, 20, "10 quadros por segundo em 2 s");
    }

    #[test]
    fn a_entrada_devolve_o_ritmo_normal_na_hora() {
        let t = Instant::now();
        let mut pacer = Pacer::new(t);
        let (last, _) = pintar(
            &mut pacer,
            t,
            t + Duration::from_secs(6),
            60,
            Reason::Surface,
        );
        assert!(pacer.limited());
        let agora = last + Duration::from_millis(10);
        assert!(!pacer.allow(agora));
        assert_eq!(
            pacer.input(agora),
            (Some(Reason::Surface), true),
            "o pedido retido sai junto"
        );
        assert!(!pacer.limited());
        assert!(pacer.allow(agora), "o quadro da tecla sai sem esperar");
        assert_eq!(pacer.wait(agora), None);
        // E a contagem recomeça: mais 4 s pintando não limitam.
        let (_, episodios) = pintar(
            &mut pacer,
            agora,
            agora + Duration::from_secs(4),
            60,
            Reason::Surface,
        );
        assert!(episodios.is_empty());
        assert!(!pacer.limited());
    }

    #[test]
    fn o_quadro_retido_nao_se_perde() {
        let t = Instant::now();
        let mut pacer = Pacer::new(t);
        let (last, _) = pintar(&mut pacer, t, t + Duration::from_secs(6), 60, Reason::Event);
        assert!(pacer.limited());
        let cedo = last + Duration::from_millis(30);
        assert!(!pacer.allow(cedo));
        // O laço continua pedindo: o primeiro pedido retido (o `Event` do laço) fica.
        pacer.hold(Reason::Blink);
        assert_eq!(pacer.release(cedo), None, "antes do horário fica retido");
        assert_eq!(
            pacer.wait(cedo),
            Some(last + LIMITED_INTERVAL),
            "o loop acorda no horário"
        );
        assert_eq!(
            pacer.clamp(cedo + Duration::from_millis(16)),
            last + LIMITED_INTERVAL,
            "os prazos da janela esperam o horário"
        );
        assert_eq!(
            pacer.release(last + LIMITED_INTERVAL),
            Some(Reason::Event),
            "no horário sai, com o motivo do primeiro pedido"
        );
        assert_eq!(pacer.release(last + LIMITED_INTERVAL), None);

        // A entrada solta o retido na hora.
        pacer.hold(Reason::Deadline);
        assert_eq!(
            pacer.input(last + Duration::from_millis(20)),
            (Some(Reason::Deadline), true)
        );
    }

    #[test]
    fn o_aviso_sai_uma_vez_por_episodio() {
        let t = Instant::now();
        let mut pacer = Pacer::new(t);
        let (last, episodios) = pintar(
            &mut pacer,
            t,
            t + Duration::from_secs(30),
            60,
            Reason::Surface,
        );
        assert_eq!(episodios.len(), 1, "30 s limitada, um aviso só");
        let (_, episodio) = &episodios[0];
        assert_eq!(
            episodio.line("Janela(0)", None),
            "tomni-ui: Janela(0) pintou 60 quadros/s por 5 s sem entrada; \
             limitando a 10/s (motivos: superfície 60)"
        );
        assert!(!episodio.has_deadline());

        // Depois de uma entrada, um novo laço é um novo episódio.
        let agora = last + Duration::from_millis(10);
        pacer.input(agora);
        let (_, episodios) = pintar(
            &mut pacer,
            agora,
            agora + Duration::from_secs(8),
            30,
            Reason::Deadline,
        );
        assert_eq!(episodios.len(), 1);
        let (_, episodio) = &episodios[0];
        assert!(episodio.has_deadline());
        assert_eq!(
            episodio.line("Janela(2)", Some("ProgressRing NodeId(7)")),
            "tomni-ui: Janela(2) pintou 30 quadros/s por 5 s sem entrada; \
             limitando a 10/s (motivos: prazo 30); prazo de: ProgressRing NodeId(7)"
        );
    }

    #[test]
    fn abaixo_de_20_quadros_por_segundo_nao_limita() {
        for fps in [2, 10, 19, 20] {
            let t = Instant::now();
            let mut pacer = Pacer::new(t);
            let (_, episodios) = pintar(
                &mut pacer,
                t,
                t + Duration::from_secs(60),
                fps,
                Reason::Deadline,
            );
            assert!(episodios.is_empty(), "{fps} quadros/s não limitam");
            assert!(!pacer.limited());
        }
    }

    #[test]
    fn uma_pausa_ou_uma_entrada_recomeca_a_contagem() {
        let t = Instant::now();
        let mut pacer = Pacer::new(t);
        let segundo = Duration::from_secs(1);
        // 4 s pintando, 2 s parada, 4 s pintando: nenhum trecho chega a 5 s.
        let (_, a) = pintar(&mut pacer, t, t + segundo * 4, 60, Reason::Event);
        let (_, b) = pintar(
            &mut pacer,
            t + segundo * 6,
            t + segundo * 10,
            60,
            Reason::Event,
        );
        assert!(a.is_empty() && b.is_empty());
        // Uma entrada a cada 3 s, com a janela pintando sem parar: nunca limita.
        let inicio = t + segundo * 20;
        for i in 0..10 {
            let de = inicio + segundo * 3 * i;
            pacer.input(de);
            let (_, episodios) = pintar(&mut pacer, de, de + segundo * 3, 60, Reason::Deadline);
            assert!(episodios.is_empty());
        }
        assert!(!pacer.limited());
    }

    #[test]
    fn rastro_devolve_o_motivo_mesmo_desligado() {
        let t = Instant::now();
        let mut rastro = FrameTrace::with(false, t);
        rastro.request(Reason::Surface);
        assert_eq!(rastro.frame(false), Reason::Surface);
        assert_eq!(
            rastro.frame(true),
            Reason::System,
            "sem pedido, é do sistema"
        );
        rastro.request(Reason::Blink);
        assert_eq!(rastro.take(), Some(Reason::Blink));
        assert_eq!(rastro.take(), None);
    }
}
