//! Byte-identical extra copies among the eligible files of one folder.
//!
//! Only files that share a size are hashed. Zero-byte files are never
//! duplicates. The keeper of each group is the oldest file, then the shortest
//! name, then the lexicographic name. Every other member is an extra copy of
//! that keeper. The keeper itself is not a duplicate.

use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use sha2::{Digest, Sha256};

use crate::error::Error;
use crate::privacy;
use crate::scan::{self, FileEntry};

/// Extra copies in one folder, keyed by the extra file's path.
#[derive(Debug, Clone, Default)]
pub struct DuplicateIndex {
    /// Extra path → keeper file name.
    extras: HashMap<PathBuf, String>,
    /// Paths of files that are the kept member of a group.
    keepers: HashSet<PathBuf>,
}

impl DuplicateIndex {
    pub fn empty() -> Self {
        Self {
            extras: HashMap::new(),
            keepers: HashSet::new(),
        }
    }

    /// Group `files` (the eligible, non-skipped top-level files).
    pub fn build(folder: &Path, files: &[FileEntry]) -> Result<Self, Error> {
        let mut by_size: HashMap<u64, Vec<&FileEntry>> = HashMap::new();
        for file in files {
            if file.size == 0 {
                continue;
            }
            by_size.entry(file.size).or_default().push(file);
        }

        let mut index = Self::empty();
        for group in by_size.into_values() {
            if group.len() < 2 {
                continue;
            }
            let mut by_hash: HashMap<[u8; 32], Vec<&FileEntry>> = HashMap::new();
            for file in group {
                let hash = hash_file(folder, &file.path)?;
                by_hash.entry(hash).or_default().push(file);
            }
            for same in by_hash.into_values() {
                if same.len() < 2 {
                    continue;
                }
                let mut ordered = same;
                ordered.sort_by(|a, b| {
                    a.modified
                        .cmp(&b.modified)
                        .then_with(|| name_len(&a.name).cmp(&name_len(&b.name)))
                        .then_with(|| a.name.cmp(&b.name))
                });
                let keeper = ordered[0];
                index.keepers.insert(keeper.path.clone());
                for extra in &ordered[1..] {
                    index.extras.insert(extra.path.clone(), keeper.name.clone());
                }
            }
        }
        Ok(index)
    }

    /// Keeper file name when `path` is an extra copy.
    pub fn keeper_of(&self, path: &Path) -> Option<&str> {
        self.extras.get(path).map(String::as_str)
    }

    pub fn is_keeper(&self, path: &Path) -> bool {
        self.keepers.contains(path)
    }

    /// Eligible files in `folder`, ignoring `--older-than`, then grouped.
    ///
    /// Watch mode and the age filter both use this so a new file identical to
    /// one that the filter would skip is still an extra copy.
    pub fn for_folder(folder: &Path, now: SystemTime) -> Result<Self, Error> {
        let (files, _) = scan::scan(folder, None, now)?;
        Self::build(folder, &files)
    }
}

fn name_len(name: &str) -> usize {
    name.chars().count()
}

