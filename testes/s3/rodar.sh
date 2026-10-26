#!/usr/bin/env bash
# Sobe o MinIO de verdade (Docker), roda o teste real do M32 e derruba.
#   testes/s3/rodar.sh            roda e derruba no fim
#   MANTER=1 testes/s3/rodar.sh   deixa o servidor de pé (derrube com `docker compose down`)
set -euo pipefail
cd "$(dirname "$0")"
pasta="$PWD"
export PATH="$HOME/.cargo/bin:$PATH"

docker compose up -d

if [ -z "${MANTER:-}" ]; then
    trap 'cd "$pasta" && docker compose down --remove-orphans >/dev/null 2>&1 || true' EXIT
fi

ok=0
for _ in $(seq 1 60); do
    if curl -fs http://127.0.0.1:19000/minio/health/ready >/dev/null 2>&1; then
        ok=1
        break
    fi
    sleep 1
done
if [ "$ok" != 1 ]; then
    echo "o MinIO não ficou pronto" >&2
    docker compose logs --tail 40 >&2
    exit 1
fi

cd ../..
TOMNI_UI_S3_TESTE=http://127.0.0.1:19000 \
    cargo test -p tomni-ui-files --features s3 --test s3_real -- --ignored --test-threads=1 --nocapture
