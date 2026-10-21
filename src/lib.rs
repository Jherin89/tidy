//! Rules-driven cleanup for a messy Downloads folder.
//!
//! The binary is `tidy`. The Cargo package is named `tidy-dl` because `tidy`
//! is already taken on crates.io. Library entry points take an [`Env`] and a
//! [`Trasher`] so tests never touch a real home directory, config directory,
//! data directory, or the system Trash.

#![forbid(unsafe_code)]

pub mod app;
pub mod apply;
pub mod cli;
pub mod duplicates;
pub mod env;
pub mod error;
pub mod explain;
pub mod launchd;
pub mod logbook;
pub mod names;
pub mod parse;
pub mod plan;
pub mod privacy;
pub mod relocate;
pub mod report;
pub mod rules;
pub mod scan;
pub mod trash;
pub mod undo;
pub mod watch;

pub use env::Env;
pub use error::Error;
pub use trash::{FakeTrasher, RestoreResult, SystemTrasher, TrashRecord, Trasher};

/// Built-in rules written by `tidy init`.
pub const DEFAULT_RULES: &str = include_str!("default_rules.toml");
