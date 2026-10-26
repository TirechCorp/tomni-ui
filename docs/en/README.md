# Tomni UI

[Português](../pt-BR/README.md) · **English**

A Rust UI library with the Windows 11 Fluent look, drawn by the library itself and identical on
Windows, Linux, macOS and Android. State is reactive (signals), and the real screen can be
viewed, inspected, tested and checked without opening a window: the same tools serve people and
AI agents alike.

<p align="center">
  <img src="../capturas/formulario-claro.png" alt="A form in the light theme" width="300">
  <img src="../capturas/formulario-escuro.png" alt="A form in the dark theme" width="300">
</p>

## What's included

- **WinUI 3 widgets**, with its colors, metrics and behavior: buttons, text fields, lists,
  `ComboBox`, `NavigationView`, `TabView`, `TreeView`, `DataGrid`, `PropertyGrid`, menus,
  dialogs, `InfoBar`, `TitleBar` with the Mica backdrop, Fluent icons and more (one example of
  each in [`crates/tomni-ui/examples`](../../crates/tomni-ui/examples)).
- **Its own rendering**, with `vello` on the GPU and `vello_cpu` on the CPU, text through
  `parley` with Selawik embedded, and flex layout through `taffy`.
- **Fine-grained reactivity**: `signal`, `memo` and `effect`, and only what changed is redrawn.
- **Windowless tests**: `TestApp` clicks, types and checks by widget id, and snapshots produce
  the same pixels on x86_64 and ARM.
- **Tools for agents**: the `tomni-ui` CLI and the `tomni-ui-mcp` MCP server take
  screenshots, show the widget tree, interact with the screen and run the accessibility and
  layout lint.
- **Terminal** with VT100, VT220 and xterm emulation, and local process, Telnet, SSH (jump host,
  proxy, agent, X11 and tunnels) and serial connections.
- **Local and remote files**: SFTP, FTP and FTPS, S3 and compatibles, and other clouds through
  `rclone`, with a transfer queue.
- **Built-in networks**: OpenVPN, WireGuard, Tailscale (and Headscale), Nebula and ZeroTier in
  Rust. They run without a virtual network adapter and without admin rights, and only the app's
  traffic goes through them, via a local SOCKS5 bridge with random credentials.
- **AI**: a language model client for the OpenAI-compatible and Anthropic Messages dialects,
  with cancelable streaming.
- **WebView** using the system control (WebView2, WKWebView or WebKitGTK) and **Markdown** as
  widgets.

