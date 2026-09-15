#!/bin/sh
set -eu
pandoc hairspring.tex -f latex -t html5 -s --css paper.css --metadata title='HAIRSPRING: Authority outside the model' -o hairspring.raw.html
python3 postprocess.py hairspring.raw.html hairspring.html
wkhtmltopdf --enable-local-file-access --page-size Letter --margin-top 18mm --margin-bottom 18mm --margin-left 18mm --margin-right 18mm --footer-center '[page]' --footer-font-size 8 hairspring.html hairspring.full.pdf
mv hairspring.full.pdf hairspring.pdf
