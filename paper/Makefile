.PHONY: all clean figures
all: hairspring.pdf
figures:
	python3 make_figures.py
hairspring.pdf: hairspring.tex hairspring.template.html paper.css postprocess.py build.sh figures/architecture.png figures/record.png figures/benchmark.png
	./build.sh
clean:
	rm -rf .pages hairspring.raw.html hairspring.full.pdf hairspring.pdf
