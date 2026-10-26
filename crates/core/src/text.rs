//! `Text`: texto solto, fixo ou reativo, na rampa tipográfica do Fluent.

use std::fmt;

use crate::color::{ThemeColor, Token};
use crate::prop::{IntoProp, Prop};
use crate::widget::{Common, Look, Widget, common_builders};

/// Degrau da rampa tipográfica do Fluent 2.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum TextRamp {
    /// 12/16, regular: legendas e notas.
    Caption,
    /// 14/20, regular: o texto comum (padrão).
    #[default]
    Body,
    /// 14/20, semibold: destaque dentro do texto comum.
    BodyStrong,
    /// 18/24, regular: texto de introdução, maior que o comum.
    BodyLarge,
    /// 20/28, semibold: título de seção.
    Subtitle,
    /// 28/36, semibold: título da tela.
    Title,
    /// 40/52, semibold: título de destaque, como o de uma página inicial.
    TitleLarge,
    /// 68/92, semibold: o maior degrau, para um número ou uma palavra em evidência.
    Display,
}

impl TextRamp {
    /// Nome em minúsculas (`body_strong`), usado na árvore do `tomni-ui tree`.
    pub fn name(self) -> &'static str {
        match self {
            Self::Caption => "caption",
            Self::Body => "body",
            Self::BodyStrong => "body_strong",
            Self::BodyLarge => "body_large",
            Self::Subtitle => "subtitle",
            Self::Title => "title",
            Self::TitleLarge => "title_large",
            Self::Display => "display",
        }
    }
}

/// Tudo o que o `Drawer` precisa saber do `Text` para medi-lo e pintá-lo.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TextVisual {
    /// Conteúdo inteiro, mesmo quando cortado com reticências.
    pub text: String,
    /// Tamanho e peso.
    pub ramp: TextRamp,
    /// Se quebra linhas na largura disponível.
    pub wrap: bool,
    /// Máximo de linhas: a última termina em reticências quando o texto não cabe.
    pub max_lines: Option<u32>,
    /// Cor. `None` é a herdada (a de um `Card` pressionado ou desabilitado em volta) ou, sem
    /// ela, o `TextFillColorPrimary`.
    pub color: Option<ThemeColor>,
}

/// Texto solto: títulos, rótulos e parágrafos. Não recebe foco nem cliques.
///
/// ```
/// use tomni_ui_core::{Text, TextRamp};
///
/// let titulo = Text::new("Criar conta").title();
/// assert_eq!(titulo.text_ramp(), TextRamp::Title);
/// let ajuda = Text::new("A senha precisa de 8 caracteres ou mais.").caption().wrap();
/// assert!(ajuda.wraps());
/// ```
///
/// Sem [`Text::wrap`], o texto fica numa linha só e o lint acusa `clipped-text` se não couber.
/// Com [`Text::max_lines`], o texto quebra e é cortado com reticências na última linha, como o
/// `TextTrimming="CharacterEllipsis"` do WinUI; aí o lint não acusa nada.
pub struct Text {
    common: Common,
    text: Prop<String>,
    ramp: TextRamp,
    wrap: bool,
    max_lines: Option<u32>,
    color: Option<ThemeColor>,
}

impl Text {
    /// Texto `text` (fixo, closure ou signal) no degrau [`TextRamp::Body`].
    pub fn new<M>(text: impl IntoProp<String, M>) -> Self {
        Self {
            common: Common::default(),
            text: text.into_prop(),
            ramp: TextRamp::Body,
            wrap: false,
            max_lines: None,
            color: None,
        }
    }

    /// Degrau da rampa.
    pub fn ramp(mut self, ramp: TextRamp) -> Self {
        self.ramp = ramp;
        self
    }

    /// [`TextRamp::Caption`]: 12/16.
    pub fn caption(self) -> Self {
        self.ramp(TextRamp::Caption)
    }

    /// [`TextRamp::BodyStrong`]: 14/20 semibold.
    pub fn body_strong(self) -> Self {
        self.ramp(TextRamp::BodyStrong)
    }

    /// [`TextRamp::BodyLarge`]: 18/24.
    pub fn body_large(self) -> Self {
        self.ramp(TextRamp::BodyLarge)
    }

    /// [`TextRamp::Subtitle`]: 20/28 semibold.
    pub fn subtitle(self) -> Self {
        self.ramp(TextRamp::Subtitle)
    }

    /// [`TextRamp::Title`]: 28/36 semibold.
    pub fn title(self) -> Self {
        self.ramp(TextRamp::Title)
    }

