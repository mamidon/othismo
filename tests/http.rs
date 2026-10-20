//! End-to-end tests for examples/static: the real binary, real sockets, raw
//! HTTP. Needs the AssemblyScript modules built (`make wasm`).

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdout, Command, Stdio};
use std::time::Duration;

struct Server {
    child: Child,
    stdout: BufReader<ChildStdout>,
    port: u16,
    dir: PathBuf,
}

impl Server {
    /// Runs a copy of static.img listening on an ephemeral port. The copy
    /// lives in a temp dir whose messages, scripts and public files are
    /// symlinks back into examples/static.
    fn start(name: &str) -> Server {
        let example = Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/static");
        for m in ["HttpCodec", "StaticServer"] {
            assert!(example.join(format!("scripts/{m}.wasm")).exists(), "run `make wasm` first");
        }
        let dir = std::env::temp_dir().join(format!("real_talk_http_{name}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        for link in ["messages.ts", "scripts", "public"] {
            std::os::unix::fs::symlink(example.join(link), dir.join(link)).unwrap();
        }
        let img = std::fs::read_to_string(example.join("static.img")).unwrap();
        assert!(img.contains("127.0.0.1:8080"));
        std::fs::write(dir.join("static.img"), img.replace("127.0.0.1:8080", "127.0.0.1:0")).unwrap();

        let mut child = Command::new(env!("CARGO_BIN_EXE_real_talk"))
            .arg(dir.join("static.img"))
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let mut stdout = BufReader::new(child.stdout.take().unwrap());
        let mut first = String::new();
        stdout.read_line(&mut first).unwrap();
        // Listening {"address": "127.0.0.1:54321"}
        let port = first
            .trim_end()
            .strip_suffix("\"}")
            .and_then(|s| s.rsplit(':').next())
            .and_then(|p| p.parse().ok())
            .unwrap_or_else(|| panic!("unexpected first line: {first:?}"));
        Server { child, stdout, port, dir }
    }

    fn connect(&self) -> TcpStream {
        let s = TcpStream::connect(("127.0.0.1", self.port)).unwrap();
        s.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
        s
    }

    /// Sends `raw` and reads the whole response (the server always closes).
    fn request(&self, raw: &[u8]) -> Response {
        let mut s = self.connect();
        s.write_all(raw).unwrap();
        read_response(&mut s)
    }

    fn get(&self, path: &str) -> Response {
        self.request(format!("GET {path} HTTP/1.1\r\nHost: test\r\n\r\n").as_bytes())
    }

    /// Next access-log line from the server's stdout.
    fn log_line(&mut self) -> String {
        let mut line = String::new();
        self.stdout.read_line(&mut line).unwrap();
        line.trim_end().to_string()
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

#[derive(Debug)]
struct Response {
    status: u16,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

impl Response {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(n, _)| n.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str())
    }
}

fn read_response(s: &mut TcpStream) -> Response {
    let mut raw = vec![];
    s.read_to_end(&mut raw).unwrap();
    let split = raw.windows(4).position(|w| w == b"\r\n\r\n").expect("complete head");
    let head = String::from_utf8(raw[..split].to_vec()).unwrap();
    let mut lines = head.split("\r\n");
    let status = lines.next().unwrap().split(' ').nth(1).unwrap().parse().unwrap();
    let headers = lines
        .map(|l| {
            let (n, v) = l.split_once(": ").unwrap();
            (n.to_string(), v.to_string())
        })
        .collect();
    Response { status, headers, body: raw[split + 4..].to_vec() }
}

fn public(path: &str) -> Vec<u8> {
    std::fs::read(Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/static/public").join(path)).unwrap()
}

#[test]
fn serves_files_with_types_and_logs_them() {
    let mut srv = Server::start("serve");

    let r = srv.get("/");
    assert_eq!(r.status, 200);
    assert_eq!(r.header("Content-Type"), Some("text/html; charset=utf-8"));
    assert_eq!(r.header("Content-Length"), Some(public("index.html").len().to_string().as_str()));
    assert_eq!(r.header("Connection"), Some("close"));
    assert_eq!(r.body, public("index.html"));
    assert_eq!(srv.log_line(), "GET / 200");

    let r = srv.get("/style.css?v=2");
    assert_eq!((r.status, r.header("Content-Type")), (200, Some("text/css; charset=utf-8")));
    assert_eq!(r.body, public("style.css"));
    assert_eq!(srv.log_line(), "GET /style.css 200");

    let r = srv.get("/pixel.png");
    assert_eq!((r.status, r.header("Content-Type")), (200, Some("image/png")));
    assert_eq!(r.body, public("pixel.png"), "binary bodies survive intact");

    let r = srv.get("/docs/");
    assert_eq!((r.status, r.body), (200, public("docs/index.html")));
}

#[test]
fn error_statuses() {
    let srv = Server::start("errors");
    assert_eq!(srv.get("/missing.html").status, 404);

    let r = srv.get("/docs");
    assert_eq!((r.status, r.header("Location")), (301, Some("/docs/")));

    let r = srv.request(b"POST / HTTP/1.1\r\nContent-Length: 2\r\n\r\nhi");
    assert_eq!((r.status, r.header("Allow")), (405, Some("GET, HEAD")));

    assert_eq!(srv.request(b"nonsense\r\n\r\n").status, 400);
    assert_eq!(srv.request(b"GET /%zz HTTP/1.1\r\n\r\n").status, 400);
    assert_eq!(srv.request(b"GET / HTTP/1.1\r\nno colon here\r\n\r\n").status, 400);
}

#[test]
fn cannot_escape_the_files_root() {
    let srv = Server::start("escape");
    assert_eq!(srv.get("/../Cargo.toml").status, 403);
    assert_eq!(srv.get("/%2e%2e/%2e%2e/Cargo.toml").status, 403);
    assert_eq!(srv.get("/docs/../../static.img").status, 403);
}

#[test]
fn head_omits_the_body() {
    let srv = Server::start("head");
    let r = srv.request(b"HEAD /index.html HTTP/1.1\r\n\r\n");
    assert_eq!(r.status, 200);
    assert_eq!(r.header("Content-Length"), Some(public("index.html").len().to_string().as_str()));
    assert!(r.body.is_empty());
}

#[test]
fn requests_split_across_packets_and_interleaved_connections() {
    let srv = Server::start("split");
    let mut a = srv.connect();
    let mut b = srv.connect();
    a.write_all(b"GET /style.css HTTP/1.1\r\nHo").unwrap();
    b.write_all(b"GET /pixel.png HTTP/1.1\r\n\r\n").unwrap();
    let rb = read_response(&mut b);
    std::thread::sleep(Duration::from_millis(50));
    a.write_all(b"st: x\r\n\r\n").unwrap();
    let ra = read_response(&mut a);
    assert_eq!((ra.status, ra.body), (200, public("style.css")));
    assert_eq!((rb.status, rb.body), (200, public("pixel.png")));
}

#[test]
fn many_concurrent_clients() {
    let srv = Server::start("concurrent");
    let port = srv.port;
    let handles: Vec<_> = (0..16)
        .map(|_| {
            std::thread::spawn(move || {
                for _ in 0..10 {
                    let mut s = TcpStream::connect(("127.0.0.1", port)).unwrap();
                    s.write_all(b"GET /index.html HTTP/1.1\r\n\r\n").unwrap();
                    let r = read_response(&mut s);
                    assert_eq!(r.status, 200);
                    assert_eq!(r.body, public("index.html"));
                }
            })
        })
        .collect();
    for h in handles {
        h.join().unwrap();
    }
}
