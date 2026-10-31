<!-- tomni-ui:begin (gerado por `tomni-ui init`; rode de novo para atualizar) -->
# Tomni UI: guia para agentes

Este app usa o **Tomni UI**, uma lib de UI em Rust com visual Fluent (Windows 11). As telas
são funções que montam widgets; o estado é reativo (signals); a tela de verdade pode ser
vista, inspecionada, testada e verificada sem abrir janela. Siga este guia ao criar ou mudar
telas.

## Fluxo de trabalho

1. Escreva a tela como uma função `fn nome() -> impl View` e registre-a no `App`.
2. Escreva um teste com `TestApp` que usa a tela pelos ids (clicar, digitar, conferir).
3. Rode `cargo test`.
4. Rode `tomni-ui lint`: zero erros. Corrija o que ele apontar (a mensagem diz como).
5. Olhe a tela: `tomni-ui screenshot --view nome` (ou `ui_screenshot` no MCP) e abra o PNG.
6. Para conferir uma interação na tela de verdade, use o `ui_interact` do MCP.

A tarefa só está pronta com os testes passando e o `tomni-ui lint` sem erros.

Se o comando `tomni-ui` (ou o `tomni-ui-mcp`) não existir, instale a partir do código do
Tomni UI: `cargo install --path CAMINHO/crates/cli` e `cargo install --path
CAMINHO/crates/mcp`. Os crates ainda não estão no crates.io.

## Uma tela

```rust
use tomni_ui::prelude::*;

fn login() -> impl View {
    let email = signal(String::new());
    let senha = signal(String::new());
    let entrou = signal(false);
    Form::new("Login").child(
        Column::new()
            .gap(12)
            .padding(24)
            .child(
                Edit::bind(email)
                    .id("email")
                    .placeholder("E-mail")
                    .height(44),
            )
            .child(
                Edit::bind(senha)
                    .id("senha")
                    .placeholder("Senha")
                    .password()
                    .height(44),
            )
            .child(
                Button::new(move || {
                    String::from(if entrou.get() { "Bem-vindo!" } else { "Entrar" })
                })
                .id("entrar")
                .accent()
                .height(44)
                .disabled(move || !email.get().contains('@') || senha.get().chars().count() < 6)
                .on_click(move || entrou.set(true)),
            ),
    )
}

fn main() -> Result<(), tomni_ui::platform::Error> {
    tomni_ui::App::new(login).size(390, 844).run()
}
```

- `App::new(tela)` recebe a **função** (não a chamada): ela pode rodar mais de uma vez.
- `.view(outra)` registra outras telas para as ferramentas (`--view outra`); o nome é o da
  função. `.size(l, a)` é o tamanho da janela que se quer: se não cabe no monitor (a área de uso é o
  monitor sem 48 px de barra de tarefas e sem 4% de margem de cada lado), ela abre menor (no mínimo
  640×400) e centralizada, e vale para toda janela, principal ou secundária; `.theme(Theme::Dark)` fixa o tema.
- `tomni_ui::set_theme(Some(Theme::Dark))` troca o tema **com o app rodando** (de um
  `on_click`, por exemplo): vale para todas as janelas, na hora, e as janelas abertas depois
  nascem com ele. `set_theme(None)` solta o tema fixado e volta ao do sistema (que a lib já
  acompanha). No teste, `app.set_theme(Some(Theme::Dark))` e `app.current_theme()`.
- `.backdrop(Backdrop::Mica)`: o fundo Mica do Windows 11 (a cor da área de trabalho, bem
  suave, atrás da janela). Fora do Windows 11, e no lint, nos testes e nas screenshots, fica a
  cor sólida do tema.
- O `main` precisa terminar em `App::...run()`: é ali que as ferramentas entram.

## Densidade compacta (desktop)

O padrão (`Density::Standard`) é para toque: o lint pede alvos de 44 × 44 px. Para um app de
desktop, de mouse e teclado, com muita coisa na tela (gerenciador de conexões, editor,
terminal), use `.density(Density::Compact)` no `Form` ou num contêiner (`Column`, `Row`,
`Card`, `UniformGrid`, `ScrollViewer`, `TabView`): vale para tudo o que está dentro, e um filho
volta ao padrão com `.density(Density::Standard)`. No compacto:

- `Edit`, senha, `NumberBox`, `AutoSuggestBox` e `ComboBox` têm 24 px; `Button`, `CheckBox`,
  `ToggleSwitch` e `Slider` ficam com as medidas do WinUI (32 px no botão); o `icon_only` tem
  32 × 24; as abas do `TabView` têm 24, com o título de 12 px.
- **Não** ponha `.height(44)`: o lint pede só 24 × 24 px (o nível AA da WCAG 2.2).
- No Android (e em qualquer tela de toque), fique no padrão.

```rust
Form::new("Conexões").density(Density::Compact).child(
    Column::new().padding(16).gap(8)
        .child(Edit::bind(host).id("host"))
        .child(Row::new().gap(8).child(Button::new("Conectar").id("conectar").accent())),
)
```

## Widgets

| Widget | Para quê | Builders |
|---|---|---|
| `Form::new(titulo)` | a tela; pinta o fundo e dá ao filho a janela (o título vai para a barra da janela, não aparece na tela) | `.child(view)` (um filho) |
| `Column::new()` | filhos de cima para baixo | `.gap(n)`, `.padding(n ou Insets)`, `.align(Align)`, `.justify(Justify)`, `.child(view)` |
| `Row::new()` | filhos da esquerda para a direita | os mesmos da `Column` e `.wrap()` (os filhos que não cabem descem para a linha de baixo, como etiquetas ou filtros) |
| `UniformGrid::new()` | grade de colunas iguais: tantas colunas quantas couberem, todos os filhos com a mesma largura e a mesma altura | `.min_column_width(n)` (padrão 200), `.gap(n)`, `.padding(n)`, `.child(view)` |
| `Card::new()` | cartão da Gallery: fundo de card, borda de 1 px, raio 8 e padding 16 | `.child(view)` (um filho), `.padding(n)`, `.on_click(move \|\| ...)` (fica clicável), `.label(prop)` (o nome do card clicável), `.disabled(prop)` |
| `SettingsCard::new(titulo)` | a linha de uma configuração, como nas Configurações do Windows 11: ícone, título, descrição e um controle à direita; mínimo de 68 px | `.description(prop)`, `.icon(Symbol)`, `.content(view)` (o controle: `ToggleSwitch`, `ComboBox`, `Button`...), `.on_click(move \|\| ...)` (a linha toda vira botão, com chevron), `.action_icon(Symbol)` (troca o chevron, por exemplo `Open`), `.disabled(prop)` |
| `SettingsExpander::new(titulo, signal_bool)` | um `SettingsCard` que abre uma lista de filhos | `.description(prop)`, `.icon(Symbol)`, `.content(view)` (controle do cabeçalho), `.item(SettingsCard)`, `.item_view(view)` |
| `Border::new()` | caixa com fundo, borda e cantos arredondados, sem nada pronto | `.background(cor)`, `.border(cor, espessura)`, `.corner_radius(n)`, `.padding(n)`, `.child(view)` (um filho) |
| `Text::new(texto)` | texto solto (títulos, parágrafos, resumos) | `.caption()`, `.body_strong()`, `.body_large()`, `.subtitle()`, `.title()`, `.title_large()`, `.display()`, `.wrap()`, `.max_lines(n)`, `.secondary()`, `.color(cor)` |
| `Button::new(rotulo)` | botão | `.icon(Symbol::Save)` (ícone antes do rótulo), `.accent()`, `.hyperlink()` (só o texto, na cor de destaque, para ações leves como "Esqueci a senha"), `.disabled(prop)`, `.on_click(move \|\| ...)` |
| `Button::icon_only(Symbol::Settings)` | botão só com o ícone, um quadrado de 44 px (32 × 24 no compacto; barras de ferramentas) | `.label(prop)` (obrigatório: o nome, que vira a dica), e os do `Button` |
| `ToggleButton::bind(signal, rotulo)` | botão que fica ligado ou desligado (negrito, "Fixar", um filtro), ligado a um `Signal<bool>`; ligado, tem a cor de destaque | `.icon(Symbol)`, `.disabled(prop)`; `ToggleButton::icon_only(signal, Symbol::Star)` com `.label(prop)` |
| `Icon::new(Symbol::Home)` | ícone Fluent solto (lista em "Ícones e imagens") | `.size(12, 16, 20 ou 24)` (padrão 20), `.color(cor)` (padrão: a do texto), `.decorative()` |
| `Image::bytes(include_bytes!("logo.png"))` / `Image::file(caminho)` | imagem PNG, JPEG, GIF (o primeiro quadro), WebP ou SVG | `.alt(texto)` ou `.decorative()` (um dos dois é obrigatório), `.width(n)`, `.height(n)`, `.stretch(Stretch::UniformToFill)`, `.corner_radius(n)`, `.natural_size(largura, altura)` (veja "Ícones e imagens") |
| `Edit::bind(signal)` | campo de texto ligado a um `Signal<String>` | `.placeholder(prop)`, `.password()`, `.password_reveal(prop)` (o olho da senha; veja "Senha"), `.disabled(prop)`, `.multiline()` (várias linhas: Enter quebra, Cima e Baixo andam por linha, rola com a roda; sem quebra automática; dê `.height(..)`), `.read_only()` (seleciona e copia, não muda) |
| `TextEditor::bind(signal)` | editor de texto de várias linhas, com quebra automática, desfazer, busca e realce; é o campo de escrever uma nota (veja "Editor de texto e Markdown") | `.placeholder(prop)`, `.wrap(bool)`, `.monospace()`, `.read_only(prop)`, `.disabled(prop)`, `.min_height(n)`, `.max_height(n)`, `.highlight(\|texto\| Vec<StyleRange>)`, `.marks(Signal<Vec<Mark>>)`, `.controller(TextEditorController)`, `.on_key(\|tecla, controle\| bool)`, `.on_copy(\|texto\| ..)`, `.on_change(\|\| ..)`, `.on_paste_files(..)`, `.on_paste_image(..)`, `.on_files_dropped(..)` |
| `RichText::new(spans)` | texto corrido com trechos em negrito, itálico, riscado, código e links (`spans` fixo, closure ou signal de `Vec<Span>`) | `Span::new(texto)` com `.bold()`, `.italic()`, `.strike()`, `.code()`, `.link(url)`; `.ramp(..)`, `.caption()`, `.wrap()`, `.max_lines(n)`, `.secondary()`, `.color(..)`, `.monospace()`, `.on_link(\|url\| ..)`, `.link_ids(prefixo, primeiro)` |
| `MarkdownView::new(texto)` | o Markdown desenhado com widgets (feature `markdown`): títulos, listas, tarefas, citações, tabelas, blocos de código, links e imagens | `.on_link`, `.on_task(\|byte, marcada\| ..)`, `.on_copy`, `.fence(lang, ..)`, `.image(..)`, `.on_files_dropped(..)`, `.id`, `.grow`, `.width`, `.max_width`, `.visible` |
| `NumberBox::bind(signal)` | campo de número ligado a um `Signal<f64>` (`::bind_optional` para `Signal<Option<f64>>`, vazio vira `None`) | `.min(n)`, `.max(n)`, `.precision(casas)` (sem ele, não arredonda), `.small_change(n)` (padrão 1), `.large_change(n)` (padrão 10), `.spin_buttons(SpinButtons::Compact)`, `.clear_button(false)`, `.placeholder(prop)`, `.disabled(prop)` |
| `CheckBox::bind(signal, rotulo)` | caixa de seleção ligada a um `Signal<bool>` | `.disabled(prop)` |
| `ToggleSwitch::bind(signal)` | chave liga/desliga ligada a um `Signal<bool>`, com "Ligado"/"Desligado" ao lado | `.header(prop)` (rótulo em cima), `.disabled(prop)` |
| `InfoBar::new(InfoBarSeverity::Error)` | barra de aviso: `Informational`, `Success`, `Warning` ou `Error` | `.title(prop)`, `.message(prop)`, `.on_close(move \|\| ...)` (mostra o X), `.action("Rótulo", move \|\| ...)` (botão de ação depois da mensagem, id `<id>-acao`) |
| `ScrollViewer::new()` | área rolável na vertical (`::horizontal()`, `::both()`) | `.child(view)` (um filho), `.grow(1)` para ocupar o resto da coluna, `.link_with_bars(ScrollLink)` (o app comanda a rolagem, como seguir o fim de uma conversa, e as barras continuam) |
| `For::each(itens, chave, view)` | uma view por item de uma lista (`Signal<Vec<T>>`, closure ou valor) | veja "Listas e partes que mudam" |
| `Show::when(cond, view)` | uma view enquanto `cond` vale | `.otherwise(view)` (a outra) |
| `ComboBox::bind(signal, opcoes)` | escolher um valor numa lista, ligado a um `Signal<String>` (vazio: nada escolhido) | `.placeholder(prop)`, `.disabled(prop)` |
| `RadioButtons::bind(signal, opcoes)` | escolher uma entre poucas opções, todas à vista, ligado a um `Signal<String>` | `.header(prop)`, `.disabled(prop)` |
| `Slider::bind(signal, 0..=100)` | um número numa faixa, ligado a um `Signal<f64>` | `.step(n)` (padrão 1), `.header(prop)`, `.disabled(prop)` |
| `ProgressBar::new()` / `ProgressRing::new()` | andamento de uma tarefa; sem `.value`, indeterminado (animação contínua) | `.value(prop)` de 0 a 1; `.paused(prop)` e `.error(prop)` pintam com a cor do estado |
| `AreaChart::new(serie)` / `Sparkline::new(serie)` | gráfico de área de um uso no tempo (CPU, memória, rede), como no Gerenciador de Tarefas; a `Sparkline` é a miniatura, sem grade | `.max(n)` (padrão 100), `.capacity(n)` (pontos à vista, padrão 60), `.color(Token::..)`, `.secondary(serie)` e `.secondary_color(..)` (uma segunda série por cima, na mesma escala: o tempo de kernel) |
| `LevelMeter::new(nivel)` | medidor de quadradinhos, do amarelo-claro ao vermelho-escuro, para um nível de 0 a 1 (a CPU na barra de título) | `.segments(n)` (padrão 10), `.on_click(move \|\| ...)` (vira botão), `.tooltip(prop)` |
| `HeatMap::new(valores)` | mapa de calor: um ladrilho por valor, mais cheio de cor quanto maior (o uso de cada núcleo quando são dezenas ou centenas) | `.max(n)` (padrão 100), `.color(Token::..)`, `.cell_tooltip(\|n, valor\| ...)` (a dica do ladrilho sob o ponteiro) |
| `Expander::new(cabecalho, signal_bool)` | seção recolhível (sanfona): o cabeçalho abre e fecha o conteúdo | `.content(view)`, `.description(texto)` |
| `TabView::bind(signal)` | abas, ligadas a um `Signal<String>` com a chave da aba escolhida | `.tab(chave, titulo, view)` (uma por aba), `.items(lista, \|t\| chave, \|t\| titulo, \|t\| view)` (uma por item), `.on_close(move \|chave\| ...)` (o X), `.on_add(move \|\| ...)` (o "+"), `.on_reorder(move \|de, para\| ...)` (arrastar a aba), `.tab_context_flyout(\|chave\| MenuFlyout)` (menu de contexto da aba), `.tab_width(TabWidth::SizeToContent)` |
| `NavigationView::bind(signal)` | painel de navegação à esquerda e a página do item escolhido ao lado; o signal guarda a chave do item | `.item(chave, Symbol::Home, rotulo, view)`, `.group(chave, icone, rotulo, \|g\| g.item(...))`, `.header(texto)`, `.footer_item(...)`, `.settings(view)`, `.search(AutoSuggestBox)`, `.pane_mode(NavigationMode::Compact)` |
| `TitleBar::new(titulo)` | a barra de título desenhada pelo app, no lugar da do sistema (vai no `Form::title_bar`) | `.subtitle(prop)` (vazio, some sem deixar espaço), `.icon(Symbol::Library)`, `.back(prop)` e `.on_back(move \|\| ...)` (o voltar), `.back_enabled(prop)`, `.content(view)` (no meio: a busca), `.right_header(view)` |
| `AutoSuggestBox::bind(signal, sugerir)` | campo de texto com sugestões numa lista embaixo (autocompletar) | `.placeholder(prop)`, `.query_icon(Symbol::Search)` (lupa no fim, que chama o `on_submit`), `.disabled(prop)`, `.on_submit(move \|valor\| ...)` |
| `MenuFlyout::new()` | menu de ações aberto por um botão (`Button::flyout`), por um item da `MenuBar` ou como menu de contexto (`.context_flyout`) | `.item(id, rotulo, move \|\| ...)`, `.toggle(id, rotulo, signal_bool)` (item com a marca ✓), `.radio(id, rotulo, signal, valor)` (item de rádio: marcado quando o signal vale `valor`; escolher grava e fecha), `.submenu(id, rotulo, MenuFlyout)`, `.separator()`, `.open(signal)`; depois de um item, `.icon(Symbol::Save)`, `.shortcut(Shortcut::ctrl('S'))` (o atalho, que aparece à direita e vale com o menu fechado), `.item_visible(cond)` (mostra o item só enquanto `cond` vale: reserve itens a mais para listas que mudam, como as janelas abertas) `.item_enabled(cond)` (o item fica esmaecido enquanto `cond` não vale: não responde ao clique, ao atalho nem ao Enter e o teclado passa por cima dele; a `cond` pode ser um signal) e `.access_key('1')` (a tecla de acesso: com o menu aberto, a tecla aciona o item; não aparece sublinhada); no menu, `.highlighted(signal_opcao_string)` grava o id do item em realce (para o app mostrar para onde a ação vai levar) |
| `MenuBar::new()` | barra de menus no alto da janela de desktop (Arquivo, Editar, Ajuda) | `.menu(titulo, MenuFlyout)` (um por item), `.access_key('A')` (o Alt+A abre o último menu) |
| `Split::row(a, b)` / `Split::column(a, b)` | dois painéis, lado a lado ou um em cima do outro, com um divisor que o usuário arrasta | `.size(signal_f64)` (o tamanho do primeiro painel; padrão 240), `.min_first(px)`, `.min_second(px)` |
| `Flyout::new(view)` | popup leve com qualquer conteúdo, aberto por um botão (`Button::flyout`) | `.open(signal)` (para fechar por um botão de dentro) |
| `ContentDialog::new(titulo, signal_bool)` | caixa modal que pede uma decisão; aberta enquanto o signal vale `true` | `.content(view)`, `.primary(rotulo, move \|\| ...)`, `.secondary(rotulo, move \|\| ...)`, `.close(rotulo)` |

Todos aceitam `.id("...")`, `.width(n)`, `.height(n)`, `.max_width(n)`, `.grow(n)`, `.visible(prop)`,
`.tooltip(prop)` (a dica; veja "Popups"), `.accelerator(Shortcut, move || ...)` (um atalho de
teclado) e `.context_flyout(MenuFlyout)` (o menu do botão direito); veja "Desktop: divisor,
menus e atalhos". O `.grow(n)`
funciona como o `flex: n` do CSS: os irmãos com `grow` dividem o eixo do contêiner na proporção de
`n`. Dois botões com `.grow(1)` numa `Row` ficam com a mesma largura, qualquer que seja o texto, e
nenhum fica menor que o próprio conteúdo. O `.max_width(n)` limita a largura: para uma coluna
de leitura centrada, ponha a coluna com `.grow(1000).max_width(1200)` numa `Row` entre dois
espaçadores `Column::new().grow(1)`.

- **`Text`**: sem nada, é o corpo (14 px). A rampa: `.caption()` (12 px), `.body_strong()`
  (14 px semibold), `.body_large()` (18 px), `.subtitle()` (20 px), `.title()` (28 px),
  `.title_large()` (40 px) e `.display()` (68 px). Sem `.wrap()`, fica numa linha (e o lint acusa
  se não couber); com `.wrap()`, quebra na largura da coluna; com `.max_lines(2)`, quebra e para
  na segunda linha com reticências (o lint não acusa: é de propósito; `tree()` marca `trimmed`).
  `.secondary()` é a cor de texto secundária, para descrições.
- **Cores** (`Text::color`, `Border::background`, `Border::border`): um `Token` do tema
  (`Token::LayerFillColorDefault`, `Token::CardStrokeColorDefault`, `Token::AccentTextFillColorPrimary`...),
  que acompanha o tema claro e escuro, ou uma `Color` fixa (`Color::hex(0x003E92)`,
  `Color::WHITE`), que é a mesma nos dois. Prefira os tokens; com cor fixa, o lint confere o
  contraste nos dois temas. O fundo do `Border` aceita um gradiente:
  `Brush::linear((0.0, 0.0), (1.0, 1.0), [(0.0, Color::hex(0x003E92)), (1.0, Color::hex(0x6B2FA0))])`
  (pontas de `(0, 0)`, canto de cima à esquerda, a `(1, 1)`, canto de baixo à direita).
  Para uma cor diferente em cada tema (um gradiente que não pode ser um token), use
  `ThemeColor::themed(Color::hex(0xD4F1F9), Color::hex(0x0E3B4C))` (claro, escuro) em lugar da
  `Color`, também nas paradas do `Brush::linear`.
- **`Card`**: sem `.on_click`, é só um quadro. Com `.on_click`, vira um botão grande: hover,
  pressionado, foco pelo Tab, Enter e Espaço clicam. Dê a ele `.id()` e `.label("Tema")` (o lint
  avisa com `missing-label`; o `label(id)` dos testes lê esse rótulo). O texto dentro do card
  que não tem `.color` acompanha o estado: pressionado, fica secundário; desabilitado, todo texto
  com cor de token fica apagado. Use `Card` para opções de configuração e itens de uma galeria:

```rust
Card::new()
    .id("tema")
    .label("Tema")
    .on_click(move || aberto.set("Tema".to_owned()))
    .child(
        Column::new()
            .gap(4)
            .child(Text::new("Tema").body_strong())
            .child(Text::new("Claro, escuro ou o do sistema.").secondary().wrap().max_lines(2)),
    )
```

- **`SettingsCard`** e **`SettingsExpander`**: a linha de uma página de configurações. O controle
  fica à direita e desce para baixo do texto quando o card tem menos de 476 px (o ícone some
  abaixo de 286 px), sem você fazer nada. O `.content` é uma view comum, com `.id()` e
  `.height(44)` por causa do lint. Com `.on_click`, o card todo é clicável (hover, Tab, Enter e
  Espaço) e mostra o chevron; sem ele, só o controle recebe cliques. No `SettingsExpander`,
  clicar no cabeçalho abre e fecha; o controle do cabeçalho (`.content`) não abre. Os filhos
  (`.item`) perdem fundo, borda e raio. Ids: `{id}-cabecalho` e `{id}-conteudo`.

```rust
SettingsExpander::new("Aparência", aberto)
    .id("aparencia")
    .description("Fonte, cores e o cursor.")
    .icon(Symbol::WindowConsole)
    .item(SettingsCard::new("Tamanho da fonte").id("fonte").content(
        NumberBox::bind(tamanho).id("fonte-valor").height(44),
    ))
```

Exemplo completo: `crates/tomni-ui/examples/settings.rs`.

- **`UniformGrid`**: numa tela de 390 px, `.min_column_width(160)` dá duas colunas; em 700 px,
  quatro. Põe os filhos (em geral `Card`s) linha a linha, todos do tamanho do maior. Dentro de um
  `ScrollViewer` quando houver muitos.
- **`CheckBox`** e **`ToggleSwitch`**: clique e Espaço alternam o signal; Enter não. Leia o
  mesmo signal para reagir: `.disabled(move || !aceito.get())` no botão que depende dos termos.
- **`ToggleButton`**: é um botão, então clique, Enter e Espaço alternam. Use para ligar um modo
  ou um filtro numa barra de botões; para uma opção de configuração, prefira o `ToggleSwitch`, e
  num formulário, a `CheckBox`.
- **Ícones e imagens**: veja a seção "Ícones e imagens".
- **`.visible(prop)`**: com `false`, o widget (e o que estiver dentro dele) some: não ocupa
  espaço, sai do Tab e do lint. Mostrar uma mensagem só em certos casos é
  `.visible(move || erro.get())`.
- **`InfoBar`**: o X não esconde a barra sozinho. Junte com `.visible`:

```rust
let erro = signal(false);
InfoBar::new(InfoBarSeverity::Error)
    .id("erro")
    .title("Erro")
    .message("Este e-mail já está cadastrado.")
    .visible(erro)
    .on_close(move || erro.set(false))
```

  `.action("Abrir agora", move || ...)` põe um botão padrão depois da mensagem (embaixo dela,
  se a barra quebra), com o id `<id da barra>-acao`. Com o de ação e o de fechar, a barra tem
  uma parada só no Tab: Tab e Shift+Tab passam de um botão ao outro, e Enter e Espaço ativam o
  que tem o foco. No `TestApp`, `click("<id>-acao")` clica nele e a árvore o mostra como um
  `button` filho da barra.

- `Align`: `Start`, `Center`, `End`, `Stretch` (padrão: os filhos ocupam a largura da coluna).
- `Justify`: `Start` (padrão), `Center`, `End`, `SpaceBetween`.
- `.padding(16)` vale nos quatro lados. Para lados diferentes: `Insets::symmetric(8.0, 16.0)`
  (vertical, horizontal) ou `Insets::new(cima, direita, baixo, esquerda)`, em `f64`.

**Propriedades** (`texto`, `rotulo`, `titulo`, `placeholder`, `header`, `message`, `disabled`,
`visible`, `tooltip`) aceitam três formas:
- valor fixo: `"Entrar"`, `true`;
- closure, recalculada quando os signals lidos mudam: `move || format!("Itens: {}", n.get())`;
- `Signal` ou `Memo` do mesmo tipo: `.disabled(bloqueado)`.

## Editor de texto e Markdown

Para escrever mais do que um campo (uma nota, um script), use o `TextEditor`; para mostrar
texto com negrito, itálico, riscado, código e links, o `RichText`; para desenhar um documento
Markdown inteiro, o `MarkdownView` (feature `markdown`). `Edit::multiline()` continua sendo o
campo simples, sem quebra automática nem desfazer próprio.

### `TextEditor`

```rust
let texto = signal(String::new());
let controle = TextEditorController::new();

TextEditor::bind(texto)
    .id("editor")
    .placeholder("Escreva aqui")
    .controller(controle.clone())   // para ler a seleção, trocar trechos e dar foco
    .max_height(240.0)              // cresce com o texto até aqui e então rola
    .on_key(|tecla, controle| {     // `true` consome a tecla
        tecla.key == Key::Tab && { controle.replace_selection("  "); true }
    })
```

- **Texto e seleção** em bytes (sempre em limite de `char`). O `TextEditorController`:
  `value()`, `selection()`, `set_selection(faixa)`, `replace(faixa, texto)`,
  `replace_selection(texto)`, `focus()`, `scroll_to(byte)`, `find(trecho, ignora_caixa)`
  (as faixas achadas), `select_all()`, `undo()`, `redo()`, `clear_history()`, e `copy`,
  `cut` e `paste` com um `&mut dyn Clipboard`. Toda troca pelo controlador entra no histórico
  de desfazer (Ctrl+Z, Ctrl+Y) e dispara `.on_change`.
- **Teclado e mouse** como num editor de verdade: setas, Home/End, Ctrl+setas por palavra,
  Shift para selecionar, Ctrl+A, duplo clique (palavra), triplo (linha), Shift+clique, rolagem
  pela roda e arrastando para fora da área. Quebra as linhas na largura (`.wrap(false)` rola
  na horizontal). `.monospace()` usa a Cascadia Mono (feature `mono`; sem ela, a fonte do
  texto).
- **Realce**: `.highlight(|texto| Vec<StyleRange>)` devolve as faixas com
  `StyleRange::new(a..b)` e `.bold()`, `.italic()`, `.strike()`, `.color(..)`,
  `.background(..)`. Roda de novo só quando o texto muda. Use cores do tema que passam no
  contraste AA (as de texto, `TextFillColorSecondary` e acima; a terciária não passa).
  `.marks(Signal<Vec<Mark>>)` pinta as marcas de busca (`Mark { range, current }`; a atual é
  mais forte).
- **Colar e soltar**: `.on_copy(|texto| ..)` recebe o que seria copiado **no lugar** de gravar
  na área de transferência (Ctrl+X ainda apaga). `.on_paste_files(|caminhos, controle| ..)`
  roda ao colar quando há arquivos copiados; `.on_paste_image(|imagem, controle| ..)` quando só
  há imagem (sem ele a imagem nem é lida); `.on_files_dropped(|caminhos, controle| ..)` com o
  cursor já no ponto solto. Sem esses callbacks, o editor só cola texto.
