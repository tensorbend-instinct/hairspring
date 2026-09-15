from pathlib import Path
import matplotlib.pyplot as plt
from matplotlib.patches import FancyBboxPatch, FancyArrowPatch
from matplotlib import rcParams
rcParams.update({'font.family':'DejaVu Sans','figure.facecolor':'white','pdf.fonttype':42})
out=Path(__file__).parent/'figures'
INK='#23252A'; MUTED='#5E646E'; BLUE='#6383A8'; BF='#EAF1F7'; GREEN='#789B7A'; GF='#EEF5EC'; ORANGE='#CC9863'; OF='#FBF1E7'; RED='#B96F67'; RF='#F8ECEA'; LINE='#AEB4BC'; DGREEN='#4F7350'; DARKRED='#8E4A44'
def arr(ax,a,b,color=INK,ls='-',rad=0,lw=1.1,ms=9):
    ax.add_patch(FancyArrowPatch(a,b,arrowstyle='-|>',mutation_scale=ms,color=color,lw=lw,linestyle=ls,connectionstyle=f'arc3,rad={rad}'))
fig,ax=plt.subplots(figsize=(7.35,4.4)); ax.set(xlim=(0,1),ylim=(0,1)); ax.axis('off')
ax.text(.005,.975,'The authority boundary',weight='bold',fontsize=9.6,color=INK)
ax.text(.005,.945,'Five kinds of decision cross from the model into mechanisms the model cannot influence. Evidence returns; authority does not.',fontsize=7.0,color=MUTED)
# left zone
ax.add_patch(FancyBboxPatch((.005,.10),.40,.79,boxstyle='round,pad=0.004',fc='#F7FAFD',ec=BLUE,lw=1.0,ls='--'))
ax.text(.02,.855,'THE MODEL MAY PROPOSE',fontsize=7.4,color=BLUE,weight='bold')
# right zone
ax.add_patch(FancyBboxPatch((.595,.10),.40,.79,boxstyle='round,pad=0.004',fc='#F6FAF5',ec=DGREEN,lw=1.0,ls='--'))
ax.text(.61,.855,'ONLY THE SUBSTRATE MAY DECIDE',fontsize=7.4,color=DGREEN,weight='bold')
# dashed boundary
ax.plot([.5,.5],[.10,.90],ls=':',color=LINE,lw=1.4)
ax.text(.5,.075,'authority boundary',fontsize=6.4,color=MUTED,ha='center',style='italic')
rows=[
 ('plans, edits, tool calls,\ndelegation, repairs','close','checks + critic close the mission;\nthe model cannot pass itself',ORANGE,OF),
 ('an artifact plus the checks\nthat would prove it','record','one writer appends canonical,\nhash-linked events',BLUE,BF),
 ('candidate world effects\nfrom child agents','share','the world service validates;\nproposals are not state',GREEN,GF),
 ('candidate memories with\ntheir origin mission','remember','memory keeps provenance and a\nreuse signal, never truth-by-vote',ORANGE,OF),
 ('candidate policies with\nsupporting evidence','improve','a pinned scorer promotes only\nafter held-out trials',RED,RF),
]
y0=.78; dy=.135
for i,(l,a,r,e,f) in enumerate(rows):
    y=y0-i*dy
    ax.add_patch(FancyBboxPatch((.02,y-.10),.36,.105,boxstyle='round,pad=0.004,rounding_size=.006',fc='white',ec=BLUE,lw=.9))
    ax.text(.20,y-.048,l,ha='center',va='center',fontsize=6.6,color=INK,linespacing=1.25)
    arr(ax,(.39,y-.048),(.61,y-.048),e,lw=1.4)
    ax.text(.5,y-.022,a,ha='center',va='center',fontsize=6.6,weight='bold',color=DARKRED,bbox=dict(fc='white',ec='none',pad=.6))
    ax.add_patch(FancyBboxPatch((.62,y-.10),.36,.105,boxstyle='round,pad=0.004,rounding_size=.006',fc=f,ec=e,lw=.9))
    ax.text(.80,y-.048,r,ha='center',va='center',fontsize=6.6,color=INK,linespacing=1.25)
    arr(ax,(.61,y-.125),(.39,y-.125),MUTED,'--',lw=.7,ms=6)
ax.text(.5,.005,'thin arrows: failed checks, grounded findings, verdicts, and scores return to the model as evidence - they carry no decision power back',fontsize=6.2,color=MUTED,ha='center',style='italic')
fig.savefig(out/'fig-authority.pdf',bbox_inches='tight'); fig.savefig(out/'fig-authority.png',dpi=200,bbox_inches='tight'); plt.close(fig)
print('ok fig3')
