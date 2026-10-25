//! The shell on the whole screen: the route bar, one view as a table with
//! its served trees opened beneath it, an inspector for the row selected,
//! what has happened, and the command line.
//!
//! The table only ever shows one view: the image's roots, or the view of
//! where the operator looks from. Opening a row lists what is served there,
//! asked of whoever serves it by the operator ([`Session::look_inside`]).

use crate::complete::{self, Completion};
use crate::session::{Event, Outcome, Session};
use crate::show;
use crate::world::{Class, Hop, Source, Viewpoint, World};
use anyhow::Result;
use protocol::{namespace::Resolved, status::Status, verb::Verb};
use protocol::payload::Stat;
use ratatui::Frame;
use ratatui::crossterm::event::{
    self, DisableMouseCapture, EnableMouseCapture, Event as TermEvent, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton,
    MouseEvent, MouseEventKind,
};
use ratatui::crossterm::execute;
use ratatui::layout::{Constraint, Layout, Position, Rect};
use ratatui::style::{Color, Modifier, Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Cell, Clear, Paragraph, Row as TableRow, Table, TableState, Wrap};
use runtime::operator::How;
use std::collections::{BTreeSet, HashMap};
use std::time::{Duration, Instant};

/// How often the counts in the table are brought up to date.
const REFRESH: Duration = Duration::from_secs(2);
/// The most lines of the transcript kept.
const KEPT: usize = 1000;

mod colour {
    use ratatui::style::Color;
    pub const PATH: Color = Color::Rgb(242, 247, 248);
    pub const DIM: Color = Color::Rgb(110, 128, 134);
    pub const HOST: Color = Color::Rgb(233, 169, 76);
    pub const MEMORY: Color = Color::Rgb(76, 195, 176);
    pub const WASM: Color = Color::Rgb(127, 168, 255);
    pub const IMAGE: Color = Color::Rgb(212, 138, 216);
    pub const RO: Color = Color::Rgb(240, 128, 110);
    pub const OK: Color = Color::Rgb(143, 209, 106);
    pub const WARN: Color = Color::Rgb(232, 207, 90);
    pub const ERR: Color = Color::Rgb(255, 107, 107);
    pub const BAR: Color = Color::Rgb(23, 32, 36);
    pub const SELECTED: Color = Color::Rgb(34, 50, 58);
}

fn class_colour(c: Class) -> Color {
    match c {
        Class::Host => colour::HOST,
        Class::Memory => colour::MEMORY,
        Class::Wasm => colour::WASM,
        Class::Image => colour::IMAGE,
    }
}

fn dim(s: impl Into<String>) -> Span<'static> {
    Span::styled(s.into(), Style::new().fg(colour::DIM))
}

fn styled(s: impl Into<String>, c: Color) -> Span<'static> {
    Span::styled(s.into(), Style::new().fg(c))
}

/// Run `session` on the whole screen until the operator quits.
pub fn run(session: Session, title: &str) -> Result<()> {
    let mut terminal = ratatui::init();
    // The wheel scrolls and the divider drags; holding shift (option in
    // some terminals) still selects text.
    execute!(std::io::stdout(), EnableMouseCapture)?;
    let mut app = App::new(session, title);
    let result = (|| -> Result<()> {
        loop {
            terminal.draw(|f| app.draw(f))?;
            if event::poll(Duration::from_millis(50))? {
                match event::read()? {
                    TermEvent::Key(key) if key.kind != KeyEventKind::Release && !app.key(key) => return Ok(()),
                    TermEvent::Mouse(m) => app.mouse(m),
                    _ => {}
                }
            }
            app.pump(Duration::ZERO);
        }
    })();
    let _ = execute!(std::io::stdout(), DisableMouseCapture);
    ratatui::restore();
    result
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    Tree,
    Command,
}

/// What a served path has been listed as.
#[derive(Debug, Clone)]
enum Listing {
    Waiting,
    Got(Vec<Stat>),
    Failed(Status),
}

/// Where the operator was in each view, to go back to.
#[derive(Debug, Default)]
struct Place {
    open: BTreeSet<String>,
    selected: usize,
}

/// One line of the table.
struct Row {
    /// The path in the view, for rows that name one.
    path: Option<String>,
    /// For the roots: which one.
    root: Option<Viewpoint>,
    /// Where the path leads.
    target: Option<Resolved>,
    /// Its stat record, if it was listed.
    stat: Option<Stat>,
    /// A mount point, rather than something in a served tree.
    mount: Option<How>,
    can_open: bool,
    cells: Vec<Line<'static>>,
}

impl Row {
    fn selectable(&self) -> bool {
        self.path.is_some() || self.root.is_some()
    }
}

pub struct App {
    pub session: Session,
    title: String,
    pub focus: Focus,
    places: HashMap<Option<Viewpoint>, Place>,
    listings: HashMap<(usize, String), Listing>,
    /// Lists in flight: request id → what it lists, and the records so far.
    listing: HashMap<u64, ((usize, String), Vec<Stat>)>,
    rows: Vec<Row>,
    table: TableState,
    log: Vec<Line<'static>>,
    pub input: String,
    cursor: usize,
    history: Vec<String>,
    recalled: Option<usize>,
    /// What can come next on the command line, and which is chosen.
    pub completion: Completion,
    chosen: usize,
    /// The operator closed it: it stays closed until tab, or a new line.
    hidden: bool,
    /// The operator asked for it with tab, though nothing is typed yet.
    asked: bool,
    /// The operator picked one with the keys or the mouse, so ⏎ puts it in.
    picked: bool,
    /// Rows given to the transcript; the table has the rest.
    log_height: u16,
    /// How far the transcript is scrolled back, in rows from its end.
    log_back: usize,
    /// How wide the transcript was last drawn, to keep scrolling steady.
    log_width: usize,
    /// How far the inspector is scrolled down, in rows.
    inspector_down: u16,
    /// Where things were last drawn, for the mouse.
    areas: Areas,
    /// The divider between the table and the transcript is being dragged.
    dragging: bool,
    refreshed: Instant,
    quit: bool,
}

impl App {
    pub fn new(session: Session, title: &str) -> App {
        let mut app = App {
            session,
            title: title.to_string(),
            focus: Focus::Tree,
            places: HashMap::new(),
            listings: HashMap::new(),
            listing: HashMap::new(),
            rows: vec![],
            table: TableState::default(),
            log: vec![],
            input: String::new(),
            cursor: 0,
            history: vec![],
            recalled: None,
            completion: Completion::default(),
            chosen: 0,
            hidden: false,
            asked: false,
            picked: false,
            log_height: 10,
            log_back: 0,
            log_width: 80,
            inspector_down: 0,
            areas: Areas::default(),
            dragging: false,
            refreshed: Instant::now(),
            quit: false,
        };
        app.say(dim(format!("{} · {} roots · :help for commands · : to type one", app.title, app.session.world.roots().len())));
        app.rebuild();
        app
    }

    // ---- events ------------------------------------------------------------------

