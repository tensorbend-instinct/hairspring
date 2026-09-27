.PHONY: all clean figures
all: hairspring.pdf
figures:
	python3 fig1.py
	python3 fig2.py
	python3 fig_decisions.py
	python3 fig4.py
	python3 fig5.py
	python3 fig6.py
hairspring.pdf: hairspring.tex figures/fig-classes.pdf figures/fig-pipeline.pdf figures/fig-decisions.pdf figures/fig-recovery.pdf figures/fig-improve.pdf figures/fig-comparison.pdf
	./build.sh
clean:
	rm -f hairspring.aux hairspring.log hairspring.out hairspring.pdf
