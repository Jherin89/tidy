//! Turn candidate files and rules into a concrete plan.
//!
//! Planning does not create directories and does not move anything.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use chrono::{DateTime, Utc};

use crate::age::{AgeProbe, SystemAge};
use crate::duplicates::DuplicateIndex;
use crate::env::{contract_home, expand_tilde, Env};
use crate::error::Error;
use crate::names::collision_name;
use crate::rules::{is_older_than, Action, Ruleset};
use crate::scan::{self, inspect, FileEntry, Inspect, Skipped};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActionKind {
    Move,
    Trash,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActionStatus {
    Planned,
    Applied,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlannedAction {
    pub file: FileEntry,
    pub rule_name: String,
    pub kind: ActionKind,
    /// Final path for a move. `None` for trash until a fake-trash backend
    /// records where the file went.
    pub destination: Option<PathBuf>,
    /// Destination as shown to a person, with `~` contracted.
    pub destination_display: String,
    pub status: ActionStatus,
    pub error: Option<String>,
    /// Unix seconds recorded when a file was trashed. Used to find the
    /// matching Trash item on undo. `None` for moves and for planned trash.
    pub trashed_at_unix: Option<i64>,
    /// Set when the winning rule matched `duplicate = true`.
    pub copy_of: Option<String>,
}

#[derive(Debug, Clone)]
pub struct Plan {
    /// Directories that `--apply` had to create, outermost first. Undo removes
    /// them again if they are empty.
    pub created_dirs: Vec<PathBuf>,
    pub folder: PathBuf,
    pub rules_source_label: String,
    pub rules_path: Option<PathBuf>,
    pub actions: Vec<PlannedAction>,
    pub unmatched: Vec<FileEntry>,
    pub skipped: Vec<Skipped>,
    pub applied: bool,
    pub log_path: Option<PathBuf>,
}

impl Plan {
    pub fn matched_size(&self) -> u64 {
        self.actions.iter().map(|a| a.file.size).sum()
    }

    pub fn applied_size(&self) -> u64 {
        self.actions
            .iter()
            .filter(|a| a.status == ActionStatus::Applied)
            .map(|a| a.file.size)
            .sum()
    }

    pub fn failure_count(&self) -> usize {
        self.actions
            .iter()
            .filter(|a| a.status == ActionStatus::Failed)
            .count()
    }
}

/// Eligible files, the skip list, and the duplicate index for one folder.
///
/// The index is built from every eligible file, then `--older-than` removes
/// young files from the plan. A young keeper still makes an old extra copy
/// match `duplicate = true`.
pub struct FolderView {
    pub entries: Vec<FileEntry>,
    pub skipped: Vec<Skipped>,
    pub duplicates: DuplicateIndex,
}

pub fn view_folder(
    folder: &Path,
    older_than: Option<std::time::Duration>,
    now: SystemTime,
) -> Result<FolderView, Error> {
    view_folder_with(folder, older_than, now, &[], &SystemAge)
}

pub fn view_folder_with(
    folder: &Path,
    older_than: Option<std::time::Duration>,
    now: SystemTime,
    bundle_exts: &[String],
    ages: &dyn AgeProbe,
) -> Result<FolderView, Error> {
    let (all_entries, mut skipped) = scan::scan_with(folder, None, now, bundle_exts, ages)?;
    let duplicates = DuplicateIndex::build(folder, &all_entries)?;
    let entries = if let Some(min_age) = older_than {
        let mut kept = Vec::new();
        for file in all_entries {
            if is_older_than(file.age, now, min_age) {
                kept.push(file);
            } else {
                skipped.push(Skipped {
                    path: file.path,
                    name: file.name,
                    reason: scan::SkipReason::NewerThanFilter,
                });
            }
        }
        skipped.sort_by(|a, b| scan::compare_names(&a.name, &b.name));
        kept
    } else {
        all_entries
    };
    Ok(FolderView {
        entries,
        skipped,
        duplicates,
    })
}

/// Scan `folder` and build a plan. `now` is injectable so tests are stable.
pub fn plan_folder(
    env: &Env,
    folder: &Path,
    rules: &Ruleset,
    older_than: Option<std::time::Duration>,
    now: SystemTime,
) -> Result<Plan, Error> {
    plan_folder_with(env, folder, rules, older_than, now, &SystemAge)
}

pub fn plan_folder_with(
    env: &Env,
    folder: &Path,
    rules: &Ruleset,
    older_than: Option<std::time::Duration>,
    now: SystemTime,
    ages: &dyn AgeProbe,
) -> Result<Plan, Error> {
    let bundles = rules.bundle_extensions();
    let view = view_folder_with(folder, older_than, now, &bundles, ages)?;
    plan_entries(
        env,
        folder,
        view.entries,
        view.skipped,
        rules,
        now,
        &view.duplicates,
    )
}

/// Plan a watch batch. Duplicate detection still sees the whole folder, so a
/// new file identical to one already there is an extra copy.
pub fn plan_watched(
    env: &Env,
    folder: &Path,
    entries: Vec<FileEntry>,
    rules: &Ruleset,
    now: SystemTime,
) -> Result<Plan, Error> {
    let duplicates = DuplicateIndex::for_folder(folder, now, &rules.bundle_extensions())?;
    plan_entries(env, folder, entries, Vec::new(), rules, now, &duplicates)
}

pub fn plan_entries(
    env: &Env,
    folder: &Path,
    entries: Vec<FileEntry>,
    skipped: Vec<Skipped>,
    rules: &Ruleset,
    now: SystemTime,
    duplicates: &DuplicateIndex,
) -> Result<Plan, Error> {
    let mut reserved = HashSet::new();
    let mut actions = Vec::new();
    let mut unmatched = Vec::new();

    for file in entries {
        match first_match(rules, &file, now, duplicates) {
            None => unmatched.push(file),
            Some(rule) => {
                let planned = plan_one(env, folder, &file, rule, duplicates, &mut reserved)?;
                actions.push(planned);
            }
        }
    }

    let rules_source_label = match rules.source {
        crate::rules::RulesSource::BuiltIn => "built-in defaults".to_string(),
        crate::rules::RulesSource::File => rules
            .path
            .as_ref()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| "rules file".to_string()),
    };

    Ok(Plan {
        created_dirs: Vec::new(),
        folder: folder.to_path_buf(),
        rules_source_label,
        rules_path: rules.path.clone(),
        actions,
        unmatched,
        skipped,
        applied: false,
        log_path: None,
    })
}

fn first_match<'a>(
    rules: &'a Ruleset,
    file: &FileEntry,
    now: SystemTime,
    duplicates: &DuplicateIndex,
) -> Option<&'a crate::rules::Rule> {
    rules
        .rules
        .iter()
        .find(|rule| rule.matches(file, now, duplicates))
}

