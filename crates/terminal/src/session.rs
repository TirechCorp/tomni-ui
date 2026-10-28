//! A [`Session`]: a conexão com o programa, a grade e o histórico, que vivem mais que o widget.
//!
//! O [`Terminal`](crate::Terminal) só mostra uma sessão. Trocar o terminal de lugar (outro
//! painel, outra aba, uma estrutura nova) não fecha a conexão nem perde o histórico: a sessão
//! fica no registro até o app chamar [`Session::dispose`].

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::fs::OpenOptions;
use std::io::{self, BufWriter, Write};
use std::path::Path;
use std::rc::{Rc, Weak};
use std::time::{Duration, Instant};

use tomni_ui_core::{Key, Modifiers};
use tomni_ui_reactive::{Get, GetUntracked, Scope, Set, Signal, signal};

use crate::emulator::{DEFAULT_SCROLLBACK, Emulator, Notice, TermProgress};
use crate::keys::{encode_key, encode_paste, encode_text};
use crate::log::LogFilter;
use crate::selection::Selection;
use crate::transport::{Status, Transport};

/// Colunas e linhas de uma sessão nova, antes de algum terminal medir o espaço.
const DEFAULT_GRID: (usize, usize) = (80, 24);
/// Bytes lidos do transporte de cada vez.
pub(crate) const READ_CHUNK: usize = 16 * 1024;

/// O que o [`Session::log_to`] grava.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LogMode {
    /// O texto que se lê na tela: sem as sequências de controle, só o imprimível, o `\n` e o
    /// `\t`.
    Text,
    /// Os bytes como chegaram, com as cores e os controles.
    Raw,
}

/// O destino do log.
enum LogOut {
    /// O arquivo do [`Session::log_to`]: `flush` a cada pedaço.
    File(BufWriter<std::fs::File>),
    /// O escritor do [`Session::log_with`]: o `flush` só vem do [`Session::flush_log`].
    Custom(Box<dyn Write>),
}

/// Para onde o log vai.
struct LogSink {
    mode: LogMode,
    out: LogOut,
    filter: LogFilter,
}

impl LogSink {
    fn write(&mut self, bytes: &[u8]) {
        let text;
        let bytes = match self.mode {
            LogMode::Raw => bytes,
            LogMode::Text => {
                text = self.filter.filter(bytes);
                &text
            }
        };
        // Um erro de disco não pode derrubar a sessão: o log só para de crescer.
        match &mut self.out {
            LogOut::File(writer) => {
                let _ = writer.write_all(bytes);
                let _ = writer.flush();
            }
            LogOut::Custom(writer) => {
                let _ = writer.write_all(bytes);
            }
        }
    }

    fn flush(&mut self) {
        match &mut self.out {
            LogOut::File(writer) => {
                let _ = writer.flush();
            }
            LogOut::Custom(writer) => {
                let _ = writer.flush();
            }
        }
    }
}

/// O texto que chegou pela conexão e ainda não foi lido por um [`OutputTap`].
#[derive(Default)]
struct TapBuffer {
    filter: LogFilter,
    text: String,
}

/// O texto que chega pela conexão, guardado desde o [`Session::tap`] que o criou, limpo como
/// no log de texto (sem as sequências de controle). Cada `OutputTap` guarda o seu; soltá-lo
/// para de guardar.
///
/// Não há teto: o buffer cresce com o que chega entre duas chamadas de [`OutputTap::take`],
/// então quem segura um tap deve chamá-lo a cada verificação.
pub struct OutputTap {
    buffer: Rc<RefCell<TapBuffer>>,
}

impl OutputTap {
    /// O texto que chegou desde a última chamada (ou desde o [`Session::tap`]), e esvazia.
    /// Uma sequência de controle cortada no fim de um bloco termina no texto seguinte.
    pub fn take(&self) -> String {
        std::mem::take(&mut self.buffer.borrow_mut().text)
    }
}

/// O estado de uma sessão: o emulador, a conexão e o que o app lê dela. O [`Terminal`] e o
/// [`TerminalVisual`] guardam um `Rc` dele.
///
/// [`Terminal`]: crate::Terminal
/// [`TerminalVisual`]: crate::TerminalVisual
pub(crate) struct SessionCore {
    key: String,
    name: Option<String>,
    pub(crate) emulator: Emulator,
    /// A seleção do widget, espelhada a cada evento dele.
    pub(crate) selection: Option<Selection>,
    selected: Option<Signal<bool>>,
    pub(crate) transport: Box<dyn Transport>,
    pub(crate) closed: bool,
    /// O motivo do encerramento, se a conexão disse um.
    close_reason: Option<String>,
    buf: Vec<u8>,
    /// Último tamanho avisado ao transporte.
    pub(crate) told: Option<(u16, u16)>,
    /// O título que o programa pôs (`OSC 0` e `OSC 2`).
    program_title: Option<String>,
    /// Quantas vezes o programa pôs o título: o widget compara para chamar o `on_title`.
    pub(crate) title_seq: u64,
    title: Option<Signal<String>>,
    closed_signal: Option<Signal<bool>>,
    /// Se a conexão ainda está abrindo, como a sessão viu da última vez.
    connecting: bool,
    connecting_signal: Option<Signal<bool>>,
    /// O progresso do `OSC 9;4`, `None` com a sessão fechada.
    progress: Option<Signal<TermProgress>>,
    /// Quantos avisos o programa deu (BEL, `OSC 9` com texto e `OSC 777;notify`).
    alerts: Option<Signal<u64>>,
    /// Quantos desses avisos a pessoa já viu ([`Session::mark_seen`]).
    seen: Option<Signal<u64>>,
    /// O último aviso que veio com texto.
    last_notice: Option<Signal<Option<Notice>>>,
    /// Quantas vezes tocou a campainha: o widget compara para acender o lampejo.
    pub(crate) bell: u64,
    /// O diretório que o programa informou pelo `OSC 7`; some quando a sessão reabre.
    cwd: Option<String>,
    /// Os textos do `OSC 52` que o widget ainda não pôs na área de transferência.
    pub(crate) clipboard_out: Vec<String>,
    log: Option<LogSink>,
    /// Quantos bytes chegaram pela conexão, somando as reaberturas.
    received: u64,
    /// Quantos bytes foram para a conexão, somando as reaberturas.
    sent: u64,
    /// O tempo da interface com a sessão: ler, interpretar e pintar.
    pub(crate) busy: Duration,
    /// Os [`OutputTap`]s vivos; os soltos saem da lista na próxima leitura.
    taps: Vec<Weak<RefCell<TapBuffer>>>,
}

impl SessionCore {
    /// Guarda a seleção do widget; devolve o sinal a acertar (fora do empréstimo) se mudou.
    pub(crate) fn set_selection(
        &mut self,
        selection: Option<Selection>,
    ) -> Option<(Signal<bool>, bool)> {
        if self.selection == selection {
            return None;
        }
        self.selection = selection;
        let has = selection.is_some();
        self.selected.map(|signal| (signal, has))
    }

