use serde_json::{Value, json};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Code {
    Internal,
    InvalidInput,
    NotFound,
    GateFailed,
    WorkerFailed,
    Cancelled,
    Conflict,
}

pub const CODES: [Code; 7] =
    [Code::Internal, Code::InvalidInput, Code::NotFound, Code::GateFailed, Code::WorkerFailed, Code::Cancelled, Code::Conflict];

impl Code {
    pub fn as_str(self) -> &'static str {
        match self {
            Code::Internal => "internal",
            Code::InvalidInput => "invalid_input",
            Code::NotFound => "not_found",
            Code::GateFailed => "gate_failed",
            Code::WorkerFailed => "worker_failed",
            Code::Cancelled => "cancelled",
            Code::Conflict => "conflict",
        }
    }

    pub fn parse(text: &str) -> Option<Code> {
        CODES.iter().copied().find(|c| c.as_str() == text)
    }

    pub fn exit(self) -> u8 {
        match self {
            Code::Internal => 1,
            Code::InvalidInput => 2,
            Code::NotFound => 3,
            Code::GateFailed => 4,
            Code::WorkerFailed => 5,
            Code::Cancelled => 6,
            Code::Conflict => 7,
        }
    }

    pub fn http(self) -> u16 {
        match self {
            Code::Internal => 500,
            Code::InvalidInput => 400,
            Code::NotFound => 404,
            Code::GateFailed => 422,
            Code::WorkerFailed => 502,
            Code::Cancelled => 499,
            Code::Conflict => 409,
        }
    }

    pub fn retryable(self) -> bool {
        matches!(self, Code::WorkerFailed | Code::Conflict)
    }
}

#[derive(Clone, Debug)]
pub struct ApiError {
    pub code: Code,
    pub message: String,
    pub details: Value,
    pub hint: Option<String>,
}

impl ApiError {
    pub fn new(code: Code, message: impl Into<String>) -> ApiError {
        ApiError { code, message: message.into(), details: json!({}), hint: None }
    }

    pub fn invalid(message: impl Into<String>) -> ApiError {
        ApiError::new(Code::InvalidInput, message)
    }

    pub fn not_found(message: impl Into<String>) -> ApiError {
        ApiError::new(Code::NotFound, message)
    }

    pub fn internal(message: impl Into<String>) -> ApiError {
        ApiError::new(Code::Internal, message)
    }

    pub fn details(mut self, details: Value) -> ApiError {
        self.details = details;
        self
    }

    pub fn hint(mut self, hint: impl Into<String>) -> ApiError {
        self.hint = Some(hint.into());
        self
    }

    pub fn to_json(&self) -> Value {
        json!({"code": self.code.as_str(), "message": self.message, "retryable": self.code.retryable(),
               "details": self.details, "hint": self.hint})
    }
}

pub fn code_table() -> Value {
    Value::Array(CODES.iter().map(|c| json!({"code": c.as_str(), "exit": c.exit(), "http": c.http(), "retryable": c.retryable()})).collect())
}
