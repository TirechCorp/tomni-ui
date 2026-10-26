#!/usr/bin/env bash
# Gera as fixtures do tomni-ui-openvpn em crates/openvpn/tests/fixtures/.
#
# Chaves só de teste: nunca use num servidor de verdade (ver LEIAME.md).
#
# Precisa do `openssl` e do `openvpn` (2.6 ou mais novo) no PATH. Sem eles, o script se
# executa de novo dentro de um contêiner Debian (precisa do Docker).
#
# As fixtures são versionadas: os testes comuns não precisam disto nem do Docker. Rode de novo
# só quando a lista de fixtures mudar; os testes leem as datas e as impressões do
# `referencias.txt`, então um conjunto novo continua valendo.
set -euo pipefail

aqui="$(cd "$(dirname "$0")" && pwd)"
raiz="$(cd "$aqui/../.." && pwd)"
destino="$raiz/crates/openvpn/tests/fixtures"

if ! command -v openssl >/dev/null || ! command -v openvpn >/dev/null; then
  command -v docker >/dev/null || { echo "faltam o openssl e o openvpn, e não há Docker" >&2; exit 1; }
  exec docker run --rm -v "$raiz:/repo" -w /repo -e "DONO=$(id -u):$(id -g)" debian:bookworm bash -c \
    'apt-get update -qq && apt-get install -y -qq openssl openvpn >/dev/null && testes/openvpn/gerar-fixtures.sh && chown -R "$DONO" crates/openvpn/tests/fixtures'
fi

mkdir -p "$destino"
trabalho="$(mktemp -d)"
trap 'rm -rf "$trabalho"' EXIT
cd "$trabalho"

dias=3650
senha_chave="senha-de-teste"
assunto() { echo "/C=BR/ST=Sao Paulo/L=Sao Paulo/O=Tomni Teste/OU=Fixtures/CN=$1"; }

# --- Autoridades -------------------------------------------------------------------------
# $1 nome do arquivo, $2 CN, $3 algoritmo (rsa|ec), $4 digest
nova_ca() {
  local nome="$1" cn="$2" alg="$3" dig="$4"
  if [ "$alg" = rsa ]; then
    openssl genrsa -out "$nome.key" 2048 2>/dev/null
  else
    openssl ecparam -name prime256v1 -genkey -noout -out "$nome.key"
  fi
  openssl req -new -x509 -"$dig" -key "$nome.key" -days "$dias" -subj "$(assunto "$cn")" \
    -addext "basicConstraints=critical,CA:TRUE" -addext "keyUsage=critical,keyCertSign,cRLSign" \
    -out "$nome.crt"
}
nova_ca ca "Tomni Teste CA" rsa sha256
nova_ca ca-ec "Tomni Teste CA EC" ec sha256
nova_ca ca-sha1 "Tomni Teste CA SHA1" rsa sha1
nova_ca outra-ca "Outra CA de Teste" rsa sha256

# --- Certificados folha --------------------------------------------------------------------
# $1 arquivo, $2 CN, $3 CA, $4 tipo (servidor|cliente|servidor-sem-eku), $5 algoritmo, $6 digest,
# $7 e $8 (opcionais) início e fim no formato do `openssl ca` (AAAAMMDDhhmmssZ).
nova_folha() {
  local nome="$1" cn="$2" ca="$3" tipo="$4" alg="$5" dig="$6" inicio="${7:-}" fim="${8:-}"
  if [ "$alg" = rsa ]; then
    openssl genrsa -out "$nome.key.rsa" 2048 2>/dev/null
    # PKCS#1 ("BEGIN RSA PRIVATE KEY"), como o easy-rsa antigo grava.
    openssl rsa -in "$nome.key.rsa" -traditional -out "$nome.key" 2>/dev/null
    openssl pkcs8 -topk8 -nocrypt -in "$nome.key.rsa" -out "$nome.pkcs8.key"
    rm "$nome.key.rsa"
  else
    # SEC1 ("BEGIN EC PRIVATE KEY").
    openssl ecparam -name prime256v1 -genkey -noout -out "$nome.key"
    openssl pkcs8 -topk8 -nocrypt -in "$nome.key" -out "$nome.pkcs8.key"
  fi
  openssl req -new -key "$nome.key" -subj "$(assunto "$cn")" -out "$nome.csr"
  case "$tipo" in
    servidor) ext="basicConstraints=CA:FALSE
