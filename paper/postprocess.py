#!/usr/bin/env python3
# The editable LaTeX source is canonical for text; the maintained HTML template owns print composition.
import pathlib,sys
pathlib.Path(sys.argv[2]).write_text(pathlib.Path(__file__).with_name('hairspring.template.html').read_text())
