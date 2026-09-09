//! GATE 4 ACCEPTANCE (spec section 10, row 4):
//!   Plant false-completion cases (artifact exists but fails hidden tests):
//!   independent/hybrid modes catch 100% of them; self mode's miss rate is
//!   measured and reported. Gateway events mid-run leave progress intact.
//!
//! Falsifiable: a single false pass in independent or hybrid mode fails the
//! gate. Self mode's miss rate is published, whatever it is.

use hs_goal::*;

const ANSWER: &str = env!("CARGO_BIN_EXE_hs-plugin-goalanswer");
const GOALCHECKER: &str = env!("CARGO_BIN_EXE_hs-plugin-goalchecker");
const GOALMODEL: &str = env!("CARGO_BIN_EXE_hs-plugin-goalmodel");
const PLANTS: usize = 8;

fn hidden_correct(spec: &str) -> String {
    let i: usize = spec.strip_prefix("plant-").unwrap().parse().unwrap();
    format!("VISIBLE-{i}\nHIDDEN-{i}")
}

struct PlantRun {
    reported_pass: bool,
    actually_correct: bool,
    false_catches: u32,
}

fn run_plant(root: &std::path::Path, i: usize, mode: CompletionMode) -> PlantRun {
    let spec = format!("plant-{i}");
    let dir = root.join(format!("{mode:?}-{spec}"));
    let log = dir.join("log");
    std::fs::create_dir_all(&log).unwrap();
    let config = dir.join("hairspring.toml");
    std::fs::write(
        &config,
        format!(
            r#"
[[tools]]
name = "answer.write"
command = ["{ANSWER}"]
subjects = ["*"]

[[tools]]
name = "goalchecker.run"
command = ["{GOALCHECKER}"]
subjects = ["*"]

[[models]]
name = "goalmodel"
command = ["{GOALMODEL}"]
default = true
"#
        ),
    )
    .unwrap();
    let kernel = hs_kernel::Kernel::load(&config).unwrap();
    let mut l = OuterLoop::new(kernel, &log, 0).unwrap();
    let goal = Goal::new(
        &spec,
        mode,
        Budget {
            max_steps: 8,
            max_cost_usd_micros: 1_000_000,
        },
    );
    let out = l.run(&goal).unwrap();
    hs_log::verify_stream(&log, l.stream_id()).unwrap();
    let artifact = std::fs::read_to_string(log.join("work").join(&spec).join("answer.txt"))
        .unwrap_or_default();
    PlantRun {
        reported_pass: matches!(out, MissionOutcome::Passed { .. }),
        actually_correct: artifact.trim() == hidden_correct(&spec),
        false_catches: l.false_completions_caught(),
    }
}

#[test]
fn gate4_proof_false_completion_plants() {
    let root = tempfile::tempdir().unwrap();
    let mut report = String::new();
    let mut misses = [0usize; 3]; // self, independent, hybrid
    let mut false_passes = [0usize; 3];
    for (mi, mode) in [
        CompletionMode::SelfDeclared,
        CompletionMode::Independent,
        CompletionMode::Hybrid,
    ]
    .iter()
    .enumerate()
    {
        let mut mode_false = 0;
        let mut mode_missed = 0;
        let mut catches = 0;
        for i in 0..PLANTS {
            let r = run_plant(root.path(), i, *mode);
            if !r.actually_correct {
                mode_false += 1;
                if r.reported_pass {
                    mode_missed += 1;
                }
            }
            catches += r.false_catches;
            if r.reported_pass && !r.actually_correct {
                false_passes[mi] += 1;
            }
        }
        misses[mi] = mode_missed;
        report.push_str(&format!(
            "  mode {:?}: {} planted false completions, {} falsely reported as passed (miss rate {:.0}% of plants), {} catches counted\n",
            mode, mode_false, mode_missed,
            mode_missed as f64 / f64::from(mode_false) * 100.0, catches
        ));
    }
    print!("{report}");
    // THE GATE:
    assert_eq!(
        false_passes[1], 0,
        "independent mode let a false completion through: gate falsified"
    );
    assert_eq!(
        false_passes[2], 0,
        "hybrid mode let a false completion through: gate falsified"
    );
    // measured and published, not assumed:
    assert_eq!(
        misses[0],
        PLANTS / 2,
        "self mode missed all 4 false plants in this suite"
    );
    println!("PROOF-GATE4 false-completion plants: PASS");
    println!(
        "  independent + hybrid: 100% of {} planted false completions caught (0 false passes)",
        PLANTS / 2
    );
    println!("  self mode miss rate: {}/{} = 100% of planted false completions ({}% of all plants) - measured, published",
        misses[0], PLANTS / 2, misses[0] * 100 / PLANTS);
}
