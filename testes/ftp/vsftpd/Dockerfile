FROM debian:bookworm-slim
RUN apt-get update \
 && DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends vsftpd openssl ca-certificates \
 && rm -rf /var/lib/apt/lists/*
# O certificado autoassinado (o cliente de teste aceita pela pergunta da impressão).
RUN openssl req -x509 -newkey rsa:2048 -nodes -days 3650 -subj "/CN=localhost" \
      -addext "subjectAltName=DNS:localhost,IP:127.0.0.1" \
      -keyout /etc/ssl/private/ftp.pem -out /tmp/cert.pem \
 && cat /tmp/cert.pem >> /etc/ssl/private/ftp.pem && rm /tmp/cert.pem
RUN useradd --home-dir /srv/ftp --create-home --shell /bin/bash teste \
 && echo 'teste:teste' | chpasswd \
 && echo 'conteúdo de exemplo' > /srv/ftp/exemplo.txt \
 && mkdir -p /srv/ftp/pasta /var/run/vsftpd/empty \
 && chown -R teste:teste /srv/ftp
COPY vsftpd.conf /etc/vsftpd-explicito.conf
COPY vsftpd-implicito.conf /etc/vsftpd-implicito.conf
COPY entrada.sh /entrada.sh
CMD ["/bin/sh", "/entrada.sh"]
