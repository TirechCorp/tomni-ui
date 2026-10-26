//! O [`PaneLayout`]: a árvore de divisões, as abas de cada painel e o painel ativo. É dado puro,
//! sem widget e sem reatividade: o app o guarda num `Signal` e o `TerminalPanes` o desenha.

use std::fmt;

use crate::Session;

/// Quanto uma divisão anda no [`PaneLayout::resize`].
const RESIZE_STEP: f64 = 0.05;
/// Os limites da fração de uma divisão ao redimensionar.
const RATIO_MIN: f64 = 0.1;
const RATIO_MAX: f64 = 0.9;
/// Tolerância das comparações de borda em [`PaneLayout::neighbor`].
const EPSILON: f64 = 1e-6;

/// Um lado de um painel: a direção de uma divisão, de um foco ou de um redimensionamento.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PaneDirection {
    /// Para a esquerda.
    Left,
    /// Para a direita.
    Right,
    /// Para cima.
    Up,
    /// Para baixo.
    Down,
}

impl PaneDirection {
    /// Se a direção anda numa linha (lado a lado), e não numa coluna.
    fn horizontal(self) -> bool {
        matches!(self, Self::Left | Self::Right)
    }

    /// Se a direção vai para o fim do eixo (direita ou baixo).
    fn forward(self) -> bool {
        matches!(self, Self::Right | Self::Down)
    }
}

/// Um painel: as sessões que ele tem como abas e a que está à mostra.
#[derive(Clone, PartialEq, Debug, Default)]
pub struct Pane {
    /// As abas, da esquerda para a direita. Pode estar vazio: o painel fica, esperando uma aba.
    pub tabs: Vec<Session>,
    /// A aba à mostra. `None` só num painel sem abas.
    pub selected: Option<Session>,
}

impl Pane {
    /// Põe `session` em `index` (ou no fim) e a escolhe.
    fn insert(&mut self, session: Session, index: usize) {
        let index = index.min(self.tabs.len());
        self.tabs.insert(index, session);
        self.selected = Some(session);
    }

    /// Tira `session`. Se era a escolhida, escolhe a vizinha (a da direita, ou a da esquerda se
    /// era a última). Devolve se estava aqui.
    fn take(&mut self, session: Session) -> bool {
        let Some(index) = self.tabs.iter().position(|&s| s == session) else {
            return false;
        };
        self.tabs.remove(index);
        if self.selected == Some(session) {
            self.selected = self.tabs.get(index).or(self.tabs.last()).copied();
        }
        true
    }
}

/// A forma da árvore: ou um painel, ou uma divisão em duas partes.
#[derive(Clone, PartialEq, Debug)]
enum Node {
    Pane(Pane),
    Split {
        /// `true` lado a lado (a primeira parte à esquerda), `false` empilhadas.
        row: bool,
        /// A fração que a primeira parte ocupa, entre 0 e 1.
        ratio: f64,
        first: Box<Node>,
        second: Box<Node>,
    },
}

impl Node {
    fn leaf_count(&self) -> usize {
        match self {
            Self::Pane(_) => 1,
            Self::Split { first, second, .. } => first.leaf_count() + second.leaf_count(),
        }
    }

    /// O painel `index` (na ordem das folhas), contando a partir deste nó.
    fn leaf(&self, index: usize) -> Option<&Pane> {
        match self {
            Self::Pane(pane) => (index == 0).then_some(pane),
            Self::Split { first, second, .. } => {
                let count = first.leaf_count();
                if index < count {
                    first.leaf(index)
                } else {
                    second.leaf(index - count)
                }
            }
        }
    }

    fn leaf_mut(&mut self, index: usize) -> Option<&mut Pane> {
        match self {
            Self::Pane(pane) => (index == 0).then_some(pane),
            Self::Split { first, second, .. } => {
                let count = first.leaf_count();
                if index < count {
                    first.leaf_mut(index)
                } else {
                    second.leaf_mut(index - count)
                }
            }
        }
    }

