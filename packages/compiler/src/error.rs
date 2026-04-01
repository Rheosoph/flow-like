use std::fmt;

#[derive(Debug)]
pub enum CompilerError {
    Jwt(String),
    Download(String),
    Upload(String),
    Compilation(String),
    Callback(String),
    Config(String),
    Timeout,
    InvalidJob(String),
}

impl fmt::Display for CompilerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CompilerError::Jwt(msg) => write!(f, "JWT error: {msg}"),
            CompilerError::Download(msg) => write!(f, "Download error: {msg}"),
            CompilerError::Upload(msg) => write!(f, "Upload error: {msg}"),
            CompilerError::Compilation(msg) => write!(f, "Compilation error: {msg}"),
            CompilerError::Callback(msg) => write!(f, "Callback error: {msg}"),
            CompilerError::Config(msg) => write!(f, "Config error: {msg}"),
            CompilerError::Timeout => write!(f, "Compilation timeout"),
            CompilerError::InvalidJob(msg) => write!(f, "Invalid job: {msg}"),
        }
    }
}

impl std::error::Error for CompilerError {}

impl From<jsonwebtoken::errors::Error> for CompilerError {
    fn from(e: jsonwebtoken::errors::Error) -> Self {
        CompilerError::Jwt(e.to_string())
    }
}
