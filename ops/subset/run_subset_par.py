#!/usr/bin/env python3
"""4-way parallel subset runner. Same contract as run_subset.py:
low effort, 50-step cap, $10/task cap, $50 guardrail, resume-safe ledger.
Parallel-safety: atomic mkdir claims, per-worker venvs (pip -e . rewrites
venv paths - a shared venv would point at the wrong ws), per-tarball
download locks, flocked ledger appends."""
import fcntl, json, os, shlex, shutil, subprocess, sys, threading, time, urllib.request

S50 = os.environ.get("S50", "/home/sandbox/swbench/subset50")
HS = os.environ.get("HS_SWE_RUN_BIN", "/home/sandbox/hairspring/target/debug/hs-swe-run")
TB_DIR = "/home/sandbox/swbench/tarballs"
VENV = "/home/sandbox/swbench/venvs"
LEDGER = os.path.join(S50, "ledger.csv")
STATUS = os.path.join(S50, "status.json")
CLAIMS = os.path.join(S50, "claims")
GUARDRAIL = 50_000_000
MAX_STEPS = os.environ.get("HS_SUBSET_MAX_STEPS", "50")
PAR = int(os.environ.get("PAR", "4"))
EXTRA_DEPS = {"haystack": "ddtrace opentelemetry-sdk",
              "streamlink": "freezegun requests-mock versioningit",
              "pdm": "pytest-mock"}
_lock = threading.Lock()

def sh(cmd, cwd=None, timeout=None, env=None):
    return subprocess.run(cmd, shell=True, cwd=cwd, timeout=timeout, env=env,
                          capture_output=True, text=True)

def spend_micros(manifest):
    total = 0
    if os.path.exists(LEDGER):
        for line in open(LEDGER):
            p = line.strip().split(",")
            if len(p) >= 7 and p[6].startswith("superseded"):
                total += int(p[4])
    for m in manifest:
        rj = os.path.join(S50, "runs", m["instance_id"], "result.json")
        if os.path.exists(rj):
            try: total += json.load(open(rj)).get("cost_micros", 0)
            except Exception: pass
    return total

def counts(manifest):
    done = passed = 0
    for m in manifest:
        rj = os.path.join(S50, "runs", m["instance_id"], "result.json")
        if os.path.exists(rj):
            done += 1
            try: passed += bool(json.load(open(rj)).get("passed"))
            except Exception: pass
    return done, passed

def write_status(manifest, note=""):
    done, passed = counts(manifest)
    json.dump({"done": done, "of": len(manifest), "passed": passed, "failed": done - passed,
               "spend_usd": round(spend_micros(manifest) / 1e6, 3),
               "note": note, "updated": time.strftime("%F %T")}, open(STATUS, "w"), indent=2)

def ledger_append(row):
    with _lock:
        with open(LEDGER, "a") as f:
            fcntl.flock(f, fcntl.LOCK_EX)
            f.write(row)
            fcntl.flock(f, fcntl.LOCK_UN)

def get_tarball(repo, commit):
    os.makedirs(TB_DIR, exist_ok=True)
    tb = os.path.join(TB_DIR, repo.replace("/", "_") + "_" + commit + ".tar.gz")
    if not os.path.exists(tb):
        lk = tb + ".lock"
        with open(lk, "w") as lf:
            fcntl.flock(lf, fcntl.LOCK_EX)
            if not os.path.exists(tb):
                tmp = tb + f".tmp{os.getpid()}{threading.get_ident()}"
                urllib.request.urlretrieve(f"https://github.com/{repo}/archive/{commit}.tar.gz", tmp)
                os.rename(tmp, tb)
    return tb

