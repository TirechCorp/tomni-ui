# Changelog

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versions
follow [SemVer](https://semver.org/). In the 0.x series, a minor version may break the API.

## [Unreleased]

### Added

- `Reuse::new(key, || view)`: inside a `Show` or `For` that remounts, the part whose key matches one
  that left in the same swap comes back whole (widgets, state, focus, signals), without creating the
  view again.
- `TerminalPanes::on_close_pane(|pane| ...)` lets the app decide what "close the pane" does for a pane
  with tabs, and `focus_active_pane(layout)` gives focus back to the active pane's terminal.
- `effect_named("name", f)`, `profile_effects`, `take_effect_totals` and the `efeitos mais caros` line
  in the stall log.

- `TabView::tab_visible(|key| bool)`: a tab whose (reactive) closure returns `false` leaves the
  strip, Ctrl+Tab, the arrow keys and the equal-width layout, and its panel stays mounted, only
  hidden. If the selected tab disappears, the selection moves to its neighbor; with
  `.on_reorder`, the indices count only the visible tabs.
- `tomni_ui_macos::temperatures()` and `cpu_brand()`: the Apple Silicon temperature sensors
  (through `IOHIDEventSystemClient`, without root), with the name the system gives and the
  degrees, and the processor name.
- `tomni_ui_win32::sensors()` and `cpu_name()`, without administrator rights: the disk
  temperatures (with the warning and critical limits), the ACPI thermal zones and, with
  LibreHardwareMonitor running, its sensors and fans through WMI; and the processor name.
- `tomni_ui::notification`: desktop notifications.
  `Notification::new(title).body(text).tag(group).on_click(f).show()`, with `close`,
  `available` and `set_app_name`; one with the same `tag` replaces the previous one and the
  click runs on the UI thread. On Linux through `org.freedesktop.Notifications` (D-Bus), on
  Windows through the system notifications (`tomni_ui_win32::Toasts`) and on macOS through
  `UNUserNotificationCenter` (`tomni_ui_macos::Notifications`, only inside a `.app`).
  `TestApp::notifications()` and `notification_click(title)` for tests.
- `app_focused()`: `false` while the user is in another program. The
  `Event::WindowFocus(bool)` event reaches the focused widget when the window gains or loses
  the system focus, and the `Terminal` also tells the program that asked for `ESC[?1004h` in
  that case.
- `Session::notices()` (reactive, the program's notices with text) and
  `Session::notify(Notice)` (a notice on the tab as if it came from the program).
- `tomni-ui-nebula`: Nebula metrics (`stats` in the config), with the same names as official
  Nebula. `stats.type: graphite` sends the lines to Graphite over TCP every `stats.interval`;
  `stats.type: prometheus` opens an HTTP endpoint at `stats.listen` and `stats.path` for
  Prometheus to scrape. `stats.message_metrics` and `stats.lighthouse_metrics` turn on the
  per-message-type counters. `Config::stats` (`Stats`, `StatsExport`, `GraphiteProtocol`) holds
  what was read and `Nebula::stats_addr()` tells where the HTTP endpoint opened. A `stats` with
  an error stops the node from starting, as in Nebula; before, it was ignored with a warning.
- `tomni-ui-nebula`: Nebula relays, compatible with official Nebula (v1 and v2 certificates).
  With `relay.relays` the node accepts being reached through the listed relays, and with
  `relay.use_relays` it reaches peers through the relays they announce; this is the path when
  both sides are behind CGNAT or symmetric NAT. `TunnelInfo::relay` tells which relay the tunnel goes through.
- `Session::held::<T, _>(|value| ..)`: reads the first value of type `T` hung with `hold`
  (`None` if there's none or the session was dropped).
- `request_focus_in(window, id)`: a focus request from an effect, which runs in any window's
  frame, with the window that answers first.
- `tomni_ui_net::HttpProxyServer`: a local HTTP proxy on `127.0.0.1` that carries each request
  (`CONNECT` or plain HTTP in absolute form) through a dial function from the app, with the
  unresolved name. Useful to open a browser that goes out through another network.
  The same server speaks SOCKS5 on the same port (`socks_url()` gives the `socks5h://` URL), and
  `bind(address, …)` picks the port and the address.
- `tomni_ui_net::PortForward::bind(address, host, port, dial)`: a local port that always carries
  each connection to the same destination, through the app's dial function (an `-L` tunnel
  without SSH).
- Terminal: `Session::hold(value)` keeps a value with the session (a proxy, a tunnel) until
  `dispose`.
- `AreaChart`, `Sparkline` and `LevelMeter`: the Task Manager's area chart (the whole series,
  `.max`, `.capacity` of 60 points), its small version and the meter of squares from light
  yellow to dark red, which becomes a button with `.on_click`. `TestApp::value` reads the meter's
  level and the chart's newest point. Example `desempenho`.
- `DataGrid`: `DataGridRow::heat(0..=1)` colors the cell's background by weight, and
  `CellValue::Sparkline` draws a small series in place of the text.
- Terminal: `Session::sent()`, `Session::busy()` (the time spent reading, parsing and painting)
  and `Session::threads()` (the connection's threads, to measure each tab's CPU; on Linux).
- `tomni_ui_win32::process_usage(pid)`: a process's CPU time and memory on Windows.
- `tomni-ui-macos`, the crate for the macOS calls with no safe API: `process_usage(pid)` (a
  process's CPU time and memory) and `child_pids(pid)` (its direct children).
- `memory_status()` in `tomni-ui-win32` and `tomni-ui-macos`: the machine's physical memory,
  with the total and the available amount (`MemoryStatus`), as Task Manager and Activity
  Monitor show them.
- `cpu_times()` in `tomni-ui-win32` and `tomni-ui-macos`: each logical core's user, kernel and
  idle time (`CpuTimes`), for the per-core graph and the kernel time.
- `network_interfaces()` in `tomni-ui-win32` and `tomni-ui-macos`: the bytes each network
  interface received and sent, without the loopback one, with its state and whether it is
  physical (`NetworkInterface`).
- `disk_counters()` in `tomni-ui-win32` and `tomni-ui-macos`: each disk's operations and bytes
  read and written (`DiskCounters`), and on Windows `ProcessUsage` gets `read_operations` and
  `write_operations`.
- `AreaChart` and `Sparkline` get `.secondary(series)` and `.secondary_color(..)`: a second
  series over the first, on the same scale, red by default, like Task Manager's kernel time.
- `HeatMap`: the heat map, one tile per value, filled with more color the higher it is, in
  columns and rows that keep each tile as close to square as possible (`heat_map_grid`,
  `heat_map_cell`, `heat_map_index_at`), with the tooltip of the tile under the pointer
  (`.cell_tooltip`). It is what Task Manager shows instead of the per-core charts when there
  are many. `TestApp::values(id)` gives a chart's series or the map's values.
- The `Symbol::DataArea` icon.

- `App::stall_log(path)` (and `WindowOptions::stall_log`): the stall log. A watchdog thread
  writes when the UI is stuck for more than 2 s in a stage (the event, the frame, layout, the
  terminals, the window texture...), with the thread state, load, memory and the busiest threads
  (on Linux), and the time per stage for each heavy minute. `tomni_ui::stages::stage("name")`
  marks an app stage.
- Terminal: `INBOX_LIMIT`, `Inbox::push_wait`, `Inbox::has_room` and `Inbox::detach`.
- `close(&self)` on the VPN engines (`Nebula`, `WireGuard`, `Tailscale`, `ZeroTier` and
  `OpenVpn`): asks it to stop without waiting and without consuming the engine, for when it sits in
  an `Arc` shared with the connections.

### Changed

- `TerminalPanes` splits a pane without recreating the terminals, and the tabs' context menu is only
  created when first opened: until then its items aren't in the tree, and a `.shortcut()` on them has
  no effect. The grid no longer remounts the focus of whoever already had it (snapshots
  `m23_grade_3x3_*`).
- Less work per frame: wake-ups merge into one frame per window, `Inbox` only wakes the UI when the
  queue goes from empty to having bytes, hidden tabs are read every 100 ms and idle animators report
  how long is left (`Widget::ticking_in`).

- An empty `TitleBar::subtitle` disappears and leaves no space, like WinUI's `Collapsed`: a
  closure can show the subtitle only when it makes sense.
- `TestApp` passes the `Form` title to the window on every frame, like the runner: a title that
  changes shows up in `WindowHandle::title`, the main window's included.
- A `MenuFlyout` taller than the window takes the window's height and scrolls, as in WinUI,
  instead of cutting off the bottom items.

### Fixed

- `dialogs::save_file()` with a filter puts the first filter's first extension on a name without one:
  the Linux portal and zenity saved the name as typed, and the file vanished from the filter on open.
- An Ed25519 `.ppk` with a leading zero in the seed (one key in 512) didn't open.

- Slowness with many tabs and windows: `Tree::deadline` first asks whether an animator is
  running, and only then whether it is shown, as `advance` already did. Before, every loop turn
  walked up the ancestors of every idle menu item (the context menus of each terminal), running
  each one's `.visible()`. `sync_layers` looks for open layers in a list of the tree's layers,
  instead of the whole tree, three times per frame.
- SSH: the password is also sent through `keyboard-interactive` when the server rejects the
  `password` method and offers that one (the FreeBSD and pfSense sshd, OpenSSH with
  `PasswordAuthentication no`). A second question after the password (a two-factor code)
  becomes an error with the question's text.
- On macOS, "servidor inalcançável" (host unreachable) says where to grant the app Local
  Network access.
- The macOS release build printed an unused `mut` warning in `tomni-ui-platform`.
- Terminal: Backspace at column 0 of a line that continues the one above (autowrap) goes back to
  the last column of the one above, as PuTTY does. A screen-wide progress bar (`pv` in a pipe
  with `7z`) no longer moves down a line on every refresh.
- A terminal with endless output (`yes`, a huge log) made memory grow without bound and the
  window stop responding. Each connection's queue now holds 1 MiB: the program waits, as in a
  system terminal, and Ctrl+C still gets through; over SSH, the other channels of the same
  connection (SFTP, `exec`, tunnels) keep moving. A `wake` from another thread no longer floods
  the window's event queue.

- Tray on Cinnamon and MATE: the Ayatana host ignores `IconPixmap` and showed an empty slot. The
  icon now also goes as a PNG in `$XDG_RUNTIME_DIR/tomni-ui/`, by path in `IconName`, as Qt does;
  the file is removed when the icon goes away.
- The transfer queue honors "Cancel" between partial writes, without waiting for the whole
  buffer to go out on a slow server.

- With the same screen mounted in two windows, `request_focus`, `request_focus_inside`,
  `start_bring_into_view` and `TextEditorController::focus` gave focus to whichever window ran
  its frame first. Now the window of the event that asked answers first, and the others only
  when it doesn't have the widget.

- SSH: the connection's own keys (`key_file`, `key`) are offered before the agent's other keys,
  as OpenSSH does. With many keys in the agent, the server (`MaxAuthTries`) dropped the
  connection before reaching the right one, and the error said the agent had refused the key.
- SSH: a key file that fails to open no longer stops the agent from authenticating.

## [0.1.0] - 2026-10-26

First public release.

- WinUI 3 widgets, light and dark themes, Fluent icons, `TitleBar` and the Mica backdrop.
- Signal-based reactivity, repainting only what changed.
- CPU (deterministic, for snapshots) and GPU backends, on Windows, Linux, macOS and Android.
- `TestApp`, snapshots, the `tomni-ui` CLI and the `tomni-ui-mcp` MCP server.
- Terminal with local process, Telnet, SSH and serial.
- Local files, SFTP, FTP and FTPS, S3 and rclone.
- Built-in networks: OpenVPN, WireGuard, Tailscale, Nebula and ZeroTier.
- AI client, `WebView` and Markdown.

[0.1.0]: https://github.com/TirechCorp/tomni-ui/releases/tag/v0.1.0
