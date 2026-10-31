//! Completion on the shell's command line, from the views, the servers'
//! headers and the formats' schemas. Needs the AssemblyScript modules
//! built first: `task test`.

use othismo::runtime::Options;
use othismo::shell::complete::{Completion, complete};
use othismo::shell::session::Session;
use othismo::shell::tui::App;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use std::path::Path;
use std::time::{Duration, Instant};

fn start(image: &str) -> Session {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples").join(image);
    let load = move || {
        let mut pack = othismo::pack::load(&path)?;
        othismo::slots::bind(&mut pack.program, &othismo::formats(), &[])?;
        Ok(pack)
    };
    Session::start(load, Options::default(), othismo::formats()).unwrap()
}

fn at(image: &str, look: &str) -> Session {
    let mut s = start(image);
    s.run_line(&format!(":look {look}")).unwrap();
    s
}

fn complete_(s: &Session, line: &str) -> Completion {
    complete(s, &|_, _| None, line)
}

fn labels(c: &Completion) -> Vec<&str> {
    c.items.iter().map(|i| i.label.as_str()).collect()
}

#[test]
fn the_verbs_come_first() {
    let s = at("static/static.test.oth", "web");
    assert_eq!(labels(&complete_(&s, "")), ["read", "stat", "list", "watch", "write", "create", "remove"]);
    let c = complete_(&s, "wr");
    assert_eq!(labels(&c), ["write"]);
    assert_eq!(c.items[0].insert, "write ");
    assert!(c.items[0].doc.contains("Change what is at the path"));
}

#[test]
fn nothing_can_be_sent_from_nowhere() {
    let s = start("static/static.test.oth");
    assert!(complete_(&s, "re").hint.unwrap().contains("Look from somewhere first"));
}

#[test]
fn paths_come_from_the_view_and_the_servers_headers() {
    let s = at("static/static.test.oth", "web");
    let c = complete_(&s, "write /");
    assert_eq!(c.from, "write ".len());
    assert!(labels(&c).contains(&"http/"), "{c:#?}");
    let c = complete_(&s, "write /http/");
    assert_eq!(labels(&c), ["hosts/"], "declared in HttpServer's header");
    assert_eq!(c.wants.as_ref().map(|(_, p)| p.as_str()), Some("/"), "and worth listing");
    let c = complete_(&s, "write /http/hosts/");
    let name = c.items.iter().find(|i| i.label == "{name}").expect("the pattern");
    assert!(name.placeholder && name.usable, "web is the operator: {name:#?}");
    assert!(name.doc.contains("serve the host from there"), "{name:#?}");
}

#[test]
fn what_the_runtime_would_refuse_is_offered_with_why() {
    let s = at("static/static.test.oth", "web.http");
    let c = complete_(&s, "write /net/t");
    let tcp = c.items.iter().find(|i| i.label.starts_with("tcp")).expect("tcp");
    assert!(tcp.detail.contains("not among web.http's needs"), "{tcp:#?}");
    let c = complete_(&s, "write /www/static/x ");
    assert!(c.hint.unwrap().contains("read-only from here"));
    let c = complete_(&s, "read /www/hits ");
    assert!(c.hint.unwrap().contains("takes no payload"), "and says so");
}

#[test]
fn a_message_is_completed_field_by_field() {
    let s = at("greeter/greeter.oth", "/");
    let c = complete_(&s, "write /greeter/greeting ");
    assert_eq!(labels(&c), ["Greeting"]);
    assert_eq!(c.items[0].insert, "Greeting {  }");
    assert_eq!(c.items[0].caret, Some("Greeting { ".len()));
    assert!(c.items[0].signature.contains("prefix: string, excited: bool"), "{:#?}", c.items[0]);
    let c = complete_(&s, "write /greeter/greeting Greeting { ");
    assert_eq!(labels(&c), ["prefix", "excited"]);
    assert_eq!(c.items[0].doc, "Put before each name.");
    let c = complete_(&s, r#"write /greeter/greeting Greeting { prefix: "Hi", "#);
    assert_eq!(labels(&c), ["excited"]);
    let c = complete_(&s, r#"write /greeter/greeting Greeting { prefix: "Hi", excited: "#);
    assert_eq!(labels(&c), ["true", "false"]);
    let c = complete_(&s, r#"write /greeter/greeting Greeting { prefix: "Hi", excited: true, "#);
    assert!(c.hint.unwrap().contains("Every field of Greeting is given"));
}

#[test]
fn the_shells_own_commands_complete_too() {
    let s = start("static/static.test.oth");
    let c = complete_(&s, ":lo");
    assert_eq!(labels(&c), [":look"]);
    let c = complete_(&s, ":look w");
    assert_eq!(labels(&c)[0], "web", "roots first");
    assert!(labels(&c).contains(&"web.http"));
}

fn press(app: &mut App, code: KeyCode) {
    assert!(app.key(KeyEvent::new(code, KeyModifiers::NONE)));
}

#[test]
fn tab_puts_the_choice_in_and_carries_on() {
    let mut app = App::new(at("static/static.test.oth", "web"), "static.test.oth");
    press(&mut app, KeyCode::Char(':'));
    for c in "wr".chars() {
        press(&mut app, KeyCode::Char(c));
    }
    press(&mut app, KeyCode::Tab);
    assert_eq!(app.input, "write ");
    for c in "/http/hosts/".chars() {
        press(&mut app, KeyCode::Char(c));
    }
    // The hosts arrive once they have been listed.
    let deadline = Instant::now() + Duration::from_secs(10);
    while !app.completion.items.iter().any(|i| i.label == "*") {
        assert!(Instant::now() < deadline, "{:#?}", app.completion);
        app.pump(Duration::from_millis(50));
    }
    let i = app.completion.items.iter().position(|i| i.label == "*").unwrap();
    for _ in 0..i {
        press(&mut app, KeyCode::Down);
    }
    press(&mut app, KeyCode::Tab);
    assert_eq!(app.input, "write /http/hosts/* ");
    press(&mut app, KeyCode::Tab);
    assert_eq!(app.input, r#"write /http/hosts/* """#, "then a text literal, the cursor inside it");
    // Esc closes the choices, then goes back to the table.
    press(&mut app, KeyCode::Esc);
    press(&mut app, KeyCode::Esc);
    assert_eq!(app.focus, othismo::shell::tui::Focus::Tree);
}

