#!/bin/sh
# O "CGNAT" do teste real dos relays: na rede do nó, só o UDP de e para o relay passa. Depois,
# os mesmos serviços dos outros nós (o eco e o sshd).
set -eu
relay=""
for _ in $(seq 1 30); do
    relay=$(getent hosts "$1" | awk '{print $1}')
    [ -n "$relay" ] && break
    sleep 1
done
[ -n "$relay" ] || { echo "sem o endereço de $1" >&2; exit 1; }
iptables -A INPUT -p udp --dport 4242 ! -s "$relay" -j DROP
iptables -A OUTPUT -p udp --sport 4242 ! -d "$relay" -j DROP
touch /tmp/so-pelo-relay
socat TCP-LISTEN:7,fork,reuseaddr EXEC:cat &
exec /usr/sbin/sshd -D -e -o PasswordAuthentication=yes -o UsePAM=no
