#!/usr/bin/env bash
# Rebuild the pinned environments the paired runner needs on a clean Linux host.
# usage: ci_setup.sh ENVROOT oh|grade <env-name>...   (env names like django38, sympy310)
set -euo pipefail
HERE=$(cd "$(dirname "$0")" && pwd)
ROOT=$1; MODE=$2; shift 2
mkdir -p "$ROOT"
pyver() { case "$1" in django50|sympy310) echo 3.10;; *) echo 3.8;; esac; }
if [ "$MODE" = oh ]; then
  uv python install 3.12.14
  PYDIR=$(uv python dir)/cpython-3.12.14-linux-x86_64-gnu
  test -x "$PYDIR/bin/python3.12"
  uv venv --python "$PYDIR/bin/python3.12" "$ROOT/openhands-env12"
  uv pip install --python "$ROOT/openhands-env12/bin/python" -r "$HERE/openhands-freeze.txt"
  HS_AB_OH_VENV="$ROOT/openhands-env12" python3 "$HERE/patch_openhands_telemetry.py"
  echo "HS_AB_PY312=$PYDIR" >> "${GITHUB_ENV:-/dev/null}"
  echo "HS_AB_OH_VENV=$ROOT/openhands-env12" >> "${GITHUB_ENV:-/dev/null}"
else
  for e in "$@"; do
    v=$(pyver "$e")
    uv python install "$v"
    uv venv --python "$v" "$ROOT/$e-env"
    uv pip install --python "$ROOT/$e-env/bin/python" -r "$HERE/$e-freeze.txt"
    "$ROOT/$e-env/bin/python" --version
  done
  echo "HS_AB_ENVROOT=$ROOT" >> "${GITHUB_ENV:-/dev/null}"
fi
