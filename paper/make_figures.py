from pathlib import Path
import matplotlib.pyplot as plt
from matplotlib.patches import FancyBboxPatch, FancyArrowPatch
from matplotlib import rcParams
rcParams.update({'font.family':'DejaVu Sans','font.size':8,'figure.facecolor':'white'})
out=Path(__file__).parent/'figures'; out.mkdir(exist_ok=True)
blue='#3B67FF'; bluefill='#E8F0FF'; green='#4C8B5B'; greenfill='#ECF7EE'; orange='#F0802B'; orangefill='#FFF1E4'; red='#FF5A3D'; redfill='#FFF0ED'; ink='#16213E'; gray='#667085'
def box(ax,xy,w,h,title,sub,edge=blue,fill=bluefill):
 x,y=xy; p=FancyBboxPatch((x,y),w,h,boxstyle='round,pad=0.012,rounding_size=.018',fc=fill,ec=edge,lw=1.25); ax.add_patch(p)
 ax.text(x+w/2,y+h*.62,title,ha='center',va='center',weight='bold',color=ink,fontsize=8.3)
 ax.text(x+w/2,y+h*.28,sub,ha='center',va='center',color=gray,fontsize=6.8,wrap=True)
def arrow(ax,a,b,color=blue,style='-|>',lw=1.2,ls='-'):
 ax.add_patch(FancyArrowPatch(a,b,arrowstyle=style,mutation_scale=9,color=color,lw=lw,linestyle=ls,connectionstyle='arc3'))
# Fig 1
fig,ax=plt.subplots(figsize=(7.1,4.0)); ax.set(xlim=(0,1),ylim=(0,1)); ax.axis('off')
ax.text(.02,.95,'(a) Mission control flow',weight='bold',color=ink,fontsize=9)
xs=[.03,.22,.41,.60,.79]; titles=['Mission','Model loop','Executable check','Independent critic','Recorded verdict']; subs=['goal + project','think · act · observe','runs declared checks','tries to refute','verified or rejected']; edges=[blue,blue,orange,red,green]; fills=[bluefill,bluefill,orangefill,redfill,greenfill]
for x,t,s,e,f in zip(xs,titles,subs,edges,fills): box(ax,(x,.66),.15,.16,t,s,e,f)
for x in xs[:-1]: arrow(ax,(x+.15,.74),(x+.19,.74),ink)
arrow(ax,(.675,.65),(.485,.55),red,ls='--'); arrow(ax,(.485,.55),(.295,.65),red,ls='--'); ax.text(.485,.52,'findings return to the working loop',ha='center',color=red,fontsize=6.8)
ax.text(.02,.42,'(b) Authority and state planes',weight='bold',color=ink,fontsize=9)
box(ax,(.05,.14),.20,.14,'Append-only log','single writer · replay',blue,bluefill); box(ax,(.30,.14),.18,.14,'World','validated proposals',green,greenfill); box(ax,(.53,.14),.18,.14,'Memory','cited reuse earns credit',orange,orangefill); box(ax,(.76,.14),.18,.14,'Scorer','held-out promotion',red,redfill)
for x in [.15,.39,.62,.85]: arrow(ax,(x,.30),(x,.62),gray,ls='--')
ax.text(.50,.04,'The model proposes work. Mechanisms outside the model decide what becomes state, a pass, or an improvement.',ha='center',color=ink,fontsize=7.2)
fig.tight_layout(pad=.3); fig.savefig(out/'architecture.pdf',bbox_inches='tight'); fig.savefig(out/'architecture.png',dpi=260,bbox_inches='tight'); plt.close(fig)
# Fig 2
fig,ax=plt.subplots(figsize=(7.1,3.7)); ax.set(xlim=(0,1),ylim=(0,1)); ax.axis('off')
ax.text(.02,.95,'The same event record drives recovery, inspection, memory, and promotion',weight='bold',color=ink,fontsize=9)
box(ax,(.04,.66),.18,.16,'Tool effect','request + result',blue,bluefill); box(ax,(.30,.66),.18,.16,'Canonical event','typed + hash-linked',orange,orangefill); box(ax,(.56,.66),.18,.16,'Durable stream','segmented + fsynced',green,greenfill)
arrow(ax,(.22,.74),(.30,.74),ink); arrow(ax,(.48,.74),(.56,.74),ink)
outs=[(.05,'Replay','restore from record'),(.28,'Inspect','trace live behavior'),(.51,'Learn','distill cited memory'),(.74,'Evolve','score candidates')]
for x,t,s in outs: box(ax,(x,.25),.18,.16,t,s,red if t=='Evolve' else blue,redfill if t=='Evolve' else bluefill); arrow(ax,(.65,.65),(x+.09,.42),gray,ls='--')
ax.text(.50,.08,'A crash does not require the model to remember. A promotion does not require the model to grade itself.',ha='center',color=ink,fontsize=7.2)
fig.tight_layout(pad=.3); fig.savefig(out/'record.pdf',bbox_inches='tight'); fig.savefig(out/'record.png',dpi=260,bbox_inches='tight'); plt.close(fig)
# Fig 3 benchmark
fig,axs=plt.subplots(1,3,figsize=(7.1,2.75));
vals=[(90,30),(987,1170),(6.484234,.9036486)]; labs=[('Resolved','%'),('Steps','count'),('Reported cost','USD')]
for ax,(a,b),(title,unit) in zip(axs,vals,labs):
 ax.bar([0,1],[a,b],color=[blue,gray],width=.58); ax.set_xticks([0,1],['HAIRSPRING','SWE-agent'],fontsize=7); ax.set_title(title,weight='bold',fontsize=8.5,color=ink); ax.spines[['top','right','left']].set_visible(False); ax.tick_params(axis='y',labelsize=6); ax.grid(axis='y',alpha=.18)
 for i,v in enumerate([a,b]): ax.text(i,v+(max(a,b)*.035),f'{v:g}{"%" if unit=="%" else ""}',ha='center',fontsize=7,weight='bold',color=ink)
fig.suptitle('Matched 10-task SWE-bench-Live subset · same model · host-side grading',fontsize=9.5,weight='bold',color=ink,y=1.03)
fig.tight_layout(); fig.savefig(out/'benchmark.pdf',bbox_inches='tight'); fig.savefig(out/'benchmark.png',dpi=260,bbox_inches='tight'); plt.close(fig)
