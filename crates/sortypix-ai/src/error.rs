use std::fmt;
use std::path::PathBuf;

/// Everything that can go wrong in the AI edition.
///
/// Non-exhaustive: later plans may add cases without breaking callers.
#[derive(Debug)]
#[non_exhaustive]
pub enum AiError {
    /// Model or runtime files that are not on disk.
    MissingFiles(Vec<PathBuf>),
    /// The inference runtime could not be loaded.
    RuntimeLoad(String),
    /// The model failed to load or run.
    Model(String),
    /// The input image is unusable.
    Image(String),
}

impl fmt::Display for AiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AiError::MissingFiles(paths) => {
                write!(f, "missing files:")?;
                for p in paths {
                    write!(f, " {}", p.display())?;
                }
                Ok(())
            }
            AiError::RuntimeLoad(m) => write!(f, "runtime load failed: {m}"),
            AiError::Model(m) => write!(f, "model error: {m}"),
            AiError::Image(m) => write!(f, "image error: {m}"),
        }
    }
}

impl std::error::Error for AiError {}
