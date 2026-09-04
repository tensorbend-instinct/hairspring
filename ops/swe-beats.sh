#!/bin/bash
RUN=/home/sandbox/swbench/smoke-force
DUMP=/home/sandbox/hairspring/target/debug/hs-log-cli
STATE=/tmp/beat.state
PARENT=agent-01M12JC19M9PNAR1KSETNPYEVM
echo "0 0" > "$STATE"
echo "$(date '+%F %T') beat watcher started" >> /tmp/swe-beats.log
while true; do
  OUT=$($DUMP dump --dir "$RUN/log" --payloads 2>/dev/null)
  MC=$(echo "$OUT" | grep -c 'ModelCall lat=')
  TC=$(echo "$OUT" | grep -c 'ToolCall lat=')
  IN=$(echo "$OUT" | grep -o '"input_tokens":[0-9]*' | cut -d: -f2 | paste -sd+ | bc)
  OUTT=$(echo "$OUT" | grep -o '"output_tokens":[0-9]*' | cut -d: -f2 | paste -sd+ | bc)
  IN=${IN:-0}; OUTT=${OUTT:-0}
  COST=$(python3 -c "print(f'{($IN*1400+$OUTT*4400)/1e6:.3f}')")
  LASTTOOL=$(echo "$OUT" | grep '"plugin"' | tail -1 | grep -o '"plugin":"[^"]*"' | cut -d'"' -f4)
  read PMC PTC < "$STATE"
  if [ -f "$RUN/result.json" ]; then
    VERDICT=$(python3 -c "import json;d=json.load(open('$RUN/result.json'));print(('PASS' if d.get('passed') else 'FAIL'),d.get('steps'),d.get('model_calls'),f\"\${d.get('cost_micros',0)/1e6:.3f}\")")
    tools agent_message send --to "$PARENT" --message "beat $(date '+%H:%M') | VERDICT: $VERDICT | result.json landed" >/dev/null 2>&1
    echo "$(date '+%F %T') verdict sent, exiting" >> /tmp/swe-beats.log
    exit 0
  fi
  if [ "$MC $TC" != "$PMC $PTC" ]; then
    tools agent_message send --to "$PARENT" --message "beat $(date '+%H:%M') | step $MC/25 | tools $TC${LASTTOOL:+ (last: $LASTTOOL)} | ~\$$COST" >/dev/null 2>&1
    echo "$MC $TC" > "$STATE"
  fi
  sleep 15
done
