# Mudanças

O formato segue o [Keep a Changelog](https://keepachangelog.com/pt-BR/1.1.0/), e as versões, o
[SemVer](https://semver.org/lang/pt-BR/). Na série 0.x, uma versão menor pode quebrar a API.

## [Não lançado]

### Adicionado

- `request_focus_in(janela, id)`: o pedido de foco de um efeito, que roda no quadro de qualquer
  janela, com a janela que atende primeiro.

### Corrigido

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