    /// Handles what has happened in the image, waiting up to `wait` for it.
    pub fn pump(&mut self, wait: Duration) {
        let mut wait = wait;
        while let Some(e) = self.session.next_event(wait) {
            wait = Duration::ZERO;
            self.event(e);
        }
        if self.refreshed.elapsed() > REFRESH && self.session.stopped.is_none() {
            self.session.refresh();
            self.refreshed = Instant::now();
        }
    }

    fn event(&mut self, e: Event) {
        match e {
            Event::Answer { id, answer } => {
                if let Some((_, stats)) = self.listing.get_mut(&id) {
                    if let Some(s) = answer.body.as_ref().and_then(Stat::decode) {
                        stats.push(s);
                    }
                    if answer.done {
                        let (key, stats) = self.listing.remove(&id).expect("just found");
                        let listed = if answer.status == Status::Ok { Listing::Got(stats) } else { Listing::Failed(answer.status) };
                        self.listings.insert(key, listed);
                        self.rebuild();
                        self.recomplete();
                    }
                    return;
                }
                let tag = if answer.done && self.session.open.is_empty() { "  ".to_string() } else { format!("  [{id}] ") };
                let ok = answer.status == Status::Ok;
                for (i, l) in show::answer(&answer, &self.session.formats).into_iter().enumerate() {
                    let style = match (ok, i) {
                        (false, _) => Style::new().fg(colour::ERR),
                        (true, 0) => Style::new().fg(colour::OK),
                        _ => Style::new(),
                    };
                    self.say(Line::from(vec![dim(tag.clone()), Span::styled(l, style)]));
                }
                if answer.done {
                    // What was asked may have changed what is served.
                    self.relist();
                }
            }
            Event::Printed(line) => self.say(Line::from(vec![styled("  console │ ", colour::HOST), Span::raw(line)])),
            Event::Refreshed => {
                self.rebuild();
                self.recomplete();
            }
            Event::Stopped(why) => {
                let why = why.map(|w| format!(": {w}")).unwrap_or_default();
                self.say(styled(format!("the image stopped{why}"), colour::ERR));
            }
        }
    }

    /// Handles a key; false once the operator has quit.
    pub fn key(&mut self, key: KeyEvent) -> bool {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        if ctrl && key.code == KeyCode::Char('c') {
            if self.session.open.is_empty() {
                if self.focus == Focus::Command && !self.input.is_empty() {
                    self.set_input(String::new());
                } else {
                    self.quit = true;
                }
            } else {
                self.cancel_latest();
            }
            return !self.quit;
        }
        match self.focus {
            Focus::Tree => self.tree_key(key),
            Focus::Command => self.command_key(key),
        }
        !self.quit
    }