fn plan_one(
    env: &Env,
    folder: &Path,
    file: &FileEntry,
    rule: &crate::rules::Rule,
    duplicates: &DuplicateIndex,
    reserved: &mut HashSet<PathBuf>,
) -> Result<PlannedAction, Error> {
    let copy_of = if rule.duplicate == Some(true) {
        duplicates.keeper_of(&file.path).map(str::to_string)
    } else {
        None
    };
    match &rule.action {
        Action::Trash => {
            let destination_display = match &copy_of {
                Some(keeper) => format!("Trash (copy of {keeper})"),
                None => "Trash".to_string(),
            };
            Ok(PlannedAction {
                file: file.clone(),
                rule_name: rule.name.clone(),
                kind: ActionKind::Trash,
                destination: None,
                destination_display,
                status: ActionStatus::Planned,
                error: None,
                trashed_at_unix: None,
                copy_of,
            })
        }
        Action::Move {
            destination,
            by_date,
        } => {
            let mut dir = resolve_destination(destination, &env.home, folder);
            if *by_date {
                dir = dir.join(month_bucket(file.modified));
            }
            let path = choose_destination(&dir, &file.name, reserved)?;
            let display = contract_home(&path, &env.home);
            Ok(PlannedAction {
                file: file.clone(),
                rule_name: rule.name.clone(),
                kind: ActionKind::Move,
                destination: Some(path),
                destination_display: display,
                status: ActionStatus::Planned,
                error: None,
                trashed_at_unix: None,
                copy_of,
            })
        }
    }
}

pub fn resolve_destination(raw: &str, home: &Path, folder: &Path) -> PathBuf {
    let expanded = expand_tilde(raw, home);
    if expanded.is_absolute() {
        expanded
    } else {
        folder.join(expanded)
    }
}

