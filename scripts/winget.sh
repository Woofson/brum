#!/usr/bin/env bash
# ==============================================================================
# Brum - Windows Package Manager (winget) manifest update & submission
# Creator: Bolt J Woofson @ Woofsons Lab (www.arf.ac)
#
# 1. Waits until the release workflow has uploaded Brum-Setup-v<version>.exe
# 2. Fills in version, download link and the installer's real SHA-256 in
#    packaging/windows/winget/*.yaml
# 3. With --submit: sends the manifests to microsoft/winget-pkgs as a pull
#    request from your fork (made on first use), entirely through the GitHub
#    API, without cloning the very large winget-pkgs repository
#
# USAGE:
#   ./scripts/winget.sh 1.21.0            # update the manifests only
#   ./scripts/winget.sh 1.21.0 --submit   # update and open the pull request
#   WINGET_WAIT_MINUTES=60 ./scripts/winget.sh 1.21.0 --submit
# ==============================================================================

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "${SCRIPT_DIR}/.." && pwd)"
cd "${ROOT_DIR}"

REPO="Woofson/brum"
UPSTREAM="microsoft/winget-pkgs"
PKG_ID="Woofson.Brum"
MANIFEST_DIR="packaging/windows/winget"

VERSION=""
SUBMIT=false
for arg in "$@"; do
    case "$arg" in
        --submit) SUBMIT=true ;;
        *) [[ "$arg" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] && VERSION="$arg" ;;
    esac
done
if [ -z "${VERSION}" ]; then
    echo "Usage: scripts/winget.sh <version> [--submit]" >&2
    exit 1
fi

ASSET="Brum-Setup-v${VERSION}.exe"
URL="https://github.com/${REPO}/releases/download/v${VERSION}/${ASSET}"
WAIT_MINUTES="${WINGET_WAIT_MINUTES:-40}"

# ------------------------------------------------------------------------------
# 1. Wait for the installer on the GitHub release
# ------------------------------------------------------------------------------
echo "Waiting for ${ASSET} on release v${VERSION} (up to ${WAIT_MINUTES} min)..."
deadline=$(( $(date +%s) + WAIT_MINUTES * 60 ))
until gh release view "v${VERSION}" --repo "${REPO}" --json assets --jq '.assets[].name' 2>/dev/null | grep -qx "${ASSET}"; do
    if [ "$(date +%s)" -ge "${deadline}" ]; then
        echo "The installer did not appear in time. Run again later: scripts/winget.sh ${VERSION} --submit" >&2
        exit 1
    fi
    sleep 30
done

# ------------------------------------------------------------------------------
# 2. Checksum and manifests
# ------------------------------------------------------------------------------
TMP_DIR="$(mktemp -d)"
trap 'rm -rf "${TMP_DIR}"' EXIT
gh release download "v${VERSION}" --repo "${REPO}" -p "${ASSET}" -D "${TMP_DIR}" --clobber
SHA=$(sha256sum "${TMP_DIR}/${ASSET}" | awk '{print toupper($1)}')
echo "Installer SHA-256: ${SHA}"

for f in "${MANIFEST_DIR}"/*.yaml; do
    sed -i "s/^PackageVersion: .*/PackageVersion: ${VERSION}/" "$f"
done
sed -i \
    -e "s#^\(\s*InstallerUrl:\).*#\1 ${URL}#" \
    -e "s/^\(\s*InstallerSha256:\).*/\1 ${SHA}/" \
    "${MANIFEST_DIR}/${PKG_ID}.installer.yaml"
sed -i "s#^ReleaseNotesUrl: .*#ReleaseNotesUrl: https://github.com/${REPO}/releases/tag/v${VERSION}#" \
    "${MANIFEST_DIR}/${PKG_ID}.locale.en-US.yaml"
echo "Manifests updated in ${MANIFEST_DIR}/"

[ "${SUBMIT}" = true ] || exit 0

# ------------------------------------------------------------------------------
# 3. Pull request to microsoft/winget-pkgs
# ------------------------------------------------------------------------------
OWNER=$(gh api user --jq .login)
FORK="${OWNER}/winget-pkgs"
BRANCH="${PKG_ID}-${VERSION}"
DEST="manifests/w/Woofson/Brum/${VERSION}"

if [ -n "$(gh pr list --repo "${UPSTREAM}" --head "${BRANCH}" --author "${OWNER}" --state open --json number --jq '.[].number')" ]; then
    echo "A pull request for ${PKG_ID} ${VERSION} is already open."
    exit 0
fi

if ! gh repo view "${FORK}" >/dev/null 2>&1; then
    echo "Creating your fork of ${UPSTREAM}..."
    gh repo fork "${UPSTREAM}" --clone=false
    sleep 10
fi
gh repo sync "${FORK}" --branch master >/dev/null

BASE_SHA=$(gh api "repos/${FORK}/git/ref/heads/master" --jq .object.sha)
gh api -X POST "repos/${FORK}/git/refs" -f "ref=refs/heads/${BRANCH}" -f "sha=${BASE_SHA}" >/dev/null 2>&1 || true

for f in "${MANIFEST_DIR}"/*.yaml; do
    name=$(basename "$f")
    existing=$(gh api "repos/${FORK}/contents/${DEST}/${name}?ref=${BRANCH}" --jq .sha 2>/dev/null || true)
    args=(-X PUT "repos/${FORK}/contents/${DEST}/${name}"
          -f "message=${PKG_ID} ${VERSION}: ${name}"
          -f "content=$(base64 -w0 "$f")"
          -f "branch=${BRANCH}")
    [ -n "${existing}" ] && args+=(-f "sha=${existing}")
    gh api "${args[@]}" >/dev/null
done

if gh api "repos/${UPSTREAM}/contents/manifests/w/Woofson/Brum" >/dev/null 2>&1; then
    TITLE="New version: ${PKG_ID} version ${VERSION}"
else
    TITLE="New package: ${PKG_ID} version ${VERSION}"
fi

gh pr create --repo "${UPSTREAM}" --base master --head "${OWNER}:${BRANCH}" \
    --title "${TITLE}" \
    --body "Manifests for ${PKG_ID} ${VERSION} (release: https://github.com/${REPO}/releases/tag/v${VERSION})."
