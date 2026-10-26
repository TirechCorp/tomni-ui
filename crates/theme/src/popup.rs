//! Camadas no visual do WinUI: a superfície dos popups e do `ContentDialog`, o fundo escurecido,
//! os itens do `MenuFlyout` e o `ComboBox` com a lista dele.

use tomni_ui_core::{
    ButtonState, ButtonVariant, ButtonVisual, ComboBoxItemVisual, ComboBoxVisual, Density,
    MenuItemKind, MenuItemVisual, PopupKind, Symbol,
};
use tomni_ui_render::kurbo::{Circle, Point, Rect, RoundedRect, Shape, Size};
use tomni_ui_render::peniko::Color;
use tomni_ui_render::{Canvas, CanvasExt, PATH_TOLERANCE, Paint};
use tomni_ui_text::TextStyle;

use crate::{Drawer, FluentDrawer, TextInfo, Tokens, icons, metrics, ring};

/// `OverlayCornerRadius`: popups e `ContentDialog`.
const OVERLAY_RADIUS: f64 = 8.0;
/// Camadas da sombra, uma por pixel em volta da superfície.
const SHADOW_STEPS: usize = 6;

/// `MenuFlyoutItemMargin`: em volta do fundo do item.
const MENU_ITEM_MARGIN: (f64, f64) = (4.0, 2.0);
/// `MenuFlyoutItemThemePadding` dos lados.
const MENU_ITEM_PADDING: f64 = 11.0;
/// Altura de um item de menu: o alvo de toque do lint, no lugar dos 41 px do WinUI.
const MENU_ITEM_HEIGHT: f64 = 44.0;
/// Altura de um item de menu na densidade compacta, com a margem: a dos itens de lista do
/// `Compact.xaml` (`ListViewItemMinHeight`). O `Compact.xaml` não muda o `MenuFlyoutItem`
/// (41 px); a adaptação está na spec (M19).
const MENU_ITEM_HEIGHT_COMPACT: f64 = 32.0;
/// `MenuFlyoutItemPlaceholderThemeThickness`: a coluna da marca e a do ícone.
const MENU_ITEM_COLUMN: f64 = 28.0;
/// Margem à esquerda do texto do atalho e do chevron do submenu.
const MENU_ITEM_TRAILING_MARGIN: f64 = 24.0;
/// A marca do item de marcar e o chevron do submenu: glifos de 12 px.
const MENU_ITEM_GLYPH: f64 = 12.0;
/// O ícone do item (`IconRoot`).
const MENU_ITEM_ICON: f64 = 16.0;
/// `MenuFlyoutSeparatorThemePadding` em cima e embaixo, mais a linha de 1 px.
const SEPARATOR_HEIGHT: f64 = 3.0;

/// Margem do fundo de um item da lista do `ComboBox` (`LayoutRoot`).
const COMBO_ITEM_MARGIN: (f64, f64) = (5.0, 2.0);
/// `ComboBoxItemCornerRadius`.
const COMBO_ITEM_RADIUS: f64 = 3.0;
/// `ComboBoxItemPillWidth` e `ComboBoxItemPillHeight`.
const PILL: Size = Size::new(3.0, 16.0);
/// `ComboBoxItemPillCornerRadius`.
const PILL_RADIUS: f64 = 1.5;
/// `ComboBoxPadding`: o texto da caixa fechada, à esquerda e em cima.
const COMBO_PADDING: (f64, f64) = (12.0, 5.0);
/// `ComboBoxPadding` do `Compact.xaml` (12,1,0,3), à esquerda e em cima.
const COMBO_PADDING_COMPACT: (f64, f64) = (12.0, 1.0);
/// Coluna do chevron, à direita: o glifo de 12 px com 14 de margem à direita e 12 à esquerda.
const COMBO_GLYPH_COLUMN: f64 = 38.0;
/// Tamanho do chevron.
const COMBO_GLYPH: f64 = 12.0;
/// Margem direita do chevron.
const COMBO_GLYPH_MARGIN: f64 = 14.0;
/// `ComboBoxThemeMinWidth`.
const COMBO_MIN_WIDTH: f64 = 64.0;

