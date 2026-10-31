# Os serviços do teste real do M48: um sshd (usuário `teste`, senha `teste`) e um eco na porta 7,
# que rodam na rede de um nó Nebula oficial (`network_mode: service:…`). O `so-pelo-relay.sh`
# fecha o UDP do nó para tudo menos o relay (o teste dos relays).
FROM alpine:3.22
RUN apk add --no-cache openssh-server socat iptables \
    && adduser -D -s /bin/sh teste \
    && echo "teste:teste" | chpasswd \
    && ssh-keygen -A
COPY so-pelo-relay.sh /usr/local/bin/so-pelo-relay.sh
CMD ["sh", "-c", "socat TCP-LISTEN:7,fork,reuseaddr EXEC:cat & exec /usr/sbin/sshd -D -e -o PasswordAuthentication=yes -o UsePAM=no"]
