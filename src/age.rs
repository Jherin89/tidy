//! Which timestamp counts as a file's age.
//!
//! `older_than` (the flag and the rule condition) uses this age, not raw
//! mtime. On macOS the preferred stamp is Finder's Date Added
//! (`ATTR_CMN_ADDEDTIME`). Everywhere else, and when Date Added is missing,
//! the age is the most recent of created (birth) and modified. A file that
//! was just copied into Downloads keeps its old mtime, but Date Added /
//! birth is now, so it is not treated as old.
//!
//! [`AgeProbe`] is the seam tests inject. Production uses [`SystemAge`].

use std::fs::Metadata;
use std::path::Path;
#[cfg(any(test, target_os = "macos"))]
use std::time::Duration;
use std::time::SystemTime;

/// Where the age timestamp came from. `tidy explain` prints this.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgeKind {
    DateAdded,
    Created,
    Modified,
}

impl AgeKind {
    pub fn as_str(self) -> &'static str {
        match self {
            AgeKind::DateAdded => "date added",
            AgeKind::Created => "created",
            AgeKind::Modified => "modified",
        }
    }
}

/// The three stamps we might have for one path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FileTimes {
    pub modified: SystemTime,
    pub created: Option<SystemTime>,
    pub added: Option<SystemTime>,
}

/// Pick the stamp `older_than` should use.
///
/// Date Added wins when present. Otherwise the later of created and
/// modified, with a tag that says which one that was.
pub fn resolve_age(times: FileTimes) -> (SystemTime, AgeKind) {
    if let Some(added) = times.added {
        return (added, AgeKind::DateAdded);
    }
    match times.created {
        Some(created) if created >= times.modified => (created, AgeKind::Created),
        Some(_) => (times.modified, AgeKind::Modified),
        None => (times.modified, AgeKind::Modified),
    }
}

/// Reads timestamps for a path. Tests pass a fake.
pub trait AgeProbe {
    fn probe(&self, path: &Path, meta: &Metadata) -> FileTimes;
}

/// Production probe: Date Added on macOS, then birth/mtime from `stat`.
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemAge;

impl AgeProbe for SystemAge {
    fn probe(&self, path: &Path, meta: &Metadata) -> FileTimes {
        FileTimes {
            modified: meta.modified().unwrap_or(SystemTime::UNIX_EPOCH),
            created: meta.created().ok(),
            added: date_added(path),
        }
    }
}

/// Probe that treats mtime as created too. Tests that backdate mtime use
/// this to simulate a file that has lived at that age (both stamps old).
#[derive(Debug, Clone, Copy, Default)]
pub struct ModifiedOnlyAge;

impl AgeProbe for ModifiedOnlyAge {
    fn probe(&self, _path: &Path, meta: &Metadata) -> FileTimes {
        let modified = meta.modified().unwrap_or(SystemTime::UNIX_EPOCH);
        FileTimes {
            modified,
            created: Some(modified),
            added: None,
        }
    }
}

fn date_added(path: &Path) -> Option<SystemTime> {
    #[cfg(target_os = "macos")]
    {
        macos_date_added(path)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = path;
        None
    }
}

/// Finder's Date Added via `getattrlist(ATTR_CMN_ADDEDTIME)`.
///
/// This is the APFS added-time attribute, not a shell-out to `mdls`.
#[cfg(target_os = "macos")]
#[allow(unsafe_code)]
fn macos_date_added(path: &Path) -> Option<SystemTime> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;

    let c_path = CString::new(path.as_os_str().as_bytes()).ok()?;

    let mut attrlist = libc::attrlist {
        bitmapcount: libc::ATTR_BIT_MAP_COUNT,
        reserved: 0,
        commonattr: libc::ATTR_CMN_ADDEDTIME,
        volattr: 0,
        dirattr: 0,
        fileattr: 0,
        forkattr: 0,
    };

    // getattrlist writes a 4-byte length, then the attribute packed to a
    // 4-byte boundary. timespec is 16 bytes on 64-bit Darwin. A 64-byte
    // buffer leaves room for alignment quirks.
    let mut buf = [0u8; 64];

    // SAFETY: `c_path` is a valid NUL-terminated path. `attrlist` is a
    // well-formed `attrlist` for the duration of the call. `buf` is a
    // writable region of `buf.len()` bytes. `FSOPT_NOFOLLOW` matches
    // `inspect`, which uses `symlink_metadata` and never follows links.
    let rc = unsafe {
        libc::getattrlist(
            c_path.as_ptr(),
            std::ptr::addr_of_mut!(attrlist).cast(),
            buf.as_mut_ptr().cast(),
            buf.len(),
            libc::FSOPT_NOFOLLOW,
        )
    };
    if rc != 0 {
        return None;
    }

    timespec_from_attr_buf(&buf)
}

