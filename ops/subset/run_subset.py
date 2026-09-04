#!/usr/bin/env python3
"""50-task SWE-bench-Live subset runner: serial missions, low effort, $10/task cap.
Resume-safe: skips tasks whose run dir has result.json. prep_ws is NOT
idempotent over an existing ws - remove partial dirs lacking result.json
before relaunching."""
import json, os, subprocess, sys, time, shlex, urllib.request

S50 = os.environ.get("S50", "/home/sandbox/swbench/subset50")
HS = os.environ.get("HS_SWE_RUN_BIN", "/home/sandbox/hairspring/target/debug/hs-swe-run")
TB_DIR = "/home/sandbox/swbench/tarballs"
VENV = "/home/sandbox/swbench/venvs"
LEDGER = os.path.join(S50, "ledger.csv")
STATUS = os.path.join(S50, "status.json")
SPEND_GUARDRAIL_MICROS = 50_000_000  # $50 subset guardrail (expected ~$2.50)
MAX_STEPS = os.environ.get("HS_SUBSET_MAX_STEPS", "50")

def sh(cmd, cwd=None, timeout=None, env=None):
    return subprocess.run(cmd, shell=True, cwd=cwd, timeout=timeout, env=env,
                          capture_output=True, text=True)

def prep_ws(m, run_dir):
    ws = os.path.join(run_dir, "ws")
    repo, commit = m["repo"], m["base_commit"]
    tb = os.path.join(TB_DIR, repo.replace("/", "_") + "_" + commit + ".tar.gz")
    if not os.path.exists(tb):
        url = f"https://github.com/{repo}/archive/{commit}.tar.gz"
        urllib.request.urlretrieve(url, tb)
    os.makedirs(ws, exist_ok=True)
    sh(f"tar xzf {shlex.quote(tb)} -C {shlex.quote(ws)} --strip-components=1")
    sh("git init -q && git add -A && git -c user.email=b@b -c user.name=b commit -qm base && git tag v9.9.9", cwd=ws)
    tp = os.path.join(run_dir, "test_patch.diff")
    open(tp, "w").write(m["test_patch"])
    r = sh(f"git apply {shlex.quote(tp)} && git add -A && git -c user.email=b@b -c user.name=b commit -qm testpatch", cwd=ws)
    if r.returncode != 0:
        return None, f"test_patch apply failed: {r.stderr[-300:]}"
    return ws, None

def f2p_nodes(m, ws, venv_python):
    """Expand dataset F2P entries to runnable pytest node ids.
    SWE-bench-Live lite truncates parametrized ids at the first comma inside
    brackets; expand fragments via pytest collection with prefix matching,
    falling back to file-level selection when collection yields no match."""
    nodes, frags = [], []
    for e in m["fail_to_pass"]:
        e = e.strip()
        if not e:
            continue
        if "[" in e and "]" not in e:
            frags.append(e)
        else:
            nodes.append(e)
    if frags:
        files = sorted(set(f.split("::")[0] for f in frags))
        r = sh(f"{shlex.quote(venv_python)} -m pytest {' '.join(files)} --co -q", cwd=ws, timeout=600)
        collected = [l.strip() for l in r.stdout.splitlines() if "::" in l]
        for f in frags:
            match = [c for c in collected if c.startswith(f)]
            nodes.extend(match if match else [f.split("::")[0]])
    return sorted(set(nodes))

def main():
    manifest = json.load(open(os.path.join(S50, "manifest.json")))
    os.makedirs(TB_DIR, exist_ok=True)
    if not os.path.exists(LEDGER):
        open(LEDGER, "w").write("instance_id,passed,steps,model_calls,cost_micros,wall_secs,note\n")
    total_micros = 0
    # superseded prior-cap results: spend stays on the books via ledger rows
    # annotated superseded-*; those tasks re-run under the current cap
    for line in open(LEDGER):
        parts = line.strip().split(",")
        if len(parts) >= 7 and parts[6].startswith("superseded"):
            total_micros += int(parts[4])
    done = passed_n = 0
    for m in manifest:
        iid = m["instance_id"]
        run_dir = os.path.join(S50, "runs", iid)
        os.makedirs(run_dir, exist_ok=True)
        rj = os.path.join(run_dir, "result.json")
        if os.path.exists(rj):
            d = json.load(open(rj))
            done += 1; passed_n += bool(d.get("passed")); total_micros += d.get("cost_micros", 0)
            continue
        slug = m["repo"].split("/")[-1]
        t0 = time.time()
        ws, err = prep_ws(m, run_dir)
        note = ""
        if err:
            res = {"instance_id": iid, "passed": False, "steps": 0, "model_calls": 0,
                   "cost_micros": 0, "wall_secs": int(time.time() - t0), "error": err}
            note = "prep_error"
        else:
            venv_py = os.path.join(VENV, slug, "bin")
            sh(f"{shlex.quote(os.path.join(venv_py, 'pip'))} install -q -e . 2>&1 | tail -2", cwd=ws, timeout=900)
            nodes = " ".join(shlex.quote(n) for n in f2p_nodes(m, ws, os.path.join(venv_py, 'python')))
            f2p_sh = os.path.join(run_dir, "f2p.sh")
            open(f2p_sh, "w").write(f"#!/bin/bash\nexec {shlex.quote(os.path.join(venv_py, 'python'))} -m pytest {nodes} -x -q\n")
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
            r = sh(f"timeout 1800 {HS} --instance {shlex.quote(os.path.join(S50, 'instances', iid + '.json'))} "
                   f"--model glm --feedback on --budget-micros 10000000 --max-steps {MAX_STEPS} "
                   f"--run-dir {shlex.quote(run_dir)}", timeout=1900, env=env)
            open(os.path.join(run_dir, "stdout.log"), "w").write(r.stdout + "\n--- STDERR ---\n" + r.stderr)
            if r.returncode == 124 and not os.path.exists(rj):
                res = {"instance_id": iid, "passed": False, "steps": 0, "model_calls": 0,
                       "cost_micros": 0, "wall_secs": 1800, "error": "wall_timeout"}
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
        done += 1; passed_n += bool(res.get("passed")); total_micros += res.get("cost_micros", 0)
        with open(LEDGER, "a") as f:
            f.write(f"{iid},{res.get('passed')},{res.get('steps',0)},{res.get('model_calls',0)},{res.get('cost_micros',0)},{res.get('wall_secs',0)},{note}\n")
        json.dump({"done": done, "of": len(manifest), "passed": passed_n, "failed": done - passed_n,
                   "spend_usd": round(total_micros / 1e6, 3), "current": iid,
                   "updated": time.strftime("%F %T")}, open(STATUS, "w"), indent=2)
        if total_micros > SPEND_GUARDRAIL_MICROS:
            json.dump({"done": done, "of": len(manifest), "passed": passed_n, "failed": done - passed_n,
                       "spend_usd": round(total_micros / 1e6, 3), "STOPPED": "spend guardrail $50",
                       "updated": time.strftime("%F %T")}, open(STATUS, "w"), indent=2)
            print("SPEND GUARDRAIL TRIPPED", flush=True)
            break
    open(os.path.join(S50, ".subset_complete"), "w").write("done")
    print("SUBSET_COMPLETE", flush=True)

if __name__ == "__main__":
    main()
