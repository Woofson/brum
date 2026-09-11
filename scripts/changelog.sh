#!/usr/bin/env bash
# ==============================================================================
# Brum Changelog & Release Notes Compiler
# Compiles and formats changelogs from CHANGELOG.md or Git commits to stdout.
# Creator: Bolt J Woofson @ Woofsons Lab (www.arf.ac)
# ==============================================================================
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "${SCRIPT_DIR}/.." && pwd)"
CHANGELOG_FILE="${ROOT_DIR}/CHANGELOG.md"

usage() {
    cat <<'USG'
Brum Changelog Compiler (work smart, not hard)

Usage:
  scripts/changelog.sh [command|version] [options]

Commands:
  latest, current      Print the latest release / candidate changelog entry (default)
  all                  Print the entire CHANGELOG.md file
  <version>            Print changelog entry for a specific version (e.g. 0.8.2, 0.8.3-rc7)
  -n <count>           Print the latest N version sections (default: 1)
  git [range]          Compile conventional changelog from Git history (e.g. git v0.8.2..HEAD)
  releases             List published GitHub releases via gh CLI
  help, -h, --help     Show this help message

Examples:
  scripts/changelog.sh                     # Output latest release notes
  scripts/changelog.sh -n 3                # Output last 3 releases
  scripts/changelog.sh 0.8.2               # Output 0.8.2 release notes
  scripts/changelog.sh git v0.8.2..HEAD    # Generate changelog from git commit range
USG
}

# ------------------------------------------------------------------------------
# 1. Print Latest / Specific Version from CHANGELOG.md
# ------------------------------------------------------------------------------
get_version_section() {
    local target_ver="${1:-latest}"
    local count="${2:-1}"

    if [ ! -f "${CHANGELOG_FILE}" ]; then
        echo "Error: ${CHANGELOG_FILE} not found." >&2
        exit 1
    fi

    if [ "${target_ver}" = "latest" ]; then
        awk -v n="${count}" '
            BEGIN { found = 0; }
            /^## \[/ {
                found++;
                if (found > n) exit;
            }
            found >= 1 { print; }
        ' "${CHANGELOG_FILE}"
    else
        local clean_ver
        clean_ver=$(echo "${target_ver}" | sed 's/^v//' | sed 's/[][]//g')
        awk -v ver="${clean_ver}" '
            BEGIN { printing = 0; }
            $0 ~ "^## \\[" ver "\\]" { printing = 1; print; next; }
            /^## \[/ { if (printing) exit; }
            printing { print; }
        ' "${CHANGELOG_FILE}"
    fi
}

# ------------------------------------------------------------------------------
# 2. Compile Changelog Directly from Git Commits
# ------------------------------------------------------------------------------
compile_git_changelog() {
    local git_range="${1:-}"
    if [ -z "${git_range}" ]; then
        local latest_tag
        latest_tag=$(git describe --tags --abbrev=0 2>/dev/null || echo "")
        if [ -n "${latest_tag}" ]; then
            git_range="${latest_tag}..HEAD"
        else
            git_range="HEAD~20..HEAD"
        fi
    fi

    echo "# 📦 Git Changelog: ${git_range}"
    echo ""

    local feats fixes styles docs chores refactors
    feats=$(git log "${git_range}" --pretty=format:"* %s ([%h](https://github.com/Woofson/brum/commit/%H))" --grep="^feat" || true)
    fixes=$(git log "${git_range}" --pretty=format:"* %s ([%h](https://github.com/Woofson/brum/commit/%H))" --grep="^fix" || true)
    styles=$(git log "${git_range}" --pretty=format:"* %s ([%h](https://github.com/Woofson/brum/commit/%H))" --grep="^style" || true)
    docs=$(git log "${git_range}" --pretty=format:"* %s ([%h](https://github.com/Woofson/brum/commit/%H))" --grep="^docs" || true)
    refactors=$(git log "${git_range}" --pretty=format:"* %s ([%h](https://github.com/Woofson/brum/commit/%H))" --grep="^refactor\|^perf" || true)
    chores=$(git log "${git_range}" --pretty=format:"* %s ([%h](https://github.com/Woofson/brum/commit/%H))" --grep="^chore\|^build\|^ci" || true)

    if [ -n "${feats}" ]; then
        echo "### ✨ Features & Enhancements"
        echo "${feats}"
        echo ""
    fi
    if [ -n "${fixes}" ]; then
        echo "### 🐛 Bug Fixes & Corrections"
        echo "${fixes}"
        echo ""
    fi
    if [ -n "${styles}" ]; then
        echo "### 🎨 UI & Design Polish"
        echo "${styles}"
        echo ""
    fi
    if [ -n "${refactors}" ]; then
        echo "### ⚡ Performance & Refactoring"
        echo "${refactors}"
        echo ""
    fi
    if [ -n "${docs}" ]; then
        echo "### 📚 Documentation & Guides"
        echo "${docs}"
        echo ""
    fi
    if [ -n "${chores}" ]; then
        echo "### 🔧 Build & Maintenance"
        echo "${chores}"
        echo ""
    fi
}

# ------------------------------------------------------------------------------
# 3. Main CLI Dispatch
# ------------------------------------------------------------------------------
cmd="${1:-latest}"

case "${cmd}" in
    latest|current|"")
        get_version_section "latest" 1
        ;;
    all)
        cat "${CHANGELOG_FILE}"
        ;;
    -n)
        count="${2:-1}"
        get_version_section "latest" "${count}"
        ;;
    git)
        shift || true
        compile_git_changelog "${1:-}"
        ;;
    releases)
        echo "=== GitHub Releases (Woofson/brum) ==="
        gh release list --limit 15
        ;;
    help|-h|--help)
        usage
        ;;
    *)
        if [[ "${cmd}" =~ ^[0-9] ]] || [[ "${cmd}" =~ ^v[0-9] ]]; then
            get_version_section "${cmd}" 1
        else
            echo "Unknown command: ${cmd}" >&2
            usage >&2
            exit 1
        fi
        ;;
esac
