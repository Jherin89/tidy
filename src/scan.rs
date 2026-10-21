//! Discover the top-level files a run is allowed to consider.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use crate::error::Error;
use crate::names::{is_hidden_name, is_partial_name};
use crate::rules::is_older_than;

/// A regular file tidy may match against the rules.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileEntry {
    pub path: PathBuf,
    pub name: String,
    pub size: u64,
    pub modified: SystemTime,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SkipReason {
    Hidden,
    PartialDownload,
    Directory,
    Symlink,
    NotRegular,
    NewerThanFilter,
}

impl SkipReason {
    pub fn as_str(self) -> &'static str {
        match self {
            SkipReason::Hidden => "hidden",
            SkipReason::PartialDownload => "partial download",
            SkipReason::Directory => "directory",
            SkipReason::Symlink => "symlink",
            SkipReason::NotRegular => "not a regular file",
            SkipReason::NewerThanFilter => "newer than --older-than",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Skipped {
    pub path: PathBuf,
    pub name: String,
    pub reason: SkipReason,
}

#[derive(Debug)]
pub enum Inspect {
    Candidate(FileEntry),
    Skip(Skipped),
    /// Not a direct child of the folder being tidied.
    Outside,
    Gone,
}

/// Classify one path. Only a regular file that is a direct child of `folder`
/// can become a candidate. Symlinks are never followed.
pub fn inspect(
    folder: &Path,
    path: &Path,
    older_than: Option<Duration>,
    now: SystemTime,
) -> Result<Inspect, Error> {
    let Some(name) = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .filter(|n| !n.is_empty())
    else {
        return Ok(Inspect::Outside);
    };
    match path.parent() {
        Some(parent) if parent == folder => {}
        _ => return Ok(Inspect::Outside),
    }

    let meta = match fs::symlink_metadata(path) {
        Ok(meta) => meta,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(Inspect::Gone),
        Err(err) => {
            return Err(Error::io(format!("reading {}", path.display()), err));
        }
    };

    if meta.file_type().is_symlink() {
        return Ok(Inspect::Skip(skipped(path, name, SkipReason::Symlink)));
    }
    if meta.is_dir() {
        return Ok(Inspect::Skip(skipped(path, name, SkipReason::Directory)));
    }
    if !meta.is_file() {
        return Ok(Inspect::Skip(skipped(path, name, SkipReason::NotRegular)));
    }
    if is_hidden_name(&name) {
        return Ok(Inspect::Skip(skipped(path, name, SkipReason::Hidden)));
    }
    if is_partial_name(&name) {
        return Ok(Inspect::Skip(skipped(
            path,
            name,
            SkipReason::PartialDownload,
        )));
    }

    let modified = meta.modified().map_err(|err| {
        Error::io(
            format!("reading the modified time of {}", path.display()),
            err,
        )
    })?;
    if let Some(min_age) = older_than {
        if !is_older_than(modified, now, min_age) {
            return Ok(Inspect::Skip(skipped(
                path,
                name,
                SkipReason::NewerThanFilter,
            )));
        }
    }

    Ok(Inspect::Candidate(FileEntry {
        path: path.to_path_buf(),
        name,
        size: meta.len(),
        modified,
    }))
}

fn skipped(path: &Path, name: String, reason: SkipReason) -> Skipped {
    Skipped {
        path: path.to_path_buf(),
        name,
        reason,
    }
}

/// Top-level entries of `folder`. The folder must already be a directory.
pub fn scan(
    folder: &Path,
    older_than: Option<Duration>,
    now: SystemTime,
) -> Result<(Vec<FileEntry>, Vec<Skipped>), Error> {
    let reader = fs::read_dir(folder)
        .map_err(|err| Error::io(format!("reading {}", folder.display()), err))?;
    let mut entries = Vec::new();
    let mut skipped_entries = Vec::new();
    for item in reader {
        let item = item.map_err(|err| Error::io(format!("reading {}", folder.display()), err))?;
        match inspect(folder, &item.path(), older_than, now)? {
            Inspect::Candidate(file) => entries.push(file),
            Inspect::Skip(skip) => skipped_entries.push(skip),
            Inspect::Outside | Inspect::Gone => {}
        }
    }
    entries.sort_by(|a, b| cmp_name(&a.name, &b.name));
    skipped_entries.sort_by(|a, b| cmp_name(&a.name, &b.name));
    Ok((entries, skipped_entries))
}

fn cmp_name(a: &str, b: &str) -> std::cmp::Ordering {
    a.to_lowercase().cmp(&b.to_lowercase()).then(a.cmp(b))
}

/// Resolve the folder a command should operate on.
///
/// A symlink to a directory is followed (Downloads is sometimes a link).
/// The returned path is absolute.
pub fn resolve_folder(path: &Path) -> Result<PathBuf, Error> {
    if !path.exists() {
        return Err(Error::NotFound {
            path: path.to_path_buf(),
        });
    }
    if !path.is_dir() {
        return Err(Error::NotADirectory {
            path: path.to_path_buf(),
        });
    }
    fs::canonicalize(path).map_err(|err| Error::io(format!("resolving {}", path.display()), err))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skips_hidden_partial_dirs_symlinks_and_nested_files() {
        let tmp = tempfile::tempdir().unwrap();
        let folder = tmp.path().join("Downloads");
        fs::create_dir_all(folder.join("nested")).unwrap();
        fs::write(folder.join("keep.pdf"), b"pdf").unwrap();
        fs::write(folder.join(".secret.pdf"), b"hide").unwrap();
        fs::write(folder.join("movie.mp4.crdownload"), b"part").unwrap();
        fs::write(folder.join("other.part"), b"part").unwrap();
        fs::write(folder.join("file.download"), b"part").unwrap();
        fs::write(folder.join("file.tmp"), b"part").unwrap();
        fs::write(folder.join("file.partial"), b"part").unwrap();
        fs::write(folder.join("nested/inner.pdf"), b"nope").unwrap();
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(folder.join("keep.pdf"), folder.join("link.pdf")).unwrap();
        }

        let now = SystemTime::now();
        let (entries, skipped) = scan(&folder, None, now).unwrap();
        let names: Vec<_> = entries.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, vec!["keep.pdf"]);

        let mut reasons: Vec<_> = skipped
            .iter()
            .map(|s| (s.name.as_str(), s.reason))
            .collect();
        reasons.sort_by(|a, b| a.0.cmp(b.0));
        assert!(reasons.contains(&(".secret.pdf", SkipReason::Hidden)));
        assert!(reasons.contains(&("movie.mp4.crdownload", SkipReason::PartialDownload)));
        assert!(reasons.contains(&("other.part", SkipReason::PartialDownload)));
        assert!(reasons.contains(&("file.download", SkipReason::PartialDownload)));
        assert!(reasons.contains(&("file.tmp", SkipReason::PartialDownload)));
        assert!(reasons.contains(&("file.partial", SkipReason::PartialDownload)));
        assert!(reasons.contains(&("nested", SkipReason::Directory)));
        #[cfg(unix)]
        assert!(reasons.contains(&("link.pdf", SkipReason::Symlink)));

        assert!(folder.join("nested/inner.pdf").is_file());
    }