/// Fundo opaco de um popup: o `AcrylicInAppFillColorDefault` sem o desfoque (a cor de reserva
/// do acrílico).
pub(crate) fn popup_surface(t: &Tokens) -> Color {
    t.acrylic_in_app_fill_default
}

/// Fundo opaco da parte de cima de um `ContentDialog`: `LayerFillColorAlt` sobre o fundo base.
pub(crate) fn dialog_surface(t: &Tokens) -> Color {
    crate::over(t.layer_fill_alt, t.solid_background_base)
}

/// Sombra aproximada: retângulos translúcidos em volta da superfície, mais fortes embaixo.
pub(crate) fn draw_shadow(canvas: &mut dyn Canvas, rect: Rect, radius: f64, t: &Tokens) {
    let alpha = match t.theme {
        crate::Theme::Light => 0.025,
        crate::Theme::Dark => 0.07,
    };
    for step in (1..=SHADOW_STEPS).rev() {
        let d = step as f64;
        let shadow = Rect::new(
            rect.x0 - d,
            rect.y0 - d / 2.0,
            rect.x1 + d,
            rect.y1 + d * 1.5,
        );
        canvas.fill(
            &RoundedRect::from_rect(shadow, radius + d),
            Color::new([0.0, 0.0, 0.0, alpha]),
        );
    }
}

/// Fundo, borda e sombra de um popup.
pub(crate) fn draw_popup(canvas: &mut dyn Canvas, rect: Rect, kind: PopupKind, t: &Tokens) {
    let radius = match kind {
        PopupKind::ToolTip => metrics::CONTROL_CORNER_RADIUS,
        _ => OVERLAY_RADIUS,
    };
    let border = metrics::BORDER;
    draw_shadow(canvas, rect, radius, t);
    let inner = rect.inset(-border);
    canvas.fill(
        &RoundedRect::from_rect(inner, radius - border),
        popup_surface(t),
    );
    canvas.fill_path(
        &ring(rect, radius, inner, radius - border),
        &Paint::Solid(t.surface_stroke_flyout),
    );
}

/// Fundo da janela atrás de um `ContentDialog`.
pub(crate) fn draw_smoke(canvas: &mut dyn Canvas, rect: Rect, t: &Tokens) {
    canvas.fill(&rect, t.smoke_fill_default);
}

/// Superfície de um `ContentDialog`: em cima o conteúdo, embaixo a faixa dos botões, com
/// `footer` de altura e uma linha entre os dois.
pub(crate) fn draw_dialog(canvas: &mut dyn Canvas, rect: Rect, footer: f64, t: &Tokens) {
    let radius = OVERLAY_RADIUS;
    let border = metrics::BORDER;
    draw_shadow(canvas, rect, radius, t);
    let inner = rect.inset(-border);
    let inner_radius = radius - border;
    canvas.fill(
        &RoundedRect::from_rect(inner, inner_radius),
        t.solid_background_base,
    );
    let split = (inner.y1 - footer).max(inner.y0);
    if footer > 0.0 {
        let top = Rect::new(inner.x0, inner.y0, inner.x1, split);
        canvas.fill(
            &RoundedRect::from_rect(top, (inner_radius, inner_radius, 0.0, 0.0)),
            t.layer_fill_alt,
        );
        // `ContentDialogSeparatorThickness`: a linha de baixo da parte de cima.
        canvas.fill(
            &Rect::new(inner.x0, split - border, inner.x1, split),
            t.card_stroke_default,
        );
    } else {
        canvas.fill(
            &RoundedRect::from_rect(inner, inner_radius),
            t.layer_fill_alt,
        );
    }
    canvas.fill_path(
        &ring(rect, radius, inner, inner_radius),
        &Paint::Solid(t.surface_stroke_default),
    );
}

/// Fundo de um item: transparente em repouso, `SubtleFill` no hover e pressionado.
fn subtle(t: &Tokens, state: ButtonState) -> Color {
    match state {
        ButtonState::Rest | ButtonState::Disabled => Color::TRANSPARENT,
        ButtonState::Hover => t.subtle_fill_secondary,
        ButtonState::Pressed => t.subtle_fill_tertiary,
    }
}

