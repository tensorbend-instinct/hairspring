from pathlib import Path
import csv,json,textwrap
import matplotlib.pyplot as plt
from matplotlib.patches import FancyBboxPatch,FancyArrowPatch,Rectangle
from matplotlib import rcParams
rcParams.update({'font.family':'DejaVu Sans','font.size':8,'figure.facecolor':'white','pdf.fonttype':42})
O=Path(__file__).parent/'figures'; O.mkdir(exist_ok=True)
INK='#20242a'; MUT='#5b6570'; BLUE='#4f78a5'; BF='#edf3f8'; GREEN='#56855a'; GF='#eef6ef'; RED='#ae554f'; RF='#faefee'; GOLD='#a96e2f'; OF='#fbf3e8'; LINE='#aab2bb'
def setup(h=4.2,title=''):
 f,a=plt.subplots(figsize=(7.05,h)); a.set(xlim=(0,100),ylim=(0,100)); a.axis('off'); a.text(1,96,title,fontsize=11,weight='bold',color=INK,va='top'); return f,a
def box(a,x,y,w,h,t,s='',fc=BF,ec=BLUE):
 a.add_patch(FancyBboxPatch((x,y),w,h,boxstyle='round,pad=.7,rounding_size=1.5',fc=fc,ec=ec,lw=1.1))
 a.text(x+w/2,y+h*(.64 if s else .5),t,ha='center',va='center',weight='bold',fontsize=8,color=INK)
 if s:a.text(x+w/2,y+h*.29,s,ha='center',va='center',fontsize=6.6,color=MUT,linespacing=1.25)
def arrow(a,p,q,label='',c=LINE):
 a.add_patch(FancyArrowPatch(p,q,arrowstyle='-|>',mutation_scale=9,color=c,lw=1.1));
 if label:a.text((p[0]+q[0])/2,(p[1]+q[1])/2+2,label,ha='center',fontsize=6.5,color=MUT,bbox=dict(fc='white',ec='none',pad=.5))
def save(f,n):
 f.subplots_adjust(left=.025,right=.975,top=.96,bottom=.05); f.savefig(O/(n+'.pdf')); f.savefig(O/(n+'.png'),dpi=180); plt.close(f)
# 1 classes
f,a=setup(4.8,'Five common agent designs, and the failure each leaves open')
rows=[('Direct loop','One model works and decides it is done.','A confident wrong answer can pass.'),('Fixed workflow','A preset graph routes work through model steps.','A new situation can have no valid route.'),('Model team','Models plan, work, debate, and vote.','Shared blind spots can win the vote.'),('Search or evolution','Many candidates are scored and selected.','A model-written score can reward the wrong thing.'),('Memory add-on','Past notes are retrieved into later prompts.','A wrong note can become a lasting fact.')]
for i,(n,d,r) in enumerate(rows):
 y=78-i*14; box(a,2,y,20,10,n,'',BF,BLUE); a.text(25,y+5,d,va='center',fontsize=7.2,color=INK); a.text(67,y+5,'\n'.join(textwrap.wrap(r,38)),va='center',fontsize=6.9,color=RED,linespacing=1.2)
a.text(25,87,'WHAT IT DOES',weight='bold',fontsize=7,color=MUT); a.text(67,87,'WHAT CAN STILL GO WRONG',weight='bold',fontsize=7,color=MUT)
a.add_patch(FancyBboxPatch((2,3),96,10,boxstyle='round,pad=.7',fc=RF,ec=RED)); a.text(50,8,'Shared problem: the model can still approve, preserve, or spread its own mistake.',ha='center',va='center',weight='bold',fontsize=8.2,color=RED)
save(f,'fig-classes')
#2 pipeline
f,a=setup(4.35,'Hairspring in one page: the model proposes; software checks and records')
box(a,2,63,15,18,'1. Goal','task, files, checks,\ntime and cost limits'); box(a,22,63,16,18,'2. Model works','reads, edits, runs tools,\nasks child agents'); box(a,43,63,16,18,'3. Model submits','artifact plus the checks\nthat should prove it'); box(a,64,63,15,18,'4. Software checks','restored workspace,\nexecutable tests',OF,GOLD); box(a,84,63,14,18,'5. Fresh review','looks only for a\nblocking defect',RF,RED)
for x in [17,38,59,79]: arrow(a,(x,72),(x+5,72))
arrow(a,(91,62),(31,50),'failure evidence returns for repair',RED)
box(a,18,28,64,15,'One durable run record','goal, model calls, tool results, file hashes, checks, review, cost, and final outcome',GF,GREEN)
for x in [9,30,51,71,91]: arrow(a,(x,63),(50,43))
a.text(50,15,'The mission ends only when the checks pass and the fresh review finds no blocking defect.',ha='center',weight='bold',fontsize=8.5,color=GREEN)
a.text(50,8,'A model can suggest success. It cannot write the final pass record.',ha='center',fontsize=7.5,color=MUT)
save(f,'fig-pipeline')
#3 boundary
f,a=setup(4.0,'Who decides what: model suggestions versus software decisions')
left=[('Finish','"I think the task is done"'),('History','summary of what happened'),('Shared state','a proposed fact or artifact'),('Memory','a note worth keeping'),('Improvement','a proposed prompt or policy')]
right=[('Pass or fail','tests + fresh review'),('Official record','single append-only event writer'),('Accepted shared state','validator'),('Stored memory','source + later-use signal'),('Promoted change','fixed scorer + held-out tasks')]
for i in range(5):
 y=76-i*14; box(a,2,y,38,10,*left[i],BF,BLUE); box(a,60,y,38,10,*right[i],GF,GREEN); arrow(a,(41,y+5),(59,y+5),'proposal')
