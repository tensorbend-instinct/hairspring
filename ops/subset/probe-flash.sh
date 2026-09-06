#!/bin/bash
# GLM-5.3-Flash high-effort probe on one instance (Eric 2026-09-04 16:56).
# Usage: probe-flash.sh <instance_id> <repo_slug>
set -e
IID=$1; SLUG=$2
S50=${S50:-/home/sandbox/swbench/subset50}
PR=/home/sandbox/swbench/probe-flash
mkdir -p "$PR/ws"
python3 - "$S50" "$IID" "$PR" <<'PY'
import json, sys
s50, iid, pr = sys.argv[1:4]
m = json.load(open(s50 + '/manifest.json'))
r = next(x for x in m if x['instance_id'] == iid)
open(pr + '/test_patch.diff', 'w').write(r['test_patch'])
open(pr + '/commit.txt', 'w').write(r['base_commit'])
PY
COMMIT=$(cat "$PR/commit.txt")
GHREPO=$(python3 -c "import json; m=json.load(open('$S50/manifest.json')); print(next(x['repo'] for x in m if x['instance_id']=='$IID'))")
TB=/home/sandbox/swbench/tarballs/$(echo $GHREPO | tr / _)_$COMMIT.tar.gz
[ -f "$TB" ] || curl -sL "https://github.com/$GHREPO/archive/$COMMIT.tar.gz" -o "$TB"
tar xzf "$TB" -C "$PR/ws" --strip-components=1
cd "$PR/ws" && git init -q && git add -A && git -c user.email=b@b -c user.name=b commit -qm base && git tag v9.9.9
git apply "$PR/test_patch.diff" && git add -A && git -c user.email=b@b -c user.name=b commit -qm testpatch
/home/sandbox/swbench/venvs/$IID/bin/pip install -q -e . 2>&1 | tail -1
python3 - "$S50" "$IID" "$SLUG" "$PR" <<'PY'
import json, shlex, subprocess, sys
s50, iid, slug, pr = sys.argv[1:5]
m = json.load(open(s50 + '/manifest.json'))
r = next(x for x in m if x['instance_id'] == iid)
vp = f'/home/sandbox/swbench/venvs/{iid}/bin/python'
nodes, frags = [], []
for e in r['fail_to_pass']:
    e = e.strip()
    if e: (frags if ('[' in e and ']' not in e) else nodes).append(e)
if frags:
    files = sorted(set(f.split('::')[0] for f in frags))
    rr = subprocess.run([vp, '-m', 'pytest', *files, '--co', '-q'], cwd=pr + '/ws', capture_output=True, text=True)
    collected = [l.strip() for l in rr.stdout.splitlines() if '::' in l]
    for f in frags:
        mm = [c for c in collected if c.startswith(f)]
        nodes.extend(mm if mm else [f.split('::')[0]])
node_str = ' '.join(shlex.quote(n) for n in sorted(set(nodes)))
open(pr + '/f2p.sh', 'w').write(f'#!/bin/bash\nexec {vp} -m pytest {node_str} -x -q\n')
PY
cd "$PR"
exec env HS_GLM_API_KEY_FILE=/home/sandbox/.keys/glm.key \
  HS_GLM_BASE_URL=http://127.0.0.1:8787/chat/completions \
  HS_GLM_MODEL=glm-5.3-flash \
  HS_GLM_EXTRA_BODY_JSON='{"reasoning_effort":"high"}' \
  HS_SWE_PROMPT_NUDGE='IMPORTANT: before every answer.submit, run the FAIL_TO_PASS command on your patch via repo.exec and fix whatever it reports.' \
  HS_SWE_WORKSPACE="$PR/ws" \
  HS_SWE_F2P="bash $PR/f2p.sh" \
  HS_SWE_P2P='' \
  HS_REALMODEL_CALL_TIMEOUT_SECS=1500 \
  /home/sandbox/hairspring/target/debug/hs-swe-run \
  --instance "$S50/instances/$IID.json" \
  --model glm --feedback on --budget-micros 10000000 --max-steps 50 \
  --run-dir "$PR" > "$PR/stdout.log" 2>&1
