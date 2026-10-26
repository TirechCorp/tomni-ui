#!/usr/bin/env bash
# Sobe a rede ZeroTier oficial (Docker), roda os testes reais do M61 e derruba.
#   testes/zerotier/rodar.sh                        roda e derruba no fim
#   MANTER=1 testes/zerotier/rodar.sh               deixa a rede de pé (`docker compose down` derruba)
#   ZEROTIER_VERSAO=1.14.2 testes/zerotier/rodar.sh usa outra versão da imagem do ZeroTier
#
# Tudo roda em contêineres próprios (nomes `zt-*`, redes 172.31.61.0/24 e 172.31.62.0/24): o
# ZeroTier do computador, se houver, não é tocado.
set -euo pipefail
cd "$(dirname "$0")"
pasta="$PWD"
raiz="$(cd ../.. && pwd)"
export PATH="$HOME/.cargo/bin:$PATH"
export ZEROTIER_VERSAO="${ZEROTIER_VERSAO:-1.14.2}"
ids="$raiz/crates/zerotier/tests/fixtures/identidades"

# O planet de teste: o `raiz` (id1) é o único raiz; o id3 assina.
mkdir -p gerado/moons.d
(cd "$raiz" && cargo run -q -p tomni-ui-zerotier --example planet_teste -- \
    "$pasta/gerado/planet" "$ids/id3.secret" \
    "$ids/id1.public=172.31.61.10:9993,172.31.62.10:9993")

docker compose up -d --build

if [ -z "${MANTER:-}" ]; then
    trap 'cd "$pasta" && docker compose down --remove-orphans >/dev/null 2>&1 || true' EXIT
fi

# O controlador da imagem: a 1.16 em diante pode vir sem ele.
if ! docker compose exec -T controlador curl -sf -H 'X-ZT1-Auth: tokenteste' \
    http://127.0.0.1:9993/controller >/dev/null 2>&1; then
    # Dá tempo de o zerotier-one subir antes de desistir.
    sleep 5
    docker compose exec -T controlador curl -sf -H 'X-ZT1-Auth: tokenteste' \
        http://127.0.0.1:9993/controller >/dev/null 2>&1 || {
        echo "a imagem ${ZEROTIER_VERSAO} não tem o controlador; use ZEROTIER_VERSAO=1.14.2" >&2
        exit 1
    }
fi

# Espera o sshd e o eco dos nós de serviço.
for servico in servicos escondido; do
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

cd "$raiz"
cargo test -p tomni-ui-zerotier --features test-util --test zerotier_real -- --ignored --test-threads=1 --nocapture
