#!/bin/sh
# Cria (ou recria) a interface `wg0` do servidor a partir de /etc/wireguard/wg0.conf.
set -eu
ip link del wg0 2>/dev/null || true
ip link add wg0 type wireguard
wg setconf wg0 /etc/wireguard/wg0.conf.strip
ip address add 10.66.0.1/24 dev wg0
ip link set wg0 up
