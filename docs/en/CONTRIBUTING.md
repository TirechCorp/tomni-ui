# Contributing to Tomni UI

Thanks for your interest. This guide is for people working **on the library's code**. To
**write an app** with it, the guide is
[`crates/cli/templates/AGENTS.md`](../../crates/cli/templates/AGENTS.md), which `tomni-ui init`
copies into the app's project.

Before a large pull request (a new widget, a change to the public API, a new dependency), open
an issue to agree on the design. Small fixes can go straight in.

## Language

Code documentation (rustdoc), comments, error messages, test names and example texts are in
Brazilian Portuguese, with accents. API identifiers are in English (`Button`, `on_click`). Don't replace accented
characters with ASCII. The public documentation exists in Portuguese (`docs/pt-BR`) and in
English (`docs/en`): if you change one, update the other too, or ask for help in the pull
request. Issues and pull requests can be in English or Portuguese.

## Before opening a pull request

All of this must pass (it's what CI runs):

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets --features tomni-ui-test/cjk,tomni-ui-test/gpu -- -D warnings
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps
cargo test --workspace
cargo test -p tomni-ui-test --features cjk
cargo deny check licenses
cargo run -p tomni-ui-cli -- lint -p tomni-ui --example login   # and the other examples
cargo run -p tomni-ui-cli -- lint -p tomni-ui --example terminal -F terminal
cargo run -p tomni-ui-cli -- lint -p tomni-ui --example webview -F webview-widget
cargo run -p tomni-ui-cli -- lint -p tomni-ui --example ia -F ai,markdown
cargo run -p tomni-ui-cli -- lint -p m6-login                    # and the other apps in aceite/
```

With a GPU (or lavapipe, `WGPU_ADAPTER_NAME=llvmpipe`):

```sh
TOMNI_UI_BACKEND=gpu cargo test -p tomni-ui-test --features gpu,cjk
```

The Rust version is pinned in `rust-toolchain.toml` (the snapshots depend on it); the declared
minimum is 1.89 (`cargo +1.89 check --workspace --all-targets`).

`WebView` with the system engine is left out of `cargo test --workspace`. With
`libwebkit2gtk-4.1-dev` installed and a display (or `xvfb-run`):

```sh
cargo clippy -p tomni-ui --features webview --all-targets -- -D warnings
cargo test -p tomni-ui --features webview --test webview_nativo
```

### Optional tests against real servers

These are not part of CI and need Docker. Run the one for the area you changed:

| Area | Command | What it starts |
| --- | --- | --- |
| FTP and FTPS (`crates/files/src/ftp/`) | `testes/ftp/rodar.sh` | vsftpd, proftpd and pure-ftpd (ports 2121 to 2991) |
| S3 (`crates/files/src/s3/`) | `testes/s3/rodar.sh` | MinIO (port 19000) |
| rclone (`crates/files/src/rclone/`) | `TOMNI_UI_RCLONE=rclone cargo test -p tomni-ui-files --features rclone --test rclone_real -- --ignored` | the system `rclone` (1.60 or newer) and `python3` |
| Nebula (`crates/nebula/`) | `testes/nebula/rodar.sh` | the official `nebula` (UDP 14242 to 14244) |
| ZeroTier (`crates/zerotier/`) | `testes/zerotier/rodar.sh` | the official `zerotier-one`, with a test planet (nothing goes out to the internet) |
| OpenVPN (`crates/openvpn/`) | `testes/openvpn/rodar.sh` | `openvpn` 2.6 (ports 19194 to 19199) |
| WireGuard (`crates/wireguard/`) | `testes/wireguard/rodar.sh` | `wg` and `wg-quick` (UDP 51820) |
| Tailscale (`crates/tailscale/`) | `testes/tailscale/rodar.sh` | the official `headscale` and `tailscale` (`MANTER=1` keeps them running) |
| Shell folder over SSH (`crates/terminal/src/ssh/pasta.rs`) | `testes/pasta/rodar.sh` | new and old OpenSSH and Dropbear (ports 22201 to 22203) |

CI covers these areas without a network: the in-memory S3 server, the fake `rcd`, `SimNet`
(in-memory UDP and NAT) with each VPN's test servers, and OpenVPN's `ServerSession`. Each
network's fixtures come from the `gerar-fixtures.sh` in the corresponding folder.

## Layout

| Folder | What |
| --- | --- |
| `crates/reactive` | signals (`reactive_graph`) |
| `crates/core` | widgets, tree, focus, events, `IntoProp` and the system tray model |
| `crates/layout` | flex with `taffy` |
| `crates/text` | `parley` and the embedded fonts |
| `crates/render` | `Canvas`, CPU (`vello_cpu`) and GPU (`vello`) backends |
| `crates/theme` | `Drawer` and `FluentDrawer` (tokens in `tokens.rs`) |
| `crates/platform` | `winit` window, input, IME, file picker, system tray and Android |
| `crates/win32` | the Win32 calls (Windows only, the only crate with `unsafe`) |
| `crates/terminal` | the `Terminal`, the emulator (`vte`), the `Session`, `TerminalPanes` and the `Telnet`, `Process`, `Ssh` and `Serial` connections |
| `crates/files` | `Fs` (local, SFTP, FTP, S3 and rclone), `Remote` and the `Transfers` queue |
| `crates/net` | `Proxy` (SOCKS5 and HTTP CONNECT), `connect` and the HTTP agent |
| `crates/netstack` | the software TCP/IP stack (`smoltcp`), the egress filter and the VPNs' SOCKS5 bridge |
| `crates/openvpn`, `crates/wireguard`, `crates/tailscale`, `crates/nebula`, `crates/zerotier` | the built-in networks; the core of each one has no I/O and uses an explicit clock |
| `crates/ai` | the AI client (OpenAI and Anthropic) and the `FakeAi` fake server |
| `crates/webview` | `WebView`, `WebViewController`, the system engine (`wry`) and the fake one |
| `crates/markdown` | `MarkdownView`, syntax highlighting and Markdown editing |
| `crates/tomni-ui` | the facade: `Ui`, `App`, `inspect`, `lint` and `tools` (the protocol) |
| `crates/tomni-ui/examples` | one example per widget |
| `crates/test` | `TestApp`, the snapshots and the integration tests |
| `crates/cli` | the protocol client and the `tomni-ui` binary |
| `crates/cli/templates` | the `AGENTS.md`, `CLAUDE.md`, `SKILL.md` and `mcp.json` used by `tomni-ui init` |
| `crates/mcp` | the `tomni-ui-mcp` binary (JSON-RPC over stdio) |
| `aceite/` | apps written by agents from the guide alone; workspace members, tested and linted |
| `testes/` | the Docker environments for the optional tests |

## Rules the compiler doesn't check

- **Look and behavior**: WinUI 3 (`microsoft/microsoft-ui-xaml`) is the reference for the
  widgets' colors, metrics, properties and behavior. When WinUI doesn't define something, or it
  can't be followed (a glyph Selawik doesn't have, a popup that would go outside the window),
  use what `fluent_ui` (the Flutter Fluent package) does and document the adaptation in the
  widget's docs.
- **Snapshots**: only approve (`TOMNI_UI_UPDATE_SNAPSHOTS=1`) after opening the `.snap.new.png`
  and the `.snap.diff.png` and agreeing with the change. The references come from the
  deterministic CPU backend.
- **Examples**: every file in `crates/tomni-ui/examples` has a `#[test]` in
  `crates/test/tests/exemplos.rs` (a test fails if one is missing) and passes the lint in both
  themes. Screens are `pub fn` so the test can see them.
- **Tools protocol**: `tomni_ui::tools` (the app) and `tomni_ui_cli` (the client) both define
  `PROTOCOL`, `ENV`, `PREFIX` and the lint rule names; a test checks they match. Changed the
  format of a request or a response? Bump `PROTOCOL` in both.
- **App guide**: changed the public API (widget, builder, `TestApp`, CLI, MCP)? Update
  `crates/cli/templates/AGENTS.md`, `llms.txt` and, if relevant, `SKILL.md`. The guide must be
  enough for a fresh agent session to build a screen that passes the tests and the lint.
- **Compiler messages**: `Widget`, `View` and `IntoProp` have
  `#[diagnostic::on_unimplemented]`; keep the notes showing the right way to write the code.
- **Dependencies**: only licenses on the `deny.toml` list. The `about.hbs` and `about.toml`
  files at the root and in `crates/cli/templates` must be identical (there's a test).
- **Reactivity**: runtime code that reads signals outside an effect (events, focus) uses
  `untrack`.
- **Apps in `aceite/`**: the code stays as the agent delivered it. A library change that breaks
  them would also break apps written from the guide: rethink the change or the guide.
- **Tests without network or devices**: a local `russh` server, a pseudoterminal pair, `sh`
  and the fake servers. The recordings in `crates/test/tests/gravacoes/` are real program
  output.
- **`unsafe`**: the workspace lint denies it. The exceptions are `android_main` (with a
  justified `#[allow]`) and the `tomni-ui-win32` crate, with one `unsafe` per block and a
  `// SAFETY:` on each. `std::env::set_var` isn't allowed either: pass the value through the
  builder or the child process.
- **Third-party code**: OpenVPN is GPL-2.0, so `tomni-ui-openvpn` is written from the protocol
  description, without copying or translating the project's code. When porting code under a
  permissive license (as with WireGuard, Tailscale and Nebula), record the origin and the
  license in the crate's `NOTICE`.

## Tools in the repository itself

```sh
cargo run -p tomni-ui-cli -- screenshot -p tomni-ui --example login --theme dark
cargo run -p tomni-ui-cli -- tree -p tomni-ui --example layout --view linha
cargo run -p tomni-ui-mcp -- -p tomni-ui --example login   # MCP server
```

The outputs go to `target/tomni-ui/`.

## Licensing of contributions

Unless you explicitly state otherwise, any contribution intentionally submitted for inclusion
in the project, as defined in the Apache-2.0 license, shall be dual licensed as MIT or
Apache-2.0, at the user's option, without any additional terms or conditions.
