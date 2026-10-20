use std::path::PathBuf;

use serde_json::{Value, json};

pub mod anim;
mod anim_glb;
mod anim_guard;
pub mod body;
mod commands;
pub mod designer;
pub mod exactjson;
pub mod eyes;
pub mod error;
pub mod grid;
pub mod jobs;
pub mod photo;
pub mod photo_fit;
pub mod photo_gate;
mod process;
pub mod tasks;
pub mod project;
pub mod resources;
pub mod schema;
pub mod zipsafe;

pub use error::{ApiError, Code};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Query,
    Mutation,
    Job,
}

impl Kind {
    pub fn as_str(self) -> &'static str {
        match self {
            Kind::Query => "query",
            Kind::Mutation => "mutation",
            Kind::Job => "job",
        }
    }
}

pub type Handler = fn(&Ctx, &Value) -> Result<Value, ApiError>;

pub struct CommandDef {
    pub name: &'static str,
    pub kind: Kind,
    pub summary: &'static str,
    pub limits: &'static str,
    pub input: fn() -> Value,
    pub output: fn() -> Value,
    pub run: Handler,
}

impl CommandDef {
    pub fn describe(&self) -> Value {
        json!({"name": self.name, "kind": self.kind.as_str(), "summary": self.summary, "limits": self.limits,
               "input_schema": (self.input)(), "output_schema": (self.output)()})
    }

    pub fn tool_name(&self) -> String {
        self.name.replace('.', "_")
    }
}

#[derive(Clone, Debug)]
pub struct Ctx {
    pub jobs_dir: PathBuf,
    pub exe: PathBuf,
    pub job_id: Option<String>,
    pub projects: PathBuf,
}

impl Ctx {
    pub fn from_env() -> Ctx {
        let jobs_dir = std::env::var_os("GAZEFORGE_JOBS").map(PathBuf::from).unwrap_or_else(|| std::env::temp_dir().join("gazeforge-jobs"));
        let exe = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("gazeforge"));
        Ctx { jobs_dir, exe, job_id: None, projects: project::default_root() }
    }
}

pub fn registry() -> &'static [CommandDef] {
    commands::REGISTRY
}

pub fn find(name: &str) -> Option<&'static CommandDef> {
    registry().iter().find(|c| c.name == name)
}

pub fn find_tool(tool: &str) -> Option<&'static CommandDef> {
    registry().iter().find(|c| c.tool_name() == tool)
}

pub fn check_input(def: &CommandDef, input: &Value) -> Result<(), ApiError> {
    let problems = schema::validate(&(def.input)(), input);
    if problems.is_empty() {
        return Ok(());
    }
    Err(ApiError::invalid(format!("{} input does not match its schema: {}", def.name, problems.join("; ")))
        .details(json!({"problems": problems}))
        .hint(format!("commands.list shows the input schema of {}", def.name)))
}

fn run(ctx: &Ctx, name: &str, input: &Value) -> Result<Value, ApiError> {
    let def = find(name).ok_or_else(|| {
        ApiError::not_found(format!("no command named {name}")).hint("commands.list names every command")
    })?;
    check_input(def, input)?;
    if def.kind == Kind::Job && ctx.job_id.is_none() {
        return Err(ApiError::invalid(format!("{name} is a job; it runs only in a worker"))
            .hint(format!("submit it with jobs.submit {{\"command\": \"{name}\", \"input\": {{...}}}}")));
    }
    (def.run)(ctx, input)
}

pub fn call(ctx: &Ctx, name: &str, input: &Value) -> Value {
    envelope(name, run(ctx, name, input))
}

pub fn envelope(name: &str, outcome: Result<Value, ApiError>) -> Value {
    match outcome {
        Ok(result) => json!({"ok": true, "command": name, "result": result}),
        Err(error) => json!({"ok": false, "command": name, "error": error.to_json()}),
    }
}

pub fn text(envelope: &Value) -> String {
    serde_json::to_string(envelope).expect("JSON values serialise")
}

pub fn error_code(envelope: &Value) -> Option<Code> {
    if envelope.get("ok") == Some(&Value::Bool(true)) {
        return None;
    }
    Some(envelope.pointer("/error/code").and_then(Value::as_str).and_then(Code::parse).unwrap_or(Code::Internal))
}

pub fn exit_code(envelope: &Value) -> u8 {
    error_code(envelope).map_or(0, Code::exit)
}

pub fn http_status(envelope: &Value) -> u16 {
    error_code(envelope).map_or(200, Code::http)
}

pub fn parse_input(text: &str) -> Result<Value, ApiError> {
    let (source, body) = match text.strip_prefix('@') {
        Some(path) => (path.to_string(), std::fs::read_to_string(path).map_err(|e| ApiError::not_found(format!("--input {path}: {e}")))?),
        None => ("inline".to_string(), text.to_string()),
    };
    exactjson::text(&body).map_err(|e| ApiError::invalid(format!("--input ({source}) is not JSON: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_unique_and_tool_names_have_no_dots() {
        let mut names: Vec<&str> = registry().iter().map(|c| c.name).collect();
        names.sort_unstable();
        let before = names.len();
        names.dedup();
        assert_eq!(before, names.len());
        assert!(registry().iter().all(|c| !c.tool_name().contains('.') && find_tool(&c.tool_name()).is_some()));
    }

    #[test]
    fn every_schema_is_an_object_schema_that_accepts_its_own_example_shape() {
        for c in registry() {
            let s = (c.input)();
            assert_eq!(s["type"], "object", "{}", c.name);
            assert_eq!(s["additionalProperties"], Value::Bool(false), "{}", c.name);
            assert!((c.output)().is_object(), "{}", c.name);
        }
    }

    #[test]
    fn unknown_commands_bad_input_and_direct_jobs_are_refused_with_codes() {
        let ctx = Ctx { jobs_dir: std::env::temp_dir(), exe: PathBuf::from("x"), job_id: None, projects: std::env::temp_dir() };
        assert_eq!(error_code(&call(&ctx, "no.such", &json!({}))), Some(Code::NotFound));
        assert_eq!(error_code(&call(&ctx, "commands.list", &json!({"x": 1}))), Some(Code::InvalidInput));
        assert_eq!(error_code(&call(&ctx, "debug.slow", &json!({"seconds": 1}))), Some(Code::InvalidInput));
        let exact = parse_input("{\"w\": 0.10937899999999999}").unwrap();
        assert_eq!(exact["w"].as_f64().unwrap().to_bits(), "0.10937899999999999".parse::<f64>().unwrap().to_bits());
        let ok = call(&ctx, "commands.list", &json!({}));
        assert_eq!(exit_code(&ok), 0);
        assert_eq!(http_status(&ok), 200);
    }
}