    pub(crate) fn new(transport: Box<dyn Transport>, key: String) -> Self {
        let (cols, rows) = DEFAULT_GRID;
        Self {
            connecting: transport.connecting(),
            key,
            name: None,
            emulator: Emulator::new(cols, rows, DEFAULT_SCROLLBACK),
            selection: None,
            selected: None,
            transport,
            closed: false,
            close_reason: None,
            buf: Vec::with_capacity(READ_CHUNK),
            told: None,
            program_title: None,
            title_seq: 0,
            title: None,
            closed_signal: None,
            connecting_signal: None,
            progress: None,
            alerts: None,
            seen: None,
            last_notice: None,
            bell: 0,
            cwd: None,
            clipboard_out: Vec::new(),
            log: None,
            received: 0,
            sent: 0,
            busy: Duration::ZERO,
            taps: Vec::new(),
        }
    }

    /// O título da sessão: o do programa, senão o nome, senão a chave.
    fn current_title(&self) -> String {
        self.program_title
            .as_deref()
            .filter(|title| !title.is_empty())
            .or(self.name.as_deref())
            .unwrap_or(&self.key)
            .to_owned()
    }

    /// Põe o título no signal, só quando muda (quem lê não roda à toa).
    fn sync_title(&self) {
        if let Some(signal) = self.title {
            let title = self.current_title();
            if signal.get_untracked() != title {
                signal.set(title);
            }
        }
    }

    /// Manda bytes ao programa. Um erro fecha a conexão.
    pub(crate) fn write(&mut self, bytes: &[u8]) {
        if self.closed || bytes.is_empty() {
            return;
        }
        match self.transport.write(bytes) {
            Ok(()) => self.sent += bytes.len() as u64,
            Err(error) => self.close(Some(error.to_string())),
        }
    }

    /// Fecha e avisa na tela.
    pub(crate) fn close(&mut self, reason: Option<String>) {
        if self.closed {
            return;
        }
        self.closed = true;
        self.close_reason.clone_from(&reason);
        self.set_connecting(false);
        let strings = tomni_ui_core::strings();
        let message = match reason {
            Some(reason) => format!(
                "\r\n\x1b[0m{}\r\n",
                strings
                    .connection_closed_reason
                    .replace("{reason}", &reason)
            ),
            None => format!("\r\n\x1b[0m{}\r\n", strings.connection_closed),
        };
        // Uma aba caída não mostra anel: o progresso do programa acaba com a conexão.
        self.emulator.feed(b"\x1b]9;4;0\x07");
        self.emulator.feed(message.as_bytes());
        self.sync_progress();
        if let Some(signal) = self.closed_signal {
            signal.set(true);
        }
    }

    /// Põe `connecting` no campo e no signal, se mudou.
    fn set_connecting(&mut self, connecting: bool) {
        if self.connecting == connecting {
            return;
        }
        self.connecting = connecting;
        if let Some(signal) = self.connecting_signal {
            signal.set(connecting);
        }
    }

    /// Confere com o transporte se a conexão acabou de abrir.
    fn sync_connecting(&mut self) {
        let connecting = !self.closed && self.transport.connecting();
        self.set_connecting(connecting);
    }

    /// Se vale a pena chamar o [`SessionCore::pump`]: chegaram bytes ou o estado de conectando
    /// mudou.
    pub(crate) fn needs_poll(&self) -> bool {
        !self.closed && (self.transport.pending() || self.transport.connecting() != self.connecting)
    }

    /// Passa a mudança de progresso do emulador para o signal.
    fn sync_progress(&mut self) {
        if let Some(progress) = self.emulator.take_progress_change()
            && let Some(signal) = self.progress
            && signal.get_untracked() != progress
        {
            signal.set(progress);
        }
    }

    /// Soma `count` ao contador de avisos.
    fn raise_alerts(&self, count: u64) {
        if let Some(signal) = self.alerts {
            signal.set(signal.get_untracked() + count);
        }
    }

    /// Conta `bytes` que chegaram pela conexão e entrega o texto deles aos taps vivos.
    fn receive(&mut self, bytes: &[u8]) {
        self.received += bytes.len() as u64;
        self.taps.retain(|tap| {
            let Some(buffer) = tap.upgrade() else {
                return false;
            };
            let mut buffer = buffer.borrow_mut();
            let text = buffer.filter.filter(bytes);
            buffer.text.push_str(&String::from_utf8_lossy(&text));
            true
        });
    }

    /// Interpreta `bytes` como se viessem do programa, e grava no log.
    pub(crate) fn feed(&mut self, bytes: &[u8]) {
        if bytes.is_empty() {
            return;
        }
        if let Some(log) = &mut self.log {
            log.write(bytes);
        }
        self.emulator.feed(bytes);
        self.after_output();
    }

    /// Depois de bytes do programa: manda as respostas, anota o título e a campainha.
    fn after_output(&mut self) {
        let responses = self.emulator.take_responses();
        self.write(&responses);
        if self.emulator.take_title_change().is_some() {
            self.program_title = Some(self.emulator.title().to_owned());
            self.title_seq += 1;
            self.sync_title();
        }
        self.sync_progress();
        if self.emulator.take_bell() {
            self.bell += 1;
            self.raise_alerts(1);
        }
        if let Some(cwd) = self.emulator.take_cwd() {
            self.cwd = Some(cwd);
        }
        self.clipboard_out.extend(self.emulator.take_clipboard());
        let notices = self.emulator.take_notices();
        if let Some(last) = notices.last().cloned() {
            self.raise_alerts(notices.len() as u64);
            if let Some(signal) = self.last_notice {
                signal.set(Some(last));
            }
        }
    }

    /// Lê o que chegou pela conexão, em blocos, até acabar ou passar de `budget`. Devolve se
    /// leu alguma coisa (ou se fechou).
    pub(crate) fn pump(&mut self, budget: Option<Duration>) -> bool {
        let start = Instant::now();
        let mut read = false;
        self.sync_connecting();
        while !self.closed && self.transport.pending() {
            self.buf.clear();
            let status = self.transport.read(&mut self.buf, READ_CHUNK);
            read = true;
            // O buffer sai da struct para o `feed` poder mexer nela.
            let buf = std::mem::take(&mut self.buf);
            self.receive(&buf);
            self.feed(&buf);
            self.buf = buf;
            if let Status::Closed(reason) = status {
                self.close(reason);
            }
            if budget.is_some_and(|budget| start.elapsed() >= budget) {
                break;
            }
        }
        if read {
            self.busy += start.elapsed();
        }
        read
    }

    /// Bytes digitados: voltam a vista ao fim e vão ao programa.
    pub(crate) fn input(&mut self, bytes: &[u8]) {
        if bytes.is_empty() {
            return;
        }
        self.emulator.set_display_offset(0);
        self.write(bytes);
    }

    /// Uma tecla, nos códigos do xterm. Devolve se virou bytes.
    pub(crate) fn key(&mut self, key: Key, mods: Modifiers) -> bool {
        let modes = self.emulator.modes();
        match encode_key(key, mods, &modes) {
            Some(bytes) => {
                self.input(&bytes);
                true
            }
            None => false,
        }
    }

    /// Texto digitado.
    pub(crate) fn text(&mut self, text: &str, mods: Modifiers) {
        self.input(&encode_text(text, mods));
    }

