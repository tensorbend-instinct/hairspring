#!/bin/sh
set -eu
if command -v tectonic >/dev/null 2>&1; then
  tectonic hairspring.tex --keep-logs
elif command -v pdflatex >/dev/null 2>&1; then
  pdflatex -interaction=nonstopmode -halt-on-error hairspring.tex
  pdflatex -interaction=nonstopmode -halt-on-error hairspring.tex
else
  echo 'paper build requires tectonic or pdflatex' >&2
  exit 1
fi
