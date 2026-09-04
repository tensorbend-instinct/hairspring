#!/bin/bash
# preflight <repo_slug> <instance_id>: venv + editable install + base-state
# F2P must FAIL ON THE BUG (assertion), never on imports.
# Per-repo test deps discovered 2026-09-04 (see ops/BOX_PLAYBOOK.md).
set -x
REPO=$1; IID=$2
S50=${S50:-/home/sandbox/swbench/subset50}
EXTRA_DEPS=""
case $REPO in
  haystack)   EXTRA_DEPS="ddtrace opentelemetry-sdk";;
  streamlink) EXTRA_DEPS="freezegun requests-mock versioningit";;
  pdm)        EXTRA_DEPS="pytest-mock";;
esac
python3 - "$S50" "$IID" "$REPO" <<'PY'
import json, sys
s50, iid, repo = sys.argv[1:4]
m = json.load(open(s50 + '/manifest.json'))
r = next(x for x in m if x['instance_id'] == iid)
json.dump(r, open(f'/tmp/pf-{repo}.json','w'))
open(f'/tmp/pf-{repo}-testpatch.diff','w').write(r['test_patch'])
open(f'/tmp/pf-{repo}-f2p.txt','w').write('\n'.join(r['fail_to_pass']))
PY
COMMIT=$(python3 -c "import json; print(json.load(open('/tmp/pf-$REPO.json'))['base_commit'])")
GHREPO=$(python3 -c "import json; print(json.load(open('/tmp/pf-$REPO.json'))['repo'])")
mkdir -p /home/sandbox/swbench/tarballs
TB=/home/sandbox/swbench/tarballs/$(echo $GHREPO | tr / _)_$COMMIT.tar.gz
[ -f "$TB" ] || curl -sL "https://github.com/$GHREPO/archive/$COMMIT.tar.gz" -o "$TB"
rm -rf /tmp/pf-ws-$REPO && mkdir -p /tmp/pf-ws-$REPO
tar xzf "$TB" -C /tmp/pf-ws-$REPO --strip-components=1
cd /tmp/pf-ws-$REPO && git init -q && git add -A && git -c user.email=b@b -c user.name=b commit -qm base && git tag v9.9.9
git apply /tmp/pf-$REPO-testpatch.diff && git add -A && git -c user.email=b@b -c user.name=b commit -qm testpatch
VENV=/home/sandbox/swbench/venvs/$REPO
python3 -m venv "$VENV"
"$VENV/bin/pip" install -q --upgrade pip 2>&1 | tail -1
"$VENV/bin/pip" install -q -e . pytest $EXTRA_DEPS 2>&1 | tail -3
# fragments (unclosed '[') expand via collection, same as the runner
python3 - "$REPO" <<'PY'
import json, subprocess, sys
repo = sys.argv[1]
frags = [l.strip() for l in open(f'/tmp/pf-{repo}-f2p.txt') if l.strip()]
nodes, todo = [], []
for e in frags:
    (todo if ('[' in e and ']' not in e) else nodes).append(e)
if todo:
    files = sorted(set(f.split('::')[0] for f in todo))
    r = subprocess.run([f'/home/sandbox/swbench/venvs/{repo}/bin/python', '-m', 'pytest', *files, '--co', '-q'],
                       cwd=f'/tmp/pf-ws-{repo}', capture_output=True, text=True)
    collected = [l.strip() for l in r.stdout.splitlines() if '::' in l]
    for f in todo:
        m = [c for c in collected if c.startswith(f)]
        nodes.extend(m if m else [f.split('::')[0]])
open(f'/tmp/pf-{repo}-nodes.txt','w').write('\n'.join(sorted(set(nodes))))
PY
NODES=$(python3 -c "
import shlex
print(' '.join(shlex.quote(l) for l in open('/tmp/pf-$REPO-nodes.txt').read().splitlines() if l.strip()))")
cd /tmp/pf-ws-$REPO && "$VENV/bin/python" -m pytest $NODES -x -q 2>&1 | tail -8
echo "PREFLIGHT_DONE $REPO"