keyUsage=digitalSignature,keyEncipherment
extendedKeyUsage=serverAuth";;
    servidor-sem-eku) ext="basicConstraints=CA:FALSE
keyUsage=digitalSignature,keyEncipherment";;
    cliente) ext="basicConstraints=CA:FALSE
keyUsage=digitalSignature
extendedKeyUsage=clientAuth";;
  esac
  printf '%s\n' "$ext" > "$nome.ext"
  if [ -n "$inicio" ]; then
    # `openssl ca` aceita datas fixas; é o jeito de gerar um certificado já vencido.
    mkdir -p "ca-$ca"; : > "ca-$ca/index.txt"; echo 01 > "ca-$ca/serial"
    cat > "ca-$ca/openssl.cnf" <<CNF
[ca]
default_ca = padrao
[padrao]
database = ca-$ca/index.txt
new_certs_dir = ca-$ca
serial = ca-$ca/serial
default_md = $dig
policy = livre
unique_subject = no
copy_extensions = none
x509_extensions = ext
[livre]
commonName = supplied
[ext]
$ext
CNF
    openssl ca -batch -config "ca-$ca/openssl.cnf" -cert "$ca.crt" -keyfile "$ca.key" \
      -startdate "$inicio" -enddate "$fim" -notext -in "$nome.csr" -out "$nome.crt" 2>/dev/null
  else
    openssl x509 -req -"$dig" -in "$nome.csr" -CA "$ca.crt" -CAkey "$ca.key" -CAcreateserial \
      -days "$dias" -extfile "$nome.ext" -out "$nome.crt" 2>/dev/null
  fi
  rm -f "$nome.csr" "$nome.ext"
}

nova_folha servidor servidor ca servidor rsa sha256
nova_folha servidor-sem-eku servidor-sem-eku ca servidor-sem-eku rsa sha256
nova_folha servidor-ec servidor-ec ca-ec servidor ec sha256
nova_folha servidor-outra-ca servidor outra-ca servidor rsa sha256
nova_folha cliente-rsa cliente-rsa ca cliente rsa sha256
nova_folha cliente-ec cliente-ec ca-ec cliente ec sha256
nova_folha cliente-vencido cliente-vencido ca cliente rsa sha256 20200101000000Z 20200201000000Z
nova_folha cliente-outra-ca cliente-outra-ca outra-ca cliente rsa sha256
nova_folha cliente-sha1 cliente-sha1 ca-sha1 cliente rsa sha1
# Uma chave que não é a do cliente-rsa: serve para o teste "a chave não corresponde".
openssl genrsa -out chave-trocada.key 2048 2>/dev/null

# PKCS#8 igual ao do cliente-rsa e as duas formas de chave com senha.
cp cliente-rsa.pkcs8.key cliente-pkcs8.key  # (os outros *.pkcs8.key são apagados no fim)
cp cliente-rsa.crt cliente-pkcs8.crt
openssl pkcs8 -topk8 -v2 aes-256-cbc -in cliente-rsa.key -passout "pass:$senha_chave" -out cliente-cifrado.key
openssl rsa -in cliente-rsa.key -traditional -aes256 -passout "pass:$senha_chave" -out cliente-cifrado-legado.key 2>/dev/null
cp cliente-rsa.crt cliente-cifrado.crt
cp cliente-rsa.crt cliente-cifrado-legado.crt

# --- Chaves estáticas do OpenVPN -----------------------------------------------------------
openvpn --genkey secret ta.key >/dev/null 2>&1
openvpn --genkey secret tc.key >/dev/null 2>&1
openvpn --genkey tls-crypt-v2-server tc2-servidor.key >/dev/null 2>&1
openvpn --tls-crypt-v2 tc2-servidor.key --genkey tls-crypt-v2-client tc2-cliente.key >/dev/null 2>&1

# --- .ovpn de exemplo ----------------------------------------------------------------------
bloco() { echo "<$1>"; sed -n '/-----BEGIN/,/-----END/p' "$2"; echo "</$1>"; }
bloco_chave_estatica() { echo "<$1>"; cat "$2"; echo "</$1>"; }

cabecalho() {
  cat <<'CNF'
# Perfil de teste do tomni-ui-openvpn. Chaves só de teste.
client
dev tun
proto udp
remote vpn.exemplo.test 1194
resolv-retry infinite
nobind
persist-key
persist-tun
remote-cert-tls server
cipher AES-256-GCM
data-ciphers AES-256-GCM:AES-128-GCM:CHACHA20-POLY1305
auth SHA256
verb 3
CNF
}

