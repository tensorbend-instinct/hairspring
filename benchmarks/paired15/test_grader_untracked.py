"""Exact .hs/checks exemption only, other untracked source must veto."""
import pathlib,subprocess,tempfile,json,sys
R=pathlib.Path(__file__).resolve().parent;case=R/'probe-django11133'
with tempfile.TemporaryDirectory(prefix='hs-untracked-') as d:
 t=pathlib.Path(d);agent=t/'agent';subprocess.run(['git','clone','--quiet','--no-hardlinks',str(case/'source'),str(agent)],check=True)
 def score(name):
  p=subprocess.run([sys.executable,str(R/'score_native.py'),'--case',str(case),'--agent-tree',str(agent),'--python',str(R/'django38-env/bin/python'),'--out',str(t/name)],capture_output=True,text=True)
  print(name,'exit',p.returncode,'score exists',(t/name/'score.json').exists(),'untracked',json.loads((t/name/'untracked.json').read_text()),'tail',(p.stderr+p.stdout)[-220:])
  return p
 (agent/'.hs').mkdir();(agent/'.hs/checks').write_text('true\n');r=score('only-metadata');assert (t/'only-metadata/score.json').exists();assert json.loads((t/'only-metadata/score.json').read_text())['resolved'] is False
 (agent/'new-source.py').write_text('x=1\n');r=score('new-source');assert not (t/'new-source/score.json').exists() and 'untracked source files' in r.stderr
