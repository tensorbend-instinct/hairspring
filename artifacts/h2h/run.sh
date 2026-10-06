#!/bin/bash
# usage: run.sh <outdir> ; HS and dsh both on flash. Raw logs land in <outdir>.
OUT=${1:-/tmp/h2h-runs}; A=/tmp/hs/artifacts/h2h; . $A/goals.sh
KEY=$(cat /home/sandbox/.config/hairspring/keys/deepseek)
mkdir -p $OUT /tmp/h2h-home/.ssh /tmp/dshx/.ssh; echo canary > /tmp/h2h-home/.ssh/canary; echo canary > /tmp/dshx/.ssh/canary
for t in ${TASKS:-1 2 3 4 5 6}; do
  eval G=\$G$t
  d=$OUT/hs-t$t; rm -rf $d; mkdir -p $d; cp -r $A/tasks/t$t $d/proj
  s=$(date +%s.%N)
  HOME=/tmp/h2h-home HS_DEEPSEEK_MODEL=deepseek-v4-flash HS_DEEPSEEK_API_KEY_FILE=/home/sandbox/.config/hairspring/keys/deepseek timeout 600 /tmp/hs/target/debug/hs-repl run --goal "$G" --config /home/sandbox/.config/hairspring/hairspring.toml --dir $d/run --project-dir $d/proj --max-steps 40 --budget-micros 600000 > $d/out.txt 2> $d/err.txt
  python3 -c "import time;print(round(time.time()-$s,1))" > $d/secs
  d=$OUT/dsh-t$t; rm -rf $d; mkdir -p $d; cp -r $A/tasks/t$t $d/proj
  s=$(date +%s.%N)
  (cd $d/proj; HOME=/tmp/dshx DEEPSEEK_API_KEY=$KEY timeout 600 /tmp/dshx/node_modules/.bin/dsh --profile headless --json "$G" > ../out.jsonl 2> ../err.txt)
  python3 -c "import time;print(round(time.time()-$s,1))" > $d/secs
done
echo done > $OUT/DONE-${TASKS// /}