/// `YYYY-MM` in the local timezone, from the file's modified time.
///
/// `chrono` stays because the standard library cannot convert a timestamp to
/// local time. The bucket has to follow the machine's timezone, including DST.
pub fn month_bucket(modified: SystemTime) -> String {
    let Ok(dur) = modified.duration_since(SystemTime::UNIX_EPOCH) else {
        return "1970-01".to_string();
    };
    let secs = i64::try_from(dur.as_secs()).unwrap_or(i64::MAX);
    let Some(utc) = DateTime::<Utc>::from_timestamp(secs, 0) else {
        return "1970-01".to_string();
    };
    utc.with_timezone(&chrono::Local)
        .format("%Y-%m")
        .to_string()
}

/// Pick a destination path that does not exist and was not already chosen.
///
/// The first try uses `file_name` unchanged. Later tries insert ` (1)`,
/// ` (2)`, … before the extension.
pub fn choose_destination(
    dir: &Path,
    file_name: &str,
    reserved: &mut HashSet<PathBuf>,
) -> Result<PathBuf, Error> {
    let mut n = 0u32;
    loop {
        let name = if n == 0 {
            file_name.to_string()
        } else {
            collision_name(file_name, n)
        };
        let candidate = dir.join(&name);
        if !reserved.contains(&candidate) && !occupied(&candidate) {
            reserved.insert(candidate.clone());
            return Ok(candidate);
        }
        n += 1;
        if n > 10_000 {
            return Err(Error::msg(format!(
                "could not find a free name for {file_name} in {}",
                dir.display()
            )));
        }
    }
}

pub fn occupied(path: &Path) -> bool {
    std::fs::symlink_metadata(path).is_ok()
}

/// Re-check a single path while watching. Uses the same rules as a full scan.
pub fn inspect_one(
    folder: &Path,
    path: &Path,
    older_than: Option<std::time::Duration>,
    now: SystemTime,
) -> Result<Inspect, Error> {
    inspect(folder, path, older_than, now)
}