    #[test]
    fn global_age_filter_skips_newer_files() {
        let tmp = tempfile::tempdir().unwrap();
        let folder = tmp.path().join("Downloads");
        fs::create_dir_all(&folder).unwrap();
        let old = folder.join("old.txt");
        let young = folder.join("new.txt");
        fs::write(&old, b"old").unwrap();
        fs::write(&young, b"new").unwrap();
        let now = SystemTime::now();
        filetime_backdate(&old, now - Duration::from_secs(10 * 86400));

        let (entries, skipped) = scan(&folder, Some(Duration::from_secs(7 * 86400)), now).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].name, "old.txt");
        assert!(skipped
            .iter()
            .any(|s| s.name == "new.txt" && s.reason == SkipReason::NewerThanFilter));
    }

    fn filetime_backdate(path: &Path, when: SystemTime) {
        let file = fs::File::options().write(true).open(path).unwrap();
        file.set_modified(when).unwrap();
    }

    #[test]
    fn missing_and_non_directory_paths_fail() {
        let tmp = tempfile::tempdir().unwrap();
        let missing = tmp.path().join("nope");
        let err = resolve_folder(&missing).unwrap_err();
        assert!(matches!(err, Error::NotFound { .. }));

        let file = tmp.path().join("file");
        fs::write(&file, b"x").unwrap();
        let err = resolve_folder(&file).unwrap_err();
        assert!(matches!(err, Error::NotADirectory { .. }));
    }
}