    fn tree_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Down | KeyCode::Char('j') => self.step(1),
            KeyCode::Up | KeyCode::Char('k') => self.step(-1),
            KeyCode::PageDown => self.step(10),
            KeyCode::PageUp => self.step(-10),
            KeyCode::Right | KeyCode::Char('l') => {
                if !self.set_open(true) && self.selected().is_some_and(|r| r.root.is_some()) {
                    self.look_from_selected();
                }
            }
            KeyCode::Left | KeyCode::Char('h') => {
                if !self.set_open(false) {
                    self.up();
                }
            }
            KeyCode::Enter => self.look_from_selected(),
            KeyCode::Backspace => self.up(),
            KeyCode::Esc => self.go(None, None),
            KeyCode::Char(':') => {
                self.focus = Focus::Command;
                self.hidden = false;
                self.recomplete();
            }
            KeyCode::Char('+') | KeyCode::Char('=') => self.resize(1),
            KeyCode::Char('-') => self.resize(-1),
            KeyCode::Char('r') => {
                self.session.refresh();
                self.listings.clear();
                self.relist();
            }
            KeyCode::Char('q') => self.quit = true,
            _ => {}
        }
    }

    fn command_key(&mut self, key: KeyEvent) {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let choosing = self.popup_open() && !self.completion.items.is_empty();
        let n = self.completion.items.len().max(1);
        let half = (self.areas.log.height / 2).max(1) as isize;
        match key.code {
            KeyCode::Esc if self.popup_open() => {
                self.hidden = true;
                self.picked = false;
            }
            KeyCode::Esc => self.focus = Focus::Tree,
            KeyCode::Tab if !self.popup_open() => {
                self.hidden = false;
                self.asked = true;
            }
            KeyCode::Tab => self.accept(),
            // A choice picked with the keys goes in; otherwise the line is sent.
            KeyCode::Enter if choosing && self.picked => self.accept(),
            KeyCode::Enter => {
                let line = std::mem::take(&mut self.input);
                self.cursor = 0;
                self.recalled = None;
                (self.hidden, self.asked, self.picked) = (false, false, false);
                self.log_back = 0;
                self.submit(&line);
            }
            KeyCode::Down if choosing => {
                self.chosen = (self.chosen + 1) % n;
                self.picked = true;
            }
            KeyCode::Up if choosing => {
                self.chosen = (self.chosen + n - 1) % n;
                self.picked = true;
            }
            KeyCode::Up => self.recall(-1),
            KeyCode::Down => self.recall(1),
            KeyCode::PageUp => self.scroll_log(half),
            KeyCode::PageDown => self.scroll_log(-half),
            KeyCode::Left => self.cursor = self.input[..self.cursor].chars().next_back().map_or(0, |c| self.cursor - c.len_utf8()),
            KeyCode::Right => self.cursor = self.input[self.cursor..].chars().next().map_or(self.cursor, |c| self.cursor + c.len_utf8()),
            KeyCode::Home => self.cursor = 0,
            KeyCode::End => self.cursor = self.input.len(),
            KeyCode::Char('a') if ctrl => self.cursor = 0,
            KeyCode::Char('e') if ctrl => self.cursor = self.input.len(),
            KeyCode::Char('u') if ctrl => self.set_input(String::new()),
            KeyCode::Backspace => {
                if let Some(c) = self.input[..self.cursor].chars().next_back() {
                    self.cursor -= c.len_utf8();
                    self.input.remove(self.cursor);
                }
            }
            KeyCode::Char(c) if !ctrl => {
                self.input.insert(self.cursor, c);
                self.cursor += c.len_utf8();
                self.picked = false;
            }
            _ => {}
        }
        if self.input.is_empty() {
            // A new line: the choices come back once something is typed.
            self.hidden = false;
        }
        if !matches!(key.code, KeyCode::Up | KeyCode::Down) || !choosing {
            self.recomplete();
        }
    }

    /// Whether the choices are showing: once something is typed, or tab
    /// asks for them, until esc closes them.
    pub fn popup_open(&self) -> bool {
        self.focus == Focus::Command
            && !self.hidden
            && (!self.input.is_empty() || self.asked)
            && (!self.completion.items.is_empty() || self.completion.hint.is_some())
    }

    /// Gives the transcript `by` more rows, taking them from the table.
    fn resize(&mut self, by: i16) {
        let most = self.areas.left.height.saturating_sub(7).max(3);
        self.log_height = (self.log_height as i16 + by).clamp(3, most as i16) as u16;
    }

    /// Scrolls the transcript back by `rows`, or on with a negative number.
    fn scroll_log(&mut self, rows: isize) {
        self.log_back = (self.log_back as isize + rows).max(0) as usize;
    }

    /// Handles the mouse: the wheel scrolls what it is over, a click picks a
    /// choice or a row, and the divider above the transcript drags.
    pub fn mouse(&mut self, m: MouseEvent) {
        let at = Position { x: m.column, y: m.row };
        let a = self.areas;
        let on_divider = a.left.contains(at) && at.y == a.log.y.saturating_sub(1);
        match m.kind {
            MouseEventKind::ScrollUp | MouseEventKind::ScrollDown => {
                let up = m.kind == MouseEventKind::ScrollUp;
                if self.popup_open() && a.popup.contains(at) && !self.completion.items.is_empty() {
                    let n = self.completion.items.len();
                    self.chosen = if up { (self.chosen + n - 1) % n } else { (self.chosen + 1) % n };
                    self.picked = true;
                } else if a.log.contains(at) {
                    self.scroll_log(if up { 3 } else { -3 });
                } else if a.inspector.contains(at) {
                    self.inspector_down = if up { self.inspector_down.saturating_sub(3) } else { self.inspector_down + 3 };
                } else if a.table.contains(at) {
                    self.step(if up { -3 } else { 3 });
                }
            }
            MouseEventKind::Down(MouseButton::Left) => {
                if self.popup_open() && a.popup.contains(at) {
                    let i = a.popup_first + at.y.saturating_sub(a.popup.y) as usize;
                    if i < self.completion.items.len() {
                        self.chosen = i;
                        self.accept();
                    }
                } else if on_divider {
                    self.dragging = true;
                } else if a.table.contains(at) && at.y > a.table.y {
                    let i = self.table.offset() + (at.y - a.table.y - 1) as usize;
                    if self.rows.get(i).is_some_and(Row::selectable) {
                        if self.table.selected() == Some(i) && self.focus == Focus::Tree {
                            self.look_from_selected();
                        } else {
                            self.select(i);
                        }
                    }
                    self.focus = Focus::Tree;
                } else if a.command.contains(at) {
                    self.focus = Focus::Command;
                }
            }
            MouseEventKind::Drag(MouseButton::Left) if self.dragging => {
                let bottom = a.command.y;
                let height = bottom.saturating_sub(at.y + 1);
                self.resize(height as i16 - self.log_height as i16);
            }
            MouseEventKind::Up(MouseButton::Left) => self.dragging = false,
            _ => {}
        }
    }

    /// Works out what can come next where the cursor is.
    fn recomplete(&mut self) {
        let listings = &self.listings;
        let listed = |inst: usize, path: &str| match listings.get(&(inst, path.to_string())) {
            Some(Listing::Got(stats)) => Some(stats.clone()),
            Some(Listing::Failed(_)) => Some(vec![]),
            _ => None,
        };
        let c = complete::complete(&self.session, &listed, &self.input[..self.cursor]);
        if c.items.len() != self.completion.items.len() || c.from != self.completion.from {
            self.chosen = 0;
        }
        if let Some((inst, path)) = c.wants.clone() {
            self.list(inst, &path);
        }
        self.completion = c;
        self.chosen = self.chosen.min(self.completion.items.len().saturating_sub(1));
    }

    /// Puts the chosen completion in, and works out what can come after it.
    fn accept(&mut self) {
        self.picked = false;
        let Some(item) = self.completion.items.get(self.chosen).cloned() else { return };
        if item.placeholder {
            return;
        }
        let from = self.completion.from.min(self.cursor);
        self.input.replace_range(from..self.cursor, &item.insert);
        self.cursor = from + item.caret.unwrap_or(item.insert.len());
        self.chosen = 0;
        self.recomplete();
    }

    /// Runs a typed line, as the command line's Enter does.
    pub fn submit(&mut self, line: &str) {
        if line.trim().is_empty() {
            return;
        }
        if self.history.last().map(String::as_str) != Some(line) {
            self.history.push(line.to_string());
        }
        let prompt = self.prompt_spans();
        self.say(Line::from([prompt, vec![styled(line.to_string(), colour::PATH)]].concat()));
        let before = self.session.here;
        match self.session.run_line(line) {
            Ok(Outcome::Nothing) | Ok(Outcome::Sent(_)) => {}
            Ok(Outcome::Said(lines)) => lines.into_iter().for_each(|l| self.say(dim(format!("  {l}")))),
            Ok(Outcome::Quit) => self.quit = true,
            Err(e) => self.say(styled(format!("  ✗ {e:#}"), colour::ERR)),
        }
        if self.session.here != before {
            self.arrived(before);
        }
    }

    fn set_input(&mut self, s: String) {
        self.cursor = s.len();
        self.input = s;
        self.recomplete();
    }

    fn recall(&mut self, by: isize) {
        if self.history.is_empty() {
            return;
        }
        let last = self.history.len() as isize - 1;
        let at = match self.recalled {
            None if by < 0 => last,
            None => return,
            Some(i) => i as isize + by,
        };
        if at > last {
            self.recalled = None;
            self.set_input(String::new());
        } else {
            let at = at.max(0) as usize;
            self.recalled = Some(at);
            self.set_input(self.history[at].clone());
            // Going through history, the arrows stay with it.
            self.hidden = true;
        }
    }

    fn cancel_latest(&mut self) {
        match self.session.cancel(None) {
            Ok(id) => self.say(dim(format!("  cancel {id}"))),
            Err(e) => self.say(styled(format!("  ✗ {e:#}"), colour::ERR)),
        }
    }

    // ---- navigation --------------------------------------------------------------

    fn place(&mut self) -> &mut Place {
        self.places.entry(self.session.here).or_default()
    }

    fn selected(&self) -> Option<&Row> {
        self.table.selected().and_then(|i| self.rows.get(i))
    }

    fn step(&mut self, by: isize) {
        let Some(from) = self.table.selected() else { return };
        let mut at = from as isize;
        let mut best = from;
        for _ in 0..by.unsigned_abs() {
            loop {
                at += by.signum();
                if at < 0 || at as usize >= self.rows.len() {
                    break;
                }
                if self.rows[at as usize].selectable() {
                    best = at as usize;
                    break;
                }
            }
        }
        self.select(best);
    }

    fn select(&mut self, i: usize) {
        if self.table.selected() != Some(i) {
            self.inspector_down = 0;
        }
        self.table.select(Some(i));
        self.place().selected = i;
    }

    /// Opens or closes the selected row; false if that changed nothing.
    fn set_open(&mut self, open: bool) -> bool {
        let Some(row) = self.selected() else { return false };
        let (Some(path), true) = (row.path.clone(), row.can_open) else { return false };
        let target = row.target.clone();
        let place = self.place();
        if place.open.contains(&path) == open {
            return false;
        }
        if open {
            place.open.insert(path);
            if let Some(t) = target {
                self.list(t.inst, &t.path);
            }
        } else {
            place.open.remove(&path);
        }
        self.rebuild();
        true
    }

    /// Asks `inst` what it serves under `path`, unless that is known or asked.
    fn list(&mut self, inst: usize, path: &str) {
        let key = (inst, path.to_string());
        if self.listings.contains_key(&key) {
            return;
        }
        if let Ok(id) = self.session.look_inside(inst, Verb::List, path) {
            self.listings.insert(key.clone(), Listing::Waiting);
            self.listing.insert(id, (key, vec![]));
        }
    }

    /// Lists again what is open here, once what is served may have changed.
    fn relist(&mut self) {
        let waiting: Vec<_> = self.listings.iter().filter(|(_, l)| !matches!(l, Listing::Waiting)).map(|(k, _)| k.clone()).collect();
        for k in waiting {
            self.listings.remove(&k);
        }
        let Some(here) = self.session.here else { return self.rebuild() };
        let open: Vec<String> = self.place().open.iter().cloned().collect();
        for path in open {
            if let Ok(r) = self.session.world.resolve(here, &path) {
                self.list(r.inst, &r.path);
            }
        }
        self.rebuild();
    }

    fn look_from_selected(&mut self) {
        let Some(row) = self.selected() else { return };
        let at = match (row.root, &row.target) {
            (Some(root), _) => root,
            (None, Some(t)) => Viewpoint::Instance(t.inst),
            _ => return,
        };
        let via = row.path.clone();
        if !self.session.world.has_view(at) {
            let name = self.session.world.name(at).to_string();
            self.say(dim(format!("  {name} is one of the runtime's objects: it serves, but has no view to look from")));
            return;
        }
        let before = self.session.here;
        if let Err(e) = self.session.look(at) {
            return self.say(styled(format!("  ✗ {e:#}"), colour::ERR));
        }
        let route = self.session.route();
        let followed = match (via, route.last()) {
            (Some(via), Some(Hop { via: Some(home), .. })) if &via != home => {
                let holder = route.get(route.len().wrapping_sub(2)).map(|h| self.session.world.name(h.at).to_string()).unwrap_or_default();
                format!(" · you followed {via}; it is held at {home} in {holder}")
            }
            _ => String::new(),
        };
        self.say(dim(format!("  ─ now looking from {}{followed}", self.session.route_line())));
        self.arrived(before);
    }

    fn up(&mut self) {
        let before = self.session.here;
        self.session.up();
        self.arrived(before);
    }

    fn go(&mut self, entered: Option<Viewpoint>, here: Option<Viewpoint>) {
        let before = self.session.here;
        self.session.entered = entered;
        self.session.here = here;
        self.arrived(before);
    }

    /// After moving from `before` to wherever the session now looks from.
    fn arrived(&mut self, before: Option<Viewpoint>) {
        if let Some(i) = self.table.selected() {
            self.places.entry(before).or_default().selected = i;
        }
        self.relist();
        let selected = self.place().selected;
        self.table = TableState::default();
        self.rebuild();
        self.select(selected.min(self.rows.len().saturating_sub(1)));
        if !self.selected().is_some_and(Row::selectable) {
            self.step(1);
        }
    }

    // ---- the table ---------------------------------------------------------------

    fn rebuild(&mut self) {
        self.rows = match self.session.here {
            None => self.root_rows(),
            Some(here) => self.view_rows(here),
        };
        let selected = self.table.selected().unwrap_or(0).min(self.rows.len().saturating_sub(1));
        if self.rows.get(selected).is_some_and(Row::selectable) {
            self.table.select(Some(selected));
        } else {
            let first = self.rows.iter().position(Row::selectable);
            self.table.select(first);
        }
    }

    pub const ROOT_COLUMNS: [&'static str; 6] = ["ROOT", "TYPE", "KIND", "VIEW", "HOLDS", "REQUESTS"];
    pub const VIEW_COLUMNS: [&'static str; 9] = ["NAME", "OBJECT", "KIND", "LINK", "ACCESS", "TYPE", "SIZE", "VER", "INFO"];

    fn root_rows(&self) -> Vec<Row> {
        let w = &self.session.world;
        let rows: Vec<Row> = w
            .roots()
            .into_iter()
            .map(|r| {
                let held = w.descendants(r);
                let holds: Vec<Span> = held
                    .iter()
                    .enumerate()
                    .flat_map(|(n, &i)| {
                        let sep = if n == 0 { None } else { Some(dim(", ")) };
                        sep.into_iter().chain([styled(w.instance(i).name.clone(), class_colour(w.class(i)))])
                    })
                    .collect();
                let mounts = w.view(r).len();
                let (name, ty, kind, requests) = match r {
                    Viewpoint::Root => (styled("/", colour::PATH), Span::raw(format!("{}'s own view", self.title)), dim("—"), dim("—")),
                    Viewpoint::Instance(i) => (
                        styled(w.name(r).to_string(), class_colour(w.class(i))),
                        Span::raw(type_name(w, i)),
                        dim(w.class(i).name()),
                        requests(w, i),
                    ),
                };
                Row {
                    path: None,
                    root: Some(r),
                    target: None,
                    stat: None,
                    mount: None,
                    can_open: false,
                    cells: vec![
                        Line::from(name.add_modifier(Modifier::BOLD)),
                        Line::from(ty),
                        Line::from(kind),
                        Line::from(dim(format!("{mounts} mount{}", if mounts == 1 { "" } else { "s" }))),
                        if holds.is_empty() { Line::from(dim("nothing")) } else { Line::from(holds) },
                        Line::from(requests),
                    ],
                }
            })
            .collect();
        rows
    }

    fn view_rows(&self, here: Viewpoint) -> Vec<Row> {
        let w = &self.session.world;
        let title = if here == Viewpoint::Root { self.title.clone() } else { w.name(here).to_string() };
        let mut rows = vec![Row {
            path: None,
            root: None,
            target: None,
            stat: None,
            mount: None,
            can_open: false,
            cells: vec![
                Line::from(styled("/", colour::PATH).add_modifier(Modifier::BOLD)),
                Line::from(styled(title, here.instance().map_or(colour::PATH, |i| class_colour(w.class(i))))),
            ],
        }];
        if w.view(here).is_empty() {
            rows.push(note("└── ".into(), "(nothing mounted: it can't make any requests)"));
        }
        let mut links: Vec<_> = w.view(here).iter().collect();
        links.sort_by(|a, b| a.at.cmp(&b.at));
        let tree = Tree::of(links.iter().map(|l| l.at.as_str()));
        self.walk_view(here, &tree, "", "", &mut rows);
        rows
    }

    fn walk_view(&self, here: Viewpoint, tree: &Tree, path: &str, guide: &str, rows: &mut Vec<Row>) {
        let w = &self.session.world;
        let open = self.places.get(&Some(here)).map(|p| &p.open);
        let n = tree.kids.len();
        for (k, (name, kid)) in tree.kids.iter().enumerate() {
            let last = k + 1 == n;
            let (pre, more) = (format!("{guide}{}", if last { "└── " } else { "├── " }), format!("{guide}{}", if last { "    " } else { "│   " }));
            let p = format!("{path}/{name}");
            if !kid.mount {
                rows.push(Row {
                    path: None,
                    root: None,
                    target: None,
                    stat: None,
                    mount: None,
                    can_open: false,
                    cells: vec![Line::from(vec![dim(pre), Span::raw("  "), styled(format!("{name}/"), colour::PATH)])],
                });
                self.walk_view(here, kid, &p, &more, rows);
                continue;
            }
            let link = w.view(here).iter().find(|l| l.at == p).expect("a mount point of the view");
            let t = link.target.inst;
            let is_open = open.is_some_and(|o| o.contains(&p));
            let how = match link.how {
                How::Mount => Span::raw("mount"),
                How::Bind => Span::raw("bind"),
                How::Runtime => styled("+ run time", colour::OK),
            };
            let info = requests(w, t);
            rows.push(Row {
                path: Some(p.clone()),
                root: None,
                target: Some(link.target.clone()),
                stat: None,
                mount: Some(link.how),
                can_open: true,
                cells: vec![
                    Line::from(vec![dim(pre), dim(if is_open { "▾ " } else { "▸ " }), styled(name.clone(), colour::PATH).add_modifier(Modifier::BOLD)]),
                    Line::from(styled(w.instance(t).name.clone(), class_colour(w.class(t)))),
                    Line::from(dim(w.class(t).name())),
                    Line::from(how),
                    access(link.target.read_only),
                    Line::from(Span::raw(type_name(w, t))),
                    Line::default(),
                    Line::default(),
                    Line::from(info),
                ],
            });
            if is_open {
                self.walk_served(here, &p, &more, rows);
            }
            // Mount points under this one, which take precedence there.
            self.walk_view(here, kid, &p, &more, rows);
        }
    }

    fn walk_served(&self, here: Viewpoint, path: &str, guide: &str, rows: &mut Vec<Row>) {
        let w = &self.session.world;
        let Ok(r) = w.resolve(here, path) else { return };
        let open = self.places.get(&Some(here)).map(|p| &p.open);
        // What a listing says is there, then what is known without one: what
        // it has exposed, which its server never sees requests for, and what
        // its header says it serves.
        let mut entries: Vec<(Stat, Source)> = match self.listings.get(&(r.inst, r.path.clone())) {
            Some(Listing::Got(stats)) => stats.iter().map(|s| (s.clone(), Source::Listed)).collect(),
            _ => vec![],
        };
        let (known, patterns) = w.known_under(r.inst, &r.path);
        for k in known {
            match entries.iter_mut().find(|(s, _)| s.name == k.name) {
                Some((s, _)) => s.dir |= k.dir,
                None => entries.push((if k.dir { Stat::dir(&k.name) } else { Stat::file(&k.name, 0, "") }, k.from)),
            }
        }
        let status = match self.listings.get(&(r.inst, r.path.clone())) {
            None | Some(Listing::Waiting) => Some("listing…".to_string()),
            Some(Listing::Failed(s)) => Some(format!("not listed: {}", s.name())),
            Some(Listing::Got(_)) if entries.is_empty() && patterns.is_empty() => Some("empty".to_string()),
            Some(Listing::Got(_)) => None,
        };
        let total = entries.len() + patterns.len() + status.is_some() as usize;
        let branch = |k: usize| format!("{guide}{}", if k + 1 == total { "└── " } else { "├── " });
        if let Some(status) = status {
            rows.push(note(branch(0), &format!("({status})")));
        }
        entries.sort_by(|a, b| a.0.name.cmp(&b.0.name));
        let offset = total - entries.len() - patterns.len();
        for (k, (s, source)) in entries.iter().enumerate() {
            let last = offset + k + 1 == total;
            let (pre, more) = (format!("{guide}{}", if last { "└── " } else { "├── " }), format!("{guide}{}", if last { "    " } else { "│   " }));
            let p = if path.ends_with('/') { format!("{path}{}", s.name) } else { format!("{path}/{}", s.name) };
            let target = w.resolve(here, &p).ok();
            let exposed = target.as_ref().is_some_and(|t| t.inst != r.inst);
            let is_open = s.dir && open.is_some_and(|o| o.contains(&p));
            let owner = target.as_ref().map_or(r.inst, |t| t.inst);
            let name = if s.dir { format!("{}/", s.name) } else { s.name.clone() };
            let marker = if s.dir { if is_open { "▾ " } else { "▸ " } } else { "  " };
            let object = if exposed { styled(w.instance(owner).name.clone(), class_colour(w.class(owner))) } else { dim(w.instance(owner).name.clone()) };
            let info = match source {
                Source::Listed => Line::default(),
                Source::Exposed => Line::from(dim("never reaches its server")),
                Source::Declared => Line::from(dim("from its header")),
            };
            rows.push(Row {
                path: Some(p.clone()),
                root: None,
                target: target.clone(),
                stat: (*source == Source::Listed).then(|| s.clone()),
                mount: None,
                can_open: s.dir,
                cells: vec![
                    Line::from(vec![dim(pre), dim(marker), Span::raw(name)]),
                    Line::from(object),
                    Line::from(dim(if s.dir { "dir" } else { "file" })),
                    Line::from(if exposed { Span::raw("expose") } else { Span::raw("") }),
                    access(target.as_ref().is_some_and(|t| t.read_only)),
                    Line::from(Span::raw(s.ty.clone())),
                    Line::from(Span::raw(if s.dir || s.length == 0 { String::new() } else { size(s.length) })).right_aligned(),
                    Line::from(dim(version(s.version))).right_aligned(),
                    info,
                ],
            });
            if is_open {
                self.walk_served(here, &p, &more, rows);
            }
        }
        for (k, pattern) in patterns.iter().enumerate() {
            let pre = branch(offset + entries.len() + k);
            let mut row = note(pre, pattern.doc.lines().next().unwrap_or_default());
            row.cells[0].spans.push(Span::styled(format!("  {}", pattern.segment), Style::new().fg(colour::DIM).add_modifier(Modifier::ITALIC)));
            rows.push(row);
        }
    }

    // ---- drawing -----------------------------------------------------------------

    pub fn draw(&mut self, f: &mut Frame) {
        let [route, pov, body, status] =
            Layout::vertical([Constraint::Length(1), Constraint::Length(1), Constraint::Min(8), Constraint::Length(1)]).areas(f.area());
        f.render_widget(Paragraph::new(self.route_line()).style(Style::new().bg(colour::BAR)), route);
        f.render_widget(Paragraph::new(dim(self.pov_line())).style(Style::new().bg(colour::BAR)), pov);
        // The inspector has its side to itself, top to bottom.
        let (left, inspector) = if body.width >= 110 {
            let [l, i] = Layout::horizontal([Constraint::Fill(1), Constraint::Length(42)]).areas(body);
            (l, Some(i))
        } else {
            (body, None)
        };
        let log_height = self.log_height.clamp(3, left.height.saturating_sub(7).max(3));
        let [table, log, command] =
            Layout::vertical([Constraint::Min(4), Constraint::Length(log_height), Constraint::Length(1)]).areas(left);
        self.areas = Areas { left, table, log, command, inspector: inspector.unwrap_or_default(), ..Areas::default() };
        self.draw_table(f, table);
        if let Some(i) = inspector {
            self.draw_inspector(f, i);
        }
        self.draw_log(f, log);
        self.draw_command(f, command);
        f.render_widget(Paragraph::new(self.status_line()).style(Style::new().bg(colour::BAR)), status);
        if self.popup_open() {
            self.draw_popup(f, command);
        }
    }

    /// What can come next, above the command line: the choices, and what
    /// the chosen one is.
    fn draw_popup(&mut self, f: &mut Frame, command: Rect) {
        let c = self.completion.clone();
        let width = command.width.saturating_sub(4).min(110);
        let x = command.x + 2;
        if c.items.is_empty() {
            let text = c.hint.clone().unwrap_or_default();
            let height = 3;
            let area = Rect { x, y: command.y.saturating_sub(height), width, height };
            f.render_widget(Clear, area);
            f.render_widget(Paragraph::new(text).wrap(Wrap { trim: true }).block(Block::bordered().border_style(Style::new().fg(colour::DIM))), area);
            return;
        }
        let shown = c.items.len().min(10);
        // Room for the chosen one's documentation, wrapped to half the width.
        let doc_lines = c.items.get(self.chosen).map_or(0, |it| {
            let per = (width as usize / 2).saturating_sub(2).max(1);
            let wrapped: usize = it.doc.lines().map(|l| l.chars().count().div_ceil(per).max(1)).sum();
            1 + (wrapped > 0) as usize + wrapped + (!it.source.is_empty()) as usize
        });
        let height = (shown.max(doc_lines).min(14) as u16 + 2).min(command.y);
        let area = Rect { x, y: command.y.saturating_sub(height), width, height };
        f.render_widget(Clear, area);
        let block = Block::bordered().border_style(Style::new().fg(colour::DIM)).title_bottom(dim(" tab complete · ↑↓ choose · ⏎ send · esc close "));
        let inner = block.inner(area);
        f.render_widget(block, area);
        let [list, doc] = Layout::horizontal([Constraint::Percentage(50), Constraint::Percentage(50)]).spacing(1).areas(inner);
        let first = self.chosen.saturating_sub(shown - 1);
        (self.areas.popup, self.areas.popup_first) = (list, first);
        let lines: Vec<Line> = c.items[first..first + shown]
            .iter()
            .enumerate()
            .map(|(n, it)| {
                let mut label = Style::new().fg(if it.usable { colour::PATH } else { colour::DIM });
                if it.placeholder {
                    label = label.add_modifier(Modifier::ITALIC);
                }
                let room = (list.width as usize).saturating_sub(it.label.chars().count() + 2);
                let detail: String = it.detail.chars().take(room).collect();
                let pad = room.saturating_sub(detail.chars().count());
                let mut line = Line::from(vec![Span::styled(it.label.clone(), label), Span::raw(" ".repeat(pad + 1)), dim(detail)]);
                if first + n == self.chosen {
                    line = line.style(Style::new().bg(colour::SELECTED));
                }
                line
            })
            .collect();
        f.render_widget(Paragraph::new(lines), list);
        if let Some(it) = c.items.get(self.chosen) {
            let mut text = vec![Line::from(styled(if it.signature.is_empty() { it.label.clone() } else { it.signature.clone() }, colour::PATH))];
            if !it.doc.is_empty() {
                text.push(Line::default());
                text.extend(it.doc.lines().map(|l| Line::from(l.to_string())));
            }
            if !it.source.is_empty() {
                text.push(Line::from(dim(format!("from {}", it.source))));
            }
            f.render_widget(Paragraph::new(text).wrap(Wrap { trim: false }), doc);
        }
    }

    fn route_line(&self) -> Line<'static> {
        let w = &self.session.world;
        let image = Span::styled(format!(" ◆ {} ", self.title), Style::new().fg(if self.session.here.is_none() { colour::WARN } else { colour::DIM }));
        let mut spans = vec![image];
        let route = self.session.route();
        for (n, h) in route.iter().enumerate() {
            match &h.via {
                None => spans.push(dim("› ")),
                Some(via) => {
                    spans.push(dim(" ─"));
                    spans.push(styled(via.clone(), colour::PATH));
                    if h.read_only {
                        spans.push(styled(" ro", colour::RO));
                    }
                    spans.push(dim("→ "));
                }
            }
            let c = h.at.instance().map_or(colour::PATH, |i| class_colour(w.class(i)));
            let mut style = Style::new().fg(c);
            if n + 1 == route.len() {
                style = style.add_modifier(Modifier::BOLD | Modifier::REVERSED);
            }
            spans.push(Span::styled(format!(" {} ", w.name(h.at)), style));
        }
        Line::from(spans)
    }

    fn pov_line(&self) -> String {
        let w = &self.session.world;
        match self.session.here {
            None => format!(" looking from nowhere · {} roots: the image's own view, and every instance nothing mounts", w.roots().len()),
            Some(Viewpoint::Root) => format!(" {}'s own view · its needs, filled by the host · sends as the operator", self.title),
            Some(at @ Viewpoint::Instance(i)) => format!(
                " {}'s view · {} · {} · {}",
                w.name(at),
                type_name(w, i),
                w.class(i).name(),
                if w.is_operator(at) { "sends as the operator" } else { "sends as itself, never as the operator" }
            ),
        }
    }

    fn status_line(&self) -> Line<'static> {
        let open = self.session.open.len();
        let left = format!(" {} · pid {} · {open} open request{}    ", self.title, std::process::id(), if open == 1 { "" } else { "s" });
        let keys = match self.focus {
            Focus::Tree => "↑↓ move  →← open/close  ⏎ look from  ⌫ up  esc roots  : command  + − resize  r refresh  q quit",
            Focus::Command => "tab complete  ⏎ send  ↑↓ history (or choose)  PgUp PgDn scroll  esc close, then back  ^C cancel stream",
        };
        Line::from(vec![Span::raw(left), dim(keys)])
    }

    fn draw_table(&mut self, f: &mut Frame, area: Rect) {
        let columns: &[&str] = if self.session.here.is_some() { &Self::VIEW_COLUMNS } else { &Self::ROOT_COLUMNS };
        let mut widths: Vec<u16> = columns.iter().map(|c| c.len() as u16).collect();
        for r in &self.rows {
            for (i, c) in r.cells.iter().enumerate().take(columns.len()) {
                // A row with only its first cell is a note, which may run on.
                if r.cells.len() > 1 || i > 0 {
                    widths[i] = widths[i].max(c.width() as u16);
                }
            }
        }
        let header = TableRow::new(columns.iter().enumerate().map(|(i, c)| {
            let cell = Line::from(*c).style(Style::new().fg(colour::DIM).add_modifier(Modifier::BOLD));
            Cell::from(if columns[i] == "SIZE" || columns[i] == "VER" { cell.right_aligned() } else { cell })
        }));
        let rows = self.rows.iter().map(|r| TableRow::new(r.cells.iter().cloned().map(Cell::from)));
        let mut constraints: Vec<Constraint> = widths.iter().map(|&w| Constraint::Length(w)).collect();
        if let Some(last) = constraints.last_mut() {
            *last = Constraint::Fill(1);
        }
        let table = Table::new(rows, constraints)
            .header(header)
            .column_spacing(2)
            .row_highlight_style(Style::new().bg(colour::SELECTED))
            // Its bottom border divides it from the transcript, and drags.
            .block(
                Block::new()
                    .borders(Borders::BOTTOM)
                    .border_style(Style::new().fg(if self.dragging { colour::WARN } else { colour::DIM }))
                    .title_bottom(Line::from(dim(" drag, or + −, to resize ")).right_aligned()),
            );
        f.render_stateful_widget(table, area, &mut self.table);
    }

    fn draw_inspector(&self, f: &mut Frame, area: Rect) {
        let w = &self.session.world;
        let mut lines: Vec<Line> = vec![];
        let heading = |s: &str| Line::from(Span::styled(s.to_string(), Style::new().fg(colour::DIM).add_modifier(Modifier::BOLD)));
        let kv = |k: &str, v: Span<'static>| Line::from(vec![dim(format!("{k:<8}")), v]);
        match self.selected() {
            None => lines.push(Line::from(dim("Nothing selected."))),
            Some(Row { root: Some(r), .. }) => {
                lines.push(Line::from(styled(w.name(*r).to_string(), colour::PATH).add_modifier(Modifier::BOLD)));
                lines.push(Line::default());
                lines.push(heading("ROOT"));
                if let Viewpoint::Instance(i) = r {
                    lines.extend(object_lines(w, *i));
                } else {
                    lines.push(Line::from("The image's own view: its needs, filled by the host, and what it mounts."));
                }
                lines.push(Line::default());
                lines.push(Line::from(dim("⏎ to look from it")));
                let others: Vec<String> = (0..w.instances().len())
                    .filter(|&i| w.has_view(Viewpoint::Instance(i)) && !w.roots().contains(&Viewpoint::Instance(i)))
                    .map(|i| w.instance(i).name.clone())
                    .collect();
                if !others.is_empty() {
                    lines.push(Line::default());
                    lines.push(heading("NOT ROOTS"));
                    lines.push(Line::from(dim(format!("Something mounts {}: look from the root that holds them.", others.join(", ")))));
                }
            }
            Some(row) => {
                let path = row.path.clone().unwrap_or_default();
                lines.push(Line::from(styled(path, colour::PATH).add_modifier(Modifier::BOLD)));
                if let Some(t) = &row.target {
                    let ro = if t.read_only { styled(" ro", colour::RO) } else { Span::raw("") };
                    lines.push(Line::from(vec![dim("answered by "), styled(w.instance(t.inst).name.clone(), class_colour(w.class(t.inst))), dim(format!(" at {}", t.path)), ro]));
                }
                if let Some(s) = &row.stat {
                    lines.push(Line::default());
                    lines.push(heading("STAT"));
                    lines.push(kv("name", Span::raw(s.name.clone())));
                    lines.push(kv("kind", Span::raw(if s.dir { "dir" } else { "file" })));
                    lines.push(kv("length", Span::raw(s.length.to_string())));
                    lines.push(kv("type", if s.ty.is_empty() { dim("—") } else { Span::raw(s.ty.clone()) }));
                    lines.push(kv("version", if s.version == 0 { dim("—") } else { Span::raw(s.version.to_string()) }));
                }
                if let Some(t) = &row.target {
                    lines.push(Line::default());
                    if row.mount.is_some() {
                        lines.push(heading("OBJECT"));
                        lines.extend(object_lines(w, t.inst));
                    } else {
                        let iface = &w.instance(t.inst).interface;
                        lines.push(heading("INTERFACE"));
                        match iface.serving(&t.path) {
                            Some(e) => {
                                lines.push(Line::from(styled(e.to_string(), colour::PATH)));
                                if !e.doc.is_empty() {
                                    lines.push(Line::from(e.doc.clone()));
                                }
                            }
                            None => lines.push(Line::from(dim("Its header says nothing about this path."))),
                        }
                    }
                    if w.has_view(Viewpoint::Instance(t.inst)) {
                        lines.push(Line::default());
                        lines.push(Line::from(dim(format!("⏎ to look from {}", w.instance(t.inst).name))));
                    }
                }
            }
        }
        let block = Block::new().borders(Borders::LEFT).border_style(Style::new().fg(colour::DIM));
        let inner = block.inner(area).inner(ratatui::layout::Margin { horizontal: 1, vertical: 0 });
        f.render_widget(block, area);
        f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }).scroll((self.inspector_down, 0)), inner);
    }

    /// The transcript, its long lines wrapped, scrolled back `log_back` rows.
    fn draw_log(&mut self, f: &mut Frame, area: Rect) {
        self.log_width = area.width.max(1) as usize;
        let rows: Vec<Line> = self.log.iter().flat_map(|l| wrap(l, self.log_width)).collect();
        let shown = area.height as usize;
        self.log_back = self.log_back.min(rows.len().saturating_sub(shown));
        let end = rows.len() - self.log_back;
        f.render_widget(Paragraph::new(rows[end.saturating_sub(shown)..end].to_vec()), area);
        if self.log_back > 0 {
            let note = format!(" ↓ {} more · wheel or PgDn ", self.log_back);
            let w = (note.chars().count() as u16).min(area.width);
            let at = Rect { x: area.right() - w, y: area.bottom() - 1, width: w, height: 1 };
            f.render_widget(Paragraph::new(note).style(Style::new().fg(colour::WARN).bg(colour::BAR)), at);
        }
    }

    fn draw_command(&self, f: &mut Frame, area: Rect) {
        let mut spans = self.prompt_spans();
        let prompt_width: usize = spans.iter().map(|s| s.width()).sum();
        if self.focus == Focus::Tree && self.input.is_empty() {
            spans.push(dim("press : to type a request, such as  read /path  or  :help"));
        } else {
            spans.push(styled(self.input.clone(), colour::PATH));
        }
        f.render_widget(Paragraph::new(Line::from(spans)), area);
        if self.focus == Focus::Command {
            let x = area.x + (prompt_width + self.input[..self.cursor].chars().count()) as u16;
            f.set_cursor_position((x.min(area.right().saturating_sub(1)), area.y));
        }
    }

    fn prompt_spans(&self) -> Vec<Span<'static>> {
        let w = &self.session.world;
        match self.session.here {
            None => vec![dim(format!("{} · ", self.title))],
            Some(at) => {
                let c = at.instance().map_or(colour::PATH, |i| class_colour(w.class(i)));
                let name = if at == Viewpoint::Root { self.title.clone() } else { w.name(at).to_string() };
                let (mark, mc) = if w.is_operator(at) { ("#", colour::HOST) } else { ("$", colour::WASM) };
                vec![styled(name, c).add_modifier(Modifier::BOLD), Span::raw(" "), styled(mark, mc).add_modifier(Modifier::BOLD), Span::raw(" ")]
            }
        }
    }

    fn say(&mut self, line: impl Into<Line<'static>>) {
        let line = line.into();
        // Scrolled back, what is being read stays put as more arrives.
        if self.log_back > 0 {
            self.log_back += wrap(&line, self.log_width).len();
        }
        self.log.push(line);
        if self.log.len() > KEPT {
            self.log.drain(..self.log.len() - KEPT);
        }
    }

    /// The table's rows as text, column by column, for tests.
    pub fn table_text(&self) -> Vec<Vec<String>> {
        self.rows.iter().map(|r| r.cells.iter().map(|c| c.to_string()).collect()).collect()
    }

    /// The transcript as text, for tests.
    pub fn log_text(&self) -> Vec<String> {
        self.log.iter().map(|l| l.to_string()).collect()
    }

    /// Selects the row for `path` in the view, for tests.
    pub fn select_path(&mut self, path: &str) -> bool {
        match self.rows.iter().position(|r| r.path.as_deref() == Some(path)) {
            Some(i) => {
                self.select(i);
                true
            }
            None => false,
        }
    }
}

