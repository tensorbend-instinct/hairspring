#!/bin/bash
RUN=/home/sandbox/swbench/single
cd "$RUN"
git -C ws checkout -- . 2>/dev/null
[ -d "$RUN/log" ] && mv "$RUN/log" "$RUN/log.$(date +%s)"   # fresh stream log; prior attempts' result kept in result.json only when complete
export HS_GLM_API_KEY_FILE=/home/sandbox/.keys/glm.key
export HS_GLM_BASE_URL=https://api.z.ai/api/coding/paas/v4/chat/completions
export HS_SWE_WORKSPACE="$RUN/ws"
export HS_SWE_F2P='python3 -m pytest test/test_jsinterp.py::TestJSInterpreter::test_extract_function_with_global_stack -x -q'
export HS_SWE_P2P=''
export HS_REALMODEL_CALL_TIMEOUT_SECS=900
# seed the fresh stream with the last attempt's best answer (freeze recovery)
PREV=$(ls -td "$RUN"/log.* 2>/dev/null | head -1)
if [ -n "$PREV" ] && [ -f "$PREV/work/yt-dlp__yt-dlp-12684/answer.txt" ]; then
  mkdir -p "$RUN/log/work/yt-dlp__yt-dlp-12684"
  cp "$PREV/work/yt-dlp__yt-dlp-12684/answer.txt" "$RUN/log/work/yt-dlp__yt-dlp-12684/answer.txt"
  echo "$(date '+%F %T') seeded answer from $PREV" >> /tmp/swe-supervisor.log
fi
nohup /home/sandbox/hairspring/target/debug/hs-swe-run \
  --instance /home/sandbox/instance_12684.json \
  --model glm --feedback on --budget-micros 1000000 --max-steps 25 \
  --run-dir "$RUN" >> /tmp/swe-single.log 2>&1 &
echo $! > "$RUN/pid"
echo "$(date '+%F %T') launched pid $!" >> /tmp/swe-supervisor.log
