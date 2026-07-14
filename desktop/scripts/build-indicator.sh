#!/bin/bash

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
DESKTOP_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
SOURCE="$DESKTOP_DIR/../prototype/mic_indicator.swift"
OUTPUT_DIR="$DESKTOP_DIR/src-tauri/binaries"
TARGET_TRIPLE="${TAURI_ENV_TARGET_TRIPLE:-}"
DEPLOYMENT_TARGET="${MACOSX_DEPLOYMENT_TARGET:-11.0}"

if [[ ! -f "$SOURCE" ]]; then
  echo "mic_indicator source not found: $SOURCE" >&2
  exit 1
fi

if [[ -z "$TARGET_TRIPLE" ]]; then
  TARGET_TRIPLE="$(rustc --print host-tuple 2>/dev/null || rustc -vV | awk '/^host:/ { print $2 }')"
fi

mkdir -p "$OUTPUT_DIR"
OUTPUT="$OUTPUT_DIR/mic_indicator-$TARGET_TRIPLE"
TEMP_DIR="$(mktemp -d "${TMPDIR:-/tmp}/claro-indicator.XXXXXX")"
trap 'rm -rf "$TEMP_DIR"' EXIT
export CLANG_MODULE_CACHE_PATH="${CLANG_MODULE_CACHE_PATH:-$TEMP_DIR/clang-module-cache}"
export SWIFT_MODULECACHE_PATH="${SWIFT_MODULECACHE_PATH:-$TEMP_DIR/swift-module-cache}"
mkdir -p "$CLANG_MODULE_CACHE_PATH" "$SWIFT_MODULECACHE_PATH"

compile_arch() {
  local swift_arch="$1"
  local output="$2"
  swiftc "$SOURCE" \
    -o "$output" \
    -O \
    -gnone \
    -target "${swift_arch}-apple-macosx${DEPLOYMENT_TARGET}" \
    -framework Cocoa \
    -framework AVFoundation
}

case "$TARGET_TRIPLE" in
  aarch64-apple-darwin)
    compile_arch arm64 "$OUTPUT"
    ;;
  x86_64-apple-darwin)
    compile_arch x86_64 "$OUTPUT"
    ;;
  universal-apple-darwin)
    compile_arch arm64 "$TEMP_DIR/mic_indicator-arm64"
    compile_arch x86_64 "$TEMP_DIR/mic_indicator-x86_64"
    lipo -create \
      "$TEMP_DIR/mic_indicator-arm64" \
      "$TEMP_DIR/mic_indicator-x86_64" \
      -output "$OUTPUT"
    ;;
  *)
    echo "mic_indicator only supports macOS targets; got '$TARGET_TRIPLE'" >&2
    exit 1
    ;;
esac

chmod 755 "$OUTPUT"
echo "Built $OUTPUT ($(lipo -archs "$OUTPUT")) for macOS $DEPLOYMENT_TARGET+"
