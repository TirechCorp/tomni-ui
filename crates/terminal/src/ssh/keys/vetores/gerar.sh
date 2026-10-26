#!/usr/bin/env bash
# Gera os vetores de teste das chaves com o `ssh-keygen` (OpenSSH 9.6) e o `puttygen` (0.81).
# NÃO roda nos testes: os arquivos gerados ficam versionados nesta pasta. Rode de novo só para
# refazer os vetores (as chaves mudam, então os `.fp` e o hash em `tests.rs` também).
#
# Todas as senhas são `senha1`. As chaves são de teste, sem valor nenhum.
set -euo pipefail
cd "$(dirname "$0")"
rm -f ed* rsa* ec* ./*.fp

ssh-keygen -q -t ed25519 -N '' -C ed -f ed
ssh-keygen -q -t ed25519 -N 'senha1' -C edc -f ed-cifrada
ssh-keygen -q -t rsa -b 2048 -N '' -C rsa -f rsa
ssh-keygen -q -t ecdsa -b 256 -N '' -C ec -f ec

# PEM tradicional (RSA PRIVATE KEY / EC PRIVATE KEY) e PKCS#8
cp rsa rsa-pem && ssh-keygen -q -p -N '' -m PEM -f rsa-pem
cp rsa rsa-pem-cifrada && ssh-keygen -q -p -N 'senha1' -m PEM -f rsa-pem-cifrada
cp ec ec-pem && ssh-keygen -q -p -N '' -m PEM -f ec-pem
cp rsa rsa-p8 && ssh-keygen -q -p -N '' -m PKCS8 -f rsa-p8
cp rsa rsa-p8-cifrada && ssh-keygen -q -p -N 'senha1' -m PKCS8 -f rsa-p8-cifrada
cp ec ec-p8 && ssh-keygen -q -p -N '' -m PKCS8 -f ec-p8

# PuTTY
puttygen ed -O private -o ed-v3.ppk --ppk-param version=3
puttygen ed -O private -o ed-v2.ppk --ppk-param version=2
puttygen rsa -O private -o rsa-v3-cifrada.ppk --new-passphrase <(echo senha1) --ppk-param version=3
puttygen rsa -O private -o rsa-v2-cifrada.ppk --new-passphrase <(echo senha1) --ppk-param version=2
puttygen ec -O private -o ec-v3.ppk --ppk-param version=3

# Públicas
ssh-keygen -e -m RFC4716 -f ed.pub > ed.rfc4716
ssh-keygen -e -m PKCS8 -f ec.pub > ec.spki.pem
ssh-keygen -e -m PEM -f rsa.pub > rsa.pkcs1.pem
ssh-keygen -e -m PKCS8 -f rsa.pub > rsa.spki.pem

# Certificado (a CA é a ed; assina a rsa)
ssh-keygen -q -s ed -I teste -n tirech -V +520w rsa.pub

# Impressões digitais (o que o `ssh-keygen -lf` mostra)
for f in ed rsa ec; do ssh-keygen -lf $f.pub | awk '{print $2}' > $f.fp; done
ssh-keygen -lf ed-cifrada.pub | awk '{print $2}' > ed-cifrada.fp
