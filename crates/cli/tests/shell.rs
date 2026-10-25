//! `othismo shell` over the examples: where to look from, routes, and
//! requests made from a point of view. Needs the AssemblyScript modules
//! built first: `task test`.

use othismo::status::Status;
use othismo::runtime::{Answered, Options};
use othismo::shell::session::{Event, Outcome, Session};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

fn example(rel: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples").join(rel)
}

fn start(image: &str) -> Session {
    let path = example(image);
    let load = move || {
        let mut pack = othismo::pack::load(&path)?;
        othismo::slots::bind(&mut pack.program, &othismo::formats(), &[])?;
        Ok(pack)
    };
    Session::start(load, Options::default(), othismo::formats()).unwrap()
}

/// Runs `line`, a request, and returns its answers up to the last.
fn ask(s: &mut Session, line: &str) -> Vec<Answered> {
    let Outcome::Sent(id) = s.run_line(line).unwrap() else { panic!("{line} isn't a request") };
    let mut answers = vec![];
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline {
        if let Some(Event::Answer { id: got, answer }) = s.next_event(Duration::from_millis(100))
            && got == id
        {
            let done = answer.done;
            answers.push(answer);
            if done {
                return answers;
            }
        }
    }
    panic!("{line} wasn't answered");
}

/// The lines printed until `want` is, or ten seconds pass.
fn printed_until(s: &mut Session, want: &str) -> Vec<String> {
    let mut lines = vec![];
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline && !lines.iter().any(|l: &String| l == want) {
        if let Some(Event::Printed(line)) = s.next_event(Duration::from_millis(100)) {
            lines.push(line);
        }
    }
    lines
}

fn said(s: &mut Session, line: &str) -> Vec<String> {
    match s.run_line(line).unwrap() {
        Outcome::Said(lines) => lines,
        other => panic!("{line}: {other:?}"),
    }
}

fn last(answers: &[Answered]) -> &Answered {
    answers.last().unwrap()
}

#[test]
fn the_roots_are_the_images_view_and_what_nothing_mounts() {
    let s = start("static/static.test.oth");
    let roots: Vec<&str> = s.world.roots().into_iter().map(|r| s.world.name(r)).collect();
    assert_eq!(roots, ["/", "web", "tests"], "web.http and the rest are held by web; net by web and tests");
}

#[test]
fn the_route_is_the_shortest_chain_of_mounts_from_the_root_entered() {
    let mut s = start("static/static.test.oth");
    said(&mut s, ":look web");
    // Step in by following paths in each view.
    said(&mut s, ":look /http");
    said(&mut s, ":look /www");
    assert_eq!(s.route_line(), "web ─/http→ web.http ─/www→ web.pages");
    let (web, files) = (s.world.find("web").unwrap(), s.world.find("web.files").unwrap());
    let route = s.world.canonical(web, files);
    assert_eq!(route.first().map(|h| h.at), Some(web));
    let last = route.last().unwrap();
    assert_eq!((last.via.as_deref(), last.read_only), (Some("/files"), true));
    // Up goes back along the route, then to the image.
    said(&mut s, ":up");
    assert_eq!(s.route_line(), "web ─/http→ web.http");
    said(&mut s, ":up");
    said(&mut s, ":up");
    assert_eq!(s.here, None);
}

#[test]
fn looking_from_an_instance_enters_the_root_that_holds_it() {
    let mut s = start("static/static.test.oth");
    said(&mut s, ":look web.pages");
    assert_eq!(s.route_line(), "web ─/http→ web.http ─/www→ web.pages");
    assert!(s.run_line(":look web.files").is_err(), "the runtime's objects have no view");
}

#[test]
fn an_image_sends_as_the_operator_and_an_object_as_itself() {
    let mut s = start("static/static.test.oth");
    assert!(s.run_line("read /x").is_err(), "nowhere to send it from at the image");
    said(&mut s, ":look web");
    assert_eq!(s.prompt(), "web # ");
    assert_eq!(last(&ask(&mut s, r#"write /http/hosts/a.example "/www""#)).status, Status::Ok);
    let read = ask(&mut s, "read /http/hosts/a.example");
    assert_eq!(last(&read).body.as_ref().map(|p| p.bytes.clone()), Some(b"/www".to_vec()));
    let listed = ask(&mut s, "list /http/hosts");
    assert_eq!(listed.len(), 4, "three hosts, then done");
    said(&mut s, ":look /http");
    assert_eq!(s.prompt(), "web.http $ ");
    assert_eq!(last(&ask(&mut s, r#"write /net/tcp "x""#)).status, Status::NotFound, "an object can't configure a listener");
}

#[test]
fn messages_are_written_as_in_an_image_and_the_console_is_typed_into() {
    let mut s = start("greeter/greeter.oth");
    said(&mut s, ":look /");
    assert_eq!(last(&ask(&mut s, r#"write /greeter/greeting Greeting { prefix: "Hi, ", excited: false }"#)).status, Status::Ok);
    assert!(s.run_line(r#"write /greeter/greeting Greeting { prefix: "Hi, " }"#).is_err(), "checked before it is sent");
    s.run_line(":type Ada").unwrap();
    let printed = printed_until(&mut s, "HI, ADA.");
    assert!(printed.contains(&"Hi, Ada.".to_string()), "{printed:?}");
}

#[test]
fn a_stream_runs_until_it_is_cancelled() {
    let mut s = start("greeter/greeter.oth");
    said(&mut s, ":look /");
    let Outcome::Sent(id) = s.run_line("watch /dev/cons").unwrap() else { panic!() };
    assert!(s.open.contains_key(&id));
    said(&mut s, ":cancel");
    let deadline = Instant::now() + Duration::from_secs(10);
    while s.open.contains_key(&id) && Instant::now() < deadline {
        s.next_event(Duration::from_millis(100));
    }
    assert!(!s.open.contains_key(&id));
}

#[test]
fn the_plain_shell_reads_commands_a_line_at_a_time() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_othismo"))
        .args(["shell", "--plain"])
        .arg(example("static/static.test.oth"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(b":look web\nread /http/hosts/files.localhost\n:quit\n").unwrap();
    let out = child.wait_with_output().unwrap();
    assert!(out.status.success(), "stderr: {}", String::from_utf8_lossy(&out.stderr));
    let out = String::from_utf8(out.stdout).unwrap();
    assert!(out.contains("web # "), "{out}");
    assert!(out.contains("ok done · text/plain\n  /www/static"), "{out}");
}
