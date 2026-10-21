#![allow(dead_code)]

use real_talk::runtime::{self, Io, Options};
use std::cell::RefCell;
use std::io::{Cursor, Write};
use std::path::{Path, PathBuf};
use std::rc::Rc;

/// An output sink the test can read back after the runtime is done with it.
#[derive(Clone, Default)]
pub struct Captured(Rc<RefCell<Vec<u8>>>);

impl Captured {
    pub fn text(&self) -> String {
        String::from_utf8(self.0.borrow().clone()).unwrap()
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

/// Loads `image` and runs it with `input` as stdin; returns what it printed.
pub fn run_image(image: &Path, input: &str, options: Options) -> anyhow::Result<String> {
    let pack = real_talk::pack::load(image)?;
    let out = Captured::default();
    let io = Io { input: Box::new(Cursor::new(input.as_bytes().to_vec())), output: Box::new(out.clone()) };
    runtime::run(&pack, io, options)?;
    Ok(out.text())
}

/// A scratch directory holding an image, its messages and its scripts.
pub struct Fixture {
    pub dir: PathBuf,
}

impl Fixture {
    pub fn new(name: &str) -> Fixture {
        let dir = std::env::temp_dir().join(format!("real_talk_{name}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("scripts")).unwrap();
        Fixture { dir }
    }

    pub fn file(&self, rel: &str, contents: impl AsRef<[u8]>) -> &Self {
        std::fs::write(self.dir.join(rel), contents).unwrap();
        self
    }

    pub fn image(&self) -> PathBuf {
        self.dir.join("test.img")
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}
