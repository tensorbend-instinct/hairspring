import sys,json
n=0
for line in sys.stdin:
 try:v=json.loads(line)
 except:continue
 method=v.get('method');p=v.get('params',{});result={}
 if method=='describe':result={'name':'wrapup-fixture','kind':'model','version':'0.1.0'}
 elif method=='preflight':result={'ready':True}
 elif method=='model.call':
  if n==0:c={'tool':'get_goal','args':{}}
  else:
   goal=None
   for m in p.get('messages',[]):
    if m.get('role')=='tool':
     try:x=json.loads(m.get('content',''))
     except:continue
     if isinstance(x,dict) and isinstance(x.get('goal'),dict):goal=x['goal']
   if n==1 and goal:c={'tool':'update_goal','args':{'goal_id':goal['id'],'revision':goal['revision'],'action':'complete'}}
   else:c={'completion':'Fixture marks the goal complete. No successful checker verdict was produced.','reasoning':''}
  n+=1;result={'completion':json.dumps(c),'input_tokens':0,'output_tokens':0,'cost_usd_micros':0}
 else:result={'$error':'unexpected method'}
 print(json.dumps({'id':v['id'],'result':result}),flush=True)