/// Topo de uma linha de texto `BODY` centralizada em `rect`.
fn line_top(rect: Rect) -> f64 {
    rect.y0 + ((rect.height() - f64::from(TextStyle::BODY.line_height)) / 2.0).floor()
}

fn menu_item_background(rect: Rect) -> Rect {
    let (x, y) = MENU_ITEM_MARGIN;
    rect.inset((-x, -y, -x, -y))
}

fn menu_item_text(t: &Tokens, visual: MenuItemVisual) -> Color {
    match visual.state {
        ButtonState::Disabled => t.text_fill_disabled,
        _ => t.text_fill_primary,
    }
}

/// Fundo do item: o do hover também com o submenu aberto
/// (`MenuFlyoutSubItemBackgroundSubMenuOpened`).
fn menu_item_fill(t: &Tokens, visual: MenuItemVisual) -> Color {
    match (visual.kind, visual.state) {
        (MenuItemKind::Submenu { open: true }, ButtonState::Rest) => t.subtle_fill_secondary,
        (_, state) => subtle(t, state),
    }
}

/// Recuo do texto pelas colunas da marca e do ícone.
fn menu_item_indent(visual: MenuItemVisual) -> f64 {
    MENU_ITEM_COLUMN
        * (f64::from(u8::from(visual.check_column)) + f64::from(u8::from(visual.icon_column)))
}

fn menu_item_origin(rect: Rect, visual: MenuItemVisual) -> Point {
    let background = menu_item_background(rect);
    Point::new(
        background.x0 + MENU_ITEM_PADDING + menu_item_indent(visual),
        line_top(rect),
    )
}

/// Largura do que fica à direita do texto: o chevron do submenu ou o texto do atalho, com a
/// margem de 24 px.
fn menu_item_trailing(drawer: &FluentDrawer, visual: MenuItemVisual) -> f64 {
    match (visual.kind, visual.shortcut) {
        (MenuItemKind::Submenu { .. }, _) => MENU_ITEM_TRAILING_MARGIN + MENU_ITEM_GLYPH,
        (_, Some(shortcut)) => {
            let width = drawer
                .fonts
                .borrow_mut()
                .layout(&shortcut.to_string(), TextStyle::CAPTION, None)
                .width();
            MENU_ITEM_TRAILING_MARGIN + f64::from(width)
        }
        _ => 0.0,
    }
}

pub(crate) fn measure_menu_item(
    drawer: &FluentDrawer,
    label: &str,
    visual: MenuItemVisual,
) -> Size {
    let width = drawer
        .fonts
        .borrow_mut()
        .layout(label, TextStyle::BODY, None)
        .width();
    let (x, _) = MENU_ITEM_MARGIN;
    Size::new(
        (f64::from(width)
            + 2.0 * (x + MENU_ITEM_PADDING)
            + menu_item_indent(visual)
            + menu_item_trailing(drawer, visual))
        .ceil(),
        match visual.density {
            Density::Standard => MENU_ITEM_HEIGHT,
            Density::Compact => MENU_ITEM_HEIGHT_COMPACT,
        },
    )
}

