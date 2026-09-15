from pathlib import Path
import matplotlib.pyplot as plt
from matplotlib.patches import FancyBboxPatch, FancyArrowPatch, Circle
from matplotlib import rcParams
rcParams.update({'font.family':'DejaVu Sans','font.size':8,'figure.facecolor':'white','pdf.fonttype':42})
out=Path(__file__).parent/'figures'; out.mkdir(exist_ok=True)
INK='#23252A'; MUTED='#5E646E'; BLUE='#6383A8'; BF='#EAF1F7'; GREEN='#789B7A'; GF='#EEF5EC'; ORANGE='#CC9863'; OF='#FBF1E7'; RED='#B96F67'; RF='#F8ECEA'; LINE='#AEB4BC'; WASH='#F5F5F3'
def box(ax,x,y,w,h,title,sub='',edge=BLUE,fill=BF,fs=8,align='center'):
 p=FancyBboxPatch((x,y),w,h,boxstyle='round,pad=0.010,rounding_size=.012',fc=fill,ec=edge,lw=1.0); ax.add_patch(p)
 ha='center' if align=='center' else 'left'; tx=x+w/2 if align=='center' else x+.018
 ax.text(tx,y+h*.63,title,ha=ha,va='center',weight='bold',color=INK,fontsize=fs)
 if sub: ax.text(tx,y+h*.30,sub,ha=ha,va='center',color=MUTED,fontsize=fs-1,linespacing=1.25)
def arr(ax,a,b,color=INK,ls='-',rad=0,lw=1.0,label=None,lo=(0,0)):
 ax.add_patch(FancyArrowPatch(a,b,arrowstyle='-|>',mutation_scale=8,color=color,lw=lw,linestyle=ls,connectionstyle=f'arc3,rad={rad}'))
 if label: ax.text((a[0]+b[0])/2+lo[0],(a[1]+b[1])/2+lo[1],label,color=color,fontsize=6.4,ha='center',va='center',bbox=dict(fc='white',ec='none',pad=.8))
# Figure 1: architecture, Colosseum-like two panel
fig,ax=plt.subplots(figsize=(7.35,5.15)); ax.set(xlim=(0,1),ylim=(0,1)); ax.axis('off')
ax.text(.01,.975,'(a) Mission workflow',weight='bold',fontsize=9,color=INK)
steps=[(.015,'Goal + project','Defines the requested artifact'),(.205,'Worker loop','Calls tools and revises'),(.395,'Declared checks','Execute in judged workspace'),(.585,'Fresh critic','Attempts a grounded refutation'),(.775,'Recorded outcome','Verified, repaired, or stopped')]
cols=[(BLUE,BF),(BLUE,BF),(ORANGE,OF),(RED,RF),(GREEN,GF)]
for (x,t,s),(e,f) in zip(steps,cols): box(ax,x,.765,.165,.12,t,s,e,f,7.5)
for x in [.18,.37,.56,.75]: arr(ax,(x,.825),(x+.025,.825),LINE)
arr(ax,(.665,.75),(.49,.685),RED,'--',.12,label='blocking finding',lo=(0,.018)); arr(ax,(.49,.685),(.285,.75),RED,'--',.12)
arr(ax,(.475,.75),(.285,.705),ORANGE,'--',.12,label='failed check',lo=(0,.015))
ax.text(.01,.64,'(b) The same record feeds five authority planes',weight='bold',fontsize=9,color=INK)
box(ax,.34,.49,.30,.10,'Canonical event record','Typed events · one writer · hash chain · replay',BLUE,BF,8)
planes=[(.01,.23,'Recovery','Restores transcript, counters, tools, and cost',BLUE,BF),(.205,.23,'World','Admits validated effects and shared artifacts',GREEN,GF),(.40,.23,'Memory','Stores cited records and reuse signals',ORANGE,OF),(.595,.23,'Accounting','Books model, tool, and child-agent work',BLUE,BF),(.79,.23,'Evolution','Compares candidates under a pinned scorer',RED,RF)]
for x,y,t,s,e,f in planes:
 box(ax,x,y,.18,.12,t,s,e,f,7.2); arr(ax,(.49,.48),(x+.09,y+.125),LINE,'--',0)
ax.text(.01,.125,'(c) Authority boundaries',weight='bold',fontsize=9,color=INK)
labels=[('Model','proposes work'),('Checker','closes missions'),('World','admits shared state'),('Scorer','promotes policy')]
for i,(t,s) in enumerate(labels):
 x=.02+i*.245; box(ax,x,.015,.215,.075,t,s,[BLUE,ORANGE,GREEN,RED][i],[BF,OF,GF,RF][i],7.6)
 if i<3: ax.text(x+.226,.052,'≠',fontsize=12,color=MUTED,ha='center',va='center')
