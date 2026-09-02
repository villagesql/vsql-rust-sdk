#!/usr/bin/env bash
#
# Print the server release that SDK_VERSION in villagesql-sys names, as
# `version=X.Y.Z`, ready for $GITHUB_OUTPUT. That constant is the single place
# recording which server the vendored headers came from, so the header check
# reads it rather than carrying its own copy.

set -euo pipefail

REPO_ROOT=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
SRC="$REPO_ROOT/villagesql-sys/src/lib.rs"

read -r major minor patch < <(
  awk '
    /pub const SDK_VERSION/ { in_const = 1 }
    in_const && /major:/ { gsub(/[^0-9]/, "", $0); major = $0 }
    in_const && /minor:/ { gsub(/[^0-9]/, "", $0); minor = $0 }
    in_const && /patch:/ { gsub(/[^0-9]/, "", $0); patch = $0; print major, minor, patch; exit }
  ' "$SRC"
)

if [ -z "${major:-}" ] || [ -z "${minor:-}" ] || [ -z "${patch:-}" ]; then
  echo "error: could not read SDK_VERSION from $SRC" >&2
  exit 1
fi

echo "version=$major.$minor.$patch"
