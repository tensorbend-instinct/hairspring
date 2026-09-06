#!/usr/bin/env python3
"""4-way parallel subset runner. Same contract as run_subset.py:
low effort, 50-step cap, $10/task cap, $50 guardrail, resume-safe ledger.
Parallel-safety: atomic mkdir claims, per-worker venvs (pip -e . rewrites
venv paths - a shared venv would point at the wrong ws), per-tarball
download locks, flocked ledger appends."""
import fcntl, json, os, shlex, shutil, subprocess, sys, threading, time, urllib.request

S50 = os.environ.get("S50", "/home/sandbox/swbench/subset50")
TASK_WALL_SECS = int(os.environ.get("HS_SUBSET_WALL_SECS", "7200"))  # Eric 2026-09-05: 2h wall is a guard
# Model under test (2026-09-05): env-selectable, config-first - the provider's
# own env vars (HS_<MODEL>_API_KEY_FILE, HS_<MODEL>_EXTRA_BODY_JSON, HS_<MODEL>_MODEL)
# carry model id and hyperparameters; nothing model-specific is hardcoded here.
MODEL = os.environ.get("HS_SUBSET_MODEL", "glm")
HS = os.environ.get("HS_SWE_RUN_BIN", "/home/sandbox/hairspring/target/debug/hs-swe-run")
# Bake-off layout (2026-09-06): tarballs shared at the bench root (download
# locks make concurrent arms safe); venvs per-S50 so two arms never share a
# worker venv (pip install -e rewrites paths per task).
_ROOT = os.path.dirname(S50.rstrip("/"))
TB_DIR = os.path.join(_ROOT, "tarballs")
VENV = os.path.join(S50, "venvs")
LEDGER = os.path.join(S50, "ledger.csv")
STATUS = os.path.join(S50, "status.json")
CLAIMS = os.path.join(S50, "claims")
GUARDRAIL = 50_000_000
MAX_STEPS = os.environ.get("HS_SUBSET_MAX_STEPS", "100000")  # Eric 2026-09-05: NO step/tool-call caps - budget + wall are the only guards
PAR = int(os.environ.get("PAR", "4"))
EXTRA_DEPS = {"haystack": "ddtrace opentelemetry-sdk flaky python-docx pypdf azure-ai-formrecognizer",
              "streamlink": "freezegun requests-mock versioningit setuptools",
              "pdm": "pytest-mock hishel<1"}  # gate-audit 2026-09-06: 10/10 gates collect+execute at base
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
    worker-private. ab2/17123 finding: a venv can exist but be broken
    (missing pytest, missing pip, missing ensurepip) - verify the import
    and repair loudly: pip route first, full recreate under the lock if
    the venv is beyond repair."""
    os.makedirs(VENV, exist_ok=True)
    vd = os.path.join(VENV, f"w{wid}", slug)
    py = os.path.join(vd, "bin", "python")
    deps = EXTRA_DEPS.get(slug, "")

    def healthy():
        return os.path.exists(py) and sh(
            f"{shlex.quote(py)} -m pytest --version >/dev/null 2>&1").returncode == 0

    def build():
        shutil.rmtree(vd, ignore_errors=True)
        os.makedirs(os.path.dirname(vd), exist_ok=True)
        sh(f"python3 -m venv {shlex.quote(vd)}")
        sh(f"{shlex.quote(py)} -m pip install -q --upgrade pip pytest {deps} 2>&1 | tail -2", timeout=900)

    if not healthy():
        lk = os.path.join(VENV, f"w{wid}-{slug}.lock")
        with open(lk, "w") as lf:
            fcntl.flock(lf, fcntl.LOCK_EX)
            if not healthy():
                if os.path.exists(py):
                    sh(f"{shlex.quote(py)} -m ensurepip -q --upgrade 2>&1 | tail -1", timeout=300)
                    sh(f"{shlex.quote(py)} -m pip install -q pytest {deps} 2>&1 | tail -2", timeout=900)
                if not healthy():
                    build()
                if not healthy():
                    raise RuntimeError(f"venv {vd} cannot be repaired or recreated with pytest")
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


IMPORT_PKG = {"cfn-lint": "cfnlint"}  # slug -> top-level import name (default: slug)

def preflight_gate(m, ws, venv_python, nodes):
    """Gate-8 2026-09-06 audit finding: a mission whose gate collects/executes
    zero tests is unpassable and indistinguishable from red in summaries
    (haystack-8619 burned 2h on a strict-markers abort). Refuse to start:
    every ::node must collect, zero collection ERRORs, >=1 test must execute,
    zero execute-time ERRORs (FAILED at base is expected - that IS the mission),
    and the venv's editable import target must be THIS task's ws (stale copied
    venvs silently grade another arm's code)."""
    import re as _re
    def fail(why):
        return f"preflight_gate_invalid: {why}"
    pip = os.path.join(os.path.dirname(venv_python), "pip")
    r = sh(f"{shlex.quote(pip)} install -q -e . 2>&1 | tail -2", cwd=ws, timeout=900)
    pkg = IMPORT_PKG.get(m["repo"].split("/")[-1], m["repo"].split("/")[-1])
    r = sh(f"{shlex.quote(venv_python)} -c \"import {pkg}, os; print(os.path.dirname(os.path.abspath({pkg}.__file__)))\"",
           cwd=ws, timeout=120)
    target = r.stdout.strip()
    if r.returncode != 0 or not target:
        return fail(f"editable import of {pkg!r} failed: {(r.stdout + r.stderr).strip()[-200:]}")
    if os.path.commonpath([os.path.abspath(ws), os.path.abspath(target)]) != os.path.abspath(ws):
        return fail(f"editable import of {pkg!r} resolves outside task ws: {target}")
    sel = " ".join(shlex.quote(n) for n in nodes)
    rc = sh(f"{shlex.quote(venv_python)} -m pytest {sel} --co -q", cwd=ws, timeout=900)
    collected = [l.strip() for l in rc.stdout.splitlines() if "::" in l and not l.startswith("=")]
    coll_err = len(_re.findall(r"^ERROR", rc.stdout, _re.M)) + len(_re.findall(r"^ERROR", rc.stderr, _re.M))
    if coll_err:
        return fail(f"{coll_err} collection ERROR(s): {(rc.stdout + rc.stderr)[-300:]}")
    for n in nodes:
        if "::" in n:
            if n not in collected:
                return fail(f"node not collected: {n} (collected {len(collected)})")
        else:
            if not any(c.startswith(n) for c in collected):
                return fail(f"no tests collected from file node: {n}")
    if not collected:
        return fail("zero tests collected")
    rx = sh(f"{shlex.quote(venv_python)} -m pytest {sel} -q", cwd=ws, timeout=1800)
    out = rx.stdout + rx.stderr
    counts = {}
    for v, k in _re.findall(r"(\d+) (passed|failed|error)s?", out):
        counts[k] = int(v)
    executed = counts.get("passed", 0) + counts.get("failed", 0)
    if counts.get("error", 0):
        return fail(f"{counts['error']} ERROR(s) at execute: {out[-300:]}")
    if executed < 1 or "found no collectors" in out or "no tests ran" in out:
        return fail(f"zero tests executed: {out[-300:]}")
    return None

SHIMS = os.path.join(os.path.dirname(S50.rstrip("/")), "shims")

def ensure_shims():
    """ab2/17123 finding: the exec env has no `python` alias (python3 only),
    so model verification commands like `python -m pytest` died with empty
    exit-2 output. Ship a shim dir on PATH instead of touching the box."""
    os.makedirs(SHIMS, exist_ok=True)
    py = os.path.join(SHIMS, "python")
    if not os.path.exists(py):
        os.symlink(__import__("shutil").which("python3"), py)

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
        node_list = f2p_nodes(m, ws, venv_py)
        nodes = " ".join(shlex.quote(n) for n in node_list)
        f2p_sh = os.path.join(run_dir, "f2p.sh")
        open(f2p_sh, "w").write(f"#!/bin/bash\nexec {shlex.quote(venv_py)} -m pytest {nodes} -x -q\n")
        pf_err = preflight_gate(m, ws, venv_py, node_list)
        if pf_err:
            res = {"instance_id": iid, "passed": False, "steps": 0, "model_calls": 0,
                   "cost_micros": 0, "wall_secs": int(time.time() - t0), "error": pf_err}
            note = "preflight_invalid"
            print(f"[w{wid}] {iid} PREFLIGHT REFUSED: {pf_err}", flush=True)
        else:
            env = dict(os.environ)
            up = MODEL.upper()
            env.update({
                f"HS_{up}_API_KEY_FILE": f"/home/sandbox/.keys/{MODEL}.key",
                f"HS_{up}_EXTRA_BODY_JSON": os.environ.get(f"HS_{up}_EXTRA_BODY_JSON", '{"reasoning_effort":"low"}'),
                "HS_SWE_PROMPT_NUDGE": os.environ.get(
                    "HS_SWE_PROMPT_NUDGE",
                    "IMPORTANT: before every answer.submit, run the FAIL_TO_PASS command via repo.exec and fix whatever it reports."),
                "HS_SWE_WORKSPACE": ws,
                "HS_SWE_F2P": f"bash {f2p_sh}",
                "HS_SWE_P2P": "",
                "HS_REALMODEL_CALL_TIMEOUT_SECS": "1500",
            })
            if MODEL == "glm":
                env["HS_GLM_BASE_URL"] = "http://127.0.0.1:8787/chat/completions"
            env["PATH"] = SHIMS + ":" + env.get("PATH", "")
            budget_flag = ""
            if os.environ.get("HS_CONTEXT_BUDGET_TOKENS"):
                budget_flag = f" --context-budget-tokens {os.environ['HS_CONTEXT_BUDGET_TOKENS']}"
            r = sh(f"timeout {TASK_WALL_SECS} {HS} --instance {shlex.quote(os.path.join(S50, 'instances', iid + '.json'))} "
                   f"--model {MODEL} --feedback on --budget-micros 10000000 --max-steps {MAX_STEPS}"
                   f"{budget_flag} "
                   f"--run-dir {shlex.quote(run_dir)}", timeout=TASK_WALL_SECS + 100, env=env)
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
                       "wall_secs": TASK_WALL_SECS, "error": "wall_timeout"}  # ab2: real cap, never a hardcoded 1800
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

ensure_shims()

if __name__ == "__main__":
    MANIFEST = json.load(open(os.path.join(S50, "manifest.json")))
    os.makedirs(CLAIMS, exist_ok=True)
    ts = [threading.Thread(target=worker, args=(i,)) for i in range(PAR)]
    for t in ts: t.start()
    for t in ts: t.join()
    open(os.path.join(S50, ".subset_complete"), "w").write("done")
    write_status(MANIFEST, "complete")
    print("SUBSET_COMPLETE", flush=True)