pub(crate) fn draw_menu_item(
    drawer: &FluentDrawer,
    canvas: &mut dyn Canvas,
    rect: Rect,
    label: &str,
    visual: MenuItemVisual,
) {
    let t = &drawer.tokens;
    let background = menu_item_background(rect);
    let radius = metrics::CONTROL_CORNER_RADIUS;
    canvas.fill(
        &RoundedRect::from_rect(background, radius),
        menu_item_fill(t, visual),
    );
    let disabled = visual.state == ButtonState::Disabled;
    let center = |size: f64| rect.y0 + ((rect.height() - size) / 2.0).round();
    let mut x = background.x0 + MENU_ITEM_PADDING;
    if visual.check_column {
        if visual.kind == (MenuItemKind::Radio { checked: true }) {
            // O marcador do `RadioMenuFlyoutItem`: um ponto de 6 px no centro da coluna da
            // marca, na mesma cor do visto do `ToggleMenuFlyoutItem`.
            let color = match visual.state {
                ButtonState::Rest => t.text_fill_secondary,
                ButtonState::Hover | ButtonState::Pressed => t.text_fill_primary,
                ButtonState::Disabled => t.text_fill_disabled,
            };
            let half = MENU_ITEM_GLYPH / 2.0;
            canvas.fill(
                &Circle::new(Point::new(x + half, center(MENU_ITEM_GLYPH) + half), 3.0),
                color,
            );
        } else if visual.kind == (MenuItemKind::Toggle { checked: true }) {
            // A marca é `MenuFlyoutSubItemChevron` parada e a cor do texto com o ponteiro.
            let color = match visual.state {
                ButtonState::Rest => t.text_fill_secondary,
                ButtonState::Hover | ButtonState::Pressed => t.text_fill_primary,
                ButtonState::Disabled => t.text_fill_disabled,
            };
            canvas.fill_path(
                &icons::CHECKMARK_12.path(Point::new(x, center(MENU_ITEM_GLYPH)), MENU_ITEM_GLYPH),
                &Paint::Solid(color),
            );
        }
        x += MENU_ITEM_COLUMN;
    }
    if visual.icon_column
        && let Some(icon) = visual.icon
    {
        let y = center(MENU_ITEM_ICON);
        crate::icon::draw_symbol(
            canvas,
            icon,
            Rect::new(x, y, x + MENU_ITEM_ICON, y + MENU_ITEM_ICON),
            menu_item_text(t, visual),
        );
    }
    let text = drawer
        .fonts
        .borrow_mut()
        .layout(label, TextStyle::BODY, None);
    text.draw(
        canvas,
        menu_item_origin(rect, visual),
        &Paint::Solid(menu_item_text(t, visual)),
    );
    let right = background.x1 - MENU_ITEM_PADDING;
    match (visual.kind, visual.shortcut) {
        (MenuItemKind::Submenu { .. }, _) => {
            let color = match visual.state {
                ButtonState::Disabled => t.text_fill_disabled,
                ButtonState::Pressed => t.text_fill_tertiary,
                ButtonState::Rest | ButtonState::Hover => t.text_fill_secondary,
            };
            let y = center(MENU_ITEM_GLYPH);
            crate::icon::draw_symbol(
                canvas,
                Symbol::ChevronRight,
                Rect::new(right - MENU_ITEM_GLYPH, y, right, y + MENU_ITEM_GLYPH),
                color,
            );
        }
        (kind, Some(shortcut)) => {
            let color = if disabled {
                t.text_fill_disabled
            } else {
                t.text_fill_secondary
            };
            let text =
                drawer
                    .fonts
                    .borrow_mut()
                    .layout(&shortcut.to_string(), TextStyle::CAPTION, None);
            // No `MenuFlyoutItem`, o texto tem 4 px de margem em cima e fica 2 px abaixo do
            // centro; no `ToggleMenuFlyoutItem`, não.
            let shift = if kind == MenuItemKind::Action {
                2.0
            } else {
                0.0
            };
            let y = center(f64::from(TextStyle::CAPTION.line_height)) + shift;
            text.draw(
                canvas,
                Point::new(right - f64::from(text.width()), y),
                &Paint::Solid(color),
            );
        }
        _ => {}
    }
    if visual.focus_ring {
        crate::draw_focus_ring(canvas, background, radius, t);
    }
}

pub(crate) fn menu_item_info(
    drawer: &FluentDrawer,
    rect: Rect,
    label: &str,
    visual: MenuItemVisual,
) -> TextInfo {
    let t = &drawer.tokens;
    let background = crate::over(menu_item_fill(t, visual), drawer.backdrop());
    let pill = menu_item_background(rect);
    crate::text::line_info(
        drawer,
        label,
        TextStyle::BODY,
        menu_item_origin(rect, visual),
        (menu_item_text(t, visual), background),
        pill.inset((-MENU_ITEM_PADDING, 0.0, -MENU_ITEM_PADDING, 0.0)),
    )
}

