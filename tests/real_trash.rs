//! CI-only check of the real macOS Trash.
//!
//! Compiled on macOS and returns immediately unless `TIDY_CI_REAL_TRASH=1`.
//! Home, config, and data stay in a temp directory. `TIDY_FAKE_TRASH` is unset
//! so the file goes through the `trash` crate. Do not set that variable locally.

#[cfg(target_os = "macos")]
#[test]
fn ci_real_trash_undo_names_the_file_for_manual_restore() {
    if std::env::var("TIDY_CI_REAL_TRASH").ok().as_deref() != Some("1") {
        return;
    }

    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("home");
    let config = tmp.path().join("config");
    let data = tmp.path().join("data");
    let folder = tmp.path().join("inbox");
    for dir in [&home, &config, &data, &folder] {
        std::fs::create_dir_all(dir).unwrap();
    }

    let name = format!("tidy-ci-trash-{}.txt", std::process::id());
    let file = folder.join(&name);
    std::fs::write(&file, b"real-trash-payload").unwrap();

    let rules = tmp.path().join("rules.toml");
    std::fs::write(
        &rules,
        "[[rules]]\nname = \"Trash text\"\nextensions = [\"txt\"]\naction = \"trash\"\n",
    )
    .unwrap();

    let bin = env!("CARGO_BIN_EXE_tidy");
    let apply = std::process::Command::new(bin)
        .env("HOME", &home)
        .env("TIDY_HOME", &home)
        .env("TIDY_CONFIG_DIR", &config)
        .env("TIDY_DATA_DIR", &data)
        .env_remove("TIDY_FAKE_TRASH")
        .args([
            "--apply",
            "--rules",
            rules.to_str().unwrap(),
            folder.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(
        apply.status.success(),
        "apply failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&apply.stdout),
        String::from_utf8_lossy(&apply.stderr)
    );
    assert!(!file.exists(), "the file should have left the inbox");

    let undo = std::process::Command::new(bin)
        .env("HOME", &home)
        .env("TIDY_HOME", &home)
        .env("TIDY_CONFIG_DIR", &config)
        .env("TIDY_DATA_DIR", &data)
        .env_remove("TIDY_FAKE_TRASH")
        .arg("undo")
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&undo.stdout);
    let stderr = String::from_utf8_lossy(&undo.stderr);
    assert!(
        stdout.contains(&name),
        "undo should name the file\n{stdout}\n{stderr}"
    );
    assert!(
        stdout.contains("macOS cannot restore") && stdout.contains("put it back"),
        "undo should print the manual restore message\n{stdout}\n{stderr}"
    );
    assert_eq!(
        undo.status.code(),
        Some(1),
        "a manual restore exits 1\n{stdout}\n{stderr}"
    );
}
