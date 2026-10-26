# Changelog

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versions
follow [SemVer](https://semver.org/). In the 0.x series, a minor version may break the API.

## [Unreleased]

### Added

- `request_focus_in(window, id)`: a focus request from an effect, which runs in any window's
  frame, with the window that answers first.

### Fixed

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