    fn panes<'a>(&'a self, out: &mut Vec<&'a Pane>) {
        match self {
            Self::Pane(pane) => out.push(pane),
            Self::Split { first, second, .. } => {
                first.panes(out);
                second.panes(out);
            }
        }
    }

    fn panes_mut<'a>(&'a mut self, out: &mut Vec<&'a mut Pane>) {
        match self {
            Self::Pane(pane) => out.push(pane),
            Self::Split { first, second, .. } => {
                first.panes_mut(out);
                second.panes_mut(out);
            }
        }
    }

    /// Troca a folha `index` por uma divisão com o painel vazio novo do lado pedido.
    fn split_leaf(&mut self, index: usize, row: bool, new_first: bool) {
        match self {
            Self::Pane(_) => {
                let old = std::mem::replace(self, Self::Pane(Pane::default()));
                let new = Self::Pane(Pane::default());
                let (first, second) = if new_first { (new, old) } else { (old, new) };
                *self = Self::Split {
                    row,
                    ratio: 0.5,
                    first: Box::new(first),
                    second: Box::new(second),
                };
            }
            Self::Split { first, second, .. } => {
                let count = first.leaf_count();
                if index < count {
                    first.split_leaf(index, row, new_first);
                } else {
                    second.split_leaf(index - count, row, new_first);
                }
            }
        }
    }

    /// Tira a folha `index`: a divisão-pai vira a outra parte. Devolve as abas e se a folha era
    /// da primeira parte de sua divisão. `None` se este nó é a folha (não há divisão a desfazer).
    fn close_leaf(&mut self, index: usize) -> Option<(Vec<Session>, bool)> {
        let Self::Split { first, second, .. } = self else {
            return None;
        };
        let count = first.leaf_count();
        let in_first = index < count;
        let (child, index) = if in_first {
            (&mut **first, index)
        } else {
            (&mut **second, index - count)
        };
        let tabs = match child {
            Self::Pane(pane) => Some(std::mem::take(&mut pane.tabs)),
            Self::Split { .. } => None,
        };
        match tabs {
            Some(tabs) => {
                let Self::Split { first, second, .. } =
                    std::mem::replace(self, Self::Pane(Pane::default()))
                else {
                    return None;
                };
                *self = if in_first { *second } else { *first };
                Some((tabs, in_first))
            }
            None => child.close_leaf(index),
        }
    }

    /// Anda a divisão mais próxima de `index` que vai na direção de `dir`. Devolve se achou.
    fn resize(&mut self, index: usize, dir: PaneDirection) -> bool {
        let Self::Split {
            row,
            ratio,
            first,
            second,
        } = self
        else {
            return false;
        };
        let count = first.leaf_count();
        let done = if index < count {
            first.resize(index, dir)
        } else {
            second.resize(index - count, dir)
        };
        if done {
            return true;
        }
        if *row != dir.horizontal() {
            return false;
        }
        let step = if dir.forward() {
            RESIZE_STEP
        } else {
            -RESIZE_STEP
        };
        *ratio = (*ratio + step).clamp(RATIO_MIN, RATIO_MAX);
        true
    }

    /// Os retângulos das folhas dentro de `area` (x, y, largura, altura), na ordem das folhas.
    fn rects(&self, area: [f64; 4], out: &mut Vec<[f64; 4]>) {
        match self {
            Self::Pane(_) => out.push(area),
            Self::Split {
                row,
                ratio,
                first,
                second,
            } => {
                let [x, y, w, h] = area;
                if *row {
                    first.rects([x, y, w * ratio, h], out);
                    second.rects([x + w * ratio, y, w * (1.0 - ratio), h], out);
                } else {
                    first.rects([x, y, w, h * ratio], out);
                    second.rects([x, y + h * ratio, w, h * (1.0 - ratio)], out);
                }
            }
        }
    }

    /// A forma da árvore, com os painéis e as divisões numerados em preordem a partir de
    /// `panes` e `splits`.
    fn shape(&self, panes: &mut usize, splits: &mut usize) -> Shape {
        match self {
            Self::Pane(_) => {
                let index = *panes;
                *panes += 1;
                Shape::Pane(index)
            }
            Self::Split {
                row, first, second, ..
            } => {
                let index = *splits;
                *splits += 1;
                Shape::Split {
                    row: *row,
                    index,
                    first: Box::new(first.shape(panes, splits)),
                    second: Box::new(second.shape(panes, splits)),
                }
            }
        }
    }

    /// As frações das divisões em preordem.
    fn ratios(&self, out: &mut Vec<f64>) {
        if let Self::Split {
            ratio,
            first,
            second,
            ..
        } = self
        {
            out.push(*ratio);
            first.ratios(out);
            second.ratios(out);
        }
    }

    /// Muda a fração da divisão `index` (em preordem, contando a partir de `next`).
    fn set_ratio(&mut self, index: usize, value: f64, next: &mut usize) -> bool {
        let Self::Split {
            ratio,
            first,
            second,
            ..
        } = self
        else {
            return false;
        };
        if *next == index {
            let changed = *ratio != value;
            *ratio = value;
            return changed;
        }
        *next += 1;
        first.set_ratio(index, value, next) || second.set_ratio(index, value, next)
    }

    /// A forma sem as frações: o índice da folha, ou `row[..]` e `col[..]` com as divisões
    /// encadeadas do mesmo sentido achatadas.
    fn write(&self, f: &mut fmt::Formatter<'_>, next: &mut usize) -> fmt::Result {
        match self {
            Self::Pane(_) => {
                write!(f, "{next}")?;
                *next += 1;
                Ok(())
            }
            Self::Split { row, .. } => {
                f.write_str(if *row { "row[" } else { "col[" })?;
                let mut parts = Vec::new();
                self.flatten(*row, &mut parts);
                for (i, part) in parts.into_iter().enumerate() {
                    if i > 0 {
                        f.write_str(",")?;
                    }
                    part.write(f, next)?;
                }
                f.write_str("]")
            }
        }
    }

    /// As partes de uma cadeia de divisões do sentido `row`.
    fn flatten<'a>(&'a self, row: bool, out: &mut Vec<&'a Node>) {
        match self {
            Self::Split {
                row: r,
                first,
                second,
                ..
            } if *r == row => {
                first.flatten(row, out);
                second.flatten(row, out);
            }
            other => out.push(other),
        }
    }
}

