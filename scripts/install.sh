#!/bin/sh
# Install a verified super-mem release for Linux or macOS without root access.
set -eu

REPOSITORY="Limme-swe/super-mem"
DOWNLOAD_BASE="${SUPER_MEM_DOWNLOAD_BASE:-https://github.com/$REPOSITORY/releases/download}"
API_URL="${SUPER_MEM_RELEASE_API:-https://api.github.com/repos/$REPOSITORY/releases/latest}"
INSTALL_DIR="${SUPER_MEM_INSTALL_DIR:-$HOME/.local/bin}"
VERSION="${SUPER_MEM_VERSION:-latest}"
TARGET="${SUPER_MEM_TARGET:-}"

usage() {
    cat <<'EOF'
Usage: sh scripts/install.sh [options]

Options:
  --version VERSION       Install a release version such as 0.1.0 (default: latest)
  --install-dir DIR       Install directory (default: $HOME/.local/bin)
  --target TARGET         Override release target detection
  --help                  Show this help

Environment overrides:
  SUPER_MEM_VERSION, SUPER_MEM_INSTALL_DIR, SUPER_MEM_TARGET,
  SUPER_MEM_DOWNLOAD_BASE, SUPER_MEM_RELEASE_API

The archive is accepted only after its entry in SHA256SUMS verifies.
EOF
}

fail() {
    printf 'super-mem installer: %s\n' "$*" >&2
    exit 1
}

need() {
    command -v "$1" >/dev/null 2>&1 || fail "required command not found: $1"
}

while [ "$#" -gt 0 ]; do
    case "$1" in
        --version)
            [ "$#" -ge 2 ] || fail "--version requires a value"
            VERSION=$2
            shift 2
            ;;
        --install-dir)
            [ "$#" -ge 2 ] || fail "--install-dir requires a value"
            INSTALL_DIR=$2
            shift 2
            ;;
        --target)
            [ "$#" -ge 2 ] || fail "--target requires a value"
            TARGET=$2
            shift 2
            ;;
        --help|-h)
            usage
            exit 0
            ;;
        *)
            fail "unknown option: $1"
            ;;
    esac
done

need curl
need tar

if [ -z "$TARGET" ]; then
    os=$(uname -s 2>/dev/null || true)
    arch=$(uname -m 2>/dev/null || true)
    case "$os:$arch" in
        Linux:x86_64|Linux:amd64) TARGET="x86_64-unknown-linux-musl" ;;
        Darwin:arm64|Darwin:aarch64) TARGET="aarch64-apple-darwin" ;;
        Darwin:x86_64|Darwin:amd64) TARGET="x86_64-apple-darwin" ;;
        *) fail "unsupported platform $os/$arch; available releases are Linux x86-64 and macOS x86-64/Apple Silicon" ;;
    esac
fi

case "$TARGET" in
    x86_64-unknown-linux-musl|aarch64-apple-darwin|x86_64-apple-darwin) ;;
    *) fail "unsupported release target: $TARGET" ;;
esac

if [ "$VERSION" = "latest" ]; then
    metadata=$(curl -fsSL --connect-timeout 10 --max-time 30 \
        -H 'Accept: application/vnd.github+json' \
        -H 'User-Agent: super-mem-installer' \
        "$API_URL") || fail "could not resolve the latest GitHub release"
    tag=$(printf '%s\n' "$metadata" | sed -n 's/.*"tag_name"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' | head -n 1)
    [ -n "$tag" ] || fail "latest release metadata did not contain tag_name"
    VERSION=${tag#v}
else
    case "$VERSION" in
        v*) tag=$VERSION; VERSION=${VERSION#v} ;;
        *) tag="v$VERSION" ;;
    esac
fi

case "$VERSION" in
    ''|*[!0-9A-Za-z._+-]*) fail "unsafe release version: $VERSION" ;;
esac

archive="super-mem-v$VERSION-$TARGET.tar.gz"
root="super-mem-v$VERSION-$TARGET"
tmp=${TMPDIR:-/tmp}/super-mem-install.$$
cleanup() { rm -rf "$tmp"; }
trap cleanup EXIT HUP INT TERM
mkdir -m 700 "$tmp" || fail "could not create temporary directory"

printf 'Downloading super-mem %s for %s...\n' "$VERSION" "$TARGET"
curl -fL --retry 3 --connect-timeout 10 --max-time 300 \
    -H 'Accept: application/octet-stream' \
    -H 'User-Agent: super-mem-installer' \
    -o "$tmp/$archive" "$DOWNLOAD_BASE/$tag/$archive" \
    || fail "could not download $archive"
curl -fL --retry 3 --connect-timeout 10 --max-time 60 \
    -H 'Accept: application/octet-stream' \
    -H 'User-Agent: super-mem-installer' \
    -o "$tmp/SHA256SUMS" "$DOWNLOAD_BASE/$tag/SHA256SUMS" \
    || fail "could not download SHA256SUMS"

checksum_line=$(awk -v name="$archive" 'length($1) == 64 && $2 == name { print; exit }' "$tmp/SHA256SUMS")
[ -n "$checksum_line" ] || fail "SHA256SUMS has no entry for $archive"
printf '%s\n' "$checksum_line" > "$tmp/CHECKSUM"
if command -v sha256sum >/dev/null 2>&1; then
    (cd "$tmp" && sha256sum --check CHECKSUM >/dev/null) || fail "checksum verification failed"
elif command -v shasum >/dev/null 2>&1; then
    (cd "$tmp" && shasum -a 256 --check CHECKSUM >/dev/null) || fail "checksum verification failed"
else
    fail "sha256sum or shasum is required to verify the release"
fi

mkdir -p "$tmp/extract"
tar -xzf "$tmp/$archive" -C "$tmp/extract" || fail "could not extract $archive"
source_binary="$tmp/extract/$root/supermem"
[ -f "$source_binary" ] || fail "archive did not contain $root/supermem"
mkdir -p "$INSTALL_DIR" || fail "could not create install directory: $INSTALL_DIR"
[ -d "$INSTALL_DIR" ] || fail "install path is not a directory: $INSTALL_DIR"
[ -w "$INSTALL_DIR" ] || fail "install directory is not writable: $INSTALL_DIR"

staged="$INSTALL_DIR/.supermem.new.$$"
cp "$source_binary" "$staged" || fail "could not stage the binary"
chmod 755 "$staged" || fail "could not mark the binary executable"
mv -f "$staged" "$INSTALL_DIR/supermem" || fail "could not install the binary"

installed=$($INSTALL_DIR/supermem --version 2>&1) || fail "installed binary did not start"
case "$installed" in
    *"$VERSION"*) ;;
    *) fail "installed binary reported an unexpected version: $installed" ;;
esac

printf 'Installed %s at %s/supermem\n' "$installed" "$INSTALL_DIR"
case ":${PATH:-}:" in
    *":$INSTALL_DIR:"*) ;;
    *) printf 'Add %s to PATH, then run: supermem init\n' "$INSTALL_DIR" ;;
esac
