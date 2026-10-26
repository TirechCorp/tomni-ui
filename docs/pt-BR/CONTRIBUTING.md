# Como contribuir com o Tomni UI

Obrigado pelo interesse. Este guia é para quem mexe **no código da lib**. Para **escrever um
app** com ela, o guia é o [`crates/cli/templates/AGENTS.md`](../../crates/cli/templates/AGENTS.md),
que o `tomni-ui init` copia para o projeto do app.

Antes de um pull request grande (um widget novo, uma mudança na API pública, uma dependência
nova), abra uma issue para combinar o desenho. Correções pequenas podem ir direto.

## Idioma

Documentação, comentários, mensagens de erro, nomes de testes e textos de exemplo em português
do Brasil, com acentos. Identificadores de API em inglês (`Button`, `on_click`). Não troque
acentos por ASCII. A documentação pública existe em português (`docs/pt-BR`) e em inglês
(`docs/en`): quem muda uma atualiza a outra, ou pede ajuda no pull request. Issues e pull
requests podem ser em português ou em inglês.

## Antes de abrir o pull request

Tudo isto precisa passar (é o que o CI roda):

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets --features tomni-ui-test/cjk,tomni-ui-test/gpu -- -D warnings
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps
cargo test --workspace
cargo test -p tomni-ui-test --features cjk
cargo deny check licenses
cargo run -p tomni-ui-cli -- lint -p tomni-ui --example login   # e os outros exemplos
cargo run -p tomni-ui-cli -- lint -p tomni-ui --example terminal -F terminal
cargo run -p tomni-ui-cli -- lint -p tomni-ui --example webview -F webview-widget
cargo run -p tomni-ui-cli -- lint -p tomni-ui --example ia -F ai,markdown
cargo run -p tomni-ui-cli -- lint -p m6-login                    # e os outros apps de aceite/
```

Com GPU (ou o lavapipe, `WGPU_ADAPTER_NAME=llvmpipe`):

```sh
TOMNI_UI_BACKEND=gpu cargo test -p tomni-ui-test --features gpu,cjk
```

A versão do Rust está fixada em `rust-toolchain.toml` (os snapshots dependem dela); a mínima
declarada é a 1.89 (`cargo +1.89 check --workspace --all-targets`).

O `WebView` com o motor do sistema fica fora do `cargo test --workspace`. Com o
`libwebkit2gtk-4.1-dev` instalado e uma tela (ou `xvfb-run`):

```sh
cargo clippy -p tomni-ui --features webview --all-targets -- -D warnings
cargo test -p tomni-ui --features webview --test webview_nativo
```

### Testes opcionais contra servidores reais

Ficam fora do CI e precisam do Docker. Rode o da área que você mexeu:

| Área | Comando | O que sobe |
| --- | --- | --- |
| FTP e FTPS (`crates/files/src/ftp/`) | `testes/ftp/rodar.sh` | vsftpd, proftpd e pure-ftpd (portas 2121 a 2991) |
| S3 (`crates/files/src/s3/`) | `testes/s3/rodar.sh` | MinIO (porta 19000) |
| rclone (`crates/files/src/rclone/`) | `TOMNI_UI_RCLONE=rclone cargo test -p tomni-ui-files --features rclone --test rclone_real -- --ignored` | o `rclone` do sistema (1.60 ou mais novo) e o `python3` |
| Nebula (`crates/nebula/`) | `testes/nebula/rodar.sh` | o `nebula` oficial (UDP 14242 a 14244) |
| ZeroTier (`crates/zerotier/`) | `testes/zerotier/rodar.sh` | o `zerotier-one` oficial, com um planet de teste (nada sai para a internet) |
| OpenVPN (`crates/openvpn/`) | `testes/openvpn/rodar.sh` | o `openvpn` 2.6 (portas 19194 a 19199) |
| WireGuard (`crates/wireguard/`) | `testes/wireguard/rodar.sh` | `wg` e `wg-quick` (UDP 51820) |
| Tailscale (`crates/tailscale/`) | `testes/tailscale/rodar.sh` | `headscale` e `tailscale` oficiais (`MANTER=1` deixa de pé) |
| Pasta do shell no SSH (`crates/terminal/src/ssh/pasta.rs`) | `testes/pasta/rodar.sh` | OpenSSH novo e antigo e Dropbear (portas 22201 a 22203) |

O CI cobre essas áreas sem rede: o servidor S3 em memória, o `rcd` falso, o `SimNet` (UDP e NAT
em memória) com os servidores de teste de cada VPN e o `ServerSession` do OpenVPN. As fixtures
de cada rede vêm do `gerar-fixtures.sh` da pasta correspondente.

## Estrutura

| Pasta | O quê |
| --- | --- |
| `crates/reactive` | signals (`reactive_graph`) |
| `crates/core` | widgets, árvore, foco, eventos, `IntoProp` e o modelo da bandeja do sistema |
| `crates/layout` | flex com `taffy` |
| `crates/text` | `parley` e as fontes embutidas |
| `crates/render` | `Canvas`, backends CPU (`vello_cpu`) e GPU (`vello`) |
| `crates/theme` | `Drawer` e `FluentDrawer` (tokens em `tokens.rs`) |
| `crates/platform` | janela `winit`, entrada, IME, seletor de arquivos, bandeja do sistema e Android |
| `crates/win32` | as chamadas Win32 (só Windows, o único crate com `unsafe`) |
| `crates/terminal` | o `Terminal`, o emulador (`vte`), a `Session`, o `TerminalPanes` e as conexões `Telnet`, `Process`, `Ssh` e `Serial` |
| `crates/files` | o `Fs` (local, SFTP, FTP, S3 e rclone), a `Remote` e a fila `Transfers` |
| `crates/net` | `Proxy` (SOCKS5 e HTTP CONNECT), `connect` e o agente HTTP |
| `crates/netstack` | a pilha TCP/IP em software (`smoltcp`), o filtro de saída e a ponte SOCKS5 das VPNs |
| `crates/openvpn`, `crates/wireguard`, `crates/tailscale`, `crates/nebula`, `crates/zerotier` | as redes embutidas; o núcleo de cada uma é sem E/S, com o relógio explícito |
| `crates/ai` | o cliente de IA (OpenAI e Anthropic) e o servidor falso `FakeAi` |
| `crates/webview` | `WebView`, `WebViewController`, o motor do sistema (`wry`) e o falso |
| `crates/markdown` | o `MarkdownView`, o realce e as edições de Markdown |
| `crates/tomni-ui` | a fachada: `Ui`, `App`, `inspect`, `lint` e `tools` (o protocolo) |
| `crates/tomni-ui/examples` | um exemplo por widget |
| `crates/test` | o `TestApp`, os snapshots e os testes de integração |
| `crates/cli` | o cliente do protocolo e o binário `tomni-ui` |
| `crates/cli/templates` | o `AGENTS.md`, o `CLAUDE.md`, o `SKILL.md` e o `mcp.json` do `tomni-ui init` |
| `crates/mcp` | o binário `tomni-ui-mcp` (JSON-RPC sobre stdio) |
| `aceite/` | apps escritos por agentes só a partir do guia; membros do workspace, testados e no lint |
| `testes/` | os ambientes Docker dos testes opcionais |

## Regras que o compilador não confere

- **Visual e comportamento**: o WinUI 3 (`microsoft/microsoft-ui-xaml`) é a referência de
  cores, medidas, propriedades e comportamento dos widgets. Quando o WinUI não define algo, ou
  não dá para segui-lo (um glifo que a Selawik não tem, um popup que sairia da janela), use o
  que o `fluent_ui` (o pacote Flutter do Fluent) faz e registre a adaptação no doc do widget.
- **Snapshots**: só aprove (`TOMNI_UI_UPDATE_SNAPSHOTS=1`) depois de abrir o `.snap.new.png` e
  o `.snap.diff.png` e concordar com a mudança. As referências são da CPU determinística.
- **Exemplos**: cada arquivo em `crates/tomni-ui/examples` tem um `#[test]` em
  `crates/test/tests/exemplos.rs` (um teste falha se faltar) e passa no lint nos dois temas. As
  telas são `pub fn` para o teste enxergar.
