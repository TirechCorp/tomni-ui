//! O cabeçalho do `DataGrid`: o clique ordena, os divisores redimensionam as colunas.

use std::fmt;
use std::rc::Rc;

use tomni_ui_reactive::prelude::*;

use super::model::{DATA_GRID_MIN_COLUMN, SortDirection};
use super::shared::Shared;
use super::visual::{DATA_GRID_CELL_PADDING, DATA_GRID_ICON, DATA_GRID_ICON_GAP};
use super::visual::{DataGridHeaderVisual, header_height};
use crate::cx::EventCx;
use crate::density::Density;
use crate::event::{Cursor, Event, FocusSource, Key};
use crate::widget::{Common, Look, Widget};

/// Quantas linhas, no máximo, o clique duplo no divisor olha para achar a largura certa.
const FIT_SAMPLE: usize = 300;
/// Espaço do glifo de ordenação no cabeçalho de uma coluna ordenável.
const SORT_GLYPH: f64 = 12.0 + 8.0;

/// Uma redimensão em andamento.
#[derive(Clone, Copy)]
struct Resize {
    column: usize,
    start_x: f64,
    start_width: f64,
}

pub(crate) struct GridHeader {
    common: Common,
    shared: Rc<Shared>,
    density: Density,
    hovered: Option<usize>,
    pressed: Option<usize>,
    divider_hover: Option<usize>,
    resize: Option<Resize>,
    focus: Option<FocusSource>,
    /// A coluna do teclado.
    column: usize,
}

impl GridHeader {
    pub(crate) fn new(shared: &Rc<Shared>) -> Self {
        let mut common = Common::generated(shared.sub("cabecalho"));
        common.height = Some(header_height(Density::Standard));
        Self {
            common,
            shared: Rc::clone(shared),
            density: Density::Standard,
            hovered: None,
            pressed: None,
            divider_hover: None,
            resize: None,
            focus: None,
            column: 0,
        }
    }

    fn visual(&self) -> DataGridHeaderVisual {
        let shared = &self.shared;
        let columns = shared.layout.get();
        let sort = super::shared::sort_index(&columns, &shared.sort.get().get());
        DataGridHeaderVisual {
            sort,
            hovered: self.hovered,
            pressed: self.pressed,
            resizing: self.resize.map(|r| r.column),
            divider_hover: self.divider_hover,
            focus_ring: (self.focus == Some(FocusSource::Keyboard))
                .then(|| self.column.min(columns.len().saturating_sub(1))),
            columns,
            density: self.density,
        }
    }

    fn visual_now(&self) -> DataGridHeaderVisual {
        untrack(|| self.visual())
    }

    /// Põe a largura de uma coluna, em px.
    fn set_width(&self, column: usize, width: f64) {
        let width = width.max(DATA_GRID_MIN_COLUMN).round();
        self.shared.widths.update(|widths| {
            if let Some(slot) = widths.get_mut(column) {
                *slot = Some(width);
            }
        });
    }

    /// Ajusta a coluna ao texto mais largo (do cabeçalho e das primeiras linhas).
    fn fit(&self, column: usize, cx: &mut EventCx) {
        let shared = &self.shared;
        let columns = shared.layout.get_untracked();
        let Some(def) = columns.get(column) else {
            return;
        };
        let mut widest = {
            let carets = cx.metrics().carets(&def.header);
            let text = carets.last().map_or(0.0, |&(_, x)| x);
            text + if def.sortable { SORT_GLYPH } else { 0.0 }
        };
        let flat = shared.flat_now();
        for row in flat.rows.iter().take(FIT_SAMPLE) {
            let Some((text, _)) = row.cells.get(column) else {
                continue;
            };
            let carets = cx.metrics().carets(text);
            let mut width = carets.last().map_or(0.0, |&(_, x)| x);
            if column == 0 && row.icon.is_some() {
                width += DATA_GRID_ICON + DATA_GRID_ICON_GAP;
            }
            widest = widest.max(width);
        }
        self.set_width(column, widest + 2.0 * DATA_GRID_CELL_PADDING);
    }

