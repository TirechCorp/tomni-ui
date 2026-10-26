# Security

## Reporting a vulnerability

Don't open a public issue. Use GitHub's private reporting: in the repository's **Security** tab,
**Report a vulnerability** (or
[this link](https://github.com/TirechCorp/tomni-ui/security/advisories/new)).

Send whatever you can: the version or commit, the affected crate, how to reproduce it and the
impact you see. We reply within 7 days and agree with you on a disclosure date after the fix.
Reporters get credit in the advisory, if they want it.

## Supported versions

The project is still in the 0.x series: fixes ship only in the latest version on `main`.

## Areas of particular interest

- **Built-in networks** (`tomni-ui-openvpn`, `-wireguard`, `-tailscale`, `-nebula`, `-zerotier`
  and `-netstack`): flaws in the cryptography, in server or peer authentication, in the handling
  of malformed packets and in the egress filter (only the reply to a connection opened by the
  app may come in).
- **The VPNs' SOCKS5 bridge**: it listens only on `127.0.0.1`, on a random port, and requires a
  random 128-bit username and password, compared in constant time. Any way for another process
  to use the bridge without the credentials is a vulnerability.
- **SSH, FTP(S), S3 and HTTP**: server key verification, TLS certificate verification and the
  handling of credentials in memory.
- **Tools mode** (`TOMNI_UI_TOOLS=1`): it talks only through the standard input and output of
  the process that started it, without opening a port or socket. A distributed app can turn it
  off with `default-features = false`.
- **The terminal**: escape sequences that break out of the emulator (writing files, running
  commands, leaking the screen contents).

The tests against real servers use test keys and certificates, generated for that purpose and
committed in `crates/*/tests/fixtures/`; they don't protect anything.
