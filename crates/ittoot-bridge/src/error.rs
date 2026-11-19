/// Everything the bridge can fail at. Input is hostile: nothing here panics.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{path}: {source}")]
    Io { path: String, source: std::io::Error },
    #[error("{path}: {message}")]
    Parse { path: String, message: String },
    #[error("{0}")]
    Invalid(String),
    #[error("not connected: {0}")]
    NotConnected(String),
    #[error("no such item: {0}")]
    Missing(String),
    /// A device or server answered with an error, or did not answer.
    #[error("{0}")]
    Remote(String),
}

pub type Result<T> = std::result::Result<T, Error>;

impl Error {
    pub fn io(path: &std::path::Path, source: std::io::Error) -> Self {
        Error::Io { path: path.display().to_string(), source }
    }
}
