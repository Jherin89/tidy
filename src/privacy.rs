//! macOS Privacy & Security (TCC) failures.
//!
//! Reading `~/Downloads` from a terminal or a launchd agent often fails with
//! `EPERM` / "Operation not permitted" until the user grants Full Disk Access
//! or Files and Folders. The wording lives in a pure function so tests can
//! lock it without touching a real folder.

use std::io::Error;
use std::path::Path;

use crate::error::Error as TidyError;
use crate::launchd;

/// What the message is allowed to mention. Callers fill this in; the function
/// does not read the process environment or the filesystem.
pub struct PrivacyContext<'a> {
    pub folder: &'a Path,
    /// Resolved tidy binary, after Homebrew Cellar normalization.
    pub binary: Option<&'a Path>,
}

/// `true` when `err` is Unix `EPERM` (os error 1) or its text says
/// "Operation not permitted". Other permission errors stay ordinary.
pub fn is_operation_not_permitted(err: &Error) -> bool {
    err.raw_os_error() == Some(1)
        || err
            .to_string()
            .to_ascii_lowercase()
            .contains("operation not permitted")
}

/// `Some` when tidy should replace a bare I/O error with the privacy hint.
pub fn privacy_message(err: &Error, ctx: &PrivacyContext<'_>) -> Option<String> {
    if !is_operation_not_permitted(err) {
        return None;
    }
    let binary = ctx
        .binary
        .map(|path| path.display().to_string())
        .unwrap_or_else(|| "the tidy binary".to_string());
    Some(format!(
        "macOS Privacy & Security is blocking access to {folder}. Grant Full Disk Access (or Files and Folders → Downloads) to the terminal app, or to the tidy binary at {binary} when running from launchd.",
        folder = ctx.folder.display(),
    ))
}

/// Attach the privacy hint when `source` is a TCC-style failure.
pub(crate) fn annotate(folder: &Path, context: impl Into<String>, source: Error) -> TidyError {
    let context = context.into();
    let binary = launchd::resolved_tidy_binary().ok();
    if let Some(hint) = privacy_message(
        &source,
        &PrivacyContext {
            folder,
            binary: binary.as_deref(),
        },
    ) {
        TidyError::msg(format!("{context}: {source}\n{hint}"))
    } else {
        TidyError::io(context, source)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::ErrorKind;

    #[test]
    fn eperm_explains_full_disk_access_and_names_the_binary() {
        let err = Error::from_raw_os_error(1);
        let folder = Path::new("/Users/me/Downloads");
        let binary = Path::new("/opt/homebrew/bin/tidy");
        let msg = privacy_message(
            &err,
            &PrivacyContext {
                folder,
                binary: Some(binary),
            },
        )
        .unwrap();
        assert!(msg.contains("macOS Privacy & Security"), "{msg}");
        assert!(msg.contains("Full Disk Access"), "{msg}");
        assert!(msg.contains("Files and Folders"), "{msg}");
        assert!(msg.contains("/Users/me/Downloads"), "{msg}");
        assert!(msg.contains("/opt/homebrew/bin/tidy"), "{msg}");
        assert!(msg.contains("launchd"), "{msg}");
    }

    #[test]
    fn the_phrase_alone_is_enough_and_other_errnos_are_not() {
        let err = Error::new(ErrorKind::PermissionDenied, "Operation not permitted");
        assert!(privacy_message(
            &err,
            &PrivacyContext {
                folder: Path::new("/tmp/box"),
                binary: None,
            },
        )
        .unwrap()
        .contains("the tidy binary"));

        let missing = Error::from_raw_os_error(2);
        assert!(privacy_message(
            &missing,
            &PrivacyContext {
                folder: Path::new("/tmp/box"),
                binary: None,
            },
        )
        .is_none());

        // EACCES is a different failure. Leave it as a normal I/O error.
        let access = Error::from_raw_os_error(13);
        assert!(privacy_message(
            &access,
            &PrivacyContext {
                folder: Path::new("/tmp/box"),
                binary: None,
            },
        )
        .is_none());
    }
}
