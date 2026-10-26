#!/bin/sh
# Sobe o sshd, o eco e, se PAPEL=no, o tailscaled em userspace ligado ao headscale.
set -eu
/usr/sbin/sshd
socat TCP-LISTEN:7,fork,reuseaddr EXEC:cat &
if [ "${PAPEL:-no}" = alvo ]; then
    exec sleep infinity
fi

# O headscale anuncia o DERP embutido em 127.0.0.1:18443 (o endereço que o computador alcança);
# aqui dentro o mesmo endereço leva ao contêiner do headscale.
socat TCP-LISTEN:18443,bind=127.0.0.1,fork,reuseaddr TCP:headscale:8443 &
export SSL_CERT_FILE=/saida/tls/ca.pem
mkdir -p /var/run/tailscale
tailscaled --tun=userspace-networking --state=mem: --socket=/var/run/tailscale/tailscaled.sock &
for _ in $(seq 1 50); do
    [ -S /var/run/tailscale/tailscaled.sock ] && break
    sleep 0.2
done
while [ ! -s /saida/chave-auth ]; do sleep 0.5; done
rotas=""
[ -n "${ROTAS:-}" ] && rotas="--advertise-routes=$ROTAS"
until tailscale up --login-server=https://127.0.0.1:18443 --authkey="$(cat /saida/chave-auth)" \
    --hostname="$NOME" --accept-dns=false $rotas; do
    sleep 1
done
echo "tailscale pronto: $NOME"
wait
