//! Reverse the newest undo log that has not been fully undone.
//!
//! [`preview_latest`] and [`undo_latest`] share [`decide_entry`]. The dry run
//! stops after that decision. The real undo then performs it. A missing file,
//! a renamed collision, a fake-trash restore, and a macOS Finder Put Back
//! therefore cannot drift between the preview and the undo.

use std::collections::HashSet;
use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};

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
    /// Where the file is now (move destination, or the fake-trash path).
    pub from: Option<String>,
    pub outcome: UndoOutcome,
    /// The original name was taken, so the restore path has a ` (N)` suffix.
    pub renamed: bool,
    /// The file or Trash item was not found. Still a [`UndoOutcome::Failed`].
    pub missing: bool,
    pub message: String,
}

#[derive(Debug, Clone)]
pub struct UndoReport {
    pub log_path: std::path::PathBuf,
    pub log_id: String,
    pub source_folder: std::path::PathBuf,
    pub items: Vec<UndoItem>,
    pub fully_undone: bool,
    /// Preview only. Nothing was moved and the log was not rewritten.
    pub dry_run: bool,
}

impl UndoReport {
    pub fn failure_count(&self) -> usize {
        self.items
            .iter()
            .filter(|item| item.outcome == UndoOutcome::Failed)
            .count()
    }

    pub fn missing_count(&self) -> usize {
        self.items.iter().filter(|item| item.missing).count()
    }

    pub fn manual_count(&self) -> usize {
        self.items
            .iter()
            .filter(|item| item.outcome == UndoOutcome::Manual)
            .count()
    }
}

/// What undo would do for one log entry. No directories are created and no
/// files are moved while building this.
enum Decision {
    MoveBack {
        from: PathBuf,
        to: PathBuf,
        renamed: bool,
    },
    Restore {
        record: TrashRecord,
        to: PathBuf,
        renamed: bool,
    },
    Manual {
        message: String,
    },
    Failed {
        message: String,
        missing: bool,
    },
}

enum Executed {
    AsPlanned,
    /// `restore` reported a manual Put Back after the plan said it could move.
    Manual {
        message: String,
    },
    Failed(UndoItem),
}

/// List what the newest pending log would restore. Does not move files and
/// does not mark the log undone.
pub fn preview_latest(env: &Env, trasher: &dyn Trasher) -> Result<UndoReport, Error> {
    undo_log(env, trasher, true)
}

/// Reverse the most recent run that still has entries to undo.
pub fn undo_latest(env: &Env, trasher: &dyn Trasher) -> Result<UndoReport, Error> {
    undo_log(env, trasher, false)
}

fn undo_log(env: &Env, trasher: &dyn Trasher, dry_run: bool) -> Result<UndoReport, Error> {
    let Some((path, mut log)) = logbook::latest_pending(env)? else {
        return Err(Error::msg("nothing to undo"));
    };
    let mut reserved = HashSet::new();
    let mut items = Vec::new();
    for entry in &mut log.entries {
        if entry.reversed {
            continue;
        }
        let decision = decide_entry(entry, trasher, &mut reserved);
        if dry_run {
            items.push(describe(entry, &decision, true));
            continue;
        }
        match execute(entry, &decision, trasher) {
            Executed::AsPlanned => {
                let item = describe(entry, &decision, false);
                if item.outcome != UndoOutcome::Failed {
                    entry.reversed = true;
                }
                if item.outcome == UndoOutcome::Manual {
                    log.notes.push(item.message.clone());
                }
                items.push(item);
            }
            Executed::Manual { message } => {
                entry.reversed = true;
                log.notes.push(message.clone());
                items.push(UndoItem {
                    file_name: entry.file_name.clone(),
                    path: entry.original_path.display().to_string(),
                    from: entry.destination.as_ref().map(|p| p.display().to_string()),
                    outcome: UndoOutcome::Manual,
                    renamed: false,
                    missing: false,
                    message,
                });
            }
            Executed::Failed(item) => items.push(item),
        }
    }
    let fully_undone = if dry_run {
        false
    } else {
        log.undone = log.entries.iter().all(|entry| entry.reversed);
        if log.undone {
            remove_empty_created_dirs(&log.created_dirs);
            log.undone_at = Some(Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true));
        }
        logbook::save_log(&path, &log)?;
        log.undone
    };
    Ok(UndoReport {
        log_path: path,
        log_id: log.id,
        source_folder: log.source_folder,
        items,
        fully_undone,
        dry_run,
    })
}

