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
#3 boundary
f,a=setup(4.0,'Who decides what: model suggestions versus software decisions')
left=[('Finish','"I think the task is done"'),('History','summary of what happened'),('Shared state','a proposed fact or artifact'),('Memory','a note worth keeping'),('Improvement','a proposed prompt or policy')]
right=[('Pass or fail','tests + fresh review'),('Official record','single append-only event writer'),('Accepted shared state','validator'),('Stored memory','source + later-use signal'),('Promoted change','fixed scorer + held-out tasks')]
for i in range(5):
 y=76-i*14; box(a,2,y,38,10,*left[i],BF,BLUE); box(a,60,y,38,10,*right[i],GF,GREEN); arrow(a,(41,y+5),(59,y+5),'proposal')
a.text(21,88,'THE MODEL MAY PROPOSE',ha='center',weight='bold',color=BLUE); a.text(79,88,'SOFTWARE MAKES THE LAST CALL',ha='center',weight='bold',color=GREEN)
a.text(50,4,'The model can suggest each outcome. A separate mechanism decides what becomes official.',ha='center',fontsize=7,color=MUT)
save(f,'fig-decisions')
