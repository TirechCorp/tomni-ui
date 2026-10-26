#!/usr/bin/env bash
# Sobe o headscale e as máquinas Tailscale de verdade (Docker), roda os testes reais do M57 e
# derruba.
#   testes/tailscale/rodar.sh            roda e derruba no fim
#   MANTER=1 testes/tailscale/rodar.sh   deixa tudo de pé (derrube com `docker compose down -v`)
#   HEADSCALE_VERSAO=0.29.5 TAILSCALE_VERSAO=1.98.4 testes/tailscale/rodar.sh   fixa as versões
# Precisa do Docker, do `openssl` e do `python3`. Usa as portas 18443 (TCP) e 3478 (UDP) do
# próprio computador.
set -euo pipefail
cd "$(dirname "$0")"
pasta="$PWD"
raiz="$(cd ../.. && pwd)"
export PATH="$HOME/.cargo/bin:$PATH"

saida="$raiz/target/testes-tailscale"
rm -rf "$saida" 2>/dev/null || docker run --rm -v "$raiz/target:/t" debian:bookworm-slim rm -rf /t/testes-tailscale
mkdir -p "$saida/tls"
export TS_SAIDA="$saida"

# A CA e o certificado do headscale (só para teste; valem 2 dias).
(
    cd "$saida/tls"
    openssl req -x509 -newkey ec -pkeyopt ec_paramgen_curve:prime256v1 -nodes \
        -keyout ca.key -out ca.pem -days 2 -subj "/CN=Tomni UI teste" \
        -addext "basicConstraints=critical,CA:TRUE" -addext "keyUsage=critical,keyCertSign" 2>/dev/null
    openssl req -newkey ec -pkeyopt ec_paramgen_curve:prime256v1 -nodes \
        -keyout servidor.key -out servidor.csr -subj "/CN=headscale" 2>/dev/null
    printf 'subjectAltName=IP:127.0.0.1,DNS:headscale\nbasicConstraints=CA:FALSE\nextendedKeyUsage=serverAuth\n' > servidor.ext
    openssl x509 -req -in servidor.csr -CA ca.pem -CAkey ca.key -CAcreateserial \
        -out servidor.pem -days 2 -extfile servidor.ext 2>/dev/null
    chmod 644 ./*
)

if [ -z "${MANTER:-}" ]; then
    trap 'cd "$pasta" && docker compose down -v --remove-orphans >/dev/null 2>&1 || true' EXIT
fi
docker compose down -v --remove-orphans >/dev/null 2>&1 || true

hs() { docker compose exec -T headscale headscale "$@"; }

docker compose up -d --build headscale
ok=0
for _ in $(seq 1 60); do
    if hs users list >/dev/null 2>&1; then ok=1; break; fi
    sleep 1
done
if [ "$ok" != 1 ]; then
    echo "o headscale não subiu" >&2
    docker compose logs --tail 40 headscale >&2
    exit 1
fi

hs users create teste >/dev/null
id_usuario="$(hs users list -o json | python3 -c 'import json,sys; print(json.load(sys.stdin)[0]["id"])')"
hs policy set -f /politicas/politica-livre.json >/dev/null
hs preauthkeys create -u "$id_usuario" --reusable -e 24h -o json \
    | python3 -c 'import json,sys; print(json.load(sys.stdin)["key"])' > "$saida/chave-auth"
chmod 644 "$saida/chave-auth"

docker compose up -d --build direto escondido alvo
ok=0
for _ in $(seq 1 90); do
    n="$(hs nodes list -o json 2>/dev/null | python3 -c 'import json,sys; print(len(json.load(sys.stdin)))' 2>/dev/null || echo 0)"
    if [ "$n" -ge 2 ] \
        && docker compose exec -T direto sh -c 'nc -z 127.0.0.1 22 && nc -z 127.0.0.1 7' >/dev/null 2>&1 \
        && hs nodes list-routes 2>/dev/null | grep -q '10.99.0.0/24'; then
        ok=1
        break
    fi
    sleep 1
done
if [ "$ok" != 1 ]; then
    echo "as máquinas Tailscale não entraram na tailnet" >&2
    docker compose logs --tail 40 >&2
    exit 1
fi
{
    echo "headscale: $(hs version 2>&1 | head -1)"
    echo "tailscale: $(docker compose exec -T direto tailscale version 2>&1 | head -1)"
} | tee "$saida/versoes"

cd ../..
TOMNI_UI_TAILSCALE_TESTE="$saida" TOMNI_UI_TAILSCALE_COMPOSE="$pasta" \
    cargo test -p tomni-ui-tailscale --test tailscale_real -- --ignored --test-threads=1 --nocapture
