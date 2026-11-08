//! Error type shared by every Daggerfall data file reader.

use std::fmt;
use std::io;
use std::path::{Path, PathBuf};

/// Errors that can occur while reading files from a Daggerfall `ARENA2` folder.
#[derive(Debug)]
pub enum Arena2Error {
    /// A file could not be read from disk.
    Io {
        /// The file that failed to load.
        path: PathBuf,
        /// The underlying operating system error.
        source: io::Error,
    },
    /// A file was read but its contents did not match the expected layout.
    Format(String),
    /// A requested record or resource does not exist.
    NotFound(String),
}

impl Arena2Error {
    /// Creates a [`Arena2Error::Format`] error from any message.
    pub fn format(message: impl Into<String>) -> Self {
        Self::Format(message.into())
    }

    /// Creates a [`Arena2Error::NotFound`] error from any message.
    pub fn not_found(message: impl Into<String>) -> Self {
        Self::NotFound(message.into())
    }
}

impl fmt::Display for Arena2Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io { path, source } => {
                write!(formatter, "could not read '{}': {source}", path.display())
            }
            Self::Format(message) => write!(formatter, "invalid data: {message}"),
            Self::NotFound(message) => write!(formatter, "not found: {message}"),
        }
    }
}

impl std::error::Error for Arena2Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            Self::Format(_) | Self::NotFound(_) => None,
        }
    }
}

/// Convenience alias for results produced by the `arena2` module.
pub type Arena2Result<T> = Result<T, Arena2Error>;

/// Reads an entire file into memory, attaching the path to any error.
pub fn read_file(path: &Path) -> Arena2Result<Vec<u8>> {
    std::fs::read(path).map_err(|source| Arena2Error::Io { path: path.to_path_buf(), source })
}
