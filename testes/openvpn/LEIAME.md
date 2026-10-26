# Fixtures do OpenVPN

**Chaves só de teste; nunca use num servidor de verdade.** Tudo que o `gerar-fixtures.sh`
grava em `crates/openvpn/tests/fixtures/` (autoridades, certificados, chaves privadas, chaves
estáticas e a senha `senha-de-teste` das chaves cifradas) é público, está no repositório e
serve só aos testes do `tomni-ui-openvpn`.

## O que o script gera

- Autoridades: `ca` (RSA 2048, SHA-256), `ca-ec` (ECDSA P-256), `ca-sha1` (assina com SHA-1,
  para a recusa) e `outra-ca`. As chaves privadas das autoridades são apagadas no fim.
- Servidores: `servidor` (com `serverAuth`), `servidor-sem-eku`, `servidor-ec` e
  `servidor-outra-ca`.
- Clientes: `cliente-rsa` (PKCS#1), `cliente-ec` (SEC1), `cliente-pkcs8`, `cliente-cifrado`
  (PKCS#8 com senha), `cliente-cifrado-legado` (PEM com `Proc-Type`), `cliente-vencido`
  (venceu em fevereiro de 2020), `cliente-outra-ca`, `cliente-sha1` e `chave-trocada.key`.
- Chaves do OpenVPN: `ta.key` e `tc.key` (estáticas), `tc2-servidor.key` e `tc2-cliente.key`
  (`tls-crypt-v2`).
- `.ovpn` de exemplo e o `referencias.txt` (impressões, sujeitos e datas, no formato
  `arquivo.campo=valor`, para os testes conferirem o que o crate lê).

## Como rodar

```sh
testes/openvpn/gerar-fixtures.sh
```

Precisa do `openssl` e do `openvpn` (2.6 ou mais novo) no `PATH`; sem eles, o script roda
sozinho dentro de um contêiner Debian (precisa do Docker). As fixtures são versionadas: os
testes comuns (`cargo test -p tomni-ui-openvpn`) não precisam do script nem do Docker.

## Servidores de verdade (opcional, fora do CI)

`testes/openvpn/rodar.sh` sobe, num contêiner Alpine (Docker e rede para o `apk add openvpn socat
dnsmasq iproute2`), seis servidores OpenVPN com as fixtures (`servidores.sh`, só no `127.0.0.1`):

| Porta | Servidor |
|---|---|
| 19194 | UDP com `tls-auth`, `explicit-exit-notify`, DNS 10.8.1.1 e domínio `corp.test` empurrados |
| 19195 | UDP com `tls-crypt` |
| 19196 | TCP |
| 19197 | UDP com `tls-crypt` e TLS 1.3 (chaves pelo exportador) |
| 19198 | UDP só com ChaCha20-Poly1305 |
| 19199 | UDP com `compress stub-v2` e `keepalive 2 8` |

Dentro da VPN (`topology subnet`, `tun0` a `tun5`) ficam um eco do `socat` na porta 7000, um
banner na 7001 e um `dnsmasq` em 10.8.1.1 que responde `alvo.corp.test`. O script espera o
"servicos prontos" e passa o nome do contêiner em `TOMNI_UI_OPENVPN_CONTAINER` (um teste manda
`USR1` ao servidor para forçar o `RESTART`). Roda os testes `#[ignore]` de
`crates/openvpn/tests/openvpn_real.rs`. Rode depois de mexer no canal de controle (`control.rs`,
`channel.rs`, `keymethod.rs`, `wrap.rs`), no canal de dados (`data.rs`, `session.rs`) ou no motor
(`engine.rs`, `vpn.rs`, `transport.rs`).