a.text(21,88,'THE MODEL MAY PROPOSE',ha='center',weight='bold',color=BLUE); a.text(79,88,'SOFTWARE MAKES THE LAST CALL',ha='center',weight='bold',color=GREEN)
a.text(50,4,'The model can suggest each outcome. A separate mechanism decides what becomes official.',ha='center',fontsize=7,color=MUT)
save(f,'fig-authority')
#4 recovery
f,a=setup(3.75,'Crash recovery: resume the same run, not a reconstructed imitation')
steps=[('Before crash','goal\nmodel replies\ntool calls'),('Durable prefix','ordered events\nfile hashes\nspend + limits'),('Crash','unfinished tail\nis detected',RF,RED),('Replay','restore transcript\ntool state\ncounters + cost',GF,GREEN),('Continue','same run id\nsame limits\nnext valid step',GF,GREEN)]
for i,item in enumerate(steps):
 x=2+i*20; fc=item[2] if len(item)>2 else BF; ec=item[3] if len(item)>3 else BLUE; box(a,x,51,16,25,item[0],item[1],fc,ec)
 if i<4:arrow(a,(x+16,63),(x+20,63))
a.text(50,30,'Only a complete, consistent prefix is replayed. A partial final event is discarded.',ha='center',fontsize=8,color=INK)
a.text(50,18,'Recovered state includes the evidence and spending history, so a restart cannot erase a failure or reset a budget.',ha='center',fontsize=7.3,color=MUT)
save(f,'fig-recovery')
#5 improve
f,a=setup(3.9,'Safe improvement: a candidate cannot grade or install itself')
box(a,2,57,17,20,'1. Propose','new prompt or policy\nfrom prior traces'); box(a,23,57,17,20,'2. Freeze the judge','scorer is pinned before\nthe candidate runs',OF,GOLD); box(a,44,57,17,20,'3. Compare','parent and candidate run\non held-out tasks'); box(a,65,57,15,20,'4. Decide','fixed scorer reads\nrecorded evidence',GF,GREEN); box(a,83,57,15,20,'5. Keep or undo','store hashes;\nundo failed change',GF,GREEN)
for x,w in [(19,4),(40,4),(61,4),(80,3)]:arrow(a,(x,67),(x+w,67))
a.text(50,35,'Recorded for every decision',ha='center',weight='bold',fontsize=8,color=INK)
a.text(50,26,'parent hash  •  candidate hash  •  tasks  •  scorer version  •  traces  •  scores  •  reason',ha='center',fontsize=7.3,color=MUT)
a.text(50,12,'Current limit: Hairspring can test prompt and policy changes. It does not yet rewrite and promote its own Rust code.',ha='center',fontsize=7.3,color=RED)
save(f,'fig-improve')
#6 benchmark
base=Path(__file__).parent/'evidence'; hs=[]
with open(base/'hairspring-swe-live.csv') as z:
 for r in csv.reader(z):
  if r and r[0].strip(): hs.append({'id':r[0].split('__')[-1],'ok':r[1]=='True','steps':int(r[2]),'wall':int(r[5]),'cost':int(r[4])/1e6})
sw=json.load(open(base/'swe-agent-swe-live.json')); sm={r['instance_id'].split('__')[-1]:r for r in sw}
f,a=setup(4.55,'Measured result on one matched set of ten software-engineering tasks')
a.text(2,86,'Green = host-side tests passed. Red = failed or timed out. This is a curated subset, not a general leaderboard.',fontsize=7.2,color=MUT)
for row,(name,y) in enumerate([('Hairspring',65),('Direct-loop baseline',43)]):
 a.text(2,y+5,name,va='center',weight='bold',fontsize=7.4)
 for i,h in enumerate(hs):
  x=18+i*7.2; ok=h['ok'] if row==0 else sm[h['id']]['passed']; st=h['steps'] if row==0 else sm[h['id']]['steps']
  a.add_patch(Rectangle((x,y),5.9,10,fc=GF if ok else RF,ec=GREEN if ok else RED)); a.text(x+2.95,y+5,str(st),ha='center',va='center',fontsize=6.2)
  if row==1:a.text(x+2.95,39,h['id'],ha='right',rotation=32,fontsize=5.2,color=MUT)
a.text(98,70,'9/10',ha='right',weight='bold',color=GREEN,fontsize=9); a.text(98,48,'3/10',ha='right',weight='bold',color=RED,fontsize=9)
H=(sum(x['steps'] for x in hs),sum(x['wall'] for x in hs),sum(x['cost'] for x in hs)); S=(sum(x['steps'] for x in sw),sum(x['wall_secs'] for x in sw),sum(x['litellm_cost'] for x in sw))
a.text(2,20,f'Worker steps: {H[0]:,} vs {S[0]:,}',fontsize=7.5); a.text(35,20,f'Wall time: {H[1]:,} s vs {S[1]:,} s',fontsize=7.5); a.text(70,20,f'Cost: \${H[2]:.2f} vs \${S[2]:.2f}',fontsize=7.5)
a.text(50,8,'Result: Hairspring recovered six more tasks, but took more time and cost more. The data does not isolate which subsystem caused the gain.',ha='center',fontsize=7,color=MUT)
save(f,'fig-comparison')
print('generated six bounded figures')
