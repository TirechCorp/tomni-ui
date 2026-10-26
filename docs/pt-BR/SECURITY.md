# Segurança

## Como relatar uma falha

Não abra issue pública. Use o relato privado do GitHub: na aba **Security** do repositório,
**Report a vulnerability** (ou
[este link](https://github.com/TirechCorp/tomni-ui/security/advisories/new)).

Mande o que for possível: a versão ou o commit, o crate afetado, como reproduzir e o impacto
que você vê. Respondemos em até 7 dias e combinamos com você a data da divulgação depois da
correção. Quem relatar recebe o crédito no aviso, se quiser.

## Versões cobertas

O projeto ainda está na série 0.x: as correções saem só na versão mais recente da `main`.

## O que interessa em especial

- **Redes embutidas** (`tomni-ui-openvpn`, `-wireguard`, `-tailscale`, `-nebula`, `-zerotier`
  e `-netstack`): falhas na criptografia, na autenticação do servidor ou dos pares, no
  tratamento de pacotes malformados e no filtro de saída (só a resposta de uma conexão aberta
  pelo app pode entrar).
- **A ponte SOCKS5 das VPNs**: escuta só em `127.0.0.1`, em porta aleatória, e exige usuário e
  senha aleatórios de 128 bits, comparados em tempo constante. Qualquer forma de outro processo
  usar a ponte sem a credencial é uma falha.
- **SSH, FTP(S), S3 e HTTP**: verificação de chave do servidor, de certificado TLS e o
  tratamento de credenciais em memória.
- **O modo de ferramentas** (`TOMNI_UI_TOOLS=1`): fala só pela entrada e pela saída padrão do
  processo que o iniciou, sem abrir porta nem socket. Um app distribuído pode desligá-lo com
  `default-features = false`.
- **O terminal**: sequências de escape que escapem do emulador (escrever arquivos, rodar
  comandos, vazar o conteúdo da tela).

Os testes contra servidores reais usam chaves e certificados de teste, gerados para isso e
versionados em `crates/*/tests/fixtures/`; eles não protegem nada.