/// A forma da árvore de um [`PaneLayout`], sem as frações: o que o `TerminalPanes` compara para
/// saber se precisa montar os widgets de novo.
#[derive(Clone, PartialEq, Debug)]
pub(crate) enum Shape {
    /// Um painel, pelo índice na ordem das folhas.
    Pane(usize),
    /// Uma divisão, numerada em preordem.
    Split {
        row: bool,
        index: usize,
        first: Box<Shape>,
        second: Box<Shape>,
    },
}

/// Uma cadeia de divisões binárias do sentido `row`: com `k` partes, a primeira fica com `1/k`
/// e o resto divide o que sobra.
fn chain(mut parts: Vec<Node>, row: bool) -> Node {
    let first = parts.remove(0);
    if parts.is_empty() {
        return first;
    }
    let ratio = 1.0 / (parts.len() + 1) as f64;
    Node::Split {
        row,
        ratio,
        first: Box::new(first),
        second: Box::new(chain(parts, row)),
    }
}

/// Os painéis de um `TerminalPanes`: uma árvore binária de divisões, as
/// abas de cada painel, o painel ativo e o painel ampliado.
///
/// É dado puro (`Clone + PartialEq + Send + Sync`), por isso vai num `Signal`. Os painéis são
/// índices na ordem das folhas, que numa grade é a ordem de leitura (esquerda para a direita, de
/// cima para baixo). Um índice fora do alcance vale o último painel.
///
/// Mudar a estrutura ([`grid`](Self::grid), [`single`](Self::single), [`split`](Self::split),
/// [`close_pane`](Self::close_pane)) desfaz o zoom. Um painel sem abas
/// fica, e o último painel nunca se fecha.
///
/// ```
/// use tomni_ui_terminal::{Loopback, PaneLayout, Session};
///
/// let abas: Vec<Session> = (0..4).map(|_| Session::new(Loopback::new())).collect();
/// let mut layout = PaneLayout::with(abas);
/// layout.grid(2, 2);
/// assert_eq!(layout.to_string(), "col[row[0,1],row[2,3]]");
/// layout.single();
/// assert_eq!(layout.pane_count(), 1);
/// ```
#[derive(Clone, PartialEq, Debug)]
pub struct PaneLayout {
    root: Node,
    active: usize,
    zoomed: Option<usize>,
    /// Fecha o painel que perdeu a última aba (`close_emptied`).
    close_emptied: bool,
}

impl Default for PaneLayout {
    fn default() -> Self {
        Self::new()
    }
}

impl PaneLayout {
    /// Um painel só, vazio.
    pub fn new() -> Self {
        Self {
            root: Node::Pane(Pane::default()),
            active: 0,
            zoomed: None,
            close_emptied: false,
        }
    }

    /// Um painel só com `sessions` como abas, a primeira à mostra.
    pub fn with(sessions: impl IntoIterator<Item = Session>) -> Self {
        let tabs: Vec<Session> = sessions.into_iter().collect();
        let selected = tabs.first().copied();
        Self {
            root: Node::Pane(Pane { tabs, selected }),
            active: 0,
            zoomed: None,
            close_emptied: false,
        }
    }

    /// Com `true`, o painel que **tinha** abas e perdeu a última (a aba foi fechada, tirada com
    /// [`remove`](Self::remove) ou levada para outro painel) fecha sozinho, a menos que seja o
    /// único. Um painel que nasce vazio, como o de uma [`split`](Self::split), fica. O padrão é
    /// `false`: o painel vazio fica esperando uma aba. A opção não é parte da forma dos painéis:
    /// [`single`](Self::single) e [`grid`](Self::grid) a mantêm, e quem cria um layout novo a
    /// aplica de novo.
    pub fn close_emptied(&mut self, on: bool) {
        self.close_emptied = on;
    }

    /// Se o painel que perde a última aba fecha sozinho (veja [`close_emptied`](Self::close_emptied)).
    pub fn closes_emptied(&self) -> bool {
        self.close_emptied
    }

    fn clamp(&self, pane: usize) -> usize {
        pane.min(self.pane_count() - 1)
    }

    /// A sessão à mostra no painel ativo.
    fn active_session(&self) -> Option<Session> {
        self.pane(self.active).selected
    }

    /// Junta todas as abas, na ordem de [`sessions`](Self::sessions), num painel só, mantendo a
    /// escolhida do painel ativo.
    pub fn single(&mut self) {
        let selected = self.active_session();
        let tabs = self.sessions();
        let selected = selected.or(tabs.first().copied());
        self.root = Node::Pane(Pane { tabs, selected });
        self.active = 0;
        self.zoomed = None;
    }

