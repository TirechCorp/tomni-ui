#!/usr/bin/env bash
# Gera as fixtures do `tomni-ui-nebula` com o `nebula-cert` oficial, num contêiner Docker, e
# grava em `crates/nebula/tests/fixtures/`. As fixtures ficam versionadas: os testes comuns
# (`cargo test -p tomni-ui-nebula`) não precisam do Docker nem rodam este script.
#   testes/nebula/gerar-fixtures.sh
#   NEBULA_VERSAO=1.10.3 testes/nebula/gerar-fixtures.sh   (a imagem; o padrão é a `latest`)
#
# Chaves só de teste; nunca use numa rede de verdade.
set -euo pipefail
cd "$(dirname "$0")/../.."
raiz="$PWD"
saida="$raiz/crates/nebula/tests/fixtures"
versao="${NEBULA_VERSAO:-latest}"
imagem="nebulaoss/nebula:$versao"

mkdir -p "$saida"
find "$saida" -maxdepth 1 -type f \( -name '*.crt' -o -name '*.key' -o -name '*.pub' -o -name '*.json' \) -delete

# A imagem oficial é `FROM scratch`: só tem o `/nebula` e o `/nebula-cert`, sem shell.
cert() {
    docker run --rm -v "$saida:/out" -w /out --user "$(id -u):$(id -g)" \
        --entrypoint /nebula-cert "$imagem" "$@"
}

echo "== $imagem: $(docker run --rm --entrypoint /nebula-cert "$imagem" -version | head -1)"

# As CAs duram 100 anos (876000 h), para as fixtures não vencerem e quebrarem os testes. Os nós
# valem até 1 s antes da CA vencer. A `ca-25519` é a v2 (o padrão do `nebula-cert`); a `ca-25519-v1`, a v1; a `ca-outra`, uma
# terceira, que nenhum config confia.
cert ca -duration 876000h -name "CA de teste 25519" -out-crt ca-25519.crt -out-key ca-25519.key
cert ca -duration 876000h -name "CA de teste 25519 v1" -version 1 -out-crt ca-25519-v1.crt -out-key ca-25519-v1.key
cert ca -duration 876000h -name "CA de teste P256" -curve P256 -out-crt ca-p256.crt -out-key ca-p256.key
cert ca -duration 876000h -name "CA de teste outra" -out-crt ca-outra.crt -out-key ca-outra.key

# no <nome> <rede> <ca> <versão> [opções do sign…]
no() {
    local nome="$1" rede="$2" ca="$3" versao="$4"
    shift 4
    cert sign -ca-crt "$ca.crt" -ca-key "$ca.key" -name "$nome" -networks "$rede" \
        -version "$versao" -out-crt "$nome.crt" -out-key "$nome.key" "$@"
}

# Os nós dos testes de certificado e de handshake (rede 10.42.0.0/24).
no no-v2 10.42.0.20/24 ca-25519 2
no no-v2-b 10.42.0.42/24 ca-25519 2
no no-v1 10.42.0.21/24 ca-25519 1
no no-v1-b 10.42.0.41/24 ca-25519 1
no no-v1-ca-v1 10.42.0.40/24 ca-25519-v1 1
no no-chacha 10.42.0.23/24 ca-25519 2
no no-p256 10.42.0.24/24 ca-p256 2
no no-p256-b 10.42.0.44/24 ca-p256 2
no no-unsafe 10.42.0.25/24 ca-25519 2 -unsafe-networks 192.168.50.0/24
no no-grupos 10.42.0.26/24 ca-25519 2 -groups admin,dev
no no-outra-ca 10.42.0.27/24 ca-outra 2

# Um nó com os dois certificados (v1 e v2) da mesma chave: o `.crt` leva os dois PEM juntos.
# no_v1v2 <nome> <rede>
no_v1v2() {
    local nome="$1" rede="$2"
    cert keygen -out-key "$nome.key" -out-pub "$nome.pub"
    for versao in 1 2; do
        cert sign -ca-crt ca-25519.crt -ca-key ca-25519.key -name "$nome" -networks "$rede" \
            -version "$versao" -in-pub "$nome.pub" -out-crt "$nome-v$versao.crt"
    done
    cat "$saida/$nome-v1.crt" "$saida/$nome-v2.crt" >"$saida/$nome.crt"
    rm -f "$saida/$nome-v1.crt" "$saida/$nome-v2.crt" "$saida/$nome.pub"
}
no_v1v2 no-v1v2 10.42.0.22/24
no_v1v2 no-v1v2-b 10.42.0.43/24

# Vencido: dura 1 s, e o script espera passar.
no no-vencido 10.42.0.28/24 ca-25519 2 -duration 1s
sleep 2

# Os nós da rede de teste real (testes/nebula/docker-compose.yml): o lighthouse, o `direto`, o nó do
# app e, para a cifra ChaCha20-Poly1305, um segundo lighthouse e um segundo `direto`.
no lighthouse 10.42.0.1/24 ca-25519 2 -groups lighthouse
no direto 10.42.0.10/24 ca-25519 2
no app 10.42.0.30/24 ca-25519 2
no lighthouse-chacha 10.42.0.3/24 ca-25519 2 -groups lighthouse
no direto-chacha 10.42.0.12/24 ca-25519 2
no app-chacha 10.42.0.31/24 ca-25519 2
# As mesmas redes com certificados v1 (o teste real confere os dois formatos).
no lighthouse-v1 10.42.0.4/24 ca-25519 1 -groups lighthouse
no direto-v1 10.42.0.13/24 ca-25519 1
no app-v1 10.42.0.32/24 ca-25519 1

# Para cada certificado, a saída do `print -json`, que os testes comparam com o que o parser lê.
for crt in "$saida"/*.crt; do
    cert print -path "$(basename "$crt")" -json >"${crt%.crt}.json"
done

# As chaves privadas das CAs só servem a este script.
echo "== fixtures em $saida:"
ls "$saida" | wc -l
