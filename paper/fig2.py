from pathlib import Path
import matplotlib.pyplot as plt
from matplotlib.patches import FancyBboxPatch, FancyArrowPatch
from matplotlib import rcParams
rcParams.update({'font.family':'DejaVu Sans','figure.facecolor':'white','pdf.fonttype':42})
out=Path(__file__).parent/'figures'; out.mkdir(exist_ok=True)
INK='#23252A'; MUTED='#5E646E'; BLUE='#6383A8'; BF='#EAF1F7'; GREEN='#789B7A'; GF='#EEF5EC'; ORANGE='#CC9863'; OF='#FBF1E7'; RED='#B96F67'; RF='#F8ECEA'; LINE='#AEB4BC'; WASH='#F5F5F3'; DARKRED='#8E4A44'; DGREEN='#4F7350'
def box(ax,x,y,w,h,title,sub='',edge=BLUE,fill=BF,fs=7.6,ec_lw=1.0):
    ax.add_patch(FancyBboxPatch((x,y),w,h,boxstyle='round,pad=0.006,rounding_size=.008',fc=fill,ec=edge,lw=ec_lw))
    if sub:
        ax.text(x+w/2,y+h*.68,title,ha='center',va='center',weight='bold',color=INK,fontsize=fs)
        ax.text(x+w/2,y+h*.30,sub,ha='center',va='center',color=MUTED,fontsize=fs-1.2,linespacing=1.25)
    else:
        ax.text(x+w/2,y+h/2,title,ha='center',va='center',weight='bold',color=INK,fontsize=fs)
def arr(ax,a,b,color=INK,ls='-',rad=0,lw=1.1,label=None,lo=(0,0),fs=6.2,ms=8):
    ax.add_patch(FancyArrowPatch(a,b,arrowstyle='-|>',mutation_scale=ms,color=color,lw=lw,linestyle=ls,connectionstyle=f'arc3,rad={rad}'))
    if label: ax.text((a[0]+b[0])/2+lo[0],(a[1]+b[1])/2+lo[1],label,color=color,fontsize=fs,ha='center',va='center',bbox=dict(fc='white',ec='none',pad=.7))

fig,ax=plt.subplots(figsize=(7.35,5.0)); ax.set(xlim=(0,1),ylim=(0,1)); ax.axis('off')
ax.text(.005,.985,'The Hairspring pipeline: the model proposes, the substrate decides',weight='bold',fontsize=9.4,color=INK)
# zones
ax.add_patch(FancyBboxPatch((.004,.615),.992,.325,boxstyle='round,pad=0.004',fc='#F7FAFD',ec=BLUE,lw=1.0,ls='--'))
ax.text(.015,.918,'INSIDE THE WORKING CONTEXT - the model is free to search, edit, call tools, delegate, revise',fontsize=6.8,color=BLUE,weight='bold')
ax.add_patch(FancyBboxPatch((.004,.335),.992,.245,boxstyle='round,pad=0.004',fc='#F6FAF5',ec=DGREEN,lw=1.0,ls='--'))
ax.text(.015,.555,'OUTSIDE THE MODEL - the substrate holds the five authorities',fontsize=6.8,color=DGREEN,weight='bold')
# top zone: model side
box(ax,.02,.70,.150,.13,'Goal + checks','root, tools, checks;\noptional limits',BLUE,BF,7.2)
box(ax,.205,.70,.185,.13,'Worker loop','typed tools; calls\nrecorded as events',BLUE,BF,6.7)
box(ax,.425,.70,.190,.13,'Submission','artifact + checks\nthat define success',BLUE,BF,6.7)
box(ax,.650,.70,.160,.13,'Children','depth + concurrency\nguards; optional caps',BLUE,BF,6.4)
arr(ax,(.170,.765),(.205,.765),LINE); arr(ax,(.390,.765),(.425,.765),LINE)
arr(ax,(.520,.70),(.135,.505),BLUE,'-',.12,label='submits',lo=(.03,.05))
# bottom zone: substrate side
box(ax,.040,.370,.180,.13,'Executable checks','restored workspace;\nnot model opinion',ORANGE,OF,6.5)
box(ax,.260,.370,.180,.13,'Fresh review','goal + artifact;\ncan block, not override',RED,RF,6.5)
box(ax,.480,.370,.190,.13,'Canonical record','single writer; hash-linked\nreplayable events',GREEN,GF,6.5)
box(ax,.710,.370,.260,.13,'Verdict','verified / rejected / stopped;\nnot a model self-grade',GREEN,GF,6.5)
arr(ax,(.220,.435),(.260,.435),LINE); arr(ax,(.440,.435),(.480,.435),LINE); arr(ax,(.670,.435),(.710,.435),LINE)
# feedback to worker
arr(ax,(.130,.370),(.260,.70),RED,'--',.18)
ax.text(.185,.605,'failure evidence\nreturns for repair',color=RED,fontsize=5.7,ha='center',va='center',bbox=dict(fc='white',ec='none',pad=.7))
# planes strip
ax.text(.005,.291,'The same record feeds five authority planes',weight='bold',fontsize=7.5,color=INK)
planes=[('Log','close + record','one append writer;\ntorn-tail recovery',BLUE,BF),
        ('World','share','validated proposals only;\nreuse is observable',GREEN,GF),
        ('Memory','remember','provenance + reuse signal;\nrelevance is not truth',ORANGE,OF),
        ('Accounting','budget','calls, tools, children,\nbilled cost',BLUE,BF),
        ('Evolution','improve','pinned scorer, held-out\nassays, rollback',RED,RF)]
pw=.192
for i,(t,a,s,e,f) in enumerate(planes):
    x=.004+i*(pw+.010)
    ax.add_patch(FancyBboxPatch((x,.085),pw,.170,boxstyle='round,pad=0.005,rounding_size=.006',fc=f,ec=e,lw=1.0))
    ax.text(x+pw/2,.225,t,ha='center',weight='bold',fontsize=7.4,color=INK)
    ax.text(x+pw/2,.196,'decides: '+a,ha='center',fontsize=6.0,color=DARKRED,weight='bold')
    ax.text(x+pw/2,.135,s,ha='center',fontsize=5.9,color=MUTED,linespacing=1.3)
    arr(ax,(.575,.370),(x+pw/2,.258),LINE,'--',0,lw=.7,ms=5)
ax.text(.004,.054,'A mission closes only after checks pass and review finds no blocker. Evidence and steps',fontsize=6.5,color=MUTED)
ax.text(.004,.027,'land in one replayable record read by recovery, shared state, memory, accounting, and evolution.',fontsize=6.5,color=MUTED)
fig.savefig(out/'fig-pipeline.pdf',bbox_inches='tight'); fig.savefig(out/'fig-pipeline.png',dpi=200,bbox_inches='tight'); plt.close(fig)
print('ok fig2')
