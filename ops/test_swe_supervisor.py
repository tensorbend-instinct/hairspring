"""Seam tests for ops/swe-supervisor.sh: the stall-kill path.

Production bite: stall-kill mismatch - supervisor must detect a wedged run
(no stream-log writes past the stall threshold) and relaunch via the launcher.
Uses test-only env knobs so thresholds are seconds, not 3300s.
"""
import os, subprocess, time

REPO = os.path.dirname(os.path.abspath(__file__))
SUP = os.path.join(REPO, "swe-supervisor.sh")

def test_stall_triggers_relaunch(tmp_path):
    run = tmp_path / "run"; (run / "log").mkdir(parents=True)
    # backdate stream log so the run is already stalled
    stale = run / "log" / "stream.old"; stale.write_text("x")
    os.utime(stale, (time.time() - 100, time.time() - 100))
    marker = tmp_path / "launched"
    launcher = tmp_path / "launch.sh"
    launcher.write_text(f"#!/bin/bash\necho fired >> {marker}\n")
    launcher.chmod(0o755)
    # fake long-running worker whose name matches the supervisor's pgrep pattern
    worker = subprocess.Popen(["bash", "-c", "exec -a hs-swe-run-test sleep 300"])
    stop = tmp_path / "stop"
    env = dict(os.environ,
               SWE_RUN_DIR=str(run), SWE_LOG=str(tmp_path / "sup.log"),
               SWE_STOP_FILE=str(stop), SWE_LAUNCHER=str(launcher),
               SWE_PROC_PATTERN="hs-swe-run-test", SWE_LOOP_S="1",
               SWE_STALL_AGE_S="1", SWE_RESTORE_GRACE_S="1",
               SWE_RESTORE_WATCH_S="1", SWE_STALL_RECHECK_S="1")
    sup = subprocess.Popen(["bash", SUP], env=env)
    try:
        deadline = time.time() + 30
        while time.time() < deadline and not marker.exists():
            time.sleep(0.5)
        assert marker.exists(), "supervisor never relaunched a stalled run"
    finally:
        stop.touch(); time.sleep(2)
        sup.terminate(); worker.terminate()

def test_done_result_exits_cleanly(tmp_path):
    run = tmp_path / "run"; (run / "log").mkdir(parents=True)
    (run / "result.json").write_text("{}")
    stop = tmp_path / "stop"
    env = dict(os.environ,
               SWE_RUN_DIR=str(run), SWE_LOG=str(tmp_path / "sup.log"),
               SWE_STOP_FILE=str(stop), SWE_LAUNCHER="/bin/true",
               SWE_PROC_PATTERN="hs-swe-run-test", SWE_LOOP_S="1",
               SWE_STALL_AGE_S="1", SWE_RESTORE_GRACE_S="1",
               SWE_RESTORE_WATCH_S="1", SWE_STALL_RECHECK_S="1")
    r = subprocess.run(["bash", SUP], env=env, timeout=20)
    assert r.returncode == 0