fn hash_file(folder: &Path, path: &Path) -> Result<[u8; 32], Error> {
    let mut file = File::open(path)
        .map_err(|err| privacy::annotate(folder, format!("reading {}", path.display()), err))?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 64 * 1024];
    loop {
        let n = file
            .read(&mut buf)
            .map_err(|err| privacy::annotate(folder, format!("reading {}", path.display()), err))?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    let digest = hasher.finalize();
    let mut out = [0u8; 32];
    out.copy_from_slice(&digest);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::time::Duration;

    fn touch(folder: &Path, name: &str, bytes: &[u8], age: Duration) -> FileEntry {
        let path = folder.join(name);
        fs::write(&path, bytes).unwrap();
        let modified = SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000) - age;
        let file = fs::File::options().write(true).open(&path).unwrap();
        file.set_modified(modified).unwrap();
        FileEntry {
            path,
            name: name.to_string(),
            size: bytes.len() as u64,
            modified,
        }
    }

    fn names_of(index: &DuplicateIndex) -> Vec<(String, String)> {
        let mut pairs: Vec<_> = index
            .extras
            .iter()
            .map(|(path, keeper)| {
                (
                    path.file_name().unwrap().to_string_lossy().into_owned(),
                    keeper.clone(),
                )
            })
            .collect();
        pairs.sort();
        pairs
    }

    #[test]
    fn groups_extras_and_keeps_the_oldest() {
        let tmp = tempfile::tempdir().unwrap();
        let folder = tmp.path();
        let day = Duration::from_secs(86_400);
        let files = vec![
            touch(folder, "invoice (1).pdf", b"same-bytes", day),
            touch(folder, "invoice.pdf", b"same-bytes", 3 * day),
            touch(folder, "invoice (2).pdf", b"same-bytes", 2 * day),
            touch(folder, "notes.txt", b"unique", day),
        ];
        let index = DuplicateIndex::build(folder, &files).unwrap();
        assert_eq!(
            names_of(&index),
            vec![
                ("invoice (1).pdf".into(), "invoice.pdf".into()),
                ("invoice (2).pdf".into(), "invoice.pdf".into()),
            ]
        );
        assert!(index.is_keeper(&folder.join("invoice.pdf")));
        assert!(!index.is_keeper(&folder.join("invoice (1).pdf")));
        assert!(index.keeper_of(&folder.join("notes.txt")).is_none());
    }

    #[test]
    fn tie_breaks_on_shortest_name_then_lexicographic_name() {
        let tmp = tempfile::tempdir().unwrap();
        let folder = tmp.path();
        let age = Duration::from_secs(10);
        let files = vec![
            touch(folder, "bb.txt", b"same", age),
            touch(folder, "c.txt", b"same", age),
            touch(folder, "a.txt", b"same", age),
        ];
        let index = DuplicateIndex::build(folder, &files).unwrap();
        assert!(index.is_keeper(&folder.join("a.txt")));
        assert_eq!(index.keeper_of(&folder.join("c.txt")), Some("a.txt"));
        assert_eq!(index.keeper_of(&folder.join("bb.txt")), Some("a.txt"));
    }

    #[test]
    fn equal_length_names_use_lexicographic_order() {
        let tmp = tempfile::tempdir().unwrap();
        let folder = tmp.path();
        let age = Duration::from_secs(10);
        let files = vec![
            touch(folder, "b.txt", b"same", age),
            touch(folder, "a.txt", b"same", age),
        ];
        let index = DuplicateIndex::build(folder, &files).unwrap();
        assert!(index.is_keeper(&folder.join("a.txt")));
        assert_eq!(index.keeper_of(&folder.join("b.txt")), Some("a.txt"));
    }

    #[test]
    fn zero_byte_files_are_never_duplicates() {
        let tmp = tempfile::tempdir().unwrap();
        let folder = tmp.path();
        let age = Duration::from_secs(10);
        let files = vec![
            touch(folder, "empty-a", b"", age),
            touch(folder, "empty-b", b"", age * 2),
        ];
        let index = DuplicateIndex::build(folder, &files).unwrap();
        assert!(index.extras.is_empty());
        assert!(index.keepers.is_empty());
    }

    #[test]
    fn same_size_different_bytes_are_not_duplicates() {
        let tmp = tempfile::tempdir().unwrap();
        let folder = tmp.path();
        let age = Duration::from_secs(10);
        let files = vec![
            touch(folder, "one.bin", b"aaaa", age),
            touch(folder, "two.bin", b"bbbb", age * 2),
        ];
        let index = DuplicateIndex::build(folder, &files).unwrap();
        assert!(index.extras.is_empty());
    }

    #[test]
    fn a_unique_size_is_not_hashed_into_a_group() {
        let tmp = tempfile::tempdir().unwrap();
        let folder = tmp.path();
        let files = vec![touch(folder, "only.pdf", b"abc", Duration::from_secs(5))];
        let index = DuplicateIndex::build(folder, &files).unwrap();
        assert!(index.keeper_of(&files[0].path).is_none());
        assert!(!index.is_keeper(&files[0].path));
    }
}
