//! `othismo shell` on the whole screen, driven by keys and drawn to a
//! buffer. Needs the AssemblyScript modules built first: `task test`.

use othismo::runtime::Options;
use othismo::shell::session::Session;
use othismo::shell::tui::{App, Focus};
use othismo::shell::world::Viewpoint;
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use std::path::Path;
use std::time::{Duration, Instant};

fn app(image: &str) -> App {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples").join(image);
    let load = move || {
        let mut pack = othismo::pack::load(&path)?;
        othismo::slots::bind(&mut pack.program, &othismo::formats(), &[])?;
        Ok(pack)
    };
    let session = Session::start(load, Options::default(), othismo::formats()).unwrap();
    App::new(session, image.rsplit('/').next().unwrap())
}

fn press(app: &mut App, code: KeyCode) {
    assert!(app.key(KeyEvent::new(code, KeyModifiers::NONE)), "{code:?} quit");
}

fn type_line(app: &mut App, line: &str) {
    if app.focus == Focus::Tree {
        press(app, KeyCode::Char(':'));
    }
    for c in line.chars() {
        press(app, KeyCode::Char(c));
    }
    press(app, KeyCode::Enter);
}

/// Back to the table: Esc closes the choices first, if they're open.
fn leave_command_line(app: &mut App) {
    for _ in 0..2 {
        if app.focus == Focus::Command {
            press(app, KeyCode::Esc);
        }
    }
    assert_eq!(app.focus, Focus::Tree);
}

/// Handles the image's events until `done` holds, or ten seconds pass.
fn until(app: &mut App, what: &str, done: impl Fn(&App) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !done(app) {
        assert!(Instant::now() < deadline, "waited for {what}; the table: {:#?}\nthe log: {:#?}", app.table_text(), app.log_text());
        app.pump(Duration::from_millis(50));
    }
}

fn first_column(app: &App) -> Vec<String> {
    app.table_text().into_iter().map(|r| r.first().cloned().unwrap_or_default()).collect()
}

fn has_row(app: &App, text: &str) -> bool {
    first_column(app).iter().any(|c| c.trim_start_matches(['│', '├', '└', '─', ' ', '▸', '▾']) == text)
}

fn screen(app: &mut App) -> String {
    let mut terminal = Terminal::new(TestBackend::new(150, 40)).unwrap();
    terminal.draw(|f| app.draw(f)).unwrap();
    let buffer = terminal.backend().buffer().clone();
    (0..buffer.area.height)
        .map(|y| (0..buffer.area.width).map(|x| buffer[(x, y)].symbol().to_string()).collect::<String>())
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn it_opens_on_the_images_roots() {
    let mut app = app("static/static.test.oth");
    assert_eq!(first_column(&app)[..3], ["/", "web", "tests"]);
    let s = screen(&mut app);
    assert!(s.contains("◆ static.test.oth"), "{s}");
    assert!(s.contains("ROOT") && s.contains("HOLDS"), "{s}");
    assert!(s.contains("NOT ROOTS"), "the inspector says why the rest aren't: {s}");
}

#[test]
fn it_steps_into_views_and_opens_served_trees() {
    let mut app = app("static/static.test.oth");
    press(&mut app, KeyCode::Down);
    press(&mut app, KeyCode::Enter);
    assert_eq!(app.session.here, app.session.world.find("web"));
    assert!(has_row(&app, "http"), "{:#?}", app.table_text());
    // Open what web.http serves, then its hosts.
    assert!(app.select_path("/http"));
    press(&mut app, KeyCode::Right);
    until(&mut app, "the listing of /http", |a| has_row(a, "hosts/"));
    assert!(app.select_path("/http/hosts"));
    press(&mut app, KeyCode::Right);
    until(&mut app, "the hosts", |a| has_row(a, "files.localhost"));
    // Look from web.http, then open what web.pages serves: it won't list
    // its root, but what it exposes is known.
    assert!(app.select_path("/http"));
    press(&mut app, KeyCode::Enter);
    assert_eq!(app.session.route_line(), "web ─/http→ web.http");
    assert!(app.select_path("/www"));
    press(&mut app, KeyCode::Right);
    until(&mut app, "web.pages' exposure", |a| has_row(a, "static/"));
    let s = screen(&mut app);
    assert!(s.contains("web.http's view"), "{s}");
    assert!(s.contains("NAME") && s.contains("OBJECT") && s.contains("ACCESS"), "{s}");
    assert!(s.contains("expose"), "{s}");
    // Up goes back along the route.
    press(&mut app, KeyCode::Backspace);
    assert_eq!(app.session.here, app.session.world.find("web"));
    press(&mut app, KeyCode::Esc);
    assert_eq!(app.session.here, None);
}

#[test]
fn the_runtimes_objects_have_no_view_to_look_from() {
    let mut app = app("static/static.test.oth");
    type_line(&mut app, ":look web.pages");
    leave_command_line(&mut app);
    assert!(app.select_path("/files"));
    press(&mut app, KeyCode::Enter);
    assert_eq!(app.session.here, app.session.world.find("web.pages"));
    assert!(app.log_text().iter().any(|l| l.contains("has no view to look from")), "{:#?}", app.log_text());
}

#[test]
fn requests_are_typed_on_the_command_line() {
    let mut app = app("static/static.test.oth");
    type_line(&mut app, ":look web");
    assert_eq!(app.focus, Focus::Command);
    type_line(&mut app, "read /http/hosts/files.localhost");
    until(&mut app, "the answer", |a| a.log_text().iter().any(|l| l.contains("/www/static")));
    assert!(app.log_text().iter().any(|l| l.contains("web # read /http/hosts/files.localhost")), "{:#?}", app.log_text());
    // Esc goes back to the table, once the choices are closed.
    leave_command_line(&mut app);
    assert_eq!(app.session.here, Some(Viewpoint::Instance(app.session.world.find("web").unwrap().instance().unwrap())));
}

#[test]
fn q_quits() {
    let mut app = app("greeter/greeter.oth");
    assert!(!app.key(KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE)));
}