/// Remove directories the run created, innermost first, but only when they
/// are empty again. `remove_dir` refuses non-empty directories, so nothing
/// the user put there afterwards can be lost.
fn remove_empty_created_dirs(dirs: &[std::path::PathBuf]) {
    for dir in dirs.iter().rev() {
        let _ = fs::remove_dir(dir);
    }
}

fn decide_entry(
    entry: &LogEntry,
    trasher: &dyn Trasher,
    reserved: &mut HashSet<PathBuf>,
) -> Decision {
    match entry.action.as_str() {
        "move" => decide_move(entry, reserved),
        "trash" => decide_trash(entry, trasher, reserved),
        other => Decision::Failed {
            message: format!("{}: unknown undo action `{other}`", entry.file_name),
            missing: false,
        },
    }
}

fn decide_move(entry: &LogEntry, reserved: &mut HashSet<PathBuf>) -> Decision {
    let Some(from) = entry.destination.clone() else {
        return Decision::Failed {
            message: "undo log has no destination for this move".to_string(),
            missing: false,
        };
    };
    if !from.exists() {
        return Decision::Failed {
            message: format!(
                "{} is missing, so it was not moved back (looked in {})",
                entry.file_name,
                from.display()
            ),
            missing: true,
        };
    }
    match restore_dest(entry, reserved) {
        Ok(to) => Decision::MoveBack {
            renamed: renamed(&entry.file_name, &to),
            from,
            to,
        },
        Err(message) => Decision::Failed {
            message,
            missing: false,
        },
    }
}

fn decide_trash(
    entry: &LogEntry,
    trasher: &dyn Trasher,
    reserved: &mut HashSet<PathBuf>,
) -> Decision {
    let to = match restore_dest(entry, reserved) {
        Ok(path) => path,
        Err(message) => {
            return Decision::Failed {
                message,
                missing: false,
            };
        }
    };
    let renamed_dest = renamed(&entry.file_name, &to);
    let record = TrashRecord {
        original_path: entry.original_path.clone(),
        file_name: entry.file_name.clone(),
        fake_path: entry.destination.clone(),
        trashed_at_unix: entry.trashed_at_unix.unwrap_or(0),
    };
    match trasher.inspect_restore(&record, &to) {
        Ok(RestoreResult::Restored) => Decision::Restore {
            record,
            to,
            renamed: renamed_dest,
        },
        Ok(RestoreResult::Manual { message }) => Decision::Manual { message },
        Err(err) => {
            let message = err.to_string();
            let missing = message.contains("is missing") || message.contains("could not find");
            Decision::Failed { message, missing }
        }
    }
}

/// Destination under the original parent. Does not create that parent.
/// `choose_destination` is the same call the real undo uses, so a collision
/// suffix in the preview is the suffix undo will use.
fn restore_dest(entry: &LogEntry, reserved: &mut HashSet<PathBuf>) -> Result<PathBuf, String> {
    let Some(parent) = entry.original_path.parent() else {
        return Err("original path has no parent directory".to_string());
    };
    if let Some(message) = parent_problem(parent) {
        return Err(message);
    }
    choose_destination(parent, &entry.file_name, reserved).map_err(|err| err.to_string())
}

fn parent_problem(parent: &Path) -> Option<String> {
    match fs::metadata(parent) {
        Ok(meta) if meta.is_dir() => None,
        Ok(_) => Some(format!(
            "could not create {}: not a directory",
            parent.display()
        )),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => None,
        Err(err) => Some(format!("could not create {}: {err}", parent.display())),
    }
}

fn renamed(file_name: &str, dest: &Path) -> bool {
    dest.file_name()
        .is_some_and(|name| name != OsStr::new(file_name))
}

fn execute(entry: &LogEntry, decision: &Decision, trasher: &dyn Trasher) -> Executed {
    match decision {
        Decision::Failed { .. } => Executed::Failed(describe(entry, decision, false)),
        Decision::Manual { .. } => Executed::AsPlanned,
        Decision::MoveBack { from, to, .. } => {
            if let Err(message) = ensure_parent(to) {
                return Executed::Failed(failed_item(entry, message, false));
            }
            match relocate(from, to) {
                Ok(()) => Executed::AsPlanned,
                Err(err) => Executed::Failed(failed_item(entry, err.to_string(), false)),
            }
        }
        Decision::Restore { record, to, .. } => {
            if let Err(message) = ensure_parent(to) {
                return Executed::Failed(failed_item(entry, message, false));
            }
            match trasher.restore(record, to) {
                Ok(RestoreResult::Restored) => Executed::AsPlanned,
                Ok(RestoreResult::Manual { message }) => Executed::Manual { message },
                Err(err) => Executed::Failed(failed_item(entry, err.to_string(), false)),
            }
        }
    }
}

