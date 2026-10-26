#!/bin/sh
# Prepara o nó do teste real e entrega para o `entrypoint.sh` da imagem oficial.
#   IDENTIDADE   nome da fixture (`id1`, `id4`…) em /fixtures/identidades; sem ela o nó gera a dele
#   SERVICOS=1   sobe o sshd e o eco da porta 7
#   GATEWAY=1    liga o NAT da rede gerenciada (10.61.0.0/24); o encaminhamento vem do compose
#   MOON         arquivo `.moon` em /gerado/moons.d que entra na `moons.d` do nó
#   FAZER_MOON   endereço:porta de onde o nó é alcançável: ele gera a própria moon (id = o
#                endereço dele, com ele mesmo de raiz) e a põe na `moons.d`
#   JOIN         redes a entrar (ZEROTIER_JOIN_NETWORKS, passado ao entrypoint)
set -e
home=/var/lib/zerotier-one
mkdir -p "$home"
cp /gerado/planet "$home/planet"
if [ -n "$IDENTIDADE" ]; then
    cp "/fixtures/identidades/$IDENTIDADE.secret" "$home/identity.secret"
    cp "/fixtures/identidades/$IDENTIDADE.public" "$home/identity.public"
    chmod 600 "$home/identity.secret"
fi
if [ -n "$MOON" ]; then
    mkdir -p "$home/moons.d"
    cp "/gerado/moons.d/$MOON" "$home/moons.d/"
fi
if [ -n "$FAZER_MOON" ]; then
    mkdir -p "$home/moons.d"
    (
        cd /tmp
        zerotier-idtool initmoon "$home/identity.public" > moon.json
        sed -z "s|\"stableEndpoints\": *\[[^]]*\]|\"stableEndpoints\": [\"$FAZER_MOON\"]|" moon.json > moon2.json
        zerotier-idtool genmoon moon2.json
        cp ./*.moon "$home/moons.d/"
    )
fi
if [ -n "$TOKEN" ]; then
    echo "$TOKEN" > "$home/authtoken.secret"
    chmod 600 "$home/authtoken.secret"
fi
if [ "$SERVICOS" = 1 ]; then
    socat TCP-LISTEN:7,fork,reuseaddr EXEC:cat &
    /usr/sbin/sshd -e -o PasswordAuthentication=yes -o UsePAM=no
fi
if [ "$GATEWAY" = 1 ]; then
    iptables -t nat -A POSTROUTING -s 10.61.0.0/24 -j MASQUERADE
fi
exec /entrypoint.sh
