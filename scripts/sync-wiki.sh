#!/usr/bin/env bash
# ==============================================================================
# Brum - GitHub Wiki Synchronization Script
# Synchronizes manuals/, README.md, and INSTALL.md with GitHub Wiki
#
# USAGE:
#   ./scripts/sync-wiki.sh               # Syncs current Cargo.toml version
#   ./scripts/sync-wiki.sh 1.6.1         # Syncs specific version
# ==============================================================================

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "${SCRIPT_DIR}/.." && pwd)"
cd "${ROOT_DIR}"

VERSION="${1:-$(grep -m1 '^version = ' Cargo.toml | cut -d '"' -f2)}"
WIKI_REPO="${WIKI_REPO:-git@github.com:Woofson/brum.wiki.git}"
TEMP_WIKI_DIR="/tmp/brum-wiki-sync-$$"

echo "======================================================"
echo "📖 Brum GitHub Wiki Synchronization: v${VERSION}"
echo "======================================================"

echo "📥 Cloning GitHub Wiki repository (${WIKI_REPO})..."
rm -rf "${TEMP_WIKI_DIR}"
if ! git clone "${WIKI_REPO}" "${TEMP_WIKI_DIR}"; then
    echo "⚠️ Failed to clone wiki repository via SSH. Retrying with HTTPS..."
    git clone "https://github.com/Woofson/brum.wiki.git" "${TEMP_WIKI_DIR}"
fi

python3 - << PYEOF
import os
import re

root_dir = "${ROOT_DIR}"
wiki_dir = "${TEMP_WIKI_DIR}"
version = "${VERSION}"

mapping = {
    'manuals/android-termux.md': 'Android-and-Termux.md',
    'manuals/authentik-sso.md': 'Authentik-and-OIDC-SSO.md',
    'manuals/chewtoys.md': 'Chewtoys.md',
    'manuals/configuration.md': 'Configuration.md',
    'manuals/docker.md': 'Docker-Deployment.md',
    'manuals/lxc-proxmox.md': 'Proxmox-LXC-Setup.md',
    'manuals/plugin-development.md': 'Plugin-and-Chewtoy-Development.md',
    'manuals/protocols.md': 'Protocols-and-VFS.md',
    'manuals/qa-testing.md': 'QA-and-Testing.md',
    'manuals/reverse-proxy.md': 'Reverse-Proxy-and-TLS.md',
    'manuals/sharing.md': 'Advanced-Sharing-and-Client-Portals.md',
    'manuals/shortcuts.md': 'Keyboard-Shortcuts.md',
    'manuals/themes-and-palette.md': 'Themes-and-Palette.md',
    'manuals/vaults.md': 'Encrypted-Vaults.md',
    'manuals/windows.md': 'Windows-Desktop.md',
    'INSTALL.md': 'Installation-Guide.md',
    'README.md': 'Home.md',
}

link_map = {
    'manuals/chewtoys.md': 'Chewtoys',
    'chewtoys.md': 'Chewtoys',
    'manuals/plugin-development.md': 'Plugin-and-Chewtoy-Development',
    'plugin-development.md': 'Plugin-and-Chewtoy-Development',
    'manuals/shortcuts.md': 'Keyboard-Shortcuts',
    'shortcuts.md': 'Keyboard-Shortcuts',
    'manuals/configuration.md': 'Configuration',
    'configuration.md': 'Configuration',
    'manuals/protocols.md': 'Protocols-and-VFS',
    'protocols.md': 'Protocols-and-VFS',
    'manuals/vaults.md': 'Encrypted-Vaults',
    'vaults.md': 'Encrypted-Vaults',
    'manuals/sharing.md': 'Advanced-Sharing-and-Client-Portals',
    'sharing.md': 'Advanced-Sharing-and-Client-Portals',
    'manuals/authentik-sso.md': 'Authentik-and-OIDC-SSO',
    'authentik-sso.md': 'Authentik-and-OIDC-SSO',
    'manuals/windows.md': 'Windows-Desktop',
    'windows.md': 'Windows-Desktop',
    'manuals/android-termux.md': 'Android-and-Termux',
    'android-termux.md': 'Android-and-Termux',
    'manuals/docker.md': 'Docker-Deployment',
    'docker.md': 'Docker-Deployment',
    'manuals/lxc-proxmox.md': 'Proxmox-LXC-Setup',
    'lxc-proxmox.md': 'Proxmox-LXC-Setup',
    'manuals/reverse-proxy.md': 'Reverse-Proxy-and-TLS',
    'reverse-proxy.md': 'Reverse-Proxy-and-TLS',
    'manuals/themes-and-palette.md': 'Themes-and-Palette',
    'themes-and-palette.md': 'Themes-and-Palette',
    'manuals/qa-testing.md': 'QA-and-Testing',
    'qa-testing.md': 'QA-and-Testing',
    'manuals/README.md': 'Home',
    '../INSTALL.md': 'Installation-Guide',
    'INSTALL.md': 'Installation-Guide',
    '../CHANGELOG.md': 'https://github.com/Woofson/brum/blob/main/CHANGELOG.md',
    'CHANGELOG.md': 'https://github.com/Woofson/brum/blob/main/CHANGELOG.md',
    '../GEMINI.md': 'https://github.com/Woofson/brum/blob/main/GEMINI.md',
    'GEMINI.md': 'https://github.com/Woofson/brum/blob/main/GEMINI.md',
}

for src_rel, wiki_file in mapping.items():
    src_path = os.path.join(root_dir, src_rel)
    wiki_path = os.path.join(wiki_dir, wiki_file)
    if not os.path.exists(src_path):
        print(f"Warning: source file {src_path} does not exist, skipping.")
        continue

    content = open(src_path, 'r', encoding='utf-8').read()

    # 1. Replace relative links with wiki page names
    for old_link, new_link in link_map.items():
        content = re.sub(r'\]\(' + re.escape(old_link) + r'\)', f']({new_link})', content)
        content = re.sub(r'\]\(\./' + re.escape(old_link) + r'\)', f']({new_link})', content)

    # 2. Replace asset links with raw GitHub URLs for proper Wiki image rendering
    content = re.sub(r'<img src=\"assets/([^\"]+)\"', r'<img src=\"https://raw.githubusercontent.com/Woofson/brum/main/assets/\1\"', content)
    content = re.sub(r'!\[([^\]]*)\]\(assets/([^\)]+)\)', r'![\1](https://raw.githubusercontent.com/Woofson/brum/main/assets/\2)', content)

    # 3. Ensure version badge in Home.md reflects current version
    if wiki_file == 'Home.md':
        content = re.sub(r'version-v[0-9]+\.[0-9]+\.[0-9]+', f'version-v{version}', content)

    with open(wiki_path, 'w', encoding='utf-8') as f:
        f.write(content)

    print(f"  ✓ Processed {src_rel} -> {wiki_file}")

PYEOF

cd "${TEMP_WIKI_DIR}"
git add -A
if git diff-index --quiet HEAD --; then
    echo "✨ Wiki is already up-to-date. No changes to commit."
else
    echo "📤 Committing and pushing Wiki updates to GitHub..."
    git commit -m "docs(wiki): synchronize all manuals and guides with v${VERSION}"
    git push origin HEAD:master
    echo "✅ GitHub Wiki updated successfully!"
fi

cd "${ROOT_DIR}"
rm -rf "${TEMP_WIKI_DIR}"

echo "======================================================"
echo "🎉 Wiki synchronization complete: https://github.com/Woofson/brum/wiki"
echo "======================================================"
