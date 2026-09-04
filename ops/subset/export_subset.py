#!/usr/bin/env python3
"""Export the 50-task subset from SWE-bench-Live lite parquet.
Selection: conan 15, cfn-lint 15, haystack 8, streamlink 7, pdm 5 (first N
by instance_id per repo) - 5 installable pure-Python envs with diversity."""
import pyarrow.parquet as pq, json, os, sys
S50 = sys.argv[1] if len(sys.argv) > 1 else "/home/sandbox/swbench/subset50"
PARQUET = "/downloads/swelive_lite.parquet"
PICK = {'conan-io/conan': 15, 'aws-cloudformation/cfn-lint': 15, 'deepset-ai/haystack': 8,
        'streamlink/streamlink': 7, 'pdm-project/pdm': 5}
os.makedirs(os.path.join(S50, "instances"), exist_ok=True)
t = pq.read_table(PARQUET).to_pylist()
sel = []
for repo, n in PICK.items():
    rows = sorted([r for r in t if r['repo'] == repo], key=lambda r: r['instance_id'])[:n]
    assert len(rows) == n, f"{repo}: only {len(rows)}"
    sel.extend(rows)
manifest = []
for r in sel:
    f2p = r['FAIL_TO_PASS']
    if isinstance(f2p, str):
        f2p = json.loads(f2p)
    json.dump({'instance_id': r['instance_id'], 'problem_statement': r['problem_statement'],
               'FAIL_TO_PASS': f2p},
              open(os.path.join(S50, 'instances', r['instance_id'] + '.json'), 'w'))
    manifest.append({'instance_id': r['instance_id'], 'repo': r['repo'],
                     'base_commit': r['base_commit'], 'test_patch': r['test_patch'],
                     'fail_to_pass': f2p, 'difficulty': r['difficulty']})
json.dump(manifest, open(os.path.join(S50, 'manifest.json'), 'w'))
print('exported', len(manifest))
