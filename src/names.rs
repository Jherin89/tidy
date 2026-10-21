//! File-name helpers: extensions, partial-download detection, collision suffixes.

/// Compound suffixes that should stay intact when a collision suffix is inserted.
const COMPOUND_EXTS: &[&str] = &["tar.gz", "tar.bz2", "tar.xz", "tar.zst"];

const PARTIAL_EXTS: &[&str] = &["crdownload", "part", "download", "tmp", "partial"];

/// Split `name` into `(stem, extension)`.
///
/// The extension does not include the dot. `archive.tar.gz` yields
/// `("archive", "tar.gz")`. A leading-dot name such as `.bashrc` has no
/// extension. The stem keeps the original casing; a compound extension is
/// returned in lowercase.
pub fn split_file_name(name: &str) -> (String, String) {
    let lower = name.to_ascii_lowercase();
    for compound in COMPOUND_EXTS {
        let suffix = format!(".{compound}");
        if let Some(stem) = lower.strip_suffix(&suffix) {
            if !stem.is_empty() {
                let stem_orig = &name[..name.len() - suffix.len()];
                return (stem_orig.to_string(), (*compound).to_string());
            }
        }
    }
    if let Some((stem, ext)) = name.rsplit_once('.') {
        if !stem.is_empty() && !ext.is_empty() {
            return (stem.to_string(), ext.to_string());
        }
    }
    (name.to_string(), String::new())
}

/// Extension after the final dot, lowercased, if the name has one.
pub fn last_extension(name: &str) -> Option<String> {
    let (stem, ext) = name.rsplit_once('.')?;
    if stem.is_empty() || ext.is_empty() {
        return None;
    }
    Some(ext.to_ascii_lowercase())
}

pub fn is_hidden_name(name: &str) -> bool {
    name.starts_with('.')
}

pub fn is_partial_name(name: &str) -> bool {
    match last_extension(name) {
        Some(ext) => PARTIAL_EXTS.contains(&ext.as_str()),
        None => false,
    }
}

/// `notes.txt` + 1 → `notes (1).txt`. `archive.tar.gz` + 2 → `archive (2).tar.gz`.
pub fn collision_name(name: &str, n: u32) -> String {
    let (stem, ext) = split_file_name(name);
    if ext.is_empty() {
        format!("{stem} ({n})")
    } else {
        format!("{stem} ({n}).{ext}")
    }
}

/// True when `name` matches any configured extension.
///
/// Plain extensions compare against the text after the last dot.
/// An extension that itself contains a dot (for example `tar.gz`) matches
/// when the whole file name ends with it.
pub fn extension_matches(name: &str, extensions: &[String]) -> bool {
    let lower_name = name.to_ascii_lowercase();
    let last = last_extension(name);
    extensions.iter().any(|ext| {
        if ext.contains('.') {
            lower_name.ends_with(&format!(".{ext}"))
        } else {
            last.as_deref() == Some(ext.as_str())
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_simple_and_compound_names() {
        assert_eq!(
            split_file_name("report.pdf"),
            ("report".to_string(), "pdf".to_string())
        );
        assert_eq!(
            split_file_name("archive.tar.gz"),
            ("archive".to_string(), "tar.gz".to_string())
        );
        assert_eq!(
            split_file_name("photo.JPG"),
            ("photo".to_string(), "JPG".to_string())
        );
        assert_eq!(
            split_file_name("README"),
            ("README".to_string(), String::new())
        );
        assert_eq!(
            split_file_name(".bashrc"),
            (".bashrc".to_string(), String::new())
        );
    }

    #[test]
    fn collision_suffix_lands_before_the_extension() {
        assert_eq!(collision_name("report.pdf", 1), "report (1).pdf");
        assert_eq!(collision_name("archive.tar.gz", 2), "archive (2).tar.gz");
        assert_eq!(collision_name("photo.JPG", 1), "photo (1).JPG");
        assert_eq!(collision_name("README", 3), "README (3)");
        assert_eq!(collision_name("my.file.pdf", 1), "my.file (1).pdf");
    }

    #[test]
    fn hidden_and_partial_detection() {
        assert!(is_hidden_name(".secret.pdf"));
        assert!(!is_hidden_name("secret.pdf"));
        for ext in ["crdownload", "part", "download", "tmp", "partial"] {
            assert!(is_partial_name(&format!("video.mp4.{ext}")), "{ext}");
            assert!(is_partial_name(&format!(
                "video.mp4.{ext_upper}",
                ext_upper = ext.to_ascii_uppercase()
            )));
        }
        assert!(!is_partial_name("video.mp4"));
        assert!(!is_partial_name(".crdownload"));
    }

    #[test]
    fn extension_match_is_case_insensitive_and_supports_compounds() {
        let exts = vec![
            "pdf".to_string(),
            "tar.gz".to_string(),
            "appimage".to_string(),
        ];
        assert!(extension_matches("Report.PDF", &exts));
        assert!(extension_matches("files.tar.gz", &exts));
        assert!(extension_matches("Tool.AppImage", &exts));
        assert!(!extension_matches("notes.txt", &exts));
        assert!(!extension_matches("notgz.txt", &exts));
    }
}
