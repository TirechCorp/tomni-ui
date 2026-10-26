# Os serviços do teste real do M48: um sshd (usuário `teste`, senha `teste`) e um eco na porta 7,
# que rodam na rede de um nó Nebula oficial (`network_mode: service:…`).
FROM alpine:3.22
RUN apk add --no-cache openssh-server socat \
    && adduser -D -s /bin/sh teste \
    && echo "teste:teste" | chpasswd \
    && ssh-keygen -A
CMD ["sh", "-c", "socat TCP-LISTEN:7,fork,reuseaddr EXEC:cat & exec /usr/sbin/sshd -D -e -o PasswordAuthentication=yes -o UsePAM=no"]
