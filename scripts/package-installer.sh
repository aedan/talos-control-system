#!/usr/bin/env bash
# Build a self-extracting TCS installer from a compiled binary.
#
# Usage:
#   ./scripts/package-installer.sh \
#     --binary backend/target/release/talos-control-system \
#     --version 0.1.0 \
#     --arch x86_64 \
#     --out dist/
#
# Produces: dist/tcs-${VERSION}-linux-${ARCH}.sh

set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
BINARY=""
VERSION="${TCS_VERSION:-0.1.0}"
COMMIT="${GIT_HASH:-$(git -C "$ROOT" rev-parse --short=12 HEAD 2>/dev/null || echo unknown)}"
ARCH="$(uname -m)"
OUT_DIR="${ROOT}/dist"
CONFIG_EXAMPLE="${ROOT}/config.example.toml"
INSTALLER_IN="${ROOT}/scripts/install.sh.in"

while [ $# -gt 0 ]; do
  case "$1" in
    --binary) BINARY="$2"; shift 2 ;;
    --version) VERSION="$2"; shift 2 ;;
    --commit) COMMIT="$2"; shift 2 ;;
    --arch) ARCH="$2"; shift 2 ;;
    --out) OUT_DIR="$2"; shift 2 ;;
    --config) CONFIG_EXAMPLE="$2"; shift 2 ;;
    -h|--help)
      sed -n '2,20p' "$0"
      exit 0
      ;;
    *) echo "Unknown arg: $1" >&2; exit 1 ;;
  esac
done

case "$ARCH" in
  x86_64|amd64) ARCH="x86_64" ;;
  aarch64|arm64) ARCH="aarch64" ;;
esac

if [ -z "$BINARY" ] || [ ! -f "$BINARY" ]; then
  echo "ERROR: --binary path required and must exist" >&2
  exit 1
fi
if [ ! -f "$INSTALLER_IN" ]; then
  echo "ERROR: missing $INSTALLER_IN" >&2
  exit 1
fi

mkdir -p "$OUT_DIR"
STAGE="$(mktemp -d)"
trap 'rm -rf "$STAGE"' EXIT

cp "$BINARY" "$STAGE/tcs"
chmod 755 "$STAGE/tcs"
if [ -f "$CONFIG_EXAMPLE" ]; then
  cp "$CONFIG_EXAMPLE" "$STAGE/config.example.toml"
fi

PAYLOAD="${STAGE}/payload.tar.gz"
tar czf "$PAYLOAD" -C "$STAGE" tcs $( [ -f "$STAGE/config.example.toml" ] && echo config.example.toml )

OUT_FILE="${OUT_DIR}/tcs-${VERSION}-linux-${ARCH}.sh"
{
  sed \
    -e "s/__VERSION__/${VERSION}/g" \
    -e "s/__COMMIT__/${COMMIT}/g" \
    "$INSTALLER_IN"
  # install.sh.in already ends with __ARCHIVE_BELOW__ line; append tarball after it
  cat "$PAYLOAD"
} > "$OUT_FILE"

# install.sh.in already contains the marker line; ensure we didn't double it.
# package: the .in file ends with marker; we cat payload after full file — good.

chmod +x "$OUT_FILE"
SIZE="$(wc -c < "$OUT_FILE" | tr -d ' ')"
echo "Wrote ${OUT_FILE} (${SIZE} bytes, version=${VERSION}, commit=${COMMIT})"