- **Foco**: `controle.focus()` dá o foco ao editor; `request_focus("id")` (função solta,
  `tomni_ui::request_focus`) faz o mesmo para qualquer widget com foco (uma barra de busca que acabou
  de abrir num `Show`): a árvore atende no quadro seguinte e tenta de novo por alguns quadros
  se o widget ainda não existe. Chame de um evento ou de um efeito, não da construção da view.
  Com várias janelas, a do evento que pediu atende primeiro, mesmo que o id exista também em
  outra (a mesma tela montada nas duas); se ela não tem o widget, outra janela atende. Um
  efeito roda no quadro de qualquer janela: dali, diga qual atende primeiro com
  `request_focus_in(janela, "id")` (a ativa, de `active_window()`).
  `start_bring_into_view("id")` (`tomni_ui::start_bring_into_view`, o `StartBringIntoView` do
  WinUI) só rola os `ScrollViewer` em volta até o widget aparecer inteiro, sem mexer no foco:
  serve para a linha de um `TreeView` escolhida pelo código (`{id da árvore}-{chave}`; escolha
  antes, pelo signal, para a linha ser montada).
- **Somente leitura e desabilitado** selecionam e copiam (o primeiro); `.min_height(n)` (padrão
  96) e `.max_height(n)` dão o tamanho; sem `.max_height`, ele ocupa o que o pai der (`grow`).
- O layout é do texto todo (não por parágrafo): textos de dezenas de milhares de linhas pedem
  o `.max_height` e o `.highlight` leve.
- No teste: `app.type_text(..)`, `app.key(..)`, `app.key_with(..)`, `app.shortcut(..)`,
  `app.drop_files(id, &[caminho])`, `app.clipboard()` e **`app.text_editor("id")`**, que devolve
  o `TextEditorVisual` (`text`, `caret`, `selection`, `marks`, `ranges`, `scroller`…). A árvore mostra o
  `kind` `text_editor`.

### `RichText`

```rust
RichText::new(vec![
    Span::new("Veja o "),
    Span::new("manual").link("https://exemplo.com/manual"),
    Span::new(" e rode "),
    Span::new("ls -la").code(),
    Span::new(", "),
    Span::new("sem medo").bold().italic(),
])
.wrap()
.id("texto")
.on_link(move |url| aberto.set(url))
```

`Span::new(texto)` com `.bold()`, `.italic()`, `.strike()`, `.code()` (Cascadia Mono sobre um
fundo suave) e `.link(url)`. Os `spans` aceitam closure ou signal. A rampa e as cores são as do
`Text` (`.ramp`, `.caption()`, `.body_strong()`, `.secondary()`, `.color(..)`, `.wrap()`,
`.max_lines(n)`); `.monospace()` pinta tudo em Cascadia Mono, sem o fundo (para um bloco de
código que já tem o dele). Os links recebem foco com Tab, abrem com Enter ou clique, mostram a
mão no hover e aparecem na árvore como `<id>-link-<n>` (`.link_ids(prefixo, primeiro)` troca
o prefixo e o primeiro número). **Lint**: um link dentro do `RichText` é "em linha" (WCAG 2.2,
critério 2.5.8), então o `touch-target` não cobra 44 px dele; os botões e outros widgets
continuam cobrados.

### `MarkdownView` (feature `markdown`)

`tomni-ui = { version = "..", features = ["markdown"] }` traz `tomni_ui::markdown`
(`MarkdownView` também vem no `prelude`). O texto é CommonMark com tabelas, riscado, listas de
tarefas e notas de rodapé como texto; **HTML aparece como texto**, nunca é interpretado.

```rust
use tomni_ui::markdown::{edit, highlight, MarkdownView};

let texto = signal(String::from("# Notas\n\n- [ ] ligar\n- [x] enviar\n"));

Split::row(
    TextEditor::bind(texto).highlight(highlight).id("editor"),
    ScrollViewer::new().child(
        Border::new().padding(16).child(
            MarkdownView::new(move || texto.get())
                .id("previa")
                .on_link(move |url| abrir(url))
                .on_task(move |byte, _marcada| {
                    let atual = texto.get_untracked();
                    if let Some(troca) = edit::toggle_task(&atual, byte) {
                        texto.set(troca.apply(&atual));
                    }
                }),
        ),
    ),
)
```

O exemplo completo (com Enter continuando a lista e Tab recuando) está em
`crates/tomni-ui/examples/markdown.rs`.

- `MarkdownView::new(texto)` aceita texto fixo, closure ou signal. A cada mudança do texto, só
  os blocos que mudaram são montados de novo: os iguais ficam com os widgets e o layout
  medido (e as closures de `.fence` e `.image` não são chamadas de novo para eles), e cada
  bloco tem a árvore de layout dele, então digitar numa nota de 200 mil caracteres refaz um
  bloco, não o documento. `.on_link(|url| ..)` recebe o destino (o app decide abrir:
  navegador, nota, arquivo). `.on_task(|byte, marcada| ..)` recebe o **byte do `[`** da tarefa
  e o novo estado; sem `.on_task`, as caixas ficam desabilitadas. `.on_copy(|texto| ..)` recebe o
  código do botão "Copiar" dos blocos de código (sem ele, grava no clipboard).
- `.fence("mermaid", |cx: FenceCx| Box<dyn Widget>)` troca o bloco de código dessa linguagem
  por um widget do app (`cx.text`, `cx.lang`, `cx.index`, `cx.id`).
  `.image(|cx: ImageCx| Option<Box<dyn Widget>>)` desenha as imagens `![alt](url)`: devolva
  `Some(widget)` (um `Image` com `.natural_size`, por exemplo) ou `None` para o padrão; o
  `cx.fallback()` é o desenho padrão (o texto alternativo como link) e `cx.fallback_text()`
  é o mesmo desenho como `RichText`, para pôr dentro de um `Show` (carregando e, se falhar,
  o padrão). Sem `.image`, toda imagem aparece como link com o texto alternativo.
- `.on_files_dropped(|caminhos| ..)` consome arquivos soltos sobre a prévia; sem ele, o evento
  sobe aos ancestrais.
- `.code_action("Inserir no terminal", Symbol::.., |cx: FenceCx| ..)` acrescenta um botão de
  ícone a **cada** bloco de código comum, antes do "Copiar" (`cx.text` é o código e `cx.lang`
  a linguagem). Pode ser chamado mais de uma vez; os botões saem na ordem. Serve para destinos
  do app (inserir no terminal, abrir numa nota). Blocos de `.fence` ficam de fora: quem os
  desenha põe os próprios botões.
- **Ids** (todos com o `.id()` do `MarkdownView` na frente): `-bloco-<n>` e `-copiar-<n>` e
  `-rolagem-<n>` (cada bloco de código), `-acao-<k>-<n>` (a ação `k`, de zero, do bloco `n`), `-tarefa-<n>`, `-link-<n>`, `-imagem-<n>`, `-texto-<n>`
  (cada parágrafo com link) e `-tabela-<n>` (a tabela, que rola na horizontal se não cabe). Uma
  imagem em parágrafo reserva um número de imagem **e** um de link: clicar na imagem de
  fallback é o `-link-<k>`, não o `-imagem-<n>`.
- Os marcadores de lista são sempre `•` (a Selawik não tem outros); o recuo mostra o nível.

### `markdown::edit`, `markdown::highlight` e `markdown::links`

Funções puras sobre o texto, para o app montar a barra de formatação e as teclas do editor.
Cada uma devolve uma `Change { range, insert, selection }` (`.apply(texto)` dá o texto novo;
o app troca `range` por `insert` com `controle.replace(..)` e põe a seleção com
`controle.set_selection(..)`) ou `None` quando não há o que fazer:
`toggle_wrap(texto, sel, "**")` (negrito, itálico `"*"`, riscado `"~~"`, código `` "`" ``),
`toggle_prefix(texto, sel, Prefix::{Bullet, Numbered, Task, Quote, Heading(n)})`,
`continue_list(texto, cursor)` (Enter dentro de uma lista), `indent(texto, sel, desfazer)`
(Tab e Shift+Tab), `insert_block(texto, cursor, Block::{Code(lang), Table { cols, rows },
Rule, Link})`, `surround(texto, sel, Block)` e `toggle_task(texto, byte)`.
`highlight(texto)` é o realce de Markdown para o `.highlight(..)` do editor.
`links(texto)` devolve cada link e imagem com a faixa em bytes do destino (`LinkRef { dest,
dest_range, image }`), para o app reescrever caminhos (por exemplo, ao exportar).

## Ícones e imagens

**`Icon`** desenha um dos ícones Fluent (MIT) do enum `Symbol`, num quadrado de 20 px; `.size`
escolhe 12, 16, 20 ou 24, os tamanhos desenhados (outro valor amplia ou reduz o mais perto). Sem
`.color`, tem a cor do texto, e dentro de um `Card` acompanha o estado como o texto. O lint
confere o contraste do ícone com o fundo (3:1). Um ícone ao lado de um texto que diz a mesma
coisa é só enfeite: marque com `.decorative()`, e o lint não o confere.

```rust
Row::new()
    .gap(8)
    .align(Align::Center)
    .child(Icon::new(Symbol::Mail).decorative())
    .child(Text::new("Caixa de entrada"))
```

O ícone aceita closure: `Icon::new(move || if fixo.get() { Symbol::PinOff } else { Symbol::Pin })`.

**Botões com ícone**: `Button::new("Salvar").icon(Symbol::Save)` põe um ícone de 16 px antes do
rótulo. `Button::icon_only(Symbol::Settings)` (e `ToggleButton::icon_only(signal, Symbol::Star)`)
é um quadrado de 44 px só com o ícone: passe `.label("Configurações")`, com o que ele faz. O
rótulo não aparece, mas vira a dica no hover, o nome na árvore e o que o `label(id)` dos testes
lê. Sem `.label`, o lint avisa com `missing-label`.

```rust
Row::new()
    .gap(4)
    .child(Button::icon_only(Symbol::ArrowUndo).id("desfazer").label("Desfazer").on_click(move || desfazer()))
    .child(ToggleButton::icon_only(favorito, Symbol::Star).id("favorito").label("Favorito"))
```

**`Image`** mostra um PNG, JPEG, GIF, WebP ou SVG (do GIF e do WebP animados, o primeiro quadro). Dos bytes do programa
(`Image::bytes(include_bytes!("imagens/logo.svg"))`, com o caminho relativo ao arquivo `.rs`) ou
de um arquivo lido ao montar a tela (`Image::file("fotos/praia.jpg")`, relativo à pasta em que o
app roda). Toda imagem pede `.alt("o que ela mostra")` ou, se for só enfeite, `.decorative()`;
sem os dois, o lint dá o erro `missing-alt`.
- Sem `.width`/`.height`, fica no tamanho natural (os pixels; no SVG, o `width`/`height` dele) e
  encolhe na proporção quando falta largura.
- Com os dois, `.stretch` diz como ela ocupa o retângulo: `Stretch::Uniform` (o padrão: inteira,
  na proporção, com sobra dos lados), `Stretch::UniformToFill` (cobre tudo e corta o que passa,
  bom para fotos de capa), `Stretch::Fill` (estica) e `Stretch::None` (tamanho natural, cortado).
- `.corner_radius(8)` arredonda os cantos.
- Um arquivo que não existe ou bytes que não são imagem não derrubam o app: a imagem vira um
  quadro vazio e a árvore mostra o motivo em `error="..."`.
