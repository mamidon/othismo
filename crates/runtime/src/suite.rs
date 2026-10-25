//! Running an image's tests, for `othismo test`. A test suite is any
//! instance whose type declares the `TestSuite` trait: it lists its tests at
//! `/tests`, a stat record each, and runs one when `/tests/<name>` is
//! written, answering `ok` if it passed and an error saying why if not. Each test gets a fresh run of the image, started but with
//! nothing outside: no stdin, no sockets.

use protocol::{status::Status, verb::Verb};
use image::pack::Pack;
use protocol::payload::Stat;
use crate::runtime::{Answered, Io, Options, Runtime};
use anyhow::{Context, Result, bail};
use std::cell::RefCell;
use std::io::Write;
use std::rc::Rc;

/// The trait a test suite's type declares.
pub const TEST_SUITE: &str = "TestSuite";

/// How one test went.
#[derive(Debug, PartialEq)]
pub struct Report {
    /// The suite's instance name.
    pub suite: String,
    pub test: String,
    pub passed: bool,
    pub detail: String,
    /// Whatever the image printed on its consoles while the test ran.
    pub output: String,
}

/// The instances in `pack` whose type declares TestSuite.
pub fn suites(pack: &Pack) -> Vec<usize> {
    let p = &pack.program;
    (0..p.instances.len())
        .filter(|&i| p.interfaces.get(&p.instances[i].type_name).is_some_and(|o| o.traits.iter().any(|t| t == TEST_SUITE)))
        .collect()
}

/// Lists every suite's tests in one run of the image, then runs each test
/// in a run of its own.
pub fn run(pack: &Pack, options: Options) -> Result<Vec<Report>> {
    let suites = suites(pack);
    if suites.is_empty() {
        bail!("nothing in the image declares {TEST_SUITE}, so it has no tests");
    }
    let mut tests: Vec<(usize, String)> = vec![];
    let (mut rt, _) = start(pack, options).context("starting the image to list its tests")?;
    for &i in &suites {
        let name = &pack.program.instances[i].name;
        let answers = rt.ask(i, Verb::List, "/tests", None)?;
        match answers.last() {
            Some(a) if a.done && a.status == Status::Ok => {}
            Some(a) if a.done => bail!("`{name}` answered `list /tests` with {}", a.status),
            _ => bail!("`{name}` never finished listing its tests"),
        }
        for body in answers.into_iter().filter_map(|a| a.body) {
            let test = Stat::decode(&body).with_context(|| format!("`{name}` listed a test without a stat record"))?;
            tests.push((i, test.name));
        }
    }
    drop(rt);

    let mut reports = vec![];
    for (i, test) in tests {
        let (passed, detail, output) = match run_one(pack, options, i, &test) {
            Ok((passed, detail, output)) => (passed, detail, output),
            Err(e) => (false, format!("the run failed: {e:#}"), String::new()),
        };
        reports.push(Report { suite: pack.program.instances[i].name.clone(), test, passed, detail, output });
    }
    Ok(reports)
}

/// Runs test `test` of suite `i` in a fresh run: whether it passed, why,
/// and what was printed.
fn run_one(pack: &Pack, options: Options, i: usize, test: &str) -> Result<(bool, String, String)> {
    if test.is_empty() || test.contains('/') {
        return Ok((false, format!("`{test}` can't be run: a test's name is one path segment"), String::new()));
    }
    let (mut rt, out) = start(pack, options)?;
    let answers = rt.ask(i, Verb::Write, &format!("/tests/{test}"), None)?;
    let (passed, detail) = outcome(answers.into_iter().find(|a| a.done));
    Ok((passed, detail, out.text()))
}

fn outcome(answer: Option<Answered>) -> (bool, String) {
    match answer {
        None => (false, "it never answered".into()),
        Some(a) if a.status == Status::Ok => (true, String::new()),
        Some(a) if a.detail.is_empty() => (false, format!("it answered {}", a.status)),
        Some(a) => (false, a.detail),
    }
}

/// A run of `pack`, started, with its output captured.
fn start(pack: &Pack, options: Options) -> Result<(Runtime<'_>, Captured)> {
    let out = Captured::default();
    let io = Io { input: Box::new(std::io::empty()), output: Box::new(out.clone()) };
    let mut rt = Runtime::new(pack, io, options)?;
    rt.start()?;
    Ok((rt, out))
}

#[derive(Clone, Default)]
struct Captured(Rc<RefCell<Vec<u8>>>);

impl Captured {
    fn text(&self) -> String {
        String::from_utf8_lossy(&self.0.borrow()).into_owned()
    }
}

impl Write for Captured {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.borrow_mut().extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn answered(status: Status, detail: &str) -> Option<Answered> {
        Some(Answered { status, body: None, detail: detail.into(), done: true })
    }

    #[test]
    fn a_test_passes_with_ok_and_fails_with_an_error() {
        assert_eq!(outcome(answered(Status::Ok, "")), (true, "".into()));
        assert_eq!(outcome(answered(Status::Invalid, "got 404")), (false, "got 404".into()));
        assert_eq!(outcome(answered(Status::NotFound, "")), (false, "it answered not_found".into()));
        assert_eq!(outcome(None), (false, "it never answered".into()));
    }
}
