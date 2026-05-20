#!/usr/bin/env bash
# Build a VillageSQL extension and package it as a .veb archive.
#
# Usage:
#   ./package.sh <extension>            # build + package to dist/
#   ./package.sh <extension> --install  # also copy to VillageSQL_BUILD_DIR
#
# The .veb layout expected by the server:
#   manifest.json
#   lib/<extension>.so
#
# On macOS, cargo produces .dylib; this script renames it to .so.

set -euo pipefail

if [[ $# -lt 1 ]]; then
    echo "Usage: $0 <extension> [--install]" >&2
    exit 1
fi

EXTENSION="$1"
INSTALL=false
for arg in "${@:2}"; do
    [[ "$arg" == "--install" ]] && INSTALL=true
done

WORKSPACE_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
EXTENSION_DIR="$WORKSPACE_DIR/examples/$EXTENSION"
DIST_DIR="$WORKSPACE_DIR/dist"

if [[ ! -d "$EXTENSION_DIR" ]]; then
    echo "Error: no extension directory found at $EXTENSION_DIR" >&2
    exit 1
fi

echo "Building $EXTENSION..."
cargo build -p "$EXTENSION" --release

# Locate the compiled library (macOS produces .dylib, Linux .so).
LIB_PATH=""
for candidate in \
    "$WORKSPACE_DIR/target/release/lib${EXTENSION}.so" \
    "$WORKSPACE_DIR/target/release/lib${EXTENSION}.dylib"; do
    if [[ -f "$candidate" ]]; then
        LIB_PATH="$candidate"
        break
    fi
done

if [[ -z "$LIB_PATH" ]]; then
    echo "Error: compiled library for '$EXTENSION' not found under target/release/" >&2
    exit 1
fi

# Stage VEB contents in a temp directory.
STAGING=$(mktemp -d)
trap 'rm -rf "$STAGING"' EXIT

mkdir -p "$STAGING/lib"
cp "$LIB_PATH" "$STAGING/lib/${EXTENSION}.so"
cp "$EXTENSION_DIR/manifest.json" "$STAGING/"

# Create the .veb archive.
mkdir -p "$DIST_DIR"
VEB="$DIST_DIR/${EXTENSION}.veb"
tar -C "$STAGING" -cf "$VEB" manifest.json lib/
echo "Created: $VEB"

# Optionally install into the VillageSQL build tree.
if [[ "$INSTALL" == true ]]; then
    if [[ -z "${VillageSQL_BUILD_DIR:-}" ]]; then
        echo "Error: VillageSQL_BUILD_DIR is not set" >&2
        exit 1
    fi
    INSTALL_DIR="$VillageSQL_BUILD_DIR/veb_output_directory"
    mkdir -p "$INSTALL_DIR"
    cp "$VEB" "$INSTALL_DIR/"
    echo "Installed to: $INSTALL_DIR/${EXTENSION}.veb"
fi
