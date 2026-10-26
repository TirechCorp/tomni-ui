FROM debian:bookworm-slim
RUN apt-get update \
 && DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends proftpd-basic proftpd-mod-crypto openssl ca-certificates \
 && rm -rf /var/lib/apt/lists/*
RUN openssl req -x509 -newkey rsa:2048 -nodes -days 3650 -subj "/CN=localhost" \
      -addext "subjectAltName=DNS:localhost,IP:127.0.0.1" \
      -keyout /etc/ssl/private/ftp.key -out /etc/ssl/certs/ftp.crt
RUN useradd --home-dir /srv/ftp --create-home --shell /bin/bash teste \
 && echo 'teste:teste' | chpasswd \
 && echo 'conteúdo de exemplo' > /srv/ftp/exemplo.txt \
 && mkdir -p /srv/ftp/pasta \
 && chown -R teste:teste /srv/ftp
COPY proftpd.conf /etc/proftpd/proftpd.conf
RUN mkdir -p /var/log/proftpd
CMD ["proftpd", "--nodaemon", "--config", "/etc/proftpd/proftpd.conf"]
