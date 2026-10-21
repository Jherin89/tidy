//! Move a file without ever overwriting an existing one.
//!
//! Same-filesystem moves use [`std::fs::rename`]. A cross-device move copies
//! the bytes, checks the length, and only then removes the source. That
//! source removal is the only time tidy deletes a file the user already had.
//! A failed move may also remove a placeholder file this function just
//! created, so a half-finished copy is not left behind under the destination
//! name.

use std::fs::{self, OpenOptions};
use std::io::ErrorKind;
use std::path::Path;

use crate::error::Error;
use crate::plan::occupied;

/// Move `src` onto `dst`.
///
/// `dst` must not already exist. The destination's parent directories are
/// created. On success, `src` is gone and `dst` has the same length.
pub fn relocate(src: &Path, dst: &Path) -> Result<(), Error> {
    if occupied(dst) {
        return Err(Error::msg(format!(
            "refusing to overwrite {}",
            dst.display()
        )));
    }
    if let Some(parent) = dst.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent)
                .map_err(|err| Error::io(format!("creating {}", parent.display()), err))?;
        }
    }

    // Reserve the name before rename. Unix rename replaces an existing file,
    // so a placeholder we created is the only thing rename is allowed to
    // replace. create_new fails if a user file won the race.
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(dst)
        .map_err(|err| {
            if err.kind() == ErrorKind::AlreadyExists {
                Error::msg(format!("refusing to overwrite {}", dst.display()))
            } else {
                Error::io(format!("reserving {}", dst.display()), err)
            }
        })?;

    match fs::rename(src, dst) {
        Ok(()) => Ok(()),
        Err(err) if is_cross_device(&err) || err.kind() == ErrorKind::AlreadyExists => {
            copy_verify_remove_source(src, dst).inspect_err(|_| {
                // The placeholder (or a partial copy) was created by this
                // process. Drop it so the destination name is not left
                // holding a truncated file. The source is still in place
                // unless the verified-copy path removed it.
                let _ = fs::remove_file(dst);
            })
        }
        Err(err) => {
            let _ = fs::remove_file(dst);
            Err(Error::io(
                format!("renaming {} to {}", src.display(), dst.display()),
                err,
            ))
        }
    }
}

fn is_cross_device(err: &std::io::Error) -> bool {
    err.kind() == ErrorKind::CrossesDevices || err.raw_os_error() == Some(18)
}

/// Copy `src` onto an existing placeholder at `dst`, check the length, then
/// remove `src`. If the lengths differ, `src` is left untouched.
fn copy_verify_remove_source(src: &Path, dst: &Path) -> Result<(), Error> {
    let src_len = fs::metadata(src)
        .map_err(|err| Error::io(format!("reading {}", src.display()), err))?
        .len();
    let copied = fs::copy(src, dst).map_err(|err| {
        Error::io(
            format!("copying {} to {}", src.display(), dst.display()),
            err,
        )
    })?;
    let dst_len = fs::metadata(dst)
        .map_err(|err| Error::io(format!("reading {}", dst.display()), err))?
        .len();
    if copied != src_len || dst_len != src_len {
        return Err(Error::msg(format!(
            "copy verification failed for {} (source {src_len} bytes, destination {dst_len} bytes); the source was left in place",
            src.display()
        )));
    }
    fs::remove_file(src).map_err(|err| {
        Error::io(
            format!(
                "removing source {} after a verified copy to {}",
                src.display(),
                dst.display()
            ),
            err,
        )
    })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn rename_moves_bytes_and_does_not_overwrite() {
        let tmp = tempfile::tempdir().unwrap();
        let src = tmp.path().join("src.txt");
        let dst = tmp.path().join("nested/dst.txt");
        fs::write(&src, b"hello").unwrap();
        relocate(&src, &dst).unwrap();
        assert!(!src.exists());
        assert_eq!(fs::read(&dst).unwrap(), b"hello");

        let keeper = tmp.path().join("keeper.txt");
        fs::write(&keeper, b"keep").unwrap();
        let again = tmp.path().join("again.txt");
        fs::write(&again, b"new").unwrap();
        let err = relocate(&again, &keeper).unwrap_err();
        assert!(err.to_string().contains("refusing to overwrite"));
        assert_eq!(fs::read(&keeper).unwrap(), b"keep");
        assert_eq!(fs::read(&again).unwrap(), b"new");
    }

    #[test]
    fn verified_copy_removes_the_source_only_after_the_bytes_match() {
        let tmp = tempfile::tempdir().unwrap();
        let src = tmp.path().join("src.bin");
        let dst = tmp.path().join("dst.bin");
        let payload = vec![7u8; 4096];
        fs::write(&src, &payload).unwrap();
        fs::write(&dst, b"").unwrap();
        copy_verify_remove_source(&src, &dst).unwrap();
        assert!(!src.exists());
        assert_eq!(fs::read(&dst).unwrap(), payload);
    }

    #[test]
    fn a_missing_source_does_not_leave_the_placeholder_behind() {
        let tmp = tempfile::tempdir().unwrap();
        let src = tmp.path().join("missing.txt");
        let dst = tmp.path().join("dest.txt");
        let err = relocate(&src, &dst).unwrap_err();
        assert!(
            err.to_string().contains("missing")
                || err.to_string().contains("renaming")
                || err.to_string().contains("copying")
                || err.to_string().contains("reading")
        );
        assert!(!dst.exists());
    }
}
