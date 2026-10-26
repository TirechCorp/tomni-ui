#!/usr/bin/env bash
# Sobe os três servidores FTP de verdade (Docker), roda a matriz do M31 e derruba.
#   testes/ftp/rodar.sh            roda tudo e derruba no fim
#   MANTER=1 testes/ftp/rodar.sh   deixa os servidores de pé (derrube com `docker compose down`)
set -euo pipefail
cd "$(dirname "$0")"
pasta="$PWD"
export PATH="$HOME/.cargo/bin:$PATH"

docker compose up -d --build

if [ -z "${MANTER:-}" ]; then
    trap 'cd "$pasta" && docker compose down --remove-orphans >/dev/null 2>&1 || true' EXIT
fi

# Espera as portas (a 220 do FTP, ou o aperto de mão do TLS nas implícitas).
for porta in 2121 2122 2123 2124 2990 2991; do
    ok=0
    for _ in $(seq 1 60); do
        if (exec 3<>"/dev/tcp/127.0.0.1/$porta") 2>/dev/null; then
            ok=1
            break
        fi
        sleep 1
    done
    if [ "$ok" != 1 ]; then
        echo "a porta $porta não abriu" >&2
        docker compose logs --tail 40 >&2
        exit 1
    fi
done

cd ../..
cargo test -p tomni-ui-files --features ftp --test ftp_real -- --ignored --test-threads=1 --nocapture
