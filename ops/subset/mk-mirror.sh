#!/bin/bash
S50=${S50:-/home/sandbox/swbench/subset50}
DUMP=/home/sandbox/hairspring/target/debug/hs-log-cli
{
echo "HAIRSPRING 50-task subset live mirror (SWE-bench-Live lite, GLM-5.3 low effort, 50-step cap, \$10/task)"
echo "updated: $(date '+%F %T %Z')"
echo "======================================================================"
echo "=== ledger (per-task results) ==="
cat "$S50/ledger.csv" 2>/dev/null || echo "(not started)"
echo
CUR=$(pgrep -af hs-swe-run | grep -o 'subset50/runs/[^ ]*' | head -1 | sed 's|.*subset50/runs/||')
echo "=== current task: ${CUR:-none} ==="
if [ -n "$CUR" ] && [ -d "$S50/runs/$CUR/log" ]; then
  $DUMP dump --dir "$S50/runs/$CUR/log" --payloads 2>&1 | tail -40
fi
echo
echo "=== runner stdout (current task) ==="
tail -10 "$S50/runs/$CUR/stdout.log" 2>/dev/null || echo "(none)"
} > /tmp/mirror.txt 2>&1