- **Protocolo das ferramentas**: `tomni_ui::tools` (o app) e `tomni_ui_cli` (o cliente)
  repetem `PROTOCOL`, `ENV`, `PREFIX` e os nomes das regras do lint; um teste confere. Mudou o
  formato de um pedido ou de uma resposta? Suba o `PROTOCOL` nos dois.
- **Guia dos apps**: mudou a API pública (widget, builder, `TestApp`, CLI, MCP)? Atualize o
  `crates/cli/templates/AGENTS.md`, o `llms.txt` e, se for o caso, o `SKILL.md`. O guia precisa
  bastar para uma sessão nova de um agente construir uma tela que passa nos testes e no lint.
- **Mensagens do compilador**: `Widget`, `View` e `IntoProp` têm
  `#[diagnostic::on_unimplemented]`; mantenha as notas com a forma certa de escrever.
- **Dependências**: só licenças da lista do `deny.toml`. Os `about.hbs` e `about.toml` da raiz e
  de `crates/cli/templates` precisam ser iguais (há teste).
- **Reatividade**: código do runtime que lê signals fora de um efeito (eventos, foco) usa
  `untrack`.
- **Apps de `aceite/`**: o código fica como o agente entregou. Uma mudança na lib que os quebra
  também quebraria apps escritos a partir do guia: reveja a mudança ou o guia.
- **Testes sem rede nem aparelho**: um servidor `russh` local, um par de pseudoterminais, o
  `sh` e os servidores falsos. As gravações de `crates/test/tests/gravacoes/` são saída real de
  programas.
- **`unsafe`**: o lint do workspace nega. As exceções são o `android_main` (com `#[allow]`
  justificado) e o crate `tomni-ui-win32`, com um `unsafe` por bloco e um `// SAFETY:` em
  cada. O `std::env::set_var` também não entra: passe o valor pelo builder ou pelo processo
  filho.
- **Código de terceiros**: o OpenVPN é GPL-2.0, então o `tomni-ui-openvpn` é escrito a partir
  da descrição do protocolo, sem copiar nem traduzir o código do projeto. Ao portar código com
  licença permissiva (como no WireGuard, no Tailscale e no Nebula), registre a origem e a
  licença no `NOTICE` do crate.

## Ferramentas no próprio repositório

```sh
cargo run -p tomni-ui-cli -- screenshot -p tomni-ui --example login --theme dark
cargo run -p tomni-ui-cli -- tree -p tomni-ui --example layout --view linha
cargo run -p tomni-ui-mcp -- -p tomni-ui --example login   # servidor MCP
```

As saídas vão para `target/tomni-ui/`.

## Licença das contribuições

Salvo indicação em contrário, toda contribuição enviada para inclusão no projeto, como
definido na licença Apache-2.0, é licenciada como MIT ou Apache-2.0, à escolha de quem usa, sem
termos ou condições adicionais.