def prep_ws(m, run_dir):
    ws = os.path.join(run_dir, "ws")
    tb = get_tarball(m["repo"], m["base_commit"])
    os.makedirs(ws, exist_ok=True)
    sh(f"tar xzf {shlex.quote(tb)} -C {shlex.quote(ws)} --strip-components=1")
    sh("git init -q && git add -A && git -c user.email=b@b -c user.name=b commit -qm base && git tag v9.9.9", cwd=ws)
    tp = os.path.join(run_dir, "test_patch.diff")
    open(tp, "w").write(m["test_patch"])
    r = sh(f"git apply {shlex.quote(tp)} && git add -A && git -c user.email=b@b -c user.name=b commit -qm testpatch", cwd=ws)
    if r.returncode != 0:
        return None, f"test_patch apply failed: {r.stderr[-300:]}"
    return ws, None

def worker_venv(wid, slug):
    """Fresh per-worker venv per slug with pytest + repo extra deps.
    pip install -e . per task rewrites package paths, so venvs must be
    worker-private."""
    vd = os.path.join(VENV, f"w{wid}", slug)
    py = os.path.join(vd, "bin", "python")
    if not os.path.exists(py):
        lk = os.path.join(VENV, f"w{wid}-{slug}.lock")
        with open(lk, "w") as lf:
            fcntl.flock(lf, fcntl.LOCK_EX)
            if not os.path.exists(py):
                os.makedirs(os.path.dirname(vd), exist_ok=True)
                sh(f"python3 -m venv {shlex.quote(vd)}")
                deps = EXTRA_DEPS.get(slug, "")
                sh(f"{shlex.quote(py)} -m pip install -q --upgrade pip pytest {deps} 2>&1 | tail -2", timeout=900)
    return py

def f2p_nodes(m, ws, venv_python):
    nodes, frags = [], []
    for e in m["fail_to_pass"]:
        e = e.strip()
        if not e: continue
        (frags if ("[" in e and "]" not in e) else nodes).append(e)
    if frags:
        files = sorted(set(f.split("::")[0] for f in frags))
        r = sh(f"{shlex.quote(venv_python)} -m pytest {' '.join(files)} --co -q", cwd=ws, timeout=600)
        collected = [l.strip() for l in r.stdout.splitlines() if "::" in l]
        for f in frags:
            match = [c for c in collected if c.startswith(f)]
            nodes.extend(match if match else [f.split("::")[0]])
    return sorted(set(nodes))