pub fn inspect_one_with(
    folder: &Path,
    path: &Path,
    older_than: Option<std::time::Duration>,
    now: SystemTime,
    bundle_exts: &[String],
    ages: &dyn AgeProbe,
) -> Result<Inspect, Error> {
    scan::inspect_with(folder, path, older_than, now, bundle_exts, ages)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rules::{RulesSource, Ruleset};
    use crate::scan::SkipReason;
    use std::fs;
    use std::time::Duration;

    fn test_env(root: &Path) -> Env {
        Env {
            home: root.join("home"),
            config_dir: root.join("config"),
            config_from_env: true,
            data_dir: root.join("data"),
            fake_trash: Some(root.join("trash")),
        }
    }

    fn ruleset(text: &str) -> Ruleset {
        Ruleset {
            source: RulesSource::File,
            path: Some(PathBuf::from("/tmp/rules.toml")),
            candidate_path: PathBuf::from("/tmp/rules.toml"),
            rules: Ruleset::parse(text).unwrap(),
        }
    }

    fn file_at(folder: &Path, name: &str, bytes: &[u8], age: Duration) -> FileEntry {
        let path = folder.join(name);
        fs::write(&path, bytes).unwrap();
        let now = SystemTime::now();
        let modified = now - age;
        let file = fs::File::options().write(true).open(&path).unwrap();
        file.set_modified(modified).unwrap();
        FileEntry::new(path, name, bytes.len() as u64, modified)
    }

    #[test]
    fn plan_matches_first_rule_and_leaves_the_rest() {
        let tmp = tempfile::tempdir().unwrap();
        let env = test_env(tmp.path());
        let folder = env.home.join("Downloads");
        fs::create_dir_all(&folder).unwrap();
        let now = SystemTime::now();
        let pdf = file_at(&folder, "Report.PDF", b"abc", Duration::from_secs(3));
        let notes = file_at(&folder, "notes.txt", b"hi", Duration::from_secs(3));
        let mystery = file_at(&folder, "data.bin", b"z", Duration::from_secs(3));
        // Re-read modified times from disk via the entries we stored.
        let _ = now;
        let set = ruleset(
            r#"
            [[rules]]
            name = "Notes first"
            glob = "notes*"
            action = "trash"

            [[rules]]
            name = "PDFs"
            extensions = ["pdf"]
            action = "move"
            destination = "~/Documents/PDFs"

            [[rules]]
            name = "Text"
            extensions = ["txt"]
            action = "move"
            destination = "~/Documents/Downloads"
            "#,
        );
        let plan = plan_entries(
            &env,
            &folder,
            vec![pdf, notes, mystery.clone()],
            vec![],
            &set,
            SystemTime::now(),
            &DuplicateIndex::empty(),
        )
        .unwrap();
        assert_eq!(plan.actions.len(), 2);
        assert_eq!(plan.actions[0].rule_name, "PDFs");
        assert_eq!(plan.actions[0].kind, ActionKind::Move);
        assert_eq!(
            plan.actions[0].destination.as_ref().unwrap(),
            &env.home.join("Documents/PDFs/Report.PDF")
        );
        assert_eq!(plan.actions[1].rule_name, "Notes first");
        assert_eq!(plan.actions[1].kind, ActionKind::Trash);
        assert_eq!(plan.unmatched.len(), 1);
        assert_eq!(plan.unmatched[0].name, mystery.name);
        assert!(!env.home.join("Documents/PDFs").exists());
    }

    #[test]
    fn collisions_are_numbered_before_the_extension() {
        let tmp = tempfile::tempdir().unwrap();
        let env = test_env(tmp.path());
        let folder = env.home.join("Downloads");
        let dest = env.home.join("Documents/Downloads");
        fs::create_dir_all(&folder).unwrap();
        fs::create_dir_all(&dest).unwrap();
        fs::write(dest.join("notes.txt"), b"existing").unwrap();
        fs::write(dest.join("notes (1).txt"), b"also existing").unwrap();

        let set = ruleset(
            r#"
            [[rules]]
            name = "Text"
            extensions = ["txt"]
            action = "move"
            destination = "~/Documents/Downloads"
            "#,
        );
        let entries = vec![
            file_at(&folder, "notes.txt", b"new", Duration::from_secs(1)),
            file_at(&folder, "notes (1).txt", b"new2", Duration::from_secs(1)),
        ];
        let plan = plan_entries(
            &env,
            &folder,
            entries,
            vec![],
            &set,
            SystemTime::now(),
            &DuplicateIndex::empty(),
        )
        .unwrap();
        let dests: Vec<_> = plan
            .actions
            .iter()
            .map(|a| {
                a.destination
                    .as_ref()
                    .unwrap()
                    .file_name()
                    .unwrap()
                    .to_string_lossy()
                    .into_owned()
            })
            .collect();
        assert_eq!(dests, vec!["notes (2).txt", "notes (1) (1).txt"]);
        assert_eq!(fs::read(dest.join("notes.txt")).unwrap(), b"existing");
    }

    #[test]
    fn by_date_appends_the_local_month_from_mtime() {
        let tmp = tempfile::tempdir().unwrap();
        let env = test_env(tmp.path());
        let folder = env.home.join("Downloads");
        fs::create_dir_all(&folder).unwrap();
        let entry = file_at(
            &folder,
            "Screen Shot 1.png",
            b"img",
            Duration::from_secs(40 * 86400),
        );
        let bucket = month_bucket(entry.modified);
        let set = ruleset(
            r#"
            [[rules]]
            name = "Shots"
            glob = "Screen Shot*"
            action = "move"
            destination = "~/Pictures/Screenshots"
            by_date = true
            "#,
        );
        let plan = plan_entries(
            &env,
            &folder,
            vec![entry],
            vec![Skipped {
                path: folder.join("nested"),
                name: "nested".into(),
                reason: SkipReason::Directory,
            }],
            &set,
            SystemTime::now(),
            &DuplicateIndex::empty(),
        )
        .unwrap();
        assert_eq!(
            plan.actions[0].destination.as_ref().unwrap(),
            &env.home
                .join("Pictures/Screenshots")
                .join(&bucket)
                .join("Screen Shot 1.png")
        );
        assert_eq!(plan.skipped.len(), 1);
    }

    #[test]
    fn relative_destination_stays_inside_the_folder() {
        let tmp = tempfile::tempdir().unwrap();
        let env = test_env(tmp.path());
        let folder = tmp.path().join("inbox");
        fs::create_dir_all(&folder).unwrap();
        let entry = file_at(&folder, "old.zip", b"zip", Duration::from_secs(20 * 86400));
        let set = ruleset(
            r#"
            [[rules]]
            name = "Archives"
            extensions = ["zip"]
            older_than = "14d"
            action = "move"
            destination = "Archives"
            "#,
        );
        let plan = plan_entries(
            &env,
            &folder,
            vec![entry],
            vec![],
            &set,
            SystemTime::now(),
            &DuplicateIndex::empty(),
        )
        .unwrap();
        assert_eq!(
            plan.actions[0].destination.as_ref().unwrap(),
            &folder.join("Archives/old.zip")
        );
    }
}
