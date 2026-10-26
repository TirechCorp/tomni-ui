FROM debian:bookworm-slim
RUN apt-get update \
 && DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends pure-ftpd openssl ca-certificates \
 && rm -rf /var/lib/apt/lists/*
RUN mkdir -p /etc/ssl/private \
 && openssl req -x509 -newkey rsa:2048 -nodes -days 3650 -subj "/CN=localhost" \
      -addext "subjectAltName=DNS:localhost,IP:127.0.0.1" \
      -keyout /etc/ssl/private/pure-ftpd.pem -out /tmp/cert.pem \
 && cat /tmp/cert.pem >> /etc/ssl/private/pure-ftpd.pem && rm /tmp/cert.pem \
 && chmod 600 /etc/ssl/private/pure-ftpd.pem
RUN useradd --home-dir /srv/ftp --create-home --shell /bin/bash teste \
 && echo 'teste:teste' | chpasswd \
 && echo 'conteúdo de exemplo' > /srv/ftp/exemplo.txt \
 && mkdir -p /srv/ftp/pasta \
 && chown -R teste:teste /srv/ftp
COPY entrada.sh /entrada.sh
CMD ["/bin/sh", "/entrada.sh"]
