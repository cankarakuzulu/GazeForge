use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
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
    let limit = if path.starts_with("/v1/upload") { gf_api::wizards::UPLOAD_LIMIT } else { MAX_BODY };
    if length > limit {
        return Err(format!("body of {length} bytes is over the {limit}-byte limit"));
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

struct Served {
    status: u16,
    content_type: &'static str,
    body: Vec<u8>,
    immutable: bool,
}

fn content_type(name: &str) -> Option<&'static str> {
    Some(match name.rsplit('.').next()? {
        "html" => "text/html; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "wasm" => "application/wasm",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "json" => "application/json",
        _ => return None,
    })
}

fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let hex = |b: u8| (b as char).to_digit(16).map(|d| d as u8);
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && let (Some(h), Some(l)) = (hex(bytes[i + 1]), hex(bytes[i + 2]))
        {
            out.push(h * 16 + l);
            i += 3;
            continue;
        }
        out.push(if bytes[i] == b'+' { b' ' } else { bytes[i] });
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn query(path: &str, key: &str) -> Option<String> {
    let (_, q) = path.split_once('?')?;
    q.split('&').find_map(|pair| pair.split_once('=').filter(|(k, _)| *k == key).map(|(_, v)| percent_decode(v)))
}

fn not_found(message: String) -> Served {
    let envelope = gf_api::envelope("http", Err(ApiError::not_found(message)));
    Served { status: 404, content_type: "application/json", body: gf_api::text(&envelope).into_bytes(), immutable: false }
}

fn static_file(web: &Path, rest: &str) -> Served {
    let rest = if rest.is_empty() { "index.html" } else { rest };
    let safe = rest.split('/').all(|s| !s.is_empty() && s != ".." && s.chars().all(|c| c.is_ascii_alphanumeric() || "._-".contains(c)));
    let Some(kind) = content_type(rest).filter(|_| safe) else {
        return not_found(format!("no designer file {rest}"));
    };
    match std::fs::read(web.join(rest)) {
        Ok(body) => Served { status: 200, content_type: kind, body, immutable: false },
        Err(_) => not_found(format!("{} has no {rest}; start the server with --web DIR (the folder holding index.html)", web.display())),
    }
}

fn blob(path: &str, hex: &str) -> Served {
    let Some(store) = query(path, "store") else {
        return not_found("GET /v1/blob/<sha256 hex>?store=<store path>".into());
    };
    if hex.len() != 64 || !hex.chars().all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()) {
        return not_found(format!("{hex} is not a sha256 hex digest"));
    }
    match std::fs::read(Path::new(&store).join("blobs").join(&hex[..2]).join(hex)) {
        Ok(body) if gf_core::hash::sha256_hex(&body) == hex => Served { status: 200, content_type: "application/octet-stream", body, immutable: true },
        Ok(_) => {
            let envelope = gf_api::envelope("http", Err(ApiError::new(gf_api::error::Code::Conflict, format!("blob {hex} in {store} is corrupt: its bytes no longer match their hash"))));
            Served { status: 409, content_type: "application/json", body: gf_api::text(&envelope).into_bytes(), immutable: false }
        }
        Err(_) => not_found(format!("no blob {hex} in {store}")),
    }
}

fn wizard_file(ctx: &Ctx, path: &str) -> Served {
    let (Some(run), Some(file)) = (query(path, "run"), query(path, "path")) else {
        return not_found("GET /v1/wizard-file?run=<run id>&path=<a file of that run>".into());
    };
    match gf_api::wizards::run_file(ctx, &run, &file) {
        Ok(p) => match (content_type(&p.display().to_string().to_ascii_lowercase()), std::fs::read(&p)) {
            (Some(kind), Ok(body)) => Served { status: 200, content_type: kind, body, immutable: false },
            _ => not_found(format!("{file}: not a file type this route serves")),
        },
        Err(e) => not_found(e.message),
    }
}

fn serve_route(ctx: &Ctx, web: &Path, request: &Request) -> Served {
    let path = request.path.split('?').next().unwrap_or("");
    let allowed = loopback(&request.host) && request.origin.as_deref().is_none_or(loopback);
    if allowed && request.method == "GET" {
        if path == "/" || path == "/designer" {
            return Served { status: 200, content_type: "text/html; charset=utf-8", body: b"<meta http-equiv=\"refresh\" content=\"0; url=/designer/\">".to_vec(), immutable: false };
        }
        if let Some(rest) = path.strip_prefix("/designer/") {
            return static_file(web, rest);
        }
        if path == "/v1/wizard-file" {
            return wizard_file(ctx, &request.path);
        }
        if let Some(hex) = path.strip_prefix("/v1/blob/") {
            return blob(&request.path, hex);
        }
    }
    let envelope = route(ctx, request);
    Served { status: gf_api::http_status(&envelope), content_type: "application/json", body: gf_api::text(&envelope).into_bytes(), immutable: false }
}

