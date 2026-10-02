#!/usr/bin/env python3
"""Re-grade HAIRSPRING candidates that the grader vetoed only for untracked scratch files.

The agent's tracked diff (candidate.patch) is applied to a clean base clone and graded with the
unchanged score_native.py; untracked scratch never reaches the grader. No model calls.
usage: regrade.py ARTS_DIR CASE_DIR PYTHON OUT_DIR
"""
import json,pathlib,subprocess,sys,glob,os
arts,case,py,out=map(pathlib.Path,sys.argv[1:5]);out.mkdir(parents=True,exist_ok=True)
public=json.loads((case/'public.json').read_text());base=public['base_commit'];R=pathlib.Path(__file__).resolve().parent
rows=[]
for rec in sorted(glob.glob(str(arts/'*'/'runs'/'*'/'HAIRSPRING'/'record.json'))):
    d=pathlib.Path(rec).parent;r=json.loads(pathlib.Path(rec).read_text());rep=pathlib.Path(rec).parts[-5]
    row={'artifact':rep,'instance_id':r['instance_id'],'harness':'HAIRSPRING','orig_state':r['state'],'steps':r.get('steps'),'wall_secs':r.get('wall_secs')}
    if r['state']!='grade_failed':
        row.update({'state':r['state'],'resolved':r.get('resolved')});rows.append(row);continue
    tree=out/(rep+'-tree');subprocess.run(['git','clone','--quiet','--no-hardlinks',str(case/'source'),str(tree)],check=True)
    subprocess.run(['git','checkout','--quiet',base],cwd=tree,check=True)
    patch=d/'candidate.patch'
    if patch.stat().st_size:
        ap=subprocess.run(['git','apply','--binary',str(patch)],cwd=tree)
        if ap.returncode:row.update({'state':'regrade_patch_failed','resolved':False});rows.append(row);continue
    sc=out/(rep+'-score')
    p=subprocess.run([sys.executable,str(R/'score_native.py'),'--case',str(case),'--agent-tree',str(tree),'--python',py,'--out',str(sc)],capture_output=True,text=True)
    s=json.loads((sc/'score.json').read_text()) if (sc/'score.json').exists() else None
    row.update({'state':'regraded' if s else 'regrade_failed','resolved':bool(s and s.get('resolved') and p.returncode==0),'rc':p.returncode,'tail':(p.stderr+p.stdout)[-200:] if not s else ''})
    rows.append(row)
(out/'regrade.json').write_text(json.dumps(rows,indent=2)+'\n')
for x in rows:print(json.dumps(x))
