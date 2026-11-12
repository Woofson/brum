#!/usr/bin/env bash
# ==============================================================================
# Brum - Scoop manifest update & bucket publishing
# Creator: Bolt J Woofson @ Woofsons Lab (www.arf.ac)
#
# 1. Waits until the release workflow has uploaded brum-v<version>-windows-x86_64.zip
# 2. Fills in version, download link and the ZIP's real SHA-256 in
#    packaging/windows/scoop/brum.json
# 3. With --publish: copies brum.json into the Woofson/scoop-bucket repository
#    (bucket/brum.json), commits and pushes it
#
# USAGE:
#   ./scripts/scoop.sh 1.21.0             # update the manifest only
#   ./scripts/scoop.sh 1.21.0 --publish   # update and push it to the bucket
#   SCOOP_WAIT_MINUTES=60 ./scripts/scoop.sh 1.21.0 --publish
# ==============================================================================

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "${SCRIPT_DIR}/.." && pwd)"
cd "${ROOT_DIR}"

REPO="Woofson/brum"
BUCKET="Woofson/scoop-bucket"
MANIFEST="packaging/windows/scoop/brum.json"

VERSION=""
PUBLISH=false
for arg in "$@"; do
    case "$arg" in
        --publish) PUBLISH=true ;;
        *) [[ "$arg" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] && VERSION="$arg" ;;
    esac
done
if [ -z "${VERSION}" ]; then
    echo "Usage: scripts/scoop.sh <version> [--publish]" >&2
    exit 1
fi

ASSET="brum-v${VERSION}-windows-x86_64.zip"
URL="https://github.com/${REPO}/releases/download/v${VERSION}/${ASSET}"
WAIT_MINUTES="${SCOOP_WAIT_MINUTES:-40}"

# ------------------------------------------------------------------------------
# 1. Wait for the ZIP on the GitHub release
# ------------------------------------------------------------------------------
echo "Waiting for ${ASSET} on release v${VERSION} (up to ${WAIT_MINUTES} min)..."
deadline=$(( $(date +%s) + WAIT_MINUTES * 60 ))
until gh release view "v${VERSION}" --repo "${REPO}" --json assets --jq '.assets[].name' 2>/dev/null | grep -qx "${ASSET}"; do
    if [ "$(date +%s)" -ge "${deadline}" ]; then
        echo "The ZIP did not appear in time. Run again later: scripts/scoop.sh ${VERSION} --publish" >&2
        exit 1
    fi
    sleep 30
done

# ------------------------------------------------------------------------------
# 2. Checksum of the file people actually download, then the manifest
# ------------------------------------------------------------------------------
TMP_DIR="$(mktemp -d)"
trap 'rm -rf "${TMP_DIR}"' EXIT
gh release download "v${VERSION}" --repo "${REPO}" -p "${ASSET}" -D "${TMP_DIR}" --clobber
SHA=$(sha256sum "${TMP_DIR}/${ASSET}" | awk '{print $1}')
echo "ZIP SHA-256: ${SHA}"

python3 -I - "${MANIFEST}" "${VERSION}" "${URL}" "${SHA}" <<'PY'
import json, sys
path, version, url, sha = sys.argv[1:5]
with open(path, encoding="utf-8") as f:
    m = json.load(f)
m["version"] = version
m["architecture"]["64bit"]["url"] = url
m["architecture"]["64bit"]["hash"] = sha
with open(path, "w", encoding="utf-8") as f:
    json.dump(m, f, indent=4, ensure_ascii=False)
    f.write("\n")
PY
echo "Manifest updated: ${MANIFEST}"

[ "${PUBLISH}" = true ] || exit 0

# ------------------------------------------------------------------------------
# 3. Push to the bucket repository
# ------------------------------------------------------------------------------
if ! gh repo view "${BUCKET}" >/dev/null 2>&1; then
    echo "The bucket repository ${BUCKET} does not exist yet; create it first." >&2
    exit 1
fi
gh repo clone "${BUCKET}" "${TMP_DIR}/bucket" -- --quiet
mkdir -p "${TMP_DIR}/bucket/bucket"
cp "${MANIFEST}" "${TMP_DIR}/bucket/bucket/brum.json"
cd "${TMP_DIR}/bucket"
git add bucket/brum.json
if git diff --cached --quiet; then
    echo "Bucket already has brum ${VERSION}."
    exit 0
fi
git commit --quiet -m "brum: update to version ${VERSION}"
git push --quiet origin HEAD
echo "Bucket ${BUCKET} now offers brum ${VERSION}."