    /// Texto confirmado pelo IME.
    pub(crate) fn commit(&mut self, text: &str) {
        self.input(text.as_bytes());
    }

    /// Texto colado, entre as marcas de colar se o programa pediu.
    pub(crate) fn paste(&mut self, text: &str) {
        let bytes = encode_paste(text, &self.emulator.modes());
        self.input(&bytes);
    }

    /// A grade mudou: o emulador muda, e o transporte é avisado se o tamanho é outro.
    pub(crate) fn resize(&mut self, cols: u16, rows: u16) {
        self.emulator.resize(usize::from(cols), usize::from(rows));
        if self.told != Some((cols, rows)) {
            self.told = Some((cols, rows));
            self.transport.resize(cols, rows);
        }
    }
}

#[derive(Default)]
struct Registry {
    /// O escopo dono dos signals das sessões: não é filho de nenhum widget.
    scope: Option<Scope>,
    sessions: BTreeMap<u64, Rc<RefCell<SessionCore>>>,
    next: u64,
}

thread_local! {
    static REGISTRY: RefCell<Registry> = RefCell::new(Registry::default());
}

/// Uma conexão com o seu terminal virtual: o programa do outro lado, a grade e o histórico.
///
/// A sessão vive num registro da thread da interface e a `Session` é só o identificador dela
/// (`Copy`), por isso cabe num `Signal`. Ela **não** acaba quando o [`Terminal`] que a mostra
/// sai da árvore: continua lendo a conexão ([`Session::poll_all`], ou o [`TerminalPanes`](crate::TerminalPanes)) e
/// guardando o histórico, e outro terminal pode mostrá-la depois. Quem acaba com ela é o app,
/// com [`Session::dispose`].
///
/// ```
/// use tomni_ui_terminal::{Loopback, Session};
///
/// let sessao = Session::new(Loopback::new().echo()).key("web-01").name("Servidor web");
/// assert_eq!(sessao.title(), "Servidor web");
/// sessao.feed(b"\x1b]0;root@web-01\x07");
/// assert_eq!(sessao.title(), "root@web-01");
/// sessao.dispose();
/// ```
///
/// Uma sessão aparece em **um** terminal por vez (o doc não confere em tempo de execução).
/// Depois de [`Session::dispose`], os métodos não fazem nada e as consultas dão o valor vazio.
///
/// [`Terminal`]: crate::Terminal
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct Session(u64);

impl Session {
    /// Sessão nova ligada a `transport`. A chave começa como `sessao-N` e o título é a chave.
    pub fn new(transport: impl Transport + 'static) -> Self {
        Self::from_boxed(Box::new(transport))
    }

    /// Como [`Session::new`], para um transporte que já está numa `Box`.
    pub(crate) fn from_boxed(transport: Box<dyn Transport>) -> Self {
        REGISTRY.with(|registry| {
            let mut registry = registry.borrow_mut();
            registry.next += 1;
            let id = registry.next;
            let mut core = SessionCore::new(transport, format!("sessao-{id}"));
            let scope = registry.scope.get_or_insert_with(Scope::detached);
            let (title, closed, connecting, progress, alerts, seen, last_notice, selected) = scope
                .run(|| {
                    (
                        signal(core.key.clone()),
                        signal(false),
                        signal(core.connecting),
                        signal(TermProgress::None),
                        signal(0_u64),
                        signal(0_u64),
                        signal(None),
                        signal(false),
                    )
                });
            core.title = Some(title);
            core.closed_signal = Some(closed);
            core.connecting_signal = Some(connecting);
            core.progress = Some(progress);
            core.alerts = Some(alerts);
            core.seen = Some(seen);
            core.last_notice = Some(last_notice);
            core.selected = Some(selected);
            registry.sessions.insert(id, Rc::new(RefCell::new(core)));
            Self(id)
        })
    }

    /// Se há bytes esperando na conexão (e ela ainda está aberta).
    pub(crate) fn pending(self) -> bool {
        self.with(|core| core.needs_poll()).unwrap_or(false)
    }

    /// Lê o que chegou, até `budget` (sem limite com `None`). Devolve se leu algo.
    pub(crate) fn pump(self, budget: Option<Duration>) -> bool {
        self.with(|core| core.pump(budget)).unwrap_or(false)
    }

    /// O núcleo, para o `Terminal` e o `TerminalVisual`.
    pub(crate) fn core(self) -> Option<Rc<RefCell<SessionCore>>> {
        REGISTRY.with(|registry| registry.borrow().sessions.get(&self.0).cloned())
    }

    fn with<R>(self, f: impl FnOnce(&mut SessionCore) -> R) -> Option<R> {
        let core = self.core()?;
        let mut core = core.borrow_mut();
        Some(f(&mut core))
    }

    /// Dá a chave à sessão: o nome estável que o app usa para achá-la ([`Session::find`]), para
    /// os ids dos terminais e para o `Broadcast::Group`. Devolve a mesma sessão.
    pub fn key(self, key: impl Into<String>) -> Self {
        let key = key.into();
        self.with(|core| {
            core.key = key;
            core.sync_title();
        });
        self
    }

    /// Dá o nome mostrado na aba, enquanto o programa não puser um título. Devolve a mesma
    /// sessão.
    pub fn name(self, name: impl Into<String>) -> Self {
        let name = name.into();
        self.with(|core| {
            core.name = Some(name);
            core.sync_title();
        });
        self
    }

    /// A sessão de chave `key`, se existir.
    pub fn find(key: &str) -> Option<Session> {
        REGISTRY.with(|registry| {
            registry
                .borrow()
                .sessions
                .iter()
                .find(|(_, core)| core.borrow().key == key)
                .map(|(&id, _)| Self(id))
        })
    }

    /// O texto que está selecionado no [`Terminal`] desta sessão, ou `None` sem seleção (ou com
    /// ela vazia). É o que o widget mostra agora: o programa escrevendo mais linhas pode mover o
    /// texto sob a seleção. Não mexe na seleção nem na área de transferência.
    ///
    /// [`Terminal`]: crate::Terminal
    pub fn selection_text(&self) -> Option<String> {
        self.with(|core| {
            let text = core.selection?.text(&core.emulator);
            (!text.is_empty()).then_some(text)
        })
        .flatten()
    }

    /// Se há texto selecionado no [`Terminal`] desta sessão. Reativo: serve a quem mostra ou
    /// esconde um item de menu conforme a seleção (`MenuFlyout::item_visible`).
    ///
    /// [`Terminal`]: crate::Terminal
    pub fn has_selection(&self) -> bool {
        self.core()
            .and_then(|core| core.borrow().selected)
            .is_some_and(|signal| signal.get())
    }

    /// A chave da sessão (o `key` sozinho é o builder).
    pub fn key_str(&self) -> String {
        self.with(|core| core.key.clone()).unwrap_or_default()
    }

    /// O título da aba: o que o programa pôs (`OSC 0` e `OSC 2`), senão o nome, senão a chave.
    /// Reativo: quem lê num efeito ou numa closure de propriedade roda de novo quando muda.
    pub fn title(&self) -> String {
        self.core()
            .and_then(|core| core.borrow().title)
            .map(|signal| signal.get())
            .unwrap_or_default()
    }

