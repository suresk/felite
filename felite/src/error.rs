//! One error type for the whole crate.

use std::fmt;
use std::io;

// @listing error
#[derive(Debug)]
pub enum Error {
    /// The operating system refused a read or a write.
    Io(io::Error),
    /// The file's contents are not something felite wrote.
    Corrupt(String),
}

pub type Result<T> = std::result::Result<T, Error>;
// @end

// @listing error-impls
impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Error::Io(e) => write!(f, "I/O error: {e}"),
            Error::Corrupt(what) => write!(f, "corrupt database: {what}"),
        }
    }
}

impl std::error::Error for Error {}

impl From<io::Error> for Error {
    fn from(e: io::Error) -> Error {
        Error::Io(e)
    }
}
// @end
