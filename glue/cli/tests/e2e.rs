//! The end-to-end corpus: Glue files on disk, expected output beside them.
//!
//! Every other test in this workspace is written against a crate API, so it
//! moves whenever the crates move. These are written against the `glue` binary
//! — the harness shells out to it — which is what makes a case survive `elab`
//! being split, a `zir` stage appearing in the middle, or the whole pass
//! structure being redesigned underneath it.
//!
//! # Layout
//!
//! ```text
//! e2e/<scenario_snake_case>/
//!   inputs/<name>.glue      one or more programs
//!   <stage>/<name>.sexpr    `glue dump inputs/<name>.glue <stage>` on stdout
//!   <stage>/<name>.stderr   …and its diagnostics, if it has any
//!   output.txt              `glue eval inputs/<name>.glue` on stdout
//!   output.stderr           …and its diagnostics, if it has any
//! ```
//!
//! The stage directories are named by [`Stage::name`], and the harness
//! enumerates [`Stage::ALL`] rather than a list of its own — so a stage added
//! to the pipeline is covered by every scenario that opts into it the day it is
//! added, rather than the day someone remembers this file exists. A directory
//! naming something that is not a stage is an error, because it is a typo.
//!
//! **Absence means "not asserted".** A scenario asserts a stage when the
//! directory for it exists, and nothing else. `mkdir e2e/foo/cst` then rerun
//! with `UPDATE_EXPECT=1` is how a scenario gains a stage; `rm -r e2e/foo/cst`
//! is how it loses one. `tokens` and `cst` are pinned only where the syntax is
//! the point, because they are long and they move for reasons that have
//! nothing to do with what a scenario is about.
//!
//! **An absent `.stderr` means the run was silent**, which is a claim worth
//! making — a program that starts reporting something it used to compile
//! cleanly is exactly the regression this corpus is for.
//!
//! With more than one input, `output.txt` becomes `output/<name>.txt`, since
//! Glue has no imports and each file is its own program. One input is the
//! ordinary case.
//!
//! # Running it
//!
//! ```text
//! cargo test -p cli --test e2e
//! UPDATE_EXPECT=1 cargo test -p cli --test e2e    # regenerate
//! E2E_FILTER=struct cargo test -p cli --test e2e  # one scenario, or a few
//! ```
//!
//! A new scenario needs `inputs/` and nothing else: `UPDATE_EXPECT=1` on a
//! scenario with no expectations at all opts it into `ir` and `output.txt`,
//! which is the pair worth having by default. Read what it generated before
//! committing it — the corpus is a **change detector**, not a specification. It
//! records what the implementation does so that changing it is a decision
//! rather than an accident. What the language *requires* is pinned in the unit
//! tests, where the section each rule comes from is named.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use cli::Stage;

/// The `glue` binary this test's crate just built. Cargo sets it, which is the
/// reason this harness is an integration test rather than a script.
const GLUE: &str = env!("CARGO_BIN_EXE_glue");

fn corpus() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../e2e")
}

#[test]
fn the_corpus_still_says_what_it_said() {
    let update = std::env::var_os("UPDATE_EXPECT").is_some();
    let filter = std::env::var("E2E_FILTER").unwrap_or_default();

    let mut failures = Vec::new();
    let mut scenarios = 0;
    let mut assertions = 0;

    for scenario in read_scenarios(&corpus()) {
        if !scenario.name.contains(&filter) {
            continue;
        }
        scenarios += 1;
        check(&scenario, update, &mut assertions, &mut failures);
    }

    assert!(scenarios > 0, "no scenarios matched `{filter}`");

    if !failures.is_empty() {
        panic!(
            "\n{} of {assertions} expectations in {scenarios} scenarios did not \
             hold.\n\n{}\n\n\
             If the new output is right, rerun with UPDATE_EXPECT=1 and read the diff.\n",
            failures.len(),
            failures.join("\n")
        );
    }
}

// ---- A scenario ------------------------------------------------------------

struct Scenario {
    name: String,
    dir: PathBuf,
    /// Sorted, so a multi-input scenario reports in a stable order.
    inputs: Vec<PathBuf>,
}

