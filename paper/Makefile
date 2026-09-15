.PHONY: all clean figures
all: hairspring.pdf
figures:
	python3 make_figures.py
hairspring.pdf: hairspring.tex figures/fig-classes.pdf figures/fig-pipeline.pdf figures/fig-decisions.pdf figures/fig-recovery.pdf figures/fig-improve.pdf figures/fig-comparison.pdf
	./build.sh
clean:
	rm -f hairspring.aux hairspring.log hairspring.out hairspring.pdf
