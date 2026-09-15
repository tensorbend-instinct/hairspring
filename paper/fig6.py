from pathlib import Path
import csv, json
import matplotlib.pyplot as plt
from matplotlib.patches import FancyBboxPatch, Rectangle
from matplotlib import rcParams
rcParams.update({'font.family':'DejaVu Sans','figure.facecolor':'white','pdf.fonttype':42})
out=Path(__file__).parent/'figures'
INK='#23252A'; MUTED='#5E646E'; BLUE='#6383A8'; BF='#EAF1F7'; GREEN='#789B7A'; GF='#EEF5EC'; ORANGE='#CC9863'; OF='#FBF1E7'; RED='#B96F67'; RF='#F8ECEA'; LINE='#AEB4BC'; DGREEN='#4F7350'; DARKRED='#8E4A44'
base=Path(__file__).parent/'evidence'
hs=[]
with open(base/'hairspring-swe-live.csv') as f:
    for row in csv.reader(f):
        if not row or not row[0].strip(): continue
        hs.append(dict(inst=row[0],passed=row[1]=='True',steps=int(row[2]),wall=int(row[5]),cost=int(row[4])/1e6,note=row[6] if len(row)>6 else ''))
sw=json.load(open(base/'swe-agent-swe-live.json'))
swm={d['instance_id']:d for d in sw}
tasks=[h['inst'] for h in hs]
short=lambda s: s.split('__')[1].replace('-','-')
fig=plt.figure(figsize=(7.35,4.6))
gs=fig.add_gridspec(2,1,height_ratios=[1.05,1],hspace=.55,left=.06,right=.98,top=.86,bottom=.10)
ax=fig.add_subplot(gs[0]); ax.set(xlim=(0,1),ylim=(0,1)); ax.axis('off')
ax.text(0,1.12,'Matched ten-task software-engineering result, same model family, host-side grading',weight='bold',fontsize=9.2,color=INK,transform=ax.transAxes)
n=len(tasks); cw=.78/n
ax.text(.02,.82,'Hairspring',fontsize=7.2,weight='bold',color=INK)
ax.text(.02,.38,'class-C1\nbaseline',fontsize=7.2,weight='bold',color=INK,linespacing=1.2)
for i,t in enumerate(tasks):
    x=.12+i*cw
    h=hs[i]; d=swm.get(t)
    hp=h['passed']; sp=d['passed'] if d else False
    ax.add_patch(Rectangle((x,.68),cw*.88,.22,fc=GF if hp else RF,ec=DGREEN if hp else RED,lw=1.0))
    ax.text(x+cw*.44,.79,'%d st'%h['steps'],ha='center',va='center',fontsize=5.4,color=INK)
    ax.add_patch(Rectangle((x,.24),cw*.88,.22,fc=GF if sp else RF,ec=DGREEN if sp else RED,lw=1.0))
    ax.text(x+cw*.44,.35,'%d st'%(d['steps'] if d else 0),ha='center',va='center',fontsize=5.4,color=INK)
    ax.text(x+cw*.44,.10,short(t),ha='center',va='center',fontsize=5.4,color=MUTED,rotation=28)
ax.text(.12+9.5*cw,.79,'',fontsize=5)
ax.text(.995,.79,'9 / 10',ha='right',fontsize=8.0,weight='bold',color=DGREEN)
ax.text(.995,.35,'3 / 10',ha='right',fontsize=8.0,weight='bold',color=DARKRED)
ax.text(.02,-.06,'cell = one task, host-side verdict; number = worker steps. The baseline submitted on most tasks; its submissions failed the host tests.',fontsize=6.2,color=MUTED)
ax2=fig.add_subplot(gs[1]); ax2.axis('off'); ax2.set(xlim=(0,1),ylim=(0,1))
hs_steps=sum(h['steps'] for h in hs); sw_steps=sum(d['steps'] for d in sw)
hs_wall=sum(h['wall'] for h in hs); sw_wall=sum(d['wall_secs'] for d in sw)
hs_cost=sum(h['cost'] for h in hs); sw_cost=sum(d['litellm_cost'] for d in sw)
metrics=[('worker steps',hs_steps,sw_steps,'fewer steps: repair is directed by evidence'),
         ('wall time (s)',hs_wall,sw_wall,'verification is expensive'),
         ('cost (USD)',hs_cost,sw_cost,'the critic + repair loop is the price')]
bw=.30
ax2.text(0,1.06,'The trade, in aggregate',weight='bold',fontsize=8.6,color=INK,transform=ax2.transAxes)
for i,(name,hv,sv,note) in enumerate(metrics):
    y=.78-i*.30
    ax2.text(.02,y+.10,name,fontsize=7.0,weight='bold',color=INK)
    mx=max(hv,sv)
    ax2.add_patch(Rectangle((.24,y),bw*hv/mx,.075,fc=BLUE,ec='none'))
    ax2.text(.245+bw*hv/mx,y+.037,'Hairspring %s'%(f'{hv:,.0f}' if hv>100 else f'{hv:.2f}'),fontsize=6.2,color=INK,va='center')
    ax2.add_patch(Rectangle((.24,y-.095),bw*sv/mx,.075,fc=LINE,ec='none'))
    ax2.text(.245+bw*sv/mx,y-.058,'baseline %s'%(f'{sv:,.0f}' if sv>100 else f'{sv:.2f}'),fontsize=6.2,color=MUTED,va='center')
    ax2.text(.70,y+.01,note,fontsize=6.4,color=DARKRED,style='italic',va='center')
fig.savefig(out/'fig-comparison.pdf',bbox_inches='tight'); fig.savefig(out/'fig-comparison.png',dpi=200,bbox_inches='tight'); plt.close(fig)
print('ok fig6',hs_steps,sw_steps,hs_wall,sw_wall,round(hs_cost,2),round(sw_cost,2))
