import os,pty,fcntl,termios,struct,subprocess,time,select,pyte
from PIL import Image,ImageDraw,ImageFont
master,slave=pty.openpty();fcntl.ioctl(slave,termios.TIOCSWINSZ,struct.pack('HHHH',36,120,0,0))
env=os.environ.copy();env.update(TERM='xterm-256color',HS_TUI='on',HS_SEQMODEL_SCRIPT='/tmp/hs/artifacts/h2h/skill-ui/script.jsonl')
p=subprocess.Popen(['/tmp/hs/target/debug/hs-repl','--config','/tmp/hs/artifacts/h2h/skill-ui/rig.toml','--dir','/tmp/hs/artifacts/h2h/skill-ui/echo-green2-run','--project-dir','/tmp/hs/artifacts/h2h/skill-ui/proj','--max-steps','1'],stdin=slave,stdout=slave,stderr=slave,env=env);os.close(slave)
screen=pyte.Screen(120,36);stream=pyte.Stream(screen);raw=b'';start=time.time();sent=False; paused=False; shown=False
while time.time()-start<6:
 if not sent and time.time()-start>.7:os.write(master,b'Use /parser instructions\r');sent=True
 
 
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
im.save('/downloads/echo-ui-green.png');open('/tmp/hs/artifacts/h2h/skill-ui/direct-terminal.txt','w').write('\n'.join(screen.display));open('/tmp/hs/artifacts/h2h/skill-ui/direct-terminal.raw','wb').write(raw)
os.write(master,b'\x03');p.wait(timeout=10);os.close(master)

assert screen.display[1].strip("│ ")=="› Use /parser instructions", repr(screen.display[1])
