#!/usr/bin/env bash
# ==============================================================================
# CommanderDog GitHub Issues & Backlog Triage Helper (Token-Efficient Digest)
# Creator: Bolt J Woofson @ Woofsons Lab (www.arf.ac)
# ==============================================================================
set -euo pipefail

REPO="Woofson/commanderdog"

usage() {
    cat <<USG
CommanderDog Issues & Backlog Digest Helper

Usage:
  scripts/issues.sh [command] [args]

Commands:
  list              List all open issues grouped by category in compact format (default)
  view <id>         View single issue body compactly without verbose GraphQL warnings
  bugs              List open bug reports
  viewports         List open viewport & responsive issues (phone, tablet, desktop)
  chewtoys          List open ChewToy issues
  closed [n]        List recently closed issues (default: 10)
  help              Show this help message
USG
}

cmd="${1:-list}"

case "$cmd" in
    list)
        echo "=== Active Backlog: $REPO ==="
        gh issue list --repo "$REPO" --state open --limit 50 --json number,title,labels \
            --jq '.[] | "#\(.number)\t[\((.labels // []) | map(.name) | join(","))]\t\(.title)"' | \
            column -t -s $'\t'
        ;;
    view)
        if [ -z "${2:-}" ]; then
            echo "Error: Issue number required (e.g., scripts/issues.sh view 16)" >&2
            exit 1
        fi
        gh issue view "$2" --repo "$REPO" --json number,title,labels,body \
            --jq '"#\(.number): \(.title)\nLabels: \((.labels // []) | map(.name) | join(", "))\n\n--- Body ---\n\(.body)"'
        ;;
    bugs)
        echo "=== Open Bug Reports ==="
        gh issue list --repo "$REPO" --label "bug" --state open --limit 30 --json number,title \
            --jq '.[] | "#\(.number)\t\(.title)"' | column -t -s $'\t'
        ;;
    viewports)
        echo "=== Viewport & Responsive Backlog ==="
        gh issue list --repo "$REPO" --state open --limit 50 --json number,title,labels \
            --jq '.[] | select((.labels // []) | map(.name) | any(. == "phone" or . == "tablet" or . == "desktop")) | "#\(.number)\t[\((.labels // []) | map(.name) | join(","))]\t\(.title)"' | \
            column -t -s $'\t'
        ;;
    chewtoys)
        echo "=== ChewToy Backlog ==="
        gh issue list --repo "$REPO" --label "chewtoy" --state open --limit 30 --json number,title \
            --jq '.[] | "#\(.number)\t\(.title)"' | column -t -s $'\t'
        ;;
    closed)
        limit="${2:-10}"
        echo "=== Recently Closed Issues (Last $limit) ==="
        gh issue list --repo "$REPO" --state closed --limit "$limit" --json number,title,closedAt \
            --jq '.[] | "#\(.number)\t\(.closedAt[:10])\t\(.title)"' | column -t -s $'\t'
        ;;
    help|--help|-h)
        usage
        ;;
    *)
        echo "Unknown command: $cmd" >&2
        usage
        exit 1
        ;;
esac
