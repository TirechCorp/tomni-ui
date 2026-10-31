#!/usr/bin/env bash
# Sobe a rede Nebula oficial (Docker), roda os testes reais do M48 e derruba.
#   testes/nebula/rodar.sh                       roda e derruba no fim
#   MANTER=1 testes/nebula/rodar.sh              deixa a rede de pé (derrube com `docker compose down`)
#   NEBULA_VERSAO=1.10.3 testes/nebula/rodar.sh  usa outra versão da imagem do Nebula
set -euo pipefail
cd "$(dirname "$0")"
pasta="$PWD"
export PATH="$HOME/.cargo/bin:$PATH"

docker compose up -d --build

if [ -z "${MANTER:-}" ]; then
    trap 'cd "$pasta" && docker compose down --remove-orphans >/dev/null 2>&1 || true' EXIT
fi

# Espera o sshd e o eco de cada nó direto.
for servico in servicos servicos-chacha servicos-v1 servicos-atras servicos-atras-v1; do
    ok=0
    for _ in $(seq 1 60); do
        if docker compose exec -T "$servico" sh -c 'nc -z 127.0.0.1 22 && nc -z 127.0.0.1 7' >/dev/null 2>&1; then
            ok=1
            break
        fi
        sleep 1
    done
    if [ "$ok" != 1 ]; then
        echo "o serviço $servico não ficou pronto" >&2
        docker compose logs --tail 40 >&2
        exit 1
    fi
done

cd ../..
cargo test -p tomni-ui-nebula --test nebula_real -- --ignored --test-threads=1 --nocapture
