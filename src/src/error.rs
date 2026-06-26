use std::path::PathBuf;

use thiserror::Error;

#[derive(Error, Debug)]
pub enum AppError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Git error: {0}")]
    Git(#[from] git2::Error),
    #[error("Parse error in {path:?}: {source}")]
    Parse {
        path: PathBuf,
        #[source]
        source: rustpython_parser::ParseError,
    },
    #[error("Not a git repository: {0}")]
    NotAGit(PathBuf),
    #[error("No Python source found in {0}")]
    NoPythonSource(PathBuf),
    #[error("Graph error: {0}")]
    Graph(String),
    #[error("Context error: {0}")]
    Context(String),
    #[error("Validation error: {0}")]
    Validation(String),
}

pub type Result<T> = std::result::Result<T, AppError>;

impl From<walkdir::Error> for AppError {
    fn from(value: walkdir::Error) -> Self {
        if let Some(err) = value.io_error() {
            Self::Io(std::io::Error::new(err.kind(), value.to_string()))
        } else {
            Self::Context(value.to_string())
        }
    }
}

impl From<serde_json::Error> for AppError {
    fn from(value: serde_json::Error) -> Self {
        Self::Validation(value.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn graph_error_contains_label() {
        assert!(
            AppError::Graph("bad edge".into())
                .to_string()
                .contains("Graph")
        );
    }

    #[test]
    fn not_git_includes_path() {
        let err = AppError::NotAGit(PathBuf::from("/tmp/repo"));
        assert!(err.to_string().contains("/tmp/repo"));
    }
}