    /// Se a conexão foi encerrada (pelo programa, por um erro ou pelo app). Reativo.
    pub fn closed(&self) -> bool {
        self.core()
            .and_then(|core| core.borrow().closed_signal)
            .is_none_or(|signal| signal.get())
    }

    /// Se a conexão ainda está abrindo: o SSH autenticando e pedindo o shell, o Telnet
    /// conectando. Começa `true` na criação (se o transporte diz que está conectando) e depois
    /// de um [`Session::restart`], e vira `false` quando a conexão fica pronta para digitar ou
    /// a sessão fecha. Transportes que já nascem prontos (o `Process`, a `Serial`) ficam
    /// sempre `false`. Reativo.
    pub fn connecting(&self) -> bool {
        self.core()
            .and_then(|core| core.borrow().connecting_signal)
            .is_some_and(|signal| signal.get())
    }

    /// O progresso que o programa informou pelo `OSC 9;4`, ou [`TermProgress::None`]. Volta a
    /// `None` quando a sessão fecha ou reinicia. Reativo.
    pub fn progress(&self) -> TermProgress {
        self.core()
            .and_then(|core| core.borrow().progress)
            .map(|signal| signal.get())
            .unwrap_or_default()
    }

    /// Quantos avisos o programa deu até agora: cada BEL, cada `OSC 9` com texto e cada
    /// `OSC 777;notify`. Só cresce; quem olha (a aba) compara com o que viu antes. Reativo.
    pub fn alerts(&self) -> u64 {
        self.core()
            .and_then(|core| core.borrow().alerts)
            .map_or(0, |signal| signal.get())
    }

    /// Se há avisos que a pessoa ainda não viu: [`alerts`](Self::alerts) passou do que
    /// [`mark_seen`](Self::mark_seen) registrou. É o que acende o ponto da aba. Reativo.
    pub fn attention(&self) -> bool {
        let Some(core) = self.core() else {
            return false;
        };
        let (alerts, seen) = {
            let core = core.borrow();
            (core.alerts, core.seen)
        };
        match (alerts, seen) {
            (Some(alerts), Some(seen)) => alerts.get() > seen.get(),
            _ => false,
        }
    }

    /// Dá os avisos de agora por vistos: [`attention`](Self::attention) volta a ser `false` até
    /// o próximo. Só grava se mudou. Pode ser chamado de dentro de um efeito.
    pub fn mark_seen(&self) {
        let Some(core) = self.core() else {
            return;
        };
        let (alerts, seen) = {
            let core = core.borrow();
            (core.alerts, core.seen)
        };
        if let (Some(alerts), Some(seen)) = (alerts, seen) {
            let now = alerts.get_untracked();
            if seen.get_untracked() != now {
                seen.set(now);
            }
        }
    }

    /// O último aviso que veio com texto (`OSC 9` ou `OSC 777;notify`); o BEL não tem texto e não
    /// entra. Reativo.
    pub fn last_notice(&self) -> Option<Notice> {
        self.core()
            .and_then(|core| core.borrow().last_notice)
            .and_then(|signal| signal.get())
    }

    /// O motivo do encerramento, se a conexão disse um (o mesmo texto do "[conexão encerrada: …]"
    /// na tela). `None` enquanto a sessão está aberta, e quando ela fechou sem motivo. Reativo,
    /// como [`Session::closed`]; o [`Session::restart`] limpa.
    pub fn close_reason(&self) -> Option<String> {
        let core = self.core()?;
        let signal = core.borrow().closed_signal?;
        signal.get();
        core.borrow().close_reason.clone()
    }

    /// Manda `bytes` ao programa, como se fossem digitados.
    pub fn send(&self, bytes: &[u8]) {
        self.with(|core| core.input(bytes));
    }

    /// Cola `text` como o Ctrl+Shift+V cola: entre as marcas do bracketed paste se o programa
    /// pediu. Não pergunta nada e não vai para outras sessões.
    pub fn paste(&self, text: &str) {
        self.with(|core| core.paste(text));
    }

    /// Quantos bytes chegaram pela conexão desde que a sessão abriu, somando as reaberturas. O
    /// que o app escreve com [`Session::feed`] não conta. Serve para saber se a saída parou.
    pub fn received(&self) -> u64 {
        self.with(|core| core.received).unwrap_or(0)
    }

    /// Quantos bytes foram para a conexão desde que a sessão abriu, somando as reaberturas: o
    /// digitado, o colado e o [`Session::send`].
    pub fn sent(&self) -> u64 {
        self.with(|core| core.sent).unwrap_or(0)
    }

    /// Quanto tempo a interface gastou com a sessão desde que ela abriu: ler o que chegou,
    /// interpretar e pintar a tela. Com o [`Session::threads`], dá a CPU de cada aba.
    pub fn busy(&self) -> Duration {
        self.with(|core| core.busy).unwrap_or(Duration::ZERO)
    }

    /// Os ids, no sistema, das threads que a conexão usa agora (`/proc/self/task/<id>` no
    /// Linux), para o app medir a CPU delas. Vazio nos outros sistemas, nas conexões sem
    /// thread e depois de a sessão acabar.
    pub fn threads(&self) -> Vec<u32> {
        self.with(|core| core.transport.threads())
            .unwrap_or_default()
    }

    /// Passa a guardar o texto que chega pela conexão, limpo como no log de texto (sem as
    /// sequências de controle). Cada [`OutputTap`] guarda o seu; solte-o para parar. O que o
    /// app escreve com [`Session::feed`] não entra, e o tap continua depois do
    /// [`Session::restart`]. Numa sessão já descartada, o tap nunca recebe nada.
    pub fn tap(&self) -> OutputTap {
        let buffer = Rc::new(RefCell::new(TapBuffer::default()));
        let weak = Rc::downgrade(&buffer);
        self.with(|core| core.taps.push(weak));
        OutputTap { buffer }
    }

    /// Interpreta `bytes` como se viessem do programa (e grava no log).
    pub fn feed(&self, bytes: &[u8]) {
        self.with(|core| core.feed(bytes));
    }

    /// Reabre uma sessão encerrada, com a mesma configuração. O
    /// histórico fica, a tela não é limpa. Devolve `false` se a sessão não está encerrada ou se
    /// o transporte não sabe reabrir.
    pub fn restart(&self) -> bool {
        self.with(|core| {
            if !core.closed || !core.transport.reopen() {
                return false;
            }
            core.closed = false;
            core.close_reason = None;
            core.told = None;
            core.cwd = None;
            if let Some(signal) = core.closed_signal {
                signal.set(false);
            }
            core.sync_connecting();
            let (cols, rows) = (core.emulator.cols(), core.emulator.rows());
            core.resize(cols as u16, rows as u16);
            true
        })
        .unwrap_or(false)
    }

    /// O diretório atual que o programa informou pelo `OSC 7` (`ESC ] 7 ; file://máquina/caminho`,
    /// que o `bash` e o `zsh` mandam com um `PROMPT_COMMAND` ou um `precmd`), já com os `%XX`
    /// decodificados. `None` se o programa nunca informou, ou depois de [`Session::restart`].
    pub fn cwd(&self) -> Option<String> {
        self.with(|core| core.cwd.clone()).flatten()
    }

