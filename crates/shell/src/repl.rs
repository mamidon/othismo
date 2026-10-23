//! The shell in plain lines, `othismo shell --plain`: a prompt, and what
//! happened since the last one. For a terminal that can't run the full
//! screen, and for scripts.

use crate::session::{Event, Outcome, Session};
use crate::show;
use anyhow::Result;
use std::io::{BufRead, Write};
use std::time::{Duration, Instant};

/// How long to wait for a request's first answer before prompting again.
const PATIENCE: Duration = Duration::from_secs(2);

/// Reads lines from `input` until it ends or `:quit`, writing to `out`.
pub fn run(session: &mut Session, input: impl BufRead, out: &mut impl Write) -> Result<()> {
    for line in session.describe_roots() {
        writeln!(out, "{line}")?;
    }
    writeln!(out, "Look from one with :look <name>; :help lists the rest.")?;
    let mut lines = input.lines();
    loop {
        settle(session, out, None)?;
        if session.stopped.is_some() {
            return Ok(());
        }
        write!(out, "{}", session.prompt())?;
        out.flush()?;
        let Some(line) = lines.next().transpose()? else { return Ok(()) };
        match session.run_line(&line) {
            Ok(Outcome::Nothing) => {}
            Ok(Outcome::Sent(id)) => settle(session, out, Some(id))?,
            Ok(Outcome::Said(said)) => {
                for l in said {
                    writeln!(out, "{l}")?;
                }
            }
            Ok(Outcome::Quit) => return Ok(()),
            Err(e) => writeln!(out, "✗ {e:#}")?,
        }
    }
}

/// Writes what has happened. With `waiting`, waits for that request's
/// first answer, or its last if it comes soon after; otherwise only for
/// what is about to arrive.
fn settle(session: &mut Session, out: &mut impl Write, waiting: Option<u64>) -> Result<()> {
    let start = Instant::now();
    let mut answered = false;
    loop {
        let wait = match waiting {
            Some(_) if !answered => PATIENCE.saturating_sub(start.elapsed()),
            _ => Duration::from_millis(30),
        };
        let Some(event) = session.next_event(wait) else { return Ok(()) };
        match event {
            Event::Answer { id, answer } => {
                let tag = if session.open.len() > 1 || !answer.done || Some(id) != waiting { format!("[{id}] ") } else { String::new() };
                for l in show::answer(&answer, &session.formats) {
                    writeln!(out, "  {tag}{l}")?;
                }
                if Some(id) == waiting {
                    answered = true;
                }
            }
            Event::Printed(line) => writeln!(out, "  │ {line}")?,
            Event::Refreshed => {}
            Event::Stopped(why) => {
                match why {
                    Some(why) => writeln!(out, "the image stopped: {why}")?,
                    None => writeln!(out, "the image stopped")?,
                }
                return Ok(());
            }
        }
    }
}
