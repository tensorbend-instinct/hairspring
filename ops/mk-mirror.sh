#!/bin/bash
RUN=${SWE_RUN_DIR:-/home/sandbox/swbench/smoke-force}
DUMP=/home/sandbox/hairspring/target/debug/hs-log-cli
GLMPID=$(pgrep -f hs-plugin-glm | head -1)
{
echo "HAIRSPRING gate-8 live mission mirror"
echo "instance: yt-dlp__yt-dlp-12684 | model: glm-5.3 | feedback: ON | cap: \$10 | updated: $(date '+%F %T %Z')"
echo "======================================================================"
if [ -f "$RUN/result.json" ]; then echo "=== RESULT ==="; cat "$RUN/result.json"; echo; fi
echo "=== now ==="
if [ -n "$GLMPID" ]; then
  echo "model call in flight, elapsed: $(ps -o etime= -p $GLMPID | tr -d ' ')"
else
  echo "no model call in flight right now"
fi
pgrep -af 'hs-swe-run|hs-plugin' | cut -c1-100 || echo "(mission processes not running)"
echo
echo "=== current run events (payloads truncated) ==="
$DUMP dump --dir "$RUN/log" --payloads 2>&1 | tail -60
echo
echo "=== prior runs (each restart = box froze mid-mission) ==="
for d in $(ls -td "$RUN"/log.* 2>/dev/null); do
  echo "--- restart archived: $d ---"
  $DUMP dump --dir "$d" --payloads 2>&1 | tail -40
done
echo
echo "=== runner stdout ==="
tail -20 /tmp/swe-single.log 2>/dev/null || echo "(empty)"
echo
echo "=== sentinel supervisor (freeze detector) ==="
tail -10 /tmp/swe-supervisor.log 2>/dev/null
} > /tmp/mirror.txt 2>&1
