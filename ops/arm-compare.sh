#!/bin/bash
# 3-way arm comparison for the SWE-bench effort A/B/C. Usage: ops/arm-compare.sh [dirs...]
# Defaults to the three standard arm run dirs. Prints one row per arm.
set -uo pipefail
HSLOG="$(dirname "$0")/../target/debug/hs-log-cli"
DIRS=("$@")
[ ${#DIRS[@]} -eq 0 ] && DIRS=(/home/sandbox/swbench/single /home/sandbox/swbench/arm-high /home/sandbox/swbench/arm-low)
printf '%-10s %-7s %-6s %-6s %-9s %-7s %s\n' arm passed steps calls cost_usd wall_s lat_med_s
for d in "${DIRS[@]}"; do
  name=$(basename "$d")
  r="$d/result.json"
  if [ ! -f "$r" ]; then printf '%-10s (running or not started)\n' "$name"; continue; fi
  python3 - "$r" "$d" "$HSLOG" <<'PY'
import json, statistics, subprocess, sys, re
r, d, hslog = sys.argv[1], sys.argv[2], sys.argv[3]
j = json.load(open(r))
try:
    out = subprocess.run([hslog, "dump", "--dir", d + "/log"],
                         capture_output=True, text=True, timeout=60).stdout
    lats = [int(m) / 1000 for m in re.findall(r"ModelCall lat=(\d+)ms", out)]
    med = f"{statistics.median(lats):.0f}" if lats else "-"
except Exception:
    med = "-"
print(f"{d.split('/')[-1]:<10} {str(j['passed']):<7} {j['steps']:<6} {j['model_calls']:<6} "
      f"{j['cost_micros']/1e6:<9.3f} {j['wall_secs']:<7} {med}")
PY
done
