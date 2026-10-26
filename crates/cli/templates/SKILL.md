---
name: tomni-ui
description: Criar, mudar, testar e verificar telas de um app Tomni UI (UI em Rust com visual Fluent). Use ao mexer em widgets, layout, signals ou testes de UI, ou para ver a tela (screenshot), inspecionar a árvore de widgets e rodar o lint de acessibilidade.
---

# Tomni UI

O guia completo da API está no `AGENTS.md` da raiz do projeto: leia antes de escrever código.

## Ciclo de cada mudança

1. Edite a tela (`fn tela() -> impl View`) e dê `.id()` a cada widget interativo (botão,
   campo, `NumberBox`, `ToggleButton`, `CheckBox`, `ToggleSwitch`, `ComboBox`, `RadioButtons`,
   `Slider`, `Expander`, `TabView`, `AutoSuggestBox`, `InfoBar` com X), com `.height(44)` onde
   o lint pedir. App de desktop (mouse e teclado): `.density(Density::Compact)` no `Form`, e
   aí sem `.height(44)` (o lint pede 24 px).
2. Escreva ou ajuste o teste com `tomni_ui_test::TestApp` (pelos ids) e termine com
   `app.assert_lint()`.
3. `cargo test`.
4. `tomni-ui lint` (ou a ferramenta `ui_lint`): precisa sair sem erros.
5. Veja o resultado: `tomni-ui screenshot --view tela` (ou `ui_screenshot`) e abra o PNG.
   Confira também o tema escuro (`--theme dark`) e uma tela de celular (`--size 390x844`).
6. Para uma interação na tela de verdade, use `ui_interact` com ações como
   `{"action": "type", "id": "email", "text": "ana@x.com"}` e `{"action": "click", "id": "entrar"}`.

## Quando algo falha

- O lint diz a regra, o widget e como corrigir; siga a sugestão (`.height(44)`, mais largura,
  "-" no lugar de "−").
- Widget que some e volta: `.visible(move || ...)`; oculto, ele não recebe clique no teste.
  Para criar e descartar (listas, telas que trocam), `For::each` e `Show::when`.
- Lista que não cabe ou o lint `offscreen`: ponha num `ScrollViewer::new().grow(1)`; no
  teste, `app.scroll("lista", 500.0)` e `app.in_view(id)`. As ações por id rolam sozinhas.
- Item de `For` que não atualiza: a view é criada uma vez por chave; o que muda fica num
  `Signal` dentro do item.
- `nenhum widget com id`: a mensagem lista os ids existentes.
- Erro de compilação em português ("não é uma view", "não serve como propriedade"): as notas
  dizem a forma certa.
- Popup (`ComboBox`, `AutoSuggestBox`, `MenuFlyout`, `Flyout`, `ContentDialog`, dica): não
  ocupa espaço no layout. No teste, `app.is_open(id)`, `app.select("uf", "SP")` e, para a dica,
  `app.hover(id).wait(TOOLTIP_DELAY)`; abra cada um e chame `assert_lint()`, porque o
  `tomni-ui lint` só vê a tela com tudo fechado. Clique que "não fez nada": havia um popup
  aberto, e o clique fora só o fechou. No `MenuFlyout`, `.access_key('1')` (depois do item) dá
  a tecla de acesso e `.highlighted(signal)` informa o item em realce.
- Rádio, abas, slider e número no teste: `app.select("tema", "Escuro")`,
  `app.select("config", "Avançado")` (o título da aba), `app.set_value("volume", 70.0)` e
  `app.value("volume")` (também no `NumberBox`). O signal do `TabView` guarda a chave da aba,
  não o título. No `NavigationView`, `app.select("nav", "Fotos")` escolhe pelo rótulo (abre o
  painel ou o grupo antes, se preciso) e `app.text("nav")` dá a chave; teste os três modos
  com `.size(1280, 800)`, `.size(800, 600)` e o tamanho padrão. Os botões de incremento do `NumberBox` (com `.spin_buttons(SpinButtons::Inline)` ou
  `Compact`; o padrão é sem botões) e o olho da senha se seguram:
  `app.press("qtd-aumentar").wait(SPIN_REPEAT * 3).release()`.
- Abas que abrem e fecham: `TabView::items(lista, ...)` com `.on_close(move |chave| ...)`,
  que precisa tirar o item da lista, e `.on_add(move || ...)`. No teste,
  `app.close_tab("notas", "1")`, `app.middle_click("notas-2")`, `app.click("notas-adicionar")`
  e `app.tab_titles("notas")`.
