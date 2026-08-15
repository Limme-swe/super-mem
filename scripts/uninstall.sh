#!/bin/sh
# Remove the supermem binary. User data is preserved unless explicitly purged.
set -eu

INSTALL_DIR="${SUPER_MEM_INSTALL_DIR:-$HOME/.local/bin}"
PURGE_DATA=0
CONFIRMED=0
DRY_RUN=0

usage() {
    cat <<'EOF'
Usage: sh scripts/uninstall.sh [options]

Options:
  --install-dir DIR  Directory containing supermem (default: $HOME/.local/bin)
  --purge-data       Also delete the default super-mem data directory
  --yes              Required with --purge-data
  --dry-run          Print actions without deleting anything
  --help             Show this help

A custom database selected with --db or SUPER_MEM_DB is never deleted by this script.
EOF
}

fail() {
    printf 'super-mem uninstaller: %s\n' "$*" >&2
    exit 1
}

while [ "$#" -gt 0 ]; do
    case "$1" in
        --install-dir)
            [ "$#" -ge 2 ] || fail "--install-dir requires a value"
            INSTALL_DIR=$2
            shift 2
            ;;
        --purge-data) PURGE_DATA=1; shift ;;
        --yes) CONFIRMED=1; shift ;;
        --dry-run) DRY_RUN=1; shift ;;
        --help|-h) usage; exit 0 ;;
        *) fail "unknown option: $1" ;;
    esac
done

if [ "$PURGE_DATA" -eq 1 ] && [ "$CONFIRMED" -ne 1 ]; then
    fail "--purge-data requires --yes because memories cannot be recovered"
fi

case "$(uname -s 2>/dev/null || true)" in
    Darwin) DATA_DIR="$HOME/Library/Application Support/super-mem" ;;
    *) DATA_DIR="${XDG_DATA_HOME:-$HOME/.local/share}/super-mem" ;;
esac

remove_path() {
    path=$1
    label=$2
    if [ ! -e "$path" ] && [ ! -L "$path" ]; then
        printf '%s not found: %s\n' "$label" "$path"
        return
    fi
    if [ "$DRY_RUN" -eq 1 ]; then
        printf 'Would remove %s: %s\n' "$label" "$path"
    else
        rm -f -- "$path"
        printf 'Removed %s: %s\n' "$label" "$path"
    fi
}

BINARY="$INSTALL_DIR/supermem"
remove_path "$BINARY" "binary"

if [ "$PURGE_DATA" -eq 1 ]; then
    case "$DATA_DIR" in
        ''|'/'|"$HOME") fail "refusing unsafe data directory: $DATA_DIR" ;;
    esac
    if [ -d "$DATA_DIR" ] || [ -L "$DATA_DIR" ]; then
        if [ "$DRY_RUN" -eq 1 ]; then
            printf 'Would permanently remove data directory: %s\n' "$DATA_DIR"
        else
            rm -rf -- "$DATA_DIR"
            printf 'Permanently removed data directory: %s\n' "$DATA_DIR"
        fi
    else
        printf 'Data directory not found: %s\n' "$DATA_DIR"
    fi
else
    printf 'Memory data was preserved at: %s\n' "$DATA_DIR"
    if [ -n "${SUPER_MEM_DB:-}" ]; then
        printf 'Custom SUPER_MEM_DB was not removed: %s\n' "$SUPER_MEM_DB"
    fi
fi
