#!/bin/bash
RUN=/home/sandbox/swbench/smoke-force
cd "$RUN"
git -C ws checkout -- . 2>/dev/null
[ -d "$RUN/log" ] && mv "$RUN/log" "$RUN/log.$(date +%s)"
export HS_GLM_API_KEY_FILE=${HS_GLM_API_KEY_FILE:-/home/sandbox/.keys/glm.key}
export HS_GLM_BASE_URL=http://127.0.0.1:8787/chat/completions
export HS_GLM_EXTRA_BODY_JSON='{"reasoning_effort":"low"}'
export HS_SWE_PROMPT_NUDGE='IMPORTANT: before every answer.submit, run the FAIL_TO_PASS command on your patch via repo.exec and fix whatever it reports.'
export HS_SWE_WORKSPACE="$RUN/ws"
export HS_SWE_F2P='python3 -m pytest test/test_jsinterp.py::TestJSInterpreter::test_extract_function_with_global_stack -x -q'
export HS_SWE_P2P=''
export HS_REALMODEL_CALL_TIMEOUT_SECS=1500
pgrep -f swe-relay.py >/dev/null || setsid env RELAY_PORT=8787 RELAY_TARGET_HOST=api.z.ai RELAY_TARGET_PREFIX=/api/coding/paas/v4 python3 /home/sandbox/.swe-relay.py </dev/null >/dev/null 2>&1 &
nohup /home/sandbox/hairspring/target/debug/hs-swe-run \
  --instance /home/sandbox/instance_12684.json \
  --model glm --feedback on --budget-micros 10000000 --max-steps 25 \
  --run-dir "$RUN" >> /tmp/swe-force.log 2>&1 &
echo $! > "$RUN/pid"
echo "$(date '+%F %T') launched pid $!" >> /tmp/swe-supervisor.log
