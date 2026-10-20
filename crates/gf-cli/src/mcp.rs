use std::io::{BufRead, Write};
use std::process::ExitCode;

use gf_api::Ctx;
use serde_json::{Value, json};

const PROTOCOL: &str = "2025-06-18";

fn tools() -> Value {
    let list: Vec<Value> = gf_api::registry()
        .iter()
        .map(|c| {
            json!({"name": c.tool_name(), "title": c.name, "description": format!("{} Limits: {}", c.summary, c.limits),
                   "inputSchema": (c.input)()})
        })
        .collect();
    json!({"tools": list})
}

fn tool_call(ctx: &Ctx, params: &Value) -> Value {
    let tool = params.get("name").and_then(Value::as_str).unwrap_or("");
    let arguments = params.get("arguments").cloned().unwrap_or_else(|| json!({}));
    let name = gf_api::find_tool(tool).map_or_else(|| tool.to_string(), |c| c.name.to_string());
    let envelope = gf_api::call(ctx, &name, &arguments);
    json!({"content": [{"type": "text", "text": gf_api::text(&envelope)}], "isError": envelope.get("ok") != Some(&Value::Bool(true))})
}

fn answer(ctx: &Ctx, message: &Value) -> Option<Value> {
    let id = message.get("id")?.clone();
    let method = message.get("method").and_then(Value::as_str).unwrap_or("");
    let params = message.get("params").cloned().unwrap_or_else(|| json!({}));
    let result = match method {
        "initialize" => Ok(json!({
            "protocolVersion": params.get("protocolVersion").and_then(Value::as_str).unwrap_or(PROTOCOL),
            "capabilities": {"tools": {"listChanged": false}},
            "serverInfo": {"name": "gazeforge", "version": env!("CARGO_PKG_VERSION")},
            "instructions": "Every tool returns one JSON envelope as text: {ok, command, result} or {ok: false, command, error: {code, message, retryable, details, hint}}. commands_list describes them all."
        })),
        "ping" => Ok(json!({})),
        "tools/list" => Ok(tools()),
        "tools/call" => Ok(tool_call(ctx, &params)),
        other => Err(json!({"code": -32601, "message": format!("method not found: {other}")})),
    };
    Some(match result {
        Ok(r) => json!({"jsonrpc": "2.0", "id": id, "result": r}),
        Err(e) => json!({"jsonrpc": "2.0", "id": id, "error": e}),
    })
}

pub fn serve() -> ExitCode {
    let ctx = Ctx::from_env();
    let stdin = std::io::stdin();
    let mut out = std::io::stdout();
    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        if line.trim().is_empty() {
            continue;
        }
        let reply = match gf_api::exactjson::envelope(&line) {
            Ok(message) => answer(&ctx, &message),
            Err(e) => Some(json!({"jsonrpc": "2.0", "id": null, "error": {"code": -32700, "message": format!("parse error: {e}")}})),
        };
        if let Some(reply) = reply
            && writeln!(out, "{reply}").and_then(|_| out.flush()).is_err()
        {
            break;
        }
    }
    ExitCode::SUCCESS
}