/// A line of the tree that names nothing: `branch` in the first column, and
/// what it says in the last, which has room.
/// Where the parts of the screen were last drawn.
#[derive(Debug, Clone, Copy, Default)]
struct Areas {
    left: Rect,
    table: Rect,
    log: Rect,
    command: Rect,
    inspector: Rect,
    /// The completion's list, and the index of its first line.
    popup: Rect,
    popup_first: usize,
}

/// `line` cut into rows of at most `width` characters.
fn wrap(line: &Line<'static>, width: usize) -> Vec<Line<'static>> {
    let mut rows = vec![];
    let (mut row, mut used): (Vec<Span<'static>>, usize) = (vec![], 0);
    for span in &line.spans {
        let mut rest: &str = &span.content;
        while !rest.is_empty() {
            if used == width {
                rows.push(Line::from(std::mem::take(&mut row)));
                used = 0;
            }
            let cut = rest.char_indices().nth(width - used).map_or(rest.len(), |(i, _)| i);
            row.push(Span::styled(rest[..cut].to_string(), span.style));
            used += rest[..cut].chars().count();
            rest = &rest[cut..];
        }
    }
    rows.push(Line::from(row));
    rows
}

fn note(branch: String, text: &str) -> Row {
    let mut cells = vec![Line::from(dim(branch))];
    cells.extend((0..7).map(|_| Line::default()));
    cells.push(Line::from(dim(text.to_string())));
    Row { path: None, root: None, target: None, stat: None, mount: None, can_open: false, cells }
}

fn access(read_only: bool) -> Line<'static> {
    Line::from(if read_only { styled("ro", colour::RO) } else { dim("rw") })
}

fn type_name(w: &World, i: usize) -> String {
    match w.class(i) {
        Class::Image => "image".into(),
        _ => w.instance(i).type_name.clone(),
    }
}

fn requests(w: &World, i: usize) -> Span<'static> {
    let inst = w.instance(i);
    match (inst.served, inst.pending) {
        (0, _) => dim("—"),
        (n, 0) => dim(format!("{n} req")),
        (n, open) => Span::raw(format!("{n} req · {open} open")),
    }
}

