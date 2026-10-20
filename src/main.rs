use real_talk::{check, runtime};
use std::path::PathBuf;
use std::process::ExitCode;

const USAGE: &str = "usage: real_talk [--check] [--check-shapes] <image.img>

  --check         validate the image, print each instance's view and what
                  it can reach at most, and exit without running it
  --check-shapes  verify every message's fields against its declaration";

fn main() -> ExitCode {
    let mut image = None;
    let mut options = runtime::Options::default();
    let mut check_only = false;
    for arg in std::env::args().skip(1) {
        match arg.as_str() {
            "--check" => check_only = true,
            "--check-shapes" => options.check_shapes = true,
            "-h" | "--help" => {
                println!("{USAGE}");
                return ExitCode::SUCCESS;
            }
            _ if arg.starts_with('-') || image.is_some() => {
                eprintln!("{USAGE}");
                return ExitCode::from(2);
            }
            _ => image = Some(PathBuf::from(arg)),
        }
    }
    let Some(image) = image else {
        eprintln!("{USAGE}");
        return ExitCode::from(2);
    };

    let result = check::load(&image).and_then(|program| {
        if check_only {
            print!("{}", check::describe(&program));
            Ok(())
        } else {
            runtime::run(&program, runtime::Io::std(), options)
        }
    });
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e:#}");
            ExitCode::FAILURE
        }
    }
}
