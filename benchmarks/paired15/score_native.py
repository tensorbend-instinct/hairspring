#!/usr/bin/env python3
"""Native diagnostic scorer for one preflighted SWE-bench Verified instance.

Never mounts grade_tree/test.patch into the agent workspace. This is NOT the official
SWE-bench Docker evaluator. Missing or unrun pinned tests fail closed.
"""
import argparse,ast,json,pathlib,re,subprocess,sys,time
p=argparse.ArgumentParser(); p.add_argument('--case',required=True); p.add_argument('--agent-tree',required=True);p.add_argument('--python',required=True);p.add_argument('--out',required=True);p.add_argument('--allow-untracked',action='store_true');a=p.parse_args()
c=pathlib.Path(a.case).resolve();agent=pathlib.Path(a.agent_tree).resolve();source=c/'source';private=json.loads((c/'private.json').read_text());out=pathlib.Path(a.out).resolve();out.mkdir(parents=True,exist_ok=True)
repo=out/'grade'
if repo.exists():sys.exit('grade output already exists; use a fresh output directory')
subprocess.run(['git','clone','--quiet','--no-hardlinks',str(source),str(repo)],check=True)
subprocess.run(['git','checkout','--quiet',subprocess.check_output(['git','rev-parse','HEAD'],cwd=source,text=True).strip()],cwd=repo,check=True)
subprocess.run(['git','apply',str(c/'test.patch')],cwd=repo,check=True)
if repo==agent or repo in agent.parents or agent in repo.parents:sys.exit('grading repository must be separate')
base=subprocess.check_output(['git','rev-parse','HEAD'],cwd=agent,text=True).strip();base2=subprocess.check_output(['git','rev-parse','HEAD'],cwd=repo,text=True).strip()
if base!=base2:sys.exit('base commit mismatch')
# Only tracked source changes; untracked files are audited separately and excluded.
patch=subprocess.run(['git','diff','--binary','HEAD','--'],cwd=agent,text=True,stdout=subprocess.PIPE,check=True).stdout
(out/'candidate.patch').write_text(patch)
file_names=subprocess.check_output(['git','ls-files','--others','--exclude-standard'],cwd=agent,text=True).splitlines();(out/'untracked.json').write_text(json.dumps(file_names,indent=2)+'\n')
non_metadata=[x for x in file_names if x not in ('.hs/checks','.hs/instruction.txt')]
if non_metadata and not a.allow_untracked:sys.exit('agent has untracked source files: '+repr(non_metadata))
if any(x.startswith(('tests/','testing/')) or '/tests/' in x for x in subprocess.check_output(['git','diff','--name-only','HEAD','--'],cwd=agent,text=True).splitlines()):sys.exit('candidate changes test files; inspect before scoring')
if patch:
 check=subprocess.run(['git','apply','--check',str(out/'candidate.patch')],cwd=repo,stdout=subprocess.PIPE,stderr=subprocess.PIPE,text=True)
 if check.returncode:sys.exit('candidate does not apply cleanly to grader: '+check.stderr[:300])
 subprocess.run(['git','apply',str(out/'candidate.patch')],cwd=repo,check=True)