    /// O número do processo local por trás da sessão (o filho do pseudoterminal de um
    /// `Process`), para o app olhar o sistema (`/proc/<pid>` no Linux). `None`
    /// nos outros transportes (SSH, Telnet, serial) e depois de a sessão acabar.
    pub fn process_id(&self) -> Option<u32> {
        self.with(|core| core.transport.process_id()).flatten()
    }

    /// Abre uma sessão nova com a mesma configuração (outra conexão, o mesmo nome), para
    /// duplicar. A chave é nova. `None` se a sessão acabou ou o transporte não sabe
    /// duplicar.
    pub fn duplicate(&self) -> Option<Session> {
        self.duplicate_in(None)
    }

    /// Como [`duplicate`](Self::duplicate), mas a cópia começa em `dir`. Só o `Process`
    /// (o shell local) sabe: o processo novo nasce na pasta, se ela existir, sem digitar nada. Nos
    /// outros transportes (SSH, Telnet, serial) a pasta é ignorada e o resultado é o do
    /// `duplicate`; quem quer a pasta num servidor digita um `cd` depois do prompt.
    pub fn duplicate_in(&self, dir: Option<&std::path::Path>) -> Option<Session> {
        let (transport, name) =
            self.with(|core| (core.transport.duplicate_in(dir), core.name.clone()))?;
        let copy = Session::from_boxed(transport?);
        if let Some(name) = name {
            copy.with(|core| {
                core.name = Some(name);
                core.sync_title();
            });
        }
        Some(copy)
    }

    /// Como [`duplicate`](Self::duplicate), mas a cópia roda `command` no lugar do programa da
    /// original (`None`: o shell), com a mesma autenticação. É o que duplica uma aba SSH que roda
    /// `tmux new-session -A -s nome`: a cópia pede outro nome, senão cairia na mesma sessão do
    /// tmux. `None` se a sessão acabou ou o transporte não sabe (só o SSH e o `Loopback` sabem).
    pub fn duplicate_running(&self, command: Option<&str>) -> Option<Session> {
        let (transport, name) =
            self.with(|core| (core.transport.duplicate_running(command), core.name.clone()))?;
        let copy = Session::from_boxed(transport?);
        if let Some(name) = name {
            copy.with(|core| {
                core.name = Some(name);
                core.sync_title();
            });
        }
        Some(copy)
    }

    /// Por onde abrir canais de subsistema (o SFTP) na conexão SSH desta sessão, sem autenticar
    /// de novo. `None` se a sessão não é SSH ou já acabou.
    #[cfg(feature = "ssh")]
    pub fn ssh_channels(&self) -> Option<crate::ssh::SshChannelOpener> {
        self.with(|core| core.transport.ssh_channels()).flatten()
    }

    /// Os túneis (`-L`, `-R`, `-D`, socket Unix) e o X11 da conexão SSH desta sessão, para a
    /// tela listar com os contadores e para subir, parar e reiniciar. `None` se a sessão não é
    /// SSH. Depois de um [`restart`](Self::restart), peça de novo.
    #[cfg(feature = "ssh")]
    pub fn ssh_forwards(&self) -> Option<crate::ssh::SshForwards> {
        self.with(|core| core.transport.ssh_forwards()).flatten()
    }

    /// Passa a gravar o que o programa escreve em `path`, em modo de acréscimo e sem
    /// cabeçalho (o arquivo é criado se não existir). Troca o log anterior, se havia.
    ///
    /// # Errors
    ///
    /// Se o arquivo não puder ser aberto.
    pub fn log_to(&self, path: impl AsRef<Path>, mode: LogMode) -> io::Result<()> {
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(path.as_ref())?;
        self.with(|core| {
            core.log = Some(LogSink {
                mode,
                out: LogOut::File(BufWriter::new(file)),
                filter: LogFilter::default(),
            });
        });
        Ok(())
    }

    /// Passa a gravar o que o programa escreve em `writer` (no modo `mode`), no lugar do log
    /// anterior, se havia. É o `log_to` para um destino que não é um arquivo simples (um
    /// arquivo cifrado, por exemplo).
    ///
    /// O `flush` do escritor só é chamado por [`Session::flush_log`]: o app decide quando
    /// fechar um registro. Soltar o log ([`Session::stop_log`], outro `log_to` ou `log_with`, ou
    /// o fim da sessão) solta o escritor, e é o `Drop` dele que termina o arquivo. Um erro do
    /// escritor não derruba a sessão: o log só para de crescer.
    ///
    /// ```
    /// use std::cell::RefCell;
    /// use std::io::{self, Write};
    /// use std::rc::Rc;
    /// use tomni_ui_terminal::{Loopback, LogMode, Session};
    ///
    /// struct Guarda(Rc<RefCell<Vec<u8>>>);
    /// impl Write for Guarda {
    ///     fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
    ///         self.0.borrow_mut().extend_from_slice(buf);
    ///         Ok(buf.len())
    ///     }
    ///     fn flush(&mut self) -> io::Result<()> {
    ///         Ok(())
    ///     }
    /// }
    ///
    /// let guardado = Rc::new(RefCell::new(Vec::new()));
    /// let sessao = Session::new(Loopback::new());
    /// sessao.log_with(Box::new(Guarda(guardado.clone())), LogMode::Text);
    /// sessao.feed(b"\x1b[31mok\x1b[0m\r\n");
    /// assert_eq!(&*guardado.borrow(), b"ok\n");
    /// sessao.dispose();
    /// ```
    pub fn log_with(&self, writer: Box<dyn Write>, mode: LogMode) {
        self.with(|core| {
            core.log = Some(LogSink {
                mode,
                out: LogOut::Custom(writer),
                filter: LogFilter::default(),
            });
        });
    }

    /// Chama o `flush` do escritor do log, se houver. No [`log_to`](Self::log_to), que já dá
    /// `flush` a cada pedaço, não muda nada. Os erros são ignorados, como na gravação.
    pub fn flush_log(&self) {
        self.with(|core| {
            if let Some(log) = core.log.as_mut() {
                log.flush();
            }
        });
    }

    /// Se a sessão está gravando um log.
    pub fn is_logging(&self) -> bool {
        self.with(|core| core.log.is_some()).unwrap_or(false)
    }

    /// Para de gravar e fecha o arquivo do log.
    pub fn stop_log(&self) {
        self.with(|core| core.log = None);
    }

    /// O texto da tela, sem o histórico: uma linha por linha, sem os espaços do fim e sem as
    /// linhas vazias do fim.
    pub fn text(&self) -> String {
        self.with(|core| core.emulator.screen_text())
            .unwrap_or_default()
    }

    /// O texto do histórico e da tela, em linhas lógicas (as quebras automáticas juntadas).
    pub fn history(&self) -> String {
        self.with(|core| core.emulator.all_text())
            .unwrap_or_default()
    }

    /// Lê o emulador: a grade, o cursor, os modos. `None` se a sessão foi descartada.
    ///
    /// # Panics
    ///
    /// Se chamada de dentro de uma ação que muda a sessão.
    pub fn emulator<R>(&self, f: impl FnOnce(&Emulator) -> R) -> Option<R> {
        let core = self.core()?;
        let core = core.borrow();
        Some(f(&core.emulator))
    }