[Tirech Omni](https://github.com/TirechCorp/tirech-omni), an SSH connection manager, is the
real-world app built with the library:

<p align="center">
  <img src="../capturas/tirech-omni-escuro.png" alt="Tirech Omni in the dark theme" width="720">
</p>

## Getting started

The crates are not on crates.io yet. Use them from Git:

```toml
[dependencies]
tomni-ui = { git = "https://github.com/TirechCorp/tomni-ui" }

[dev-dependencies]
tomni-ui-test = { git = "https://github.com/TirechCorp/tomni-ui" }
```

A screen is a function that builds widgets:

```rust
use tomni_ui::prelude::*;

fn login() -> impl View {
    let email = signal(String::new());
    let senha = signal(String::new());
    Form::new("Login").child(
        Column::new()
            .gap(12)
            .padding(24)
            .child(Edit::bind(email).id("email").placeholder("E-mail"))
            .child(Edit::bind(senha).id("senha").placeholder("Senha").password())
            .child(
                Button::new("Entrar")
                    .id("entrar")
                    .accent()
                    .disabled(move || !email.get().contains('@') || senha.get().len() < 6),
            ),
    )
}

fn main() -> Result<(), tomni_ui::platform::Error> {
    tomni_ui::App::new(login).size(390, 844).run()
}
```

And it is tested without opening a window:

```rust
use tomni_ui_test::TestApp;

let mut app = TestApp::new(login).size(390, 844);
app.type_into("email", "ana@x.com").type_into("senha", "123456");
assert!(app.enabled("entrar"));
// Só o que mudou foi redesenhado (o olho da senha aparece depois da digitação).
assert_eq!(app.repaints_since_last(), ["email", "senha", "senha-revelar", "entrar"]);
```

To run the examples:

```sh
cargo run -p tomni-ui --example login
cargo run -p tomni-ui --example terminal --features terminal
cargo run -p tomni-ui --example webview --features webview
```

The theme follows the system's light or dark mode, unless `.theme(...)` pins it. When the GPU is
a software driver (WARP in a VM, llvmpipe), the window draws on the CPU. On Android,
`android_main` passes the `AndroidApp` with `.android(app)` (see the `login` example).

The complete API guide, which `tomni-ui init` also copies into your project, is in
[`crates/cli/templates/AGENTS.md`](../../crates/cli/templates/AGENTS.md). The short map is in
[`llms.txt`](../../llms.txt).

## Facade features

The `tomni-ui` crate enables everything else through features, so you don't pay for what you
don't use:

| Feature | What it enables |
| --- | --- |
| `tools` (default) | tools mode (`TOMNI_UI_TOOLS=1`), used by the CLI and the MCP server |
| `terminal` | the `Terminal` widget, the emulator and Telnet, with Cascadia Mono embedded |
| `pty`, `ssh`, `serial` | local process, SSH (`russh`) and serial port connections |
| `files`, `sftp`, `ftp`, `s3`, `rclone` | local and remote files, with the transfer queue |
| `ai` | the language model client |
| `markdown` | `MarkdownView` and syntax highlighting in `TextEditor` |
| `webview` | `WebView` with the system engine (on Linux, requires `libwebkit2gtk-4.1-dev`) |
| `webview-widget` | only the widget, with the fake engine used in tests |
| `cjk` | Noto Sans JP embedded, for Chinese, Japanese and Korean |
| `mono` | Cascadia Mono embedded for `TextEditor` |
| `mcp` | the framing for an MCP server (line-delimited JSON-RPC) |

The built-in networks are separate crates (`tomni-ui-openvpn`, `tomni-ui-wireguard`,
`tomni-ui-tailscale`, `tomni-ui-nebula` and `tomni-ui-zerotier`). Each one has a builder and a
`proxy()`, the SOCKS5 proxy you pass to `Ssh`, `Telnet` or `Sftp`.

## Crates

| Crate | Role |
| --- | --- |
| `tomni-ui` | the facade the app uses: `App`, `Ui`, tree, lint and tools mode |
| `tomni-ui-core` | widgets, tree with stable ids, layers, focus and events |
| `tomni-ui-reactive` | `signal`, `memo` and `effect` (on top of `reactive_graph`) |
| `tomni-ui-layout` | flex layout through `taffy` |
| `tomni-ui-text` | text through `parley`, with the embedded fonts |
| `tomni-ui-render` | the `Canvas`, with the CPU and GPU backends |
| `tomni-ui-theme` | the `FluentDrawer`, the Fluent icons and the WinUI 3 color tokens |
| `tomni-ui-platform` | `winit` window, input, IME, file picker and system tray |
| `tomni-ui-terminal` | the `Terminal` and the `Process`, `Telnet`, `Ssh` and `Serial` connections |
| `tomni-ui-files` | `Fs` (local, SFTP, FTP, S3, rclone) and the transfer queue |
| `tomni-ui-net` | SOCKS5 and HTTP CONNECT proxy, TCP connection and the HTTP client |
| `tomni-ui-netstack` | the software TCP/IP stack (`smoltcp`) and the VPNs' SOCKS5 bridge |
| `tomni-ui-openvpn` | the OpenVPN client (TLS 1.2 and 1.3, `tls-auth`, `tls-crypt`, AES-GCM and ChaCha20) |
| `tomni-ui-wireguard` | WireGuard (reads the `wg-quick` `.conf`) |
| `tomni-ui-tailscale` | the Tailscale and Headscale node, with DERP and MagicDNS |
| `tomni-ui-nebula` | the Nebula network (v1 and v2 certificates, lighthouse and punch) |
| `tomni-ui-zerotier` | the ZeroTier node (VL1 and VL2) |
| `tomni-ui-ai` | the language model client |
| `tomni-ui-markdown` | Markdown as widgets and Markdown editing |
| `tomni-ui-webview` | the `WebView` |
| `tomni-ui-win32` | Win32 calls with no safe API (the only crate with `unsafe`) |
| `tomni-ui-test` | `TestApp` and snapshots |
| `tomni-ui-cli` | the `tomni-ui` CLI |
| `tomni-ui-mcp` | the `tomni-ui-mcp` MCP server |

## Tools for agents

The CLI and the MCP server build the app with `cargo` and run it in tools mode: no window, with
the real screen, signals and handlers of the app, drawn by the deterministic CPU backend.

```sh
cargo install --path crates/cli   # tomni-ui
cargo install --path crates/mcp   # tomni-ui-mcp

tomni-ui screenshot -p tomni-ui --example login --size 390x844 --theme dark
tomni-ui tree -p tomni-ui --example layout --view linha
tomni-ui lint -p tomni-ui --example form
tomni-ui build --target linux -p tomni-ui --example login
```

| CLI | MCP | What it does |
| --- | --- | --- |
| `screenshot` | `ui_screenshot` | PNG of the screen, with size and theme |
| `tree` | `ui_tree` | the widgets, with id, type, rectangle and state |
| — | `ui_interact` | actions by id (click, type, keys, scroll, select), returning the screenshot and the tree |
| `lint` | `ui_lint` | AA contrast, 44 px touch target, clipped text, out of the window, character without a glyph and widget without an id |
| `run` | — | opens the window |
| `build --target windows\|linux\|macos\|android` | — | the release binary (the APK on Android) and `THIRD-PARTY-NOTICES.txt` |
| `init` | — | writes `AGENTS.md`, `CLAUDE.md`, the Claude Code skill and `.mcp.json` into the app |

In MCP, the session persists between calls (typed text and clicks are kept) until the app is
rebuilt, the screen changes or `reset: true` is sent. Build errors come back as tool errors.
`TestApp::assert_lint()` runs the same lint checks inside tests.

## Snapshots

`assert_snapshot!("nome", pixmap)` compares the image with `snapshots/nome.png` in the folder of
the crate under test. With no reference, or with a difference above 0.5% of the pixels, the test
fails and writes `nome.snap.new.png` and `nome.snap.diff.png` (differences in red). After
checking the new image, approve it with:

```sh
TOMNI_UI_UPDATE_SNAPSHOTS=1 cargo test
```

Snapshots use `CpuCanvas::deterministic`: no SIMD, a single thread and an `f32` pipeline, so
that x86_64 and ARM produce the same pixels. To check the GPU against the CPU references:

```sh
TOMNI_UI_BACKEND=gpu cargo test -p tomni-ui-test --features gpu,cjk
```

## Building and testing

The Rust version is pinned in `rust-toolchain.toml` (requires `rustup`); the minimum supported
version is 1.89.

```sh
cargo build
cargo test --workspace
cargo test -p tomni-ui-test --features cjk
```

The tests against real servers (FTP, S3, rclone, OpenVPN, WireGuard, Tailscale, Nebula and
ZeroTier) are optional and start the servers in Docker: see
[`CONTRIBUTING.md`](CONTRIBUTING.md).

## Contributing

Read [`CONTRIBUTING.md`](CONTRIBUTING.md) (the CI checks, the layout and the rules) and the
[code of conduct](CODE_OF_CONDUCT.md). Security issues go through
[`SECURITY.md`](SECURITY.md), not through a public issue.

## License

MIT or Apache-2.0, at your option ([`LICENSE-MIT`](../../LICENSE-MIT),
[`LICENSE-APACHE`](../../LICENSE-APACHE)).

- The embedded fonts (Selawik in `tomni-ui-text`, Noto Sans JP from the `cjk` feature and
  Cascadia Mono from the `mono` feature) are OFL-1.1.
- The color tokens come from `microsoft/microsoft-ui-xaml` (MIT).
- WireGuard, Tailscale and Nebula have parts ported from the original code; the references and
  licenses are in each crate's `NOTICE`. OpenVPN was written from the public description of the
  protocol, and ZeroTier was checked against real packets, without copying code.

Dependencies are restricted to permissive licenses, checked by `cargo deny check licenses`.
MIT, BSD, ISC, Zlib, Unicode-3.0 and OFL require the copyright notice to ship with the binary,
and that also applies to apps built with the library. `tomni-ui build` generates
`THIRD-PARTY-NOTICES.txt` next to the binary, or you can generate it by hand:

```sh
cargo install --locked cargo-deny
cargo install --locked cargo-about --features cli
cargo about generate about.hbs -o target/THIRD-PARTY-NOTICES.txt
```
