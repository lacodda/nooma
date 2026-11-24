#!/bin/sh
# nooma installer for macOS and Linux:
#   curl -fsSL https://raw.githubusercontent.com/lacodda/nooma/main/tools/install.sh | sh
set -eu

REPO="lacodda/nooma"

case "$(uname -s)-$(uname -m)" in
    Linux-x86_64) TARGET="x86_64-unknown-linux-gnu" ;;
    Darwin-arm64) TARGET="aarch64-apple-darwin" ;;
    # Git Bash, MSYS2 and Cygwin run this script happily on Windows, where it
    # has no business being: the Windows build is installed by install.ps1.
    MINGW*|MSYS*|CYGWIN*)
        echo "This is the macOS/Linux installer, but you are on Windows ($(uname -s))." >&2
        echo "Install with PowerShell instead:" >&2
        echo "  irm https://raw.githubusercontent.com/$REPO/main/tools/install.ps1 | iex" >&2
        exit 1
        ;;
    *)
        echo "No prebuilt binary for $(uname -s)/$(uname -m); build it from source:" >&2
        echo "  cargo install nooma --locked --git https://github.com/$REPO" >&2
        exit 1
        ;;
esac

# The tag comes from the /releases/latest redirect rather than the REST API:
# unauthenticated API calls are capped at 60 per hour per IP, and an installer
# that fails because someone else on the same address ran it is no installer.
TAG="${NOOMA_VERSION:-}"
if [ -z "$TAG" ]; then
    LOCATION=$(curl -fsSLI -o /dev/null -w '%{url_effective}' "https://github.com/$REPO/releases/latest" || true)
    TAG="${LOCATION##*/}"
fi
case "$TAG" in
    v[0-9]*) ;;
    *)
        echo "Cannot resolve the latest release of $REPO - set NOOMA_VERSION to a release tag (vX.Y.Z)" >&2
        exit 1
        ;;
esac

NAME="nooma-$TAG-$TARGET"
URL="https://github.com/$REPO/releases/download/$TAG/$NAME.tar.gz"
TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT

echo "Downloading $URL"
curl -fsSL "$URL" | tar xz -C "$TMP"

BIN_DIR="${NOOMA_INSTALL_DIR:-$HOME/.local/bin}"
mkdir -p "$BIN_DIR"
install -m 755 "$TMP/$NAME/nooma" "$BIN_DIR/nooma"
echo "Installed nooma $TAG to $BIN_DIR/nooma"

# NOOMA_NO_PATH: an install into a scratch directory - the release
# check that proves the installer works - has no PATH to talk about.
if [ -z "${NOOMA_NO_PATH:-}" ]; then
    case ":$PATH:" in
        *":$BIN_DIR:"*) ;;
        *) echo "Note: add $BIN_DIR to your PATH." ;;
    esac
fi