- `tomni_ui::image::probe(&bytes)` devolve `Ok((largura, altura))` ou a mensagem de erro em
  português, pelo mesmo caminho que desenha a imagem. Decodifica tudo: chame numa thread (ou ao
  guardar a imagem), nunca no quadro. Passe o resultado a `.natural_size(largura, altura)`: sem
  `.width`/`.height`, a imagem mede então `min(largura, largura disponível)` de largura e a
  altura na proporção, **sem decodificar para medir** (é a "imagem ajustada à largura da
  coluna" de uma nota).

```rust
static FOTO: &[u8] = include_bytes!("imagens/praia.jpg");

Image::bytes(FOTO)
    .id("capa")
    .height(160)
    .stretch(Stretch::UniformToFill)
    .corner_radius(8)
    .alt("Praia ao entardecer")
```

Os ícones do `Symbol`, por grupo:

<!-- ícones: início (gerado por crates/theme/icons/gerar.py) -->

- **Navegação**: `Home`, `ArrowLeft`, `ArrowRight`, `ArrowUp`, `ArrowDown`, `ChevronLeft`, `ChevronRight`, `ChevronUp`, `ChevronDown`, `Navigation`, `MoreHorizontal`, `MoreVertical`, `Open`, `ArrowClockwise`, `ArrowCounterclockwise`, `ArrowSync`, `ArrowUndo`, `ArrowRedo`, `ArrowExit`, `SignOut`, `ArrowEnter`, `Grid`, `Apps`, `AppsList`, `PanelLeft`, `PanelRight`, `PanelBottom`, `SplitVertical`, `SplitHorizontal`, `Globe`, `Map`, `Location`, `Compass`, `Search`, `Filter`, `ArrowSort`, `ZoomIn`, `ZoomOut`, `FullScreenMaximize`, `FullScreenMinimize`, `Maximize`, `Dismiss`.
- **Ações**: `Add`, `Subtract`, `Edit`, `Delete`, `Save`, `Copy`, `Cut`, `ClipboardPaste`, `Share`, `Send`, `Print`, `Checkmark`, `Pin`, `PinOff`, `Star`, `StarFilled`, `StarOff`, `Heart`, `HeartFilled`, `Bookmark`, `BookmarkFilled`, `Flag`, `Tag`, `Link`, `Attach`, `Archive`, `Rename`, `LockClosed`, `LockOpen`, `Key`, `Eye`, `EyeOff`, `Options`, `ArrowReply`, `ArrowForward`, `ArrowDownload`, `ArrowUpload`, `Cloud`, `CloudArrowUp`, `CloudArrowDown`, `SelectAll`.
- **Arquivos e conteúdo**: `Document`, `DocumentAdd`, `DocumentText`, `Folder`, `FolderOpen`, `FolderAdd`, `Image`, `ImageMultiple`, `Video`, `MusicNote`, `Code`, `Database`, `Server`, `Clipboard`, `Notepad`, `Book`, `Library`, `Briefcase`, `Box`.
- **Status**: `Info`, `Warning`, `ErrorCircle`, `CheckmarkCircle`, `DismissCircle`, `QuestionCircle`, `Alert`, `AlertOff`, `Shield`, `ShieldCheckmark`, `Clock`, `History`, `Calendar`, `Hourglass`, `Circle`, `CircleFilled`, `Sparkle`, `Lightbulb`, `Bug`, `Wifi`, `Battery`, `Flash`, `DataArea`.
- **Mídia**: `Play`, `Pause`, `Stop`, `Previous`, `Next`, `Record`, `Speaker`, `SpeakerMute`, `Mic`, `MicOff`, `Camera`, `Headphones`, `FastForward`, `Rewind`, `Broadcast`.
- **Sistema e pessoas**: `Settings`, `Person`, `People`, `PersonAdd`, `PersonCircle`, `Color`, `PaintBrush`, `DarkTheme`, `WeatherMoon`, `WeatherSunny`, `Desktop`, `Laptop`, `Phone`, `Tablet`, `Keyboard`, `Window`, `WindowNew`, `PlugConnected`, `Power`, `Accessibility`, `LocalLanguage`, `TextFont`, `Wrench`, `Toolbox`.
- **Comunicação**: `Mail`, `Chat`, `ChatMultiple`, `Comment`, `Call`, `Megaphone`.
- **Texto**: `TextBold`, `TextItalic`, `TextUnderline`, `TextStrikethrough`, `TextAlignLeft`, `TextAlignCenter`, `TextAlignRight`, `TextBulletList`, `TextNumberList`, `TextHeader1`, `Table`, `TaskListLtr`, `TextQuote`, `LineHorizontal1`, `CodeBlock`.
- **Desenvolvimento**: `Branch`, `WindowConsole`, `PuzzlePiece`, `Beaker`.
- **Diversos**: `Cart`, `Money`, `Gift`, `Trophy`, `Rocket`, `Emoji`, `ThumbLike`, `ThumbLikeFilled`, `ThumbDislike`.

<!-- ícones: fim -->

## Opções, valores e seções

Os grupos dão ids às partes que montam, a partir do id deles: a opção, a aba ou a sugestão vira
minúsculas, e o que não é letra ou número vira `-` (`tema-do-sistema`, `config-avancado`,
`cidade-belo-horizonte`). Dê `.id()` ao grupo; as partes não têm builder.

**`RadioButtons`**: as opções ficam uma embaixo da outra, com 44 px cada. O signal guarda o
texto da opção escolhida; um valor que não é de nenhuma deixa todas desmarcadas. O grupo é uma
parada só do Tab (entra na opção escolhida); ↑ e ↓ mudam o foco e a escolha juntos.

```rust
let tema = signal("Do sistema".to_owned());
RadioButtons::bind(tema, ["Claro", "Escuro", "Do sistema"])
    .id("tema")
    .header("Tema")
```

**`Slider`**: o signal é `f64`, e o valor fica sempre dentro da faixa e num múltiplo do
`.step`. Clique na trilha ou arrasto; com o foco, ← → ↑ ↓ andam um passo, PageUp/PageDown um
décimo da faixa, Home/End vão às pontas. Mostre o número num `Text` se ele importa:
`Text::new(move || format!("{:.0}%", volume.get()))`. Sem `.header`, tem 32 px de altura: use
`.height(44)`.

**`ProgressBar` e `ProgressRing`**: `.value(prop)` vai de 0 a 1 (`move || feitos.get() as f64
/ total as f64`). Sem `.value`, a animação roda enquanto o widget aparece; esconda com
`.visible(sincronizando)` quando a tarefa termina. Não recebem foco nem clique. Só a
`ProgressBar` tem estado: `.paused(prop)` pinta a barra com a cor de alerta (`ShowPaused` do
WinUI) e `.error(prop)` com a de erro (`ShowError`, vale mais que a pausa); sem `.value`, a
barra com estado fica cheia e parada. O `ProgressRing` tem o mesmo `.paused(prop)` e
`.error(prop)`: o anel com estado, sem `.value`, é inteiro e parado, na cor do estado.
`app.look("id")` dá `Look::ProgressBar(v)` com `v.status` (`ProgressStatus::{Normal, Paused,
Error}`); o anel é `Look::ProgressRing(v)`.

**`AreaChart`, `Sparkline` e `LevelMeter`**: os gráficos recebem a série inteira
(`Vec<f64>`, fixa, closure ou `Signal`), do ponto mais antigo ao mais novo; mostram os
`.capacity(..)` mais novos, com o mais novo na borda direita, na escala de 0 a `.max(..)`.
Guarde a série num signal e limite o tamanho no app (`if s.len() > 60 { s.remove(0); }`). O
`AreaChart` tem 160×100 de tamanho natural (use `.grow(1)` ou `.height(..)`), a `Sparkline`
72×40. `.secondary(serie)` desenha uma segunda série por cima da primeira, na mesma escala, com
a linha e a área translúcida na `.secondary_color(..)` (o vermelho `SystemFillColorCritical`
por padrão): o tempo de kernel embaixo da CPU, como no Gerenciador de Tarefas; vazia, some. O
`Look::Chart(v)` traz `v.secondary`. O `LevelMeter` acende `nivel × segmentos` quadradinhos; com `.on_click` recebe foco e
Enter, e precisa de `.height(44)` para o alvo de toque. Não são acessíveis só pela cor:
mostre o número num `Text` ou no `.tooltip`. `app.value("id")` dá o nível do medidor e o
ponto mais novo do gráfico (falha se a série está vazia); `app.look("id")` dá
`Look::Chart(v)` (`v.kind`: `ChartKind::{Area, Sparkline}`) ou `Look::LevelMeter(v)`. Exemplo:
`crates/tomni-ui/examples/desempenho.rs`.

**`HeatMap`**: um ladrilho por valor (`Vec<f64>`, fixa, closure ou `Signal`), da esquerda para
a direita e de cima para baixo, em colunas e linhas que deixam cada ladrilho o mais perto de
quadrado e ocupam a área toda (`tomni_ui::heat_map_grid`); a cor vai de quase transparente (0)
a cheia (`.max(..)`, 100 por padrão). É o que o Gerenciador de Tarefas mostra no lugar dos
gráficos por núcleo quando a máquina tem muitos: 512 ladrilhos cabem onde 512 `AreaChart` não
caberiam. Tem 160×100 de tamanho natural e encolhe com `.height(0).grow(1)`. O ladrilho sob o
ponteiro ganha borda forte, e `.cell_tooltip(|n, valor| ...)` dá a dica dele (n começa em 0).
Nos testes, `app.values("id")` dá os valores, e `tomni_ui::heat_map_cell(total, app.rect("id"), n)`
o retângulo do ladrilho `n` (para `app.move_to(.. .center())`); a dica é o `"{id}-tooltip"`.
`app.look("id")` dá `Look::HeatMap(v)`, com `v.hovered`.

```rust
HeatMap::new(move || nucleos.with(|s| s.iter().map(|h| h.last().copied().unwrap_or_default()).collect()))
    .id("nucleos")
    .cell_tooltip(|n, uso| format!("Processador {n}: {uso:.0}%"))
    .height(0)
    .grow(1)
```

**`Expander`**: o cabeçalho (48 px) abre e fecha com clique, Enter ou Espaço; o signal
acompanha. Fechado, o conteúdo sai do layout e do Tab; o estado dele (texto digitado) fica.
`.description(texto)` põe uma segunda linha no cabeçalho (legenda, na cor secundária, cortada
com reticências), que passa a ter 64 px: bom para uma sanfona de seções com um resumo de cada.

```rust
let avancado = signal(false);
Expander::new("Avançado", avancado).id("avancado").content(
    Column::new().gap(8).child(CheckBox::bind(depurar, "Modo de depuração").id("depurar").height(44)),
)
```

**`TabView`**: o signal guarda a **chave** da aba (o primeiro argumento de `.tab`), não o
título. As abas (44 px) ficam numa faixa no alto e o conteúdo da escolhida embaixo; as outras
continuam montadas, só escondidas, e não perdem o que foi digitado. Clique, ← → (a escolha
segue o foco) e Ctrl+Tab/Ctrl+Shift+Tab de qualquer lugar dentro dele. Ids: `{id}-{chave}` na
aba e `{id}-{chave}-conteudo` no painel. Numa `Column`, use `.grow(1)` para o conteúdo ocupar o
resto da tela.

`.tab_progress(|chave| Option<TabProgress>)` põe um anel de progresso no lugar do ícone da aba
(`TabProgress { value: Option<f64>, status: ProgressStatus }`; sem valor o anel gira) e
`.tab_attention(|chave| bool)` acende um ponto de atenção no canto do ícone, que fica por cima do
anel. `.tab_connecting(|chave| bool)` só muda o nome da aba: com o anel indeterminado, ela diz
"conectando" no lugar de "trabalhando" (o `TerminalPanes` liga isso a `Session::connecting()`).
As closures são reativas. O texto da aba (`app.text("id-chave")`) ganha o estado
("Testes, 45%, pede atenção"): `app.select` casa pelo texto inteiro, então numa aba com estado
use `app.click("id-chave")`. Exemplo: `examples/tab_view.rs`, tela `atividade`.

```rust
let aba = signal("geral".to_owned());
TabView::bind(aba)
    .id("config")
    .grow(1)
    .tab("geral", "Geral", Column::new().gap(12).child(/* ... */))
    .tab("avancado", "Avançado", Column::new().gap(12).child(/* ... */))
```

Abas que abrem e fecham (documentos, notas, conexões) vêm de uma lista, como no `For`:
`.items(lista, chave, título, view)`. Um item novo cria a aba; um item que sai descarta a aba
(e o que estava nela); mover itens não perde o conteúdo. Com `.on_close(move |chave| ...)`, as
abas ganham o X (na escolhida e na que está sob o mouse); o clique do meio na aba e o Ctrl+F4
também fecham. O `on_close` só avisa: **o app tira o item da lista**. Se a fechada era a
escolhida, a escolha passa para a vizinha da direita (ou da esquerda, se era a última).
`.on_add(move || ...)` põe o "+" depois da última aba: crie o item e escolha a aba nova. As
abas dividem a largura por igual, entre 100 e 240 px (`.tab_width(TabWidth::SizeToContent)`
mede pelo título); quando não cabem, aparecem as setas ◀ ▶ nas pontas da faixa. Ids:
`{id}-{chave}-fechar` (o X), `{id}-adicionar` (o "+"), `{id}-rolar-esquerda` e
`{id}-rolar-direita`; a chave entra no id em minúsculas, com `-` no lugar do que não é letra
nem número. Com `.on_reorder(move |de, para| ...)`, o usuário arrasta uma aba para outro lugar
da faixa (uma linha mostra onde ela vai cair): mova o item na sua lista, de `de` para `para`
(os índices da ordem atual). Não use `adicionar`, `abas`, `rolar-esquerda` nem `rolar-direita` como chave: os
ids colidiriam com os dos botões e da faixa.

```rust
#[derive(Clone)]
struct Nota { id: u32, titulo: String, texto: Signal<String> }

let notas = signal(vec![Nota { id: 1, titulo: "Compras".into(), texto: signal(String::new()) }]);
let aberta = signal("1".to_owned());
Form::new("Notas").density(Density::Compact).child(
    TabView::bind(aberta)
        .id("notas")
        .grow(1)
        .items(notas, |n| n.id, |n| n.titulo.clone(), |n| Edit::bind(n.texto).id(format!("texto-{}", n.id)))
        .on_close(move |chave| notas.update(|v| v.retain(|n| n.id.to_string() != chave)))
        .on_add(move || {
            let id = notas.with_untracked(|v| v.iter().map(|n| n.id).max().unwrap_or(0)) + 1;
            notas.update(|v| v.push(Nota { id, titulo: format!("Nota {id}"), texto: signal(String::new()) }));
            aberta.set(id.to_string());
        })
        .on_reorder(move |de, para| notas.update(|v| {
            let nota = v.remove(de);
            v.insert(para, nota);
        })),
)
```

**`NavigationView`**: o app inteiro com um menu lateral. Ponha-o direto no `Form` (ele ocupa
a janela e escolhe o modo pela largura dela): a partir de 1008 px, o painel de 320 fica aberto
ao lado da página (`NavigationMode::Expanded`); de 641 a 1007, só os ícones, numa faixa de 48
(`Compact`); abaixo, só o ☰ no canto (`Minimal`). Nos dois últimos, o ☰ abre o painel por cima
da página, e ele fecha com Esc, com um clique fora ou ao escolher um item; `.pane_mode(modo)`
fixa um modo e `.pane_mode_with(signal_ou_closure)`, de `Option<NavigationMode>`, o troca em
tempo de execução (`None` volta a escolher pela largura). O signal guarda a **chave** do item (o primeiro argumento de `.item`), não o
rótulo. Um grupo (`.group`) não tem página: abre e fecha os filhos (no compacto, num popup ao
lado do ícone). As páginas dos outros itens continuam montadas, só escondidas. Teclado: o
painel é uma parada só de Tab (o item escolhido); ↑ ↓ andam, Enter e Espaço escolhem, → abre
o grupo e ← fecha. Ids: `{id}-{chave}` no item, `{id}-{chave}-conteudo` na página,
`{id}-{chave}-filhos` no popup dos filhos (compacto) e `{id}-{chave}-flyout` em cada filho
dele, `{id}-alternar` no ☰, `{id}-configuracoes` no item do `.settings`. A busca é um
`AutoSuggestBox` comum: `.height(44)`.

```rust
let pagina = signal("inicio".to_owned());
Form::new("Biblioteca").child(
    NavigationView::bind(pagina)
        .id("nav")
        .item("inicio", Symbol::Home, "Início", Column::new().padding(24).child(/* ... */))
        .header("Coleção")
        .item("livros", Symbol::Book, "Livros", Column::new().padding(24).child(/* ... */))
        .group("midia", Symbol::ImageMultiple, "Mídia", |g| {
            g.item("fotos", Symbol::Camera, "Fotos", Column::new().child(/* ... */))
                .item("videos", Symbol::Video, "Vídeos", Column::new().child(/* ... */))
        })
        .footer_item("conta", Symbol::Person, "Conta", Column::new().child(/* ... */))
        .settings(Column::new().padding(24).child(/* ... */)),
)
```

**`TitleBar`** (a barra de título do WinUI): `Form::new(...).title_bar(TitleBar::new("Título"))`
troca a barra do sistema por uma desenhada pelo app, no tema dele. Ela tem, da esquerda para a
direita: o voltar (`.back(true)`, com `.on_back`), o ☰ de um `NavigationView` da tela (vai para
a barra sozinho, e o painel perde o dele), o ícone, o título, o subtítulo, o `.content` (uma
busca, no meio), o `.right_header` e os botões de minimizar, maximizar e fechar. Com
`.content` ou `.right_header`, ela tem 48 px de altura; sem, 32. Arrastar a barra move a
janela, o duplo clique maximiza e restaura, e as bordas da janela redimensionam; os controles
dentro dela continuam funcionando normalmente. Com a janela em segundo plano, a barra fica
apagada. No macOS, os três botões são os do sistema, à esquerda; onde não dá para tirar a
barra do sistema (Android), ela continua, e a do app fica embaixo, sem os botões da janela. Os
widgets de dentro da barra (a busca, um botão) podem ter 32 px: o lint não pede 44 px dentro
dela. Ids: `barra-de-titulo` (ou o `.id`), com `-voltar`, `-alternar`, `-minimizar`,
`-maximizar` e `-fechar`.

```rust
let pagina = signal("inicio".to_owned());
let busca = signal(String::new());
Form::new("Biblioteca")
    .title_bar(
        TitleBar::new("Biblioteca")
            .icon(Symbol::Library)
            .back(true)
            .back_enabled(move || pagina.get() != "inicio")
            .on_back(move || pagina.set("inicio".to_owned()))
            .content(AutoSuggestBox::bind(busca, |_| Vec::new()).id("busca").width(360)),
    )
    .child(NavigationView::bind(pagina).id("nav").item(/* ... */))
```

**`AutoSuggestBox`**: a closure recebe o texto digitado e devolve as sugestões (`Vec<String>`);
filtrar é com você. A lista abre embaixo quando há sugestões e fecha quando não há. O foco fica
sempre no campo: ↓ e ↑ andam pela lista e põem a sugestão no campo, Enter ou um clique
escolhem (fecham a lista e chamam o `.on_submit` com o valor), Esc fecha e volta ao que foi
digitado. 32 px de altura: use `.height(44)`. `.focus_shortcut(Shortcut::ctrl('E'))` leva o
foco ao campo com o atalho (vale enquanto o campo aparece), como a busca da barra de título O atalho aceita `Signal` ou closure, como o `.accelerator`.

```rust
let cidade = signal(String::new());
AutoSuggestBox::bind(cidade, |texto: &str| {
    let texto = texto.to_lowercase();
    ["Belo Horizonte", "Belém", "Brasília"]
        .into_iter()
        .filter(|c| !texto.is_empty() && c.to_lowercase().starts_with(&texto))
        .map(str::to_owned)
        .collect()
})
.id("cidade")
.placeholder("Cidade")
.height(44)
```

**Senha** (o `PasswordBox` do WinUI): `.password()` troca cada caractere por `•` e não deixa
copiar nem recortar. Depois que a pessoa digita, aparece um olho no fim do campo
(`{id}-revelar`) que mostra a senha só enquanto está pressionado. O olho some quando o campo
perde o foco com texto e só volta quando o campo fica vazio: quem pega o aparelho depois não
vê a senha. `.password_reveal(...)` muda isso: `PasswordReveal::Peek` (padrão), `Hidden` (sem olho) e `Visible` (a senha à vista,
para um "Mostrar senha" do app):

```rust
let mostrar = signal(false);
Edit::bind(senha)
    .id("senha")
    .password()
    .password_reveal(move || if mostrar.get() { PasswordReveal::Visible } else { PasswordReveal::Hidden })
    .height(44)
```

**`NumberBox`** (o do WinUI): digita-se livremente; o número vale com Enter ou quando o foco
sai, preso entre `.min` e `.max`. Com `.precision(casas)`, é arredondado (`.precision(0)` para
inteiros); sem ele, fica como veio e o campo mostra até 10 dígitos significativos (`0,1 + 0,2`
aparece `0,3`). O campo mostra com vírgula (`1,5`) e aceita vírgula ou ponto. Ao receber o
foco pelo Tab, o texto fica todo selecionado e digitar troca o número (num clique, o cursor
vai para o ponto clicado). Esc volta ao valor. Texto que não
é número volta ao valor anterior; vazio também, a não ser no `bind_optional` (vazio, as setas
não fazem nada). ↑ e ↓ somam e subtraem o `.small_change`, PageUp e PageDown o
`.large_change`, e a roda do mouse muda o valor com o foco no campo. Com foco e texto, um X
(`{id}-limpar`) apaga. Os botões de incremento (`{id}-aumentar` e `{id}-diminuir`) não
aparecem por padrão (`SpinButtons::Hidden`, como no WinUI); com `.spin_buttons`, ficam
`SpinButtons::Compact` (num popup sobre a ponta do campo enquanto ele tem o foco) ou `Inline`
(dentro do campo, sempre à vista). Segurados, repetem a cada `tomni_ui::SPIN_REPEAT` (250 ms);
no `.max` (ou no `.min`), o botão desabilita. 32 px de altura: use `.height(44)`. Para mostrar
um número num `Text`, `tomni_ui::format_number(valor, casas)` formata igual ao campo com
`.precision(casas)`.

```rust
let quantidade = signal(1.0);
NumberBox::bind(quantidade)
    .id("quantidade")
    .min(1)
    .max(99)
    .precision(0)
    .height(44)
```

## Listas e partes que mudam

`For::each(itens, |item| chave, |item| view)` cria uma view para cada item. Quando a lista muda,
compara as chaves: item novo ganha view, item que saiu tem a view descartada e os outros mantêm
a view e o estado (texto digitado, foco), mesmo trocando de posição. As views entram direto no
contêiner de cima, com o `gap` dele.

```rust
#[derive(Clone)]
struct Tarefa {
    id: u32,              // a chave: única e que não muda
    texto: String,
    feita: Signal<bool>,  // o que muda no item fica num signal dentro dele
}

let tarefas = signal(Vec::<Tarefa>::new());
let proximo = signal(1_u32);
Column::new().gap(8).child(For::each(
    tarefas,
    |t| t.id,
    move |t| {
        let id = t.id;
        Row::new()
            .gap(8)
            .child(CheckBox::bind(t.feita, t.texto).id(format!("tarefa-{id}")).height(44).grow(1))
            .child(
                Button::new("Remover")
                    .id(format!("remover-{id}"))
                    .height(44)
                    .on_click(move || tarefas.update(|v| v.retain(|t| t.id != id))),
            )
    },
))
```

- **A view de cada item é criada uma vez**, com o valor do item naquele momento. Mudar um campo
  comum do item (sem mudar a chave) não aparece na tela: guarde o que muda num `Signal` dentro
  do item (`feita` acima) e passe esse signal ao widget.
- Para adicionar: `tarefas.update(|v| v.push(Tarefa { id, texto, feita: signal(false) }))`, com
  o `id` vindo de um contador (`proximo`), nunca da posição na lista.
- Contar ou filtrar lendo os signals dos itens:
  `let pendentes = move || tarefas.with(|v| v.iter().filter(|t| !t.feita.get()).count());`
- Ids dos widgets de cada item com a chave (`format!("remover-{id}")`): únicos na tela.

`Show::when(cond, || view).otherwise(|| outra)` troca uma view pela outra quando `cond` muda: a
que sai é descartada (com o estado e os signals criados nela) e a que entra é criada de novo.
Mensagem de lista vazia:

```rust
Show::when(move || tarefas.with(Vec::is_empty), || Text::new("Nada aqui").id("vazia"))
```

Para só esconder, guardando o estado (o texto de um `Edit`), use `.visible(prop)`.

**Rolagem**: conteúdo que não cabe vai num `ScrollViewer`. Numa `Column`, com `.grow(1)`, ele
fica com a altura que sobra e o resto da tela continua no lugar:

```rust
Form::new("Tarefas").child(
    Column::new()
        .gap(12)
        .padding(24)
        .child(Text::new("Tarefas").title())
        .child(
            ScrollViewer::new().id("lista").grow(1).child(
                Column::new()
                    .gap(8)
                    .padding(Insets::new(0.0, 12.0, 0.0, 0.0)) // a barra fica por cima, à direita
                    .child(For::each(tarefas, |t| t.id, move |t| /* ... */)),
            ),
        ),
)
```

Rola com a roda do mouse, o dedo, a barra (que expande com o mouse em cima), PageUp/PageDown,
Home/End e setas; o Tab rola até o widget focado.

## TreeView (pastas e itens)

`TreeView::bind(itens, selecionado)`: árvore de pastas e itens, como uma lista de
favoritos. Os itens ficam num `Signal<Vec<TreeItem>>`, a seleção num `Signal<Option<String>>`
(a chave do item); o app muda a lista com os métodos de `TreeItems`, que mantêm a seleção e as
pastas abertas, porque tudo é identificado pela **chave** (única na árvore toda, que não muda).

```rust
fn favoritos() -> impl View {
    let itens = signal(vec![
        TreeItem::folder("producao", "Produção")
            .child(TreeItem::leaf("web-01", "web-01").icon(Symbol::Server))
            .child(TreeItem::leaf("db-01", "db-01").icon(Symbol::Database)),
        TreeItem::leaf("bastion", "bastion").icon(Symbol::Desktop),
    ]);
    let selecionado = signal(None::<String>);
    let aberto = signal(BTreeSet::from(["producao".to_owned()])); // opcional
    let filtro = signal(String::new());
    let renomeando = signal(None::<String>);
    let status = signal("Pronto".to_owned());

    Form::new("Favoritos").child(
        Column::new()
            .gap(8)
            .padding(16)
            .child(Edit::bind(filtro).id("filtro").placeholder("Filtrar").height(44))
            .child(
                TreeView::bind(itens, selecionado)
                    .id("servidores")
                    .grow(1)
                    .expanded(aberto)
                    .filter(filtro)
                    .renaming(renomeando)
                    .on_invoke(move |chave| status.set(format!("Abrir {chave}")))
                    .on_rename(move |chave, texto| {
                        itens.update(|v| {
                            v.rename(&chave, texto);
                        });
                        renomeando.set(None);
                    })
                    .on_delete(move |chave| {
                        itens.update(|v| {
                            v.remove_item(&chave);
                        });
                    })
                    .on_move(move |chave, pai, posicao| {
                        itens.update(|v| {
                            v.move_to(&chave, pai.as_deref(), posicao);
                        });
                    }),
            )
            .child(
                Button::new("Renomear").id("renomear").height(44).on_click(move || {
                    renomeando.set(selecionado.get_untracked());
                }),
            )
            .child(Text::new(status).id("status")),
    )
}
```

- **`TreeItem::folder(chave, rótulo)`** é pasta (com `.child(...)`/`.children(...)`);
  **`TreeItem::leaf`** é folha. `.icon(Symbol::...)` troca o ícone (o padrão é pasta/arquivo); `.badge(Symbol::...)` põe um
  ícone pequeno depois do rótulo (para marcar o que tem algo a mais); `.search_text(...)` dá um
  texto que o `.filter` também procura, sem aparecer na linha (uma descrição).
- **`TreeItems`** (no `Vec<TreeItem>` do signal, dentro de `itens.update(|v| ...)`): `find`,
  `find_mut`, `parent_of`, `insert_item(pai, posição, item)`, `remove_item(chave)`,
  `move_to(chave, pai, posição)`, `move_up`, `move_down`, `rename(chave, rótulo)`. Os que podem
  falhar (chave repetida, pai que não é pasta, mover para dentro de si mesmo) devolvem `false`
  e não mudam nada. `pai` é `None` para a raiz. `on_move` entrega o pai e a posição já na
  convenção do `move_to`.
- **Os callbacks não mudam a lista sozinhos**: `.on_rename`, `.on_delete` e `.on_move` só avisam;
  o app aplica em `itens` (acima). `.on_invoke` roda no clique duplo e no Enter (a pasta também
  abre ou fecha).
- **Interação**: clique seleciona; setas ↑/↓ andam, → abre a pasta ou desce para o primeiro
  filho, ← fecha ou sobe para o pai; Home/End; letras vão para o próximo rótulo que começa por
  elas; F2 renomeia (ou `renomeando.set(Some(chave))`); Delete chama `.on_delete`; Esc cancela
  o renomear. Arrastar uma linha mostra a linha de inserção, abre a pasta
  fechada depois de 1 s parado e rola nas bordas; `.can_drop(|chave, pai| ...)` proíbe destinos.
  Linhas de um `DataGrid` também caem nas pastas: `.on_grid_drop("grupo", |d: DataGridDrop| ...)`
  recebe as de um grid com `.drag_group("grupo")`, com `target: Some(chave da pasta)`; a pasta
  ganha a marca de "para dentro" e abre depois de 1 s parada. Folhas e o espaço entre as linhas
  não aceitam; `.can_grid_drop(|d| bool)` proíbe pastas. Como os outros, só avisa: o app move.
- **Filtro**: `.filter(signal)` mostra as linhas que casam (sem acento e sem diferença de
  maiúsculas) e os ancestrais delas; os filhos de uma pasta que casa só aparecem se casarem.
- **Menu de contexto**: `.item_context_flyout(|chave| MenuFlyout::new()...)`; o clique direito
  seleciona a linha e abre o menu.
- **Estado e dica da linha**: `.item_status(|chave| Option<ItemStatus>)` põe um selo de 10 px no
  canto do ícone (`Success` verde cheio, `Caution` anel âmbar, `Critical` vermelho com "!",
  `Neutral` anel cinza; `None` não põe nada) e `.item_tooltip(|chave| Option<String>)` dá a dica
  da linha (texto vazio ou `None` não abre). As duas funções são lidas ao desenhar: se leem
  signals, só a linha é redesenhada, sem montar a árvore de novo. O nome acessível da linha
  ganha o estado ("web-01, conectado"; textos em `Strings`, `tree_status_*`) e o inspetor
  mostra `status` (`success`, `caution`, `critical`, `neutral`). A cor não é a única pista: o
  selo muda de forma. Quem mostra o estado de uma pasta recolhida (o mais grave de dentro)
  calcula isso no app.
- **Densidade**: linhas de 44 px (24 px com `.density(Density::Compact)` ou no contêiner).
- **Muitos itens**: só as linhas perto da área visível existem como widgets (10.000 filhos numa
  pasta continuam leves); no teste, leia `app.tree_items(id)`, não procure widgets de linhas
  fora da janela.
- **Ids**: a árvore é o `.id(...)` dela; cada linha montada é `{id}-{chave}` (`servidores-web-01`),
  e a que está sendo renomeada tem o `Edit` `{id}-{chave}-nome`.
- **No teste**: `app.tree_items("servidores")` (chave, rótulo, nível, aberta, selecionada, só as
  linhas visíveis), `app.expand("servidores", "producao")`, `app.collapse(...)`,
  `app.click("servidores-web-01")`, `app.double_click(...)`, `app.key(Key::F(2))`,
  `app.drag("servidores-web-01", "servidores-bastion")`, `app.right_click(...)`.

## DataGrid (tabela de linhas e colunas)

`DataGrid::bind(linhas, selecionadas)`: tabela com cabeçalho que ordena, seleção múltipla,
filtro, renomear e arrastar, como a lista de arquivos de um gerenciador. As linhas ficam num
`Signal<Vec<DataGridRow>>` e a seleção num `Signal<BTreeSet<String>>` (as chaves); a ordem
(`.sort`), o filtro (`.filter`) e o nome em edição (`.renaming`) também são signals do app.
Cada linha traz o texto de cada célula **e** o valor que ordena; o `DataGrid` ordena e filtra
sozinho. A chave da linha é única no grid e não muda (renomear troca o texto, não a chave).

```rust
fn arquivos() -> impl View {
    let linhas = signal(vec![
        DataGridRow::new("docs")
            .cell("docs", CellValue::Text("docs".into()))
            .cell("", CellValue::Number(0.0))
            .icon(Symbol::Folder)
            .rank(1)        // pastas antes dos arquivos, em qualquer ordem
            .drop_target(), // aceita itens soltos sobre ela
        DataGridRow::new("notas.txt")
            .cell("notas.txt", CellValue::Text("notas.txt".into()))
            .cell("12 KB", CellValue::Number(12288.0))
            .icon(Symbol::Document)
            .rank(2),
    ]);
    let selecionadas = signal(BTreeSet::<String>::new());
    let ordem = signal(Some(("nome".to_owned(), SortDirection::Ascending)));
    let filtro = signal(String::new());
    let renomeando = signal(None::<String>);

    Form::new("Arquivos").child(
        DataGrid::bind(linhas, selecionadas)
            .id("arquivos")
            .grow(1)
            .columns([
                DataGridColumn::new("nome", "Nome"), // Star(1.0) por padrão
                DataGridColumn::new("tamanho", "Tamanho")
                    .width(ColumnWidth::Pixels(110.0))
                    .align_end(),
            ])
            .sort(ordem)
            .filter(filtro)
            .renaming(renomeando)
            .empty_text("Nenhum arquivo")
            .on_invoke(move |chave| println!("abrir {chave}"))
            .on_rename(move |chave, texto| {
                linhas.update(|v| {
                    if let Some(l) = v.iter_mut().find(|l| l.key == chave) {
                        l.cells[0] = (texto.clone(), CellValue::Text(texto));
                    }
                });
                renomeando.set(None);
            })
            .on_delete(move |chaves| linhas.update(|v| v.retain(|l| !chaves.contains(&l.key))))
            .item_context_flyout(move |_chave| {
                MenuFlyout::new().item("apagar", "Apagar", move || { /* lê `selecionadas` aqui */ })
            }),
    )
}
```

- **Células**: `DataGridRow::cell(texto, valor)`, uma por coluna, na ordem das colunas. O valor é
  `CellValue::Text` (ordem natural, sem acento nem maiúsculas: "a2" antes de "a10"), `Number`,
  `Time(Option<SystemTime>)` (`None` vem antes de qualquer data), `Progress(0.0..=1.0)` (a
  célula desenha uma barra de andamento; o texto vira o nome acessível) ou
  `Sparkline(Vec<f64>)` (uma minissérie de 0 a 1 no lugar do texto, que vira o nome acessível;
  ordena pela média). **Não há célula com widget**: só texto, o ícone na primeira coluna
  (`.icon`), o `Progress` e a `Sparkline`.
- **Calor**: `.heat(0.0..=1.0)` logo depois de um `.cell(..)` pinta o fundo daquela célula do
  amarelo-claro ao vermelho (o "calor" das colunas do Gerenciador de Tarefas); 0 não pinta. O
  texto mantém o contraste, e `DataGridRow::cell_heat(i)` lê o valor.
- **Ordem**: o clique no cabeçalho ordena (de novo, inverte); `.sort(signal)` guarda a coluna
  (a chave dela) e o `SortDirection`; `DataGridColumn::not_sortable()` desliga. `DataGridRow::rank`
  (".." 0, pastas 1, arquivos 2) fica antes da coluna e **não inverte** na ordem decrescente.
  Empate pela chave.
- **Tom da linha**: `DataGridRow::tone(RowTone::Success | Caution | Critical)` pinta o fundo da
  linha de verde, amarelo ou vermelho (as cores de fundo da `InfoBar`), para marcar o resultado
  de uma comparação. O ponteiro e a seleção aparecem por cima, o realce de soltar vence, e o
  texto mantém o contraste (o lint confere). Sem `.tone(..)`, a linha fica como sempre. Não
  depende só da cor: na inspeção, a linha traz `tone` (`success`, `caution` ou `critical`,
  também em `app.grid(..).rows[i].tone`) e o nó dela tem a descrição "destaque: sucesso",
  "destaque: atenção" ou "destaque: crítico".
- **Colunas**: `.width(ColumnWidth::Pixels(n))` ou `Star(peso)`; `.align_end()` (números). O divisor
  do cabeçalho redimensiona (o clique duplo ajusta ao texto) e a largura vale até o grid sumir.
- **Seleção**: clique seleciona uma; Ctrl soma ou tira; Shift marca a faixa; setas, Home, End,
  PageUp e PageDown navegam (com Shift marcam a faixa); Ctrl+A seleciona tudo; Enter e clique
  duplo chamam `.on_invoke(chave)`; F2 renomeia (`renomeando.set(Some(chave))` também); Delete
  chama `.on_delete(chaves)` com a seleção (ou só a linha do foco, se ela está fora da seleção);
  Esc cancela o renomear. O clique direito numa linha **fora** da seleção seleciona só ela; numa
  linha da seleção mantém tudo. O menu é por linha (`.item_context_flyout(|chave| MenuFlyout...)`):
  o item lê a seleção ao ser acionado, não ao montar o menu.
- **Os callbacks não mudam a lista sozinhos**: `.on_rename`, `.on_delete` e `.on_drop` só avisam.
- **Arrastar**: leva a seleção inteira (começa depois de 4 px). `.drag_group("nome")` deixa grids
  do mesmo grupo trocarem linhas (sem grupo, só no próprio grid). `.on_drop(|d: DataGridDrop| ...)`
  recebe `source_grid` (o id de onde saiu), `keys` (na ordem da origem) e `target` (`Some(chave)`
  se soltou sobre uma linha `.drop_target()`, `None` no fundo ou sobre um arquivo);
  `.can_drop(|d| bool)` proíbe destinos. Um `TreeView` com `.on_grid_drop("nome", ..)` recebe
  essas linhas sobre as pastas dele. Arquivos do sistema (gerenciador de arquivos do SO)
  soltos sobre a janela chegam em `.on_files_dropped(|caminhos, alvo: Option<String>| ...)`
  (Windows, macOS e X11; o Wayland do winit 0.30 não entrega).
- **Filtro**: `.filter(signal)` mostra as linhas cujo texto da primeira célula casa.
- **Densidade**: linhas e cabeçalho de 32 px (24 px com `.density(Density::Compact)`); o lint
  não cobra os 44 px de alvo de toque nelas (é lista de ponteiro, como no WinUI).
- **Muitas linhas**: só as perto da área visível existem como widgets (10.000 linhas continuam
  leves); no teste, leia `app.grid(id)`, não procure widgets de linhas fora da janela.
- **Ids**: o grid é o `.id(...)`; o cabeçalho é `{id}-cabecalho`, a rolagem `{id}-rolagem`, cada
  linha montada `{id}-{chave}` e a que está sendo renomeada tem o `Edit` `{id}-{chave}-nome`.
  O menu de cada linha repete os ids por linha: dê ids com a chave (`format!("apagar-{chave}")`).
- **No teste**: `app.grid("arquivos")` (`DataGridInfo`: `columns` com chave, título e largura,
  `rows` com chave, textos e `selected` de **todas** as linhas filtradas, `selected`, `sort`,
  `filter`), `app.grid_select(id, &["a", "b"])` (sem clicar; o foco não muda),
  `app.click_with(id, Modifiers::COMMAND)` / `Modifiers::SHIFT`, `app.grid_divider(id, coluna)`
  (o ponto do divisor, para `app.drag`), `app.drag_rows(id, &["a"], alvo_id, Some("pasta"))`
  (ou `None` para o fundo), `app.drop_files(id_ou_linha, &[caminho])`, `app.key(..)`,
  `app.right_click(..)`.

## PropertyGrid (propriedades)

`PropertyGrid::new()`: a lista de propriedades de duas colunas (nome e valor) de um objeto, como
a de um editor de conexões. Cada propriedade se liga a um `Signal` do app; a grade recebe
closures que devolvem o signal da conexão escolhida, e trocar a escolha troca os valores sem
recriar a grade.

```rust
#[derive(Clone, Copy)]
struct Conexao {
    host: Signal<String>,
    porta: Signal<f64>,
    protocolo: Signal<String>,
    usuario: Signal<String>,
    senha: Signal<String>,
    chave: Signal<String>,
    bipe: Signal<bool>,
}

fn nova(host: &str, porta: f64) -> Conexao {
    Conexao {
        host: signal(host.to_owned()),
        porta: signal(porta),
        protocolo: signal("SSH".to_owned()),
        usuario: signal(String::new()),
        senha: signal(String::new()),
        chave: signal(String::new()),
        bipe: signal(true),
    }
}

fn conexoes() -> impl View {
    let (a, b) = (nova("a.exemplo.com", 22.0), nova("b.exemplo.com", 23.0));
    let escolhida = signal("Produção".to_owned());
    let atual = move || if escolhida.get() == "Produção" { a } else { b };
    Form::new("Conexões").child(
        Row::new()
            .grow(1)
            .padding(12)
            .gap(12)
            .child(RadioButtons::bind(escolhida, ["Produção", "Legado"]).id("conexoes"))
            .child(
                PropertyGrid::new()
                    .id("props")
                    .grow(1)
                    .category("Conexão")
                    .text("Host", move || atual().host)
                    .description("Nome ou endereço do servidor.")
                    .default("")
                    .number("Porta", move || atual().porta, 1.0..=65535.0)
                    .default(22)
                    .choice("Protocolo", move || atual().protocolo, ["SSH", "Telnet"])
                    .default("SSH")
                    .category("Autenticação")
                    .text("Usuário", move || atual().usuario)
                    .password("Senha", move || atual().senha)
                    .file("Chave privada", move || atual().chave)
                    .filter("Chaves", &["pem", "key"])
                    .category("Terminal")
                    .collapsed()
                    .check("Bipe", move || atual().bipe)
                    .default(true),
            ),
    )
}
```

- **Tipos**: `.text`, `.password` (valor mascarado), `.number(nome, signal_f64, min..=max)`,
  `.choice(nome, signal, opções)`, `.check(nome, signal_bool)`, `.file(nome, signal)`,
  `.folder(nome, signal)` (como o `.file`, com o "…" que abre o seletor de **pasta**, o
  `dialogs::pick_folder()`; sem `.filter`; cancelar não muda nada; no `TestApp`,
  `app.file_dialogs()[0].kind == FileDialogKind::Folder`) e `.button(nome, resumo, || ..)`. O valor
  é um `Signal<String>`, `Signal<f64>` ou `Signal<bool>`, ou uma closure que o devolve. Para
  uma lista que muda com o app, `.choice_from(nome, signal, || -> Vec<String>)`: a closure pode
  ler signals e o editor da linha se refaz quando a lista muda.
- **Coleção (`.button`)**: `.button(nome, resumo, on_click)` é uma propriedade só de leitura com
  um botão "…" (`{id}-{nome}-botao`, só na linha escolhida) que chama `on_click`; o Enter, o F2 e
  o espaço também. `resumo` é um `String`, closure ou `Signal<String>` ("2 túneis"); a edição
  fica num diálogo seu, como o editor "(Coleção)" do Visual Studio. `.description`, `.disabled`
  e `.category` valem; `.default` não.
- **Ordem**: `.category("nome")` abre uma categoria e as propriedades seguintes entram nela
  (`.collapsed()` começa fechada); as declaradas antes da primeira categoria ficam no topo, sem
  cabeçalho. O botão da barra de cima passa para a ordem alfabética, sem categorias.
- **Modificadores** valem para a última propriedade declarada: `.description("...")` (o painel
  de baixo), `.default(valor)` (o valor aparece em **negrito** enquanto difere dele; o tipo
  precisa combinar: `&str`, `f64`/`i32` ou `bool`), `.disabled(bool | closure | signal)` (sem
  foco, as setas pulam), `.read_only()` (sem editor, valor em cinza; a linha ainda escolhe) e
  `.filter("Chaves", &["pem", "key"])` (só na `.file`). `.collapsed()` vale para a categoria.
- **Esconder e padrão calculado**: `.hidden(bool | closure | signal)` esconde a linha (depois de
  uma `.category`, a categoria inteira, com o cabeçalho). A escondida sai das linhas: sem Tab,
  as setas passam por ela, o leitor de tela não a lê, e a categoria sem nenhuma linha visível
  perde o cabeçalho sozinha. **Nada se apaga**: o valor fica no signal do app e volta com a
  linha. Regra: **esconda o que nunca vale; desabilite (`.disabled`) o que depende de outro
  campo**. `.default_with(|| valor)` é o `.default` calculado (lido de forma reativa): a porta
  padrão que muda com o tipo da conexão e não deixa o valor em negrito. No `TestApp`,
  `app.properties("props")` lista todas as propriedades, com `hidden`.
- **Editores**: só a linha escolhida tem editor (a `.check` fica à vista em toda linha). O
  texto, a senha, o número e a `.file` são um **rascunho**: o signal do app só muda ao
  confirmar (Enter, Tab ou perder o foco); o Esc desfaz. `.choice` e `.check` gravam direto.
  O nome da propriedade é fixo (uma `String`, não uma closure).
- **Teclado**: ↑/↓ andam, Home/End, ← fecha a categoria (ou sobe ao cabeçalho), → abre. Enter
  ou F2 entram no editor (na `.check` invertem; na `.choice` abrem a lista, como F4 e Alt+↓);
  o Esc volta o foco à linha. Digitar numa linha de texto, senha ou arquivo abre o editor com
  o texto; na `.number`, só Enter, F2 ou o clique abrem.
- **Divisores**: arrastar a linha entre nome e valor muda a largura do nome; arrastar a faixa
  acima da descrição muda a altura do painel. Só com o ponteiro.
- **Densidade**: a grade vem compacta (linhas de 24 px); `.density(Density::Standard)` dá 44 px.
- **Ids**: a grade é o `.id(...)`; a linha é `{id}-{nome}` (`props-Porta`), o editor
  `{id}-{nome}-valor` (`props-Porta-valor`; na `.choice`, a lista é `{id}-{nome}-valor-lista` e
  as opções `{id}-{nome}-valor-{opção}` em minúsculas), o botão "…" `{id}-{nome}-procurar` (na `.button`, `{id}-{nome}-botao`), o
  cabeçalho `{id}-categoria-{nome}`, a barra `{id}-barra`, o botão da ordem `{id}-alfabetica`,
  o divisor `{id}-divisor` e o painel `{id}-descricao`.
- **No teste**: `app.properties("props")` devolve todas as propriedades, na ordem da
  declaração (`category`, `name`, `value` com a senha mascarada e a `.check` como `true` ou
  `false`, `changed`, `selected`, `read_only`, `enabled`, `hidden`), mesmo as das categorias fechadas.
  Escolha a linha clicando no **nome** (`app.click_at(ponto, 1)` à esquerda; o centro da linha
  é o divisor das colunas), digite com `app.type_text(..)` e confirme com `app.key(Key::Enter)`.

### Seletor de arquivo

O "…" da `.file` chama o seletor do sistema. Para um botão seu, use `tomni_ui::dialogs`:

```rust
use tomni_ui::dialogs; // não vem no `prelude`

Button::new("Escolher…").on_click(move || {
    dialogs::open_file()                       // ou `dialogs::save_file()` / `dialogs::pick_folder()`
        .title("Chave privada")
        .filter("Chaves", &["pem", "key"])     // extensões sem o ponto
        .directory("/home")                    // onde abre (opcional)
        .show(move |caminho| {                 // `Option<PathBuf>`: None se cancelou
            if let Some(caminho) = caminho {
                chave.set(caminho.to_string_lossy().into_owned());
            }
        });
})
```

- `dialogs::open_files()` com `.show_many(move |caminhos: Vec<PathBuf>| ..)` deixa marcar
  **vários** arquivos (vazio se cancelou); o `.show(..)` de qualquer diálogo devolve só o primeiro.
- `.remember("chaves")` abre na pasta da última resposta com o mesmo nome (como o
  `SettingsIdentifier` do WinUI), no lugar do `.directory(..)`, que fica valendo enquanto não há
  pasta lembrada. A lib guarda na memória; para lembrar entre execuções, o app instala um
  `dialogs::FolderMemory` com `dialogs::set_folder_memory(..)`.
- `dialogs::pick_folder()` escolhe uma **pasta** que existe (`.title(..)` e `.directory(..)`
  valem; os filtros e o nome sugerido não).
- O callback do `.show(..)` roda na thread da interface, **no frame seguinte**, não dentro do
  clique.
- `dialogs::is_supported()` é `false` no Android: o pedido é respondido com `None`. Deixe a
  pessoa digitar o caminho (a `.file` já esconde o "…" e fica só com o campo).
- No teste não abre janela: `app.file_dialogs()` lista os pedidos (`title`, `filters`,
  `directory`) e `app.file_dialog_answer(Some("/tmp/a.pem"))` responde o mais antigo (`None`
  cancela); `app.file_dialog_answer_many(["/a/1", "/a/2"])` responde com vários. Uma resposta
  dada antes do pedido vale para o próximo.

### Copiar para o clipboard

O `on_click` de um botão não tem acesso ao clipboard. Use `tomni_ui::clipboard::write`:

```rust
use tomni_ui::clipboard; // não vem no `prelude`

Button::new("Copiar").on_click(move || clipboard::write(texto.get_untracked()))
```

O texto chega ao clipboard da janela no quadro seguinte. No teste, `app.frame()` entrega e
`app.clipboard().read()` devolve o texto.

Para ler, `clipboard::read(move |texto: Option<String>| ...)` chama o callback no quadro
seguinte (nunca dentro da chamada). `clipboard::clear_if("senha")` apaga o clipboard no quadro
seguinte **só se** ele ainda tem exatamente esse texto: é o "limpar depois de copiar uma
senha" sem apagar o que a pessoa copiou depois.

**Imagem e arquivos**: `clipboard::with(move |clip: &mut dyn Clipboard| ..)` entrega, no
quadro seguinte, a área de transferência inteira. O `Clipboard` tem, além de `read` e `write`
do texto, `read_files() -> Option<Vec<PathBuf>>` (arquivos copiados no gerenciador),
`read_image() -> Option<ClipboardImage>`, `write_files(Vec<PathBuf>)` e
`write_image(ClipboardImage)`. `ClipboardImage { width, height, rgba }` guarda os pixels RGBA
(sem alfa pré-multiplicado) e tem `to_png() -> Result<Vec<u8>, String>`. Quem cola num
`TextEditor` não precisa disso: use `.on_paste_files` e `.on_paste_image` (veja "Editor de texto
e Markdown"). No teste, `app.clipboard()` é o mesmo trait, e `write_files` / `write_image`
preparam o que o Ctrl+V vai colar.

## Popups

`ComboBox`, `MenuFlyout`, `Flyout`, `ContentDialog`, a lista do `AutoSuggestBox` e a dica de
`.tooltip()` abrem numa camada acima da tela:

- **Não ocupam espaço no layout.** O `MenuFlyout` e o `Flyout` entram no botão com
  `.flyout(...)`; o `ContentDialog` vai em qualquer lugar dentro do contêiner da tela (o último
  filho da `Column`, por exemplo; o `Form` aceita só um filho).
- Abrem junto de quem os abre (embaixo, ou em cima se não couber; a dica, em cima) e o dialog,
  no centro. Esc ou
  um clique fora fecham. **O clique fora só fecha**: o widget que está atrás não recebe.
- Ao abrir, o foco vai para dentro do popup; ao fechar, volta para quem abriu. A exceção é a
  lista do `AutoSuggestBox`: o foco fica no campo, para continuar digitando.

**`ComboBox`**: 32 px de altura, como o WinUI; use `.height(44)`. Clique, Enter, Espaço ou
Alt+↓ abrem a lista; ↑ e ↓ andam, Enter ou clique escolhem. Fechado e com foco, ↑ e ↓ trocam o
valor direto.

```rust
let uf = signal(String::new());
ComboBox::bind(uf, ["MG", "RJ", "SP"])
    .id("uf")
    .placeholder("UF")
    .height(44)
```

Cada item tem o id `{id}-{opção}`, com a opção em minúsculas e o que não é letra ou número
trocado por `-` (`uf-sp`, `cidade-são-paulo`); a lista é `{id}-lista`.

**`MenuFlyout`**: cada item tem o id e a ação que você dá; escolher roda a ação e fecha o menu.
Um item pode abrir um `ContentDialog` (`move || confirmar.set(true)`).

```rust
Button::new("Mais opções").id("mais").height(44).flyout(
    MenuFlyout::new()
        .id("menu")
        .item("duplicar", "Duplicar", move || copias.update(|n| *n += 1))
        .separator()
        .item("arquivar", "Arquivar", move || arquivar.set(true)),
)
```

Com o menu aberto, a **tecla de acesso** (`.access_key('1')` depois do item; maiúscula e
minúscula valem igual) aciona o item como o clique: roda a ação e fecha, ou abre o submenu.
`.highlighted(signal)` grava no `Signal<Option<String>>` o id do item que o ponteiro ou o teclado
está prestes a escolher (o mais fundo, olhando os submenus abertos; `None` com o menu fechado).
Serve para o resto da tela reagir ao realce, por exemplo `TerminalPanes::highlight_pane`.

**Sobreposição** (`Placement::Over`, para quem escreve um widget): uma camada com
`input: false` e `pointer: false` e `placement: Placement::Over` fica **no centro** do pai de
layout, pintada acima do conteúdo e abaixo de menus e diálogos, e o ponteiro e o foco passam
por ela. Se a camada (mais o `gap` de cada lado) não couber no pai, não aparece.

**`Flyout`**: fecha sozinho com Esc ou um clique fora. Para fechar por um botão de dentro, dê a
ele um signal seu com `.open(signal)` e ponha `false` nele:

```rust
let ajuda = signal(false);
Button::new("Ajuda").id("ajuda").height(44).flyout(
    Flyout::new(
        Column::new()
            .gap(12)
            .child(Text::new("A senha precisa de 8 caracteres.").wrap())
            .child(Button::new("Entendi").id("entendi").height(44).on_click(move || ajuda.set(false))),
    )
    .id("ajuda-popup")
    .open(ajuda),
)
```

**`ContentDialog`**: abre quando o signal vira `true` e escurece o resto da janela, que não
recebe nada enquanto ele estiver aberto (o Tab dá a volta dentro dele). Cada botão roda a ação
dele e fecha; Enter faz o mesmo que o primário e Esc fecha sem ação. Os botões têm os ids
`{id}-primary`, `{id}-secondary` e `{id}-close`. Uma terceira escolha vai em
`.close_with(rotulo, move || ...)` (o botão de fechar com ação; o Esc segue fechando sem ação), que
`.close_visible(move || bool)` esconde quando não cabe (o secundário tem `.secondary_visible(move || bool)`). A ação que só acontece com confirmação fica no
`.primary`:

```rust
let confirmar = signal(false);
let arquivado = signal(false);
// ... dentro da Column da tela:
ContentDialog::new("Arquivar projeto?", confirmar)
    .id("arquivar-dialogo")
    .content(Text::new("O projeto sai da lista, mas não é apagado.").wrap())
    .primary("Arquivar", move || arquivado.set(true))
    .close("Cancelar")
```

**Dica** (`.tooltip("Copia o link")`): abre depois de meio segundo com o mouse parado em cima, ou
na hora quando o foco chega pelo Tab; fecha quando o mouse ou o foco saem, com Esc ou com um
clique. Tem o id `{id}-tooltip`. Explique com ela, mas não esconda nela o único jeito de saber o
que um botão faz: no toque, não há hover.

Popups abertos entram no lint (contraste, alvo de toque, texto cortado). O `tomni-ui lint` vê
a tela no estado inicial, com tudo fechado: abra cada popup num teste e chame `assert_lint()`.

## Desktop: divisor, menus e atalhos

Para apps de desktop (veja também "Densidade compacta"). Os exemplos `split` e `menu_bar` de
`crates/tomni-ui/examples` mostram tudo junto.

**`Split`**: dois painéis com um divisor de 8 px entre eles. `Split::row(a, b)` põe `a` à
esquerda e `Split::column(a, b)` põe `a` em cima. O tamanho do primeiro painel fica num
`Signal<f64>` seu (`.size(signal)`, padrão 240 px): arrastar o divisor ou usar as setas com o
foco nele (8 px por vez) muda o signal, e mudar o signal move o divisor. O segundo painel fica
com o resto. `.min_first(px)` e `.min_second(px)` limitam os painéis. Um `Split` dentro de
outro faz uma grade. Os painéis esticam o conteúdo; ponha um `ScrollViewer` (com `.id`) no
que pode ficar pequeno. Ids: `{id}-primeiro`, `{id}-divisor`, `{id}-segundo`.

```rust
let lateral = signal(220.0);
Split::row(
    ScrollViewer::new().id("servidores").child(lista),
    Text::new("Conteúdo"),
)
.id("lados")
.size(lateral)
.min_first(160.0)
.min_second(320.0)
```

**`MenuBar`**: a barra no alto da janela, com um `MenuFlyout` por item. Cada item tem o id
`{id}-{título}` (`barra-arquivo`; o título entra em minúsculas, com `-` no lugar do que não é
letra ou número). Clicar abre o menu; com um aberto, passar o mouse sobre outro item troca de
menu; ←/→ andam entre os itens, ↓ abre, F10 ou Alt levam o foco à barra e Esc devolve.
`.access_key('A')` (logo depois do `.menu`) faz o Alt+A abrir aquele menu. Os itens do menu têm
os ids que você deu; um submenu (`.submenu(id, rotulo, MenuFlyout)`) abre ao passar o mouse ou
com →, e o `MenuFlyout` dele tem o id próprio (`exibir-menu`).

```rust
MenuBar::new()
    .id("barra")
    .menu(
        "Arquivo",
        MenuFlyout::new()
            .id("arquivo")
            .item("abrir", "Abrir...", move || abrir())
            .icon(Symbol::FolderOpen)
            .shortcut(Shortcut::ctrl('O'))
            .submenu(
                "exibir",
                "Exibir",
                MenuFlyout::new()
                    .id("exibir-menu")
                    .toggle("quebra", "Quebra de linha", quebra),
            )
            .separator()
            .item("sair", "Sair", move || sair()),
    )
    .access_key('A')
```

`.toggle(id, rotulo, signal_bool)` é um item com a marca ✓ que liga e desliga o signal.
`.radio(id, rotulo, grupo: Signal<T>, valor)` é um item de rádio (`RadioMenuFlyoutItem`): fica
marcado, com um ponto, quando `grupo` vale `valor`; escolher faz `grupo.set(valor)` e fecha o
menu, e escolher o que já está marcado só fecha. Os rádios que dividem o mesmo `Signal` formam o
grupo, e o `T` é qualquer tipo `Clone + PartialEq + Send + Sync` (um enum, por exemplo). Na
inspeção o tipo é `radio_menu_item`, com `checked`/`unchecked` como o `toggle_menu_item`, e no
`TestApp` `app.checked("id")` lê os dois. Enter e espaço escolhem o item com foco.
`.icon(...)` e `.shortcut(...)` valem para o item logo antes deles.

**Atalhos** (`Shortcut`, do prelude): `Shortcut::ctrl('N')`, `Shortcut::ctrl_shift('W')`,
`Shortcut::alt('A')`, `Shortcut::key(Key::F(5))` (sem modificador; `Key::F(n)` são as teclas
de função). As letras valem com ou sem Caps Lock; o Shift conta. Há duas formas de dar um
atalho (e `"Ctrl+Shift+T".parse::<Shortcut>()` lê o texto que o `Display` escreve, com o erro
`tecla desconhecida: Ctrl+Xyz`; serve para atalhos que a pessoa digita nas configurações):

- `.shortcut(Shortcut)` num item de menu: aparece à direita do item e roda a ação do item
  **com o menu fechado**. Não precisa repetir a ação. Aceita também um `Signal<Shortcut>` ou
  uma closure: a tecla é lida na hora em que se pressiona, então trocar o signal troca o
  atalho (e o texto do item) sem montar a tela de novo. `Shortcut::NONE` é "sem atalho": nunca
  dispara e o item não mostra texto à direita (use quando a ação não tem atalho de fábrica;
  `is_none()` pergunta; o lint `duplicate-accelerator` ignora). A tecla que a lib não conhece (`Key::Other`) também nunca dispara.
- `.accelerator(Shortcut, move || ...)` em qualquer widget (um `Button`, por exemplo): vale na
  janela toda enquanto o widget aparece e está habilitado. O widget com foco trata a tecla
  antes (o `Edit` fica com o Ctrl+C e o Ctrl+V), menos o `Terminal`, que deixa os atalhos do
  app passarem. Com um `ContentDialog` aberto, só valem os de dentro dele.
  O atalho também pode ser um `Signal<Shortcut>` ou uma closure, lidos na hora da tecla: trocar
  o signal troca o atalho sem montar a tela de novo (é como a página de configurações do app
  deixa editar os atalhos; um texto vira atalho com `"Ctrl+Shift+T".parse::<Shortcut>()`).

Dois widgets com o mesmo atalho valendo ao mesmo tempo são um erro do lint
(`duplicate-accelerator`): troque um dos dois.

**Menu de contexto**: `.context_flyout(MenuFlyout)` em qualquer widget (um `Text`, uma
`Column`, uma lista) abre o menu com o botão direito, na posição do mouse, e com Shift+F10, a
tecla Menu ou o toque longo, junto do widget. Vale o `.context_flyout` do widget mais fundo
sob o mouse que tenha um; os itens com `.shortcut()` valem como atalhos mesmo com o menu
fechado. O `Terminal` sem menu de contexto cola com o botão direito, como antes. Nas abas:
`TabView::tab_context_flyout(|chave| MenuFlyout::new()...)` dá um menu a cada aba (os ids dos
menus ficam por sua conta; use `format!("menu-{chave}")`).

**Arrastar e soltar** (dentro da janela): só o `TabView` arrasta por enquanto. Com
`.on_reorder(move |de, para| ...)`, uma aba arrasta depois de andar 4 px (antes disso é um
clique); uma linha de 2 px na cor de destaque mostra onde ela vai cair, uma etiqueta com o
título segue o mouse, Esc cancela e soltar fora da faixa não faz nada. No toque, o arrasto
começa ao segurar o dedo. O `on_reorder` só avisa: mova o item na sua lista (`de` e `para`
são índices da ordem atual, `para` já descontado da posição que a aba deixa).

## Painéis encaixáveis (`DockHost`)

Para apps de desktop com painéis nas bordas da janela (a lista de conexões, as propriedades, o
registro): o exemplo `dock` de `crates/tomni-ui/examples` mostra tudo junto.

```rust
let layout = signal(DockLayout::default());          // o app guarda e devolve (ver abaixo)
let controller = DockController::new();
let host = DockHost::new(conteudo)                   // o centro, entre os painéis
    .id("dock")
    .layout(layout)
    .controller(&controller)
    .min_content(240.0)                              // o menor tamanho do centro (padrão 200)
    .pane(
        DockPane::new("arvore", Symbol::Folder, "Árvore", move || minha_arvore(selecionado))
            .size(240.0),                            // lado esquerdo e fixado, por padrão
    )
    .pane(
        DockPane::new("saida", Symbol::Document, "Saída", || saida())
            .side(DockSide::Bottom)
            .mode(DockMode::AutoHide)
            .size(140.0),
    );
```

**Modos** (`DockMode`): `Pinned` ocupa espaço ao lado do conteúdo; `AutoHide` vira um botão
(ícone e dica com o título) numa faixa da borda, e o painel abre **por cima** do conteúdo sem
mexer nele; `Floating` abre numa janela própria; `Closed` esconde. O alfinete do cabeçalho
alterna `Pinned` e `AutoHide`; o ícone de janela flutua; o X fecha; o duplo clique no título
maximiza (Esc restaura); o botão direito no cabeçalho abre o menu (alfinete, flutuar, lado,
maximizar, fechar). No auto-ocultar, parar o ponteiro 400 ms no botão da faixa (ou clicar, ou
Enter) abre o painel; sair dele (ponteiro e foco) por 400 ms, ou Esc, recolhe e devolve o foco
ao botão. As constantes são `DOCK_HOVER_DELAY`, `DOCK_LEAVE_DELAY`, `DOCK_STRIP_WIDTH` (48),
`DOCK_STRIP_HEIGHT` (32), `DOCK_PANE_SIZE` (320) e `DOCK_HEADER_HEIGHT` (32).

**O estado é seu.** O `Signal<DockLayout>` guarda o modo, o lado, o peso, o tamanho do painel
aberto por cima e a geometria da janela de cada painel (`layout.pane("chave")`), mais o
tamanho e a ordem de cada área (`layout.area(DockSide::Left)`) e o painel maximizado. Um
`DockLayout::default()` vale: o `DockHost` o preenche com o padrão dos `DockPane`. Chave gravada
que não existe mais é ignorada; painel novo entra com o padrão. O tipo não tem serde (a lib
não depende dele): converta para o seu formato ao gravar e volte ao abrir.

**O conteúdo é montado de novo** quando o painel muda de lugar (fixado para auto-ocultar,
outro lado, flutuar, maximizar): o construtor é `Fn() -> View`. Guarde o estado da tela em
signals de fora (a seleção da árvore, a nota aberta), como no exemplo. Recolher e reabrir um
auto-ocultar não monta de novo; fechar desmonta.

**`DockController`** liga menus e atalhos: `show`, `hide`, `toggle` (aberto fecha, fechado ou
recolhido mostra), `pin`, `unpin`, `float`, `dock`, `close`, `move_to(chave, lado)`,
`maximize`, `restore`, `mode`, `is_open` e `shown(chave)`, um `Signal<bool>` ("não está
fechado") que vai direto no `MenuFlyout::toggle`. `.closable(false)` e `.floatable(false)` do
`DockPane` só tiram o botão e o item de menu; o controlador continua fazendo. Os efeitos que
ligam o `shown` ao layout rodam no quadro seguinte (nos testes, `app.frame()`).
`.header_actions(|| ...)` põe botões seus no cabeçalho.

**Ids:** `{id}-{chave}` (o painel), `-cabecalho`, `-titulo`, `-corpo`, `-flutuar`, `-alfinete`,
`-fechar`, `-faixa` (o botão da faixa), `-borda` (o divisor do painel aberto por cima),
`-divisor` (entre dois painéis do mesmo lado), `{id}-{esquerda|direita|embaixo}-divisor` (o da
área), `{id}-conteudo` e `{id}-{chave}-menu-{alfinete|flutuar|esquerda|direita|embaixo|maximizar|fechar}`.
Os divisores se arrastam e andam com as setas, como o do `Split`. Com a janela pequena, o
centro encolhe até o `min_content` e só depois as áreas dividem o que sobra.

**Lint:** use `Form::density(Density::Compact)` (o cabeçalho e a faixa têm 32 px, abaixo dos
44 px do toque). A janela secundária de um app não deve criar um `DockHost` (as janelas
flutuantes pertencem ao host; elas não fecham sozinhas se a tela some).

## Terminal

Um console de texto (VT100, VT220 e xterm: cores, `vim`, `htop`), ligado
a uma conexão. Fica atrás de features do `tomni-ui` no `Cargo.toml` do app:
`features = ["terminal"]` dá o widget; some `"pty"` (`Process`), `"ssh"` (`Ssh`) ou `"serial"`
(`Serial`) para as conexões de verdade. O `Telnet` e o `Loopback` vêm com o `terminal`.

```rust
use tomni_ui::prelude::*;
use tomni_ui::terminal::Process; // e Telnet, Ssh, Serial

pub fn console() -> impl View {
    let titulo = signal(String::from("shell"));
    Form::new("Console").child(
        Column::new().gap(8).padding(16)
            .child(Text::new(move || titulo.get()).id("titulo"))
            .child(
                Terminal::new(Process::shell())          // o shell do usuário
                    .id("console")
                    .grow(1)                             // a grade acompanha o tamanho
                    .on_title(move |t| titulo.set(t)),   // o título que o programa escolhe
            ),
    )
}
```

| Conexão | Como criar |
|---|---|
| `Process` (`pty`; fora do Android) | `Process::shell()` ou `Process::builder("htop").arg("-d").arg("10").cwd(pasta).env("CHAVE", "valor").spawn()`; manda `TERM=xterm-256color`; solto, o processo recebe SIGHUP |
| `Telnet` | `Telnet::connect("roteador.local", 23)` (negocia eco, tamanho e tipo do terminal; o Enter vai como CR LF); `Telnet::builder("roteador.local", 23).proxy(Proxy::Socks5 { .. }).before_connect(..).connect()` passa por um proxy e roda um gancho antes (o `connect` de cima é o builder sem opções) |
| `Ssh` (`ssh`) | `Ssh::builder("servidor", 22, "ana").password(senha).key_file(home.join(".ssh/id_ed25519"), None).connect()`; veja abaixo, com o salto (`.jump`), o proxy (`.proxy`), o agente (`.agent_auth`, `.agent_forward`) e o keepalive |
| `Serial` (`serial`) | `Serial::open("/dev/ttyUSB0", 115_200)` ou `Serial::builder("COM3").baud_rate(9600).flow_control(FlowControl::None).open()` (`tomni_ui::terminal::serial::{FlowControl, Parity, StopBits, CharSize}`); padrão: 9600 8N1, XON/XOFF |
| `Loopback` | `Loopback::new()` (`.echo()` devolve o que se digita), para testes e demonstrações: `.handle()` escreve como o programa e lê o que o usuário digitou; `.running(cmd)` guarda o comando que um transporte de verdade rodaria, lido em `handle.command()` (a cópia do `duplicate_running` de um `Loopback` guarda o comando novo) |

Nenhum construtor falha nem bloqueia: a conexão roda numa thread, e um erro ("servidor não
encontrado: x", "não foi possível abrir /dev/ttyUSB0: permissão negada", "autenticação
recusada para ana", "o processo saiu com o código 1") aparece no próprio terminal, que fica
fechado. A senha do SSH é do app: peça num `Edit` com `.password()` antes de conectar, nunca
no terminal. A chave do servidor é conferida no `~/.ssh/known_hosts` (ou
`.known_hosts(caminho)`); servidor desconhecido é recusado, a não ser que o app pergunte:
`.check_host_key(move |chave, situacao| ...)` recebe a `HostKey` (`host()`, `algorithm()`,
`fingerprint()` como "SHA256:…", `remember()` grava no `known_hosts`) e um `KnownHost`
(`Known`, `Unknown` ou `Changed { line }`), e devolve se aceita. Ela roda na thread da
conexão: para perguntar ao usuário, mande a pergunta à tela por um canal e espere a resposta.
Opções do `Ssh` além de usuário, senha e chave (todas no builder, nada bloqueia):
`.jump(Ssh::builder("bastiao", 22, "ana").password(..))` passa por um servidor de salto (a
cadeia pode ter vários, cada um com a própria autenticação e a própria conferência de chave;
um erro no meio do caminho aparece com o `host:porta` do salto); `.proxy(Proxy::Socks5 { host,
port, user, password })` ou `Proxy::Http { .. }` (HTTP CONNECT) chega ao primeiro servidor por
um proxy; `.agent_auth()` tenta as chaves do ssh-agent (o `SSH_AUTH_SOCK`; `.agent_path(caminho)`
escolhe outro socket), na ordem do OpenSSH: a cópia no agente das chaves de `.key_file`/`.key`
primeiro, as outras do agente depois delas (o servidor derruba quem erra mais que o `MaxAuthTries`); `.agent_forward(true)` deixa o servidor usar o agente local;
`.keepalive(Duration::from_secs(30))` (30 s por padrão) derruba a conexão que parou de
responder; `.term("xterm-256color")` muda o tipo do terminal;
`.before_connect(|aviso| { aviso("Subindo a rede…"); Ok(()) })` roda na thread da conexão antes
de abrir o TCP e antes do proxy (os `aviso(..)` aparecem no terminal; um `Err(mensagem)` fecha a
sessão com a mensagem, como um erro de conexão; roda de novo a cada reconexão, então precisa
poder repetir; só vale o do servidor de destino, não o dos saltos). É o ponto para subir uma
VPN ou um contêiner que o proxy usa: dois destinos com o mesmo IP em redes diferentes saem por
dois proxies, cada um com o seu `known_hosts`. O `Telnet::builder` tem o `.proxy` e o
`.before_connect` iguais. O `Proxy` mostra a senha como `…` no `Debug`. Não há redirecionamento
de portas nem X11.

**Proxy HTTP e SOCKS5 local** (`tomni_ui::net::HttpProxyServer`, sempre presente).
`HttpProxyServer::start(|host, porta| tomni_ui::net::connect(host, porta, Some(&proxy),
CONNECT_TIMEOUT))?` ouve numa porta livre do `127.0.0.1` e leva cada pedido pela função de
discagem que o app dá (direto, por um `Proxy`, pela ponte SOCKS de uma VPN embutida): o nome
chega sem resolver, para ser resolvido do lado de lá. Entende o `CONNECT` (HTTPS, WebSockets) e
o pedido em forma absoluta do HTTP puro, que segue em forma de origem, sem os cabeçalhos do
proxy e com `Connection: close`. `url()` dá `http://127.0.0.1:porta`, para o `--proxy-server`
do Chromium ou o `network.proxy.http` do Firefox; `addr()`, `connections()`. Uma discagem que
falha vira `502` com a mensagem em português; outro esquema, `501`. Na mesma porta, fala o
SOCKS5 (sem autenticação, só o `CONNECT`, o nome segue sem resolver; o primeiro byte `0x05`
decide): `socks_url()` dá `socks5h://127.0.0.1:porta`, para o `ALL_PROXY` de um shell; a
discagem que falha vira o código do SOCKS (recusada, inalcançável, falha geral). Não pede
usuário e senha (os navegadores não aceitam a credencial na linha de comando). Para quando é
solto ou com `stop()`, e derruba os túneis abertos.

**Porta encaminhada** (`tomni_ui::net::PortForward`): o `ssh -L` sem o SSH, para os programas
que não sabem usar proxy. `PortForward::bind("127.0.0.1:13389", "10.0.0.5", 3389, discagem)?`
leva toda conexão da porta ao mesmo destino pela discagem do app; `127.0.0.1:0` escolhe uma
livre (`addr()` diz qual). A discagem que falha fecha a conexão do cliente. `connections()`;
`stop()` ou soltar derruba as conexões.

**Rede Nebula embutida** (crate `tomni-ui-nebula`, sem placa virtual e sem administrador; só o
tráfego do app entra na rede, o sistema não vê nada). `NebulaBuilder::new(Config::from_yaml(yaml,
Some(pasta_dos_certificados))?.0).on_state(|estado| ..).start().await?` sobe a rede; o config é o
do Nebula (`pki`, `static_host_map`, `lighthouse`, `punchy`, `cipher`, o `firewall` é ignorado: o que entra é filtrado pela pilha, e um `firewall.outbound` restritivo só gera um `Warning`),
com `listen.port: 0` para uma porta qualquer. `Config::check(agora)` lista os `Problem`s (a CA
não assinou, o certificado venceu). A `Nebula` devolve `proxy()`, um `Proxy::Socks5` em `127.0.0.1`
com porta e credencial aleatórias: passe-o ao `.proxy(..)` do `Ssh::builder`, do `Telnet::builder`
e do `Sftp`, ou ao `Proxy::open`; mostre-o na interface nunca (nem porta nem credencial) e não o
exponha a outros programas. `state()` é `Connecting`, `Connected { address, lighthouses, tunnels }`,
`Retrying(msg)`, `Failed(msg)` ou `Stopped`; `wait_connected(limite)` espera; `tunnels()` lista
os pares; `stop().await` avisa os pares com `CloseTunnel`; `close()` pede o mesmo sem esperar e sem consumir a `Nebula` (quando ela está num `Arc` dividido com as conexões, o estado vai a `Stopped`). Certificados v1 e v2, `aes` e
`chachapoly`. Os relays do Nebula funcionam: com `relay.relays` no config, o nó aceita ser
alcançado pelos relays listados (útil atrás de CGNAT, onde o punch não passa), e com
`relay.use_relays` (padrão `true`) chega aos pares pelos relays que eles anunciam; o
`TunnelInfo::relay` diz por qual relay o túnel passa quando `remote` é `None`. O app não faz papel
de relay (`relay.am_relay` vira um `Warning`). Sem P-256 por enquanto (um certificado P-256 é
lido, mas usá-lo dá "certificado P-256 ainda não é suportado"). Nos testes, a feature `test-util` dá o `SimNet`
(UDP e NAT em memória) e `NebulaBuilder::start_on(..)`.

**WireGuard embutido** (crate `tomni-ui-wireguard`, sem placa virtual, sem administrador e sem o
`wg`; só o tráfego do app entra na rede). `WgConfig::from_conf(texto)?` lê o `.conf` do
`wg-quick` e devolve `(config, avisos)`: `Table`, `PostUp`, `ListenPort` e o que o app não usa
viram avisos para mostrar ao usuário; `config.without_secrets()` é a versão para a tela (sem
`PrivateKey` nem `PresharedKey`; o `Debug` da `WgConfig` também esconde). `WireGuardBuilder::new(config)
.on_state(|estado| ..).start().await?` sobe o túnel (uma tarefa no runtime do `tokio`); a
`WireGuard` devolve `proxy()`, um `Proxy::Socks5` em `127.0.0.1` com porta e credencial
aleatórias: passe-o ao `.proxy(..)` do `Ssh::builder`, do `Telnet::builder` e do `Sftp`; nunca
mostre a porta nem a credencial e não o exponha a outros programas. `state()` é `Connecting`,
`Connected { address, peers_up, peers }`, `Retrying(msg)`, `Failed(msg)` ou `Stopped`;
`wait_connected(limite)` espera o primeiro handshake; `peers()` lista os pares; `stop(espera)` (ou `close()`, que não espera nem consome o motor, para quando ele está num `Arc` dividido).
Os `AllowedIPs` mandam: um destino fora deles falha na hora com "… não está nos AllowedIPs de
nenhum par". A pilha é só IPv4 por enquanto (um `Endpoint` só com IPv6 dá erro), e o `DNS` do
`.conf` é lido, mas ainda não resolve nomes dentro do túnel.
Nos testes, `WireGuardBuilder::start_on(..)` recebe o `Underlay` simulado do netstack
(`SimNet`, com NAT, na feature `test-util`).

**Tailscale e Headscale embutidos** (crate `tomni-ui-tailscale`, sem `tailscaled`, sem placa
virtual e sem administrador; o app vira uma máquina da tailnet só para o próprio tráfego).
`TailscaleBuilder::new(url_do_controle, NodeState::default())` com `.hostname(..)`,
`.auth_key(Zeroizing<String>)` (sem chave de autenticação o `State` vira `NeedsLogin { url }`:
mostre a URL e o motor espera o login sozinho), `.ephemeral(bool)`, `.accept_routes(bool)`
(sub-redes anunciadas por outras máquinas, desligado por padrão), `.extra_roots(pem)` (a CA de um
Headscale com certificado próprio), `.udp(false)` (só pelo DERP), `.on_state(f)` e
`.on_node_state(f)`; `.start().await?` devolve a `Tailscale`. O `NodeState` (chaves da máquina,
nome, endereços e domínio) chega por `on_node_state` sempre que muda: grave-o **cifrado** e passe-o
na próxima vez, senão cada partida vira uma máquina nova na tailnet. `proxy()` é o `Proxy::Socks5`
em `127.0.0.1` (porta e credencial aleatórias, nunca na interface) e resolve IPs da tailnet,
sub-redes aceitas e nomes do MagicDNS (`servidor`, `servidor.minha-rede.ts.net`); um nome que
não existe falha na hora. `State` é `Connecting`, `NeedsLogin { url }`, `NeedsApproval`,
`Connected { address, tailnet, peers_online, derp_home }`, `Expired`, `Retrying(msg)`,
`Failed(msg)` ou `Stopped`; `peers()` lista as máquinas com `online` e o caminho (`direto (ip:porta)`
ou `DERP (região)`); `wait_connected(limite)`; `logout()` pede ao controle que a chave vença;
`stop(espera)` (ou `close()`, que não espera nem consome o motor, para quando ele está num `Arc` dividido). O nó fica sempre em ShieldsUp: não atende nada que chegue pela tailnet, não anuncia
rotas e não é saída. Uma máquina desligada, uma chave vencida e um IP fora da tailnet falham
na hora com mensagem em português. Nos testes, a feature `test-util` dá o `ControlFake`
(`testing`, no modo tailnet), o `DerpFake` e `spawn_stun`.

**ZeroTier embutido** (crate `tomni-ui-zerotier`, sem `zerotier-one`, sem placa virtual e sem
administrador; o app vira um membro da rede só para o próprio tráfego, e convive com o ZeroTier
do sistema). `ZeroTierBuilder::new(NetworkId, NodeState::default())` com `.allow_global(bool)`,
`.allow_default(bool)` e `.allow_dns(bool)` (as chaves do `local.conf` do ZeroTier; desligadas
por padrão), `.static_address(Cidr)` (IP fixo para as redes que não distribuem endereço),
`.orbit(moon, semente)`, `.planet(bytes)`, `.port(n)` (o padrão é uma porta UDP aleatória, aberta
em `0.0.0.0`), `.on_state(f)` e `.on_node_state(f)`; `.start().await?` devolve a `ZeroTier`.
O `NodeState` (a `Identity`, o planet, as moons, a última configuração assinada e os caminhos)
chega por `on_node_state`: grave-o **cifrado** (a identidade é um segredo) e passe-o na vez
seguinte; sem ele, cada partida é um membro novo que o administrador precisa autorizar de novo.
`Identity::generate()` é síncrona e leva de 1 a alguns segundos (prova de trabalho): o `start`
a chama em `spawn_blocking`, mas quem chama `start` dentro de uma thread da interface congela a
tela nesse tempo. `proxy()` é o `Proxy::Socks5` em `127.0.0.1` (porta e credencial aleatórias,
nunca na interface) e resolve os IPs gerenciados da rede e as rotas dela; com `allow_dns`, os
nomes do domínio que a rede anuncia vão por DNS sobre TCP aos servidores dela. `State` é
`Connecting`, `WaitingConfig`, `NotAuthorized { address }` (o administrador precisa marcar
“Auth” nesse endereço; o nó continua pedindo e conecta sozinho quando ele autoriza),
`NotFound`, `AuthenticationRequired` (login único pelo navegador, ainda não tratado),
`Connected { address, name, warnings }`, `Retrying(msg)`, `Failed(msg)` ou `Stopped`;
`wait_connected(limite)`, `peers()` e `roots()` (endereço, papel, caminho direto ou pelo raiz e
latência), `stop(espera)` (ou `close()`, que não espera nem consome o motor, para quando ele está num `Arc` dividido). O nó só leva TCP em IPv4, não recebe conexões (o filtro deixa entrar
só a resposta do que o app abriu) e não responde a nada que não pediu. Sem UDP a rede não
funciona nesta versão. Nos testes, a feature `test-util` dá o `RootFake` e o
`ControllerFake` (`testing`), que sobem numa `SimNet` ou em UDP real.

**Rede OpenVPN embutida** (crate `tomni-ui-openvpn`, sem placa virtual, sem administrador e sem o
binário `openvpn`; só o tráfego do app entra na VPN, o sistema não vê nada). Leia o `.ovpn` com
`Config::from_ovpn(texto, Some(pasta))?` (a pasta é onde ficam os arquivos que o perfil cita; um
caminho absoluto ou com `..` é recusado; o `Vec<Warning>` lista o que o app ignora) e confira com
`config.check(agora)` (certificado vencido, CA que não assinou). Suba com
`OpenVpnBuilder::new(config).credentials(usuario, Zeroizing::new(senha)).on_state(|estado| ..).start()?`:
é síncrono e a VPN roda numa thread própria; `.proxy(..)` é o proxy por onde o próprio transporte
sai (útil no TCP). `vpn.proxy()` é um `Proxy::Socks5` em `127.0.0.1` com porta e credencial
aleatórias: passe-o ao `.proxy(..)` do `Ssh::builder`, do `Telnet::builder` e do `Sftp`; mostre-o na
interface nunca (nem porta nem credencial) e não o exponha a outros programas. O estado
(`vpn.state()` e o `on_state`, chamado de outra thread) é `Connecting { remote }`, `Asking(Prompt)`,
`Waiting { message, url }`, `Connected { address, remote, cipher, warnings }`, `Retrying(msg)`,
`Failed(msg)` ou `Stopped`. Em `Asking`, mostre `prompt.text` (com `prompt.echo` dizendo se a
resposta aparece) e responda com `vpn.answer(prompt.id, Some(Zeroizing::new(texto)))`; `None`
cancela e a VPN termina em `Failed`. `PromptKind::Password` é a senha recusada (ou ausente),
`Challenge` o código de uso único do servidor e `StaticChallenge` o do `.ovpn`, pedido antes de
conectar. Em `Waiting` com `url`, o servidor quer uma confirmação pelo navegador. A VPN refaz a
conexão sozinha (`ping-restart`, `RESTART`, rede fora) com espera crescente; uma recusa definitiva
(`HALT`, certificado inválido, `ping-exit`) vira `Failed`. Os DNS e domínios que o servidor empurra
valem só dentro da ponte (um nome do domínio de busca é resolvido pela VPN; o resto, pelo sistema,
a menos que o perfil tenha `redirect-gateway`). `wait_connected(limite)` espera e devolve o
endereço; `stop(espera)` (ou `close()`, que não espera nem consome o motor, para quando ele está num `Arc` dividido) avisa o servidor e encerra. Suportado: UDP e TCP, `tls-auth`, `tls-crypt`,
`tls-crypt-v2` (cliente), TLS 1.2 e 1.3, AES-128/256-GCM e ChaCha20-Poly1305, `compress` em
passagem, `static-challenge` e `reneg-sec`. Ainda não: cifras CBC, LZ4 e LZO de
verdade, IPv6 dentro da VPN, PKCS#12, `crl-verify` e `http-proxy`/`socks-proxy` do perfil.

**Chaves, agente e `ssh-copy-id`** (feature `ssh`; tudo em `tomni_ui::terminal::ssh`, nada bloqueia
a escolha de quem usa o app: passphrase vazia e chave sem confirmação são permitidas):

```rust
use tomni_ui::terminal::ssh::agent::{Agent, Constraints};
use tomni_ui::terminal::ssh::keys::{KdfLevel, KeyFile, KeyGen, KeyType};
use tomni_ui::terminal::ssh::{CopyIdOptions, CopyIdOutcome, copy_id};

// Gerar: Ed25519 (padrão), ECDSA P-256/384/521 e RSA 3072/4096; DSA e RSA < 3072 não existem.
let chave = KeyGen::new(KeyType::Ed25519)
    .comment("ana@notebook")
    .passphrase("frase")          // sem esta linha, a chave fica sem passphrase
    .kdf(KdfLevel::High)          // rodadas do bcrypt-pbkdf: Standard 16, High 64, VeryHigh 100
    .generate()?;
chave.write(&home.join(".ssh/id_ed25519"), false)?; // grava a privada (0600) e o `.pub`; `true` sobrescreve
let (linha, impressao, arte) = (chave.public_openssh(), chave.fingerprint(), chave.randomart());

// Ler (OpenSSH, PEM, PKCS#8 e .ppk) e trocar a passphrase: `None` ou vazia tira.
let info = KeyFile::inspect(&caminho)?;           // tipo, bits, comentário e `encrypted`, sem pedir a passphrase
let arquivo = KeyFile::open(&caminho, Some("frase"))?; // `NeedsPassphrase` / `WrongPassphrase` se faltar ou errar
arquivo.rewrite(&caminho, Some("outra"), KdfLevel::Standard)?;

// De texto (colado, vindo do cofre) e para texto, sem arquivo: o M44.
let arquivo = KeyFile::from_text(&texto, Some("frase"))?;   // `inspect_text(&texto)` olha sem a passphrase
let saida = export_private(arquivo.private_key(), PrivateFormat::PpkV3, Some("frase"), KdfLevel::Standard)?;
// `PrivateFormat::{OpenSsh, Pkcs8, PemLegacy, PpkV3, PpkV2}`; `PrivateFormat::available(&chave)` diz quais
// existem para ela (o PEM antigo não existe para Ed25519). `FormatNotSupported` e `PassphraseNotSupported`
// (o PEM antigo é sem cifra) explicam o que falta.
let publica = export_public(chave.public_key(), "ana@notebook", PublicFormat::Rfc4716)?;
// `PublicFormat::{OpenSsh, Rfc4716, SpkiPem, Pkcs1Pem}` (o último só RSA); `parse_public(&texto)?` lê qualquer um
// e devolve `(PublicKey, comentário)`. `write_key_text(caminho, &texto, privada, sobrescrever)` grava o texto
// (0600 se `privada`) por um arquivo temporário. `SshBuilder::key(chave)` autentica com a chave já aberta.

// O agente (compatível com o ssh-agent do OpenSSH; `SSH_AUTH_SOCK` aponta para `agente.path()`).
let agente = Agent::start(None)?; // `Some(caminho)` escolhe o socket; "Já há um agente em …" se algo responde lá
agente.add(chave.private_key().clone(), "ana@notebook", Constraints { lifetime: None, confirm: true })?; // `AgentError::SecurityKey` para `sk-`
agente.add_certificate(certificado, "ana@notebook"); // `false` se a privada dele não está no agente
agente.on_confirm(|pedido| {
    // `pedido.key` é a chave; `pedido.peer.pid`, `pedido.peer.chain` (["ssh", "git", "codex"]) e
    // `pedido.peer.host_key` (o servidor, se o ssh mandou o `session-bind`) dizem quem pede.
    // true permite este uso; roda fora da thread da tela
    true
});
agente.on_change(|| { /* a lista, o travamento ou a vida mudou */ });
// Windows: o Pageant (o agente do PuTTY), para o `plink`, o WinSCP e o `git` do PuTTY. Liga o pipe
// `\\.\pipe\pageant.<usuário>.<hash>` e a janela oculta "Pageant"; soltar a guarda desliga.
// `Err(PageantError::OutroPageant)` se já há um (o do PuTTY, por exemplo); nada é ligado nesse caso.
#[cfg(windows)]
let pageant = agente.start_pageant()?;
let _ = (agente.list(), agente.lock("x"), agente.unlock("x"), agente.is_locked());

// ssh-copy-id: instala a pública no authorized_keys (sem duplicar; no Windows, o dos administradores).
let servidor = Ssh::builder("servidor", 22, "ana").password("segredo");
match copy_id(&servidor, chave.public_key(), CopyIdOptions::default())? {
    CopyIdOutcome::Installed { path } => println!("instalada em {path}"),
    CopyIdOutcome::AlreadyThere | CopyIdOutcome::WouldInstall { .. } => {}
}
```

Os erros de `keys` são `KeyError` (com `Display` em português). `copy_id` e `Agent::add` bloqueiam
um instante: chame de uma thread, nunca do evento de um widget, e devolva o resultado à tela com
um `Arc<Mutex<Option<…>>>` e um contador de signal (veja `tirech-omni/crates/tirech-omni/src/tela_chaves.rs`).
`SshBuilder::exec("comando", b"stdin")` roda um comando remoto sem terminal e devolve
`ExecOutput` (`success()`, `stdout_text()`, `stderr_text()`). Para a linha de comando existe o
pacote `tirech-omni/crates/tirech-omni-tools` (`tirech-omni-keygen`, `tirech-omni-agent`, `tirech-omni-add` e
`tirech-omni-copy-id`, com um subconjunto das opções do OpenSSH). O app `tirech-omni/crates/tirech-omni` tem as
páginas "Chaves SSH" e "Agente" e o diálogo "Copiar a chave para o servidor" sobre essas APIs.

**X11 e túneis** (feature `ssh`, em `tomni_ui::terminal::ssh`). `Ssh::builder(..).x11(X11::trusted())`
(o `ssh -Y`) ou `X11::untrusted()` (o `ssh -X`, com `xauth generate`; cai para o confiável com
aviso se o servidor X não tem a extensão SECURITY) encaminha o X11: o servidor precisa de
`X11Forwarding yes` e do `xauth`. `X11::display(":0")`, `.auth_file(..)` e `.xauth_program(..)`
mudam o que vem do ambiente (`DISPLAY`, `XAUTHORITY`; sem `DISPLAY`, `localhost:0` no Windows e
no macOS). O servidor recebe um cookie falso e o servidor X desta máquina recebe o real.
`.forward(Forward::local(5433, "db", 5432))` (`-L`), `Forward::remote(9000, "localhost", 3000)`
(`-R`, a porta 0 deixa o servidor escolher), `Forward::dynamic(1080)` (`-D`, SOCKS5 só CONNECT,
sem autenticação) e `Forward::unix(LocalSocket::Path(..) | LocalSocket::Tcp(..), "/var/run/docker.sock")`
(`-L` para um socket Unix do servidor); `.open_to_network()` escuta em `0.0.0.0` (no `-R` o
servidor exige `GatewayPorts clientspecified`). Por padrão a escuta é só em `127.0.0.1`.
Porta ocupada vira aviso no terminal e o shell segue; `.exit_on_forward_failure(true)` fecha a
conexão. `.no_shell(true)` mantém só os túneis, `.command("..")` roda um comando em vez do
shell e `.launch("virt-manager --no-fork")` roda um programa numa conexão própria, sem
terminal, e devolve um `RemoteApp` (`status()`: `Connecting`, `Running`, `Exited(código)`,
`Failed(motivo)`; `stderr_tail()` e `close()`). Com a sessão aberta,
`sessao.ssh_forwards()` (`Option<SshForwards>`; peça de novo depois de `restart`) dá
`list()` (`ForwardStatus`: `id`, `forward`, `listening`, `state` com `Starting | Listening |
Stopped | Failed(motivo)`, `active`, `total`, `sent`, `received`, `last_error`), `x11()`
(`X11Status`), `add(Forward)`, `stop(id)` e `restart(id)`. Tudo vale com salto e com proxy.
No `tirech-omni/crates/tirech-omni`: `tela/tuneis.rs` (editor e túneis ativos) e `tela/programas.rs`
(virt-manager e aplicativos gráficos).

Para um programa seu, implemente `tomni_ui::terminal::Transport` (`write`, `read`, `pending`;
`resize` é opcional) com uma `Inbox`, como o exemplo `terminal` do Tomni UI. A `Inbox` guarda
até `INBOX_LIMIT` (1 MiB) do que chegou e a interface ainda não leu: a thread que lê o programa
usa `inbox.push_wait(&bytes)`, que espera vaga (o programa que despeja sem fim fica esperando,
como num terminal de verdade, e a memória não cresce), ou confere `inbox.has_room()` antes de ler
mais; `push` não espera (mensagens curtas, avisos). No `Drop` do transporte, chame
`inbox.detach()`: solta a thread que espera vaga e descarta o que vier depois.

Builders: `.id(id)`, `.grow(1)` ou `.width(n)`/`.height(n)`, `.cols_rows(80, 24)` (grade fixa;
sem ele, a grade segue o tamanho do widget, no mínimo 20 × 5), `.font_size(n)` (Cascadia
Mono, padrão 16; `n` é `f32`, escreva `14.0`), `.scheme(Scheme::Campbell)` (o padrão, escuro nos dois temas;
`Scheme::Theme` segue o tema do app; `Scheme::Custom(paleta)`), `.scrollback(linhas)` (9001;
0 desliga; mudar o limite não apaga a tela nem o histórico, só o que passar do novo limite sai, e vale para as sessões novas do mesmo jeito), `.cursor_shape(CursorShape::Block)` (`tomni_ui::terminal::CursorShape`; o padrão é a barra), `.cursor_blink(false)` (valor, closure ou `Signal`; `false` deixa o cursor fixo mesmo que o programa peça piscante, e o padrão segue o programa; com a sessão encerrada o cursor fica fixo e só no contorno, vazado), `.on_title(...)`, `.copy_on_select(true)`,
`.paste_warning(false)` (sem a confirmação de colar várias linhas),
`.right_click(RightClick::Menu | CopyPaste)` (`tomni_ui::terminal::RightClick`; o clique direito abre o `.context_flyout`, o
padrão quando há um, ou copia com seleção e cola sem, como no PuTTY; a tecla Menu e o Shift+F10
abrem o menu nos dois, e no `CopyPaste` o Ctrl+clique direito também), `.bell(BellStyle::Flash | None)` (`tomni_ui::terminal::BellStyle`; o padrão pisca a tela com o BEL), `.visible(prop)` e
`.tooltip(prop)`. `.font_size`, `.scheme`, `.cursor_shape`, `.copy_on_select`,
`.paste_warning`, `.right_click` e `.bell` aceitam valor fixo, closure ou `Signal`: trocar o
signal vale na hora (a fonte refaz a grade e avisa a conexão do novo tamanho; a forma do
cursor só até o programa pedir outra com DECSCUSR).

**Zoom da fonte pela roda.** `.on_font_zoom(|passos: i32| ..)` liga o Ctrl+roda (Ctrl sozinho): o
handler recebe os passos (positivo aumenta, com a roda para cima; um passo é uma volta do mouse,
`tomni_ui::terminal::FONT_ZOOM_WHEEL` = 48 px, e o touchpad soma os pixels até completar um) e a
roda **não** rola o histórico, não manda setas e não vai ao programa, nem com o mouse pedido por
ele. Sem o handler, nada muda. O terminal não guarda o tamanho: some os passos num signal ou num
deslocamento por aba e devolva por `.font_size(closure)`, que refaz a grade e avisa a conexão;
`TERMINAL_FONT_MIN` e `TERMINAL_FONT_MAX` (6 e 72) são os limites. Os atalhos de teclado
(Ctrl+=, Ctrl+-, Ctrl+0) são do app, com `.accelerator(..)`: eles vêm antes do terminal. Não
confunda com o zoom do painel do `TerminalPanes`, que amplia o painel.

Links: o terminal guarda os links que o programa marca (`OSC 8`) e acha as URLs `http://` e
`https://` do texto, e também `localhost:3000`, `127.x.y.z:8080`, `0.0.0.0:8000` e `[::1]:5173`
sem esquema (porta de 1 a 65535, depois de espaço, começo de linha ou `(["'<`; o `uri` sai
com `http://` na frente e `explicit` falso). `.on_link(|link: &LinkHit| ..)` (`tomni_ui::terminal::LinkHit`: `uri`
inteira, `explicit` se veio do `OSC 8`) é chamado no **Ctrl+clique** (esquerdo, sem Alt) num
link; o clique não seleciona nem vai ao programa. Sem Ctrl, o clique num link marcado seleciona
como sempre. `.link_action(|link| Some("Abrir no navegador".into()))` dá o rótulo do que o
Ctrl+clique faz (na dica e no menu); `None` diz que aquele link não abre daqui (o Ctrl+clique não
chama o `on_link` e o menu só oferece copiar). Sem `link_action`, vale "Abrir link" para `http` e
`https` e `None` para o resto (o `OSC 8` com `file:` ou `mailto:` chega ao `on_link` só com a
ação). `.links(false)` (valor, closure ou `Signal`) desliga sublinhado, mão, dica e
Ctrl+clique. O ponteiro sobre um link marcado mostra a dica com o endereço (cortado no meio com
"…" se não cabe) e "Ctrl+clique: <rótulo>"; com o Ctrl apertado, o link do texto e o marcado
ganham sublinhado contínuo e a mão. O clique direito, a tecla Menu e o Shift+F10 sobre um link
abrem um menu com "Abrir…" e "Copiar link" (ids `{id}-link-abrir` e `{id}-link-copiar`), um
separador e o menu do `.context_flyout` do app, ou "Copiar" e "Colar" (`{id}-menu-copiar`,
`{id}-menu-colar`) sem ele; fora de um link nada muda. `.context_copy_paste()` põe esse "Copiar" (só com seleção) e "Colar" também antes dos itens do `.context_flyout` do app, com um separador depois. `.on_ctrl_secondary(quando, || ..)` dá o Ctrl+clique direito ao app enquanto `quando` (valor, closure ou `Signal`) for verdadeiro: a ação roda e o clique não abre o menu, não copia nem cola (por exemplo, `move || sessao.has_selection()` para agir sobre o texto marcado). Nos testes, `app.ui().set_modifiers(
Modifiers::CTRL)` antes de `move_to`/`click_at` simula o Ctrl, e a dica é `{id}-tooltip`.

Teclado: o Tab vai ao programa e **F6** leva o foco ao próximo
widget (Shift+F6, ao anterior). Ctrl+Shift+C copia e Ctrl+Shift+V cola; Ctrl+C copia quando
há seleção e, sem ela, vai ao programa. O clique direito cola (ou copia, com seleção). A roda
e Shift+PgUp/PgDn rolam o histórico; na tela alternativa (vim, less, htop), a roda manda setas,
a menos que o programa desligue (`ESC[?1007l`). O terminal já tem 44 px e cumpre o lint; precisa de
`.id()`.

Mouse do programa: quando o programa pede o mouse (o relato do xterm: `ESC[?1000h`, `?1002h`
para o arrastar, `?1003h` para todo movimento e `?9h`, só o pressionar; as posições na forma
padrão, SGR com `?1006h`, `?1015h` ou `?1005h`), a roda, os três botões, o arrastar e o
movimento vão a ele, na célula sob o ponteiro e com Shift, Alt e Ctrl. Enquanto isso, o clique
não seleciona, o Ctrl+clique não abre link e o clique direito não abre o menu nem cola. **Com
o Shift pressionado, o mouse volta ao terminal**: seleciona, rola o histórico, abre o menu (e o
Ctrl+Shift+clique abre o link). A barra de rolagem é sempre do terminal. Com `ESC[?1004h`, o
programa recebe `ESC [ I` e `ESC [ O` quando o terminal ganha e perde o foco. Os modos ligados
estão em `visual.emulator().modes()` (`mouse: MouseTracking`, `mouse_encoding: MouseEncoding`,
`alternate_scroll`, `focus_events`; `tomni_ui::terminal::{MouseTracking, MouseEncoding}`).

Testes: `app.feed(id, bytes)` entrega bytes como se viessem do programa;
`app.terminal_text(id)` é o texto da tela (sem os espaços do fim),
`app.terminal_history(id)` inclui o histórico e `app.terminal_cell(id, col, linha)` dá o
caractere, as cores e os atributos. `type_into(id, texto)` e `type_text(texto)` escrevem na
conexão (`\n` é o Enter); `right_click(id)` pressiona e solta o botão direito: cola ou, com o
mouse pedido pelo programa, vai a ele; `right_click_with(id, Modifiers::CTRL)` é o mesmo com
modificadores (no `CopyPaste`, abre o menu). Para o Shift do mouse, `app.ui().set_modifiers(
Modifiers::SHIFT)` antes do `click_at`, do `drag` ou do `wheel` (e `Modifiers::NONE` depois).
Com um `Loopback`, o handle lê o que foi
escrito (`handle.take_written()`) e responde (`handle.feed("...")`). Com um processo de
verdade, a saída chega depois: repita `app.wait(Duration::from_millis(20))` até o texto
aparecer, com um limite de tempo. No `ui_interact`, `type` num terminal escreve na conexão e
`wait` dá tempo ao programa; o `ui_tree` mostra o título, a grade, o cursor e o texto da tela.

### Colar imagem e arquivos

O terminal só cola texto. Imagem (uma captura de tela) e arquivos (copiados no gerenciador de
arquivos, ou soltos no terminal) chegam ao app por `.on_rich_paste(|colagem: RichPaste| ..)`
(`tomni_ui::terminal::{RichPaste, RichPasteContent, RichPasteSource}`):

- `colagem.content` é `RichPasteContent::Files(Vec<PathBuf>)` (arquivos e pastas desta máquina)
  ou `RichPasteContent::Image(ClipboardImage)` (pixels RGBA; `.to_png()` dá o PNG).
- `colagem.source` diz de onde veio: `Paste` (Ctrl+Shift+V, Shift+Insert, Cmd+V, botão direito,
  "Colar" do menu), `CtrlV` ou `Drop` (arquivos soltos no terminal).
- `colagem.sessions` são as sessões que recebem: a do terminal e, com o envio para vários
  ligado num `TerminalPanes`, as outras, na ordem do `TerminalPanes`. A colagem rica **não**
  passa pelos avisos de várias linhas nem de vários terminais: quem trata decide o que fazer.

A ordem de leitura é arquivos, depois texto, depois imagem; só o que sobra chega ao handler (uma
planilha, que põe texto e imagem, cola o texto como sempre). O handler decide o texto e o cola
com **`sessao.paste(texto)`**, que respeita o *bracketed paste* do programa e vai às mesmas
conexões que o teclado. `.rich_ctrl_v(true)` (valor, closure ou `Signal`; desligado por padrão)
faz o Ctrl+V sem Shift também chamar o handler quando a área de transferência não tem texto;
com texto, o `^V` continua indo ao programa. Num `TerminalPanes`, ponha o handler em
`.terminal(|t| t.on_rich_paste(..))`.

**Diretório e cópia pelo programa.** `sessao.cwd()` é o diretório que o programa informou pelo
`OSC 7` (`ESC ] 7 ; file://máquina/caminho`, que o `bash` e o `zsh` mandam num `PROMPT_COMMAND`
ou `precmd`): o caminho já vem com os `%XX` decodificados, a máquina é ignorada e `None` quer dizer
que o programa nunca informou (ou que a sessão reabriu com `restart()`). O `OSC 52`
(`ESC ] 52 ; c ; base64`, o que o `tmux`, o `vim` e o `ssh` usam para copiar) põe o texto na área
de transferência do sistema: `.osc52(b)` (valor, closure ou `Signal`; **ligado por padrão**) o
liga e desliga. Só o último pedido de cada quadro vale, o pedido de leitura (`?`), o base64 torto
e o que passa de 1 MiB são ignorados, e a área de transferência nunca é lida pelo programa.

Sem handler, imagem não faz nada e os arquivos soltos colam os caminhos locais, cada um com
aspas e um espaço depois. Para montar um caminho (remoto, por exemplo) como **um** argumento do
shell, use `shell_quote(texto, ShellQuote::{Posix, PowerShell, Cmd})`: sem aspas quando o texto só
tem caracteres seguros, aspas simples (`'` vira `'\''` no POSIX e `''` no PowerShell) ou duplas
(`cmd`) nos outros casos.

Para mandar o arquivo ao servidor, abra um canal na mesma conexão SSH da sessão:
`sessao.ssh_channels()` (feature `ssh`; `None` se a sessão não é SSH) entrega um
`SshChannelOpener`, que serve ao `Sftp::from_channel` e tem **`exec(comando, &mut stdin, &mut |enviado| .., limite)`**, que roda um
comando sem terminal numa conexão que já está aberta e devolve o `ExecOutput` (`success()`,
`stdout`, `stderr`). Bloqueia: chame de uma thread. **`exec_cancelable(.., &AtomicBool)`** é o mesmo,
e para quando a bandeira vira `true`: fecha o canal (o comando do servidor perde a conexão) e devolve
`Interrupted`; olha a bandeira a cada bloco de `stdin` e a cada 100 ms de espera pela saída. Os dois devolvem erro se o servidor recusar o pedido de `exec`. `Ssh::builder(..).compression(true)` pede
`zlib` na conexão (o `ssh -C`; o servidor que não comprime segue sem).
**`shell_dir(limite)`** descobre a pasta atual do shell da sessão: roda no servidor um `sh` que sobe da própria
árvore de processos até o `sshd`/`dropbear` da conexão, acha o único processo com terminal filho dele e lê o
`cwd` do processo em primeiro plano (ou do shell). Devolve `Ok(Some("/pasta"))`, `Ok(None)` (não achou,
mais de um shell na conexão, servidor sem `/proc`, ou passou do `limite`) ou `Err` (a conexão caiu). Bloqueia:
chame de uma thread; é o que o app usa para duplicar a aba na pasta atual.
**`open_direct(host, porta, limite) -> io::Result<SshChannel>`** abre um canal `direct-tcpip` até
`host:porta` visto do servidor, sem escutar porta nenhuma aqui (o `SshChannel` é `AsyncRead` +
`AsyncWrite`; use-o num runtime tokio). Bloqueia até o servidor aceitar: chame de uma thread. O canal
mantém a conexão aberta mesmo depois do `exit` do shell. A recusa vem escrita ("Nada escuta em
db:5432 no servidor.", "o servidor proíbe túneis (AllowTcpForwarding)"); `ConnectionReset` é sessão
acabada.

No teste: `app.clipboard().write_files(..)` / `write_image(..)` (o `MemoryClipboard`),
`app.key_with(Key::Char('v'), Modifiers::CTRL.with(Modifiers::SHIFT))` e
`app.drop_files(id, &[caminho])`; com um `Loopback`, `handle.take_written()` mostra o que foi
escrito.

### Sessões e painéis

`Terminal::new(conexao)` guarda a conexão no próprio widget. Para ter vários terminais lado a
lado, em abas e em grade, a conexão vive numa `Session` (um valor `Copy`, com o terminal
dentro de um registro da thread) e o `TerminalPanes` desenha o `PaneLayout` que o app guarda
num signal. Precisa da feature `terminal`.

```rust
use tomni_ui::prelude::*;
use tomni_ui::terminal::{Broadcast, Loopback, PaneLayout, Session, TerminalPanes};

pub fn tela() -> impl View {
    let sessoes: Vec<Session> = ["web-01", "web-02", "db-01"]
        .into_iter()
        .map(|nome| Session::new(Loopback::new().echo()).key(nome).name(nome))
        .collect();
    let layout = signal(PaneLayout::with(sessoes)); // um painel, com as três abas
    let envio = signal(Broadcast::None);

    Form::new("Servidores").child(
        Column::new().gap(8).padding(16)
            .child(
                Row::new().gap(8)
                    .child(Button::new("1").id("layout-1").grow(1).height(44)
                        .on_click(move || layout.update(|l| l.single())))
                    .child(Button::new("2 × 2").id("layout-2x2").grow(1).height(44)
                        .on_click(move || layout.update(|l| l.grid(2, 2))))
                    .child(Button::new("Visíveis").id("envio-visiveis").grow(1).height(44)
                        .on_click(move || envio.set(Broadcast::Visible))),
            )
            .child(
                TerminalPanes::bind(layout)
                    .id("paineis")
                    .broadcast(envio)
                    .new_session(|| Session::new(Loopback::new().echo())) // o painel criado ao dividir
                    .on_session_closed(|sessao| sessao.dispose())         // a aba foi fechada
                    .grow(1),
            ),
    )
}
```

- **`Session`**: `Session::new(transporte)`, `.key("web-01")` (identifica a sessão nas
  ferramentas e no `Broadcast::Group`) e `.name("Servidor web")` (o título inicial da aba; o
  programa pode mudar por cima). Leituras: `key_str()`, `title()`, `closed()` e
  `close_reason()` (o motivo que a conexão deu ao encerrar, ou `None`; os três últimos são
  reativos), `connecting()` (reativo: `true` enquanto a conexão abre, no `Ssh` até o shell
  abrir e no `Telnet` até o TCP abrir; volta a `true` no `restart()`; um transporte seu a
  informa por `Transport::connecting()`, que deve acordar a interface com `wake()` ao passar a
  `false`), `text()` (a tela) e `history()`. Ações: `send(bytes)` (como se o
  usuário digitasse), `feed(bytes)` (como se viesse do programa), `restart()` (reabre uma sessão encerrada com a mesma configuração, no `Ssh`, `Process`,
  `Telnet`, `Serial` e `Loopback`; devolve `false` se a sessão está viva ou o transporte próprio
  não implementa `reopen`), `duplicate()` (outra conexão com a mesma configuração, ou `None`), `duplicate_in(Some(pasta))` (o mesmo, com a pasta inicial do shell local; só vale se a pasta existe, e transporte que não é processo ignora), `duplicate_running(Some("comando"))` (outra conexão SSH com a mesma autenticação que roda o comando no lugar do shell, como o `SshBuilder::command`; `None` volta ao shell; `None` nos transportes que não sabem, via `Transport::duplicate_running`), `process_id()` (o pid do processo filho de `Process`; `None` nos demais), `log_to(caminho, LogMode::Text)` (ou `Raw`, com as cores),
  `log_with(Box<dyn Write>, modo)` (o mesmo log num escritor seu, por exemplo um arquivo cifrado: o `flush` só vem do
  `flush_log()`, e soltar o log solta o escritor), `is_logging()` e
  `stop_log()`, `received()` (quantos bytes chegaram pela conexão; o `feed` não conta, então um aviso
  seu não parece saída do servidor), `sent()` (quantos bytes foram para a conexão), `busy()` (o
  `Duration` que a interface gastou com a sessão: ler, interpretar e pintar), `threads()` (os ids
  das threads da conexão no sistema, para medir a CPU delas em `/proc/self/task/<id>/stat`; vazio
  fora do Linux e no `Loopback`; com o `process_id()` do shell local, dá o peso de cada aba), `tap()` (um `OutputTap` que guarda o texto limpo, como no log
  de texto, que chega depois dele; `take()` devolve e esvazia, e soltar o tap para de guardar; sem
  teto, então chame `take()` a cada verificação), `dispose()` (solta a conexão; chame ao fechar a
  aba), `hold(valor)` (pendura um valor qualquer na sessão, solto só no `dispose()`: as portas
  que o app abre para a aba, por exemplo; o fim da conexão e o `restart()` não soltam),
  `held::<T, _>(|valor| ..)` (lê o primeiro valor do tipo `T` pendurado; `None` se não há) e
  `Session::find(chave)`. `selection_text()` devolve o texto selecionado no `Terminal`
  que mostra a sessão (`None` se não há seleção; não é reativo) e `has_selection()` diz se há
  seleção (reativo: use em `item_visible` ou `disabled` de um item de menu de contexto, que é
  montado uma vez). `tomni_ui::terminal::plain_text(bytes)` dá o texto limpo do log de texto
  (sem cores nem controles) para qualquer trecho de bytes.
  `Terminal::session(sessao)` põe uma sessão num `Terminal` solto; `terminal.session_ref()`
  devolve a de um `Terminal::new`.
- **Atividade da sessão**: o que o programa diz ao emulador vira estado, todo reativo.
  `progress()` é o `OSC 9;4` (`TermProgress::{None, Normal(u8), Error(Option<u8>),
  Indeterminate, Paused(Option<u8>)}`), `alerts()` conta os avisos (BEL, `OSC 9` com texto e
  `OSC 777;notify`, uma vez por bloco recebido), `last_notice()` é o último com texto
  (`Notice { title, body }`), `attention()` diz se há aviso não visto e `mark_seen()` o apaga. O
  `TerminalPanes` liga isso às abas sozinho: anel de progresso, ponto de atenção (que some ao
  escolher a aba, estando a janela com o foco) e o pedido de atenção da janela
  (`WindowHandle::request_attention()`) quando o aviso chega com a janela sem o foco.
  `terminal_panes.activity()` soma tudo (`PanesActivity { progress: Option<f64>, busy,
  attention: usize }`) para o app levar ao dock ou à barra de tarefas. O título do programa
  (`OSC 0/2`) com um caractere como `◐` ou `✳` no começo chega à aba, mas as fontes embutidas não
  desenham esses caracteres (nem o braile).
- **`PaneLayout`** é dado puro (cabe num `Signal`): `with(sessoes)`, `single()`,
  `grid(colunas, linhas)`, `split(painel, PaneDirection::Right)`, `add_tab`, `move_tab`,
  `move_tab_to(sessao, painel, indice)`, `remove`, `close_pane`, `close_emptied(true)` (o painel
  que perde a última aba some sozinho, se houver outro; `closes_emptied()` consulta),
  `zoom(painel)` (alterna e ativa o painel), `set_active`, `select(sessao)`, `resize` e as consultas `active()`,
  `zoomed()`, `pane_count()`, `pane(n)`, `pane_of(sessao)`, `sessions()` e `visible()`.
  `Display` mostra a forma: `col[row[0,1],row[2,3]]`.
- **Atalhos** (`.shortcuts(PaneShortcuts)` troca ou
  `PaneShortcuts::none()` desliga; aceita também um `Signal<PaneShortcuts>`, lido na hora da
  tecla): Alt+Shift++ e Alt+Shift+- dividem, Alt+Shift+D duplica, (`TerminalPanes::on_duplicate(|origem, lugar: DuplicatePlace| ..)` troca o duplicar: o app cria a cópia como quiser, até depois de uma sondagem em thread, e entrega com `lugar.place(sessao)`, que divide o painel pelo lado maior; sem o gancho, é o `origem.duplicate()`),
  Alt+setas mudam de painel, Alt+Shift+setas movem a divisão, Ctrl+Shift+W fecha o painel. O
  zoom não tem atalho padrão: `atalhos.zoom = vec![Shortcut::ctrl_shift('z')]`. O menu de
  contexto da aba move para outro painel, divide, reinicia e fecha.
- **Abas caídas e dica**: a aba de uma sessão encerrada (`Session::closed()`) troca o ícone por
  `Symbol::Warning`, e o terminal dela ganha o contorno de aviso. Se a conexão deu um motivo
  (`close_reason()`), o ícone fica vermelho; encerrar sem motivo não é queda. Enquanto a sessão
  conecta (`Session::connecting()`), a aba mostra o anel indeterminado, que o progresso do
  programa (`OSC 9;4`) substitui. `.on_tab_tooltip(|sessao|
  Option<String>)` põe a dica da aba (`{id}-terminal-{chave}`… no `TerminalPanes`; o id da dica é
  `{id da aba}-tooltip`, com o texto num `Text` filho); sem ela, ou com `None`, a dica é o título.
  `.on_tab_mark(|sessao| Option<Symbol>)` troca o ícone da aba por uma marca do app (reativa; a sessão
  encerrada vale mais, a marca vale mais que a do envio para várias abas; o anel e o ponto seguem).
  `.on_tab_mark_tint(|sessao| Option<ThemeColor>)` pinta essa marca (`TabView::tab_icon_tint(|chave| ..)` no solto).
  Num `TabView` solto, `.tab_tooltip(|chave| Option<String>)` faz o mesmo. Um painel sem abas
  mostra "Fechar o painel" (`{id}-painel-{n}-fechar`, e o item igual no menu de contexto dele),
  quando há mais de um painel.
- **Envio** (`Broadcast`): `None` (cada terminal recebe o que é dele), `Visible` (a aba escolhida
  de cada painel), `All` e `Group(vec!["web-01".into()])` (por chave). O que se digita ou cola no
  terminal com o foco vai a todas as sessões do envio, e as abas delas mostram o ícone de envio.
- **Identificar** (`.identify(valor)`, valor fixo, closure ou signal): põe no centro de cada
  painel um crachá grande com o número dele (`Painel 1` é o `pane(0)`, a ordem das folhas), acima
  do terminal e abaixo dos menus; ponteiro e teclado passam por ele, e ligar e desligar não
  remonta os painéis. `.highlight_pane(Some(2))` (valor fixo, closure ou signal de
  `Option<usize>`) pinta o crachá desse painel na cor de destaque e dá ao quadro dele a borda
  sólida do ativo. Painel pequeno demais (menos de 120 px mais a folga) fica sem crachá. O
  crachá tem o id `{id}-painel-{n}-numero`. O app que quer "identificar por 2 segundos" liga o
  signal e põe um alarme para desligar.
- **Aba escolhida** (as duas ficam desligadas por padrão): `.tab_scheme(Some(Scheme::Campbell))`
  (valor fixo, closure ou signal de `Option<Scheme>`) pinta a aba escolhida de cada painel com o
  fundo e o texto do esquema dos terminais, como o Windows Terminal faz, para ela "emendar" no
  terminal (`Scheme::tab_colors()` dá o par; o `Scheme::Theme` usa os tokens do tema).
  `.active_tab_indicator(true)` põe uma barra de 16 × 3 px na cor de destaque embaixo da aba
  escolhida do painel ativo, para ver de longe qual painel recebe o teclado. No `TabView` solto,
  `.selected_colors(Some((fundo, texto)))` (dois `ThemeColor`) e `.selected_indicator(bool)` fazem
  o mesmo (o X da escolhida acompanha a cor do título); o `Look::TabHeader` mostra os dois em
  `selected_colors` e `indicator`.
- **Ids** de um `TerminalPanes` com `.id("paineis")`: `paineis-terminal-{chave}` (cada
  terminal, pela chave da sessão), `paineis-painel-{n}` e `paineis-divisao-{n}`. O lint exige
  o `.id()` e botões de 44 px; abas e botões da barra seguem as regras de sempre.
- **Teste**: `app.click("paineis-terminal-web-01")` ativa um painel; `app.type_into("paineis", "ls\n")`
  escreve no terminal do painel ativo (e no envio); `app.session_text("web-02")` e
  `app.session_history("web-02")` leem uma sessão pela chave, **à mostra ou não**;
  `app.panes("paineis")` devolve o `PanesInfo` (layout, painel ativo, zoom, envio, `identifying`, `highlighted` e as abas de
  cada painel). Com um `Loopback`, `handle.take_written()` mostra o que cada sessão recebeu.
  As sessões moram num registro da thread: ao repetir o mesmo app em outro tema na mesma
  thread de teste, chame `dispose()` nelas antes, senão `session_text` acha as do primeiro.
  No `tomni-ui tree`/`ui_tree`, o nó `terminal_panes` mostra `layout=`, `active=`, `zoomed=`,
  `broadcast=`, `identifying` e `highlighted=N` (quando ligados) e uma linha por painel com as abas; o `type` do `ui_interact` num
  `terminal_panes` escreve no terminal do painel ativo.

O exemplo `terminal_panes` do Tomni UI (`--features terminal`) junta tudo isso com três
sessões `Loopback`.

### Salvar conexões

A lib **não** guarda conexões nem senhas: quem decide onde e como é o app. O `PaneLayout`
guarda sessões, não configurações, então o app mantém a lista de conexões (host, porta,
usuário, chave) e monta as sessões a partir dela. Com `serde` (dependência do app):

```rust
#[derive(serde::Serialize, serde::Deserialize)]
struct Conexao {
    nome: String,
    host: String,
    porta: u16,
    usuario: String,
    chave: Option<std::path::PathBuf>,
}

fn salvar(conexoes: &[Conexao], arquivo: &std::path::Path) -> std::io::Result<()> {
    std::fs::write(arquivo, serde_json::to_string_pretty(conexoes)?)
}

fn abrir(conexao: &Conexao, senha: Option<String>) -> Session {
    let mut ssh = Ssh::builder(&conexao.host, conexao.porta, &conexao.usuario);
    if let Some(chave) = &conexao.chave {
        ssh = ssh.key_file(chave, None);
    }
    if let Some(senha) = senha {
        ssh = ssh.password(senha);
    }
    Session::new(ssh.connect()).key(conexao.nome.clone()).name(conexao.nome.clone())
}
```

A senha **nunca** vai para o arquivo: guarde no cofre do sistema com o crate `keyring`
(`keyring::Entry::new("meu-app", &conexao.nome)?.set_password(&senha)?`, e
`.get_password()` ao conectar) ou peça num `Edit` com `.password()` a cada conexão (num
`ContentDialog`, e descarte a senha depois de abrir a sessão). Para restaurar o layout, grave
as chaves das sessões de cada painel (`layout.pane(n).tabs` e `key_str()`), reabra as sessões
e refaça o `PaneLayout` com `grid`, `add_tab` e `move_tab`.

Um exemplo completo disso está no repositório `tirech-omni`, em `crates/tirech-omni`: árvore de
conexões em grupos, propriedades editáveis, `connections.json` e `estado.json` gravados na
hora, a pergunta da chave do servidor (`check_host_key`) num diálogo, a senha pedida a cada
conexão e as abas reabertas na próxima execução.

## Arquivos remotos (SFTP, FTP, S3, rclone e o `Fs`)

Features `files` (o `Fs`, a `Remote`, a fila de transferências e as ferramentas de caminho),
`sftp` (o SFTP; liga `files` e `ssh`) e `ftp` (o FTP e o FTPS; liga `files`) e `s3` (o S3 e os compatíveis; liga `files`) e `rclone` (as demais nuvens pelo binário `rclone`; liga `files`). Tudo em
`tomni_ui::files`; o proxy e a conexão TCP
que o SSH e o SFTP dividem estão em `tomni_ui::net` (`Proxy`, `connect`). O `MemFs` (um disco
em memória, para os testes) não vem pela fachada: acrescente ao `Cargo.toml` do app
`tomni-ui-files = { version = "..", features = ["test-util"] }` e use `tomni_ui_files::MemFs`.

- **`Fs`** (trait, bloqueante): `home`, `list(dir)`, `stat`, `lstat`, `open_read(path, offset)`,
  `open_write(path, offset)`, `mkdir`, `remove_file`, `remove_dir` (só vazia), `rename(de, para,
  sobrescrever)`, `set_permissions`, `set_modified`, `symlink(alvo, link)`, `read_link`,
  `real_path(caminho)` (o caminho absoluto com os links resolvidos, o `realpath` do SFTP; `NotFound`
  se não existe, `Unsupported` no FTP, no S3 e no rclone; `Remote::real_path` responde por
  `Reply<String>`), `caps()`. `Local::new()` é o disco; `Sftp`, `Ftp` e `S3` são servidores; `MemFs` é o de teste. Os caminhos
  são absolutos e com `/` mesmo no Windows; use `files::path::{join, parent, file_name,
  normalize, numbered}` e não `std::path`. Os erros são `io::Error` com a mensagem em
  português e o caminho.
- **`Entry`**: `name`, `kind` (`EntryKind::{File, Dir, Symlink, Other}`), `size`, `modified`
  (`Option<SystemTime>`), `permissions` (`Option<u32>`, os 12 bits), `owner`, `group` e
  `link_target`. `Caps` diz o que o backend sabe (`permissions`, `set_modified`, `symlinks`,
  `resume_upload`, `owner`): esconda ou desabilite o que falta.
- **`Sftp`** (feature `sftp`): `Sftp::connect(&SshBuilder)` abre uma conexão própria (a mesma
  `Ssh::builder(..)` do terminal, com salto, proxy, chave e agente); `Sftp::from_channel(&opener)`
  abre um canal novo na conexão SSH que um terminal já tem
  (`sessao.ssh_channels()` devolve `Option<SshChannelOpener>`), sem pedir a senha de novo. O
  `Opener` guarda o que precisa para reabrir: capture o `SshBuilder` (ou o
  `SshChannelOpener`) no fecho.
- **`Ftp`** (feature `ftp`, em `tomni_ui::files::ftp`): um cliente FTP e FTPS próprio e
  bloqueante, que fala com o vsftpd, o proftpd e o pure-ftpd. `FtpBuilder::new(host, porta)` e,
  encadeando: `.login(usuario, senha)` (sem isso entra como `anonymous`), `.security(..)`,
  `.mode(..)`, `.tls_version(..)`, `.encoding(..)`, `.active_ports(10000..=10100)`,
  `.timezone_offset(minutos)` (o fuso do servidor, para as datas do `LIST`), `.proxy(..)` (só no
  modo passivo), `.check_certificate(..)`, `.timeout(..)`; então `.connect()` (um `Ftp`, que é
  um `Fs`) ou `.opener()` (um `Opener` para a `Remote` e para a fila, que abre uma conexão por
  chamada). A **segurança** é `FtpSecurity::{Explicit (o padrão: `AUTH TLS` na porta 21, recusa
  se o servidor não aceitar), ExplicitIfAvailable (cai para o claro e deixa o aviso
  `NOTICE_NO_TLS`), Implicit (TLS desde o início, porta 990), None}`; o **modo**,
  `FtpMode::{Passive (o padrão: EPSV e depois PASV), Active (EPRT e PORT; não passa por proxy)}`;
  a **versão do TLS**, `TlsVersion::{Auto, Only12, Only13}` (a `Auto` começa no 1.3 e passa ao 1.2
  sozinha, com um aviso, se o servidor não retomar a sessão); a **codificação**,
  `FtpEncoding::{Auto, Utf8, Latin1}`. Os dados vão pela mesma sessão TLS do controle, retomada
  (o vsftpd e o proftpd recusam sem isso): não há o que configurar.
- **`S3`** (feature `s3`, em `tomni_ui::files::s3`): a Amazon e os compatíveis (MinIO, Cloudflare
  R2, Wasabi, Backblaze B2, DigitalOcean Spaces, Ceph), por HTTP, com a assinatura SigV4 do
  `rusty-s3` e o `ureq`. `S3::connect(S3Config)` confere as chaves (lista uma chave, ou os buckets
  sem `bucket`) e dá um `Fs`; `config.opener()` dá o `Opener` para a `Remote` e a fila.
  Com `bucket`, `/` é a raiz do bucket; sem ele, `/` lista os buckets como pastas e
  `/bucket/chave` é um objeto. Uma pasta é um prefixo terminado em `/` (`mkdir` cria o
  marcador `pasta/`; buckets não se criam nem se apagam por aqui; `remove_dir` apaga todo o
  prefixo, em lotes). Os envios grandes vão em partes e
  **retomam** pelo servidor (`Fs::resume_offset`); `rename` copia e apaga (em partes acima de
  `copy_limit`); `set_permissions`, `set_modified` e `symlink` dão erro e não há `owner` (`caps()` diz). A data
  do arquivo vai no envio, em `x-amz-meta-mtime` (`open_write_with`).
  ```rust
  use tomni_ui::files::{Fs, s3::{S3, S3Config, S3UrlStyle}};

  let mut s3 = S3::connect(S3Config {
      bucket: Some("meu-bucket".into()),
      url_style: S3UrlStyle::Path, // `VirtualHost` na Amazon e na maioria dos provedores
      ..S3Config::new("http://127.0.0.1:9000", "minioadmin", "minioadmin")
  })?;
  let entradas = s3.list("/")?;
  ```
  Campos de `S3Config`: `endpoint`, `region` (`us-east-1`; o R2 usa `auto`), `bucket`,
  `url_style`, `access_key`, `secret_key`, `session_token` (chaves temporárias), `storage_class`
  (`STANDARD_IA`…), `tls` (`TlsMode::{Verify, CustomCa(arquivo PEM), NoVerify}`), `proxy`,
  `part_size` (no mínimo 5 MiB) e `copy_limit`. Extras de `S3` fora do `Fs`: `presign_get(path,
  validade)` (um link de download, de até 7 dias), `head(path)` (`ObjectInfo`),
  `set_storage_class(path, classe)`, `pending_uploads(pasta)` e `abort_upload(path, id)` (os
  envios em partes que não terminaram, que o provedor cobra). As chaves nunca vão para o
  `Debug` nem para um arquivo de configuração: guarde a secreta num cofre (o `tirech-omni` usa o
  item `Tipo::S3`, com o token num campo oculto). Não faz: SSE-C, ACLs, versionamento,
  políticas de ciclo de vida, `gzip`. O `tirech-omni/crates/tirech-omni` é o exemplo completo
  (`src/arquivos/s3.rs`); os testes dele e da lib usam o servidor falso `S3d`
  (`crates/files/tests/comum/s3d.rs`) em vez de rede.
- **`Rclone`** (feature `rclone`, em `tomni_ui::files::rclone`): as demais nuvens (Google Drive,
  OneDrive, Dropbox, Box, Azure Blob, Google Cloud Storage, WebDAV, SMB, Mega… mais de 70) sem
  reimplementar nenhuma: a lib conversa com o binário `rclone` (instalado pela pessoa, versão
  1.60 ou mais nova) pela API do `rclone rcd`. O `Rcd` é o processo: `Rcd::spawn(RcdConfig::new(
  find_rclone(None)?))` sobe o `rclone rcd` em `127.0.0.1` com usuário e senha aleatórios
  (`RcdConfig` também tem `config_file`, `config_password` da configuração cifrada e `proxy`),
  confere a versão (`RcloneError::{NotFound, TooOld, ConfigPassword, ..}`; `ENDERECO_DE_INSTALACAO`
  é o link para o aviso) e encerra no `Drop` (`shutdown()`). Subir leva até 10 s: **chame de
  uma thread**, nunca da tela. `Rcd::attach(url, usuario, senha)` liga a um `rcd` que já existe
  (os testes). `Rclone::new(rcd, "gdrive:Backup")` é um `Fs` com `/` como a raiz do remoto
  (o `:memory:` serve para testar). `Fs::delegated_copy` faz as cópias local ↔ remoto e remoto ↔
  remoto pelo próprio rclone, com progresso e cancelamento; extras fora do `Fs`: `features()`,
  `about()` (espaço) e `public_link(path, validade)`. A configuração dos remotos (`files::rclone::
  config`, métodos do `Rcd`): `config_remotes()`, `config_providers()` (cada `Provider` com as
  `ProviderOption`: `name`, `help`, `default`, `examples`, `exclusive`, `required`, `is_password`,
  `advanced`, `hide`, `provider`, `kind`, e `applies_to(provider)` para as que só valem para um
  `provider`), `config_create(nome, tipo, &[(opção, valor)])`, `config_get`, `config_update` e
  `config_delete`. Se o rclone ainda precisa de respostas, `config_create` devolve
  `ConfigStep::Question { state, option, error }`: mostre a pergunta, mande a resposta com
  `config_answer(nome, &state, resposta)` e repita até `ConfigStep::Done`; responder `true` ao
  `config_is_local` abre o navegador e **só volta quando o login termina** (até 10 minutos),
  então vá numa thread, com um botão Cancelar que apague o remoto inacabado. Um exemplo:

  ```rust,no_run
  use tomni_ui::files::Fs;
  use tomni_ui::files::rclone::config::ConfigStep;
  use tomni_ui::files::rclone::{Rcd, RcdConfig, Rclone, find_rclone};

  let rcd = Rcd::spawn(RcdConfig::new(find_rclone(None).ok_or("sem rclone")?))?;
  // Um remoto novo: `config_create` devolve a primeira pergunta, se houver.
  let mut passo = rcd.config_create("meu-s3", "s3", &[("provider".into(), "AWS".into())])?;
  while let ConfigStep::Question { state, option, .. } = passo {
      let resposta = option.default.clone(); // na tela: um diálogo com a `option`
      passo = rcd.config_answer("meu-s3", &state, &resposta)?;
  }
  let mut nuvem = Rclone::new(rcd, "meu-s3:bucket");
  let entradas = nuvem.list("/")?;
  # Ok::<(), Box<dyn std::error::Error>>(())
  ```

  Guarde as senhas dos remotos num cofre (o `tirech-omni` usa o item `Tipo::Rclone`, e o rclone as
  ofusca no `rclone.conf`). Não faz: montar, sincronizar, agendar, instalar o rclone e ajuda para
  os remotos que embrulham outros (`crypt`, `union`). O `tirech-omni/crates/tirech-omni` é o exemplo completo
  (`src/rclone.rs` os `Rcd` por proxy, `src/tela/rclone.rs` e `src/tela/rclone_form.rs` a
  subpágina e o formulário gerado das opções); os testes dele e da lib usam um `rcd` falso
  (`crates/files/tests/comum/rcd_falso.rs`) em vez do rclone.
- **Certificado do FTPS**: o servidor é verificado pelas raízes do sistema. Um certificado que
  elas não aceitam (autoassinado, vencido, nome errado) chama o fecho de
  `.check_certificate(|info: &CertificateInfo| -> bool)` com `host`, `sha256` (a impressão, em
  hexadecimais com `:`), `subject`, `issuer`, `not_before`, `not_after` e `error` (o motivo);
  `true` segue, `false` recusa a conexão. O fecho roda na thread da conexão: para perguntar à
  pessoa, ponha o pedido numa fila, acorde a tela com `tomni_ui::wake()` e espere a resposta
  por um canal (o `tirech-omni` faz isso em `tirech-omni/crates/tirech-omni/src/abrir.rs`,
  `conferir_certificado`, e guarda os confiados em `certificados.json`, chave `host:porta`).
  Sem o `.check_certificate`, o certificado não confiável falha.
- **Avisos**: `Remote::notice()` (`Live<Option<String>>`; `.set(None)` apaga) e
  `TransferInfo.notice` trazem o que o servidor ou o cliente quis dizer sem ser um erro (o
  fallback para o TLS 1.2, a conexão que seguiu sem TLS). Mostre num `InfoBar` ou na linha da
  fila.
- **O que o FTP não tem**: num servidor sem `MLST` (o vsftpd) o `stat` não traz as permissões
  (a listagem traz); `caps()` diz o que falta. As datas usam `MFMT` e, sem ele, `MDTM` de dois
  argumentos ou `SITE UTIME`.
  Retomar o envio usa `REST STREAM` ou `APPE`. Fora do escopo: FXP, `MODE Z`, modo ASCII,
  certificado de cliente.
- **`Remote`**: uma conexão numa thread, que a tela comanda sem travar.
  `Remote::spawn("nome", Arc::new(move || Ok(Box::new(Sftp::connect(&ssh)?) as Box<dyn Fs>)))`
  (`Opener` = `Arc<dyn Fn() -> io::Result<Box<dyn Fs>> + Send + Sync>`). `remote.state()` é um
  `Live<RemoteState>` (`Connecting`, `Ready`, `Failed(msg)`, `Closed`); `reconnect()` e
  `close()`. Cada pedido devolve um `Reply<T>` (`Live<Option<Result<T, String>>>`: `None`
  até chegar): `home()`, `list(dir)`, `stat`, `mkdir`, `remove_file`, `remove_dir`,
  `rename(de, para, sobrescrever)`, `set_permissions`, `set_modified` e `run(|fs| ...)` para o
  resto (o fecho pode rodar duas vezes se a conexão cair, por isso é `Fn`). A conexão que cai
  reabre sozinha uma vez; depois vira `Failed`.
- **`Live<T>`**: o valor que uma thread grava e a tela lê como um signal (`get`, `with`,
  `get_untracked`, `with_untracked`; num `effect` ou numa closure reativa ele rastreia).
  `Live::new` nasce no escopo reativo de quem cria; `Live::detached` não morre com ele (o que
  `Remote::run` usa para as respostas). Quem guarda signals de uma aba ou de um diálogo e os
  lê depois que o escopo morreu leva o panic "reactive value disposed": crie num
  `Scope::detached()` guardado numa `thread_local!` e leia com `try_get` nos efeitos.
- **`Transfers`**: a fila de envios, baixas, cópias e exclusões, com trabalhadores em paralelo
  (`Transfers::new(3)`, `set_parallel`, `set_preserve_date`, `set_preserve_permissions`).
  `add((Endpoint, "origem"), (Endpoint, "destino"), Conflict)` devolve o número;
  `Endpoint::Local` ou `Endpoint::Remote(remote.opener())`. Pasta vai inteira (recursiva); uma
  pasta nova do disco para o SFTP vai de uma vez, com 32 arquivos e 64 escritas em voo, e vira um
  item só (`size` e `done` em bytes do total; `Fs::upload_tree(de, para, &TreeOptions, progress,
  cancel)` faz isso e devolve `None` nos backends que não sabem). A pausa só vale entre itens.
  `delete(endpoint, caminho)`. Para a tela: `items()` (`Live<Vec<TransferInfo>>`: `id`,
  `kind` (`TransferKind::{Upload, Download, Copy, Delete}`), `from`, `to`, `size`, `done`,
  `speed`, `state`), `pause_all`, `resume_all`, `cancel(id)`, `cancel_all`, `retry(id)`,
  `clear_finished`, `wait_idle(timeout)`. `TransferState`: `Queued`, `Running`, `Paused`,
  `Done`, `Failed(msg)`, `Skipped`, `Cancelled`, `Asking`.
  `add_sized(origem, destino, conflito, Some(tamanho))` é o `add` com o tamanho que a tela já
  conhece da listagem: sem ele a leva fica "contando" até a fila ler cada arquivo.
  **Tempo restante (M40):** `summary()` (`Live<TransferSummary>`, recalculado a cada 250 ms)
  resume a leva: `state` (`QueueState::{Idle, Running, Paused, Asking}`), `files_total`,
  `files_done`, `failed`, `deletes`, `bytes_total`, `bytes_done`, `counting` (os totais ainda
  crescem), `speed` (B/s), `eta` (`Option<Eta { low, likely, high }>`: uma faixa, não um
  número; `None` enquanto não se mediu o bastante) e `stalled`. Mostre a faixa e não o
  provável sozinho, e não deixe o texto tremer a cada 250 ms. Cada `TransferInfo` também tem
  `is_dir` e `remaining` (o `Eta` do item). A estimativa é por rota (origem e destino) e soma
  o custo por arquivo à vazão medida. `Transfers::with_clock` troca o relógio (`ManualClock`
  nos testes) e `refresh_summary()` recalcula na hora.
- **Conflito** (o destino já existe): `Conflict::{Ask, Overwrite, Resume, Skip, Rename}`
  (`Rename` grava `nome (1).ext`; `Resume` continua de onde o destino parou). Com `Ask`, a
  fila para e `transfers.asking()` (`Live<Option<ConflictQuestion>>`: `id`, `path`, `source` e
  `target`, cada um com `size` e `modified`) pede um diálogo; responda com
  `transfers.answer(id, escolha, aplicar_aos_proximos)`. O diálogo que `tirech-omni` usa
  (`ContentDialog` com "Sobrescrever" como primário, "Pular" como secundário e botões para
  "Retomar" e "Renomear") está em `tirech-omni/crates/tirech-omni/src/arquivos/fila.rs`.

Uma tela de arquivos junta a `Remote` (ou o `Local`), um `DataGrid` com as `Entry` em
`DataGridRow` e a fila. O desenho que funciona:

```rust
let remoto = Remote::spawn("servidor", opener);
let pasta = signal("/".to_owned());
let linhas = signal(Vec::<DataGridRow>::new());
let selecao = signal(BTreeSet::<String>::new());
let transferencias = Transfers::new(3);

// Lista de novo quando a pasta muda; a resposta chega de outra thread.
let resposta = signal(None::<Reply<Vec<Entry>>>);
effect(move || resposta.set(Some(remoto.list(&pasta.get()))));
effect(move || {
    let Some(Some(Ok(entradas))) = resposta.get().map(|r| r.get()) else { return };
    linhas.set(entradas.iter().map(|e| {
        let (rank, icone) = if e.is_dir() { (1, Symbol::Folder) } else { (2, Symbol::Document) };
        DataGridRow::new(e.name.clone())
            .cell(e.name.clone(), CellValue::Text(e.name.clone()))
            .cell(tamanho(e.size), CellValue::Number(e.size as f64))
            .cell(data(e.modified), CellValue::Time(e.modified))
            .icon(icone)
            .rank(rank) // as pastas ficam antes e o ".." (rank 0) em cima
    }).collect());
});

DataGrid::bind(linhas, selecao)
    .id("remoto")
    .drag_group("arquivos") // arrastar entre dois painéis
    .on_invoke(move |nome| pasta.set(files::path::join(&pasta.get_untracked(), &nome)))
    .on_drop(move |d: DataGridDrop| { /* transferencias.add(origem, destino, Conflict::Ask) */ })
    .on_files_dropped(move |caminhos, alvo| { /* envio dos arquivos do sistema */ })
```

Regras que pegam:

- O `Live` de um `Reply` muda fora do ciclo da tela. Leia com `get()` dentro de um `effect` ou
  de uma closure reativa (como acima) e **nunca** calcule a tela uma vez só com o valor atual.
- Um `effect` que esvazia o próprio signal volta a rodar para sempre: só mexa se houver o que
  tirar (`if !v.is_empty()`).
- Datas do servidor chegam em UTC; mostre-as assim e diga na tela, ou converta.
- O Wayland não entrega `DroppedFile`: o arrastar de arquivos do sistema só funciona no X11,
  no Windows e no macOS. Dê também um botão "Enviar…" com o `FileDialog` (veja "Seletor de
  arquivo").
- Uma senha ou frase-senha do servidor não vai para a `Entry`, para a fila nem para o log:
  fica no `SshBuilder` capturado pelo `Opener`.
- Abrir um arquivo remoto no editor do sistema: baixe para uma pasta temporária, abra com o
  programa do sistema (`xdg-open`, `open`, `start`) e, enquanto ele estiver aberto, vigie a
  data do arquivo e envie de volta quando mudar (o `tirech-omni` faz isso com uma thread de 1 s;
  veja `tirech-omni/crates/tirech-omni/src/arquivos/editar.rs`).

No teste, use o `MemFs` no lugar do servidor e do disco:

```rust
let servidor = MemFs::new();
servidor.put("/home/ana/a.txt", "oi").dir("/home/ana/docs");
let remoto = Remote::spawn("teste", servidor.opener());
// ... monte a tela, `app.wait(Duration::from_millis(50))` até a listagem chegar,
// `app.grid("remoto")`, `app.drag_rows("local", &["a.txt"], "remoto", None)`,
// `app.drop_files("remoto", &[caminho])`.
assert_eq!(servidor.contents("/home/ana/docs/a.txt"), Some(b"oi".to_vec()));
```

`MemFs::new()`, `.connect()`, `.opener()`, `.put(caminho, bytes)`, `.dir`, `.link`, `.touch`,
`.contents`, `.exists`, `.paths`, `.set_chunk`, `.set_delay` (para a transferência durar o
bastante para o teste ver o progresso), `.set_caps`, `.deny(caminho)` (nega o acesso) e
`.drop_connections()` (a conexão cai). Para um servidor SFTP de verdade, sem rede, os testes da
lib sobem um `russh` + `russh-sftp` local (`crates/files/tests/comum/servidor.rs`; o
`tirech-omni/crates/tirech-omni/tests/real.rs` o reaproveita). Um exemplo
completo, com as duas abas, a fila, o diálogo de conflito, as permissões e o editor externo,
está em `tirech-omni/crates/tirech-omni` (página Arquivos).

**Espaço livre**: `tomni_ui::files::available_space(&caminho) -> io::Result<u64>` dá os bytes
livres para o usuário no volume do caminho (se ele ainda não existe, mede o do primeiro
ancestral que existe). É uma função solta, sem `Fs` nem estado: use antes de baixar ou gravar
algo grande.

## IA (`tomni_ui::ai`, feature `ai`)

Fala com um provedor de modelos por HTTP, em dois dialetos: `Dialect::OpenAi` (o
`/chat/completions`, que Ollama, LM Studio, OpenRouter, OpenAI e Groq entendem) e
`Dialect::Anthropic` (o `/messages`). A feature `ai` liga também `files` (o `Live`).

```rust
use tomni_ui::ai::{Dialect, Endpoint, Event, Message, Request, stream};

let mut ponto = Endpoint::new(Dialect::OpenAi, "http://localhost:11434/v1", "llama3.2");
ponto.api_key = Some("chave".to_string().into()); // Zeroizing<String>: apagada ao soltar
let pedido = stream(
    ponto,
    Request {
        system: "Responda em português.".into(),
        messages: vec![Message::user("Oi")],
        ..Request::default()
    },
    move |evento| match evento {
        Event::Text(pedaco) => { /* grave num Live: acorda a tela */ }
        Event::Reasoning(_) => {}
        Event::Done(_) => {}
        Event::Error(erro) => { /* erro.message já está em português */ }
    },
); // StreamHandle: .cancel() para parar, .finished() para saber se a thread acabou
```

- `stream` roda numa thread e chama `on_event` **nela**: grave num `files::Live` (que acorda a
  tela) ou numa fila com `wake()`; não mexa em signals de dentro do callback. O último evento é
  um `Done` ou um `Error`; depois do `cancel()` nenhum evento chega mais.
- `Endpoint` tem `headers`, `proxy`, `tls` (`Tls::System`, `CustomCa(pem)`, `NoVerify`),
  `connect_timeout` e `response_timeout` (`None` espera sem limite: um modelo local carregando
  leva minutos). Servidor sem streaming responde inteiro num único `Text`. Redirecionamentos
  não são seguidos (a chave não vai para outro host).
- `models(&endpoint)` lista os modelos do provedor (segue a paginação) e `locality(url)` diz se
  o endereço é `Machine`, `Lan` ou `Internet` (sem resolver DNS), para o app avisar antes do
  primeiro envio.
- **A lib só fala com o provedor.** Ela não redige segredos, não tira ANSI nem controles do
  que sai e não decide o que fazer com a resposta: isso é do app, e a resposta nunca é
  executada sem a pessoa mandar. Para a saída do terminal, use `terminal::plain_text`.
- Testes sem rede: `tomni_ui::ai::test_util::FakeAi` é um servidor HTTP local nos dois
  dialetos (`responder(&["pedaço", ..])`, `erro(status, corpo)`, `sem_streaming(texto)`,
  `com_pausa(d)`, `fechar_no_meio()`, `modelos(&[..])`, `exigir_chave(..)`, `pedidos()` para
  conferir o que chegou). Aponte o `Endpoint` para `falso.url()`. O exemplo completo (campo,
  "Perguntar", "Parar" e a resposta num `MarkdownView`) é `crates/tomni-ui/examples/ia.rs`.

## Servidor MCP do próprio app (`tomni_ui::mcp`, feature `mcp`)

Para o app oferecer ferramentas a um assistente de código (um cliente MCP: JSON-RPC 2.0, uma
mensagem por linha), implemente `McpTools` e chame `serve`. O enquadramento (`initialize` com a
versão de `PROTOCOL_VERSIONS`, `ping`, `tools/list`, `tools/call`, `notifications/cancelled` e
os erros -32700, -32600 e -32601) é da lib; o app só diz o que as ferramentas fazem.

```rust
use serde_json::{Value, json};
use tomni_ui::mcp::{self, McpError, McpTools};

struct Eco;

impl McpTools for Eco {
    fn server_info(&self) -> (String, String) { ("meu-app".into(), "0.1.0".into()) }
    fn instructions(&self) -> String { "Devolve o texto recebido.".into() }
    fn tools(&self) -> Value {
        json!([{ "name": "eco", "description": "Devolve o texto.",
                 "inputSchema": { "type": "object", "properties": { "texto": { "type": "string" } } } }])
    }
    fn call(&mut self, name: &str, args: &Value, _id: &Value) -> Result<Value, McpError> {
        match name {
            "eco" => Ok(mcp::text_result(args["texto"].as_str().unwrap_or(""))),
            _ => Err(McpError::new(-32602, "ferramenta desconhecida")),
        }
    }
}

// mcp::serve(&mut Eco, std::io::stdin().lock(), std::io::stdout().lock());
```

`call` devolve `Ok(text_result(..))` ou `Ok(error_result(..))` (o erro da ferramenta, que o
assistente lê, com `isError: true`) e `Err(McpError)` só para o erro de protocolo. `cancelled(id)`
avisa que o cliente desistiu da chamada `id`. `handle(&mut tools, &mensagem)` trata uma mensagem
já lida (devolve `None` nas notificações), para quem fala por outro canal que não o stdio.

## WebView

Uma página da web dentro da tela, no controle do sistema (WebView2 no Windows, WKWebView no
macOS, WebKitGTK no Linux), como o `WebView2` do WinUI. Fica atrás de uma feature do
`tomni-ui`: `features = ["webview"]`. No Linux, compilar pede o `libwebkit2gtk-4.1-dev`; só
`"webview-widget"` dá o widget com o motor falso dos testes (sem a dependência do sistema).

```rust
use tomni_ui::prelude::*;

const AJUDA: &str = r#"<!doctype html><html lang="pt-BR"><head><meta charset="utf-8">
<title>Ajuda</title><link rel="stylesheet" href="estilo.css"></head>
<body><button onclick="window.fluent.postMessage('pronto')">Entendi</button></body></html>"#;

pub fn ajuda() -> impl View {
    let web = WebViewController::new();
    let lida = signal(false);
    Form::new("Ajuda").child(
        Column::new().gap(8).padding(16)
            .child(
                Row::new().gap(4).align(Align::Center)
                    .child(
                        Button::icon_only(Symbol::ArrowLeft).id("voltar").label("Voltar")
                            .disabled(move || !web.can_go_back())
                            .on_click(move || web.go_back()),
                    )
                    .child(Text::new(move || web.title()).id("titulo").max_lines(1)),
            )
            .child(
                WebView::html(AJUDA)                       // servida em fluent://app/index.html
                    .asset("estilo.css", "body { margin: 24px }")
                    .id("pagina")
                    .grow(1)                               // a página fica com o espaço que sobra
                    .controller(web)
                    .on_message(move |texto| lida.set(texto == "pronto")),
            ),
    )
}
```

Conteúdo: `WebView::new(url)` carrega um endereço (fixo, closure ou signal: navega de novo
quando muda); `WebView::html(texto)` e `.asset("pasta/arquivo.js", bytes)` embutem páginas no
app, servidas em `fluent://app/` (o tipo vem da extensão; `fetch`, módulos e links entre elas
funcionam). A página navega à vontade na própria origem, em `fluent://app/` e nas de
`.allow("https://docs.exemplo.com")`; um link para outra origem (`http`, `https`, `mailto`,
`tel`), um `target="_blank"` ou um `window.open` abre no navegador do sistema.

Builders: `.id(id)`, `.grow(1)` ou `.width(n)`/`.height(n)` (no mínimo 200 × 150),
`.controller(web)`, `.on_message(|texto| ...)`, `.on_load(|carga| ...)` (um
`tomni_ui::webview::Loaded`, com `url` e `error: Option<String>`), `.on_title(|t| ...)`,
`.visible(prop)` e `.tooltip(prop)`.

O `WebViewController` (`Copy`, crie com `WebViewController::new()` e passe com `.controller`)
lê e comanda a página: `url()`, `title()`, `state()` (`LoadState::Loading`, `Ready` ou
`Failed`), `loading()`, `error()`, `can_go_back()` e `can_go_forward()` são reativos (use em
closures); `navigate(url)`, `go_back()`, `go_forward()`, `reload()`, `post_message(texto)` e
`eval(script)` valem no frame seguinte.

A página e o app conversam por texto (para dados, mande JSON): na página,
`window.fluent.postMessage(texto)` chega no `on_message`, e
`window.fluent.addEventListener('message', e => ... e.data ...)` recebe o `post_message` do
app. É a única ponte: a página não chama nada do app. Trate o que chega como dado não
confiável, igual a uma entrada do usuário: compare com os valores esperados
(`match texto.as_str() { "sim" => ..., "nao" => ..., _ => {} }`) ou leia o JSON e confira
cada campo, e nunca o passe a um comando, caminho de arquivo ou `eval`. A página recebe o tema do app no `prefers-color-scheme` (use
`:root { color-scheme: light dark; }`).

Enquanto a primeira carga anda, o lugar da página mostra um `ProgressRing`
(`{id}-carregando`); se falhar, um aviso com o erro e o botão **Tentar de novo**
(`{id}-erro`, `{id}-tentar`). O controle do sistema é uma janela por cima da do app: quando um
`ContentDialog`, um `Flyout`, um menu ou uma dica se abre sobre a página, o WebView mostra uma
captura dela no lugar. Por isso, **não ponha o WebView num `ScrollViewer`** (o lint dá o erro
`webview-in-scroll`): a página já rola sozinha; dê a ela o espaço que sobra com `.grow(1)`.
O Tab leva o foco para dentro da página e o **F6** (Shift+F6) o traz de volta ao próximo
(anterior) widget, como no `Terminal`.

Sistemas: no Windows, o motor é o runtime do WebView2, que vem no Windows 11; no Windows 10,
o instalador do app instala o runtime (o *Evergreen Bootstrapper* da Microsoft). No Linux, o
app roda com o X11 (no Wayland, pelo XWayland) e precisa do WebKitGTK 4.1 instalado. No
Android, o WebView ainda não funciona. Se o motor não abrir, o lugar da página mostra o erro
("o WebView do sistema não abriu: ...").

Sem janela (testes, `tomni-ui lint`, capturas da CLI e do MCP), o motor é um falso: o lugar
da página mostra um globo com o endereço, e cada carga termina no passo seguinte ao pedido. A
primeira é pedida no primeiro passo: depois do `TestApp::new`, rode `app.frame()` duas vezes
para a página ficar pronta (`state` `"ready"`, com o título). Testes:
`app.webview_url(id)`, `webview_navigations(id)` (o que o app mandou carregar),
`webview_follow(id, url)` (um link clicado na página; devolve se carregou),
`webview_opened(id)` (o que abriu no navegador), `webview_message(id, texto)` (a página manda
ao app), `webview_posted(id)` e `webview_scripts(id)` (o que o app mandou e rodou na página),
`webview_fail(id, erro)`, `webview_hold(id, true)` (segura as cargas em `loading`, como numa
rede lenta), `webview_shown(id)` (onde a página está, ou `None` se escondida),
`webview_dark(id)`, `webview_page_focus(id)`, `webview_focused(id)` e
`webview_f6(id, false)`. Os que mudam algo já rodam um passo; uma carga nova fica pronta no
passo seguinte (`app.frame()`). No
`tree()`, o nó do WebView tem `url`, `title`, `state` (`"loading"`, `"ready"`, `"failed"`),
`error`, `can_go_back` e `can_go_forward`; o `ui_tree` mostra o mesmo.

## Várias janelas

Todas as janelas do app rodam na mesma thread e no mesmo runtime reativo. Abra outra com
`open_window` (no `prelude`), de um clique, de um atalho ou de um menu:

```rust
use tomni_ui::prelude::*;

fn principal() -> impl View {
    let contador = signal(0); // nasce na janela principal
    Form::new("Principal").child(
        Column::new()
            .gap(8)
            .padding(16)
            .child(Text::new(move || format!("Contador: {}", contador.get())).id("contador"))
            .child(Button::new("Nova janela").id("nova-janela").height(44).on_click(move || {
                open_window(move || secundaria(contador))
                    .title("Outra janela")
                    .size(640, 420)
                    .open();
            })),
    )
}

fn secundaria(contador: Signal<i32>) -> impl View {
    let eu = WindowHandle::current(); // guarde aqui, ao montar a tela
    on_close_requested(move || true); // `false` cancela o fechar
    Form::new("Outra").child(
        Column::new()
            .gap(8)
            .padding(16)
            .child(Text::new(move || format!("Contador: {}", contador.get())).id("contador"))
            .child(Button::new("Fechar").id("fechar").height(44).on_click(move || eu.close())),
    )
}
```

- **A regra dos signals.** Um signal criado na tela de uma janela morre com ela. Estado que mais
  de uma janela usa nasce na janela principal (ou num `Scope::detached()` do app) e entra nas
  outras por captura, como o `contador` acima. Um `Signal` é `Copy`: passe-o à função da
  outra tela.
- **`WindowHandle::current()`** vale na montagem da tela e dentro dos eventos dela, mas **não**
  dentro de um `effect` (o valor é o da janela cujo frame rodou o efeito). Guarde o handle ao
  montar a tela: `let eu = WindowHandle::current();`. O resto: `WindowHandle::main()`,
  `WindowHandle::all()`, `.is_open()`, `.close()`, `.focus()`, `.hide()`, `.show()`, `.is_hidden()`, `.title()` e `.geometry()`
  (`Option<WindowGeometry>`: `position: Option<(i32, i32)>` e `size: (u32, u32)` em px lógicos,
  lidos na hora, não reativos; serve para gravar a disposição das janelas).
- **`open_window(tela)`** devolve um pedido: `.title(..)`, `.size(largura, altura)` (padrão: o
  do `App::size`, reduzido ao monitor se não cabe), `.position(x, y)` (em px lógicos da tela; a
  janela é puxada para dentro do monitor se passar da borda) e `.open()`, que devolve o
  `WindowHandle` (`None` no Android, que tem uma tela só). A janela abre no próximo ciclo, mas o
  handle já vale. As telas das janelas secundárias entram no lint com `App::view(tela)`.
- **Processos de console sem janela (Windows).** Um app de interface que roda `rclone`, `hostname`,
  `powershell`… faz o Windows abrir uma janela de console que pisca. Passe o `Command` por
  `tomni_ui::process::no_window(&mut comando)` antes do `spawn`/`output`/`status`: no Windows põe
  `CREATE_NO_WINDOW`, nos outros sistemas não faz nada, e devolve o `&mut Command` para encadear. A lib
  já o usa no `rclone rcd` e nos programas do X11. Quem usa `windows_subsystem = "windows"` no
  `main.rs` precisa dele em todo `Command`.
- **`hide()` e `show()`** escondem e mostram a janela sem fechá-la: a árvore, os signals e os efeitos
  seguem rodando, e esconder a principal **não** encerra o app (é como um app de bandeja vive).
  `show()` também traz à frente e foca. `is_hidden()` não é reativo. `App::start_hidden(true)`
  abre a principal escondida. No Wayland, que não tem como esconder, a janela é minimizada.
  No teste, `app.window_hidden(janela)`.
- **A bandeja do sistema** (`tomni_ui::tray`): `Tray::new(TrayImage::from_png(PNG))` com
  `.tooltip(|| String)`, `.on_activate(|| ..)` (o clique no ícone), `.item(id, || rótulo, || ação)`,
  `.check(id, || rótulo, || marcado, |valor| ..)`, `.enabled(|| bool)` (vale para o último item),
  `.separator()` e `.show()`. O `TrayHandle` que `show()` devolve **remove o ícone ao ser solto**
  e a closure da view morre depois de montar: guarde-o ou chame `.keep()` (vive até o fim do app).
  A dica, os rótulos, as marcas e o `enabled` são recalculados quando os signals mudam. O item
  `check` recebe o valor novo; o app é quem atualiza o signal. `tray::available()` diz se o
  sistema tem bandeja (é reativo; no GNOME sem a extensão de indicadores é `false`). Esconder
  em vez de fechar é do app: **só esconda se `tray::available()`**, senão a pessoa perde a
  janela. O padrão:

  ```rust
  Tray::new(TrayImage::from_png(ICONE))
      .tooltip(move || format!("Cliques: {}", cliques.get()))
      .on_activate(|| WindowHandle::main().show())
      .item("sair", || "Sair".into(), move || { saindo.set(true); WindowHandle::main().close(); })
      .show()
      .keep();
  on_close_requested(move || {
      if saindo.get_untracked() || !untrack(tomni_ui::tray::available) { return true; }
      WindowHandle::main().hide();
      false
  });
  ```

  Não há bandeja no Android. No teste: `app.tray()` (`Option<TrayState>` com `tooltip` e `items`,
  `None` sem bandeja), `app.tray_click("id")`, `app.tray_activate()` e
  `app.set_tray_available(false)`. O exemplo completo é `examples/bandeja.rs`.
- **Ícone e nome do app**: `App::icon(WindowIcon::new(PNG_GRANDE).with_small(PNG_PEQUENO))` (PNG
  ou SVG embutidos com `include_bytes!`; o SVG é rasterizado no tamanho de cada uso) vale para a
  janela, a barra de tarefas e as janelas que o app abrir; `App::app_id("meu-app")` é o nome com
  que o ambiente agrupa as janelas e as casa com o atalho `.desktop` (no Wayland o ícone vem do
  `.desktop`, não da janela). `TitleBar::icon_image(PNG)` põe uma imagem no lugar do ícone de
  símbolo e `.title_badge(TitleBadge::new(fundo).part("Meu", cor).bold_part("App", cor))` troca
  o título por um selo colorido. `WindowHandle::request_attention()` pisca o botão da janela na
  barra de tarefas (nada se ela já tem o foco).
- **O dock do Linux**: `LauncherEntry::new("meu-app.desktop").update(LauncherState { progress:
  Some(0.4), count: Some(2), urgent: false })` mostra a barra e o número no ícone do app pela
  interface `com.canonical.Unity.LauncherEntry`. Só envia quando o estado muda, não faz nada sem
  barramento de sessão (nem fora do Linux) e só aparece se existir o `.desktop`.
- **`active_window()`** é um `Memo<WindowHandle>` com a janela com o foco do sistema.
- **`on_close_requested(|| bool)`** roda quando a janela vai fechar (botão da `TitleBar`, Alt+F4,
  o sistema, `.close()`); `false` cancela. Fechar a **principal** pergunta a todas as janelas e,
  se nenhuma recusar, fecha todas e encerra o app: é também o gancho de saída (salvar o estado).
  Fechar uma secundária pergunta só a ela. A última janela fechada encerra o app.
- **Os ids** de cada janela são independentes: `contador` pode existir nas duas.
- **Abas entre janelas.** `TabView::drag_group(nome)` + `on_tab_drop` deixam a aba ir para
  qualquer `TabView` do grupo, de qualquer janela, e `on_tab_drop_outside(|chave, ponto|)` roda
  quando a aba é solta fora de todas as janelas (`ponto` é `None` onde o sistema não informa a
  posição, como no Wayland, onde arrastar para **outra janela** não funciona: ofereça o
  menu). Quem tira a aba da origem é o app. No `TerminalPanes`, `.drag_group("sessoes")` faz
  isso sozinho: a sessão sai do layout de origem **sem ser descartada** (o histórico e a conexão
  continuam) e entra no de destino. `.on_tear_off(|sessao, ponto|)` é o gancho do "destacar
  numa janela nova" (sem ele, soltar fora não faz nada), `.tab_menu_extra(|sessao, menu| menu.item(..))`
  acrescenta itens ao menu da aba, `.on_tab_click(|sessao|)` (`TabView::on_tab_click(|chave|)` no
  solto, o `Tapped` do WinUI) avisa o clique numa aba mesmo quando ela já era a escolhida (o
  signal não muda, então um efeito não veria), e `move_session(sessao, layout_destino, painel)`
  (`tomni_ui::terminal`) leva uma sessão para outro layout, de qualquer janela. O exemplo
  `janelas` (`--features terminal`) junta tudo isso.
- **Um app inteiro assim.** `tirech-omni/crates/tirech-omni` (M28) tem uma janela principal com a casca (árvore,
  páginas, menus) e janelas secundárias só com `TitleBar`, um `MenuBar` curto e um
  `TerminalPanes`. Cada janela guarda o layout e a grade num `Scope::detached()` do app (uma lista
  de janelas que vive na principal nunca guarda um signal já descartado), o menu "Janela" lista
  as abertas com `item_visible`, e as janelas e os painéis viram **espaços de trabalho**: um
  arquivo com a grade, as referências às conexões (nunca senhas) e a posição e o tamanho de
  cada janela (`WindowHandle::geometry`).
- **Limites.** O `WebView` só funciona na janela principal, não há janelas modais (o
  `ContentDialog` fica dentro da janela) e `tomni-ui lint`/`screenshot`/MCP olham a janela
  principal.
- **Teste.** O `TestApp` acompanha as janelas: `app.windows()`, `app.current_window()`,
  `app.switch_to(janela)` (daí em diante todos os métodos agem nela), `app.close_window(janela)`
  (passa pelos `on_close_requested`; `false` se recusaram), `app.exited()` (a principal foi
  fechada), `app.set_active_window(janela)`, `app.drag_tab_to_window("aba", janela, "alvo")`,
  `app.drag_tab_outside("aba")` e `.without_window_positions()` (o Wayland). Um pedido de
  `open_window` vira janela depois de `app.frame()`.

## Estado reativo

`use tomni_ui::prelude::*` traz tudo:

- `let n = signal(0);` cria. `n.get()` lê, `n.set(1)` troca, `n.update(|n| *n += 1)` muda.
  Signals são `Copy`: use `move ||` nas closures.
- `let valido = memo(move || email.get().contains('@'));` deriva com cache.
- `effect(move || ...)` roda de novo quando o que leu muda (efeitos colaterais).
- A função da tela roda uma vez; só os widgets que dependem do signal alterado repintam.
- Para ler sem criar dependência: `n.get_untracked()` ou `untrack(|| ...)`.

## Acessibilidade e layout (o que o lint confere)

| Regra | O que exige | Como corrigir |
|---|---|---|
| `touch-target` | widgets interativos com pelo menos 44 × 44 px (24 × 24 na densidade compacta) | no compacto, nada a fazer: os widgets já têm 24 ou mais. No padrão, `.height(44)` (e `.width(44)` em botões estreitos); `Button` (também o `.hyperlink()`), `ToggleButton`, `Edit`, `NumberBox`, `CheckBox`, `ComboBox`, `AutoSuggestBox` e `Slider` sem `.header` têm 32 px e o `ToggleSwitch` sem `.header` tem 40: todos precisam de `.height(44)`. As opções do `RadioButtons`, as abas e o cabeçalho do `Expander` já têm 44 ou mais. Dentro da `TitleBar`, a regra não vale (a barra tem 32 ou 48 px, como a do Windows); nem nos itens da `MenuBar` (32 px, como no Windows) e no divisor do `Split` (8 px, que as setas movem sem arrastar) |
| `contrast` | texto com contraste AA (4,5:1; 3:1 em texto grande) e ícones com 3:1, sobre o fundo de verdade (o `Card`, o `Border` ou a ponta mais próxima do gradiente) | use as cores do tema (tokens); com `Color` fixa, escolha uma que passe nos dois temas ou ponha o texto num `Border` de fundo fixo. O texto de um `Card` desabilitado não entra; um ícone que só enfeita leva `.decorative()` |
| `clipped-text` | o texto cabe no widget | mais largura, texto mais curto ou menos `padding`/`gap` em volta |
| `offscreen` | nada fora da janela (dentro de um `ScrollViewer`, nada fora da área que dá para alcançar rolando) | ponha o que não cabe num `ScrollViewer` com `.grow(1)`; ou menos conteúdo, `gap` ou `padding`; teste em 390 × 844 |
| `missing-glyph` | só caracteres que as fontes embutidas têm | troque o sinal de menos "−" por "-", tire emojis e "º"/"ª" ("nº 5" vira "número 5"); português acentuado funciona |
| `missing-alt` | `Image` com texto alternativo | `.alt("o que a imagem mostra")` ou, se for só enfeite, `.decorative()` |
| `webview-in-scroll` | `WebView` fora de `ScrollViewer` (a página é uma janela do sistema e rola sozinha) | tire o `ScrollViewer` e dê ao `WebView` o espaço que sobra com `.grow(1)` |
| `duplicate-accelerator` | dois atalhos iguais valendo ao mesmo tempo (`.accelerator`, `.shortcut` de item de menu) | troque o atalho de um dos dois; a mensagem diz quem é o dono do primeiro |
| `missing-id` (aviso) | widgets interativos com `.id()` | `.id("nome")`, único na tela; num `RadioButtons` ou `TabView`, o id vai no grupo |
| `missing-label` (aviso) | `Card` clicável e botão só de ícone com nome | `.label("Tema")`: no card, o título que aparece nele; no `icon_only`, o que o botão faz |

Dê `.id()` a todo botão, campo, caixa, chave, grupo, slider, sanfona e barra: testes, CLI e MCP
encontram os widgets por ele. Widgets ocultos (`.visible(false)`) ficam fora do lint.

## Textos da lib (idiomas)

Os textos que **você** escreve (`Text::new`, `.label`, `.title`) são do app: traduza como
quiser, e `String` montada na hora entra direto. A lib escreve uns poucos sozinha, em português:
a dica do ✕ e do + das abas, "Voltar", os botões da janela e o ☰ da barra de título, o
"Configurações" do `NavigationView`, "Mostrar senha", "Aumentar", "Diminuir", "Limpar" e o "Buscar" do `AutoSuggestBox::query_icon`, o "Ligado"/"Desligado" do `ToggleSwitch`, a
"Ordem alfabética" do `PropertyGrid`, o aviso de colar várias linhas, os menus e o painel vazio
do `TerminalPanes`, "[conexão encerrada]" e a falha do `WebView`.

Para trocar, parta do padrão, mude os campos e chame `tomni_ui::set_strings` **antes de montar a
tela** (no início do `main`): a tabela é por thread e vale para os widgets montados depois.

```rust
let mut s = tomni_ui::Strings::default();
s.tab_close = "Close tab".into();
s.back = "Back".into();
tomni_ui::set_strings(s);
```

`Strings` é `#[non_exhaustive]`: um campo novo da lib fica em português até você preenchê-lo, e o
app continua compilando. `{n}` (`pane_name`), `{count}` (`paste_warning_many`) e `{reason}`
(`connection_closed_reason`) são trocados pela lib. As **mensagens de erro** (conexão, arquivos,
agente) não passam por aqui: chegam como a lib as escreve. Exemplo: `examples/strings.rs`.

## Testes

Em `[dev-dependencies]`: `tomni-ui-test`. O `TestApp` monta a tela sem janela (400 × 300,
tema claro) e roda um frame depois de cada evento:

```rust
use tomni_ui_test::TestApp;

#[test]
fn login_habilita_com_email_e_senha() {
    let mut app = TestApp::new(login).size(390, 844);
    assert!(!app.enabled("entrar"));
    app.type_into("email", "ana@exemplo.com")
        .type_into("senha", "123456");
    assert!(app.enabled("entrar"));
    app.click("entrar");
    assert_eq!(app.label("entrar"), "Bem-vindo!");
    app.assert_lint();
}
```

Num crate binário, os testes de `tests/` não enxergam o `main.rs`: escreva o teste no mesmo
arquivo (`#[cfg(test)] mod tests { use super::*; ... }`) ou mova as telas para um `src/lib.rs`.
`Key` e `Modifiers` vêm de `tomni_ui::{Key, Modifiers}` (`Modifiers::SHIFT`, `Modifiers::CTRL`).
As teclas: `Enter`, `Space`, `Tab`, `Escape`, `Backspace`, `Delete`, `Left`, `Right`, `Up`,
`Down`, `Home`, `End`, `PageUp`, `PageDown`, `F(5)` (as teclas de função são `Key::F(n)`) e
`Char('a')` (as setas não têm `Arrow` no nome).

- Ações: `click(id)`, `double_click(id)`, `middle_click(id)` (o botão do meio: fecha a aba),
  `hover(id)`, `type_into(id, texto)` (clica, vai ao
  fim e digita), `type_text(texto)` (no foco), `key(Key::Enter)`, `key_with(Key::Tab,
  Modifiers::SHIFT)`, `tab()`, `shift_tab()`,
  `shortcut('c')` (Ctrl/Cmd+C) ou `shortcut(Shortcut::ctrl_shift('W'))` (qualquer atalho),
  `context_menu(id)` (o botão direito no widget, que abre o `.context_flyout`; veja "Desktop"),
  `ime_preedit`/`ime_commit`, `resize(l, a)`, `scroll(id, dy)` (rola o `ScrollViewer` `id`;
  `dy` positivo desce), `scroll_x(id, dx)`, `wheel(ponto, Vec2)` (a roda num ponto),
  `drag(de, ate)` (arrasta o mouse entre dois ids ou dois `kurbo::Point`, ou um de cada:
  `drag("tab-1", "tab-3")`, `drag(ponto, ponto)`; passa pelo limiar de 4 px como uma pessoa;
  `drag_to(id, ponto)` é o mesmo com o destino num ponto), `press(id)` + `move_to(ponto)` (o
  arrasto em partes, para conferir a linha de inserção antes de soltar com `release()`),
  `drag_label()` (o texto da etiqueta de arrasto, `None` sem arrasto), `set_value(id, n)` (clica na
  trilha do `Slider` no ponto do valor; no `NumberBox`, digita o número e confirma com
  Enter), `press(id)` e `release()` (segura e solta, para o olho da senha ou os botões de
  incremento, que repetem a cada `tomni_ui::SPIN_REPEAT` passado com `wait`),
  `close_tab(id, chave)` (passa o mouse na aba e clica no X; falha se o `TabView` não tem
  `.on_close`). Todas devolvem o app e podem ser encadeadas:
  `app.click("termos").click("criar")`.
- As ações por id rolam os `ScrollViewer` em volta até o widget aparecer, como uma pessoa
  faria: `click("remover-50")` funciona com a tarefa 50 fora da área visível.
- Popups: `is_open(id)` (pelo id do `ContentDialog`, `Flyout` ou `MenuFlyout`; pelo id do
  `ComboBox` ou do `AutoSuggestBox`, a lista; pelo id do `NumberBox`, o popup dos botões de
  incremento; `"{id}-tooltip"`, a dica; pelo id do `Expander`, se está aberto), `select(id, opção)` (clica no `ComboBox` e na opção, como uma pessoa; num
  `RadioButtons`, clica na opção; num `TabView`, na aba com esse **título**; num
  `NavigationView`, no item com esse **rótulo**, abrindo antes o painel ou o grupo, se
  preciso), `wait(duração)`
  (passa o tempo: a dica abre depois de `tomni_ui::TOOLTIP_DELAY`). Com um popup aberto, `click` num widget de trás só fecha o popup.
- Leitura: `label(id)` (botão, também o `.label` do `icon_only`, `ToggleButton`, `CheckBox`, o `.label` do `Card`, o `.header` do `ToggleSwitch`, do `Slider` ou do
  `RadioButtons`, item
  de menu ou do `ComboBox`, opção de rádio, título da aba, cabeçalho do `Expander`, rótulo do
  item do `NavigationView`), `text(id)`
  (campo, com o valor real mesmo com senha; `Text`; mensagem da `InfoBar`; valor do `ComboBox`,
  do `RadioButtons` ou do `AutoSuggestBox`; a chave da aba escolhida no `TabView` ou do item
  escolhido no `NavigationView`; o rótulo de um item dele), `tab_titles(id)` (os títulos das
  abas do `TabView`, em ordem),
  `value(id)` (`f64` do `Slider` ou do `NumberBox`, o andamento de 0 a 1 da `ProgressBar`/`ProgressRing`, o
  nível do `LevelMeter` ou o ponto mais novo do `AreaChart`/`Sparkline`; falha no indeterminado e na série vazia), `values(id)` (a série inteira do `AreaChart`/`Sparkline` ou os valores do `HeatMap`), `checked(id)` (`CheckBox` marcada, `ToggleButton` ou `ToggleSwitch` ligado, opção de rádio
  escolhida, aba ou item do `NavigationView` escolhido), `visible(id)`, `enabled(id)`, `focused()` (id com foco),
  `rect(id)` (na janela, já com a rolagem; um `kurbo::Rect` com `x0`, `y0`, `x1` e `y1`),
  `in_view(id)` (`false` se rolado para fora), `scroll_offset(id)` e `scroll_content(id)` (do
  `ScrollViewer`), `tree()` (um `tomni_ui::Node`, não texto: `tree().find(id)` devolve o nó,
  com `kind`, `label`, `text`, `value`, `open`, `checked`, `icon` (o nome do `Symbol`), `alt`
  e `error` da `Image`, `children`...; no `NavigationView`, `variant` é o modo (`"expanded"`,
  `"compact"`, `"minimal"`), `open` diz se o painel está aberto e, num item, `selected` e, num
  grupo, `open`; no `TabView`, `tabs` (a chave e o título de cada aba) e `closable`; `density`
  onde a densidade muda; para ver tudo,
  `{:#?}`).
- Widget criado por `For` ou `Show` só existe enquanto o item (ou a condição) existe:
  `app.ui().find("tarefa-3").is_none()` confere que saiu.
- `repaints_since_last()`: ids repintados desde a última chamada. Rolar repinta só o
  `ScrollViewer` (a barra); adicionar um item repinta o item e os que mudaram de lugar na tela.
- `click` numa `InfoBar` clica no X (o botão de ação é `click("<id>-acao")`). `click`, `hover` e `double_click` num widget oculto
  falham (o teste para com "está oculto").
- Janela (com `TitleBar`): o `TestApp` faz o papel do sistema. `window_requests()` devolve o
  que a barra pediu desde a última chamada (`[tomni_ui::WindowRequest::Close]` depois de
  `click("barra-de-titulo-fechar")`; `Minimize`, `ToggleMaximize`, `Drag`, `Resize(borda)`),
  `is_maximized()` (o duplo clique na barra maximiza de verdade), `set_maximized(bool)`,
  `set_window_active(false)` (outro app em primeiro plano: a barra apaga) e, no começo,
  `.chrome(tomni_ui::WindowChrome::System)` (a plataforma que mantém a barra do sistema). O centro da
  barra pode ser a busca: para o duplo clique, use `click_at` num ponto livre.
- Conferências: `assert_lint()` (falha com os erros do lint), `lint()` (a lista),
  `assert_snapshot("nome")` (compara com `snapshots/nome.png`; aprove com
  `TOMNI_UI_UPDATE_SNAPSHOTS=1 cargo test`).
- `.theme(Theme::Dark)` (`tomni_ui::theme::Theme`) e `.size(l, a)` no começo.

## Ferramentas

A CLI e o MCP compilam o app (`cargo build`) e o rodam em modo de ferramentas, sem janela.
Com mais de um executável no pacote, passe `--bin NOME` ou `--example NOME`.

| CLI | MCP | O que faz |
|---|---|---|
| `tomni-ui screenshot --view X --size 390x844 --theme dark` | `ui_screenshot` | PNG da tela (o caminho sai na saída) |
| `tomni-ui tree --view X` (`--json`) | `ui_tree` | widgets com id, tipo, retângulo e estado (`checked`/`unchecked`, severidade da `InfoBar`, `hidden` nos ocultos, `scroll=(x, y) content=L×A` no `ScrollViewer`, `scrolled-out` no que está rolado para fora; popups dentro de quem os abre, `open` ou `closed`, e `selected` no item escolhido do `ComboBox`; `value=50 range=0..=100` no `Slider`, `value=0.25` ou `indeterminate` no progresso, `open`/`closed` no `Expander`, `checked`/`unchecked` nas opções do rádio, `selected` na aba escolhida, `closable` no `TabView` com `.on_close` e `reorderable` com `.on_reorder`, `row`/`column` e o tamanho (`value=220`) no `split` (as partes são `split_pane` e `split_sizer`), `menu_bar` com os `menu_bar_item` (`open`/`closed`), `checked`/`unchecked` no `toggle_menu_item` e no `radio_menu_item` e `open`/`closed` no `menu_sub_item`, `keys=Ctrl+N` em quem tem atalho e `context-menu` em quem tem `.context_flyout`, `density=compact` onde a densidade muda, `background=` e `radius=` no `Card` e no `Border`, o degrau da rampa (`title`, `body`...) e `trimmed` no `Text` cortado com reticências; `icon=Save` no `Icon` e nos botões com ícone, `decorative` no que é enfeite; o `Stretch`, `alt="..."` e, se não abriu, `error="..."` na `Image`; `active`/`inactive` na `title_bar`, com os botões `window_minimize`, `window_maximize` e `window_close`; no `webview`, o título da página, `url="..."`, `back`/`forward` quando dá para voltar ou avançar, `loading`/`ready`/`failed` e, se falhou, `error="..."`; na `PropertyGrid`, as `properties` (a senha mascarada) e as `categories`, cada uma com `hidden` quando `.hidden` a esconde, e `alphabetical`, e nas linhas `property_grid_row` o valor em `text=`, `selected` e `changed` quando difere do `.default`, e `open`/`closed` nas `property_grid_category`) |
| — | `ui_interact` | ações (`click`, `type`, `key`, `drag`, `context`...) e devolve screenshot e árvore |
| `tomni-ui lint` (`--view X`, `--allow REGRA`) | `ui_lint` | regras acima; sem `--view`, todas as telas nos dois temas |
| `tomni-ui run` | — | abre a janela |
| `tomni-ui build --target windows\|linux\|macos\|android` | — | binário de release (ou APK) com o THIRD-PARTY-NOTICES |

Ações do `ui_interact`: `{"action": "click", "id": "entrar"}`, `double_click`,
`middle_click` (o botão do meio: fecha a aba), `hover`,
`{"action": "drag", "id": "notas-1", "to": "notas-3"}` (arrasta um widget até outro; no lugar
de `to`, `"x"` e `"y"` dão um ponto da janela; a resposta diz o que foi arrastado),
`{"action": "context", "id": "lista"}` (o botão direito: abre o menu de contexto do widget ou
do contêiner em volta; dá erro se nenhum tem `.context_flyout`),
`{"action": "click_at", "x": 10, "y": 20}`, `{"action": "type", "id": "email", "text":
"ana@x.com"}`, `{"action": "key", "key": "space"}` (alterna a caixa ou a chave com foco),
`{"action": "key", "key": "enter", "modifiers": ["shift"]}`, `ime_preedit`,
`ime_commit`, `{"action": "scroll", "id": "lista", "dy": 300}` (e `dx`; teclas `pageup`,
`pagedown`, `home`, `end`, `f1` a `f12`; um atalho é a tecla com os modificadores:
`{"action": "key", "key": "n", "modifiers": ["ctrl"]}`), `{"action": "select", "id": "uf", "option": "SP"}` (escolhe no
`ComboBox`, no `RadioButtons`, pelo título, a aba do `TabView` ou, pelo rótulo, o item do
`NavigationView`), `{"action": "set_value",
"id": "volume", "value": 70}` (clica no trilho do `Slider`; num `NumberBox`, digita e confirma) e `{"action": "wait", "ms": 500}` (passa o tempo, para a dica abrir). As ações por
id rolam até o widget antes; se ele estiver atrás de um popup aberto, a resposta avisa. O estado
continua entre chamadas até o app ser recompilado, trocar de tela ou `reset: true`.

A CLI sempre abre a tela no estado inicial. Para ver a tela depois de uma interação, use o
`ui_interact` (ele devolve o screenshot) ou, sem MCP, um teste com
`TestApp::assert_snapshot`.

**Janela parada gastando CPU?** Parada, a janela não pinta nenhum quadro: só um evento, um
prazo (uma animação à mostra, como o `ProgressBar` e o `ProgressRing` sem `.value`, a contagem
da dica) ou o pisca do cursor (530 ms, só com um campo com foco e a janela com o foco do
sistema; sem ele, o cursor fica apagado e parado, como no WinUI 3) pedem quadro. Rode o app com
`TOMNI_UI_TRACE_FRAMES=1`: cada janela escreve no stderr, uma vez por segundo em que pintou,
quantos quadros foram e por quê:

```text
tomni-ui: quadros de Janela(0): 60 em 1,0 s, 60 na tela (prazo 60); prazo de: progress_ring NodeId(812v1)
```

`evento` é tecla, ponteiro ou tamanho; `prazo`, o que o `Ui::deadline` espera (com os widgets);
`pisca`, o cursor; `superfície`, a superfície da GPU refeita; `sistema`, um redesenho que o
sistema pediu. Para guardar num teste que a tela parada não pede quadro:
`assert_eq!(app.ui().deadline(), None)` e `assert!(!app.ui().frame())` depois de alguns
`app.frame()`.

Há também uma rede de segurança, sempre ligada: se uma janela pinta mais de 20 quadros por
segundo, sem parar, por 5 s sem nenhuma entrada (tecla, ponteiro, roda, toque, IME, arquivos
soltos, tamanho ou foco), o runner limita o ritmo dela a 10 quadros por segundo e avisa no
stderr, uma vez por episódio, mesmo sem o rastro:

```text
tomni-ui: Janela(0) pintou 60 quadros/s por 5 s sem entrada; limitando a 10/s (motivos: superfície 60)
```

A primeira entrada devolve o ritmo normal na hora, e nenhum quadro se perde (o pedido que chega
cedo sai no próximo horário). A linha é a denúncia de um defeito, não algo a silenciar: veja o
motivo e, com `prazo`, os widgets (`; prazo de: ...`), e rode com `TOMNI_UI_TRACE_FRAMES=1` para
o rastro. A saída de um terminal, o `wake()` de outra thread e as animações não contam como
entrada: um terminal despejando texto sem ninguém mexer, ou um `ProgressRing` à mostra por mais
de 5 s, também caem nos 10 quadros por segundo (e no aviso). O `TestApp` não passa pelo runner:
os testes não mudam.

### Registro de travamentos

`App::new(tela).stall_log(pasta.join("travamentos.log"))` liga uma thread vigia que escreve no
arquivo quando a interface fica parada mais de 2 s numa etapa: qual (`evento`, `acordar`,
`ciclo › prazos › terminal`, `desenho › textura`, `desenho › apresentação`, `efeitos`, `layout`,
`gravação`...), o estado da thread da interface (rodando, dormindo, esperando o disco), a carga,
a memória do app e do sistema, o swap e as threads do app que mais gastaram CPU. Repete a cada
10 s e diz quanto durou. A cada minuto em que a interface usou mais de 30% de uma CPU, ou uma
etapa passou de meio segundo, escreve o tempo por etapa; o aviso do ritmo limitado também vai
para lá. O estado do sistema só aparece no Linux; o arquivo recomeça acima de 1 MiB. Sem o
`stall_log`, nada é medido. Para marcar uma etapa sua (uma conta pesada num efeito), use
`let _etapa = tomni_ui::stages::stage("minha conta");`: o guarda fecha a etapa ao sair de
escopo, e ela aparece dentro da etapa em que estava (`ciclo › prazos › minha conta`).

## Erros comuns

- "`&str` não é uma view": texto solto como filho. Use `Text::new("...")`.
- Um quadrado no lugar de um caractere: o lint `missing-glyph` diz qual é e o que usar.
- "não serve como propriedade": tipo errado num builder. Texto que muda vira closure:
  `Button::new(move || format!("{}", n.get()))`.
- O valor não atualiza na tela: leia o signal **dentro** de uma closure passada ao widget,
  não antes (`let x = n.get();` congela o valor).
- `nenhum widget com id`: confira o `.id()`; a mensagem lista os ids que existem. Num item de
  `For` ou dentro de `Show`, o widget só existe enquanto o item ou a condição existe.
- O item da lista não atualiza ao mudar um campo dele: a view do `For` é criada uma vez por
  chave. Ponha o campo num `Signal` dentro do item.
- O clique num botão não fez nada (no teste ou no `ui_interact`): havia um popup aberto, e o
  clique fora só o fechou. Escolha o item, feche com Esc ou clique de novo.
- Tudo some ou fica espremido numa lista longa: falta o `ScrollViewer` (com `.grow(1)` numa
  `Column`).
- O `TabView` mostra a primeira aba e não a do signal: o signal guarda a chave (`"avancado"`),
  não o título (`"Avançado"`).
- `select("config", "avancado")` não acha a aba: o `select` usa o título, como uma pessoa
  leria (`"Avançado"`); o id da aba é que usa a chave (`config-avancado`). Vale também para o
  rótulo e a chave dos itens do `NavigationView`.
- O X (ou o Ctrl+F4) não fecha a aba: o `on_close` só avisa; tire o item da lista nele.
- Campos altos demais numa tela compacta: sobrou um `.height(44)`, que o lint só pede no
  padrão. Tire-o.
- A screenshot e o snapshot mostram a `TitleBar` como parte da tela (ela é do app); a barra
  do sistema nunca aparece neles.
- `click("nav-fotos")` falha porque o item não aparece: ele está num grupo fechado, no popup
  do painel compacto ou no painel mínimo fechado. Use `select("nav", "Fotos")`, que abre o
  caminho como uma pessoa, ou teste numa janela larga (`.size(1280, 800)`).
- O atalho do item de menu não roda no teste: `shortcut('n')` é o Ctrl+N; para outro
  modificador ou uma tecla de função, passe um `Shortcut` (`Shortcut::ctrl_shift('W')`,
  `Shortcut::key(Key::F(5))`). Com um popup aberto ou o foco num campo que usa a tecla, ela
  não chega ao atalho.
- `lint` com `duplicate-accelerator`: o mesmo `Shortcut` em dois widgets que aparecem ao mesmo
  tempo. O item de um menu fechado não conta; troque o atalho de um deles.
- `context_menu("l1")` dá erro "não tem menu de contexto": o `.context_flyout` está em outro
  widget, que não é o dele nem um contêiner em volta. Ponha-o na lista ou na `Column` que
  envolve os itens.
- A aba não arrasta: falta o `.on_reorder` no `TabView`. O arrasto só começa depois de 4 px;
  `click` nunca arrasta.
- O arrasto não muda a ordem na tela: o `on_reorder` só avisa. Mova o item na lista que
  alimenta o `.items` (`v.remove(de)` e `v.insert(para, item)`).
- O divisor do `Split` não se mexe: falta `.size(signal)` com o signal lido por mais ninguém
  além dele, ou o `.min_first`/`.min_second` não deixam espaço. `app.value("id-do-split")`
  lê o tamanho.
<!-- tomni-ui:end -->
