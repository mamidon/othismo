use anyhow::{Context, Result};
use real_talk::{pack, runtime, slots};
use std::path::PathBuf;
use std::process::ExitCode;

const USAGE: &str = "usage: real_talk [run] [--check] [--check-shapes] [--with <file>]... [--set <slot.field=value>]... <image>
       real_talk build [-v] [--no-sources] <image.img> -o <out.rti>
       real_talk unpack <image> <dir>

An image is a .img source, which is built in memory, or a built .rti file.

  --check         validate the image, print each instance's view and what
                  it can reach at most, and exit without running it
  --check-shapes  verify every message's fields against its declaration
  --with          give slots values from a file: `listen { address: \"0.0.0.0:80\" }`
  --set           give one slot field a value, after any files; raw text for
                  a string, a literal otherwise
  --no-sources    leave the reference copies of the sources out of a build
  -v              list each packed path that a later include replaced";

/// A command-line mistake: print the usage and exit with status 2.
struct Usage;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "-h" || a == "--help") {
        println!("{USAGE}");
        return ExitCode::SUCCESS;
    }
    match command(&args) {
        Err(Usage) => {
            eprintln!("{USAGE}");
            ExitCode::from(2)
        }
        Ok(Err(e)) => {
            eprintln!("error: {e:#}");
            ExitCode::FAILURE
        }
        Ok(Ok(())) => ExitCode::SUCCESS,
    }
}

fn command(args: &[String]) -> Result<Result<()>, Usage> {
    match args.first().map(String::as_str) {
        Some("build") => build(&args[1..]),
        Some("unpack") => match &args[1..] {
            [image, dir] if !image.starts_with('-') && !dir.starts_with('-') => {
                Ok(pack::load(image.as_ref()).and_then(|p| p.unpack(dir.as_ref())))
            }
            _ => Err(Usage),
        },
        Some("run") => run(&args[1..]),
        _ => run(args),
    }
}

fn build(args: &[String]) -> Result<Result<()>, Usage> {
    let mut options = pack::BuildOptions::default();
    let (mut image, mut out, mut verbose) = (None, None, false);
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--no-sources" => options.no_sources = true,
            "-v" => verbose = true,
            "-o" => out = Some(PathBuf::from(args.next().ok_or(Usage)?)),
            _ if arg.starts_with('-') || image.is_some() => return Err(Usage),
            _ => image = Some(PathBuf::from(arg)),
        }
    }
    let (Some(image), Some(out)) = (image, out) else { return Err(Usage) };
    Ok(pack::build(&image, options).and_then(|p| {
        if verbose {
            p.log.iter().for_each(|line| eprintln!("{line}"));
        }
        std::fs::write(&out, p.seal()).with_context(|| format!("writing {}", out.display()))
    }))
}

fn run(args: &[String]) -> Result<Result<()>, Usage> {
    let mut image = None;
    let mut options = runtime::Options::default();
    let mut check_only = false;
    let (mut files, mut sets) = (vec![], vec![]);
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--check" => check_only = true,
            "--check-shapes" => options.check_shapes = true,
            "--with" => files.push(PathBuf::from(args.next().ok_or(Usage)?)),
            "--set" => sets.push(args.next().ok_or(Usage)?),
            _ if arg.starts_with('-') || image.is_some() => return Err(Usage),
            _ => image = Some(PathBuf::from(arg)),
        }
    }
    let image = image.ok_or(Usage)?;
    Ok(pack::load(&image).and_then(|mut pack| {
        // Files first, in order, then --set: later values win.
        let mut given = vec![];
        for file in &files {
            given.extend(slots::read_values(file)?);
        }
        for set in sets {
            given.push(slots::parse_set(set)?);
        }
        slots::bind(&mut pack.program, &given)?;
        if check_only {
            print!("{}", pack::describe(&pack));
            Ok(())
        } else {
            runtime::run(&pack, runtime::Io::std(), options)
        }
    }))
}
