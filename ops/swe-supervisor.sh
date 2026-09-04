#!/bin/bash
# v6 sentinel for the single-task SWE run (adapted from v5).
RUN=/home/sandbox/swbench/single
LOG=/tmp/swe-supervisor.log
echo "$(date '+%F %T') v6 supervisor started" >> "$LOG"
upt() { cut -d' ' -f1 /proc/uptime; }
done_run() { [ -f "$RUN/result.json" ]; }
relaunch() {
  echo "$(date '+%F %T') relaunch - $1" >> "$LOG"
  pkill -9 -f hs-swe-run; pkill -9 -f hs-plugin; sleep 5
  /home/sandbox/.swe-launch.sh
}
reinit() { LASTR=$(date +%s); LASTUP=$(upt); LASTUP=${LASTUP%.*}; LASTSK=$((LASTR-LASTUP)); }
SIG_REASON=""
restore_sig() {
  local R UP SK DR DU DSK
  R=$(date +%s); UP=$(upt); UP=${UP%.*}; SK=$((R-UP))
  DR=$((R-LASTR)); DU=$((UP-LASTUP)); DSK=$((SK-LASTSK))
  LASTR=$R; LASTUP=$UP; LASTSK=$SK
  SIG_REASON=""
  if [ "$DU" -lt -5 ]; then SIG_REASON="restore-detected uptime-rollback=${DU}s"; return 0; fi
  if [ "$DR" -gt 90 ]; then SIG_REASON="restore-detected wall-jump=${DR}s"; return 0; fi
  if [ "$DSK" -gt 60 ]; then SIG_REASON="restore-detected skew-jump=${DSK}s"; return 0; fi
  return 1
}
if ! done_run && ! pgrep -f hs-swe-run >/dev/null; then relaunch "startup (not running)"; fi
reinit
while true; do
  sleep 30
  [ -f /tmp/swe-supervisor.stop ] && exit 0
  if done_run; then echo "$(date '+%F %T') result.json present, done" >> "$LOG"; exit 0; fi
  if restore_sig; then
    echo "$(date '+%F %T') restore: $SIG_REASON - grace, watching for in-run recovery" >> "$LOG"
    RESUMED_AT=$(date +%s)
    sleep 90
    MTIME=$(find "$RUN/log" -type f -printf '%T@\n' 2>/dev/null | sort -n | tail -1 | cut -d. -f1); MTIME=${MTIME:-0}
    if [ "$MTIME" -lt "$RESUMED_AT" ]; then
      sleep 240
      MTIME=$(find "$RUN/log" -type f -printf '%T@\n' 2>/dev/null | sort -n | tail -1 | cut -d. -f1); MTIME=${MTIME:-0}
      if [ "$MTIME" -lt "$RESUMED_AT" ]; then relaunch "no progress after restore"; sleep 30; reinit; continue; fi
    fi
    echo "$(date '+%F %T') survived restore (in-run recovery)" >> "$LOG"
    reinit; continue
  fi
  if ! pgrep -f hs-swe-run >/dev/null; then
    if done_run; then echo "$(date '+%F %T') done" >> "$LOG"; exit 0; fi
    relaunch "process died without result.json"; sleep 30; reinit; continue
  fi
  # stall: no stream-log write for >=1200s (900s watchdog + slack)
  NOW=$(date +%s)
  MTIME=$(find "$RUN/log" -type f -printf '%T@\n' 2>/dev/null | sort -n | tail -1 | cut -d. -f1)
  MTIME=${MTIME:-$NOW}
  AGE=$((NOW-MTIME))
  if [ "$AGE" -ge 1200 ]; then
    sleep 60
    [ -f /tmp/swe-supervisor.stop ] && exit 0
    if restore_sig; then relaunch "$SIG_REASON (post-grace)"; sleep 30; reinit; continue; fi
    NOW=$(date +%s)
    MTIME=$(find "$RUN/log" -type f -printf '%T@\n' 2>/dev/null | sort -n | tail -1 | cut -d. -f1)
    MTIME=${MTIME:-$NOW}; AGE=$((NOW-MTIME))
    if [ "$AGE" -ge 1200 ]; then relaunch "stall age=${AGE}s"; sleep 30; reinit; continue; fi
  fi
done
