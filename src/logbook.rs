//! JSON undo logs.
//!
//! Each applied run writes `run-<UTC timestamp>.json` under
//! `data_dir/tidy/logs`. The log lists every file that was actually moved or
//! trashed. Undo flips `reversed` on each entry it puts back and sets
//! `undone` once every entry is reversed.

use std::fs;
use std::path::{Path, PathBuf};

use chrono::{DateTime, SecondsFormat, Utc};
use serde::{Deserialize, Serialize};

use crate::env::Env;
use crate::error::Error;
use crate::plan::{ActionKind, ActionStatus, Plan};

pub const LOG_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RunLog {
    pub version: u32,
    pub id: String,
    pub created_at: String,
    pub source_folder: PathBuf,
    pub undone: bool,
    #[serde(default)]
    pub undone_at: Option<String>,
    pub entries: Vec<LogEntry>,
    #[serde(default)]
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LogEntry {
    pub original_path: PathBuf,
    pub file_name: String,
    /// `move` or `trash`.
    pub action: String,
    /// Move destination, or the fake-trash path when one was used.
    pub destination: Option<PathBuf>,
    pub trashed_at_unix: Option<i64>,
    #[serde(default)]
    pub reversed: bool,
}

/// Write a log for the actions that succeeded. Returns `None` when nothing
/// succeeded, so a failed run does not invent something to undo.
pub fn write_run_log(env: &Env, plan: &Plan) -> Result<Option<PathBuf>, Error> {
    let entries: Vec<LogEntry> = plan
        .actions
        .iter()
        .filter(|action| action.status == ActionStatus::Applied)
        .map(|action| LogEntry {
            original_path: action.file.path.clone(),
            file_name: action.file.name.clone(),
            action: match action.kind {
                ActionKind::Move => "move".to_string(),
                ActionKind::Trash => "trash".to_string(),
            },
            destination: action.destination.clone(),
            trashed_at_unix: action.trashed_at_unix,
            reversed: false,
        })
        .collect();
    if entries.is_empty() {
        return Ok(None);
    }

    let dir = env.logs_dir();
    fs::create_dir_all(&dir)
        .map_err(|err| Error::io(format!("creating {}", dir.display()), err))?;
    let created = Utc::now();
    let (id, path) = unique_log_path(&dir, created);
    let log = RunLog {
        version: LOG_VERSION,
        id: id.clone(),
        created_at: created.to_rfc3339_opts(SecondsFormat::Millis, true),
        source_folder: plan.folder.clone(),
        undone: false,
        undone_at: None,
        entries,
        notes: Vec::new(),
    };
    save_log(&path, &log)?;
    Ok(Some(path))
}

pub fn save_log(path: &Path, log: &RunLog) -> Result<(), Error> {
    let bytes = serde_json::to_vec_pretty(log)
        .map_err(|err| Error::msg(format!("could not encode undo log: {err}")))?;
    fs::write(path, bytes).map_err(|err| Error::io(format!("writing {}", path.display()), err))?;
    Ok(())
}

pub fn read_log(path: &Path) -> Result<RunLog, Error> {
    let text = fs::read_to_string(path)
        .map_err(|err| Error::io(format!("reading {}", path.display()), err))?;
    serde_json::from_str(&text).map_err(|err| {
        Error::msg(format!(
            "could not parse undo log {}: {err}",
            path.display()
        ))
    })
}

/// Newest log that still has work to undo.
pub fn latest_pending(env: &Env) -> Result<Option<(PathBuf, RunLog)>, Error> {
    let dir = env.logs_dir();
    if !dir.exists() {
        return Ok(None);
    }
    let mut logs = Vec::new();
    let reader =
        fs::read_dir(&dir).map_err(|err| Error::io(format!("reading {}", dir.display()), err))?;
    for item in reader {
        let item = item.map_err(|err| Error::io(format!("reading {}", dir.display()), err))?;
        let name = item.file_name();
        let name = name.to_string_lossy();
        if name.starts_with("run-") && name.ends_with(".json") {
            logs.push(item.path());
        }
    }
    logs.sort();
    for path in logs.into_iter().rev() {
        let log = read_log(&path)?;
        if !log.undone && log.entries.iter().any(|entry| !entry.reversed) {
            return Ok(Some((path, log)));
        }
    }
    Ok(None)
}

fn unique_log_path(dir: &Path, created: DateTime<Utc>) -> (String, PathBuf) {
    let base = format!(
        "run-{}{:06}Z",
        created.format("%Y%m%dT%H%M%S"),
        created.timestamp_subsec_micros()
    );
    let mut id = base.clone();
    let mut path = dir.join(format!("{id}.json"));
    let mut n = 1u32;
    while path.exists() {
        id = format!("{base}_{n}");
        path = dir.join(format!("{id}.json"));
        n += 1;
    }
    (id, path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plan::{ActionStatus, PlannedAction};
    use crate::scan::FileEntry;
    use std::time::{Duration, SystemTime};

    fn env_at(root: &Path) -> Env {
        Env {
            home: root.join("home"),
            config_dir: root.join("config"),
            config_from_env: true,
            data_dir: root.join("data"),
            fake_trash: Some(root.join("trash")),
        }
    }

    #[test]
    fn round_trip_picks_the_newest_pending_log() {
        let tmp = tempfile::tempdir().unwrap();
        let env = env_at(tmp.path());
        let folder = env.home.join("Downloads");
        let mut plan = Plan {
            folder: folder.clone(),
            rules_source_label: "test".into(),
            rules_path: None,
            actions: vec![PlannedAction {
                file: FileEntry {
                    path: folder.join("a.pdf"),
                    name: "a.pdf".into(),
                    size: 3,
                    modified: SystemTime::now() - Duration::from_secs(5),
                },
                rule_name: "PDFs".into(),
                kind: ActionKind::Move,
                destination: Some(env.home.join("Documents/PDFs/a.pdf")),
                destination_display: "~/Documents/PDFs/a.pdf".into(),
                status: ActionStatus::Applied,
                error: None,
                trashed_at_unix: None,
            }],
            unmatched: vec![],
            skipped: vec![],
            applied: true,
            log_path: None,
        };
        let first = write_run_log(&env, &plan).unwrap().unwrap();
        plan.actions[0].file.name = "b.pdf".into();
        plan.actions[0].file.path = folder.join("b.pdf");
        std::thread::sleep(Duration::from_millis(2));
        let second = write_run_log(&env, &plan).unwrap().unwrap();
        assert_ne!(first, second);

        let (path, log) = latest_pending(&env).unwrap().unwrap();
        assert_eq!(path, second);
        assert_eq!(log.entries[0].file_name, "b.pdf");
        assert!(!log.undone);

        let mut done = log;
        done.undone = true;
        done.entries[0].reversed = true;
        save_log(&second, &done).unwrap();

        let (path, log) = latest_pending(&env).unwrap().unwrap();
        assert_eq!(path, first);
        assert_eq!(log.entries[0].file_name, "a.pdf");
    }
}