fn route(ctx: &Ctx, request: &Request) -> Value {
    if !loopback(&request.host) || request.origin.as_deref().is_some_and(|o| !loopback(o)) {
        return gf_api::envelope("http", Err(ApiError::invalid("only loopback hosts and origins are served")));
    }
    let path = request.path.split('?').next().unwrap_or("");
    match (request.method.as_str(), path.strip_prefix("/v1/")) {
        ("GET", Some("commands")) => gf_api::call(ctx, "commands.list", &json!({})),
        ("GET", Some(rest)) if rest.starts_with("jobs/") => gf_api::call(ctx, "jobs.status", &json!({"id": &rest[5..]})),
        ("POST", Some("upload")) => {
            let (Some(run), Some(name)) = (query(&request.path, "run"), query(&request.path, "name")) else {
                return gf_api::envelope("upload", Err(ApiError::invalid("POST /v1/upload?run=<run id>&name=<file name> with the file as the body")));
            };
            gf_api::envelope("upload", gf_api::wizards::upload(ctx, &run, &name, &request.body).map(|p| json!({"path": p.display().to_string(), "bytes": request.body.len()})))
        }
        ("POST", Some(name)) if !name.is_empty() && !name.contains('/') => {
            let input = if request.body.iter().all(u8::is_ascii_whitespace) {
                Ok(json!({}))
            } else {
                std::str::from_utf8(&request.body).map_err(|e| e.to_string()).and_then(gf_api::exactjson::text).map_err(|e| ApiError::invalid(format!("the body is not JSON: {e}")))
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

fn handle(ctx: &Ctx, web: &Path, mut stream: TcpStream) {
    let served = match read_request(&mut stream) {
        Ok(request) => serve_route(ctx, web, &request),
        Err(e) => {
            let envelope = gf_api::envelope("http", Err(ApiError::invalid(e)));
            Served { status: gf_api::http_status(&envelope), content_type: "application/json", body: gf_api::text(&envelope).into_bytes(), immutable: false }
        }
    };
    let status = served.status;
    let cache = if served.immutable { "Cache-Control: public, max-age=31536000, immutable\r\n" } else { "" };
    let head = format!(
        "HTTP/1.1 {status} {}\r\nContent-Type: {}\r\nContent-Length: {}\r\n{cache}Connection: close\r\n\r\n",
        reason(status),
        served.content_type,
        served.body.len()
    );
    let _ = stream.write_all(head.as_bytes()).and_then(|_| stream.write_all(&served.body)).and_then(|_| stream.flush());
}

fn serve_args(args: &[String]) -> Result<(u16, PathBuf, Option<PathBuf>), String> {
    let usage = || "usage: gazeforge serve [--port 7878] [--web DIR] [--projects DIR]".to_string();
    let (mut port, mut web, mut projects) = (7878u16, None, None);
    let mut rest = args;
    while let [flag, value, tail @ ..] = rest {
        match flag.as_str() {
            "--port" => port = value.parse::<u16>().map_err(|_| format!("--port must be 0 to 65535, got {value}"))?,
            "--web" => web = Some(PathBuf::from(value)),
            "--projects" => projects = Some(PathBuf::from(value)),
            _ => return Err(usage()),
        }
        rest = tail;
    }
    if !rest.is_empty() {
        return Err(usage());
    }
    let web = web.or_else(|| std::env::var_os("GAZEFORGE_WEB").map(PathBuf::from)).unwrap_or_else(|| PathBuf::from("web"));
    Ok((port, web, projects))
}

pub fn serve(args: &[String]) -> ExitCode {
    let fail = |message: String| {
        println!("{}", json!({"ok": false, "command": "serve", "error": {"code": "invalid_input", "message": message}}));
        ExitCode::from(2)
    };
    let (port, web, projects) = match serve_args(args) {
        Ok(p) => p,
        Err(e) => return fail(e),
    };
    let listener = match TcpListener::bind(("127.0.0.1", port)) {
        Ok(l) => l,
        Err(e) => return fail(format!("cannot listen on 127.0.0.1:{port}: {e}")),
    };
    let address = listener.local_addr().map(|a| a.to_string()).unwrap_or_default();
    let mut ctx = Ctx::from_env();
    if let Some(p) = projects {
        ctx.projects = p;
    }
    println!("{}", json!({"ok": true, "command": "serve", "listening": address, "jobs_dir": ctx.jobs_dir.display().to_string()}));
    let _ = std::io::stdout().flush();
    for stream in listener.incoming().flatten() {
        let (ctx, web) = (ctx.clone(), web.clone());
        std::thread::spawn(move || handle(&ctx, &web, stream));
    }
    ExitCode::SUCCESS
}
