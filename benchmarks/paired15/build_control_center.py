#!/usr/bin/env python3
"""Publish an allowlisted, public-safe snapshot of paired benchmark state.

Do not copy raw logs, request/response bodies, ledger, local paths, or model text.
"""
import argparse,datetime,json,pathlib,re,time
P=argparse.ArgumentParser()
P.add_argument('--proxy-log',type=pathlib.Path,required=True,action='append',help='May be repeated for sequential proxy generations')
P.add_argument('--ledger',type=pathlib.Path,help='Private ledger used only to verify manual reserve bookings; no amounts published')
P.add_argument('--runs',type=pathlib.Path,required=True)
P.add_argument('--output',type=pathlib.Path,required=True)
A=P.parse_args()
ID=re.compile(r'^[a-f0-9]{16}$')
CASE=re.compile(r'^[a-zA-Z0-9_.-]+__[a-zA-Z0-9_.-]+$')
STAGES={'admitted','upstream_response','watchdog_heartbeat','settled_usage','settled_at_reserve','upstream_error','upstream_http_error','client_disconnected','admission_denied','coordinator_forward_enter'}
OUTCOMES={'upstream_complete','upstream_total_deadline','upstream_error','upstream_http_error','unknown'}
rows={};calibration=None
for log in A.proxy_log:
 for line in log.open(errors='replace'):
  try:e=json.loads(line)
  except ValueError:continue
  if e.get('event')=='proxy_clock_calibration':
   ratio=e.get('wall_per_monotonic');calibration=round(ratio,4) if isinstance(ratio,(int,float)) and 0<ratio<100 else None
   continue
  if e.get('event')!='proxy_transition' or not ID.fullmatch(str(e.get('request_id',''))):continue
  stage=e.get('stage')
  if stage not in STAGES:continue
  rid=e['request_id'];r=rows.setdefault(rid,{'id':rid,'bytes':None,'status':None,'phase':'admitted','latency_s':None,'heartbeat_count':0,'heartbeat_elapsed_s':None,'deadline_s':1020,'settlement':None,'error_kind':None,'_admission':None,'_last_elapsed':None})
  n=e.get('elapsed_monotonic_secs')
  if isinstance(n,(int,float)) and 0<=n<100000:r['_last_elapsed']=round(n,2)
  if stage=='admitted':
   size=e.get('request_bytes');r['bytes']=size if isinstance(size,int) and 0<=size<16_000_001 else None
   admission=e.get('monotonic_admission');r['_admission']=admission if isinstance(admission,(int,float)) else None
   deadline=e.get('monotonic_deadline')
   if isinstance(deadline,(int,float)) and r['_admission'] is not None:r['deadline_s']=round(deadline-r['_admission'],1)
  elif stage=='upstream_response':
   code=e.get('status');r['status']=code if isinstance(code,int) and 100<=code<=599 else None;r['phase']='response'
  elif stage=='watchdog_heartbeat':
   r['heartbeat_count']+=1;r['heartbeat_elapsed_s']=r['_last_elapsed']
  elif stage.startswith('settled_'):
   outcome=e.get('outcome');r['settlement']='deadline reserve' if outcome=='upstream_total_deadline' else ('usage' if stage=='settled_usage' else 'reserve')
   r['phase']='settled';r['latency_s']=r['_last_elapsed']
  elif stage=='upstream_error':
   r['phase']='upstream error';kind=e.get('error');r['error_kind']=kind if kind in ('TimeoutError','RemoteDisconnected','HTTPError','ConnectionResetError','BrokenPipeError','IncompleteRead') else 'other'
  elif stage=='upstream_http_error':r['phase']='upstream HTTP error'
  elif stage=='client_disconnected':r['phase']='client disconnected'
  elif stage=='admission_denied':r['phase']='admission denied'

if A.ledger:
 ledger=json.loads(A.ledger.read_text())
 for adj in ledger.get('adjustments',[]):
  if not isinstance(adj,dict):continue
  rid=adj.get('request_id');label=adj.get('label','')
  if not ID.fullmatch(str(rid)) or not isinstance(label,str) or 'owner-directed stop for SSE proxy repair' not in label:continue
  assert rid in rows and rows[rid]['settlement'] is None and adj.get('micros')==771860
  rows[rid]['settlement']='manual reserve';rows[rid]['phase']='settled';rows[rid]['latency_s']=rows[rid]['_last_elapsed']
now=time.monotonic();calls=[]
for r in rows.values():
 if r['_admission'] is not None and r['settlement'] is None:r['latency_s']=round(max(0,now-r['_admission']),2)
 r.pop('_last_elapsed');r.pop('_admission');calls.append(r)
# Insertion order follows the proxy's admission order. Never include raw messages.
records=[]
for path in sorted(A.runs.glob('*/*/record.json')):
 try:v=json.loads(path.read_text())
 except (OSError,ValueError):continue
 cid=v.get('instance_id');h=v.get('harness')
 if not CASE.fullmatch(str(cid)) or h not in ('HAIRSPRING','OpenHands'):continue
 state=v.get('state');state=state if state in ('started','graded','unfinished','grade_failed') else 'unknown'
 steps=v.get('steps');duration=v.get('wall_secs');grade=v.get('score',{})
 records.append({'id':cid,'harness':h,'state':state,
  'steps':steps if isinstance(steps,int) and 0<=steps<100000 else None,
  'wall_s':round(duration,1) if isinstance(duration,(int,float)) and 0<=duration<1e8 else None,
  'grade':'pass' if state=='graded' and grade.get('resolved') is True else ('fail' if state=='graded' else 'pending')})
payload={'schema':1,'generated_at':datetime.datetime.now(datetime.timezone.utc).isoformat(timespec='seconds'),
 'source':'HAIRSPRING paired diagnostic run; published snapshots, not direct provider telemetry',
 'clock_ratio':calibration,'calls':calls,'tasks':records,
 'summary':{'calls':len(calls),'settled':sum(bool(r['settlement']) for r in calls),
  'open':sum(not r['settlement'] for r in calls),
  'deadline_settlements':sum(r['settlement']=='deadline reserve' for r in calls),
  'graded':sum(r['state']=='graded' for r in records)}}
A.output.parent.mkdir(parents=True,exist_ok=True)
temp=A.output.with_suffix('.tmp');temp.write_text(json.dumps(payload,separators=(',',':'),ensure_ascii=False)+'\n');temp.replace(A.output)
print(f"snapshot: {len(calls)} calls, {len(records)} tasks, {payload['summary']['open']} open")
