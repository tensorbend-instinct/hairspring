#!/usr/bin/env python3
import csv,json,pathlib,sys
root=pathlib.Path(__file__).resolve().parent
mode=sys.argv[1] if len(sys.argv)>1 else 'benchmark'
if mode=='benchmark':
 hs=list(csv.reader((root/'evidence/hairspring-swe-live.csv').open()))
 swe=json.load((root/'evidence/swe-agent-swe-live.json').open())
 print(json.dumps({'hairspring':{'tasks':len(hs),'resolved':sum(r[1]=='True' for r in hs),'steps':sum(int(r[2]) for r in hs),'cost_usd':sum(int(r[4]) for r in hs)/1e6,'wall_seconds':sum(int(r[5]) for r in hs)},'swe_agent':{'tasks':len(swe),'resolved':sum(bool(x['passed']) for x in swe),'steps':sum(int(x['steps']) for x in swe),'cost_usd':sum(float(x['litellm_cost']) for x in swe),'wall_seconds':sum(int(x['wall_secs']) for x in swe)}},indent=2))
elif mode=='source':
 rs=list((root.parent/'crates').glob('**/*.rs')); print(json.dumps({'rust_files':len(rs),'rust_lines':sum(sum(1 for _ in p.open(errors='ignore')) for p in rs),'crates':len(list((root.parent/'crates').glob('*/Cargo.toml')))},indent=2))
else: raise SystemExit('usage: reproduce.py benchmark|source')
