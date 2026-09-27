# HAIRSPRING paper

`hairspring.tex` is the editable manuscript. It describes the locked paired
15-instance SWE-bench Verified comparison with OpenHands and DeepSeek V4.1
Flash on both sides. Results are **pending**: no paid pair has completed, so
this revision reports no solve count or winner. The older ten-task CSV/JSON
under `evidence/` belong to a different comparison and are archived, not
inputs to the current table or figure.

`fig1.py`, `fig2.py`, `fig_decisions.py`, `fig4.py`, `fig5.py`, and
`fig6.py` generate the six manuscript figures. The older all-in-one
`make_figures.py` also overwrites figures, so do not use it for this paper.
`fig6.py` reads the locked manifest and draws only the protocol and
pending cells. Build with pdfLaTeX or Tectonic:

```sh
cd paper
python3 fig1.py && python3 fig2.py && python3 fig_decisions.py
python3 fig4.py && python3 fig5.py && python3 fig6.py
./build.sh
```

The native grader, input digests, validation notes and conservative spending
record are in `benchmarks/paired15/`. `paper/reproduce.py benchmark` still
recomputes the *archived ten-task comparison only* and is not a reproduction
command for the pending study.
