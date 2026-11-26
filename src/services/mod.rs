//! Business logic. The web layer and the command line interface use these functions.

pub mod accounts;
pub mod budget;
pub mod categories;
pub mod categorise;
pub mod defaults;
pub mod importer;
pub mod reports;
pub mod transfers;
pub mod users;

use std::fmt;

/// An error of a service function.
#[derive(Debug)]
pub enum Error {
    /// The operation is not permitted or the input is not valid. The message is safe to
    /// show to the user.
    Service(String),
    /// The object does not exist or the user does not own it.
    NotFound(&'static str),
    /// A database error.
    Db(rusqlite::Error),
}

impl Error {
    pub fn service(message: impl Into<String>) -> Error {
        Error::Service(message.into())
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Service(message) => f.write_str(message),
            Error::NotFound(what) => write!(f, "The {what} does not exist."),
            Error::Db(error) => write!(f, "Database error: {error}"),
        }
    }
}

impl std::error::Error for Error {}

impl From<rusqlite::Error> for Error {
    fn from(error: rusqlite::Error) -> Error {
        Error::Db(error)
    }
}

impl From<crate::money::MoneyError> for Error {
    fn from(error: crate::money::MoneyError) -> Error {
        Error::Service(error.0)
    }
}

pub type Result<T> = std::result::Result<T, Error>;

/// Change a text to a form for comparisons without case (as Python's `str.casefold`).
pub fn casefold(text: &str) -> String {
    text.to_lowercase().replace('ß', "ss")
}
