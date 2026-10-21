//! Guards the "never permanently delete a user's file" rule.

use std::fs;
use std::path::Path;

#[test]
fn user_file_removal_lives_only_in_the_relocate_fallback() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut hits = Vec::new();
    visit(&src, &src, &mut hits);
    hits.sort();
    assert_eq!(
        hits,
        vec!["relocate.rs".to_string()],
        "remove_file / remove_dir should only appear in relocate.rs"
    );
}

fn visit(root: &Path, dir: &Path, hits: &mut Vec<String>) {
    for entry in fs::read_dir(dir).unwrap() {
        let entry = entry.unwrap();
        let path = entry.path();
        if path.is_dir() {
            visit(root, &path, hits);
            continue;
        }
        if path.extension().and_then(|ext| ext.to_str()) != Some("rs") {
            continue;
        }
        let text = fs::read_to_string(&path).unwrap();
        if text.contains("remove_file") || text.contains("remove_dir") {
            hits.push(path.strip_prefix(root).unwrap().display().to_string());
        }
    }
}