fig.tight_layout(pad=.2); fig.savefig(out/'architecture.pdf',bbox_inches='tight'); fig.savefig(out/'architecture.png',dpi=260,bbox_inches='tight'); plt.close(fig)
# Figure 2 event anatomy
fig,ax=plt.subplots(figsize=(7.25,3.9)); ax.set(xlim=(0,1),ylim=(0,1)); ax.axis('off')
ax.text(.01,.96,'A mission is reconstructed from events, not from model memory',weight='bold',fontsize=9.3,color=INK)
for i,(t,s,e,f) in enumerate([('1  Request','typed tool call',BLUE,BF),('2  Effect','result + artifact hash',ORANGE,OF),('3  Evidence','check or critic finding',RED,RF),('4  Verdict','state transition',GREEN,GF)]):
 x=.02+i*.245; box(ax,x,.70,.205,.12,t,s,e,f,7.8)
 if i<3: arr(ax,(x+.205,.76),(x+.245,.76),LINE)
box(ax,.12,.43,.76,.11,'Append-only stream','canonical bytes  →  content-addressed payloads  →  segmented fsync  →  consistent prefix after a torn tail',BLUE,BF,7.8)
outs=[(.03,'Resume','continue the same mission'),(.275,'Inspect','render trace and evidence'),(.52,'Distill','extract reusable records'),(.765,'Score','attribute policy changes')]
for x,t,s in outs:
 box(ax,x,.16,.205,.11,t,s,RED if t=='Score' else BLUE,RF if t=='Score' else WASH,7.5)
 arr(ax,(.5,.42),(x+.102,.275),LINE,'--')
ax.text(.5,.055,'Provider swaps, steering, child work, and billed cost remain infrastructure facts in the same history.',ha='center',color=MUTED,fontsize=7.2)
fig.tight_layout(pad=.2); fig.savefig(out/'record.pdf',bbox_inches='tight'); fig.savefig(out/'record.png',dpi=260,bbox_inches='tight'); plt.close(fig)
# Figure 3 benchmark
fig,axs=plt.subplots(1,3,figsize=(7.25,2.55)); data=[([9,3],'Tasks resolved','of 10'),([987,1170],'Agent steps','count'),([6.484234,.9036486],'Reported cost','USD')]
for ax,(vals,title,unit) in zip(axs,data):
 ax.bar([0,1],vals,color=[BLUE,LINE],width=.55); ax.set_xticks([0,1],['HAIRSPRING','SWE-agent'],fontsize=7); ax.set_title(title,fontsize=8.5,weight='bold',color=INK); ax.spines[['top','right','left']].set_visible(False); ax.tick_params(axis='y',labelsize=6,length=0); ax.grid(axis='y',alpha=.15)
 for i,v in enumerate(vals): ax.text(i,v+max(vals)*.04,f'{v:g}',ha='center',fontsize=7.2,weight='bold',color=INK)
fig.suptitle('Matched ten-task SWE-bench-Live subset',fontsize=9.5,weight='bold',color=INK,y=1.02)
fig.tight_layout(); fig.savefig(out/'benchmark.pdf',bbox_inches='tight'); fig.savefig(out/'benchmark.png',dpi=260,bbox_inches='tight'); plt.close(fig)
# Figure 4 evaluation layers
fig,ax=plt.subplots(figsize=(7.25,3.65)); ax.set(xlim=(0,1),ylim=(0,1)); ax.axis('off')
ax.text(.01,.95,'Evidence strengthens as it moves outward from code to outcome',weight='bold',fontsize=9.2,color=INK)
layers=[(.07,.70,.86,.13,'1  Source and unit tests','Does the mechanism exist, and do local invariants hold?',BLUE,BF),(.13,.51,.74,.13,'2  Installed-path seam tests','Can a model reach it through the real registry, sandbox, TUI, and checker?',ORANGE,OF),(.20,.32,.60,.13,'3  Live artifact trials','Does a real run produce the requested artifact under independent judgment?',GREEN,GF),(.29,.13,.42,.13,'4  Matched external evaluation','Does the harness change outcomes against an incumbent under a shared judge?',RED,RF)]
for x,y,w,h,t,s,e,f in layers: box(ax,x,y,w,h,t,s,e,f,8,align='left')
fig.tight_layout(pad=.2); fig.savefig(out/'evidence-layers.pdf',bbox_inches='tight'); fig.savefig(out/'evidence-layers.png',dpi=260,bbox_inches='tight'); plt.close(fig)
