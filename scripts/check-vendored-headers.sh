#!/usr/bin/env bash
#
# Compare the ABI headers vendored in include/ against the ones a real VillageSQL
# server build ships. The generated bindings are only as correct as these copies,
# and nothing else in CI notices when the server's headers move.
#
# One divergence is deliberate and allowed, in types.h: VEF_PARAM_VARARGS is spelled
# as a literal rather than UINT_MAX, because bindgen only captures a #define it can
# constant-fold and UINT_MAX folds differently on macOS and Linux. Any other
# difference fails the job.
#
# Usage: check-vendored-headers.sh <extracted-dev-server-dir>

set -euo pipefail

SERVER_ROOT=${1:?usage: check-vendored-headers.sh <extracted-dev-server-dir>}
REPO_ROOT=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
OURS="$REPO_ROOT/include/villagesql/abi"

HEADERS=(
  types.h
  preview/ping.h
  preview/sys_var.h
  preview/status_var.h
  preview/keyring.h
  preview/thread_worker.h
  preview/sql_query.h
)

# The server tarball's layout is not something this script should assume. Find the
# stable include tree by locating types.h, and prefer include/ over include-dev/.
CANDIDATES=()
while IFS= read -r line; do
  CANDIDATES+=("$line")
done < <(find "$SERVER_ROOT" -path '*/villagesql/abi/types.h' -print | sort)
if [ ${#CANDIDATES[@]} -eq 0 ]; then
  echo "error: no villagesql/abi/types.h found under $SERVER_ROOT" >&2
  echo "The server tarball layout changed, or the download step produced nothing." >&2
  find "$SERVER_ROOT" -maxdepth 3 -type d -print >&2
  exit 1
fi

# A server checkout carries several abi trees. stable_sdk/v3 is the one we vendor —
# it becomes the staged SDK's include/ — while sdk/ is the dev ABI and carries
# protocol 4 additions we do not ship. Prefer it explicitly rather than by sort order.
THEIRS=""
for pattern in '*/stable_sdk/v3/include/villagesql/abi' '*/include/villagesql/abi'; do
  for candidate in "${CANDIDATES[@]}"; do
    dir=$(dirname "$candidate")
    # shellcheck disable=SC2254
    case "$dir" in
      $pattern) THEIRS="$dir"; break 2 ;;
    esac
  done
done
# Fall back to the first match if there is no plain include/ tree.
THEIRS=${THEIRS:-$(dirname "${CANDIDATES[0]}")}

echo "Comparing $OURS against $THEIRS"

# Every changed line in types.h must be one of the allowed divergences: the two
# swapped lines, or a comment we added to explain them.
ALLOWED='^-#include <limits\.h>$|^\+// #include <limits\.h>$|^-#define VEF_PARAM_VARARGS UINT_MAX$|^\+#define VEF_PARAM_VARARGS 0xFFFFFFFFu$|^\+//'

status=0
for header in "${HEADERS[@]}"; do
  theirs="$THEIRS/$header"

  if [ ! -f "$theirs" ]; then
    echo "FAIL $header — the server no longer ships this header at $theirs"
    status=1
    continue
  fi

  if diff -q "$theirs" "$OURS/$header" >/dev/null; then
    echo "ok   $header"
    continue
  fi

  unexpected=$(diff -u "$theirs" "$OURS/$header" \
    | grep -E '^[+-]' \
    | grep -vE '^(\+\+\+|---)' \
    | grep -vE "$ALLOWED" || true)

  if [ -z "$unexpected" ]; then
    echo "ok   $header (known VEF_PARAM_VARARGS divergence only)"
    continue
  fi

  echo "FAIL $header — differs from the server beyond the known divergence:"
  echo "$unexpected"
  status=1
done

if [ "$status" -ne 0 ]; then
  cat >&2 <<'MSG'

The vendored headers no longer match the server.

To fix:
  1. Copy the changed headers from the server into include/villagesql/abi/.
  2. Re-apply the VEF_PARAM_VARARGS divergence in types.h (see the comment there).
  3. Regenerate the bindings:
       cargo build -p villagesql-sys --features villagesql-sys/regenerate-bindings
  4. Update SDK_VERSION in villagesql-sys/src/lib.rs to the server release you
     vendored from.
MSG
fi

exit "$status"
