# Brum — Official Ultra-Minimal Container Image (Alpine Linux 3.20)
FROM alpine:3.20

LABEL org.opencontainers.image.title="Brum" \
      org.opencontainers.image.description="Multi-Pane Web Environment (File Commander/Manager) - By Woofson" \
      org.opencontainers.image.vendor="Woofsons Lab" \
      org.opencontainers.image.url="https://www.arf.ac" \
      org.opencontainers.image.source="https://github.com/woofson/brum" \
      org.opencontainers.image.licenses="MIT"

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
    tzdata

WORKDIR /app

# Copy compiled musl release binary and master config.toml
COPY target/x86_64-unknown-linux-musl/release/brum /usr/local/bin/brum
COPY config.toml /etc/brum/config.toml

# Setup storage and runtime directories
RUN mkdir -p /data /mnt

EXPOSE 3140

ENV RUST_LOG=brum=info,tower_http=info

HEALTHCHECK --interval=30s --timeout=5s --start-period=5s --retries=3 \
  CMD curl -f http://localhost:3140/api/system/status || exit 1

ENTRYPOINT ["/usr/local/bin/brum"]

