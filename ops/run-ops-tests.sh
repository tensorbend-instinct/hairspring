#!/bin/bash
# Runs the ops seam tests (relay + supervisor). Usage: ops/run-ops-tests.sh
set -euo pipefail
cd "$(dirname "$0")"
exec python3 -m pytest test_swe_relay.py test_swe_supervisor.py -q