/// A version, which only ever needs telling apart from the last one: its
/// low 32 bits in hex. The inspector shows all of it.
fn version(v: u64) -> String {
    match v {
        0 => String::new(),
        1..10_000 => v.to_string(),
        _ => format!("{:08x}", v as u32),
    }
}

fn size(bytes: u64) -> String {
    match bytes {
        0..1024 => format!("{bytes} B"),
        1024..1_048_576 => format!("{:.1} KB", bytes as f64 / 1024.0),
        _ => format!("{:.1} MB", bytes as f64 / 1_048_576.0),
    }
}

/// An object's type, kind, requests, doc and what it serves.
fn object_lines(w: &World, i: usize) -> Vec<Line<'static>> {
    let inst = w.instance(i);
    let mut lines = vec![
        Line::from(vec![dim(format!("{:<8}", "name")), styled(inst.name.clone(), class_colour(w.class(i)))]),
        Line::from(vec![dim(format!("{:<8}", "type")), Span::raw(type_name(w, i))]),
        Line::from(vec![dim(format!("{:<8}", "kind")), Span::raw(w.class(i).name())]),
        Line::from(vec![dim(format!("{:<8}", "served")), requests(w, i)]),
    ];
    let iface = &inst.interface;
    if !iface.doc.is_empty() {
        lines.push(Line::default());
        lines.extend(iface.doc.lines().map(|l| Line::from(l.to_string())));
    }
    if !iface.serves.is_empty() {
        lines.push(Line::default());
        lines.push(Line::from(Span::styled("SERVES", Style::new().fg(colour::DIM).add_modifier(Modifier::BOLD))));
        for e in &iface.serves {
            lines.push(Line::from(styled(e.to_string(), colour::PATH)));
            if !e.doc.is_empty() {
                lines.push(Line::from(dim(e.doc.clone())));
            }
        }
    }
    lines
}

/// Mount points, as a tree of their segments.
#[derive(Default)]
struct Tree {
    kids: Vec<(String, Tree)>,
    mount: bool,
}

impl Tree {
    fn of<'a>(points: impl Iterator<Item = &'a str>) -> Tree {
        let mut t = Tree::default();
        for p in points {
            let mut node = &mut t;
            for seg in p.split('/').filter(|s| !s.is_empty()) {
                let i = match node.kids.iter().position(|(n, _)| n == seg) {
                    Some(i) => i,
                    None => {
                        node.kids.push((seg.to_string(), Tree::default()));
                        node.kids.len() - 1
                    }
                };
                node = &mut node.kids[i].1;
            }
            node.mount = true;
        }
        t
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn long_lines_wrap_keeping_their_styles() {
        let line = Line::from(vec![styled("abcd", colour::OK), Span::raw("efghij")]);
        let rows: Vec<String> = wrap(&line, 4).iter().map(|l| l.to_string()).collect();
        assert_eq!(rows, ["abcd", "efgh", "ij"]);
        assert_eq!(wrap(&line, 4)[0].spans[0].style.fg, Some(colour::OK));
        assert_eq!(wrap(&Line::default(), 4).len(), 1, "an empty line is still a row");
    }
}
