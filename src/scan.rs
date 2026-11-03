//! Discover the top-level files a run is allowed to consider.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use crate::age::{resolve_age, AgeKind, AgeProbe, SystemAge};
use crate::error::Error;
use crate::names::{extension_matches, is_hidden_name, is_partial_name, last_extension};
use crate::privacy;
use crate::rules::is_older_than;

/// Document-package extensions treated as one item. Case-insensitive.
/// Custom rules cannot add to this list: a folder named `Ubuntu.dmg` stays a folder.
pub const PACKAGE_EXTENSIONS: &[&str] = &["pages", "numbers", "key", "rtfd"];

/// A regular file, or a package directory treated as one item.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileEntry {
    pub path: PathBuf,
    pub name: String,
    pub size: u64,
    pub modified: SystemTime,
    /// Timestamp `older_than` uses. See [`AgeKind`].
    pub age: SystemTime,
    pub age_kind: AgeKind,
    /// `.pages` / `.numbers` / `.key` / `.rtfd` directory, moved/trashed whole.
    pub is_bundle: bool,
}

impl FileEntry {
    /// Test helper. Age is `modified` tagged as modified, not a bundle.
    pub fn new(path: PathBuf, name: impl Into<String>, size: u64, modified: SystemTime) -> Self {
        Self {
            path,
            name: name.into(),
            size,
            modified,
            age: modified,
            age_kind: AgeKind::Modified,
            is_bundle: false,
        }
    }

    /// Name as shown in tables: packages get a trailing `/`.
    pub fn display_name(&self) -> String {
        if self.is_bundle {
            format!("{}/", self.name)
        } else {
            self.name.clone()
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SkipReason {
    Hidden,
    PartialDownload,
    Directory,
    /// A directory whose name looks like a file (`Ubuntu.dmg`, `Vacation.png`).
    FolderNotAFile,
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
            SkipReason::FolderNotAFile => "folder, not a file",
            SkipReason::Symlink => "symlink",
            SkipReason::NotRegular => "not a regular file",
            SkipReason::NewerThanFilter => "newer than --older-than",
        }
    }

    pub fn is_directory(self) -> bool {
        matches!(self, SkipReason::Directory | SkipReason::FolderNotAFile)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Skipped {
    pub path: PathBuf,
    pub name: String,
    pub reason: SkipReason,
}

impl Skipped {
    /// Name as shown in skip lists: directories get a trailing `/`.
    pub fn display_name(&self) -> String {
        if self.reason.is_directory() {
            format!("{}/", self.name)
        } else {
            self.name.clone()
        }
    }
}

#[derive(Debug)]
pub enum Inspect {
    Candidate(FileEntry),
    Skip(Skipped),
    /// Not a direct child of the folder being tidied.
    Outside,
    Gone,
}

/// Classify one path. Only a regular file (or a document package:
/// `.pages`, `.numbers`, `.key`, `.rtfd`) that is a direct child of
/// `folder` can become a candidate. Symlinks are never followed.
pub fn inspect(
    folder: &Path,
    path: &Path,
    older_than: Option<Duration>,
    now: SystemTime,
) -> Result<Inspect, Error> {
    inspect_with(folder, path, older_than, now, &[], &SystemAge)
}

pub fn inspect_with(
    folder: &Path,
    path: &Path,
    older_than: Option<Duration>,
    now: SystemTime,
    bundle_exts: &[String],
    ages: &dyn AgeProbe,
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
            return Err(privacy::annotate(
                folder,
                format!("reading {}", path.display()),
                err,
            ));
        }
    };

    if meta.file_type().is_symlink() {
        return Ok(Inspect::Skip(skipped(path, name, SkipReason::Symlink)));
    }

    let is_bundle = meta.is_dir() && is_bundle_name(&name, bundle_exts);
    if meta.is_dir() && !is_bundle {
        let reason = if last_extension(&name).is_some() {
            SkipReason::FolderNotAFile
        } else {
            SkipReason::Directory
        };
        return Ok(Inspect::Skip(skipped(path, name, reason)));
    }
    if !is_bundle && !meta.is_file() {
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
        privacy::annotate(
            folder,
            format!("reading the modified time of {}", path.display()),
            err,
        )
    })?;
    let times = ages.probe(path, &meta);
    let (age, age_kind) = resolve_age(times);
    if let Some(min_age) = older_than {
        if !is_older_than(age, now, min_age) {
            return Ok(Inspect::Skip(skipped(
                path,
                name,
                SkipReason::NewerThanFilter,
            )));
        }
    }

    let size = if is_bundle {
        directory_size(folder, path)?
    } else {
        meta.len()
    };

    Ok(Inspect::Candidate(FileEntry {
        path: path.to_path_buf(),
        name,
        size,
        modified,
        age,
        age_kind,
        is_bundle,
    }))
}

/// The four document-package extensions, as owned strings.
pub fn package_extension_list() -> Vec<String> {
    PACKAGE_EXTENSIONS
        .iter()
        .map(|s| (*s).to_string())
        .collect()
}

