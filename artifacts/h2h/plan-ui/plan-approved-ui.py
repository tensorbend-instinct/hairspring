import os,pty,fcntl,termios,struct,subprocess,time,select,pyte
from PIL import Image,ImageDraw,ImageFont
master,slave=pty.openpty();fcntl.ioctl(slave,termios.TIOCSWINSZ,struct.pack('HHHH',36,120,0,0))
env=os.environ.copy();env.update(TERM='xterm-256color',HS_TUI='on',HS_SEQMODEL_SCRIPT='/tmp/hs/artifacts/h2h/plan-ui/review-script.jsonl')
p=subprocess.Popen(['/tmp/hs/target/debug/hs-repl','--config','/tmp/hs/artifacts/h2h/plan-ui/rig.toml','--dir','/tmp/hs/artifacts/h2h/plan-ui/review-approved-final-run','--project-dir','/tmp/hs/artifacts/h2h/plan-ui/proj','--max-steps','2'],stdin=slave,stdout=slave,stderr=slave,env=env);os.close(slave)
screen=pyte.Screen(120,36);stream=pyte.Stream(screen);raw=b'';start=time.time();sent=False; paused=False; shown=False
while time.time()-start<7:
 if not sent and time.time()-start>.7:os.write(master,b'/plan Repair parser\r');sent=True
 if sent and not paused and time.time()-start>2.0:os.write(master,b'Approve\r');paused=True
 if select.select([master],[],[],.1)[0]:
  try:b=os.read(master,65536)
  except OSError:break
  raw+=b;stream.feed(b.decode(errors='replace'))
font=ImageFont.truetype('/usr/share/fonts/truetype/dejavu/DejaVuSansMono.ttf',15);im=Image.new('RGB',(1200,720),'#101010');d=ImageDraw.Draw(im)
for y in range(36):
 for x in range(120):
  c=screen.buffer[y][x];color='#ddd' if c.fg=='default' else c.fg
  try:d.text((x*10,y*20),c.data,font=font,fill=color)
  except ValueError:d.text((x*10,y*20),c.data,font=font,fill='#ddd')
im.save('/downloads/plan-approved.png');open('/tmp/hs/artifacts/h2h/plan-ui/terminal.txt','w').write('\n'.join(screen.display));open('/tmp/hs/artifacts/h2h/plan-ui/terminal.raw','wb').write(raw)
os.write(master,b'/quit\r');p.wait(timeout=10);os.close(master)



assert any("Plan approved" in x for x in screen.display),screen.display
