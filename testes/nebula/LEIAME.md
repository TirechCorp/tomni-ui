# Teste real da rede Nebula (M48)

Este diretório sobe uma rede Nebula **oficial** (as imagens `nebulaoss/nebula`) no Docker, para
conferir que o `tomni-ui-nebula`, que reimplementa o protocolo em Rust, conversa com o `nebula`
de verdade. O CI não roda isto (precisa do Docker); os testes comuns usam a rede simulada em
memória (`crates/nebula/tests/rede.rs`).

- `rodar.sh`: sobe a rede, roda os testes `#[ignore]` de `crates/nebula/tests/nebula_real.rs` e
  derruba. `MANTER=1 testes/nebula/rodar.sh` deixa a rede de pé. `NEBULA_VERSAO=1.10.3` escolhe
  a versão da imagem (o padrão é a `latest`).
- `docker-compose.yml`: três redes independentes (cifra AES, cifra ChaCha20-Poly1305 e certificados
  v1), cada uma com um lighthouse publicado em `127.0.0.1` (portas UDP 14242, 14244 e 14243) e um
  nó `direto` com um sshd (usuário e senha `teste`) e um eco na porta 7.
- `config/`: os `config.yml` dos nós oficiais.
- `servicos/`: a imagem do sshd e do eco.
- `gerar-fixtures.sh`: gera os certificados e as chaves de `crates/nebula/tests/fixtures/` com o
  `nebula-cert` oficial.

**As chaves e os certificados daqui são só de teste; nunca use numa rede de verdade.**