    /// Refaz os painéis como uma grade de `cols` × `rows`, com as divisões iguais. As abas atuais
    /// são redistribuídas na ordem de leitura, uma por painel; as que sobram vão para o último,
    /// e os painéis que faltam ficam vazios. O painel ativo passa a ser o que tem a sessão que
    /// estava ativa.
    pub fn grid(&mut self, cols: usize, rows: usize) {
        let (cols, rows) = (cols.max(1), rows.max(1));
        if cols * rows == 1 {
            self.single();
            return;
        }
        let active_session = self.active_session();
        let total = cols * rows;
        let mut sessions = self.sessions().into_iter();
        let mut panes: Vec<Pane> = (0..total)
            .map(|_| {
                let session = sessions.next();
                Pane {
                    tabs: session.into_iter().collect(),
                    selected: session,
                }
            })
            .collect();
        if let Some(last) = panes.last_mut() {
            last.tabs.extend(sessions);
        }
        let mut nodes = panes.into_iter().map(Node::Pane);
        let lines = (0..rows)
            .map(|_| chain(nodes.by_ref().take(cols).collect(), true))
            .collect();
        self.root = chain(lines, false);
        self.zoomed = None;
        self.active = active_session
            .and_then(|s| self.pane_of(s))
            .unwrap_or_default();
    }

    /// Divide o painel `pane` ao meio e põe um painel vazio do lado `direction`. Devolve o
    /// índice do painel novo, que passa a ser o ativo.
    pub fn split(&mut self, pane: usize, direction: PaneDirection) -> usize {
        let pane = self.clamp(pane);
        let new_first = !direction.forward();
        self.root
            .split_leaf(pane, direction.horizontal(), new_first);
        self.zoomed = None;
        self.active = if new_first { pane } else { pane + 1 };
        self.active
    }

    /// Põe `session` no fim das abas de `pane` e a escolhe, sem mudar o painel ativo. Se ela já
    /// estava em algum painel, sai de lá.
    pub fn add_tab(&mut self, session: Session, pane: usize) {
        self.move_tab_to(session, pane, usize::MAX);
        // Quem adiciona não toma o foco.
    }

    /// Tira `session` de onde estiver e põe no fim de `pane`, escolhida, e `pane` fica ativo. O
    /// painel de origem escolhe a aba vizinha (a da direita, ou a da esquerda se era a última).
    pub fn move_tab(&mut self, session: Session, pane: usize) {
        let pane = self.move_tab_inner(session, pane, usize::MAX);
        self.set_active(pane);
    }

    /// Como [`move_tab`](Self::move_tab), na posição `index` das abas de `pane` (contada depois
    /// de tirar a aba de onde estava) e sem mudar o painel ativo.
    pub fn move_tab_to(&mut self, session: Session, pane: usize, index: usize) {
        self.move_tab_inner(session, pane, index);
    }

    /// Move a aba e devolve o índice do painel de destino depois da mudança: com
    /// `close_emptied`, o painel de origem pode fechar e deslocar os índices.
    fn move_tab_inner(&mut self, session: Session, pane: usize, index: usize) -> usize {
        let mut pane = self.clamp(pane);
        let origin = self.take_from_pane(session);
        if let Some(target) = self.root.leaf_mut(pane) {
            target.insert(session, index);
        }
        if let Some(origin) = origin
            && origin != pane
            && self.should_close(origin)
        {
            self.close_pane(origin);
            if origin < pane {
                pane -= 1;
            }
        }
        pane
    }

    /// Tira `session` das abas. Devolve se ela estava em algum painel. O painel fica, mesmo
    /// vazio, a menos que [`close_emptied`](Self::close_emptied) esteja ligada e haja outros
    /// painéis.
    pub fn remove(&mut self, session: Session) -> bool {
        let Some(origin) = self.take_from_pane(session) else {
            return false;
        };
        if self.should_close(origin) {
            self.close_pane(origin);
        }
        true
    }

    /// Tira `session` do painel em que está e devolve o índice dele, sem fechar nada.
    fn take_from_pane(&mut self, session: Session) -> Option<usize> {
        let mut panes = Vec::new();
        self.root.panes_mut(&mut panes);
        panes.into_iter().position(|pane| pane.take(session))
    }

    /// Se o painel `pane`, que acabou de perder uma aba, deve fechar.
    fn should_close(&self, pane: usize) -> bool {
        self.close_emptied && self.pane_count() > 1 && self.pane(pane).tabs.is_empty()
    }

    /// Fecha o painel `pane`: a divisão-pai vira o painel irmão, e o vizinho que ocupa o lugar
    /// fica ativo. Devolve as abas que ele tinha, que ficam por conta de quem chamou (fechar ou
    /// mover). No último painel só tira as abas.
    pub fn close_pane(&mut self, pane: usize) -> Vec<Session> {
        let pane = self.clamp(pane);
        self.zoomed = None;
        let Some((tabs, was_first)) = self.root.close_leaf(pane) else {
            // O último painel não se fecha.
            let Node::Pane(only) = &mut self.root else {
                return Vec::new();
            };
            only.selected = None;
            return std::mem::take(&mut only.tabs);
        };
        self.active = match self.active.cmp(&pane) {
            std::cmp::Ordering::Less => self.active,
            std::cmp::Ordering::Greater => self.active - 1,
            // O irmão ocupa o lugar: a primeira folha dele, ou a última se ele vinha antes.
            std::cmp::Ordering::Equal if was_first => pane,
            std::cmp::Ordering::Equal => pane - 1,
        };
        tabs
    }

