"""Paired SWE-bench Verified protocol. Do not plot results before graded pairs exist."""
from pathlib import Path
import json
import matplotlib.pyplot as plt
from matplotlib.patches import FancyBboxPatch, FancyArrowPatch, Rectangle
from matplotlib import rcParams
rcParams.update({'font.family':'DejaVu Sans','figure.facecolor':'white','pdf.fonttype':42})
base=Path(__file__).parent
manifest=json.loads((base.parent/'benchmarks/paired15/manifest.json').read_text())
ids=manifest['ids']
assert len(ids)==15 and len(set(ids))==15
out=base/'figures';out.mkdir(exist_ok=True)
ink='#23252A'; muted='#5E646E'; blue='#6383A8'; wash='#EAF1F7'; green='#789B7A'; gold='#CC9863'
fig,ax=plt.subplots(figsize=(7.35,5.2));ax.set(xlim=(0,1),ylim=(0,1));ax.axis('off')
ax.text(.03,.96,'Paired SWE-bench Verified evaluation: protocol and pending outcomes',weight='bold',fontsize=10,color=ink)
ax.text(.03,.914,'15 locked instances  |  DeepSeek V4.1 Flash for both harnesses  |  one compute instance, one run at a time',fontsize=7.4,color=muted)
def box(x,y,w,h,title,detail,edge=blue,face=wash):
 ax.add_patch(FancyBboxPatch((x,y),w,h,boxstyle='round,pad=.008',fc=face,ec=edge,lw=1))
 ax.text(x+w/2,y+h*.68,title,ha='center',va='center',weight='bold',fontsize=8,color=ink)
 ax.text(x+w/2,y+h*.32,detail,ha='center',va='center',fontsize=6.7,color=muted)
box(.03,.73,.25,.13,'Locked sample','dataset + base/test digests')
box(.38,.73,.25,.13,'Two native harnesses','HAIRSPRING / OpenHands')
box(.73,.73,.24,.13,'Independent grader','pinned tests, fail closed',green,'#EEF5EC')
for start,end in [(.28,.38),(.63,.73)]:ax.add_patch(FancyArrowPatch((start,.795),(end,.795),arrowstyle='-|>',mutation_scale=11,color=muted,lw=1))
ax.text(.03,.655,'Outcome matrix',fontsize=8.4,color=ink,weight='bold')
ax.text(.03,.62,'Each column is one locked task. Pale cells are pending, not failures or solves.',fontsize=7.2,color=muted)
for row,(label,y) in enumerate([('HAIRSPRING',.47),('OpenHands',.34)]):
 ax.text(.03,y+.043,label,fontsize=7.3,va='center',color=ink,weight='bold')
 for i,_ in enumerate(ids):
  x=.24+i*.048
  ax.add_patch(Rectangle((x,y),.039,.085,fc='#F4F4F1',ec='#BBC0C4',lw=.75))
  ax.text(x+.0195,y+.043,'?',ha='center',va='center',color=muted,fontsize=8)
ax.text(.24,.267,'15 paired tasks; no completed paid pair at this revision',fontsize=7.6,color=muted)
ax.add_patch(FancyBboxPatch((.03,.08),.94,.13,boxstyle='round,pad=.008',fc='#FBF1E7',ec=gold,lw=1))
ax.text(.05,.164,'Publish only verified comparisons',weight='bold',color=ink,fontsize=8)
ax.text(.05,.112,'Correctly solved = terminal finish + passing native grade; report steps and wall time.\nUnfinished is not a solve.',color=ink,fontsize=6.9,linespacing=1.45,va='center')
fig.savefig(out/'fig-comparison.pdf',bbox_inches='tight');fig.savefig(out/'fig-comparison.png',dpi=200,bbox_inches='tight');plt.close(fig)
print('fig-comparison: protocol only, 15 pending pairs')
