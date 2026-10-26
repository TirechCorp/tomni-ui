# O servidor WireGuard do teste real do M57: um Debian com `wireguard-tools` (o módulo `wireguard`
# vem do kernel do computador), um sshd (usuário `teste`, senha `teste`) e um eco TCP na porta 7.
FROM debian:bookworm-slim
RUN apt-get update \
    && DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends \
        wireguard-tools iproute2 openssh-server socat netcat-openbsd procps \
    && rm -rf /var/lib/apt/lists/* \
    && useradd -m -s /bin/sh teste \
    && echo "teste:teste" | chpasswd \
    && mkdir -p /run/sshd
COPY entrada.sh /entrada.sh
COPY subir-wg.sh /subir-wg.sh
RUN chmod +x /entrada.sh /subir-wg.sh
CMD ["/entrada.sh"]
