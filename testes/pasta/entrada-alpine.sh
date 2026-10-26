#!/bin/sh
if [ "$1" = dropbear ]; then
    exec dropbear -F -E -R -p 22
fi
exec /usr/sbin/sshd -D -e
