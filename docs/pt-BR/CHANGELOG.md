# Mudanças

O formato segue o [Keep a Changelog](https://keepachangelog.com/pt-BR/1.1.0/), e as versões, o
[SemVer](https://semver.org/lang/pt-BR/). Na série 0.x, uma versão menor pode quebrar a API.

## [Não lançado]

### Adicionado

- `request_focus_in(janela, id)`: o pedido de foco de um efeito, que roda no quadro de qualquer
  janela, com a janela que atende primeiro.
- `tomni_ui_net::HttpProxyServer`: um proxy HTTP local no `127.0.0.1` que leva cada pedido
  (`CONNECT` ou HTTP puro em forma absoluta) por uma função de discagem do app, com o nome sem
  resolver. Serve para abrir um navegador que sai por outra rede.
- `AreaChart`, `Sparkline` e `LevelMeter`: o gráfico de área do Gerenciador de Tarefas (a
  série inteira, `.max`, `.capacity` de 60 pontos), a miniatura dele e o medidor de quadradinhos
  do amarelo-claro ao vermelho-escuro, que vira botão com `.on_click`. `TestApp::value` lê o
  nível do medidor e o ponto mais novo do gráfico. Exemplo `desempenho`.
- `DataGrid`: `DataGridRow::heat(0..=1)` pinta o fundo da célula conforme o peso, e
  `CellValue::Sparkline` desenha uma minissérie no lugar do texto.
- Terminal: `Session::sent()`, `Session::busy()` (o tempo de ler, interpretar e pintar) e
  `Session::threads()` (as threads da conexão, para medir a CPU de cada aba; no Linux).
- `tomni_ui_win32::process_usage(pid)`: o tempo de CPU e a memória de um processo no Windows.
- O ícone `Symbol::DataArea`.

- `App::stall_log(caminho)` (e `WindowOptions::stall_log`): o registro de travamentos. Uma
  thread vigia escreve quando a interface fica parada mais de 2 s numa etapa (o evento, o quadro,
  o layout, os terminais, a textura da janela...), com o estado da thread, a carga, a memória e
  as threads mais ocupadas (no Linux), e o tempo por etapa a cada minuto pesado.
  `tomni_ui::stages::stage("nome")` marca uma etapa do app.
- Terminal: `INBOX_LIMIT`, `Inbox::push_wait`, `Inbox::has_room` e `Inbox::detach`.
- `close(&self)` nos motores de VPN (`Nebula`, `WireGuard`, `Tailscale`, `ZeroTier` e
  `OpenVpn`): pede para parar sem esperar e sem consumir o motor, para quando ele está num `Arc`
  dividido com as conexões.

### Mudado

- `TitleBar::subtitle` vazio some e não deixa espaço, como o `Collapsed` do WinUI: uma closure
  pode mostrar o subtítulo só quando faz sentido.
- O `TestApp` passa o título do `Form` à janela a cada quadro, como o runner: um título que muda
  aparece em `WindowHandle::title`, inclusive o da janela principal.
- O `MenuFlyout` mais alto que a janela fica do tamanho dela e rola, como no WinUI, em vez de
  cortar os itens de baixo.

### Corrigido

- Um terminal com saída sem fim (`yes`, um log enorme) fazia a memória crescer sem limite e a
  janela parar de responder. A fila de cada conexão agora tem 1 MiB: o programa espera, como
  num terminal do sistema, e o Ctrl+C continua chegando; no SSH, os outros canais da mesma
  conexão (SFTP, `exec`, túneis) seguem andando. O `wake` de outra thread não enche mais a fila
  de eventos da janela.

- Bandeja no Cinnamon e no MATE: o host do Ayatana ignora o `IconPixmap` e mostrava uma vaga
  vazia. O ícone agora também vai como um PNG em `$XDG_RUNTIME_DIR/tomni-ui/`, pelo caminho no
  `IconName`, como o Qt faz; o arquivo some quando o ícone sai.
- A fila de transferências atende ao "Cancelar" entre as escritas parciais, sem esperar o
  buffer inteiro sair num servidor lento.

- Com a mesma tela montada em duas janelas, `request_focus`, `request_focus_inside`,
  `start_bring_into_view` e `TextEditorController::focus` davam o foco à janela que rodasse o
  quadro primeiro. Agora a janela do evento que pediu atende primeiro, e as outras só quando ela
  não tem o widget.

- SSH: as chaves da conexão (`key_file`, `key`) vão ao servidor antes das outras do agente, como
  no OpenSSH. Com muitas chaves no agente, o servidor (`MaxAuthTries`) derrubava a conexão antes
  de chegar à certa, e o erro dizia que o agente tinha recusado a chave.
- SSH: um arquivo de chave que não abre não impede mais o agente de autenticar.

## [0.1.0] - 2026-10-26

Primeira versão pública.

- Widgets do WinUI 3, tema claro e escuro, ícones Fluent, `TitleBar` e fundo Mica.
- Reatividade por signals, com repaint só do que mudou.
- Backends CPU (determinístico, para os snapshots) e GPU, no Windows, Linux, macOS e Android.
- `TestApp`, snapshots, a CLI `tomni-ui` e o servidor MCP `tomni-ui-mcp`.
- Terminal com processo local, Telnet, SSH e serial.
- Arquivos locais, SFTP, FTP e FTPS, S3 e rclone.
- Redes embutidas: OpenVPN, WireGuard, Tailscale, Nebula e ZeroTier.
- Cliente de IA, `WebView` e Markdown.

[0.1.0]: https://github.com/TirechCorp/tomni-ui/releases/tag/v0.1.0
