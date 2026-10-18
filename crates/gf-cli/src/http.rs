use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::process::ExitCode;

use gf_api::{ApiError, Ctx};
use serde_json::{Value, json};

const MAX_HEAD: usize = 64 * 1024;
const MAX_BODY: usize = 16 * 1024 * 1024;

struct Request {
    method: String,
    path: String,
    host: String,
    origin: Option<String>,
    body: Vec<u8>,
}

fn read_request(stream: &mut TcpStream) -> Result<Request, String> {
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    reader.read_line(&mut line).map_err(|e| e.to_string())?;
    let mut parts = line.split_whitespace();
    let (method, path) = match (parts.next(), parts.next()) {
        (Some(m), Some(p)) => (m.to_string(), p.to_string()),
        _ => return Err("malformed request line".into()),
    };
    let (mut length, mut host, mut origin, mut seen) = (0usize, String::new(), None, line.len());
    loop {
        let mut header = String::new();
        let n = reader.read_line(&mut header).map_err(|e| e.to_string())?;
        seen += n;
        if n == 0 || seen > MAX_HEAD {
            return Err("request head too large or cut short".into());
        }
        let header = header.trim_end();
        if header.is_empty() {
            break;
        }
        if let Some((name, value)) = header.split_once(':') {
            let value = value.trim();
            match name.trim().to_ascii_lowercase().as_str() {
                "content-length" => length = value.parse().map_err(|_| "bad Content-Length")?,
                "host" => host = value.to_string(),
                "origin" => origin = Some(value.to_string()),
                "transfer-encoding" => return Err("chunked bodies are not accepted; send Content-Length".into()),
                _ => {}
            }
        }
    }
    if length > MAX_BODY {
        return Err(format!("body of {length} bytes is over the {MAX_BODY}-byte limit"));
    }
    let mut body = vec![0u8; length];
    reader.read_exact(&mut body).map_err(|e| e.to_string())?;
    Ok(Request { method, path, host, origin, body })
}

fn loopback(authority: &str) -> bool {
    let host = authority.trim_start_matches("http://").split('/').next().unwrap_or("");
    let name = host.rsplit_once(':').map_or(host, |(h, _)| h);
    matches!(name, "127.0.0.1" | "localhost" | "[::1]")
}

fn route(ctx: &Ctx, request: &Request) -> Value {
    if !loopback(&request.host) || request.origin.as_deref().is_some_and(|o| !loopback(o)) {
        return gf_api::envelope("http", Err(ApiError::invalid("only loopback hosts and origins are served")));
    }
    let path = request.path.split('?').next().unwrap_or("");
    match (request.method.as_str(), path.strip_prefix("/v1/")) {
        ("GET", Some("commands")) => gf_api::call(ctx, "commands.list", &json!({})),
        ("GET", Some(rest)) if rest.starts_with("jobs/") => gf_api::call(ctx, "jobs.status", &json!({"id": &rest[5..]})),
        ("POST", Some(name)) if !name.is_empty() && !name.contains('/') => {
            let input = if request.body.iter().all(u8::is_ascii_whitespace) {
                Ok(json!({}))
            } else {
                serde_json::from_slice::<Value>(&request.body).map_err(|e| ApiError::invalid(format!("the body is not JSON: {e}")))
            };
            match input {
                Ok(input) => gf_api::call(ctx, name, &input),
                Err(e) => gf_api::envelope(name, Err(e)),
            }
        }
        _ => gf_api::envelope(
            "http",
            Err(ApiError::not_found(format!("no route {} {}", request.method, request.path))
                .hint("GET /v1/commands, POST /v1/<command> with a JSON body, GET /v1/jobs/<id>")),
        ),
    }
}

fn reason(status: u16) -> &'static str {
    match status {
        200 => "OK",
        400 => "Bad Request",
        404 => "Not Found",
        409 => "Conflict",
        422 => "Unprocessable Content",
        499 => "Client Closed Request",
        502 => "Bad Gateway",
        _ => "Internal Server Error",
    }
}

fn handle(ctx: &Ctx, mut stream: TcpStream) {
    let envelope = match read_request(&mut stream) {
        Ok(request) => route(ctx, &request),
        Err(e) => gf_api::envelope("http", Err(ApiError::invalid(e))),
    };
    let body = gf_api::text(&envelope);
    let status = gf_api::http_status(&envelope);
    let head = format!(
        "HTTP/1.1 {status} {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        reason(status),
        body.len()
    );
    let _ = stream.write_all(head.as_bytes()).and_then(|_| stream.write_all(body.as_bytes())).and_then(|_| stream.flush());
}

pub fn serve(args: &[String]) -> ExitCode {
    let port = match args {
        [] => Ok(7878u16),
        [flag, value] if flag == "--port" => value.parse::<u16>().map_err(|_| format!("--port must be 0 to 65535, got {value}")),
        _ => Err("usage: gazeforge serve [--port 7878]".to_string()),
    };
    let fail = |message: String| {
        println!("{}", json!({"ok": false, "command": "serve", "error": {"code": "invalid_input", "message": message}}));
        ExitCode::from(2)
    };
    let port = match port {
        Ok(p) => p,
        Err(e) => return fail(e),
    };
    let listener = match TcpListener::bind(("127.0.0.1", port)) {
        Ok(l) => l,
        Err(e) => return fail(format!("cannot listen on 127.0.0.1:{port}: {e}")),
    };
    let address = listener.local_addr().map(|a| a.to_string()).unwrap_or_default();
    let ctx = Ctx::from_env();
    println!("{}", json!({"ok": true, "command": "serve", "listening": address, "jobs_dir": ctx.jobs_dir.display().to_string()}));
    let _ = std::io::stdout().flush();
    for stream in listener.incoming().flatten() {
        let ctx = ctx.clone();
        std::thread::spawn(move || handle(&ctx, stream));
    }
    ExitCode::SUCCESS
}
