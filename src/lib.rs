//! Feljosit: a local-first personal budgeting application for the Swiss financial
//! sector. One executable file contains the web server, the templates and the static
//! files. The data is in one SQLite file.

pub mod camt053;
pub mod cardcsv;
pub mod cli;
pub mod config;
pub mod dates;
pub mod db;
pub mod encoding;
pub mod iban;
pub mod models;
pub mod money;
pub mod net;
pub mod services;
pub mod shutdown;
pub mod terminal;
pub mod web;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