def run_task(wid, m):
    iid = m["instance_id"]
    slug = m["repo"].split("/")[-1]
    run_dir = os.path.join(S50, "runs", iid)
    os.makedirs(run_dir, exist_ok=True)
    rj = os.path.join(run_dir, "result.json")
    t0 = time.time()
    note = ""
    ws, err = prep_ws(m, run_dir)
    if err:
        res = {"instance_id": iid, "passed": False, "steps": 0, "model_calls": 0,
               "cost_micros": 0, "wall_secs": int(time.time() - t0), "error": err}
        note = "prep_error"
    else:
        venv_py = worker_venv(wid, slug)
        sh(f"{shlex.quote(os.path.join(os.path.dirname(venv_py), 'pip'))} install -q -e . 2>&1 | tail -2", cwd=ws, timeout=900)
        for rf in ("conans/requirements_dev.txt", "requirements_dev.txt"):
            if os.path.exists(os.path.join(ws, rf)):
                sh(f"{shlex.quote(os.path.join(os.path.dirname(venv_py), 'pip'))} install -q -r {shlex.quote(rf)} 2>&1 | tail -2", cwd=ws, timeout=900)
                break
        nodes = " ".join(shlex.quote(n) for n in f2p_nodes(m, ws, venv_py))
        f2p_sh = os.path.join(run_dir, "f2p.sh")
        open(f2p_sh, "w").write(f"#!/bin/bash\nexec {shlex.quote(venv_py)} -m pytest {nodes} -x -q\n")
        env = dict(os.environ)
        env.update({
            "HS_GLM_API_KEY_FILE": "/home/sandbox/.keys/glm.key",
            "HS_GLM_BASE_URL": "http://127.0.0.1:8787/chat/completions",
            "HS_GLM_EXTRA_BODY_JSON": os.environ.get("HS_GLM_EXTRA_BODY_JSON", '{"reasoning_effort":"low"}'),
            "HS_SWE_PROMPT_NUDGE": "IMPORTANT: before every answer.write, run the FAIL_TO_PASS command on your patch via repo.exec and fix whatever it reports.",
            "HS_SWE_WORKSPACE": ws,
            "HS_SWE_F2P": f"bash {f2p_sh}",
            "HS_SWE_P2P": "",
            "HS_REALMODEL_CALL_TIMEOUT_SECS": "1500",
        })
        r = sh(f"timeout 3600 {HS} --instance {shlex.quote(os.path.join(S50, 'instances', iid + '.json'))} "
               f"--model glm --feedback on --budget-micros 10000000 --max-steps {MAX_STEPS} "
               f"--run-dir {shlex.quote(run_dir)}", timeout=3700, env=env)
        open(os.path.join(run_dir, "stdout.log"), "w").write(r.stdout + "\n--- STDERR ---\n" + r.stderr)
        if r.returncode == 124 and not os.path.exists(rj):
            prog = {}
            try:
                prog = json.load(open(os.path.join(run_dir, "progress.json")))
            except Exception:
                pass
            res = {"instance_id": iid, "passed": False,
                   "steps": prog.get("steps", 0), "model_calls": prog.get("model_calls", 0),
                   "cost_micros": prog.get("cost_micros", 0), "outcome": "wall_killed",
                   "wall_secs": 1800, "error": "wall_timeout"}
            note = "wall_timeout"
        elif os.path.exists(rj):
            res = json.load(open(rj))
        else:
            res = {"instance_id": iid, "passed": False, "steps": 0, "model_calls": 0,
                   "cost_micros": 0, "wall_secs": int(time.time() - t0),
                   "error": f"runner rc={r.returncode}: {r.stderr[-300:]}"}
            note = "runner_error"
    if not os.path.exists(rj):
        json.dump(res, open(rj, "w"), indent=2)
    res.setdefault("wall_secs", int(time.time() - t0))
    ledger_append(f"{iid},{res.get('passed')},{res.get('steps',0)},{res.get('model_calls',0)},{res.get('cost_micros',0)},{res.get('wall_secs',0)},{note}\n")
    with _lock:
        write_status(MANIFEST)
    print(f"[w{wid}] {iid} passed={res.get('passed')} steps={res.get('steps',0)} cost=${res.get('cost_micros',0)/1e6:.3f} {note}", flush=True)
    shutil.rmtree(os.path.join(CLAIMS, iid), ignore_errors=True)

def worker(wid):
    while True:
        claimed = None
        for m in MANIFEST:
            iid = m["instance_id"]
            if os.path.exists(os.path.join(S50, "runs", iid, "result.json")):
                continue
            try:
                os.makedirs(os.path.join(CLAIMS, iid))
                claimed = m
                break
            except FileExistsError:
                continue
        if claimed is None:
            return
        with _lock:
            if spend_micros(MANIFEST) > GUARDRAIL:
                write_status(MANIFEST, "STOPPED spend guardrail $50")
                print("SPEND GUARDRAIL TRIPPED", flush=True)
                os._exit(1)
        run_task(wid, claimed)

if __name__ == "__main__":
    MANIFEST = json.load(open(os.path.join(S50, "manifest.json")))
    os.makedirs(CLAIMS, exist_ok=True)
    ts = [threading.Thread(target=worker, args=(i,)) for i in range(PAR)]
    for t in ts: t.start()
    for t in ts: t.join()
    open(os.path.join(S50, ".subset_complete"), "w").write("done")
    write_status(MANIFEST, "complete")
    print("SUBSET_COMPLETE", flush=True)