/// True when `name` is a `.pages` / `.numbers` / `.key` / `.rtfd` package.
///
/// Case-insensitive. Other directories are never bundles: a folder named
/// `Ubuntu.dmg` is skipped even if a rule lists `dmg`. `bundle_exts` may
/// further restrict the set (tests); it cannot add types.
pub fn is_bundle_name(name: &str, bundle_exts: &[String]) -> bool {
    if !is_package_name(name) {
        return false;
    }
    bundle_exts.is_empty() || extension_matches(name, bundle_exts)
}

/// True when `name`'s last extension is one of [`PACKAGE_EXTENSIONS`].
pub fn is_package_name(name: &str) -> bool {
    last_extension(name).is_some_and(|ext| PACKAGE_EXTENSIONS.iter().any(|pkg| ext == *pkg))
}

/// Sum of regular-file lengths under `path`. Symlinks are not followed.
fn directory_size(folder: &Path, path: &Path) -> Result<u64, Error> {
    let mut total = 0u64;
    let mut stack = vec![path.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let reader = fs::read_dir(&dir)
            .map_err(|err| privacy::annotate(folder, format!("reading {}", dir.display()), err))?;
        for item in reader {
            let item = item.map_err(|err| {
                privacy::annotate(folder, format!("reading {}", dir.display()), err)
            })?;
            let child = item.path();
            let meta = fs::symlink_metadata(&child).map_err(|err| {
                privacy::annotate(folder, format!("reading {}", child.display()), err)
            })?;
            if meta.is_dir() {
                stack.push(child);
            } else if meta.is_file() {
                total = total.saturating_add(meta.len());
            }
        }
    }
    Ok(total)
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
    scan_with(folder, older_than, now, &[], &SystemAge)
}

pub fn scan_with(
    folder: &Path,
    older_than: Option<Duration>,
    now: SystemTime,
    bundle_exts: &[String],
    ages: &dyn AgeProbe,
) -> Result<(Vec<FileEntry>, Vec<Skipped>), Error> {
    let reader = fs::read_dir(folder)
        .map_err(|err| privacy::annotate(folder, format!("reading {}", folder.display()), err))?;
    let mut entries = Vec::new();
    let mut skipped_entries = Vec::new();
    for item in reader {
        let item = item.map_err(|err| {
            privacy::annotate(folder, format!("reading {}", folder.display()), err)
        })?;
        match inspect_with(folder, &item.path(), older_than, now, bundle_exts, ages)? {
            Inspect::Candidate(file) => entries.push(file),
            Inspect::Skip(skip) => skipped_entries.push(skip),
            Inspect::Outside | Inspect::Gone => {}
        }
    }
    entries.sort_by(|a, b| compare_names(&a.name, &b.name));
    skipped_entries.sort_by(|a, b| compare_names(&a.name, &b.name));
    Ok((entries, skipped_entries))
}

pub(crate) fn compare_names(a: &str, b: &str) -> std::cmp::Ordering {
    a.to_lowercase().cmp(&b.to_lowercase()).then(a.cmp(b))
}

