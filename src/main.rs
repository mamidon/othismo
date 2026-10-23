use anyhow::{Context, Result};
use othismo::format::{Bson, Format, Messages};
use othismo::{doc, iface, pack, runtime, slots, suite};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

const USAGE: &str = "usage: othismo [run] [--check] [--no-checks] [--with <file>]... [--set <slot[.field]=value>]... <image>
       othismo test [--no-checks] <image>
       othismo build [-v] [--no-sources] <image.oth> -o <out.rti>
       othismo unpack <image> <dir>
       othismo bindings [--library <path>] <schemas.messages> [-o <out.ts>]
       othismo embed <header.oth> <module.wasm>
       othismo doc <module.wasm | header.oth | built-in type | env>

An image is a .oth source, which is built in memory, or a built .rti file.

  --check         validate the image, print each instance's view and what
                  it can reach at most, and exit without running it
  --no-checks     don't hold objects to their interfaces while they run
  --with          give slots values from a file: `listen = \"0.0.0.0:80\"`, or
                  `db { port: 5432 }` for a message
  --set           give a slot, or one field of a message slot, a value, after
                  any files; raw text for a payload or a string, a literal
                  otherwise
  --no-sources    leave the reference copies of the sources out of a build
  -v              list each packed path that a later include replaced

`test` runs the image's tests: each instance whose type declares TestSuite
lists its tests, and each test runs in a fresh run of the image, with nothing
outside started. What the image prints is shown for the tests that fail.

`bindings` writes AssemblyScript classes for the message types a schema file
declares, to <schemas.messages>.ts unless -o says otherwise; a file that
declares none gets no file. They import the BSON library from
\"othismo/bson\", or from --library.

`embed` puts a module's header into the module, with the message types in
the .messages file beside the header, replacing any already there.
The header declares one object, named for the module, and exports it:
`export object Echo` for Echo.wasm.

`doc` prints an object's guide and a reference generated from its header, as
Markdown: a module's, from the header inside it; each object in a header
file; one of the runtime's traits, such as Listener; or, for `env`, all of
them.";

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
        Some("test") => test(&args[1..]),
        Some("bindings") => bindings(&args[1..]),
        Some("doc") => match &args[1..] {
            [target] if !target.starts_with('-') => Ok(document(target).map(|md| print!("{md}"))),
            _ => Err(Usage),
        },
        Some("embed") => match &args[1..] {
            [header, module] if !header.starts_with('-') && !module.starts_with('-') => {
                Ok(embed(header.as_ref(), module.as_ref()))
            }
            _ => Err(Usage),
        },
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

fn bindings(args: &[String]) -> Result<Result<()>, Usage> {
    let (mut header, mut out, mut library) = (None, None, "othismo/bson".to_string());
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-o" => out = Some(PathBuf::from(args.next().ok_or(Usage)?)),
            "--library" => library = args.next().ok_or(Usage)?.clone(),
            _ if arg.starts_with('-') || header.is_some() => return Err(Usage),
            _ => header = Some(PathBuf::from(arg)),
        }
    }
    let source = header.ok_or(Usage)?;
    let out = out.unwrap_or_else(|| PathBuf::from(format!("{}.ts", source.display())));
    Ok((|| {
        if source.extension().is_none_or(|e| e != Bson::EXTENSION) {
            anyhow::bail!("{} isn't a schema file; bindings for message types come from a `.{}` file", source.display(), Bson::EXTENSION);
        }
        let schemas = iface::Loader::default().schemas(&source)?;
        // Nothing to bind, and nothing left over from when there was.
        if schemas.declared.is_empty() {
            if out.exists() {
                std::fs::remove_file(&out).with_context(|| format!("removing {}", out.display()))?;
            }
            return Ok(());
        }
        let name = source.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        std::fs::write(&out, Bson::bindings(&schemas, &name, &library)).with_context(|| format!("writing {}", out.display()))
    })())
}

/// Puts `header` into `module`, and the message types in the `.messages`
/// file beside the header, if there is one.
fn embed(header: &Path, module: &Path) -> Result<()> {
    let name = module.file_stem().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let mut loader = iface::Loader::default();
    let resolved = loader.header(header)?;
    iface::for_module(&resolved, &name).with_context(|| format!("{} can't be {name}'s header", header.display()))?;
    let schemas = header.with_extension(Bson::EXTENSION);
    let schemas = if schemas.exists() { loader.schemas(&schemas)? } else { Messages::default() };
    let wasm = std::fs::read(module).with_context(|| format!("reading {}", module.display()))?;
    let wasm = iface::embed(&wasm, &resolved).with_context(|| module.display().to_string())?;
    let wasm = iface::embed_schemas::<Bson>(&wasm, &schemas).with_context(|| module.display().to_string())?;
    std::fs::write(module, wasm).with_context(|| format!("writing {}", module.display()))
}

fn document(target: &str) -> Result<String> {
    let path = Path::new(target);
    let ext = path.extension().and_then(|e| e.to_str());
    if target == "env" {
        return Ok(doc::env());
    }
    let env = iface::env();
    if let Some(o) = env.object(target).cloned().or_else(|| env.traits.get(target).map(|t| t.as_object())) {
        return Ok(doc::object(&o, &Messages::default()));
    }
    let mut loader = iface::Loader::default();
    let (header, schemas) = match ext {
        Some("wasm") => (loader.header(path)?, loader.schemas(path)?),
        Some("oth") => {
            let schemas = path.with_extension(Bson::EXTENSION);
            let schemas = if schemas.exists() { loader.schemas(&schemas)? } else { Messages::default() };
            (loader.header(path)?, schemas)
        }
        _ => anyhow::bail!("{target} is not a module, a header or a built-in type"),
    };
    let pages: Vec<String> = header.objects.iter().map(|o| doc::object(o, &schemas)).collect();
    Ok(pages.join("\n"))
}

fn test(args: &[String]) -> Result<Result<()>, Usage> {
    let mut options = runtime::Options::default();
    let image = match args {
        [flag, image] if flag == "--no-checks" && !image.starts_with('-') => {
            options.no_checks = true;
            image
        }
        [image] if !image.starts_with('-') => image,
        _ => return Err(Usage),
    };
    Ok(pack::load(image.as_ref()).and_then(|mut pack| {
        slots::bind(&mut pack.program, &[])?;
        let reports = suite::run(&pack, options)?;
        for r in &reports {
            println!("{} {} ... {}", r.suite, r.test, if r.passed { "ok" } else { "FAILED" });
            if !r.passed {
                println!("    {}", r.detail);
                for line in r.output.lines() {
                    println!("    | {line}");
                }
            }
        }
        let failed = reports.iter().filter(|r| !r.passed).count();
        println!("\n{} passed, {failed} failed", reports.len() - failed);
        if failed > 0 {
            anyhow::bail!("{failed} test{} failed", if failed == 1 { "" } else { "s" });
        }
        Ok(())
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
            "--no-checks" => options.no_checks = true,
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
