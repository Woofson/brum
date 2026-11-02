# Brum — Official Container Image (Alpine Linux 3.20)
FROM alpine:3.20

LABEL org.opencontainers.image.title="Brum" \
      org.opencontainers.image.description="Multi-Pane Web Environment (File Commander/Manager) - By Woofson" \
      org.opencontainers.image.vendor="Woofsons Lab" \
      org.opencontainers.image.url="https://www.arf.ac" \
      org.opencontainers.image.source="https://github.com/woofson/brum" \
      org.opencontainers.image.licenses="MIT"

# ffmpeg/imagemagick: Format Converter; git: Git tool; samba-client/nfs-utils: SMB and NFS
# browsing; rclone: Proton Drive; openssh-keygen/nano: SSH keys and editing in the terminal
RUN apk add --no-cache \
    ca-certificates \
    curl \
    bash \
    coreutils \
    procps \
    libssl3 \
    libcrypto3 \
    libssh2 \
    sqlite-libs \
    zlib \
    bzip2 \
    tar \
    gzip \
    7zip \
    openssh-client \
    openssh-keygen \
    nano \
    su-exec \
    tzdata \
    ffmpeg \
    imagemagick \
    imagemagick-jpeg \
    imagemagick-webp \
    imagemagick-tiff \
    imagemagick-heic \
    git \
    samba-client \
    nfs-utils \
    rclone

WORKDIR /app

# Copy compiled musl release binary and master config.toml
COPY target/x86_64-unknown-linux-musl/release/brum /usr/local/bin/brum
COPY config.toml /etc/brum/config.toml
COPY docker-entrypoint.sh /usr/local/bin/docker-entrypoint.sh

# Setup storage and runtime directories
RUN mkdir -p /data /data/admin /data/users /mnt /etc/brum/plugins \
    && chmod 755 /usr/local/bin/docker-entrypoint.sh

# Brum runs as user PUID:PGID (default 1000:1000); see docker-entrypoint.sh
ENV PUID=1000 PGID=1000

EXPOSE 3140

ENV RUST_LOG=brum=info,tower_http=info

HEALTHCHECK --interval=30s --timeout=5s --start-period=5s --retries=3 \
  CMD curl -f http://localhost:3140/api/system/status || exit 1

ENTRYPOINT ["/usr/local/bin/docker-entrypoint.sh"]

