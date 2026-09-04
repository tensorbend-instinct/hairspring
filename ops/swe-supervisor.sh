#!/bin/bash
# v6 sentinel for the single-task SWE run (adapted from v5).
# env knobs (defaults = production single-task run; tests override all)
RUN=${SWE_RUN_DIR:-/home/sandbox/swbench/single}
LOG=${SWE_LOG:-/tmp/swe-supervisor.log}
STOP=${SWE_STOP_FILE:-"$STOP"}
LAUNCHER=${SWE_LAUNCHER:-/home/sandbox/.swe-launch.sh}
PROC=${SWE_PROC_PATTERN:-hs-swe-run}
LOOP_S=${SWE_LOOP_S:-30}
STALL_AGE=${SWE_STALL_AGE_S:-3300}
GRACE_S=${SWE_RESTORE_GRACE_S:-90}
WATCH_S=${SWE_RESTORE_WATCH_S:-240}
RECHECK_S=${SWE_STALL_RECHECK_S:-60}
echo "$(date '+%F %T') v6 supervisor started" >> "$LOG"
upt() { cut -d' ' -f1 /proc/uptime; }
done_run() { [ -f "$RUN/result.json" ]; }
relaunch() {
  echo "$(date '+%F %T') relaunch - $1" >> "$LOG"
  pkill -9 -f "$PROC"; [ "$PROC" = "hs-swe-run" ] && pkill -9 -f hs-plugin; sleep 5
  "$LAUNCHER"
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
if ! done_run && ! pgrep -f "$PROC" >/dev/null; then relaunch "startup (not running)"; fi
reinit
while true; do
  sleep "$LOOP_S"
  [ -f "$STOP" ] && exit 0
  if done_run; then echo "$(date '+%F %T') result.json present, done" >> "$LOG"; exit 0; fi
  if restore_sig; then
    echo "$(date '+%F %T') restore: $SIG_REASON - grace, watching for in-run recovery" >> "$LOG"
    RESUMED_AT=$(date +%s)
    sleep 90
    MTIME=$(find "$RUN/log" -type f -printf '%T@\n' 2>/dev/null | sort -n | tail -1 | cut -d. -f1); MTIME=${MTIME:-0}
    if [ "$MTIME" -lt "$RESUMED_AT" ]; then
      sleep "$WATCH_S"
      MTIME=$(find "$RUN/log" -type f -printf '%T@\n' 2>/dev/null | sort -n | tail -1 | cut -d. -f1); MTIME=${MTIME:-0}
      if [ "$MTIME" -lt "$RESUMED_AT" ]; then relaunch "no progress after restore"; sleep "$LOOP_S"; reinit; continue; fi
    fi
    echo "$(date '+%F %T') survived restore (in-run recovery)" >> "$LOG"
    reinit; continue
  fi
  if ! pgrep -f "$PROC" >/dev/null; then
    if done_run; then echo "$(date '+%F %T') done" >> "$LOG"; exit 0; fi
    relaunch "process died without result.json"; sleep "$LOOP_S"; reinit; continue
  fi
  # stall: no stream-log write for >=3300s (900s watchdog + slack)
  NOW=$(date +%s)
  MTIME=$(find "$RUN/log" -type f -printf '%T@\n' 2>/dev/null | sort -n | tail -1 | cut -d. -f1)
  MTIME=${MTIME:-$NOW}
  AGE=$((NOW-MTIME))
  if [ "$AGE" -ge "$STALL_AGE" ]; then
    sleep "$RECHECK_S"
    [ -f "$STOP" ] && exit 0
    if restore_sig; then relaunch "$SIG_REASON (post-grace)"; sleep "$LOOP_S"; reinit; continue; fi
    NOW=$(date +%s)
    MTIME=$(find "$RUN/log" -type f -printf '%T@\n' 2>/dev/null | sort -n | tail -1 | cut -d. -f1)
    MTIME=${MTIME:-$NOW}; AGE=$((NOW-MTIME))
    if [ "$AGE" -ge "$STALL_AGE" ]; then relaunch "stall age=${AGE}s"; sleep "$LOOP_S"; reinit; continue; fi
  fi
done