/// Resolve the folder a command should operate on.
///
/// A symlink to a directory is followed (Downloads is sometimes a link).
/// The returned path is absolute.
pub fn resolve_folder(path: &Path) -> Result<PathBuf, Error> {
    // `Path::exists` hides EPERM, which is how macOS Privacy & Security
    // refuses `~/Downloads`. `metadata` surfaces that error.
    match fs::metadata(path) {
        Ok(meta) if meta.is_dir() => {}
        Ok(_) => {
            return Err(Error::NotADirectory {
                path: path.to_path_buf(),
            });
        }
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            return Err(Error::NotFound {
                path: path.to_path_buf(),
            });
        }
        Err(err) => {
            return Err(privacy::annotate(
                path,
                format!("reading {}", path.display()),
                err,
            ));
        }
    }
    fs::canonicalize(path).map_err(|err| {
        if err.kind() == std::io::ErrorKind::NotFound {
            Error::NotFound {
                path: path.to_path_buf(),
            }
        } else {
            privacy::annotate(path, format!("resolving {}", path.display()), err)
        }
    })
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

        // ModifiedOnlyAge makes a backdated mtime count as the file's age, so
        // this still tests the filter itself. Production uses birth/Date Added.
        let (entries, skipped) = scan_with(
            &folder,
            Some(Duration::from_secs(7 * 86400)),
            now,
            &[],
            &crate::age::ModifiedOnlyAge,
        )
        .unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].name, "old.txt");
        assert!(skipped
            .iter()
            .any(|s| s.name == "new.txt" && s.reason == SkipReason::NewerThanFilter));
    }

    struct RecentBirth;

    impl AgeProbe for RecentBirth {
        fn probe(&self, _path: &Path, meta: &fs::Metadata) -> crate::age::FileTimes {
            crate::age::FileTimes {
                modified: meta.modified().unwrap_or(SystemTime::UNIX_EPOCH),
                created: Some(SystemTime::now()),
                added: None,
            }
        }
    }

    #[test]
    fn old_mtime_with_recent_birth_is_not_old() {
        let tmp = tempfile::tempdir().unwrap();
        let folder = tmp.path().join("Downloads");
        fs::create_dir_all(&folder).unwrap();
        let dmg = folder.join("Installer.dmg");
        fs::write(&dmg, b"dmg").unwrap();
        let now = SystemTime::now();
        filetime_backdate(&dmg, now - Duration::from_secs(40 * 86400));

        let (entries, skipped) = scan_with(
            &folder,
            Some(Duration::from_secs(7 * 86400)),
            now,
            &[],
            &RecentBirth,
        )
        .unwrap();
        assert!(
            entries.is_empty(),
            "a just-copied installer with old mtime must not match older_than"
        );
        assert!(skipped
            .iter()
            .any(|s| s.name == "Installer.dmg" && s.reason == SkipReason::NewerThanFilter));
        let inspected = inspect_with(&folder, &dmg, None, now, &[], &RecentBirth).unwrap();
        let Inspect::Candidate(file) = inspected else {
            panic!("expected a candidate without the age filter, got {inspected:?}");
        };
        assert_eq!(file.age_kind, AgeKind::Created);
        assert!(now.duration_since(file.age).unwrap_or(Duration::ZERO) < Duration::from_secs(60));
    }

    #[test]
    fn listed_bundle_is_one_item_and_unlisted_dir_is_skipped() {
        let tmp = tempfile::tempdir().unwrap();
        let folder = tmp.path().join("Downloads");
        fs::create_dir_all(&folder).unwrap();
        let pages = folder.join("Report.pages");
        fs::create_dir_all(pages.join("Contents")).unwrap();
        fs::write(pages.join("index.xml"), b"<pages/>").unwrap();
        fs::write(pages.join("Contents/Data"), b"hello").unwrap();
        fs::create_dir_all(folder.join("nested")).unwrap();
        fs::write(folder.join("nested/inner.pdf"), b"nope").unwrap();
        fs::create_dir_all(folder.join("Some.app")).unwrap();
        fs::write(folder.join("Some.app/Info.plist"), b"app").unwrap();

        let bundles = vec![
            "pages".to_string(),
            "numbers".to_string(),
            "key".to_string(),
        ];
        let now = SystemTime::now();
        let (entries, skipped) = scan_with(&folder, None, now, &bundles, &SystemAge).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].name, "Report.pages");
        assert!(entries[0].is_bundle);
        assert_eq!(
            entries[0].size,
            b"<pages/>".len() as u64 + b"hello".len() as u64
        );
        assert!(skipped
            .iter()
            .any(|s| s.name == "nested" && s.reason == SkipReason::Directory));
        assert!(skipped
            .iter()
            .any(|s| s.name == "Some.app" && s.reason == SkipReason::FolderNotAFile));
        assert!(folder.join("nested/inner.pdf").is_file());
        assert!(pages.join("Contents/Data").is_file());
    }

    #[test]
    fn folder_named_like_a_file_is_skipped_and_pages_is_still_a_package() {
        let tmp = tempfile::tempdir().unwrap();
        let folder = tmp.path().join("Downloads");
        fs::create_dir_all(&folder).unwrap();
        let dmg = folder.join("Ubuntu.dmg");
        fs::create_dir_all(dmg.join("payload")).unwrap();
        fs::write(dmg.join("payload/disk"), b"iso").unwrap();
        filetime_backdate(
            &dmg.join("payload/disk"),
            SystemTime::now() - Duration::from_secs(40 * 86400),
        );
        fs::create_dir_all(folder.join("Vacation.png")).unwrap();
        let pages = folder.join("Report.PAGES");
        fs::create_dir_all(pages.join("Contents")).unwrap();
        fs::write(pages.join("index.xml"), b"<pages/>").unwrap();

        let now = SystemTime::now();
        let (entries, skipped) =
            scan_with(&folder, None, now, &package_extension_list(), &SystemAge).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].name, "Report.PAGES");
        assert!(entries[0].is_bundle);
        assert_eq!(entries[0].display_name(), "Report.PAGES/");
        assert!(skipped
            .iter()
            .any(|s| { s.name == "Ubuntu.dmg" && s.reason == SkipReason::FolderNotAFile }));
        assert!(skipped
            .iter()
            .any(|s| { s.name == "Vacation.png" && s.reason == SkipReason::FolderNotAFile }));
        assert!(is_bundle_name("Notes.pages", &[]));
        assert!(is_bundle_name("Budget.NUMBERS", &package_extension_list()));
        assert!(!is_bundle_name(
            "Ubuntu.dmg",
            &["dmg".into(), "pages".into()]
        ));
        assert!(!is_bundle_name("Vacation.png", &["png".into()]));
        assert!(dmg.is_dir());
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