    /// Alterna o zoom do painel `pane`: ele ocupa a área toda até sair do zoom. O painel
    /// ampliado fica ativo.
    pub fn zoom(&mut self, pane: usize) {
        let pane = self.clamp(pane);
        self.zoomed = if self.zoomed == Some(pane) {
            None
        } else {
            Some(pane)
        };
        self.active = pane;
    }

    /// Faz de `pane` o painel ativo. Com zoom, o zoom vai junto.
    pub fn set_active(&mut self, pane: usize) {
        let pane = self.clamp(pane);
        self.active = pane;
        if self.zoomed.is_some() {
            self.zoomed = Some(pane);
        }
    }

    /// Escolhe a aba `session` no painel dela e faz desse painel o ativo. Não faz nada se a
    /// sessão não está em nenhum painel.
    pub fn select(&mut self, session: Session) {
        let Some(pane) = self.pane_of(session) else {
            return;
        };
        if let Some(target) = self.root.leaf_mut(pane) {
            target.selected = Some(session);
        }
        self.set_active(pane);
    }

    /// Anda a divisão mais próxima de `pane` na direção de `direction` em 5%, entre 10% e 90%
    /// (Left/Right numa linha, Up/Down numa coluna). Devolve `false` se não há divisão desse
    /// sentido.
    pub fn resize(&mut self, pane: usize, direction: PaneDirection) -> bool {
        let pane = self.clamp(pane);
        self.root.resize(pane, direction)
    }

    /// O painel que encosta no lado `direction` de `pane`; entre vários, o de maior contato, e
    /// no empate o de menor índice.
    pub fn neighbor(&self, pane: usize, direction: PaneDirection) -> Option<usize> {
        let pane = self.clamp(pane);
        let rects = self.rects();
        let [x, y, w, h] = rects[pane];
        let overlap = |a0: f64, a1: f64, b0: f64, b1: f64| a1.min(b1) - a0.max(b0);
        let mut best: Option<(usize, f64)> = None;
        for (i, &[ox, oy, ow, oh]) in rects.iter().enumerate() {
            let (touches, contact) = match direction {
                PaneDirection::Right => (
                    (ox - (x + w)).abs() < EPSILON,
                    overlap(y, y + h, oy, oy + oh),
                ),
                PaneDirection::Left => (
                    (ox + ow - x).abs() < EPSILON,
                    overlap(y, y + h, oy, oy + oh),
                ),
                PaneDirection::Down => (
                    (oy - (y + h)).abs() < EPSILON,
                    overlap(x, x + w, ox, ox + ow),
                ),
                PaneDirection::Up => (
                    (oy + oh - y).abs() < EPSILON,
                    overlap(x, x + w, ox, ox + ow),
                ),
            };
            if touches && contact > EPSILON && best.is_none_or(|(_, c)| contact > c + EPSILON) {
                best = Some((i, contact));
            }
        }
        best.map(|(i, _)| i)
    }

    /// O painel ativo.
    pub fn active(&self) -> usize {
        self.active
    }

    /// O painel ampliado, se houver.
    pub fn zoomed(&self) -> Option<usize> {
        self.zoomed
    }

    /// Quantos painéis há (sempre pelo menos um).
    pub fn pane_count(&self) -> usize {
        self.root.leaf_count()
    }

    /// O painel `index`.
    pub fn pane(&self, index: usize) -> &Pane {
        let index = self.clamp(index);
        self.root
            .leaf(index)
            .expect("o índice está dentro dos painéis")
    }

    /// O painel que tem `session` como aba.
    pub fn pane_of(&self, session: Session) -> Option<usize> {
        let mut panes = Vec::new();
        self.root.panes(&mut panes);
        panes.iter().position(|pane| pane.tabs.contains(&session))
    }

    /// Todas as sessões, na ordem dos painéis e, dentro deles, das abas.
    pub fn sessions(&self) -> Vec<Session> {
        let mut panes = Vec::new();
        self.root.panes(&mut panes);
        panes
            .into_iter()
            .flat_map(|pane| pane.tabs.iter().copied())
            .collect()
    }

    /// As sessões à mostra: a aba escolhida de cada painel; com zoom, só a do painel ampliado.
    pub fn visible(&self) -> Vec<Session> {
        let mut panes = Vec::new();
        self.root.panes(&mut panes);
        match self.zoomed {
            Some(zoomed) => panes
                .get(zoomed)
                .and_then(|pane| pane.selected)
                .into_iter()
                .collect(),
            None => panes.into_iter().filter_map(|pane| pane.selected).collect(),
        }
    }

    /// A forma da árvore (sem as frações), para o `TerminalPanes` saber se mudou.
    pub(crate) fn shape(&self) -> Shape {
        self.root.shape(&mut 0, &mut 0)
    }

    /// A fração de cada divisão, em preordem.
    pub(crate) fn ratios(&self) -> Vec<f64> {
        let mut out = Vec::new();
        self.root.ratios(&mut out);
        out
    }

