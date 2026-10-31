#!/usr/bin/env bash
# Sobe os servidores SSH de verdade (Docker), roda o teste real da sonda de pasta e derruba.
#   testes/pasta/rodar.sh            roda e derruba no fim
#   MANTER=1 testes/pasta/rodar.sh   deixa os servidores de pé (derrube com `docker compose down`)
set -euo pipefail
cd "$(dirname "$0")"
pasta="$PWD"
export PATH="$HOME/.cargo/bin:$PATH"

docker compose up -d --build

if [ -z "${MANTER:-}" ]; then
    trap 'cd "$pasta" && docker compose down --remove-orphans >/dev/null 2>&1 || true' EXIT
fi

for porta in 22201 22202 22203 22204; do
    ok=0
    for _ in $(seq 1 60); do
        if (exec 3<>"/dev/tcp/127.0.0.1/$porta" && head -c 4 <&3 | grep -q SSH) 2>/dev/null; then
            ok=1
            break
        fi
        sleep 1
    done
    if [ "$ok" != 1 ]; then
        echo "o servidor da porta $porta não ficou pronto" >&2
        docker compose logs --tail 40 >&2
        exit 1
    fi
done

cd ../..
cargo test -p tomni-ui-terminal --features ssh,pty --test pasta_real -- --ignored --test-threads=1 --nocapture