# Select an explicit repository-specific test adapter.
if private['instance_id'].startswith(('pytest-dev__','sympy__','sphinx-doc__','pylint-dev__','scikit-learn__')):
    import os,shutil
    if private['instance_id'].startswith('pytest-dev__'):
        # Historical pytest generates this file during setup.py --version.
        init=subprocess.run([a.python,'setup.py','--version'],cwd=repo,stdout=subprocess.PIPE,stderr=subprocess.PIPE,text=True)
        if init.returncode:sys.exit('pytest setup failed: '+init.stderr[-300:])
    targets=private['FAIL_TO_PASS']+private['PASS_TO_PASS']
    if private['instance_id'].startswith('pytest-dev__'):
        if not all(x.startswith('testing/') and '::' in x for x in targets):sys.exit('unrecognized pytest pinned ID')
    elif private['instance_id'].startswith('sympy__'):
        if not (repo/'sympy/utilities/tests/test_lambdify.py').exists():sys.exit('unexpected SymPy test location')
        if not all(re.fullmatch(r'test_[A-Za-z0-9_]+',x) for x in targets):sys.exit('unrecognized SymPy pinned ID')
        targets=['sympy/utilities/tests/test_lambdify.py::'+x for x in targets]
    elif private['instance_id'].startswith('scikit-learn__'):
        if not all(x.startswith('sklearn/tree/tests/test_export.py::') for x in targets):sys.exit('unrecognized scikit-learn pinned ID')
    else:
        if not all(x.startswith('tests/') and '::' in x for x in targets):sys.exit('unrecognized test pinned ID')
    expansion={x:[x] for x in targets}
    if private['instance_id']=='pylint-dev__pylint-4551':
        # Two dataset IDs are truncated parameter prefixes. Expand every
        # collected match and require all of them to pass as one pinned ID.
        env_collect=os.environ.copy();env_collect['PYTHONPATH']=str(repo);env_collect['PYTEST_DISABLE_PLUGIN_AUTOLOAD']='1'
        collected=subprocess.run([a.python,'-m','pytest','--collect-only','-q','-p','no:cacheprovider','tests/unittest_pyreverse_writer.py'],cwd=repo,env=env_collect,stdout=subprocess.PIPE,stderr=subprocess.STDOUT,text=True)
        if collected.returncode:
            # The base cannot collect because hidden tests import the absent
            # function. Return a fail-closed result instead of a false pass.
            (out/'tests.log').write_text(collected.stdout)
            fail={'instance_id':private['instance_id'],'resolved':False,'fail_to_pass_total':len(private['FAIL_TO_PASS']),'fail_to_pass_ok':0,'pass_to_pass_total':len(private['PASS_TO_PASS']),'pass_to_pass_ok':0,'not_run':len(targets),'test_exit_code':collected.returncode,'failures':[{'test':x,'status':'NOT_RUN'} for x in targets]}
            (out/'score.json').write_text(json.dumps(fail,indent=2)+'\n');print(json.dumps(fail));sys.exit(1)
        nodes=set(line for line in collected.stdout.splitlines() if line.startswith('tests/unittest_pyreverse_writer.py::'))
        for x in targets:
            matches=sorted(y for y in nodes if y.startswith(x))
            if not matches:sys.exit('pylint pinned prefix has no collected node: '+repr(x))
            if x.endswith(']') and matches!=[x]:sys.exit('complete pylint pinned ID must match exactly: '+repr(x))
            expansion[x]=matches
        (out/'pylint-expansion.json').write_text(json.dumps(expansion,indent=2)+'\n')
        targets=[y for x in targets for y in expansion[x]]
        if len(set(targets))!=len(targets):sys.exit('pylint expansion has duplicate nodes')
    if private['instance_id'].startswith('scikit-learn__'):
        # Built artifacts are not a candidate patch. Build the fresh grade clone
        # from its exact base, then run the pinned tests in that clone.
        with (out/'build.log').open('w') as buildlog:
            build=subprocess.run([a.python,'setup.py','build_ext','--inplace','-j','1'],cwd=repo,stdout=buildlog,stderr=subprocess.STDOUT,timeout=1100)
        if build.returncode:sys.exit('scikit-learn grade build failed; see build.log')
    env=os.environ.copy();env['PYTHONPATH']=str(repo/('src' if private['instance_id'].startswith('pytest-dev__') else ''));env['PYTEST_DISABLE_PLUGIN_AUTOLOAD']='1'
    start=time.monotonic();cmd=[a.python,'-m','pytest','-q','-rA','-p','no:cacheprovider',*targets]
    with (out/'tests.log').open('w') as log:proc=subprocess.run(cmd,cwd=repo,env=env,stdout=log,stderr=subprocess.STDOUT)
    log=(out/'tests.log').read_text(errors='replace');status={}
    for line in log.splitlines():
        m=re.match(r'^(PASSED|FAILED|ERROR|SKIPPED) ((?:testing/|sympy/|tests/|sklearn/).+?)(?: - |$)',line)
        if m:status[m.group(2)]=m.group(1)
    verdict=[{'test':x,'status':'PASSED' if all(status.get(y)=='PASSED' for y in expansion[x]) else next((status.get(y,'NOT_RUN') for y in expansion[x] if status.get(y)!='PASSED'),'NOT_RUN'),'matched_nodes':len(expansion[x])} for x in private['FAIL_TO_PASS']+private['PASS_TO_PASS']]
    f=verdict[:len(private['FAIL_TO_PASS'])];t=verdict[len(f):]
    result={'instance_id':private['instance_id'],'resolved':all(x['status']=='PASSED' for x in verdict),'fail_to_pass_total':len(f),'fail_to_pass_ok':sum(x['status']=='PASSED' for x in f),'pass_to_pass_total':len(t),'pass_to_pass_ok':sum(x['status']=='PASSED' for x in t),'not_run':sum(x['status']=='NOT_RUN' for x in verdict),'test_exit_code':proc.returncode,'test_wall_secs':round(time.monotonic()-start,3),'patch_sha256':__import__('hashlib').sha256(patch.encode()).hexdigest(),'failures':[x for x in verdict if x['status']!='PASSED'][:30]}
    (out/'score.json').write_text(json.dumps(result,indent=2)+'\n');print(json.dumps(result))
    if os.environ.get('HS_KEEP_GRADE_CLONE')!='1':shutil.rmtree(repo)
    sys.exit(0 if result['resolved'] and proc.returncode==0 else 1)
