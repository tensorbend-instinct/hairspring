# HAIRSPRING paper

`hairspring.tex` is the editable manuscript. Generated figures live in
`figures/`; raw matched-run records live in `evidence/`.

Build with either Tectonic or a complete pdfLaTeX installation:

```sh
cd paper
make figures
make clean all
```

Recompute the source and benchmark aggregates without paid inference:

```sh
python3 paper/reproduce.py source
python3 paper/reproduce.py benchmark
```

The measurements in the paper describe source revision
`4273dccf6d00fa727b01fcbdedc6773ace9eca12`. Later paper commits preserve those
raw evidence files and identify the evidence revision in the manuscript.