- Barra de título própria (`Form::title_bar(TitleBar::new(...))`): no teste, o `TestApp` faz
  o papel do sistema: `app.click("barra-de-titulo-fechar")` e depois
  `app.window_requests()` dá `[WindowRequest::Close]`; o duplo clique num ponto livre da barra
  (`click_at`, porque o meio pode ser a busca) maximiza (`app.is_maximized()`).
  `app.set_window_active(false)` mostra a barra apagada. A busca e os botões dentro dela
  podem ter 32 px.
- Texto que passa da largura num card ou numa grade: `.wrap()` (quebra) ou `.max_lines(2)`
  (corta com reticências, sem erro no lint). Card clicável: `.id()`, `.label("Título")` e
  `.on_click`; o aviso `missing-label` pede o rótulo (também no `Button::icon_only`).
- Toda `Image` pede `.alt("o que mostra")` ou `.decorative()`; sem os dois, o lint dá o erro
  `missing-alt`. Ícone ao lado de um texto que diz o mesmo: `Icon::new(...).decorative()`. Os
  nomes do `Symbol` estão na lista do AGENTS.md.
- `Terminal` (features `terminal` e, para as conexões, `pty`, `ssh` ou `serial`): a conexão
  nunca falha no construtor; o erro aparece no próprio terminal. No teste, use um `Loopback`
  e `app.feed(id, bytes)`, ou, com um `Process`, repita `app.wait(...)` até
  `app.terminal_text(id)` mostrar a saída. O Tab vai ao programa: para sair do terminal com o
  teclado, F6. A senha do SSH é pedida pelo app num `Edit` com `.password()`.
  Imagem e arquivos colados ou soltos chegam em `.on_rich_paste(|c| ..)`; o handler cola o
  texto com `sessao.paste(..)` e monta caminhos com `shell_quote(..)`. No teste, o
  `app.clipboard()` aceita `write_files` e `write_image`.
- Chaves SSH (feature `ssh`): `tomni_ui::terminal::ssh::{keys, agent, copy_id}` geram chaves
  (`KeyGen`), leem e regravam arquivos (`KeyFile`), sobem um agente próprio (`Agent`) e
  instalam a pública no servidor (`copy_id`). `export_private`/`export_public` convertem entre OpenSSH, PKCS#8, PEM, `.ppk` e as públicas (RFC 4716, SPKI, PKCS#1); `Ssh::builder(..).key(chave)` usa uma chave já aberta; o agente recusa `sk-` e guarda certificados. `WindowHandle::hide()`/`show()` e `App::start_hidden` escondem janelas sem fechar; `tomni_ui::tray::Tray` põe o ícone na bandeja do sistema (guarde o `TrayHandle` ou `keep()`; só esconda ao fechar se `tray::available()`). Tudo bloqueia: rode numa thread e devolva o
  resultado por `Arc<Mutex<Option<..>>>` e um contador de signal. O AGENTS.md tem o código.
- `TerminalPanes` (feature `terminal`): vários terminais em abas e em grade, cada um numa
  `Session` (`Session::new(conexao).key("web-01")`), com o `PaneLayout` num `signal` e o envio
  para vários em `Broadcast`. `.identify(signal)` mostra o número de cada painel e `.highlight_pane(signal)` destaca um. Dê `.id()` e `.grow(1)`; chame `dispose()` na sessão em
  `on_session_closed`. No teste, `app.session_text(chave)` lê uma sessão (à mostra ou não) e
  `app.panes(id)` descreve os painéis; as sessões moram num registro da thread, então `dispose()`
  nelas antes de repetir o app em outro tema. A lib não guarda conexões nem senhas: o guia
  mostra como o app salva as conexões (sem a senha) e usa o cofre do sistema.
- `WebView` (feature `webview`; no Linux, o `libwebkit2gtk-4.1-dev`): dê `.grow(1)` e não o
  ponha num `ScrollViewer` (erro `webview-in-scroll`). Nos testes o motor é falso: depois do
  `TestApp::new`, dois `app.frame()` deixam a primeira página pronta; a conversa com a página
  sai com `app.webview_message(id, texto)` e `app.webview_posted(id)`. F6 tira o foco da página.
- App de desktop com divisor, menus e atalhos: `Split::row(a, b).size(signal)` (o divisor se
  arrasta; `app.value("id")` lê o tamanho), `MenuBar::new().menu("Arquivo", MenuFlyout::new()
  .item(..).shortcut(Shortcut::ctrl('O')))` (itens `barra-arquivo`), `.accelerator(Shortcut,
  move || ...)` e `.context_flyout(MenuFlyout)` em qualquer widget. No teste:
  `app.shortcut('o')` (Ctrl+O) ou `app.shortcut(Shortcut::key(Key::F(5)))`,
  `app.context_menu("lista")` e `app.is_open("lista-menu")`. Dois widgets com o mesmo atalho
  são o erro `duplicate-accelerator`. O `.shortcut(..)` do item aceita um `Signal<Shortcut>`
  (atalho editável; `"Ctrl+Shift+T".parse::<Shortcut>()` lê o texto). Abas que mudam de ordem: `.on_reorder(move |de, para|
  ...)` no `TabView` (o app move o item na lista); no teste, `app.drag("notas-1", "notas-3")`;
  no `ui_interact`, `{"action": "drag", "id": "notas-1", "to": "notas-3"}` e
  `{"action": "context", "id": "lista"}`.