pub(crate) fn measure_separator() -> Size {
    Size::new(0.0, SEPARATOR_HEIGHT)
}

pub(crate) fn draw_separator(canvas: &mut dyn Canvas, rect: Rect, t: &Tokens) {
    let y = rect.y0 + ((rect.height() - 1.0) / 2.0).floor();
    canvas.fill(
        &Rect::new(rect.x0, y, rect.x1, y + 1.0),
        t.divider_stroke_default,
    );
}

/// O `ComboBox` fechado tem as cores e a borda de um botão comum.
fn combo_box_button(visual: &ComboBoxVisual) -> ButtonVisual {
    ButtonVisual {
        variant: ButtonVariant::Standard,
        state: visual.state,
        focus_ring: visual.focus_ring,
        ..ButtonVisual::default()
    }
}

/// O texto da caixa e a cor dele: o valor ou, sem valor, o placeholder.
fn combo_box_text<'a>(t: &Tokens, visual: &'a ComboBoxVisual) -> (&'a str, Color) {
    if visual.text.is_empty() {
        let color = match visual.state {
            ButtonState::Rest | ButtonState::Hover => t.text_fill_secondary,
            ButtonState::Pressed => t.text_fill_tertiary,
            ButtonState::Disabled => t.text_fill_disabled,
        };
        (&visual.placeholder, color)
    } else {
        let color = match visual.state {
            ButtonState::Rest | ButtonState::Hover => t.text_fill_primary,
            ButtonState::Pressed => t.text_fill_secondary,
            ButtonState::Disabled => t.text_fill_disabled,
        };
        (&visual.text, color)
    }
}

fn combo_box_origin(rect: Rect, density: Density) -> Point {
    let (left, top) = match density {
        Density::Standard => COMBO_PADDING,
        Density::Compact => COMBO_PADDING_COMPACT,
    };
    let offset = crate::center_offset_from(rect, crate::control_height(density));
    Point::new(rect.x0 + left, rect.y0 + top + offset)
}

fn combo_box_clip(rect: Rect) -> Rect {
    let (left, _) = COMBO_PADDING;
    let border = metrics::BORDER;
    Rect::new(
        rect.x0 + left,
        rect.y0 + border,
        rect.x1 - COMBO_GLYPH_COLUMN,
        rect.y1 - border,
    )
}

pub(crate) fn measure_combo_box(visual: &ComboBoxVisual) -> Size {
    Size::new(COMBO_MIN_WIDTH, crate::control_height(visual.density))
}

pub(crate) fn draw_combo_box(
    drawer: &mut FluentDrawer,
    canvas: &mut dyn Canvas,
    rect: Rect,
    visual: &ComboBoxVisual,
) {
    // A caixa é um botão sem rótulo; o anel de foco vem junto.
    drawer.draw_button(canvas, rect, "", combo_box_button(visual));
    let t = &drawer.tokens;
    let (text, color) = combo_box_text(t, visual);
    let layout = drawer
        .fonts
        .borrow_mut()
        .layout(text, TextStyle::BODY, None);
    let clip = combo_box_clip(rect);
    canvas.push_clip(&clip.to_path(PATH_TOLERANCE));
    layout.draw(
        canvas,
        combo_box_origin(rect, visual.density),
        &Paint::Solid(color),
    );
    canvas.pop_clip();

    let glyph = if visual.state == ButtonState::Disabled {
        t.text_fill_disabled
    } else {
        t.text_fill_secondary
    };
    let origin = Point::new(
        rect.x1 - COMBO_GLYPH_MARGIN - COMBO_GLYPH,
        rect.y0 + ((rect.height() - COMBO_GLYPH) / 2.0).round(),
    );
    canvas.fill_path(
        &icons::CHEVRON_DOWN_12.path(origin, COMBO_GLYPH),
        &Paint::Solid(glyph),
    );
}