    /// Descarta a sessão: fecha a conexão (soltar o transporte a encerra) e libera o registro.
    /// O terminal que a mostrava continua com a tela até sair da árvore.
    pub fn dispose(self) {
        let core = REGISTRY.with(|registry| registry.borrow_mut().sessions.remove(&self.0));
        // Fora do `with` da thread: soltar o transporte pode acordar a interface.
        drop(core);
    }

    /// Se a sessão ainda está no registro (não passou por [`Session::dispose`]).
    pub fn is_alive(&self) -> bool {
        REGISTRY.with(|registry| registry.borrow().sessions.contains_key(&self.0))
    }

    /// Lê o que chegou pela conexão de todas as sessões, sem o limite de tempo do frame. É o que
    /// o `TestApp` faz a cada passo; um app com terminais montados não precisa chamar. Devolve se
    /// alguma leu alguma coisa.
    pub fn poll_all() -> bool {
        let cores: Vec<_> =
            REGISTRY.with(|registry| registry.borrow().sessions.values().cloned().collect());
        let mut read = false;
        for core in cores {
            read |= core.borrow_mut().pump(None);
        }
        read
    }
}

#[cfg(test)]
mod tests {
    use tomni_ui_reactive::{Scope as TestScope, effect, run_effects};

    use super::*;
    use crate::transport::Loopback;

    #[test]
    fn titulo_vem_do_programa_depois_do_nome_e_da_chave() {
        let s = Session::new(Loopback::new());
        assert!(s.title().starts_with("sessao-"));
        let s = s.key("web-01");
        assert_eq!(s.title(), "web-01");
        let s = s.name("Servidor web");
        assert_eq!(s.title(), "Servidor web");
        s.feed(b"\x1b]0;root@web-01\x07");
        assert_eq!(s.title(), "root@web-01");
        s.feed(b"\x1b]2;\x07");
        assert_eq!(s.title(), "Servidor web");
        assert_eq!(Session::find("web-01"), Some(s));
        assert_eq!(s.key_str(), "web-01");
        s.dispose();
        assert_eq!(Session::find("web-01"), None);
    }

    #[test]
    fn titulo_e_closed_sao_reativos() {
        let scope = TestScope::root();
        scope.run(|| {
            let s = Session::new(Loopback::new()).key("a");
            let visto = Rc::new(RefCell::new(Vec::new()));
            let dentro = Rc::clone(&visto);
            effect(move || dentro.borrow_mut().push((s.title(), s.closed())));
            run_effects();
            s.feed(b"\x1b]0;novo\x07");
            run_effects();
            // O mesmo título não acorda o efeito de novo.
            s.feed(b"\x1b]0;novo\x07");
            run_effects();
            s.core().unwrap().borrow_mut().close(None);
            run_effects();
            assert_eq!(
                *visto.borrow(),
                [
                    ("a".to_owned(), false),
                    ("novo".to_owned(), false),
                    ("novo".to_owned(), true)
                ]
            );
        });
        scope.clear();
    }

    #[test]
    fn send_vai_ao_transporte_e_feed_vai_a_tela() {
        let lb = Loopback::new();
        let handle = lb.handle();
        let s = Session::new(lb);
        s.send(b"ls\r");
        assert_eq!(handle.take_written(), b"ls\r");
        s.feed("olá\r\nmundo".as_bytes());
        assert_eq!(s.text(), "olá\nmundo");
        assert_eq!(s.history(), "olá\nmundo");
        assert_eq!(s.emulator(|e| e.cols()), Some(80));
    }

    #[test]
    fn poll_all_le_o_que_chegou_sem_widget() {
        let lb = Loopback::new();
        let handle = lb.handle();
        let s = Session::new(lb);
        handle.feed("de fora");
        assert!(Session::poll_all());
        assert_eq!(s.text(), "de fora");
        assert!(!Session::poll_all());
        handle.close(Some("caiu".into()));
        Session::poll_all();
        assert!(s.closed());
        assert!(s.text().contains("[conexão encerrada: caiu]"));
    }

    #[test]
    fn dispose_esvazia_a_sessao() {
        let s = Session::new(Loopback::new());
        assert!(s.is_alive());
        s.dispose();
        assert!(!s.is_alive());
        assert!(s.closed());
        assert_eq!(s.title(), "");
        assert_eq!(s.text(), "");
        s.send(b"x");
        assert!(!s.restart());
    }

