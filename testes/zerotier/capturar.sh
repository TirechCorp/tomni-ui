#!/usr/bin/env bash
# Captura, entre nós oficiais do ZeroTier (Docker), os pacotes que os testes de `tomni-ui-zerotier`
# usam como vetores (`tests/pacote.rs`): o HELLO em claro e o OK(HELLO) de um nó atual (1.14.2) e
# o tráfego de um nó antigo (1.4.6, que só fala Salsa20/12) com o atual. As identidades são as
# das fixtures de `gerar-fixtures.sh` (id1 = A atual, id2 = B atual, id3 = C antigo): rode o
# `gerar-fixtures.sh` de novo e este script tem de ser rodado de novo também.
#
# Nada aqui toca o ZeroTier do computador: os nós são contêineres numa rede interna própria
# (sem saída para a internet, então os raiz do planet ficam sem resposta).
#
#   capturar.sh                        usa as imagens zerotier/zerotier:1.14.2 e :1.4.6
#   ZT_ATUAL=1.12.2 ZT_ANTIGA=1.6.6 capturar.sh
set -euo pipefail

atual="${ZT_ATUAL:-1.14.2}"
antiga="${ZT_ANTIGA:-1.4.6}"
raiz="$(cd "$(dirname "$0")/../.." && pwd)"
fix="$raiz/crates/zerotier/tests/fixtures"
ids="$fix/identidades"
saida="$fix/captura"
trabalho="$(mktemp -d)"
rede=tomni-zt-cap
segundos="${SEGUNDOS:-25}"

for cmd in docker zerotier-idtool python3; do
    command -v "$cmd" >/dev/null || { echo "faltou $cmd" >&2; exit 1; }
done
[ -f "$ids/id3.secret" ] || { echo "rode gerar-fixtures.sh antes" >&2; exit 1; }

limpar() {
    docker rm -f tomni-zt-a tomni-zt-b tomni-zt-c tomni-zt-cap >/dev/null 2>&1 || true
    docker network rm "$rede" >/dev/null 2>&1 || true
    rm -rf "$trabalho"
}
trap limpar EXIT
limpar
trap limpar EXIT
trabalho="$(mktemp -d)"

# A imagem do tcpdump (o contêiner usa a rede do nó que ele observa).
printf 'FROM alpine:3\nRUN apk add --no-cache tcpdump\nENTRYPOINT ["tcpdump"]\n' | docker build -q -t tomni-zt-tcpdump - >/dev/null

# A moon de B: um mundo com B como único raiz, no endereço fixo da rede de captura.
zerotier-idtool initmoon "$ids/id2.public" > "$trabalho/moon.json"
python3 - "$trabalho/moon.json" <<'PY'
import json, sys
m = json.load(open(sys.argv[1]))
m["roots"][0]["stableEndpoints"] = ["172.31.70.11/9993"]
json.dump(m, open(sys.argv[1], "w"), indent=1)
PY
mkdir "$trabalho/moons.d"
(cd "$trabalho/moons.d" && zerotier-idtool genmoon "$trabalho/moon.json" >/dev/null)
moon="$(ls "$trabalho/moons.d")"

docker network create --subnet 172.31.70.0/24 --internal "$rede" >/dev/null

no() { # nome ip imagem id
    docker run -d --name "$1" --net "$rede" --ip "$2" \
        -v "$trabalho/moons.d:/var/lib/zerotier-one/moons.d:ro" \
        -e ZEROTIER_IDENTITY_PUBLIC="$(cat "$ids/$4.public")" \
        -e ZEROTIER_IDENTITY_SECRET="$(cat "$ids/$4.secret")" \
        "zerotier/zerotier:$3" >/dev/null
}

no tomni-zt-b 172.31.70.11 "$atual" id2
docker run -d --name tomni-zt-cap --net container:tomni-zt-b -v "$trabalho:/cap" \
    tomni-zt-tcpdump -i eth0 -U -w /cap/captura.pcap udp port 9993 >/dev/null
sleep 1
no tomni-zt-a 172.31.70.10 "$atual" id1
no tomni-zt-c 172.31.70.12 "$antiga" id3
sleep "$segundos"

{
    echo "# O que $(basename "$0") capturou ($(date -u +%Y-%m-%dT%H:%M:%SZ))"
    echo "atual: $(docker exec tomni-zt-b zerotier-one -v | head -1)"
    echo "antiga: $(docker exec tomni-zt-c zerotier-one -v | head -1)"
    echo "moon: $moon"
    echo "A (id1, atual):  $(cut -d: -f1 "$ids/id1.public") 172.31.70.10"
    echo "B (id2, atual, raiz da moon): $(cut -d: -f1 "$ids/id2.public") 172.31.70.11"
    echo "C (id3, antiga): $(cut -d: -f1 "$ids/id3.public") 172.31.70.12"
    echo "--- peers de A"; docker exec tomni-zt-a zerotier-cli listpeers | grep -v PLANET
    echo "--- peers de C"; docker exec tomni-zt-c zerotier-cli listpeers | grep -v PLANET
    echo "--- peers de B"; docker exec tomni-zt-b zerotier-cli listpeers | grep -v PLANET
} > "$saida/LEIAME.txt"

docker stop tomni-zt-cap >/dev/null
mkdir -p "$saida"
cp "$trabalho/captura.pcap" "$saida/captura.pcap"
cp "$trabalho/moons.d/$moon" "$saida/$moon"
cat "$saida/LEIAME.txt"
echo "captura: $(wc -c < "$saida/captura.pcap") bytes em $saida/captura.pcap"