fn read_scenarios(corpus: &Path) -> Vec<Scenario> {
    let mut scenarios = Vec::new();
    for entry in sorted_dir(corpus) {
        if !entry.is_dir() {
            continue;
        }
        let name = file_name(&entry);
        let inputs: Vec<PathBuf> = sorted_dir(&entry.join("inputs"))
            .into_iter()
            .filter(|path| path.extension().is_some_and(|ext| ext == "glue"))
            .collect();
        assert!(
            !inputs.is_empty(),
            "{name} has no inputs/*.glue — a scenario is a program and a claim about it"
        );
        scenarios.push(Scenario {
            name,
            dir: entry,
            inputs,
        });
    }
    scenarios
}

/// Everything a scenario has opted into, checked or regenerated.
fn check(scenario: &Scenario, update: bool, assertions: &mut usize, failures: &mut Vec<String>) {
    let stages: Vec<Stage> = Stage::ALL
        .into_iter()
        .filter(|stage| scenario.dir.join(stage.name()).is_dir())
        .collect();
    let evaluates =
        scenario.dir.join("output.txt").exists() || scenario.dir.join("output").is_dir();

    // A brand-new scenario: the pair worth having by default.
    let (stages, evaluates) = if update && stages.is_empty() && !evaluates {
        fs::create_dir_all(scenario.dir.join(Stage::Ir.name())).unwrap();
        (vec![Stage::Ir], true)
    } else {
        (stages, evaluates)
    };

    reject_misspelled_stage_directories(scenario);

    for input in &scenario.inputs {
        let stem = input.file_stem().unwrap().to_str().unwrap().to_string();

        for stage in &stages {
            let run = run(&["dump", input.to_str().unwrap(), stage.name()], input);
            let dir = scenario.dir.join(stage.name());
            compare(
                scenario,
                &run,
                &dir.join(format!("{stem}.sexpr")),
                &dir.join(format!("{stem}.stderr")),
                update,
                assertions,
                failures,
            );
        }

        if evaluates {
            let run = run(&["eval", input.to_str().unwrap()], input);
            let (out, err) = if scenario.inputs.len() == 1 {
                (
                    scenario.dir.join("output.txt"),
                    scenario.dir.join("output.stderr"),
                )
            } else {
                let dir = scenario.dir.join("output");
                fs::create_dir_all(&dir).unwrap();
                (
                    dir.join(format!("{stem}.txt")),
                    dir.join(format!("{stem}.stderr")),
                )
            };
            compare(scenario, &run, &out, &err, update, assertions, failures);
        }
    }

    reject_stale_expectations(scenario, &stages, failures);
}

/// A directory that is not a stage is a typo, and a typo that silently asserts
/// nothing is the worst outcome available.
fn reject_misspelled_stage_directories(scenario: &Scenario) {
    let known: BTreeSet<&str> = Stage::ALL
        .iter()
        .map(|stage| stage.name())
        .chain(["inputs", "output"])
        .collect();
    for entry in sorted_dir(&scenario.dir) {
        let name = file_name(&entry);
        assert!(
            !entry.is_dir() || known.contains(name.as_str()),
            "{}/{name}/ names no stage — expected one of {known:?}",
            scenario.name
        );
    }
}

/// An expectation whose input was renamed or deleted asserts nothing and looks
/// like coverage, so it is a failure rather than a thing to ignore.
fn reject_stale_expectations(scenario: &Scenario, stages: &[Stage], failures: &mut Vec<String>) {
    let stems: BTreeSet<String> = scenario
        .inputs
        .iter()
        .map(|input| input.file_stem().unwrap().to_str().unwrap().to_string())
        .collect();
    let mut dirs: Vec<PathBuf> = stages
        .iter()
        .map(|stage| scenario.dir.join(stage.name()))
        .collect();
    if scenario.inputs.len() > 1 {
        dirs.push(scenario.dir.join("output"));
    }
    for dir in dirs {
        for entry in sorted_dir(&dir) {
            let stem = entry.file_stem().unwrap().to_str().unwrap();
            if !stems.contains(stem) {
                failures.push(format!(
                    "  {}: {} expects `{stem}.glue`, which is not in inputs/",
                    scenario.name,
                    display(&entry)
                ));
            }
        }
    }
}

// ---- Running the binary ----------------------------------------------------

struct Run {
    command: String,
    stdout: String,
    stderr: String,
}