    #[test]
    fn log_texto_e_bruto_acrescentam() {
        let dir = std::env::temp_dir().join(format!("tomni-ui-log-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let texto = dir.join("texto.log");
        let bruto = dir.join("bruto.log");
        let _ = std::fs::remove_file(&texto);
        let _ = std::fs::remove_file(&bruto);

        let s = Session::new(Loopback::new());
        s.log_to(&texto, LogMode::Text).unwrap();
        s.feed(b"\x1b[31mum\x1b[0m\r\ndois\r\n");
        s.stop_log();
        s.feed(b"fora\r\n");
        s.log_to(&texto, LogMode::Text).unwrap();
        s.feed(b"tres\r\n");
        s.stop_log();
        assert_eq!(std::fs::read_to_string(&texto).unwrap(), "um\ndois\ntres\n");

        s.log_to(&bruto, LogMode::Raw).unwrap();
        s.feed(b"\x1b[1mx\x1b[0m\r\n");
        s.stop_log();
        assert_eq!(std::fs::read(&bruto).unwrap(), b"\x1b[1mx\x1b[0m\r\n");
        let _ = std::fs::remove_dir_all(&dir);
        s.dispose();
    }

    /// Um escritor de teste: guarda os bytes, conta os `flush` e marca o `Drop`.
    struct Escritor {
        bytes: Rc<RefCell<Vec<u8>>>,
        flushes: Rc<std::cell::Cell<usize>>,
        solto: Rc<std::cell::Cell<bool>>,
    }

    impl Write for Escritor {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.bytes.borrow_mut().extend_from_slice(buf);
            Ok(buf.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            self.flushes.set(self.flushes.get() + 1);
            Ok(())
        }
    }

    impl Drop for Escritor {
        fn drop(&mut self) {
            self.solto.set(true);
        }
    }

    #[allow(clippy::type_complexity)]
    fn escritor() -> (
        Box<dyn Write>,
        Rc<RefCell<Vec<u8>>>,
        Rc<std::cell::Cell<usize>>,
        Rc<std::cell::Cell<bool>>,
    ) {
        let bytes = Rc::new(RefCell::new(Vec::new()));
        let flushes = Rc::new(std::cell::Cell::new(0));
        let solto = Rc::new(std::cell::Cell::new(false));
        let e = Escritor {
            bytes: bytes.clone(),
            flushes: flushes.clone(),
            solto: solto.clone(),
        };
        (Box::new(e), bytes, flushes, solto)
    }

    #[test]
    fn log_with_recebe_o_texto_filtrado() {
        let s = Session::new(Loopback::new());
        let (e, bytes, _, _) = escritor();
        s.log_with(e, LogMode::Text);
        s.feed(b"\x1b[31mok\x1b[0m\r\n");
        assert_eq!(&*bytes.borrow(), b"ok\n");
        s.dispose();
    }

    #[test]
    fn log_with_no_modo_bruto_recebe_os_bytes_como_chegaram() {
        let s = Session::new(Loopback::new());
        let (e, bytes, _, _) = escritor();
        s.log_with(e, LogMode::Raw);
        s.feed(b"\x1b[1mx\x1b[0m");
        assert_eq!(&*bytes.borrow(), b"\x1b[1mx\x1b[0m");
        s.dispose();
    }

    #[test]
    fn log_with_so_da_flush_quando_pedem() {
        let s = Session::new(Loopback::new());
        let (e, _, flushes, _) = escritor();
        s.log_with(e, LogMode::Text);
        s.feed(b"um\r\n");
        s.feed(b"dois\r\n");
        assert_eq!(flushes.get(), 0);
        s.flush_log();
        assert_eq!(flushes.get(), 1);
        s.dispose();
    }

    #[test]
    fn flush_log_sem_log_nao_faz_nada() {
        let s = Session::new(Loopback::new());
        s.flush_log();
        assert!(!s.is_logging());
        s.dispose();
    }

    #[test]
    fn stop_log_solta_o_escritor() {
        let s = Session::new(Loopback::new());
        let (e, _, _, solto) = escritor();
        s.log_with(e, LogMode::Text);
        assert!(!solto.get());
        s.stop_log();
        assert!(solto.get());

        // Um `log_with` por cima de outro solta o primeiro.
        let (e1, _, _, solto1) = escritor();
        let (e2, _, _, solto2) = escritor();
        s.log_with(e1, LogMode::Text);
        s.log_with(e2, LogMode::Text);
        assert!(solto1.get());
        assert!(!solto2.get());
        // O `log_to` por cima também.
        let dir = std::env::temp_dir().join(format!("tomni-ui-log-with-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        s.log_to(dir.join("a.log"), LogMode::Text).unwrap();
        assert!(solto2.get());
        s.stop_log();
        let _ = std::fs::remove_dir_all(&dir);

        // E o `dispose` solta o que estiver gravando.
        let (e3, _, _, solto3) = escritor();
        s.log_with(e3, LogMode::Text);
        s.dispose();
        assert!(solto3.get());
    }

    #[test]
    fn is_logging_acompanha_o_log_to_o_log_with_e_o_stop_log() {
        let dir = std::env::temp_dir().join(format!("tomni-ui-is-logging-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let s = Session::new(Loopback::new());
        assert!(!s.is_logging());
        s.log_to(dir.join("a.log"), LogMode::Text).unwrap();
        assert!(s.is_logging());
        s.stop_log();
        assert!(!s.is_logging());
        let (e, _, _, _) = escritor();
        s.log_with(e, LogMode::Text);
        assert!(s.is_logging());
        s.stop_log();
        assert!(!s.is_logging());
        let _ = std::fs::remove_dir_all(&dir);
        s.dispose();
    }

    #[test]
    fn close_reason_guarda_o_motivo_e_o_restart_limpa() {
        let lb = Loopback::new();
        let handle = lb.handle();
        let s = Session::new(lb);
        assert_eq!(s.close_reason(), None);
        handle.close(Some("caiu".into()));
        Session::poll_all();
        assert_eq!(s.close_reason().as_deref(), Some("caiu"));
        assert!(s.restart());
        assert_eq!(s.close_reason(), None);
        handle.close(None);
        Session::poll_all();
        assert!(s.closed());
        assert_eq!(s.close_reason(), None);
        s.dispose();
    }

    #[test]
    fn o_osc_9_4_muda_o_progresso_da_sessao() {
        let s = Session::new(Loopback::new());
        assert_eq!(s.progress(), TermProgress::None);
        s.feed(b"\x1b]9;4;1;45\x07");
        assert_eq!(s.progress(), TermProgress::Normal(45));
        s.feed(b"\x1b]9;4;4\x1b\\");
        assert_eq!(s.progress(), TermProgress::Paused(Some(45)));
        s.feed(b"\x1b]9;4;0\x07");
        assert_eq!(s.progress(), TermProgress::None);
        s.dispose();
    }

    #[test]
    fn o_progresso_e_reativo_e_so_roda_quando_muda() {
        let scope = TestScope::root();
        scope.run(|| {
            let s = Session::new(Loopback::new());
            let visto = Rc::new(RefCell::new(Vec::new()));
            let dentro = Rc::clone(&visto);
            effect(move || dentro.borrow_mut().push(s.progress()));
            run_effects();
            s.feed(b"\x1b]9;4;1;10\x07");
            run_effects();
            // O mesmo valor não acorda o efeito de novo.
            s.feed(b"\x1b]9;4;1;10\x07");
            run_effects();
            s.feed(b"\x1b]9;4;3\x07");
            run_effects();
            assert_eq!(
                *visto.borrow(),
                [
                    TermProgress::None,
                    TermProgress::Normal(10),
                    TermProgress::Indeterminate
                ]
            );
            s.dispose();
        });
        scope.clear();
    }

    #[test]
    fn bel_osc_9_e_osc_777_sobem_os_avisos() {
        let s = Session::new(Loopback::new());
        assert_eq!(s.alerts(), 0);
        assert_eq!(s.last_notice(), None);
        s.feed(b"\x07");
        assert_eq!(s.alerts(), 1);
        assert_eq!(s.last_notice(), None, "o BEL não tem texto");
        s.feed("\x1b]9;Terminou\x07".as_bytes());
        assert_eq!(s.alerts(), 2);
        assert_eq!(s.last_notice().unwrap().body, "Terminou");
        s.feed(b"\x1b]777;notify;Build;Pronto\x07");
        assert_eq!(s.alerts(), 3);
        let aviso = s.last_notice().unwrap();
        assert_eq!(
            (aviso.title.as_deref(), aviso.body.as_str()),
            (Some("Build"), "Pronto")
        );
        // Vários BEL no mesmo bloco contam um só (o emulador guarda um sinal); o `9;9` e o
        // progresso não contam.
        s.feed(b"\x07\x07\x1b]9;9;/tmp\x07\x1b]9;4;1;5\x07");
        assert_eq!(s.alerts(), 4);
        // Dois avisos de texto no mesmo bloco contam os dois.
        s.feed(b"\x1b]9;a\x07\x1b]9;b\x07");
        assert_eq!(s.alerts(), 6);
        assert_eq!(s.last_notice().unwrap().body, "b");
        s.dispose();
    }

    #[test]
    fn a_atencao_acende_com_o_aviso_e_apaga_ao_ver() {
        let s = Session::new(Loopback::new());
        assert!(!s.attention());
        s.feed(b"\x07");
        assert!(s.attention());
        s.mark_seen();
        assert!(!s.attention());
        s.mark_seen();
        assert!(!s.attention(), "ver duas vezes não muda nada");
        s.feed(b"\x1b]9;pronto\x07");
        assert!(s.attention());
        s.dispose();
        assert!(!s.attention(), "sessão descartada não pede atenção");
    }

    #[test]
    fn fechar_e_reiniciar_apagam_o_progresso() {
        let lb = Loopback::new();
        let handle = lb.handle();
        let s = Session::new(lb);
        s.feed(b"\x1b]9;4;1;70\x07");
        assert_eq!(s.progress(), TermProgress::Normal(70));
        handle.close(None);
        Session::poll_all();
        assert!(s.closed());
        assert_eq!(s.progress(), TermProgress::None);
        assert!(s.restart());
        assert_eq!(s.progress(), TermProgress::None);
        // O mesmo valor de antes volta a valer: o emulador não ficou com o anterior.
        s.feed(b"\x1b]9;4;1;70\x07");
        assert_eq!(s.progress(), TermProgress::Normal(70));
        s.dispose();
    }

    #[test]
    fn restart_reabre_no_lugar_e_o_historico_fica() {
        let lb = Loopback::new();
        let handle = lb.handle();
        let s = Session::new(lb);
        // Aberta: não há o que reabrir.
        assert!(!s.restart());
        assert_eq!(handle.reopened(), 0);

        s.feed(b"antes\r\n");
        handle.close(Some("caiu".into()));
        Session::poll_all();
        assert!(s.closed());
        assert!(s.restart());
        assert!(!s.closed());
        assert_eq!(handle.reopened(), 1);
        // O mesmo handle continua valendo, e o que foi escrito antes fica na tela.
        handle.feed("depois\r\n");
        Session::poll_all();
        let texto = s.history();
        assert!(texto.contains("antes"), "{texto}");
        assert!(texto.contains("[conexão encerrada: caiu]"), "{texto}");
        assert!(texto.contains("depois"), "{texto}");
        // O tamanho da grade é avisado de novo à conexão reaberta.
        assert_eq!(handle.size(), Some((80, 24)));
        // Reaberta e aberta: o segundo restart não faz nada.
        assert!(!s.restart());
        assert_eq!(handle.reopened(), 1);
        s.dispose();
    }

    #[test]
    fn duplicate_abre_outra_conexao_com_a_mesma_configuracao() {
        let s = Session::new(Loopback::new().echo())
            .key("a")
            .name("Servidor");
        let copia = s.duplicate().expect("o Loopback sabe duplicar");
        assert_ne!(copia, s);
        assert_ne!(copia.key_str(), "a");
        assert_eq!(copia.title(), "Servidor");
        // O mesmo eco, numa conexão só dela.
        copia.send(b"oi\r");
        Session::poll_all();
        assert_eq!(copia.text(), "oi");
        assert_eq!(s.text(), "");
        s.dispose();
        assert!(s.duplicate().is_none());
        copia.dispose();
    }

    #[test]
    fn duplicate_running_troca_o_comando_da_copia() {
        let lb = Loopback::new().echo().running("tmux new-session -A -s a");
        let original = lb.handle();
        let s = Session::new(lb).key("a").name("Servidor");
        let copia = s
            .duplicate_running(Some("tmux new-session -A -s b"))
            .expect("o Loopback sabe duplicar");
        assert_ne!(copia.key_str(), "a");
        assert_eq!(copia.title(), "Servidor");
        let shell = s.duplicate_running(None).expect("o Loopback sabe duplicar");
        let igual = s.duplicate().expect("o Loopback sabe duplicar");
        assert_eq!(
            original.command().as_deref(),
            Some("tmux new-session -A -s a")
        );
        // O eco vai junto; o comando é o pedido.
        copia.send(b"oi\r");
        Session::poll_all();
        assert_eq!(copia.text(), "oi");
        s.dispose();
        assert!(s.duplicate_running(None).is_none());
        for outra in [copia, shell, igual] {
            outra.dispose();
        }
    }

    #[test]
    fn recebidos_contam_a_conexao_e_nao_o_feed() {
        let lb = Loopback::new().echo();
        let s = Session::new(lb);
        assert_eq!(s.received(), 0);
        s.send(b"abc");
        Session::poll_all();
        assert_eq!(s.received(), 3);
        // O aviso que o app escreve na tela não parece saída do servidor.
        s.feed(b"aviso");
        assert_eq!(s.received(), 3);
        s.dispose();
    }

    #[test]
    fn enviados_e_tempo_de_interface_somam() {
        let lb = Loopback::new().echo();
        let s = Session::new(lb);
        assert_eq!((s.sent(), s.busy()), (0, Duration::ZERO));
        s.send(b"abc");
        s.paste("de");
        assert_eq!(s.sent(), 5);
        Session::poll_all();
        assert!(s.busy() > Duration::ZERO);
        // O Loopback não tem thread.
        assert!(s.threads().is_empty());
        s.dispose();
        assert_eq!((s.sent(), s.busy()), (0, Duration::ZERO));
    }

    #[test]
    fn tap_guarda_o_texto_limpo_que_chega() {
        let lb = Loopback::new();
        let handle = lb.handle();
        let s = Session::new(lb);
        let tap = s.tap();
        handle.feed("\x1b[1mOi\x1b[0m\r\nlá");
        Session::poll_all();
        // Como no log de texto: o `\r` cai e o `\n` fica.
        assert_eq!(tap.take(), "Oi\nlá");
        assert_eq!(tap.take(), "");
        // O que o app escreve com o `feed` não entra.
        s.feed(b"aviso");
        Session::poll_all();
        assert_eq!(tap.take(), "");
        s.dispose();
    }

    #[test]
    fn tap_junta_a_sequencia_cortada_entre_dois_blocos() {
        let lb = Loopback::new();
        let handle = lb.handle();
        let s = Session::new(lb);
        let tap = s.tap();
        handle.feed("a\x1b[3");
        Session::poll_all();
        handle.feed("1mb");
        Session::poll_all();
        assert_eq!(tap.take(), "ab");
        s.dispose();
    }

    #[test]
    fn tap_solto_para_de_guardar() {
        let lb = Loopback::new();
        let handle = lb.handle();
        let s = Session::new(lb);
        let fica = s.tap();
        let sai = s.tap();
        let tamanho = |s: Session| s.core().unwrap().borrow().taps.len();
        assert_eq!(tamanho(s), 2);
        drop(sai);
        handle.feed("x");
        Session::poll_all();
        // O tap solto saiu da lista na leitura; o outro continua.
        assert_eq!(tamanho(s), 1);
        assert_eq!(fica.take(), "x");
        s.dispose();
    }

    #[test]
    fn tap_continua_depois_do_restart() {
        let lb = Loopback::new();
        let handle = lb.handle();
        let s = Session::new(lb);
        let tap = s.tap();
        handle.feed("antes");
        handle.close(Some("caiu".into()));
        Session::poll_all();
        assert!(s.closed());
        assert!(s.restart());
        handle.feed("depois");
        Session::poll_all();
        assert!(tap.take().ends_with("depois"));
        assert_eq!(s.received(), "antesdepois".len() as u64);
        s.dispose();
    }

    #[test]
    fn tap_de_sessao_descartada_fica_vazio() {
        let s = Session::new(Loopback::new());
        s.dispose();
        let tap = s.tap();
        assert_eq!(tap.take(), "");
        assert_eq!(s.received(), 0);
    }
}
