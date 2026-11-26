//! Feljosit: a local-first personal budgeting application for the Swiss financial
//! sector. One executable file contains the web server, the templates and the static
//! files. The data is in one SQLite file.

pub mod camt053;
pub mod cardcsv;
pub mod cli;
pub mod config;
pub mod db;
pub mod iban;
pub mod models;
pub mod money;
pub mod services;
pub mod web;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
