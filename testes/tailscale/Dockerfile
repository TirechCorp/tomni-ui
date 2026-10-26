# As máquinas do teste real do M57 (Tailscale): o `tailscale` oficial (userspace, sem TUN) com um
# sshd (usuário `teste`, senha `teste`) e um eco TCP na porta 7. A mesma imagem, sem o
# `tailscaled`, serve de `alvo` na rede interna. Só para teste.
ARG TAILSCALE_VERSAO=latest
FROM tailscale/tailscale:${TAILSCALE_VERSAO}
RUN apk add --no-cache openssh socat netcat-openbsd python3 \
    && adduser -D -s /bin/sh teste \
    && echo "teste:teste" | chpasswd \
    && ssh-keygen -A \
    && sed -i 's/^#\?PasswordAuthentication.*/PasswordAuthentication yes/; s/^#\?UsePAM.*/UsePAM no/' /etc/ssh/sshd_config
COPY entrada.sh /entrada.sh
RUN chmod +x /entrada.sh
ENTRYPOINT ["/entrada.sh"]