pub(crate) fn combo_box_info(
    drawer: &FluentDrawer,
    rect: Rect,
    visual: &ComboBoxVisual,
) -> Option<TextInfo> {
    let t = &drawer.tokens;
    let (text, color) = combo_box_text(t, visual);
    if text.is_empty() {
        return None;
    }
    let colors = drawer.button_colors(combo_box_button(visual));
    let background = crate::over(colors.fill, drawer.backdrop());
    Some(crate::text::line_info(
        drawer,
        text,
        TextStyle::BODY,
        combo_box_origin(rect, visual.density),
        (color, background),
        combo_box_clip(rect),
    ))
}

fn combo_item_background(rect: Rect) -> Rect {
    let (x, y) = COMBO_ITEM_MARGIN;
    rect.inset((-x, -y, -x, -y))
}

fn combo_item_fill(t: &Tokens, visual: ComboBoxItemVisual) -> Color {
    match (visual.selected, visual.state) {
        (true, ButtonState::Rest | ButtonState::Pressed | ButtonState::Disabled) => {
            t.subtle_fill_secondary
        }
        (true, ButtonState::Hover) => t.subtle_fill_tertiary,
        (false, state) => subtle(t, state),
    }
}

fn combo_item_text(t: &Tokens, visual: ComboBoxItemVisual) -> Color {
    match visual.state {
        ButtonState::Pressed => t.text_fill_secondary,
        ButtonState::Disabled => t.text_fill_disabled,
        _ => t.text_fill_primary,
    }
}

fn combo_item_origin(rect: Rect) -> Point {
    let background = combo_item_background(rect);
    Point::new(background.x0 + MENU_ITEM_PADDING, line_top(rect))
}

pub(crate) fn measure_combo_item(
    drawer: &FluentDrawer,
    label: &str,
    visual: ComboBoxItemVisual,
) -> Size {
    let width = drawer
        .fonts
        .borrow_mut()
        .layout(label, TextStyle::BODY, None)
        .width();
    let (x, _) = COMBO_ITEM_MARGIN;
    Size::new(
        (f64::from(width) + 2.0 * (x + MENU_ITEM_PADDING)).ceil(),
        match visual.density {
            Density::Standard => tomni_ui_core::COMBO_BOX_ITEM_HEIGHT,
            Density::Compact => tomni_ui_core::COMBO_BOX_ITEM_HEIGHT_COMPACT,
        },
    )
}

pub(crate) fn draw_combo_item(
    drawer: &FluentDrawer,
    canvas: &mut dyn Canvas,
    rect: Rect,
    label: &str,
    visual: ComboBoxItemVisual,
) {
    let t = &drawer.tokens;
    let background = combo_item_background(rect);
    canvas.fill(
        &RoundedRect::from_rect(background, COMBO_ITEM_RADIUS),
        combo_item_fill(t, visual),
    );
    if visual.selected {
        let y = background.y0 + ((background.height() - PILL.height) / 2.0).round();
        let pill = Rect::from_origin_size((background.x0, y), PILL);
        canvas.fill(
            &RoundedRect::from_rect(pill, PILL_RADIUS),
            t.accent_fill_default,
        );
    }
    let text = drawer
        .fonts
        .borrow_mut()
        .layout(label, TextStyle::BODY, None);
    text.draw(
        canvas,
        combo_item_origin(rect),
        &Paint::Solid(combo_item_text(t, visual)),
    );
    if visual.focus_ring {
        crate::draw_focus_ring(canvas, background, COMBO_ITEM_RADIUS, t);
    }
}

pub(crate) fn combo_item_info(
    drawer: &FluentDrawer,
    rect: Rect,
    label: &str,
    visual: ComboBoxItemVisual,
) -> TextInfo {
    let t = &drawer.tokens;
    let background = crate::over(combo_item_fill(t, visual), drawer.backdrop());
    let pill = combo_item_background(rect);
    crate::text::line_info(
        drawer,
        label,
        TextStyle::BODY,
        combo_item_origin(rect),
        (combo_item_text(t, visual), background),
        pill.inset((-MENU_ITEM_PADDING, 0.0, -MENU_ITEM_PADDING, 0.0)),
    )
}
