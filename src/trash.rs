//! Trash backends.
//!
//! [`SystemTrasher`] sends files to the operating-system Trash through the
//! `trash` crate and never deletes a file outright. Restore uses
//! `trash::os_limited` on Linux and Windows. macOS cannot restore
//! programmatically; undo reports that clearly.
//!
//! [`FakeTrasher`] moves files into a directory. Tests and demos set
//! `TIDY_FAKE_TRASH` so they never touch the real Trash.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::error::Error;
use crate::plan::choose_destination;
use crate::relocate::relocate;

/// What `trash` did, enough to undo it later.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrashRecord {
    pub original_path: PathBuf,
    pub file_name: String,
    pub fake_path: Option<PathBuf>,
    pub trashed_at_unix: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RestoreResult {
    Restored,
    /// The file is still in the Trash and a person has to put it back.
    Manual {
        message: String,
    },
}

pub trait Trasher {
    fn trash(&self, path: &Path) -> Result<TrashRecord, Error>;
    fn restore(&self, record: &TrashRecord, dest: &Path) -> Result<RestoreResult, Error>;
}

/// Real system Trash. Do not construct this in tests.
#[derive(Debug, Default)]
pub struct SystemTrasher;

impl Trasher for SystemTrasher {
    fn trash(&self, path: &Path) -> Result<TrashRecord, Error> {
        let file_name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .ok_or_else(|| Error::msg(format!("cannot trash {}", path.display())))?;
        let original_path = path.to_path_buf();
        trash::delete(path).map_err(|err| {
            Error::msg(format!(
                "could not move {} to the Trash: {err}",
                original_path.display()
            ))
        })?;
        Ok(TrashRecord {
            original_path,
            file_name,
            fake_path: None,
            trashed_at_unix: unix_now(),
        })
    }

    fn restore(&self, record: &TrashRecord, dest: &Path) -> Result<RestoreResult, Error> {
        restore_system(record, dest)
    }
}

/// Moves files into `root`, which stands in for the Trash.
#[derive(Debug, Clone)]
pub struct FakeTrasher {
    pub root: PathBuf,
}

impl Trasher for FakeTrasher {
    fn trash(&self, path: &Path) -> Result<TrashRecord, Error> {
        let file_name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .ok_or_else(|| Error::msg(format!("cannot trash {}", path.display())))?;
        fs::create_dir_all(&self.root)
            .map_err(|err| Error::io(format!("creating trash dir {}", self.root.display()), err))?;
        let mut reserved = std::collections::HashSet::new();
        let dest = choose_destination(&self.root, &file_name, &mut reserved)?;
        relocate(path, &dest)?;
        Ok(TrashRecord {
            original_path: path.to_path_buf(),
            file_name,
            fake_path: Some(dest),
            trashed_at_unix: unix_now(),
        })
    }

    fn restore(&self, record: &TrashRecord, dest: &Path) -> Result<RestoreResult, Error> {
        let Some(fake) = &record.fake_path else {
            return Err(Error::msg(format!(
                "cannot restore {}: the undo log has no fake-trash path",
                record.file_name
            )));
        };
        if !fake.exists() {
            return Err(Error::msg(format!(
                "cannot restore {}: {} is missing from the fake trash",
                record.file_name,
                fake.display()
            )));
        }
        relocate(fake, dest)?;
        Ok(RestoreResult::Restored)
    }
}

pub fn from_env(env: &crate::env::Env) -> Box<dyn Trasher> {
    if let Some(dir) = &env.fake_trash {
        Box::new(FakeTrasher { root: dir.clone() })
    } else {
        Box::new(SystemTrasher)
    }
}

fn unix_now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
        .unwrap_or(0)
}

#[cfg(not(any(target_os = "linux", target_os = "windows")))]
fn restore_system(record: &TrashRecord, _dest: &Path) -> Result<RestoreResult, Error> {
    Ok(RestoreResult::Manual {
        message: macos_manual_message(record),
    })
}

#[cfg(any(target_os = "linux", target_os = "windows"))]
fn restore_system(record: &TrashRecord, dest: &Path) -> Result<RestoreResult, Error> {
    if dest != record.original_path {
        return Ok(RestoreResult::Manual {
            message: format!(
                "{} is still in the Trash because {} is occupied. tidy will not overwrite it; restore the file yourself.",
                record.file_name,
                record.original_path.display()
            ),
        });
    }
    let items = trash::os_limited::list()
        .map_err(|err| Error::msg(format!("could not list the Trash: {err}")))?;
    let mut matches: Vec<_> = items
        .into_iter()
        .filter(|item| trash_item_matches(item, record))
        .collect();
    if matches.is_empty() {
        return Err(Error::msg(format!(
            "could not find {} in the Trash (original path {})",
            record.file_name,
            record.original_path.display()
        )));
    }
    matches.sort_by_key(|item| (item.time_deleted - record.trashed_at_unix).saturating_abs());
    let item = matches.remove(0);
    match trash::os_limited::restore_all(std::iter::once(item)) {
        Ok(()) => Ok(RestoreResult::Restored),
        Err(trash::Error::RestoreCollision { path, .. }) => Ok(RestoreResult::Manual {
            message: format!(
                "{} is still in the Trash because {} is occupied. tidy will not overwrite it; restore the file yourself.",
                record.file_name,
                path.display()
            ),
        }),
        Err(err) => Err(Error::msg(format!(
            "could not restore {} from the Trash: {err}",
            record.file_name
        ))),
    }
}

#[cfg(any(target_os = "linux", target_os = "windows"))]
fn trash_item_matches(item: &trash::TrashItem, record: &TrashRecord) -> bool {
    if item.original_path() == record.original_path {
        return true;
    }
    let name_matches = item.name == std::ffi::OsString::from(&record.file_name);
    let parent_matches = record
        .original_path
        .parent()
        .is_some_and(|parent| parent == item.original_parent);
    name_matches && parent_matches
}

#[cfg(not(any(target_os = "linux", target_os = "windows")))]
fn macos_manual_message(record: &TrashRecord) -> String {
    format!(
        "{} is in the Trash. macOS cannot restore Trash items programmatically; put it back yourself. Original location: {}",
        record.file_name,
        record.original_path.display()
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn fake_trash_round_trip_and_collision_restore() {
        let tmp = tempfile::tempdir().unwrap();
        let trash_dir = tmp.path().join("trash");
        let src_dir = tmp.path().join("Downloads");
        fs::create_dir_all(&src_dir).unwrap();
        let src = src_dir.join("old.dmg");
        fs::write(&src, b"installer").unwrap();

        let trasher = FakeTrasher {
            root: trash_dir.clone(),
        };
        let record = trasher.trash(&src).unwrap();
        assert!(!src.exists());
        let fake = record.fake_path.clone().unwrap();
        assert_eq!(fs::read(&fake).unwrap(), b"installer");
        assert!(fake.starts_with(&trash_dir));

        fs::write(&src, b"a newer file").unwrap();
        let dest = src_dir.join("old (1).dmg");
        let result = trasher.restore(&record, &dest).unwrap();
        assert_eq!(result, RestoreResult::Restored);
        assert_eq!(fs::read(&src).unwrap(), b"a newer file");
        assert_eq!(fs::read(&dest).unwrap(), b"installer");
        assert!(!fake.exists());
    }
}