    /// Muda a fração da divisão `index` (em preordem). Devolve se mudou.
    pub(crate) fn set_ratio(&mut self, index: usize, ratio: f64) -> bool {
        self.root.set_ratio(index, ratio, &mut 0)
    }

    /// Onde cada painel fica na área toda: `[x, y, largura, altura]`, de 0 a 1, ignorando o
    /// zoom e a espessura dos divisores.
    pub fn rects(&self) -> Vec<[f64; 4]> {
        let mut out = Vec::new();
        self.root.rects([0.0, 0.0, 1.0, 1.0], &mut out);
        out
    }
}

/// A forma sem as frações: `[0]` para um painel só, senão `row[a,b]` (lado a lado) e `col[a,b]`
/// (empilhados), com as divisões do mesmo sentido encadeadas achatadas (`row[0,1,2]`). É o que
/// o `inspect` mostra.
impl fmt::Display for PaneLayout {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut next = 0;
        match &self.root {
            Node::Pane(_) => f.write_str("[0]"),
            root => root.write(f, &mut next),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::PaneDirection::{Down, Left, Right, Up};
    use super::*;
    use crate::Loopback;

    fn abas(n: usize) -> Vec<Session> {
        (0..n).map(|_| Session::new(Loopback::new())).collect()
    }

    fn descarta(abas: Vec<Session>) {
        for aba in abas {
            aba.dispose();
        }
    }

    /// Os índices das sessões de cada painel, na ordem, em relação a `todas`.
    fn indices(layout: &PaneLayout, todas: &[Session]) -> Vec<Vec<usize>> {
        (0..layout.pane_count())
            .map(|p| {
                layout
                    .pane(p)
                    .tabs
                    .iter()
                    .map(|s| todas.iter().position(|t| t == s).unwrap())
                    .collect()
            })
            .collect()
    }

    fn area_total(layout: &PaneLayout) -> f64 {
        layout.rects().iter().map(|r| r[2] * r[3]).sum()
    }

    #[test]
    fn layout_e_dado_puro() {
        fn confere<T: Clone + PartialEq + std::fmt::Debug + Send + Sync>() {}
        confere::<PaneLayout>();
    }

    #[test]
    fn with_e_single_nao_perdem_sessoes() {
        let a = abas(4);
        let mut layout = PaneLayout::with(a.clone());
        assert_eq!(layout.to_string(), "[0]");
        assert_eq!(layout.pane(0).selected, Some(a[0]));
        assert_eq!(layout.sessions(), a);

        layout.grid(2, 2);
        layout.select(a[2]);
        layout.single();
        assert_eq!(layout.pane_count(), 1);
        assert_eq!(layout.sessions(), a);
        // A escolhida do painel ativo continua à mostra.
        assert_eq!(layout.pane(0).selected, Some(a[2]));
        assert_eq!(layout.active(), 0);
        descarta(a);
    }

    #[test]
    fn grade_2x2_e_3x3_distribuem_em_ordem_de_leitura() {
        let a = abas(9);
        let mut layout = PaneLayout::with(a.clone());
        layout.grid(2, 2);
        assert_eq!(layout.to_string(), "col[row[0,1],row[2,3]]");
        // Uma por painel; as que sobram vão para o último.
        assert_eq!(
            indices(&layout, &a),
            [vec![0], vec![1], vec![2], vec![3, 4, 5, 6, 7, 8]]
        );
        assert_eq!(layout.pane(3).selected, Some(a[3]));

        layout.grid(3, 3);
        assert_eq!(layout.to_string(), "col[row[0,1,2],row[3,4,5],row[6,7,8]]");
        assert_eq!(layout.pane_count(), 9);
        assert_eq!(layout.sessions(), a);
        for (p, aba) in a.iter().enumerate() {
            assert_eq!(layout.pane(p).tabs, [*aba]);
        }
        assert!((area_total(&layout) - 1.0).abs() < 1e-9);
        // Os terços são iguais.
        for r in layout.rects() {
            assert!((r[2] - 1.0 / 3.0).abs() < 1e-9 && (r[3] - 1.0 / 3.0).abs() < 1e-9);
        }

        layout.single();
        assert_eq!(layout.sessions(), a);
        layout.grid(2, 2);
        assert_eq!(layout.sessions().len(), 9);
        descarta(a);
    }

    #[test]
    fn grade_com_poucas_sessoes_deixa_paineis_vazios() {
        let a = abas(2);
        let mut layout = PaneLayout::with(a.clone());
        layout.grid(3, 3);
        assert_eq!(layout.pane_count(), 9);
        assert_eq!(layout.pane(0).tabs, [a[0]]);
        assert_eq!(layout.pane(1).tabs, [a[1]]);
        assert!(layout.pane(2).tabs.is_empty());
        assert_eq!(layout.pane(2).selected, None);
        assert_eq!(layout.visible(), a);
        descarta(a);
    }

    #[test]
    fn grade_mantem_o_painel_da_sessao_ativa() {
        let a = abas(4);
        let mut layout = PaneLayout::with(a.clone());
        layout.grid(2, 2);
        layout.select(a[2]);
        assert_eq!(layout.active(), 2);
        layout.grid(1, 4);
        assert_eq!(layout.to_string(), "col[0,1,2,3]");
        assert_eq!(layout.active(), 2);
        layout.grid(4, 1);
        assert_eq!(layout.to_string(), "row[0,1,2,3]");
        assert_eq!(layout.active(), 2);
        descarta(a);
    }

    #[test]
    fn split_nas_quatro_direcoes() {
        let a = abas(1);
        for (direcao, novo, antigo, forma) in [
            (Right, 1, 0, "row[0,1]"),
            (Left, 0, 1, "row[0,1]"),
            (Down, 1, 0, "col[0,1]"),
            (Up, 0, 1, "col[0,1]"),
        ] {
            let mut layout = PaneLayout::with(a.clone());
            assert_eq!(layout.split(0, direcao), novo, "{direcao:?}");
            assert_eq!(layout.to_string(), forma);
            assert_eq!(layout.active(), novo);
            assert!(layout.pane(novo).tabs.is_empty());
            assert_eq!(layout.pane(antigo).tabs, a);
        }
        // Dividir no meio de uma grade mantém os outros painéis.
        let mut layout = PaneLayout::new();
        layout.grid(2, 1);
        assert_eq!(layout.split(1, Down), 2);
        assert_eq!(layout.to_string(), "row[0,col[1,2]]");
        assert_eq!(layout.split(0, Right), 1);
        assert_eq!(layout.to_string(), "row[0,1,col[2,3]]");
        descarta(a);
    }

    #[test]
    fn mover_abas_e_remover() {
        let a = abas(3);
        let mut layout = PaneLayout::with(a.clone());
        layout.grid(2, 1);
        // [a0] [a1, a2]: a2 vai para o painel 0 no fim.
        assert_eq!(indices(&layout, &a), [vec![0], vec![1, 2]]);
        layout.move_tab(a[1], 0);
        assert_eq!(indices(&layout, &a), [vec![0, 1], vec![2]]);
        assert_eq!(layout.pane(0).selected, Some(a[1]));
        assert_eq!(layout.active(), 0);
        // O painel de origem escolheu a vizinha.
        assert_eq!(layout.pane(1).selected, Some(a[2]));

        // `move_tab_to` conta o índice depois de tirar a aba, e não muda o ativo.
        layout.move_tab_to(a[1], 0, 0);
        assert_eq!(indices(&layout, &a), [vec![1, 0], vec![2]]);
        layout.move_tab_to(a[2], 0, 1);
        assert_eq!(indices(&layout, &a), [vec![1, 2, 0], vec![]]);
        assert_eq!(layout.pane(1).selected, None);
        assert_eq!(layout.active(), 0);
        layout.add_tab(a[0], 1);
        assert_eq!(indices(&layout, &a), [vec![1, 2], vec![0]]);
        assert_eq!(layout.active(), 0);

        // Tirar a escolhida passa para a da direita, ou a da esquerda se era a última.
        layout.select(a[1]);
        assert!(layout.remove(a[1]));
        assert_eq!(layout.pane(0).selected, Some(a[2]));
        assert!(!layout.remove(a[1]));
        assert!(layout.remove(a[2]));
        assert_eq!(layout.pane(0).selected, None);
        assert_eq!(layout.pane_count(), 2);
        descarta(a);
    }

    #[test]
    fn fechar_painel_e_o_ultimo() {
        let a = abas(3);
        let mut layout = PaneLayout::with(a.clone());
        layout.grid(3, 1);
        layout.set_active(1);
        // O irmão ocupa o lugar e fica ativo.
        assert_eq!(layout.close_pane(1), [a[1]]);
        assert_eq!(layout.to_string(), "row[0,1]");
        assert_eq!(indices(&layout, &a), [vec![0], vec![2]]);
        assert_eq!(layout.active(), 1);

        layout.set_active(1);
        assert_eq!(layout.close_pane(1), [a[2]]);
        assert_eq!(layout.to_string(), "[0]");
        assert_eq!(layout.active(), 0);

        // O último painel só esvazia.
        assert_eq!(layout.close_pane(0), [a[0]]);
        assert_eq!(layout.pane_count(), 1);
        assert!(layout.pane(0).tabs.is_empty());
        assert_eq!(layout.pane(0).selected, None);
        descarta(a);
    }

    #[test]
    fn fechar_painel_acerta_o_ativo_dos_outros() {
        let a = abas(4);
        let mut layout = PaneLayout::with(a.clone());
        layout.grid(2, 2);
        layout.set_active(3);
        layout.close_pane(0);
        assert_eq!(layout.pane_count(), 3);
        assert_eq!(layout.active(), 2);
        assert_eq!(layout.pane(2).tabs, [a[3]]);
        layout.set_active(0);
        layout.close_pane(2);
        assert_eq!(layout.active(), 0);
        descarta(a);
    }

    #[test]
    fn zoom_alterna_e_a_estrutura_limpa() {
        let a = abas(4);
        let mut layout = PaneLayout::with(a.clone());
        layout.grid(2, 2);
        layout.zoom(1);
        assert_eq!(layout.zoomed(), Some(1));
        assert_eq!(layout.active(), 1);
        assert_eq!(layout.visible(), [a[1]]);
        // Mudar o ativo leva o zoom junto.
        layout.set_active(2);
        assert_eq!(layout.zoomed(), Some(2));
        layout.zoom(2);
        assert_eq!(layout.zoomed(), None);
        assert_eq!(layout.visible().len(), 4);

        for operacao in [
            (|l: &mut PaneLayout| l.grid(2, 2)) as fn(&mut PaneLayout),
            |l| l.single(),
            |l| {
                l.split(0, Right);
            },
            |l| {
                l.close_pane(0);
            },
        ] {
            layout.zoom(0);
            assert_eq!(layout.zoomed(), Some(0));
            operacao(&mut layout);
            assert_eq!(layout.zoomed(), None);
            layout.grid(2, 2);
        }
        descarta(a);
    }

    #[test]
    fn vizinhos_numa_grade_3x3() {
        let mut layout = PaneLayout::new();
        layout.grid(3, 3);
        for (de, direcao, para) in [
            (4, Left, Some(3)),
            (4, Right, Some(5)),
            (4, Up, Some(1)),
            (4, Down, Some(7)),
            (0, Left, None),
            (0, Up, None),
            (2, Right, None),
            (8, Down, None),
            (8, Left, Some(7)),
            (6, Up, Some(3)),
        ] {
            assert_eq!(layout.neighbor(de, direcao), para, "{de} {direcao:?}");
        }
    }

    #[test]
    fn vizinhos_num_layout_irregular() {
        let mut layout = PaneLayout::new();
        layout.grid(2, 1);
        layout.split(1, Down);
        assert_eq!(layout.to_string(), "row[0,col[1,2]]");
        // O 0 encosta nos dois da direita com o mesmo contato: vence o menor índice.
        assert_eq!(layout.neighbor(0, Right), Some(1));
        assert_eq!(layout.neighbor(1, Left), Some(0));
        assert_eq!(layout.neighbor(2, Left), Some(0));
        assert_eq!(layout.neighbor(1, Down), Some(2));
        assert_eq!(layout.neighbor(2, Up), Some(1));
        assert_eq!(layout.neighbor(1, Up), None);

        // Com a divisão do 2 desigual, vence o de maior contato.
        layout.resize(0, Right);
        layout.resize(0, Right);
        assert_eq!(layout.neighbor(2, Left), Some(0));
        layout.split(0, Down);
        assert_eq!(layout.to_string(), "row[col[0,1],col[2,3]]");
        layout.resize(0, Down);
        layout.resize(0, Down);
        // O 0 desceu até 60%: o 2 (até 50%) toca menos que o 3.
        assert_eq!(layout.neighbor(0, Right), Some(2));
        assert_eq!(layout.neighbor(1, Right), Some(3));
    }

    #[test]
    fn resize_anda_a_divisao_certa_dentro_dos_limites() {
        let mut layout = PaneLayout::new();
        layout.grid(2, 2);
        let largura = |l: &PaneLayout| l.rects()[0][2];
        let altura = |l: &PaneLayout| l.rects()[0][3];
        assert!((largura(&layout) - 0.5).abs() < 1e-9);

        assert!(layout.resize(0, Right));
        assert!((largura(&layout) - 0.55).abs() < 1e-9);
        assert!((altura(&layout) - 0.5).abs() < 1e-9);
        assert!(layout.resize(0, Left));
        assert!((largura(&layout) - 0.5).abs() < 1e-9);
        assert!(layout.resize(0, Down));
        assert!((altura(&layout) - 0.55).abs() < 1e-9);
        // Só a linha de cima andou.
        assert!((layout.rects()[2][2] - 0.5).abs() < 1e-9);

        for _ in 0..30 {
            layout.resize(0, Right);
        }
        assert!((largura(&layout) - 0.9).abs() < 1e-9);
        for _ in 0..30 {
            layout.resize(0, Left);
        }
        assert!((largura(&layout) - 0.1).abs() < 1e-9);

        let mut um = PaneLayout::new();
        assert!(!um.resize(0, Right));
        let mut linha = PaneLayout::new();
        linha.grid(2, 1);
        assert!(!linha.resize(0, Up));
        assert!((area_total(&layout) - 1.0).abs() < 1e-9);
    }

    #[test]
    fn rects_somam_a_area_toda() {
        let mut layout = PaneLayout::new();
        layout.grid(3, 2);
        layout.split(1, Down);
        layout.split(0, Left);
        layout.resize(2, Right);
        assert!((area_total(&layout) - 1.0).abs() < 1e-9);
        assert_eq!(layout.rects().len(), layout.pane_count());
    }

    #[test]
    fn indice_fora_do_alcance_vale_o_ultimo() {
        let a = abas(2);
        let mut layout = PaneLayout::with(a.clone());
        layout.grid(2, 1);
        assert_eq!(layout.pane(9).tabs, [a[1]]);
        layout.set_active(9);
        assert_eq!(layout.active(), 1);
        layout.select(Session::new(Loopback::new()));
        assert_eq!(layout.active(), 1);
        descarta(a);
    }
}