    fn pointer_move(&mut self, cx: &EventCx, inside: bool) {
        let x = cx.pointer().x;
        if let Some(resize) = self.resize {
            self.set_width(resize.column, resize.start_width + (x - resize.start_x));
            return;
        }
        if !inside {
            self.hovered = None;
            self.divider_hover = None;
            return;
        }
        let visual = self.visual_now();
        let rect = cx.rect();
        self.divider_hover = visual.divider_at(rect, x);
        self.hovered = visual.column_at(rect, x);
    }

    fn pointer_down(&mut self, cx: &mut EventCx, clicks: u8) {
        let x = cx.pointer().x;
        let visual = self.visual_now();
        let rect = cx.rect();
        if let Some(column) = visual.divider_at(rect, x) {
            if clicks >= 2 {
                self.fit(column, cx);
                return;
            }
            let start_width = visual.cells(rect)[column].width();
            self.resize = Some(Resize {
                column,
                start_x: x,
                start_width,
            });
            self.divider_hover = Some(column);
            self.column = column;
            return;
        }
        self.pressed = visual.column_at(rect, x);
        if let Some(column) = self.pressed {
            self.column = column;
        }
    }

    fn pointer_up(&mut self, cx: &EventCx, inside: bool) {
        if self.resize.take().is_some() {
            self.pointer_move(cx, inside);
            return;
        }
        let Some(pressed) = self.pressed.take() else {
            return;
        };
        let visual = self.visual_now();
        if inside && visual.column_at(cx.rect(), cx.pointer().x) == Some(pressed) {
            self.shared.sort_by(pressed);
        }
    }

    fn key_down(&mut self, key: Key) -> bool {
        let n = self.shared.layout.get_untracked().len();
        if n == 0 {
            return false;
        }
        self.column = self.column.min(n - 1);
        match key {
            Key::Left => self.column = self.column.saturating_sub(1),
            Key::Right => self.column = (self.column + 1).min(n - 1),
            Key::Home => self.column = 0,
            Key::End => self.column = n - 1,
            Key::Enter | Key::Space => self.shared.sort_by(self.column),
            _ => return false,
        }
        true
    }
}

impl Widget for GridHeader {
    fn kind(&self) -> &'static str {
        "data_grid_header"
    }

    fn common(&self) -> &Common {
        &self.common
    }

    fn look(&self) -> Look {
        Look::DataGridHeader(self.visual())
    }

    fn set_density(&mut self, density: Density) {
        self.density = density;
        self.common.height = Some(header_height(density));
    }

    fn handle(&mut self, event: &Event, cx: &mut EventCx) -> bool {
        match *event {
            Event::PointerMove { inside, .. } => self.pointer_move(cx, inside),
            Event::PointerDown { clicks, .. } => self.pointer_down(cx, clicks),
            Event::PointerUp { inside, .. } => self.pointer_up(cx, inside),
            Event::PointerCancel => {
                self.pressed = None;
                self.resize = None;
                self.hovered = None;
                self.divider_hover = None;
            }
            Event::FocusIn(source) => self.focus = Some(source),
            Event::FocusOut => self.focus = None,
            Event::KeyDown(key) if self.focus.is_some() => return self.key_down(key),
            // O alvo do arrasto é o `DataGrid`: o cabeçalho só deixa passar.
            Event::DragOver(_) | Event::DragLeave | Event::Drop(_) | Event::FilesDropped { .. } => {
                return false;
            }
            _ => return false,
        }
        true
    }

    fn interactive(&self) -> bool {
        true
    }

    fn focusable(&self) -> bool {
        true
    }

    fn cursor(&self) -> Option<Cursor> {
        (self.divider_hover.is_some() || self.resize.is_some()).then_some(Cursor::EwResize)
    }

    fn text(&self) -> Option<String> {
        let columns = untrack(|| self.shared.layout.get_untracked());
        let sort = untrack(|| self.shared.sort_now());
        let titles: Vec<String> = columns
            .iter()
            .enumerate()
            .map(|(i, column)| match sort {
                Some((index, direction)) if index == i => {
                    let way = match direction {
                        SortDirection::Ascending => "crescente",
                        SortDirection::Descending => "decrescente",
                    };
                    format!("{} ({way})", column.header)
                }
                _ => column.header.clone(),
            })
            .collect();
        Some(titles.join(", "))
    }
}

impl fmt::Debug for GridHeader {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GridHeader")
            .field("common", &self.common)
            .finish_non_exhaustive()
    }
}