if not (repo/'tests/runtests.py').exists():sys.exit('Django test runner not found; add explicit repository-specific adapter')
targets=private['FAIL_TO_PASS']+private['PASS_TO_PASS']
pattern=re.compile(r'^(test_\w+) \(([A-Za-z_]\w*(?:\.[A-Za-z_]\w*)+)\)$')
parsed=[]
# Django 2.x's unittest test IDs sometimes store just the method docstring,
# not the executable ID. Map the exact first docstring line to a unique test
# method in this case's tests tree, then fail closed on ambiguity.
docs={}
for file in (repo/'tests').rglob('*.py'):
    try:tree=ast.parse(file.read_text(encoding='utf8'))
    except (SyntaxError,UnicodeError):continue
    package='.'.join(file.relative_to(repo/'tests').with_suffix('').parts)
    for cls in [n for n in tree.body if isinstance(n,ast.ClassDef)]:
        for fn in [n for n in cls.body if isinstance(n,ast.FunctionDef) and n.name.startswith('test_')]:
            doc=ast.get_docstring(fn)
            if doc:docs.setdefault(doc.splitlines()[0].strip(),[]).append((fn.name,package+'.'+cls.name))
for name in targets:
 m=pattern.fullmatch(name)
 if m:
  method,klass=m.groups()
  if klass.endswith('.'+method):klass=klass[:-(len(method)+1)]
  parsed.append((name,klass+'.'+method));continue
 matches=docs.get(name,[])
 if not matches:sys.exit('pinned docstring has no matching test method: '+repr(name))
 # Ambiguous descriptions count as one pinned ID; require every exact-match
 # method to pass, rather than silently selecting one.
 parsed.append((name,[klass+'.'+method for method,klass in matches]))
# Report no result if opaque doctest descriptions cannot map uniquely.
flat=[target for _,targets in parsed for target in (targets if isinstance(targets,list) else [targets])]
if len(set(flat))!=len(flat):sys.exit('duplicate normalized pinned test ID')
cmd=[a.python,'tests/runtests.py','--parallel','1','--verbosity','2','--noinput',*flat]
import os
env=os.environ.copy();env['PYTHONPATH']=str(repo)+os.pathsep+str(repo/'tests');start=time.monotonic()
with (out/'tests.log').open('w') as log:proc=subprocess.run(cmd,cwd=repo,env=env,stdout=log,stderr=subprocess.STDOUT)
log=(out/'tests.log').read_text(errors='replace');status={}
pending=None
for line in log.splitlines():
    m=re.match(r'^(test_\w+) \(([^ ()]+)\)(.*)$',line)
    if m:
        klass=m.group(2)
        if klass.endswith('.'+m.group(1)):klass=klass[:-(len(m.group(1))+1)]
        pending=(m.group(1),klass);tail=m.group(3)
    else:tail=line
    if pending:
        end=re.search(r'\.\.\. (ok|FAIL|ERROR|skipped .*|expected failure|unexpected success)$',tail)
        if end:status[pending]=end.group(1);pending=None
verdict=[]
for original,target in parsed:
 vals=[]
 for one in (target if isinstance(target,list) else [target]):
  method,klass=one.rsplit('.',1)[-1],one.rsplit('.',1)[0]
  vals.append(status.get((method,klass),'NOT_RUN'))
 s='ok' if all(v=='ok' for v in vals) else (next((v for v in vals if v!='ok'),'NOT_RUN'))
 verdict.append({'test':original,'status':s,'matched_methods':len(vals)})
f=verdict[:len(private['FAIL_TO_PASS'])];t=verdict[len(f):]
resolved=all(x['status']=='ok' for x in f+t)
result={'instance_id':private['instance_id'],'resolved':resolved,'fail_to_pass_total':len(f),'fail_to_pass_ok':sum(x['status']=='ok' for x in f),'pass_to_pass_total':len(t),'pass_to_pass_ok':sum(x['status']=='ok' for x in t),'not_run':sum(x['status']=='NOT_RUN' for x in verdict),'test_exit_code':proc.returncode,'test_wall_secs':round(time.monotonic()-start,3),'patch_sha256':__import__('hashlib').sha256(patch.encode()).hexdigest(),'failures':[x for x in verdict if x['status']!='ok'][:30]}
(out/'score.json').write_text(json.dumps(result,indent=2)+'\n');print(json.dumps(result));
if os.environ.get('HS_KEEP_GRADE_CLONE')!='1':__import__('shutil').rmtree(repo)
sys.exit(0 if proc.returncode==0 and result['not_run']==0 else 1)
