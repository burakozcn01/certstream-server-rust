#!/bin/sh
# Install certstream-server-rust from GitHub releases.
#
#   curl -fsSL https://raw.githubusercontent.com/reloading01/certstream-server-rust/main/install.sh | sh
#
# Environment:
#   VERSION   release to install, e.g. v1.5.5 (default: latest)
#   PREFIX    install prefix (default: /usr/local, so the binary lands in
#             /usr/local/bin); set PREFIX=$HOME/.local to avoid needing root
set -eu

REPO="reloading01/certstream-server-rust"
PREFIX="${PREFIX:-/usr/local}"
VERSION="${VERSION:-latest}"
BIN="certstream-server-rust"

die() {
    echo "install: $1" >&2
    exit 1
}

need() {
    command -v "$1" >/dev/null 2>&1 || die "$1 is required but not installed"
}

need uname
need tar
need mktemp

if command -v curl >/dev/null 2>&1; then
    fetch() { curl -fsSL "$1" -o "$2"; }
    fetch_stdout() { curl -fsSL "$1"; }
elif command -v wget >/dev/null 2>&1; then
    fetch() { wget -qO "$2" "$1"; }
    fetch_stdout() { wget -qO- "$1"; }
else
    die "curl or wget is required"
fi

os="$(uname -s)"
arch="$(uname -m)"

case "$os" in
    Linux) os_part="unknown-linux-musl" ;;
    Darwin) os_part="apple-darwin" ;;
    *) die "unsupported OS: $os (prebuilt binaries cover Linux and macOS; build from source with cargo)" ;;
esac

case "$arch" in
    x86_64 | amd64) arch_part="x86_64" ;;
    aarch64 | arm64) arch_part="aarch64" ;;
    *) die "unsupported architecture: $arch" ;;
esac

target="${arch_part}-${os_part}"

if [ "$VERSION" = "latest" ]; then
    # Resolve without jq: the tag_name line is the first one in the payload.
    VERSION="$(fetch_stdout "https://api.github.com/repos/${REPO}/releases/latest" \
        | sed -n 's/.*"tag_name"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' \
        | head -n 1)"
    [ -n "$VERSION" ] || die "could not determine the latest release; set VERSION explicitly"
fi

version_no_v="${VERSION#v}"
archive="${BIN}-${version_no_v}-${target}.tar.gz"
url="https://github.com/${REPO}/releases/download/${VERSION}/${archive}"

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

echo "certstream-server-rust ${VERSION} for ${target}"

fetch "$url" "${tmp}/${archive}" || die "download failed: ${url}"

# Checksum is published next to the archive. A release without one is a
# release someone edited by hand, so refuse rather than install it blind.
if fetch "${url}.sha256" "${tmp}/${archive}.sha256" 2>/dev/null; then
    expected="$(cut -d' ' -f1 < "${tmp}/${archive}.sha256")"
    if command -v sha256sum >/dev/null 2>&1; then
        actual="$(sha256sum "${tmp}/${archive}" | cut -d' ' -f1)"
    elif command -v shasum >/dev/null 2>&1; then
        actual="$(shasum -a 256 "${tmp}/${archive}" | cut -d' ' -f1)"
    else
        die "sha256sum or shasum is required to verify the download"
    fi
    [ "$expected" = "$actual" ] || die "checksum mismatch: expected ${expected}, got ${actual}"
    echo "checksum ok"
else
    die "no checksum published for ${archive}; refusing to install unverified"
fi

tar -xzf "${tmp}/${archive}" -C "$tmp"
extracted="${tmp}/${BIN}-${version_no_v}-${target}/${BIN}"
[ -f "$extracted" ] || die "archive did not contain ${BIN}"

dest_dir="${PREFIX}/bin"
if [ -w "$PREFIX" ] || [ -w "$dest_dir" ] 2>/dev/null; then
    install_cmd=""
elif command -v sudo >/dev/null 2>&1; then
    install_cmd="sudo"
    echo "${dest_dir} needs root; using sudo"
else
    die "${dest_dir} is not writable and sudo is unavailable; set PREFIX=\$HOME/.local"
fi

$install_cmd mkdir -p "$dest_dir"
$install_cmd install -m 755 "$extracted" "${dest_dir}/${BIN}"

echo "installed ${dest_dir}/${BIN}"

if ! command -v "$BIN" >/dev/null 2>&1; then
    echo "note: ${dest_dir} is not on your PATH"
fi

"${dest_dir}/${BIN}" --version || true

cat <<EOF

Run it:
  ${BIN}

Serves WebSocket on :8080. Configuration is optional; see
https://certstream.dev/docs.html for the environment variables and
https://github.com/${REPO}/blob/main/config.example.yaml for the YAML form.
EOF
