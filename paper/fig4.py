from pathlib import Path
import matplotlib.pyplot as plt
from matplotlib.patches import FancyBboxPatch, FancyArrowPatch
from matplotlib import rcParams
rcParams.update({'font.family':'DejaVu Sans','figure.facecolor':'white','pdf.fonttype':42})
out=Path(__file__).parent/'figures'
INK='#23252A'; MUTED='#5E646E'; BLUE='#6383A8'; BF='#EAF1F7'; GREEN='#789B7A'; GF='#EEF5EC'; ORANGE='#CC9863'; OF='#FBF1E7'; RED='#B96F67'; RF='#F8ECEA'; LINE='#AEB4BC'; DGREEN='#4F7350'; DARKRED='#8E4A44'
def evbox(ax,x,y,w,h,t,fill,edge,fs=6.0):
    ax.add_patch(FancyBboxPatch((x,y),w,h,boxstyle='round,pad=0.003,rounding_size=.004',fc=fill,ec=edge,lw=.9))
    ax.text(x+w/2,y+h/2,t,ha='center',va='center',fontsize=fs,color=INK,weight='bold')
def arr(ax,a,b,color=INK,ls='-',rad=0,lw=1.1,ms=8,label=None,lo=(0,0),fs=6.4):
    ax.add_patch(FancyArrowPatch(a,b,arrowstyle='-|>',mutation_scale=ms,color=color,lw=lw,linestyle=ls,connectionstyle=f'arc3,rad={rad}'))
    if label: ax.text((a[0]+b[0])/2+lo[0],(a[1]+b[1])/2+lo[1],label,color=DGREEN,fontsize=fs,ha='center',va='center',bbox=dict(fc='white',ec='none',pad=.7))
fig,ax=plt.subplots(figsize=(7.35,4.1)); ax.set(xlim=(0,1),ylim=(0,1)); ax.axis('off')
ax.text(.005,.975,'Recovery is replay, not recollection',weight='bold',fontsize=9.6,color=INK)
ax.text(.005,.945,'A crash mid-mission. The record - not the model - reconstructs what happened, and the same mission resumes with its evidence intact.',fontsize=7.0,color=MUTED)
evs=[('mission\nopened',BF,BLUE),('model\ncall 1',BF,BLUE),('tool\nresult',GF,GREEN),('model\ncall 2',BF,BLUE),('edit\napplied',GF,GREEN),('check\nfailed',OF,ORANGE),('repair\nstep',BF,BLUE)]
n=len(evs); w=.098; gap=(.86-n*w)/(n-1); y=.66
for i,(t,f,e) in enumerate(evs):
    x=.02+i*(w+gap); evbox(ax,x,y,w,.115,t,f,e)
    if i<n-1: arr(ax,(x+w,y+.058),(x+w+gap,y+.058),LINE,lw=.9,ms=6)
# torn tail
xt=.02+n*(w+gap)+.01
ax.add_patch(FancyBboxPatch((xt,y),.075,.115,boxstyle='round,pad=0.003',fc=RF,ec=RED,lw=.9,ls='--'))
ax.text(xt+.0375,y+.058,'partial\nwrite',ha='center',va='center',fontsize=5.8,color=DARKRED)
# crash mark
ax.plot([.955,.97],[.83,.79],color=DARKRED,lw=2); ax.plot([.97,.96],[.79,.75],color=DARKRED,lw=2); ax.plot([.96,.985],[.75,.72],color=DARKRED,lw=2)
ax.text(.965,.865,'CRASH',fontsize=7.5,color=DARKRED,weight='bold',ha='center')
ax.text(.02,.625,'every request, effect, and verdict is already a canonical, hash-linked event - large payloads live in a content-addressed store',fontsize=6.4,color=MUTED)
# replay arrow down
arr(ax,(.5,.645),(.5,.50),DGREEN,lw=1.3)
ax.text(.5,.575,'replay reads the consistent prefix; the torn tail is detected and discarded',color=DGREEN,fontsize=6.6,ha='center',va='center',bbox=dict(fc='white',ec='none',pad=.7))
# restored state panel
ax.add_patch(FancyBboxPatch((.02,.28),.44,.185,boxstyle='round,pad=0.005,rounding_size=.006',fc=GF,ec=DGREEN,lw=1.0))
ax.text(.04,.435,'replay restores',weight='bold',fontsize=7.0,color=DGREEN)
for j,t in enumerate(['full transcript + tool results','step, refusal, and cost counters','billed spend to the cent','registered tools and config']):
    ax.text(.04,.402-j*.034,'\u2022 '+t,fontsize=6.4,color=INK)
ax.add_patch(FancyBboxPatch((.52,.28),.46,.185,boxstyle='round,pad=0.005,rounding_size=.006',fc='white',ec=LINE,lw=1.0))
ax.text(.54,.435,'what does not happen',weight='bold',fontsize=7.0,color=DARKRED)
for j,t in enumerate(['no new mission with a fresh identity','no model-written summary of the past','no lost child-agent work or ledger entries','no reset budgets']):
    ax.text(.54,.402-j*.034,'\u00d7 '+t,fontsize=6.4,color=INK)
# resume row
ax.text(.02,.245,'the same mission continues',weight='bold',fontsize=7.2,color=INK)
evs2=[('repair\nstep n',BF,BLUE),('check\npassed',GF,GREEN),('critic:\nno finding',GF,GREEN),('mission closed:\nverified',GF,GREEN)]
n2=len(evs2); w2=.115; gap2=.03
for i,(t,f,e) in enumerate(evs2):
    x=.02+i*(w2+gap2); evbox(ax,x,.085,w2,.105,t,f,e,6.2)
    if i<n2-1: arr(ax,(x+w2,.138),(x+w2+gap2,.138),DGREEN,lw=1.0,ms=7)
WASH='#F5F5F3'
ax.add_patch(FancyBboxPatch((.64,.075),.345,.125,boxstyle='round,pad=0.004,rounding_size=.006',fc=WASH,ec=LINE,lw=.9))
ax.text(.8125,.138,'a resumed mission carries its transcript,\ncounters, ledger, and cost - continuity\nis a property of the record,\nnot the prompt',ha='center',va='center',fontsize=5.8,color=MUTED,linespacing=1.3)
fig.savefig(out/'fig-recovery.pdf',bbox_inches='tight'); fig.savefig(out/'fig-recovery.png',dpi=200,bbox_inches='tight'); plt.close(fig)
print('ok fig4')
