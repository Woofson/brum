#!/bin/sh
# Starts Brum as an ordinary user (PUID/PGID, default 1000:1000) instead of root.
# Set BRUM_RUN_AS_ROOT=true only if you need system-account (PAM) logins or the
# terminal switching to each user's own account.
set -e

PUID="${PUID:-1000}"
PGID="${PGID:-1000}"

# Already started as a non-root user (e.g. `user:` in compose): nothing to switch
if [ "$(id -u)" != "0" ]; then
    exec /usr/local/bin/brum "$@"
fi

case "${BRUM_RUN_AS_ROOT:-false}" in
    true|1|yes)
        echo "brum: running as root because BRUM_RUN_AS_ROOT is set"
        exec /usr/local/bin/brum "$@"
        ;;
esac

case "$PUID$PGID" in
    *[!0-9]*) echo "brum: PUID and PGID must be numbers" >&2; exit 1 ;;
esac
if [ "$PUID" = "0" ]; then
    echo "brum: PUID=0 would run as root; use BRUM_RUN_AS_ROOT=true for that" >&2
    exit 1
fi

# Account for the chosen ids, reusing any that already exist in the image
group="$(getent group "$PGID" | cut -d: -f1)"
if [ -z "$group" ]; then
    group=brum
    addgroup -g "$PGID" "$group"
fi
user="$(getent passwd "$PUID" | cut -d: -f1)"
if [ -z "$user" ]; then
    user=brum
    adduser -D -H -h /data -s /bin/sh -G "$group" -u "$PUID" "$user"
fi

# One-time move from root: hand Brum's own data folder to the new user.
# Host folders mounted for browsing are never touched; give PUID/PGID access to them yourself.
if [ "$(stat -c %u /data)" != "$PUID" ] || { [ -e /data/brum.db ] && [ "$(stat -c %u /data/brum.db)" != "$PUID" ]; }; then
    echo "brum: giving /data to user $PUID:$PGID (one-time after upgrading from root)"
    chown -R "$PUID:$PGID" /data
fi
# The bundled settings and plugin folder; a read-only mounted config.toml is left as is
chown -R "$PUID:$PGID" /etc/brum 2>/dev/null || true

export HOME=/data
exec su-exec "$PUID:$PGID" /usr/local/bin/brum "$@"
