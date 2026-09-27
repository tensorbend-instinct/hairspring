#!/usr/bin/env python3
"""Read-only preflight of a paired Verified instance. Never expose hidden tests to agents."""
import argparse,hashlib,json,pathlib,subprocess,sys
import pandas as pd
ROOT=pathlib.Path(__file__).resolve().parent
p=argparse.ArgumentParser();p.add_argument('--id',required=True);p.add_argument('--out',required=True);a=p.parse_args()
manifest=json.loads((ROOT/'manifest.json').read_text()); source=ROOT/'verified.parquet'
if hashlib.sha256(source.read_bytes()).hexdigest()!=manifest['source_sha256']:sys.exit('dataset hash mismatch')
if a.id not in manifest['ids']:sys.exit('instance not predeclared')
rows=pd.read_parquet(source); r=rows.loc[rows.instance_id.eq(a.id)].iloc[0];out=pathlib.Path(a.out).resolve();out.mkdir(parents=True,exist_ok=True)
repo=out/'source'; url='https://github.com/'+r.repo+'.git'
if not repo.exists():subprocess.run(['git','clone','--quiet','--filter=blob:none',url,str(repo)],check=True)
subprocess.run(['git','checkout','--quiet',r.base_commit],cwd=repo,check=True)
if subprocess.check_output(['git','rev-parse','HEAD'],cwd=repo,text=True).strip()!=r.base_commit:sys.exit('base mismatch')
(out/'test.patch').write_text(r.test_patch)
agent=out/'agent-source';subprocess.run(['git','clone','--quiet','--no-hardlinks',str(repo),str(agent)],check=True)
subprocess.run(['git','checkout','--quiet',r.base_commit],cwd=agent,check=True)
# The agent-source tree is clean; the test patch is visible only to orchestrator/grade clone.
subprocess.run(['git','apply','--check',str(out/'test.patch')],cwd=repo,check=True)
subprocess.run(['git','apply',str(out/'test.patch')],cwd=repo,check=True)
(out/'public.json').write_text(json.dumps({'instance_id':a.id,'repo':r.repo,'base_commit':r.base_commit,'problem_statement':r.problem_statement},indent=2)+'\n')
(out/'private.json').write_text(json.dumps({'instance_id':a.id,'FAIL_TO_PASS':json.loads(r.FAIL_TO_PASS),'PASS_TO_PASS':json.loads(r.PASS_TO_PASS),'test_patch_sha256':hashlib.sha256(r.test_patch.encode()).hexdigest()},indent=2)+'\n')
print(json.dumps({'instance_id':a.id,'base_commit':r.base_commit,'agent_tree':str(agent),'grade_tree':str(repo),'test_patch_sha256':hashlib.sha256(r.test_patch.encode()).hexdigest()}))
