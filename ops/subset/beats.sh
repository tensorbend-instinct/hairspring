#!/bin/bash
# subset beats -> parent on state change. Counts from disk truth
# (runs/*/result.json), current task from the live runner process.
S50=${S50:-/home/sandbox/swbench/subset50}
STATE=/tmp/beat50.state
PARENT=agent-01M12JC19M9PNAR1KSETNPYEVM
echo "init" > "$STATE"
echo "$(date '+%F %T') subset beat watcher started" >> /tmp/swe-beats.log
while true; do
  LINE=$(python3 - <<'PY'
import json, os, subprocess, glob
res = glob.glob('/home/sandbox/swbench/subset50/runs/*/result.json')
done = len(res); passed = 0; micros = 0
for p in res:
    try:
        d = json.load(open(p))
        passed += bool(d.get('passed')); micros += d.get('cost_micros', 0)
    except Exception: pass
# superseded prior-cap spend stays on the books
try:
    for line in open('/home/sandbox/swbench/subset50/ledger.csv'):
        parts = line.strip().split(',')
        if len(parts) >= 7 and parts[6].startswith('superseded'):
            micros += int(parts[4])
except Exception: pass
cur = ''
try:
    out = subprocess.run(['pgrep','-af','hs-swe-run'], capture_output=True, text=True).stdout
    for tok in out.split():
        if '/subset50/runs/' in tok:
            cur = tok.split('/runs/')[1]; break
except Exception: pass
mc = ''
log = f'/home/sandbox/swbench/subset50/runs/{cur}/log'
if cur and os.path.isdir(log):
    try:
        out = subprocess.run(['/home/sandbox/hairspring/target/debug/hs-log-cli','dump','--dir',log],
                             capture_output=True, text=True, timeout=10).stdout
        mc = f"step {out.count('ModelCall lat=')}"
    except Exception: pass
stop = 'STOPPED' if os.path.exists('/home/sandbox/swbench/subset50/.subset_complete') else ''
print(f"{done}|50|{passed}|{done-passed}|{round(micros/1e6,3)}|{cur}|{mc}|{stop}")
PY
)
  IFS='|' read -r DONE OF PASS FAIL SPEND CUR MC STOP <<< "$LINE"
  SIG="$DONE $MC $CUR"
  read -r PSIG < "$STATE"
  if [ "$SIG" != "$PSIG" ]; then
    tools agent_message send --to "$PARENT" --message "subset beat $(date '+%H:%M') | ${DONE}/${OF} done (${PASS}P/${FAIL}F) | spend \$${SPEND} | now: ${CUR} ${MC}" >/dev/null 2>&1
    echo "$SIG" > "$STATE"
  fi
  if [ -n "$STOP" ]; then
    tools agent_message send --to "$PARENT" --message "subset beat $(date '+%H:%M') | SUBSET COMPLETE: ${DONE}/${OF} (${PASS}P/${FAIL}F) spend \$${SPEND}" >/dev/null 2>&1
    echo "$(date '+%F %T') complete, exiting" >> /tmp/swe-beats.log
    exit 0
  fi
  sleep 30
done
