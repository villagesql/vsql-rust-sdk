#!/usr/bin/env bash
#
# Compare the ABI headers vendored in include/ against the latest VillageSQL server
# source. The generated bindings are only as correct as these copies, and nothing
# else in CI notices when the server's headers move.
#
# Every header comes from the server's dev ABI (villagesql/sdk): it carries the
# protocol 4 bind hook and the current preview capability ABIs, which is what
# the nightly server these examples are tested against implements.
#
# One divergence is deliberate and allowed, in types.h: VEF_PARAM_VARARGS is spelled
# as a literal rather than UINT_MAX, because bindgen only captures a #define it can
# constant-fold and UINT_MAX folds differently on macOS and Linux. Any other
# difference fails the job.
#
# Usage: check-vendored-headers.sh <server-source-dir>

set -euo pipefail

SERVER_ROOT=${1:?usage: check-vendored-headers.sh <server-source-dir>}
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

THEIRS="$SERVER_ROOT/villagesql/sdk/include/villagesql/abi"
if [ ! -d "$THEIRS" ]; then
  echo "error: $THEIRS not found; the server layout changed or the checkout is empty" >&2
  exit 1
fi

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
  4. Every header comes from villagesql/sdk/include/villagesql/abi/.
MSG
fi

exit "$status"