/// One invocation of `glue`, with the input's path rewritten out of its
/// diagnostics so an expectation is not specific to this machine.
fn run(args: &[&str], input: &Path) -> Run {
    let output = Command::new(GLUE)
        .args(args)
        .output()
        .unwrap_or_else(|problem| panic!("could not run {GLUE}: {problem}"));

    let name = file_name(input);
    let stderr = String::from_utf8_lossy(&output.stderr).replace(input.to_str().unwrap(), &name);

    // The CLI exits non-zero exactly when it reported something, and a corpus
    // that pins the messages should not have to pin the status separately.
    assert_eq!(
        output.status.success(),
        stderr.is_empty(),
        "`glue {}` exited {:?} having said {stderr:?}",
        args.join(" "),
        output.status.code()
    );

    // Shown the way a person would type it, from inside the scenario directory.
    let shown: Vec<String> = args
        .iter()
        .map(|arg| {
            if *arg == input.to_str().unwrap() {
                format!("inputs/{name}")
            } else {
                arg.to_string()
            }
        })
        .collect();
    Run {
        command: format!("glue {}", shown.join(" ")),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr,
    }
}

// ---- Comparing, or writing ------------------------------------------------

fn compare(
    scenario: &Scenario,
    run: &Run,
    stdout: &Path,
    stderr: &Path,
    update: bool,
    assertions: &mut usize,
    failures: &mut Vec<String>,
) {
    if update {
        write(stdout, &run.stdout);
        // Absence is the claim "this was silent", so an emptied `.stderr` is
        // removed rather than left as a zero-byte file saying the same thing
        // in a second way.
        if run.stderr.is_empty() {
            let _ = fs::remove_file(stderr);
        } else {
            write(stderr, &run.stderr);
        }
        return;
    }

    for (path, actual, what) in [
        (stdout, &run.stdout, "stdout"),
        (stderr, &run.stderr, "diagnostics"),
    ] {
        *assertions += 1;
        let expected = match fs::read_to_string(path) {
            Ok(expected) => expected,
            // Only `stdout` is required — an absent `.stderr` means silence.
            Err(_) if what == "diagnostics" => String::new(),
            Err(_) => {
                failures.push(format!(
                    "  {}: no expectation at {} for `{}`",
                    scenario.name,
                    display(path),
                    run.command
                ));
                continue;
            }
        };
        if expected != *actual {
            failures.push(format!(
                "  {}: `{}` {what} differs from {}\n{}",
                scenario.name,
                run.command,
                display(path),
                diff(&expected, actual)
            ));
        }
    }
}

fn write(path: &Path, contents: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, contents).unwrap_or_else(|problem| panic!("{}: {problem}", display(path)));
}

/// The first place the two disagree, with a little either side of it. Whole
/// dumps are long and the first divergence is nearly always the story.
fn diff(expected: &str, actual: &str) -> String {
    let (want, got): (Vec<&str>, Vec<&str>) =
        (expected.lines().collect(), actual.lines().collect());
    let length = want.len().max(got.len());
    let Some(first) = (0..length).find(|&line| want.get(line) != got.get(line)) else {
        return format!(
            "      the lines match; the bytes do not ({} expected, {} actual) — \
             trailing whitespace\n",
            expected.len(),
            actual.len()
        );
    };

    let (start, end) = (first.saturating_sub(2), (first + 6).min(length));
    let mut out = String::from("      (- expected, + actual)\n");
    for line in start..end {
        match (want.get(line), got.get(line)) {
            (Some(want), Some(got)) if want == got => {
                out.push_str(&format!("       {:>4} │ {want}\n", line + 1));
            }
            (want, got) => {
                if let Some(want) = want {
                    out.push_str(&format!("      -{:>4} │ {want}\n", line + 1));
                }
                if let Some(got) = got {
                    out.push_str(&format!("      +{:>4} │ {got}\n", line + 1));
                }
            }
        }
    }
    if end < length {
        out.push_str("            ⋮\n");
    }
    out
}

// ---- Paths -----------------------------------------------------------------

/// Sorted, because a corpus that reports its cases in inode order is a corpus
/// whose diffs move on their own. A missing directory is empty, which is what
/// "this scenario did not opt in" looks like.
fn sorted_dir(dir: &Path) -> Vec<PathBuf> {
    let mut entries: Vec<PathBuf> = fs::read_dir(dir)
        .into_iter()
        .flatten()
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| !file_name(path).starts_with('.'))
        .collect();
    entries.sort();
    entries
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned()
}

/// Corpus-relative, so a failure names `e2e/foo/ir/main.sexpr` rather than
/// this machine's home directory.
fn display(path: &Path) -> String {
    let corpus = corpus();
    let shown = path.strip_prefix(&corpus).unwrap_or(path);
    format!("e2e/{}", shown.display())
}
