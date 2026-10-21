//! Reverse the newest undo log that has not been fully undone.

use std::collections::HashSet;
use std::fs;

use chrono::{SecondsFormat, Utc};

use crate::env::Env;
use crate::error::Error;
use crate::logbook::{self, LogEntry};
use crate::plan::choose_destination;
use crate::relocate::relocate;
use crate::trash::{RestoreResult, TrashRecord, Trasher};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UndoOutcome {
    MovedBack,
    Restored,
    Manual,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UndoItem {
    pub file_name: String,
    pub path: String,
    pub outcome: UndoOutcome,
    pub message: String,
}

#[derive(Debug, Clone)]
pub struct UndoReport {
    pub log_path: std::path::PathBuf,
    pub log_id: String,
    pub source_folder: std::path::PathBuf,
    pub items: Vec<UndoItem>,
    pub fully_undone: bool,
}

impl UndoReport {
    pub fn failure_count(&self) -> usize {
        self.items
            .iter()
            .filter(|item| item.outcome == UndoOutcome::Failed)
            .count()
    }

    pub fn manual_count(&self) -> usize {
        self.items
            .iter()
            .filter(|item| item.outcome == UndoOutcome::Manual)
            .count()
    }
}

/// Reverse the most recent run that still has entries to undo.
pub fn undo_latest(env: &Env, trasher: &dyn Trasher) -> Result<UndoReport, Error> {
    let Some((path, mut log)) = logbook::latest_pending(env)? else {
        return Err(Error::msg("nothing to undo"));
    };
    let mut reserved = HashSet::new();
    let mut items = Vec::new();
    for entry in &mut log.entries {
        if entry.reversed {
            continue;
        }
        let item = reverse_entry(entry, trasher, &mut reserved);
        if item.outcome != UndoOutcome::Failed {
            entry.reversed = true;
        }
        if item.outcome == UndoOutcome::Manual {
            log.notes.push(item.message.clone());
        }
        items.push(item);
    }
    log.undone = log.entries.iter().all(|entry| entry.reversed);
    if log.undone {
        log.undone_at = Some(Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true));
    }
    logbook::save_log(&path, &log)?;
    Ok(UndoReport {
        log_path: path,
        log_id: log.id,
        source_folder: log.source_folder,
        items,
        fully_undone: log.undone,
    })
}

fn reverse_entry(
    entry: &LogEntry,
    trasher: &dyn Trasher,
    reserved: &mut HashSet<std::path::PathBuf>,
) -> UndoItem {
    match entry.action.as_str() {
        "move" => reverse_move(entry, reserved),
        "trash" => reverse_trash(entry, trasher, reserved),
        other => UndoItem {
            file_name: entry.file_name.clone(),
            path: entry.original_path.display().to_string(),
            outcome: UndoOutcome::Failed,
            message: format!("{}: unknown undo action `{other}`", entry.file_name),
        },
    }
}

fn reverse_move(entry: &LogEntry, reserved: &mut HashSet<std::path::PathBuf>) -> UndoItem {
    let Some(from) = &entry.destination else {
        return failed(entry, "undo log has no destination for this move");
    };
    if !from.exists() {
        return failed(
            entry,
            format!(
                "{} is missing, so it was not moved back (looked in {})",
                entry.file_name,
                from.display()
            ),
        );
    }
    let Some(parent) = entry.original_path.parent() else {
        return failed(entry, "original path has no parent directory");
    };
    if let Err(err) = fs::create_dir_all(parent) {
        return failed(
            entry,
            format!("could not create {}: {err}", parent.display()),
        );
    }
    let dest = match choose_destination(parent, &entry.file_name, reserved) {
        Ok(path) => path,
        Err(err) => return failed(entry, err.to_string()),
    };
    match relocate(from, &dest) {
        Ok(()) => UndoItem {
            file_name: entry.file_name.clone(),
            path: dest.display().to_string(),
            outcome: UndoOutcome::MovedBack,
            message: format!("moved {} back to {}", entry.file_name, dest.display()),
        },
        Err(err) => failed(entry, err.to_string()),
    }
}

fn reverse_trash(
    entry: &LogEntry,
    trasher: &dyn Trasher,
    reserved: &mut HashSet<std::path::PathBuf>,
) -> UndoItem {
    let Some(parent) = entry.original_path.parent() else {
        return failed(entry, "original path has no parent directory");
    };
    if let Err(err) = fs::create_dir_all(parent) {
        return failed(
            entry,
            format!("could not create {}: {err}", parent.display()),
        );
    }
    let dest = match choose_destination(parent, &entry.file_name, reserved) {
        Ok(path) => path,
        Err(err) => return failed(entry, err.to_string()),
    };
    let record = TrashRecord {
        original_path: entry.original_path.clone(),
        file_name: entry.file_name.clone(),
        fake_path: entry.destination.clone(),
        trashed_at_unix: entry.trashed_at_unix.unwrap_or(0),
    };
    match trasher.restore(&record, &dest) {
        Ok(RestoreResult::Restored) => UndoItem {
            file_name: entry.file_name.clone(),
            path: dest.display().to_string(),
            outcome: UndoOutcome::Restored,
            message: format!("restored {} to {}", entry.file_name, dest.display()),
        },
        Ok(RestoreResult::Manual { message }) => UndoItem {
            file_name: entry.file_name.clone(),
            path: entry.original_path.display().to_string(),
            outcome: UndoOutcome::Manual,
            message,
        },
        Err(err) => failed(entry, err.to_string()),
    }
}

fn failed(entry: &LogEntry, message: impl Into<String>) -> UndoItem {
    UndoItem {
        file_name: entry.file_name.clone(),
        path: entry.original_path.display().to_string(),
        outcome: UndoOutcome::Failed,
        message: message.into(),
    }
}
