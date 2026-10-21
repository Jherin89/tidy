//! Guards the "never permanently delete a user's file" rule.

use std::fs;
use std::path::Path;

#[test]
fn user_file_removal_lives_only_in_the_relocate_fallback() {
    let hits = scan(|text| text.contains("remove_file") || text.contains("remove_dir_all"));
    assert_eq!(
        hits,
        vec!["relocate.rs".to_string()],
        "remove_file / remove_dir_all should only appear in relocate.rs"
    );
}

#[test]
fn empty_directory_cleanup_lives_only_in_undo() {
    // `fs::remove_dir` only removes empty directories; undo uses it to tidy up
    // folders that `--apply` created. Nowhere else may call it.
    let hits = scan(|text| text.contains("remove_dir("));
    assert_eq!(hits, vec!["undo.rs".to_string()]);
}

fn scan(matches: impl Fn(&str) -> bool) -> Vec<String> {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut hits = Vec::new();
    visit(&src, &src, &matches, &mut hits);
    hits.sort();
    hits
}

fn visit(root: &Path, dir: &Path, matches: &impl Fn(&str) -> bool, hits: &mut Vec<String>) {
    for entry in fs::read_dir(dir).unwrap() {
        let entry = entry.unwrap();
        let path = entry.path();
        if path.is_dir() {
            visit(root, &path, matches, hits);
            continue;
        }
        if path.extension().and_then(|ext| ext.to_str()) != Some("rs") {
            continue;
        }
        let text = fs::read_to_string(&path).unwrap();
        if matches(&text) {
            hits.push(path.strip_prefix(root).unwrap().display().to_string());
        }
    }
}
