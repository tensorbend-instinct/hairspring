from pathlib import Path
import textwrap, math
import matplotlib.pyplot as plt
from matplotlib.patches import FancyBboxPatch, FancyArrowPatch, Circle, Ellipse, Rectangle
from matplotlib import rcParams
rcParams.update({'font.family':'DejaVu Sans','figure.facecolor':'white','pdf.fonttype':42})
out=Path(__file__).parent/'figures'; out.mkdir(exist_ok=True)
INK='#23252A'; MUTED='#5E646E'; BLUE='#6383A8'; BF='#EAF1F7'; GREEN='#789B7A'; GF='#EEF5EC'; ORANGE='#CC9863'; OF='#FBF1E7'; RED='#B96F67'; RF='#F8ECEA'; LINE='#AEB4BC'; WASH='#F5F5F3'; DARKRED='#8E4A44'
def arr(ax,a,b,color=INK,ls='-',rad=0,lw=1.0,ms=7):
    ax.add_patch(FancyArrowPatch(a,b,arrowstyle='-|>',mutation_scale=ms,color=color,lw=lw,linestyle=ls,connectionstyle=f'arc3,rad={rad}'))
def wrap(s,n): return '\n'.join(textwrap.fill(p,n) for p in s.split('\n'))

fig,ax=plt.subplots(figsize=(7.35,6.9)); ax.set(xlim=(0,1),ylim=(0,1)); ax.axis('off')
ax.text(.005,.985,'Five ways agent harnesses are built today, and the authority each leaves inside the model',weight='bold',fontsize=9.2,color=INK)
ax.text(.005,.960,'Each class gets work done. Each keeps some of five authorities (close, record, share, remember, improve) in the model that does the work.',fontsize=7.0,color=MUTED)

