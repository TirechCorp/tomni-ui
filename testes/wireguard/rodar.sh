#!/usr/bin/env bash
# Sobe o servidor WireGuard de verdade (Docker), roda os testes reais do M57 e derruba.
#   testes/wireguard/rodar.sh            roda e derruba no fim
#   MANTER=1 testes/wireguard/rodar.sh   deixa o servidor de pé (derrube com `docker compose down`)
# Precisa do módulo `wireguard` no kernel do computador (Linux 5.6 ou mais novo) e do Docker.
set -euo pipefail
cd "$(dirname "$0")"
pasta="$PWD"
raiz="$(cd ../.. && pwd)"
export PATH="$HOME/.cargo/bin:$PATH"

saida="$raiz/target/testes-wireguard"
rm -rf "$saida" 2>/dev/null || docker run --rm -v "$raiz/target:/t" debian:bookworm-slim rm -rf /t/testes-wireguard
mkdir -p "$saida"
export WG_SAIDA="$saida"

docker compose up -d --build

if [ -z "${MANTER:-}" ]; then
    trap 'cd "$pasta" && docker compose down --remove-orphans >/dev/null 2>&1 || true' EXIT
fi

ok=0
for _ in $(seq 1 60); do
    if docker compose exec -T wg sh -c 'nc -z 127.0.0.1 22 && nc -z 127.0.0.1 7 && wg show wg0 >/dev/null' >/dev/null 2>&1 \
        && [ -f "$saida/cliente.conf" ]; then
        ok=1
        break
    fi
    sleep 1
done
if [ "$ok" != 1 ]; then
    echo "o servidor WireGuard não ficou pronto (o kernel tem o módulo wireguard?)" >&2
    docker compose logs --tail 40 >&2
    exit 1
fi

cd ../..
TOMNI_UI_WIREGUARD_TESTE="$saida" \
    cargo test -p tomni-ui-wireguard --test wireguard_real -- --ignored --test-threads=1 --nocapture
