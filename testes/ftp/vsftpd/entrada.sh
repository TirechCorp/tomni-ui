#!/bin/sh
# Dois vsftpd (2121 explícito e 2990 implícito) lado a lado. O `init: true` do compose fica como
# PID 1: um vsftpd como PID 1 cai com SIGSEGV ao fechar a primeira sessão.
vsftpd /etc/vsftpd-explicito.conf &
vsftpd /etc/vsftpd-implicito.conf &
wait
