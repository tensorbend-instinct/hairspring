import runpy, pathlib
d = pathlib.Path(__file__).parent
for f in ['fig1.py','fig2.py','fig3.py','fig4.py','fig5.py','fig6.py']:
    runpy.run_path(str(d/f))
print('all figures built')
