#!/bin/sh
# Roda os testes `#[ignore]` de crates/openvpn/tests/openvpn_real.rs contra servidores OpenVPN
# de verdade, num contêiner Alpine (precisa do Docker e de rede para o `apk add openvpn`).
# As portas 19194 a 19199 ficam só no 127.0.0.1 do próprio computador.
set -eu
aqui=$(cd "$(dirname "$0")" && pwd)
raiz=$(cd "$aqui/../.." && pwd)
nome=tomni-ui-openvpn-teste
docker rm -f "$nome" >/dev/null 2>&1 || true
trap 'docker rm -f "$nome" >/dev/null 2>&1 || true' EXIT
docker run -d --name "$nome" \
  -p 127.0.0.1:19194:19194/udp -p 127.0.0.1:19195:19195/udp \
  -p 127.0.0.1:19196:19196/tcp -p 127.0.0.1:19197:19197/udp \
  -p 127.0.0.1:19198:19198/udp -p 127.0.0.1:19199:19199/udp \
  --cap-add NET_ADMIN --device /dev/net/tun \
  -v "$raiz/crates/openvpn/tests/fixtures:/f:ro" \
  -v "$aqui/servidores.sh:/servidores.sh:ro" \
  alpine:3.22 sh /servidores.sh >/dev/null
echo "esperando os servidores..."
i=0
while [ "$(docker logs "$nome" 2>&1 | grep -c 'servicos prontos')" -lt 1 ]; do
  i=$((i + 1))
  if [ "$i" -gt 90 ]; then
    docker logs "$nome" 2>&1 | tail -30
    echo "os servidores não subiram" >&2
    exit 1
  fi
  sleep 1
done
cd "$raiz"
TOMNI_UI_OPENVPN_CONTAINER="$nome" \
  cargo test -p tomni-ui-openvpn --test openvpn_real -- --ignored --test-threads=1