/// `getattrlist` writes a 4-byte length, then attributes packed to a 4-byte
/// boundary (`man getattrlist`). A C `struct { u32; timespec; }` inserts
/// 4 bytes of padding on 64-bit, so timespec might sit at offset 8 instead.
/// Try both. Invalid nsec (padding mixed into the field) makes that offset fail.
#[cfg(target_os = "macos")]
fn timespec_from_attr_buf(buf: &[u8]) -> Option<SystemTime> {
    let length = u32::from_ne_bytes(buf.get(0..4)?.try_into().ok()?);
    if (length as usize) < 20 || (length as usize) > buf.len() {
        return None;
    }
    parse_timespec(buf, 4).or_else(|| parse_timespec(buf, 8))
}

#[cfg(target_os = "macos")]
fn parse_timespec(buf: &[u8], offset: usize) -> Option<SystemTime> {
    let sec = i64::from_ne_bytes(buf.get(offset..offset + 8)?.try_into().ok()?);
    let nsec = i64::from_ne_bytes(buf.get(offset + 8..offset + 16)?.try_into().ok()?);
    if sec < 0 || !(0..1_000_000_000).contains(&nsec) {
        return None;
    }
    SystemTime::UNIX_EPOCH.checked_add(Duration::new(sec as u64, nsec as u32))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(secs: u64) -> SystemTime {
        SystemTime::UNIX_EPOCH + Duration::from_secs(secs)
    }

    #[test]
    fn date_added_wins_even_when_mtime_is_older() {
        let (age, kind) = resolve_age(FileTimes {
            modified: at(10),
            created: Some(at(20)),
            added: Some(at(50)),
        });
        assert_eq!(age, at(50));
        assert_eq!(kind, AgeKind::DateAdded);
    }

    #[test]
    fn fallback_picks_the_later_of_created_and_modified() {
        let (age, kind) = resolve_age(FileTimes {
            modified: at(10),
            created: Some(at(40)),
            added: None,
        });
        assert_eq!(age, at(40));
        assert_eq!(kind, AgeKind::Created);

        let (age, kind) = resolve_age(FileTimes {
            modified: at(80),
            created: Some(at(40)),
            added: None,
        });
        assert_eq!(age, at(80));
        assert_eq!(kind, AgeKind::Modified);
    }

    #[test]
    fn missing_created_uses_modified() {
        let (age, kind) = resolve_age(FileTimes {
            modified: at(15),
            created: None,
            added: None,
        });
        assert_eq!(age, at(15));
        assert_eq!(kind, AgeKind::Modified);
    }

    #[test]
    fn equal_created_and_modified_is_tagged_created() {
        let (age, kind) = resolve_age(FileTimes {
            modified: at(7),
            created: Some(at(7)),
            added: None,
        });
        assert_eq!(age, at(7));
        assert_eq!(kind, AgeKind::Created);
    }

    #[test]
    fn kind_labels_match_explain_output() {
        assert_eq!(AgeKind::DateAdded.as_str(), "date added");
        assert_eq!(AgeKind::Created.as_str(), "created");
        assert_eq!(AgeKind::Modified.as_str(), "modified");
    }
}
