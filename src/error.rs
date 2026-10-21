use std::path::PathBuf;

/// An error tidy can show to a person. The CLI prints it as `tidy: {error}`.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{0}")]
    Message(String),

    #[error("invalid rules file {path}:\n{message}")]
    Rules { path: PathBuf, message: String },

    #[error("{path} is not a directory")]
    NotADirectory { path: PathBuf },

    #[error("folder not found: {path}")]
    NotFound { path: PathBuf },

    #[error("{context}: {source}")]
    Io {
        context: String,
        #[source]
        source: std::io::Error,
    },
}

impl Error {
    pub fn msg(message: impl Into<String>) -> Self {
        Error::Message(message.into())
    }

    pub fn io(context: impl Into<String>, source: std::io::Error) -> Self {
        Error::Io {
            context: context.into(),
            source,
        }
    }
}