fn ensure_parent(dest: &Path) -> Result<(), String> {
    let Some(parent) = dest.parent() else {
        return Ok(());
    };
    if parent.as_os_str().is_empty() {
        return Ok(());
    }
    fs::create_dir_all(parent)
        .map_err(|err| format!("could not create {}: {err}", parent.display()))
}

fn describe(entry: &LogEntry, decision: &Decision, dry_run: bool) -> UndoItem {
    match decision {
        Decision::MoveBack { from, to, renamed } => {
            let message = if dry_run {
                if *renamed {
                    format!(
                        "would move {} back to {} (destination occupied)",
                        entry.file_name,
                        to.display()
                    )
                } else {
                    format!("would move {} back to {}", entry.file_name, to.display())
                }
            } else {
                format!("moved {} back to {}", entry.file_name, to.display())
            };
            UndoItem {
                file_name: entry.file_name.clone(),
                path: to.display().to_string(),
                from: Some(from.display().to_string()),
                outcome: UndoOutcome::MovedBack,
                renamed: *renamed,
                missing: false,
                message,
            }
        }
        Decision::Restore {
            record,
            to,
            renamed,
        } => {
            let message = if dry_run {
                if *renamed {
                    format!(
                        "would restore {} from Trash to {} (destination occupied)",
                        entry.file_name,
                        to.display()
                    )
                } else {
                    format!(
                        "would restore {} from Trash to {}",
                        entry.file_name,
                        to.display()
                    )
                }
            } else {
                format!("restored {} to {}", entry.file_name, to.display())
            };
            UndoItem {
                file_name: entry.file_name.clone(),
                path: to.display().to_string(),
                from: record
                    .fake_path
                    .as_ref()
                    .map(|path| path.display().to_string()),
                outcome: UndoOutcome::Restored,
                renamed: *renamed,
                missing: false,
                message,
            }
        }
        Decision::Manual { message } => UndoItem {
            file_name: entry.file_name.clone(),
            path: entry.original_path.display().to_string(),
            from: entry
                .destination
                .as_ref()
                .map(|path| path.display().to_string()),
            outcome: UndoOutcome::Manual,
            renamed: false,
            missing: false,
            message: if dry_run {
                dry_manual_message(message)
            } else {
                message.clone()
            },
        },
        Decision::Failed { message, missing } => {
            failed_item(entry, present_failure(message, *missing, dry_run), *missing)
        }
    }
}

fn dry_manual_message(message: &str) -> String {
    if message.contains("macOS cannot restore") {
        format!("needs Finder Put Back. {message}")
    } else {
        message.to_string()
    }
}

fn present_failure(message: &str, missing: bool, dry_run: bool) -> String {
    if !dry_run || !missing {
        return message.to_string();
    }
    if message.contains("was not moved back") {
        message.replace("was not moved back", "would be skipped")
    } else {
        format!("missing, skip: {message}")
    }
}

fn failed_item(entry: &LogEntry, message: impl Into<String>, missing: bool) -> UndoItem {
    UndoItem {
        file_name: entry.file_name.clone(),
        path: entry.original_path.display().to_string(),
        from: entry
            .destination
            .as_ref()
            .map(|path| path.display().to_string()),
        outcome: UndoOutcome::Failed,
        renamed: false,
        missing,
        message: message.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dry_run_manual_message_names_finder_put_back() {
        let message = "notes.txt is in the Trash. macOS cannot restore Trash items programmatically; put it back yourself. Original location: /tmp/notes.txt";
        let dry = dry_manual_message(message);
        assert!(dry.starts_with("needs Finder Put Back."), "{dry}");
        assert!(dry.contains("macOS cannot restore"), "{dry}");
        assert!(dry.contains("put it back yourself"), "{dry}");
        assert_eq!(dry_manual_message("leave this"), "leave this");
    }
}
