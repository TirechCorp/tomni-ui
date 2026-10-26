#!/usr/bin/env bash
# Extrai o `planet` (o mundo oficial do ZeroTier, com as raízes e as assinaturas) de uma imagem
# oficial do ZeroTier e grava em `crates/zerotier/planet/terra.planet`. O nó escreve o arquivo na
# pasta dele ao iniciar; o contêiner roda sem rede (`--network none`) e não toca o ZeroTier do
# computador. Rode só quando as raízes mudarem: o arquivo vai junto no crate.
#
#   extrair-planet.sh                    usa zerotier/zerotier:1.14.2
#   ZT_IMAGEM=zerotier/zerotier:1.16.2 extrair-planet.sh
set -euo pipefail

imagem="${ZT_IMAGEM:-zerotier/zerotier:1.14.2}"
destino="$(cd "$(dirname "$0")/../.." && pwd)/crates/zerotier/planet/terra.planet"
nome=tomni-zt-planet

trap 'docker rm -f "$nome" >/dev/null 2>&1 || true' EXIT
docker rm -f "$nome" >/dev/null 2>&1 || true
docker run -d --name "$nome" --network none "$imagem" >/dev/null
for _ in $(seq 1 30); do
    docker exec "$nome" test -s /var/lib/zerotier-one/planet 2>/dev/null && break
    sleep 1
done
docker cp "$nome:/var/lib/zerotier-one/planet" "$destino"
echo "planet de $imagem ($(wc -c < "$destino") bytes) em $destino"