{
  cabecalho
  echo "key-direction 1"
  bloco ca ca.crt
  bloco cert cliente-rsa.crt
  bloco key cliente-rsa.key
  bloco_chave_estatica tls-auth ta.key
} > tudo-embutido.ovpn

{
  cabecalho
  echo "ca ca.crt"
  echo "cert cliente-rsa.crt"
  echo "key cliente-rsa.key"
  echo "tls-crypt tc.key"
} > com-arquivos.ovpn

{
  cat <<'CNF'
client
dev tun
remote-random
<connection>
remote a.exemplo.test 1194 udp
</connection>
<connection>
remote b.exemplo.test 443 tcp
http-proxy-option EXT1 "X-Teste: sim"
</connection>
<connection>
remote 192.0.2.10
port 4443
proto tcp-client
</connection>
remote-cert-tls server
CNF
  bloco ca ca.crt
  bloco cert cliente-rsa.crt
  bloco key cliente-rsa.key
  bloco_chave_estatica tls-crypt tc.key
} > connection-blocks.ovpn

{
  cabecalho | sed 's/^dev tun/dev tap/'
  bloco ca ca.crt
} > tap.ovpn

{
  cat <<'CNF'
dev tun
remote vpn.exemplo.test 1194
ifconfig 10.8.0.2 10.8.0.1
secret estatica.key
CNF
} > secret.ovpn

cat > servidor.conf <<CNF
port 1194
proto udp
dev tun
mode server
tls-server
server 10.8.0.0 255.255.255.0
ca ca.crt
cert servidor.crt
key servidor.key
dh none
keepalive 10 60
CNF

{
  cabecalho
  cat <<'CNF'
script-security 2
up /etc/openvpn/up.sh
down /etc/openvpn/down.sh
plugin /usr/lib/openvpn/plugins/openvpn-plugin-auth-pam.so login
CNF
  bloco ca ca.crt
} > scripts.ovpn

cat > antigo-cbc.ovpn <<CNF
client
dev tun
proto tcp
remote antigo.exemplo.test 443
cipher AES-256-CBC
auth SHA1
comp-lzo
tls-version-min 1.2
auth-user-pass
verify-x509-name servidor name
$(bloco ca ca.crt)
$(bloco_chave_estatica tls-auth ta.key)
key-direction 1
CNF

# --- Referências para os testes ---------------------------------------------------------------
# Um formato "arquivo.campo=valor". O sujeito vem no formato que o OpenVPN mostra
# (campos na ordem do certificado, separados por ", ").
{
  for c in ca ca-ec ca-sha1 outra-ca servidor servidor-sem-eku servidor-ec servidor-outra-ca \
           cliente-rsa cliente-ec cliente-vencido cliente-outra-ca cliente-sha1; do
    echo "$c.sha256=$(openssl x509 -in "$c.crt" -noout -fingerprint -sha256 | cut -d= -f2 | tr -d : | tr A-F a-f)"
    echo "$c.sha1=$(openssl x509 -in "$c.crt" -noout -fingerprint -sha1 | cut -d= -f2 | tr -d : | tr A-F a-f)"
    echo "$c.subject=$(openssl x509 -in "$c.crt" -noout -subject -nameopt sep_comma_plus_space,sname,esc_ctrl,utf8 | sed 's/^subject=//; s/^ *//; s/ = /=/g')"
    echo "$c.notAfter=$(date -u -d "$(openssl x509 -in "$c.crt" -noout -enddate | cut -d= -f2)" +%s)"
    echo "$c.notBefore=$(date -u -d "$(openssl x509 -in "$c.crt" -noout -startdate | cut -d= -f2)" +%s)"
  done
  echo "senha_chave=$senha_chave"
} > referencias.txt

# --- Instala ---------------------------------------------------------------------------------
# As chaves privadas das CAs não são usadas pelos testes (só o servidor de teste usa as dos
# servidores), então não vão para a pasta.
rm -f ca.key ca-ec.key ca-sha1.key outra-ca.key *.srl *.pkcs8.key
for f in *; do
  [ -f "$f" ] && cp "$f" "$destino/$f"
done
echo "fixtures em $destino"