    /// [`TextRamp::TitleLarge`]: 40/52 semibold.
    pub fn title_large(self) -> Self {
        self.ramp(TextRamp::TitleLarge)
    }

    /// [`TextRamp::Display`]: 68/92 semibold.
    pub fn display(self) -> Self {
        self.ramp(TextRamp::Display)
    }

    /// Quebra as linhas na largura que o pai der, em vez de ficar numa linha só.
    pub fn wrap(mut self) -> Self {
        self.wrap = true;
        self
    }

    /// Quebra as linhas (como [`Text::wrap`]) e mostra no máximo `lines` delas: a última
    /// termina em reticências quando o texto não cabe. A árvore continua com o texto inteiro.
    pub fn max_lines(mut self, lines: u32) -> Self {
        self.wrap = true;
        self.max_lines = Some(lines.max(1));
        self
    }

    /// Cor do texto: um [`Token`] do tema (acompanha a troca de tema) ou uma
    /// [`Color`](crate::Color) fixa. Sem ela, o texto usa o `TextFillColorPrimary`. Uma cor fixa
    /// precisa de contraste nos dois temas: o lint confere.
    pub fn color(mut self, color: impl Into<ThemeColor>) -> Self {
        self.color = Some(color.into());
        self
    }

    /// Texto secundário (`TextFillColorSecondary`), mais claro: descrições e detalhes.
    pub fn secondary(self) -> Self {
        self.color(Token::TextFillColorSecondary)
    }

    /// Conteúdo atual.
    pub fn content(&self) -> String {
        self.text.get()
    }

    /// Degrau da rampa.
    pub fn text_ramp(&self) -> TextRamp {
        self.ramp
    }

    /// Se quebra linhas.
    pub fn wraps(&self) -> bool {
        self.wrap
    }

    /// Estado para o `Drawer`.
    pub fn visual(&self) -> TextVisual {
        TextVisual {
            text: self.content(),
            ramp: self.ramp,
            wrap: self.wrap,
            max_lines: self.max_lines,
            color: self.color,
        }
    }
}

common_builders!(Text);

impl Widget for Text {
    fn kind(&self) -> &'static str {
        "text"
    }

    fn common(&self) -> &Common {
        &self.common
    }

    fn look(&self) -> Look {
        Look::Text(self.visual())
    }

    fn text(&self) -> Option<String> {
        Some(self.content())
    }
}

impl fmt::Debug for Text {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Text")
            .field("common", &self.common)
            .field("text", &self.text)
            .field("ramp", &self.ramp)
            .field("wrap", &self.wrap)
            .field("max_lines", &self.max_lines)
            .field("color", &self.color)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use tomni_ui_reactive::prelude::*;

    use super::*;

    #[test]
    fn builders_e_look() {
        let t = Text::new("Olá").id("ola").subtitle().wrap();
        assert_eq!(t.common().id.as_deref(), Some("ola"));
        assert_eq!(
            t.look(),
            Look::Text(TextVisual {
                text: "Olá".into(),
                ramp: TextRamp::Subtitle,
                wrap: true,
                max_lines: None,
                color: None,
            })
        );
        assert!(t.look().reflows());
        assert!(!Text::new("x").look().reflows());
        assert!(!t.interactive());
        assert!(!t.focusable());
        assert_eq!(t.text().as_deref(), Some("Olá"));
        assert_eq!(TextRamp::default(), TextRamp::Body);
    }

    #[test]
    fn rampa_cor_e_linhas() {
        assert_eq!(Text::new("x").body_large().text_ramp(), TextRamp::BodyLarge);
        assert_eq!(
            Text::new("x").title_large().text_ramp(),
            TextRamp::TitleLarge
        );
        assert_eq!(Text::new("x").display().text_ramp().name(), "display");
        let t = Text::new("Descrição").secondary().max_lines(2);
        let v = t.visual();
        assert!(v.wrap, "max_lines quebra as linhas");
        assert_eq!(v.max_lines, Some(2));
        assert_eq!(
            v.color,
            Some(ThemeColor::Token(Token::TextFillColorSecondary))
        );
        assert!(t.look().reflows());
        assert_eq!(Text::new("x").max_lines(0).visual().max_lines, Some(1));
    }

    #[test]
    fn texto_reativo() {
        let scope = Scope::root();
        scope.run(|| {
            let n = signal(1);
            let t = Text::new(move || format!("{} itens", n.get()));
            assert_eq!(t.content(), "1 itens");
            n.set(2);
            assert_eq!(t.content(), "2 itens");
        });
        scope.clear();
    }
}
