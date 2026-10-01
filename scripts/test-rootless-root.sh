#!/bin/sh
# Run the root-assuming sandbox tests (repexec_sandbox_red) as uid 0 inside a
# rootless user+mount namespace with a tmpfs /root, so no host root is needed.
# Usage: scripts/test-rootless-root.sh [extra cargo test args]
set -eu
exec unshare -Urm sh -c '
  mount -t tmpfs tmpfs /root
  mkdir -p /root/.cache /root/.cargo /root/.npm /root/.local /root/go
  exec cargo test -p hs-loop --test repexec_sandbox_red "$@"
' sh "$@"
