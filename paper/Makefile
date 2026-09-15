.PHONY: all clean figures
all: hairspring.pdf
figures:
	python3 make_figures.py
hairspring.pdf: hairspring.tex figures/architecture.pdf figures/record.pdf figures/benchmark.pdf figures/evidence-layers.pdf
	./build.sh
clean:
	rm -f hairspring.aux hairspring.log hairspring.out hairspring.pdf
