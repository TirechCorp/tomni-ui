#!/bin/sh
# Roda DENTRO do contêiner (Alpine): sobe os servidores OpenVPN de teste com as fixtures de /f
# (somente leitura) e, dentro da rede de cada um, os serviços que os testes alcançam pelo túnel.
# Quem chama é o rodar.sh. Chaves só de teste. Nada daqui é exposto ao app: o app só chega lá
# pelo túnel, e as portas do contêiner que o computador enxerga são as do próprio OpenVPN.
#   19194/udp  tls-auth                  tun0  10.8.1.1  (e o DNS da rede, empurrado)
#   19195/udp  tls-crypt                 tun1  10.8.2.1
#   19196/tcp  sem proteção extra        tun2  10.8.3.1
#   19197/udp  tls-crypt, TLS 1.3 e chaves pelo exportador  tun3  10.8.4.1
#   19198/udp  tls-crypt, só ChaCha20-Poly1305              tun4  10.8.5.1
#   19199/udp  tls-crypt, compress stub-v2, keepalive 2 8 (o teste do ping)  tun5  10.8.6.1
# Em cada endereço do servidor: eco na porta 7000, um banner na 7001 (no lugar de um sshd).
set -e
apk add -q openvpn socat dnsmasq iproute2 >/dev/null
cd /f
cat > /tmp/comum.conf <<'X'
dev-type tun
mode server
tls-server
topology subnet
ca ca.crt
cert servidor.crt
key servidor.key
dh none
auth SHA256
data-ciphers AES-256-GCM:AES-128-GCM:CHACHA20-POLY1305
verb 4
route-noexec
push "route 10.9.0.0 255.255.255.0"
keepalive 10 60
persist-tun
X
# O pidfile de cada servidor deixa o teste do reinício mandar o USR1 para o certo.
sobe() {
  porta=$1
  shift
  openvpn --config /tmp/comum.conf --port "$porta" --writepid "/tmp/ovpn-$porta.pid" "$@" &
}
sobe 19194 --proto udp --dev tun0 --server 10.8.1.0 255.255.255.0 --tls-auth ta.key 0 \
  --explicit-exit-notify 1 --push "dhcp-option DNS 10.8.1.1" --push "dhcp-option DOMAIN corp.test"
sobe 19195 --proto udp --dev tun1 --server 10.8.2.0 255.255.255.0 --tls-crypt tc.key
sobe 19196 --proto tcp-server --dev tun2 --server 10.8.3.0 255.255.255.0
sobe 19197 --proto udp --dev tun3 --server 10.8.4.0 255.255.255.0 --tls-crypt tc.key \
  --tls-version-min 1.3
sobe 19198 --proto udp --dev tun4 --server 10.8.5.0 255.255.255.0 --tls-crypt tc.key \
  --data-ciphers CHACHA20-POLY1305
sobe 19199 --proto udp --dev tun5 --server 10.8.6.0 255.255.255.0 --tls-crypt tc.key \
  --compress stub-v2 --allow-compression yes --keepalive 2 8

# Os serviços de dentro da rede: esperam as interfaces subirem.
for i in $(seq 1 60); do
  [ "$(ip -o addr show | grep -c ' 10\.8\.[1-6]\.1/24')" -ge 6 ] && break
  sleep 1
done
socat TCP-LISTEN:7000,fork,reuseaddr EXEC:cat &
socat TCP-LISTEN:7001,fork,reuseaddr SYSTEM:"echo SSH-2.0-OpenSSH_teste" &
dnsmasq --keep-in-foreground --no-resolv --no-hosts --bind-interfaces \
  --listen-address=10.8.1.1 --address=/alvo.corp.test/10.8.1.1 --log-facility=- &
echo "servicos prontos"
wait