- Árvore de pastas e itens (favoritos, servidores): `TreeView::bind(itens, selecionado)` com
  `TreeItem::folder/leaf` e `.on_move`/`.on_rename`/`.on_delete`, que só avisam: o app aplica
  em `itens.update(|v| v.move_to(..))` (trait `TreeItems`). No teste, `app.tree_items(id)` e
  `app.expand(id, chave)`; as linhas são `{id}-{chave}`. Linhas de um `DataGrid` caem nas
  pastas com `.on_grid_drop("grupo", ..)` (o grid usa `.drag_group("grupo")`).
- Tabela de linhas e colunas (arquivos, resultados): `DataGrid::bind(linhas, selecionadas)` com
  `DataGridRow::new(chave).cell(texto, CellValue::..)`, `DataGridColumn::new(chave, titulo)`,
  `.sort`/`.filter`/`.renaming` em signals e `.on_rename`/`.on_delete`/`.on_drop`, que só
  avisam. Só texto, ícone e `CellValue::Progress` nas células (sem widget). No teste,
  `app.grid(id)`, `app.grid_select(id, &[..])`, `app.click_with(id, Modifiers::COMMAND)`,
  `app.drag_rows(..)` e `app.drop_files(..)`.
- Arquivos locais e remotos (features `files`, `sftp`, `ftp`, `s3` e `rclone`): `tomni_ui::files` tem o trait `Fs`
  (`Local`, `Sftp::connect(&SshBuilder)` ou `Sftp::from_channel(..)` na sessão do terminal,
  `Ftp` pelo `files::ftp::FtpBuilder` (FTP, FTPS explícito e implícito, com a pergunta do
  certificado em `.check_certificate`), `S3` pelo `files::s3::S3Config` (Amazon, MinIO, R2 e
  outros; `S3::connect(config)` ou `config.opener()`), `Rclone` (`files::rclone`: as demais nuvens pelo binário `rclone`
  1.60+ que a pessoa instala; `Rcd::spawn(RcdConfig::new(find_rclone(None)?))` numa thread e `Rclone::new(rcd, "gdrive:")`;
  a configuração dos remotos e as perguntas do OAuth em `config_*` e `files::rclone::config::ConfigStep`) e o `MemFs` dos testes), a `Remote` (uma conexão numa thread; os pedidos devolvem `Reply<T>`, que
  a tela lê como um signal) e a fila `Transfers` (`add(origem, destino, Conflict)`, pausar,
  cancelar, retomar, e `asking()` para o diálogo "O arquivo já existe"). Monte a tela com
  `DataGrid`; o guia tem a seção "Arquivos remotos" e o `tirech-omni/crates/tirech-omni` é o exemplo.
- Página de configurações: `SettingsCard::new(titulo).description(..).icon(..).content(controle)` por opção (o controle desce em card estreito), `.on_click` para a linha toda clicável; `SettingsExpander::new(titulo, signal_bool).item(card)` agrupa. Ids `{id}-cabecalho` e `{id}-conteudo`.
- Propriedades de um objeto (editor de conexões): `PropertyGrid::new().category(..).text/number/choice/choice_from/check/file(nome, signal)` com `.default`, `.description`, `.read_only` e `.filter` e, para coleções, `.button(nome, resumo, || ..)` (um "…" que abre o seu diálogo); o texto e o número só vão ao signal ao confirmar (Enter). O botão "…" da `.file` usa `dialogs::open_file()`; no teste, `app.properties(id)` e `app.file_dialog_answer(..)`.
- Tema em tempo de execução: `tomni_ui::set_theme(Some(Theme::Dark))` (`None` volta ao do sistema); no teste, `app.set_theme(..)`. Pasta: `dialogs::pick_folder()`.
- `Terminal`: `.font_size(14.0)`, `.scheme(..)`, `.cursor_shape(..)`, `.right_click(RightClick::CopyPaste)` e `.bell(BellStyle::None)` aceitam `Signal` e valem na hora.
- Copiar texto num botão: `tomni_ui::clipboard::write(texto)` (`clipboard::clear_if(texto)` apaga depois, só se ainda for ele); no teste, `app.frame(); app.clipboard().read()`. Motivo de uma sessão encerrada: `sessao.close_reason()`.
- Com mais de um executável, passe `--bin NOME` ou `--example NOME` (no MCP, `bin`/`example`).
