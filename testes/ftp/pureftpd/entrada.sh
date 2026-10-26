#!/bin/sh
# pure-ftpd 1.0.50: a 2123 aceita FTP com ou sem TLS (--tls=1) e a 2124 só com TLS (--tls=2).
# -l unix: usuários do sistema; -A: tudo preso na pasta do usuário; -c: no máximo 50 clientes.
/usr/sbin/pure-ftpd -S 127.0.0.1,2123 -p 30200:30249 --tls=1 -l unix -A -c 50 -C 50 -P 127.0.0.1 &
exec /usr/sbin/pure-ftpd -S 127.0.0.1,2124 -p 30250:30299 --tls=2 -l unix -A -c 50 -C 50 -P 127.0.0.1
