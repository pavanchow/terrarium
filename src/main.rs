//! The `terrarium` command line: run an untrusted script file under limits.
//!
//! Usage:
//!   terrarium run <file> [--fuel N] [--mem BYTES] [--depth D]
//!                        [--grant cap,cap,...] [-- args...]

use std::process::ExitCode;

use terrarium::{DefaultHost, Limits, Sandbox};

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(code) => code,
        Err(msg) => {
            eprintln!("terrarium: {msg}");
            eprintln!();
            eprintln!("{USAGE}");
            ExitCode::from(2)
        }
    }
}

const USAGE: &str = "usage: terrarium run <file> [--fuel N] [--mem BYTES] [--depth D] [--grant cap,cap,...] [-- args...]";

fn run(args: &[String]) -> Result<ExitCode, String> {
    let mut it = args.iter();
    match it.next().map(String::as_str) {
        Some("run") => {}
        Some("--help") | Some("-h") | None => {
            println!("{USAGE}");
            return Ok(ExitCode::SUCCESS);
        }
        Some(other) => return Err(format!("unknown command '{other}'")),
    }

    let mut file: Option<String> = None;
    let mut limits = Limits::default();
    let mut grants: Vec<String> = Vec::new();
    let mut script_args: Vec<String> = Vec::new();

    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--fuel" => {
                let v = it.next().ok_or("--fuel needs a value")?;
                limits.fuel = v.parse().map_err(|_| "invalid --fuel value")?;
            }
            "--mem" => {
                let v = it.next().ok_or("--mem needs a value")?;
                limits.max_memory = v.parse().map_err(|_| "invalid --mem value")?;
            }
            "--depth" => {
                let v = it.next().ok_or("--depth needs a value")?;
                limits.max_depth = v.parse().map_err(|_| "invalid --depth value")?;
            }
            "--grant" => {
                let v = it.next().ok_or("--grant needs a value")?;
                for name in v.split(',').filter(|s| !s.is_empty()) {
                    grants.push(name.to_string());
                }
            }
            "--" => {
                for rest in it.by_ref() {
                    script_args.push(rest.clone());
                }
            }
            other if file.is_none() && !other.starts_with("--") => {
                file = Some(other.to_string());
            }
            other => {
                // Anything after the file that is not a flag is a script arg.
                if file.is_some() {
                    script_args.push(other.to_string());
                } else {
                    return Err(format!("unexpected argument '{other}'"));
                }
            }
        }
    }

    let file = file.ok_or("missing script file")?;
    let source = std::fs::read_to_string(&file).map_err(|e| format!("cannot read {file}: {e}"))?;

    let mut sb = Sandbox::with_host(limits, DefaultHost::new()).grant_all(grants);
    let outcome = sb.run(&source, &script_args);

    // Flush anything the script printed through the granted `print` capability.
    for line in &sb.host().output {
        println!("{line}");
    }

    match &outcome.result {
        Ok(value) => {
            eprintln!(
                "=> {value}  [fuel {} / mem {} bytes]",
                outcome.fuel_used, outcome.mem_used
            );
            Ok(ExitCode::SUCCESS)
        }
        Err(trap) => {
            eprintln!(
                "trap: {trap}  [fuel {} / mem {} bytes]",
                outcome.fuel_used, outcome.mem_used
            );
            Ok(ExitCode::FAILURE)
        }
    }
}
