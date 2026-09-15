from pathlib import Path
import matplotlib.pyplot as plt
from matplotlib.patches import FancyBboxPatch, FancyArrowPatch
from matplotlib import rcParams
rcParams.update({'font.family':'DejaVu Sans','figure.facecolor':'white','pdf.fonttype':42})
out=Path(__file__).parent/'figures'
INK='#23252A'; MUTED='#5E646E'; BLUE='#6383A8'; BF='#EAF1F7'; GREEN='#789B7A'; GF='#EEF5EC'; ORANGE='#CC9863'; OF='#FBF1E7'; RED='#B96F67'; RF='#F8ECEA'; LINE='#AEB4BC'; DGREEN='#4F7350'; DARKRED='#8E4A44'
def box(ax,x,y,w,h,title,sub='',edge=BLUE,fill=BF,fs=7.4):
    ax.add_patch(FancyBboxPatch((x,y),w,h,boxstyle='round,pad=0.005,rounding_size=.007',fc=fill,ec=edge,lw=1.0))
    if sub:
        ax.text(x+w/2,y+h*.70,title,ha='center',va='center',weight='bold',color=INK,fontsize=fs)
        ax.text(x+w/2,y+h*.32,sub,ha='center',va='center',color=MUTED,fontsize=fs-1.3,linespacing=1.25)
    else:
        ax.text(x+w/2,y+h/2,title,ha='center',va='center',weight='bold',color=INK,fontsize=fs)
def arr(ax,a,b,color=INK,ls='-',rad=0,lw=1.1,ms=8,label=None,lo=(0,0),fs=6.2):
    ax.add_patch(FancyArrowPatch(a,b,arrowstyle='-|>',mutation_scale=ms,color=color,lw=lw,linestyle=ls,connectionstyle=f'arc3,rad={rad}'))
    if label: ax.text((a[0]+b[0])/2+lo[0],(a[1]+b[1])/2+lo[1],label,color=color,fontsize=fs,ha='center',va='center',bbox=dict(fc='white',ec='none',pad=.7))
fig,ax=plt.subplots(figsize=(7.35,4.3)); ax.set(xlim=(0,1),ylim=(0,1)); ax.axis('off')
ax.text(.005,.975,'The improvement loop: a candidate must beat its parent under a scorer pinned before it existed',weight='bold',fontsize=9.0,color=INK)
ax.text(.005,.945,'The inner loop repairs the task. This outer loop changes the policy that drives the worker - and no candidate grades itself.',fontsize=7.0,color=MUTED)
box(ax,.02,.72,.24,.14,'Proposer model','reads prior policy, traces,\nscores, reflections, frontier',BLUE,BF,7.0)
box(ax,.36,.72,.22,.14,'Candidate policy','prompt + policy artifacts;\nnever Rust code',ORANGE,OF,7.0)
box(ax,.68,.72,.30,.14,'Held-out assay','candidate vs parent on tasks\nneither side selected',GREEN,GF,7.0)
arr(ax,(.26,.79),(.36,.79),LINE); arr(ax,(.58,.79),(.68,.79),LINE)
box(ax,.36,.40,.28,.15,'Pinned scorer','fixed before mutation;\ncanaries detect drift;\ninvalid evidence scores zero',RED,RF,7.0)
arr(ax,(.83,.72),(.58,.48),DGREEN,lw=1.2,label='evidence',lo=(.10,.02))
box(ax,.02,.40,.24,.15,'Decision record','candidate + parent hashes,\nassay conditions, scores,\ndecision, reason',BLUE,BF,7.0)
arr(ax,(.36,.475),(.26,.475),LINE)
box(ax,.68,.40,.30,.15,'Promote or roll back','winner loads through the live\npath; loser is rewound;\nlineage is published',GREEN,GF,7.0)
ax.plot([.83,.83],[.395,.24],ls='--',color=MUTED,lw=.9)
ax.plot([.83,.012],[.24,.24],ls='--',color=MUTED,lw=.9)
ax.plot([.012,.012],[.24,.79],ls='--',color=MUTED,lw=.9)
ax.add_patch(FancyArrowPatch((.012,.79),(.015,.79),arrowstyle='-|>',mutation_scale=8,color=MUTED,lw=.9,linestyle='--'))
ax.text(.42,.252,'next cycle: a promoted policy becomes the new parent',color=MUTED,fontsize=6.2,ha='center',va='center',style='italic',bbox=dict(fc='white',ec='none',pad=.7))
# observed results strip
ax.add_patch(FancyBboxPatch((.02,.05),.96,.16,boxstyle='round,pad=0.005,rounding_size=.006',fc=GF,ec=DGREEN,lw=1.0))
ax.text(.04,.175,'what every promotion leaves behind (no live-promotion rate is claimed)',weight='bold',fontsize=6.8,color=DGREEN)
ax.text(.04,.105,'decision record: candidate + parent hashes, assay conditions, traces, scores, decision, reason',fontsize=6.8,color=INK)
ax.text(.04,.075,'a malformed or fabricated verdict scores zero and is rewound; gate tests exercise the full path',fontsize=6.8,color=INK)
fig.savefig(out/'fig-improve.pdf',bbox_inches='tight'); fig.savefig(out/'fig-improve.png',dpi=200,bbox_inches='tight'); plt.close(fig)
print('ok fig5')
