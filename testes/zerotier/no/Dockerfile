# O nó do teste real do M61: o `zerotier-one` oficial (a versão vem de ZEROTIER_VERSAO) com um
# sshd (usuário `teste`, senha `teste`), um eco na porta 7 e o iptables para o gateway da rota.
# Nada do ZeroTier é alterado; a imagem só ganha as ferramentas de teste.
ARG ZEROTIER_VERSAO=1.14.2
FROM zerotier/zerotier:${ZEROTIER_VERSAO}
RUN apt-get update \
    && apt-get install -y --no-install-recommends openssh-server socat iptables netcat-openbsd iputils-ping \
    && rm -rf /var/lib/apt/lists/* \
    && useradd -m -s /bin/sh teste \
    && echo "teste:teste" | chpasswd \
    && mkdir -p /run/sshd \
    && ssh-keygen -A
COPY entrada.sh /entrada.sh
RUN chmod +x /entrada.sh
ENTRYPOINT ["/entrada.sh"]
