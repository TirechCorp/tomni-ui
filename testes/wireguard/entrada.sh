#!/bin/sh
# Gera as chaves, escreve o wg0.conf do servidor e os .conf dos clientes em /saida, sobe a
# interface, o eco (porta 7) e o sshd.
set -eu
umask 022
mkdir -p /etc/wireguard
srv_priv=$(wg genkey)
srv_pub=$(printf '%s' "$srv_priv" | wg pubkey)
cli_priv=$(wg genkey)
cli_pub=$(printf '%s' "$cli_priv" | wg pubkey)
psk_priv=$(wg genkey)
psk_pub=$(printf '%s' "$psk_priv" | wg pubkey)
psk=$(wg genpsk)

cat > /etc/wireguard/wg0.conf.strip <<EOF
[Interface]
PrivateKey = $srv_priv
ListenPort = 51820

[Peer]
PublicKey = $cli_pub
AllowedIPs = 10.66.0.2/32

[Peer]
PublicKey = $psk_pub
PresharedKey = $psk
AllowedIPs = 10.66.0.3/32
EOF

cat > /saida/cliente.conf <<EOF
# Cliente do teste real do M57 (sem chave pré-compartilhada).
[Interface]
PrivateKey = $cli_priv
Address = 10.66.0.2/24

[Peer]
PublicKey = $srv_pub
Endpoint = 127.0.0.1:51890
AllowedIPs = 10.66.0.0/24
PersistentKeepalive = 25
EOF

cat > /saida/cliente-psk.conf <<EOF
# Cliente do teste real do M57, com PresharedKey.
[Interface]
PrivateKey = $psk_priv
Address = 10.66.0.3/24

[Peer]
PublicKey = $srv_pub
PresharedKey = $psk
Endpoint = 127.0.0.1:51890
AllowedIPs = 10.66.0.0/24
EOF
chmod a+rw /saida/*.conf

/subir-wg.sh
socat TCP-LISTEN:7,fork,reuseaddr EXEC:cat &
exec /usr/sbin/sshd -D -e -o PasswordAuthentication=yes -o UsePAM=no