cols=[
 ('C1. Direct loop','one model in a\nthink-act-observe loop;\nit submits when it\nsays it is done',
  ['close','record','share','remember','improve'],None,
  'The model grades its\nown work. Fluent failures\ncross as passes.'),
 ('C2. Static\nworkflow graph','human-authored DAG of\nLLM steps; edges fixed\nbefore the run begins',
  ['close','record'],'share, remember, improve:\nno mechanism at all',
  'Off-script states have no\nroute. The judging node is\nthe same model kind.'),
 ('C3. Role\nensemble','copies of one model\ndebate, vote, or play\nroles',
  ['close','share','record'],'remember, improve:\nno mechanism at all',
  'Correlated errors vote\ntogether. Consensus is\nnot correctness.'),
 ('C4. Search and\nevolution','tree search or prompt\nevolution scored by the\nmodel itself',
  ['close','improve'],'record, share, remember:\nno mechanism at all',
  'Search optimizes the\nmodel\'s own confidence.\nCost grows per branch.'),
 ('C5. Memory-\naugmented','retrieval store or\nsummaries re-injected\ninto the prompt',
  ['remember','close'],'record, share, improve:\nno mechanism at all',
  'Unvalidated writes persist.\nRetrieval relevance is\nnot correctness.'),
]
cw=.192; gap=.010; x0=.004; top=.935; ch=.665
for i,(name,shape,auths,absent,fail) in enumerate(cols):
    x=x0+i*(cw+gap)
    ax.add_patch(FancyBboxPatch((x,top-ch),cw,ch,boxstyle='round,pad=0.005,rounding_size=.006',fc='white',ec=LINE,lw=1.0))
    ax.add_patch(FancyBboxPatch((x,top-.062),cw,.062,boxstyle='round,pad=0.005,rounding_size=.006',fc=WASH,ec=LINE,lw=1.0))
    ax.text(x+cw/2,top-.031,name,ha='center',va='center',weight='bold',fontsize=7.6,color=INK,linespacing=1.15)
    ax.text(x+cw/2,top-.108,shape,ha='center',va='center',fontsize=5.9,color=MUTED,linespacing=1.3)
    sy=top-.225
    if i==0:
        ax.add_patch(Circle((x+cw*.30,sy),.032,fc=BF,ec=BLUE,lw=1)); ax.text(x+cw*.30,sy,'M',ha='center',va='center',fontsize=7,weight='bold',color=BLUE)
        ax.add_patch(Circle((x+cw*.70,sy),.032,fc=GF,ec=GREEN,lw=1)); ax.text(x+cw*.70,sy,'T',ha='center',va='center',fontsize=7,weight='bold',color=GREEN)
        arr(ax,(x+cw*.37,sy+.024),(x+cw*.63,sy+.024),LINE,rad=-.4,lw=.9); arr(ax,(x+cw*.63,sy-.024),(x+cw*.37,sy-.024),LINE,rad=-.4,lw=.9)
    elif i==1:
        for bx,by in [(x+cw*.18,sy+.032),(x+cw*.52,sy+.032),(x+cw*.86,sy+.032),(x+cw*.52,sy-.048)]:
            ax.add_patch(FancyBboxPatch((bx-.05,by-.014),.10,.028,boxstyle='round,pad=0.002,rounding_size=.004',fc=BF,ec=BLUE,lw=.9))
        arr(ax,(x+cw*.23,sy+.032),(x+cw*.47,sy+.032),LINE,lw=.8,ms=5); arr(ax,(x+cw*.57,sy+.032),(x+cw*.81,sy+.032),LINE,lw=.8,ms=5)
        arr(ax,(x+cw*.52,sy+.018),(x+cw*.52,sy-.034),LINE,lw=.8,ms=5)
    elif i==2:
        for ang in [90,210,330]:
            cx=x+cw/2+.040*math.cos(math.radians(ang)); cy=sy+.030*math.sin(math.radians(ang))
            ax.add_patch(Circle((cx,cy),.024,fc=OF,ec=ORANGE,lw=1)); ax.text(cx,cy,'M',ha='center',va='center',fontsize=6,weight='bold',color=ORANGE)
        arr(ax,(x+cw/2+.016,sy+.042),(x+cw/2-.040,sy-.006),ORANGE,rad=.3,lw=.7,ms=5)
        arr(ax,(x+cw/2-.046,sy+.006),(x+cw/2+.046,sy+.006),ORANGE,rad=.3,lw=.7,ms=5)
        arr(ax,(x+cw/2+.040,sy-.008),(x+cw/2-.012,sy+.042),ORANGE,rad=.3,lw=.7,ms=5)
    elif i==3:
        ax.add_patch(Circle((x+cw*.5,sy+.048),.019,fc=BF,ec=BLUE,lw=1))
        for dx in [-0.055,0,0.055]:
            ax.add_patch(Circle((x+cw*.5+dx,sy-.002),.016,fc=BF,ec=BLUE,lw=.9))
            arr(ax,(x+cw*.5,sy+.030),(x+cw*.5+dx,sy+.013),LINE,lw=.7,ms=4)
        for dx2 in [-0.028,0.028]:
            ax.add_patch(Circle((x+cw*.5+dx2,sy-.048),.013,fc=RF,ec=RED,lw=.9))
            arr(ax,(x+cw*.5,sy-.017),(x+cw*.5+dx2,sy-.036),LINE,lw=.7,ms=4)
    else:
        ax.add_patch(Rectangle((x+cw*.22-.040,sy-.028),.080,.068,fc=GF,ec='none'))
        ax.add_patch(Ellipse((x+cw*.22,sy+.040),.080,.022,fc=GF,ec=GREEN,lw=1))
        ax.add_patch(Ellipse((x+cw*.22,sy-.028),.080,.022,fc=GF,ec=GREEN,lw=1))
        ax.plot([x+cw*.22-.040,x+cw*.22-.040],[sy+.040,sy-.028],color=GREEN,lw=1)
        ax.plot([x+cw*.22+.040,x+cw*.22+.040],[sy+.040,sy-.028],color=GREEN,lw=1)
        ax.add_patch(Circle((x+cw*.66,sy+.006),.032,fc=BF,ec=BLUE,lw=1)); ax.text(x+cw*.66,sy+.006,'M',ha='center',va='center',fontsize=7,weight='bold',color=BLUE)
        arr(ax,(x+cw*.28,sy+.006),(x+cw*.62,sy+.006),GREEN,lw=1.0)
    ax.text(x+cw/2,top-.345,'kept in-model',fontsize=6.0,color=DARKRED,weight='bold',ha='center')
    ay=top-.378
    for a in auths:
        ax.add_patch(FancyBboxPatch((x+.012,ay-.026),cw-.024,.026,boxstyle='round,pad=0.002,rounding_size=.008',fc=RF,ec=RED,lw=.7))
        ax.text(x+cw/2,ay-.013,a,ha='center',va='center',color=INK,fontsize=5.9)
        ay-=.032
    if absent: ax.text(x+.010,ay-.004,absent,fontsize=5.5,color=MUTED,style='italic',linespacing=1.25,va='top')
    ax.text(x+.010,top-ch+.070,'where it breaks',fontsize=6.0,color=DARKRED,weight='bold')
    ax.text(x+.010,top-ch+.004,fail,fontsize=5.7,color=INK,va='bottom',linespacing=1.35)
band=wrap('In every class the component that produces the work also holds authority over it: it declares the result, owns the history, writes shared state, decides what persists, or promotes its own changes. Model confidence substitutes for execution truth, and every added safeguard is one more opinion from the same source.',118)
ax.add_patch(FancyBboxPatch((.004,.070),.992,.148,boxstyle='round,pad=0.005,rounding_size=.006',fc=RF,ec=RED,lw=1.1))
ax.text(.018,.196,'Shared root cause',weight='bold',fontsize=8.0,color=DARKRED)
ax.text(.018,.098,band,fontsize=6.9,color=INK,linespacing=1.5,va='bottom')
ax.text(.004,.038,'This paper moves the five authorities out of the model and into a replayable substrate. Figure 2 shows the resulting pipeline.',fontsize=7.4,color=DARKRED,weight='bold')
ax.text(.004,.012,'Class evidence is cited in the bibliography; the argument here is about shapes, not products.',fontsize=6.2,color=MUTED,style='italic')
fig.savefig(out/'fig-classes.pdf',bbox_inches='tight'); fig.savefig(out/'fig-classes.png',dpi=200,bbox_inches='tight'); plt.close(fig)
print('ok')
