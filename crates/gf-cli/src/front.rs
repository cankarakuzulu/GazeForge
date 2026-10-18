use std::io::Write;
use std::path::Path;
use std::process::ExitCode;

use gf_api::{ApiError, Ctx};
use serde_json::{Value, json};

fn print_envelope(envelope: &Value) -> ExitCode {
    let mut out = std::io::stdout().lock();
    let _ = writeln!(out, "{}", gf_api::text(envelope));
    let _ = out.flush();
    ExitCode::from(gf_api::exit_code(envelope))
}

fn call_args(args: &[String]) -> Result<(String, Value), (String, ApiError)> {
    let name = args.first().cloned().ok_or_else(|| ("call".to_string(), ApiError::invalid("usage: gazeforge call <command> [--input JSON|@FILE]")))?;
    let input = match &args[1..] {
        [] => json!({}),
        [flag, value] if flag == "--input" => gf_api::parse_input(value).map_err(|e| (name.clone(), e))?,
        _ => return Err((name, ApiError::invalid("usage: gazeforge call <command> [--input JSON|@FILE]"))),
    };
    Ok((name, input))
}

fn hidden(result: Result<(), String>) -> ExitCode {
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("{e}");
            ExitCode::FAILURE
        }
    }
}

pub fn dispatch(args: &[String]) -> Option<ExitCode> {
    let first = args.first()?.as_str();
    Some(match first {
        "commands" if args.len() == 1 => print_envelope(&gf_api::call(&Ctx::from_env(), "commands.list", &json!({}))),
        "call" => match call_args(&args[1..]) {
            Ok((name, input)) => print_envelope(&gf_api::call(&Ctx::from_env(), &name, &input)),
            Err((name, error)) => print_envelope(&gf_api::envelope(&name, Err(error))),
        },
        "serve" => crate::http::serve(&args[1..]),
        "mcp" if args.len() == 1 => crate::mcp::serve(),
        "__job-supervise" if args.len() == 3 => hidden(gf_api::jobs::supervise(Path::new(&args[1]), &args[2])),
        "__job-work" if args.len() == 3 => hidden(gf_api::jobs::work(Path::new(&args[1]), &args[2])),
        "__sleep" if args.len() >= 2 => {
            gf_api::jobs::sleep_child(args[1].parse().unwrap_or(0.0));
            ExitCode::SUCCESS
        }
        _ => return None,
    })
}
