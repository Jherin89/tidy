//! CLI tests. HOME, config, data, and trash are all pointed at a temp dir.

use std::fs::{self, File};
use std::path::Path;
use std::time::{Duration, SystemTime};

use assert_cmd::Command;
use predicates::prelude::*;

fn sandbox() -> tempfile::TempDir {
    tempfile::tempdir().unwrap()
}

fn tidy(root: &Path) -> Command {
    let home = root.join("home");
    let config = root.join("config");
    let data = root.join("data");
    let trash = root.join("trash");
    fs::create_dir_all(&home).unwrap();
    fs::create_dir_all(&config).unwrap();
    fs::create_dir_all(&data).unwrap();
    let mut cmd = Command::cargo_bin("tidy").unwrap();
    cmd.env("HOME", &home)
        .env("TIDY_HOME", &home)
        .env("XDG_CONFIG_HOME", &config)
        .env("XDG_DATA_HOME", &data)
        .env("TIDY_CONFIG_DIR", &config)
        .env("TIDY_DATA_DIR", &data)
        .env("TIDY_FAKE_TRASH", &trash)
        .env("NO_COLOR", "1")
        .current_dir(root);
    cmd
}

fn downloads(root: &Path) -> std::path::PathBuf {
    let path = root.join("home/Downloads");
    fs::create_dir_all(&path).unwrap();
    path
}

fn write_aged(path: &Path, bytes: &[u8], age: Duration) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(path, bytes).unwrap();
    let file = File::options().write(true).open(path).unwrap();
    file.set_modified(SystemTime::now() - age).unwrap();
}

#[test]
fn dry_run_prints_a_table_and_changes_nothing() {
    let tmp = sandbox();
    let dir = downloads(tmp.path());
    write_aged(&dir.join("report.pdf"), b"pdf", Duration::from_secs(86_400));
    write_aged(
        &dir.join("mystery.bin"),
        b"bin",
        Duration::from_secs(86_400),
    );
    write_aged(
        &dir.join(".hidden.pdf"),
        b"hide",
        Duration::from_secs(86_400),
    );

    let assert = tidy(tmp.path()).arg(dir.as_os_str()).assert().success();
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
    assert!(stdout.contains("report.pdf"), "{stdout}");
    assert!(stdout.contains("PDFs"), "{stdout}");
    assert!(stdout.contains("mystery.bin"), "{stdout}");
    assert!(stdout.contains("Dry run"), "{stdout}");
    assert!(stdout.contains("--apply"), "{stdout}");
    assert!(dir.join("report.pdf").is_file());
    assert!(!tmp.path().join("home/Documents/PDFs").exists());
    assert!(dir.join(".hidden.pdf").is_file());
}

#[test]
fn json_dry_run_is_machine_readable() {
    let tmp = sandbox();
    let dir = downloads(tmp.path());
    write_aged(&dir.join("report.pdf"), b"pdf", Duration::from_secs(86_400));
    write_aged(
        &dir.join("file.crdownload"),
        b"part",
        Duration::from_secs(10),
    );

    let assert = tidy(tmp.path())
        .args(["--json", dir.to_str().unwrap()])
        .assert()
        .success();
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
    let value: serde_json::Value = serde_json::from_str(&stdout).expect(&stdout);
    assert_eq!(value["mode"], "dry-run");
    assert_eq!(value["version"], 1);
    assert_eq!(value["entries"][0]["file"], "report.pdf");
    assert_eq!(value["entries"][0]["action"], "move");
    assert_eq!(value["entries"][0]["status"], "planned");
    let skipped = value["skipped"].as_array().unwrap();
    assert!(skipped
        .iter()
        .any(|item| item["reason"] == "partial download"));
}

#[test]
fn apply_then_undo_restores_the_folder() {
    let tmp = sandbox();
    let dir = downloads(tmp.path());
    write_aged(
        &dir.join("report.pdf"),
        b"pdf-bytes",
        Duration::from_secs(86_400),
    );
    write_aged(
        &dir.join("invoice.pdf"),
        b"same-bytes",
        Duration::from_secs(86_400),
    );
    write_aged(
        &dir.join("invoice (1).pdf"),
        b"same-bytes",
        Duration::from_secs(86_400),
    );
    write_aged(
        &dir.join("notes.txt"),
        b"hello",
        Duration::from_secs(86_400),
    );
    fs::create_dir_all(dir.join("subdir")).unwrap();
    fs::write(dir.join("subdir/inner.txt"), b"stay").unwrap();

    tidy(tmp.path())
        .args(["--apply", "--json"])
        .arg(&dir)
        .assert()
        .success()
        .stdout(predicate::str::contains("\"mode\": \"applied\""));

    assert!(!dir.join("report.pdf").exists());
    assert!(!dir.join("notes.txt").exists());
    assert_eq!(
        fs::read(tmp.path().join("home/Documents/PDFs/report.pdf")).unwrap(),
        b"pdf-bytes"
    );
    assert_eq!(
        fs::read(tmp.path().join("home/Documents/Downloads/notes.txt")).unwrap(),
        b"hello"
    );
    assert!(
        trash_has(&tmp.path().join("trash"), b"same-bytes"),
        "the extra invoice copy should be trashed"
    );
    assert_eq!(fs::read(dir.join("subdir/inner.txt")).unwrap(), b"stay");

    tidy(tmp.path())
        .arg("undo")
        .assert()
        .success()
        .stdout(predicate::str::contains("moved back"))
        .stdout(predicate::str::contains("restored"));

    assert_eq!(fs::read(dir.join("report.pdf")).unwrap(), b"pdf-bytes");
    assert_eq!(fs::read(dir.join("notes.txt")).unwrap(), b"hello");
    assert!(!tmp.path().join("home/Documents/PDFs/report.pdf").exists());
    assert!(!trash_has(&tmp.path().join("trash"), b"same-bytes"));
}

#[test]
fn bad_paths_and_bad_rules_fail_with_a_tidy_prefix() {
    let tmp = sandbox();
    tidy(tmp.path())
        .arg(tmp.path().join("missing"))
        .assert()
        .failure()
        .stderr(predicate::str::starts_with("tidy: "))
        .stderr(predicate::str::contains("not found"));

    let file = tmp.path().join("not-a-dir");
    fs::write(&file, b"x").unwrap();
    tidy(tmp.path())
        .arg(&file)
        .assert()
        .failure()
        .stderr(predicate::str::starts_with("tidy: "))
        .stderr(predicate::str::contains("not a directory"));

    let rules = tmp.path().join("broken.toml");
    fs::write(&rules, "[[rules]\n").unwrap();
    tidy(tmp.path())
        .args(["--rules"])
        .arg(&rules)
        .arg(downloads(tmp.path()))
        .assert()
        .failure()
        .stderr(predicate::str::starts_with("tidy: "))
        .stderr(predicate::str::contains("invalid rules file"));

    tidy(tmp.path())
        .args(["--older-than", "nope"])
        .arg(downloads(tmp.path()))
        .assert()
        .failure()
        .stderr(predicate::str::contains("--older-than"));
}

#[test]
fn init_refuses_to_overwrite_unless_forced_and_rules_shows_the_source() {
    let tmp = sandbox();
    tidy(tmp.path())
        .arg("rules")
        .assert()
        .success()
        .stdout(predicate::str::contains("built-in defaults"))
        .stdout(predicate::str::contains("Duplicates"))
        .stdout(predicate::str::contains("Old installers"));

    tidy(tmp.path()).args(["init"]).assert().success();
    let rules = tmp.path().join("config/tidy/rules.toml");
    assert!(rules.is_file());
    let text = fs::read_to_string(&rules).unwrap();
    assert!(text.contains("[[rules]]"));
    assert!(text.contains("name = \"Duplicates\""));
    assert!(text.contains("Screenshots"));

    tidy(tmp.path())
        .args(["init"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("--force"));

    tidy(tmp.path())
        .args(["config", "--init", "--force", "--show"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Rules source:"))
        .stdout(predicate::str::contains("rules.toml"));

    tidy(tmp.path())
        .args(["rules", "--json"])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"source\": \"file\""));
}

#[test]
fn older_than_flag_filters_before_rules() {
    let tmp = sandbox();
    let dir = downloads(tmp.path());
    write_aged(
        &dir.join("old.pdf"),
        b"old",
        Duration::from_secs(10 * 86_400),
    );
    write_aged(&dir.join("new.pdf"), b"new", Duration::from_secs(86_400));

    let assert = tidy(tmp.path())
        .args(["--json", "--older-than", "7d"])
        .arg(&dir)
        .assert()
        .success();
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
    let value: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    let files: Vec<_> = value["entries"]
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| entry["file"].as_str().unwrap().to_string())
        .collect();
    // Birth/Date Added is now, so a backdated mtime does not make the file old.
    assert!(
        files.is_empty(),
        "just-created files must not pass --older-than 7d: {files:?}"
    );
    let skipped = value["skipped"].as_array().unwrap();
    assert!(skipped
        .iter()
        .any(|item| { item["file"] == "old.pdf" && item["reason"] == "newer than --older-than" }));
    assert!(skipped.iter().any(|item| item["file"] == "new.pdf"));
}

#[test]
fn subcommand_help_omits_flags_that_do_not_apply() {
    let init = String::from_utf8(
        tidy(sandbox().path())
            .args(["init", "--help"])
            .assert()
            .success()
            .get_output()
            .stdout
            .clone(),
    )
    .unwrap();
    assert!(!init.contains("--apply"), "{init}");
    assert!(!init.contains("--older-than"), "{init}");
    assert!(!init.contains("--rules"), "{init}");
    assert!(!init.contains("--json"), "{init}");

    let undo = String::from_utf8(
        tidy(sandbox().path())
            .args(["undo", "--help"])
            .assert()
            .success()
            .get_output()
            .stdout
            .clone(),
    )
    .unwrap();
    assert!(undo.contains("--json"), "{undo}");
    assert!(!undo.contains("--apply"), "{undo}");
    assert!(!undo.contains("--older-than"), "{undo}");
    assert!(!undo.contains("--rules"), "{undo}");

    let watch = String::from_utf8(
        tidy(sandbox().path())
            .args(["watch", "--help"])
            .assert()
            .success()
            .get_output()
            .stdout
            .clone(),
    )
    .unwrap();
    assert!(watch.contains("--older-than"), "{watch}");
    assert!(watch.contains("--rules"), "{watch}");
    assert!(watch.contains("--json"), "{watch}");
    assert!(!watch.contains("--apply"), "{watch}");
}

#[test]
fn explain_json_matches_a_duplicate_and_leaves_a_young_installer() {
    let tmp = sandbox();
    let dir = downloads(tmp.path());
    write_aged(
        &dir.join("invoice.pdf"),
        b"same-invoice",
        Duration::from_secs(3 * 86_400),
    );
    write_aged(
        &dir.join("invoice (1).pdf"),
        b"same-invoice",
        Duration::from_secs(86_400),
    );
    write_aged(
        &dir.join("Docker.dmg"),
        b"dmg-new",
        Duration::from_secs(5 * 86_400),
    );
    write_aged(
        &dir.join(".secret.pdf"),
        b"hide",
        Duration::from_secs(86_400),
    );

    let assert = tidy(tmp.path())
        .args(["explain", "--json"])
        .arg(dir.join("invoice (1).pdf"))
        .assert()
        .success();
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
    let value: serde_json::Value = serde_json::from_str(&stdout).expect(&stdout);
    assert_eq!(value["version"], 1);
    assert_eq!(value["outcome"]["status"], "matched");
    assert_eq!(value["outcome"]["rule"], "Duplicates");
    assert_eq!(value["outcome"]["action"], "trash");
    assert_eq!(
        value["outcome"]["destination"],
        "Trash (copy of invoice.pdf)"
    );
    assert_eq!(value["outcome"]["copy_of"], "invoice.pdf");
    assert!(value["rules"]
        .as_array()
        .unwrap()
        .iter()
        .any(|rule| rule["name"] == "Duplicates" && rule["result"] == "wins"));
    assert!(value["rules"]
        .as_array()
        .unwrap()
        .iter()
        .any(|rule| rule["result"] == "not evaluated"));

    tidy(tmp.path())
        .args(["explain"])
        .arg(dir.join("Docker.dmg"))
        .assert()
        .success()
        .stdout(predicate::str::contains("no rule matches; left in place"))
        .stdout(predicate::str::contains("needs ≥ 30d"));

    tidy(tmp.path())
        .args(["explain"])
        .arg(dir.join(".secret.pdf"))
        .assert()
        .success()
        .stdout(predicate::str::contains("hidden"));

    tidy(tmp.path())
        .args(["explain", "--json"])
        .arg(dir.join("missing.pdf"))
        .assert()
        .failure()
        .stderr(predicate::str::contains("file not found"));

    let rules = tmp.path().join("broken.toml");
    fs::write(&rules, "[[rules]\n").unwrap();
    tidy(tmp.path())
        .args(["explain", "--rules"])
        .arg(&rules)
        .arg(dir.join("invoice.pdf"))
        .assert()
        .failure()
        .stderr(predicate::str::contains("invalid rules file"));
}

#[test]
fn json_before_the_subcommand_still_selects_json() {
    let tmp = sandbox();
    tidy(tmp.path())
        .args(["--json", "rules"])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"source\""));

    tidy(tmp.path())
        .args(["undo", "--json"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("tidy:"));
}

#[cfg(unix)]
#[test]
fn launchd_install_status_and_uninstall_use_the_fake_launchctl() {
    let tmp = sandbox();
    let root = tmp.path();
    let log = root.join("launchctl.log");
    let script = root.join("launchctl");
    let body = "if [ \"$1\" = print ]; then\ncat <<'EOF'\nstate = running\npid = 4242\nlast exit code = 0\nEOF\nfi\nif [ \"$1\" = bootout ]; then\nexit 3\nfi\nexit 0\n";
    let text = format!(
        "#!/bin/sh\nprintf '%s\\n' \"$*\" >> '{}'\n{body}",
        log.display()
    );
    fs::write(&script, text).unwrap();
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();

    let folder = root.join("inbox & <drafts>");
    fs::create_dir_all(&folder).unwrap();
    let home = root.join("home");

    let tidy_at = |args: &[&str]| {
        let mut cmd = tidy(root);
        cmd.env("TIDY_LAUNCHCTL", &script)
            .env("TIDY_UID", "501")
            .args(args);
        cmd
    };

    let printed = tidy_at(&["watch", "--install", "--print"])
        .arg(&folder)
        .assert()
        .success();
    let xml = String::from_utf8(printed.get_output().stdout.clone()).unwrap();
    let err = String::from_utf8(printed.get_output().stderr.clone()).unwrap();
    assert!(xml.starts_with("<?xml"), "{xml}");
    assert!(xml.contains("com.github.jherin89.tidy"), "{xml}");
    assert!(xml.contains("a&amp;") || xml.contains("&amp;"), "{xml}");
    assert!(xml.contains("&lt;drafts&gt;"), "{xml}");
    assert!(xml.contains("<string>watch</string>"), "{xml}");
    assert!(xml.contains("Background"), "{xml}");
    assert!(err.contains("Using binary"), "{err}");
    assert!(
        !home
            .join("Library/LaunchAgents/com.github.jherin89.tidy.plist")
            .exists(),
        "--print must not write a plist"
    );

    tidy_at(&["watch", "--install"])
        .arg(&folder)
        .assert()
        .success()
        .stdout(predicate::str::contains("Using binary"))
        .stdout(predicate::str::contains(
            "Loaded gui/501/com.github.jherin89.tidy",
        ));

    let plist = home.join("Library/LaunchAgents/com.github.jherin89.tidy.plist");
    let written = fs::read_to_string(&plist).unwrap();
    assert!(written.contains("&amp;"), "{written}");
    assert!(written.contains("&lt;drafts&gt;"), "{written}");
    assert!(written.contains("<key>RunAtLoad</key>"), "{written}");
    assert!(written.contains("<key>KeepAlive</key>"), "{written}");
    assert!(written.contains("Library/Logs/tidy.log"), "{written}");

    let calls = fs::read_to_string(&log).unwrap();
    assert!(
        calls.contains("bootout gui/501/com.github.jherin89.tidy"),
        "{calls}"
    );
    assert!(
        calls.contains(&format!("bootstrap gui/501 {}", plist.display())),
        "{calls}"
    );

    fs::write(&plist, "<plist>different</plist>\n").unwrap();
    tidy_at(&["watch", "--install"])
        .arg(&folder)
        .assert()
        .failure()
        .stderr(predicate::str::contains("--force"));
    assert_eq!(
        fs::read_to_string(&plist).unwrap(),
        "<plist>different</plist>\n"
    );

    tidy_at(&["watch", "--install", "--force"])
        .arg(&folder)
        .assert()
        .success();
    assert!(fs::read_to_string(&plist)
        .unwrap()
        .contains("com.github.jherin89.tidy"));

    fs::create_dir_all(home.join("Library/Logs")).unwrap();
    fs::write(
        home.join("Library/Logs/tidy.log"),
        "line-one\nline-two\nsettled invoice.pdf\n",
    )
    .unwrap();

    let status = tidy_at(&["watch", "--status"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let status = String::from_utf8(status).unwrap();
    assert!(status.contains("present"), "{status}");
    assert!(status.contains("running"), "{status}");
    assert!(status.contains("4242"), "{status}");
    assert!(status.contains("inbox & <drafts>"), "{status}");
    assert!(status.contains("settled invoice.pdf"), "{status}");
    assert!(status.contains("tidy.log"), "{status}");

    let json = tidy_at(&["watch", "--status", "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let json: serde_json::Value = serde_json::from_slice(&json).unwrap();
    assert_eq!(json["loaded"], true);
    assert_eq!(json["state"], "running");
    assert_eq!(json["pid"], 4242);
    assert_eq!(json["last_exit_code"], 0);
    assert_eq!(json["plist_exists"], true);

    tidy_at(&["watch", "--uninstall"]).assert().success();
    assert!(!plist.exists());
    let calls = fs::read_to_string(&log).unwrap();
    assert!(
        calls
            .matches("bootout gui/501/com.github.jherin89.tidy")
            .count()
            >= 2,
        "{calls}"
    );
}

#[cfg(not(target_os = "macos"))]
#[test]
fn install_without_a_launchctl_override_explains_that_launchd_is_macos_only() {
    let tmp = sandbox();
    tidy(tmp.path())
        .env_remove("TIDY_LAUNCHCTL")
        .args(["watch", "--install"])
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "launchd is macOS-only; on Linux use a systemd user service — see README",
        ));
    assert!(!tmp
        .path()
        .join("home/Library/LaunchAgents/com.github.jherin89.tidy.plist")
        .exists());
}

#[test]
fn bundle_is_listed_moved_and_undone() {
    let tmp = sandbox();
    let dir = downloads(tmp.path());
    let pages = dir.join("Report.pages");
    fs::create_dir_all(pages.join("Contents")).unwrap();
    fs::write(pages.join("index.xml"), b"<doc/>").unwrap();
    fs::write(pages.join("Contents/Data"), b"inside").unwrap();
    fs::create_dir_all(dir.join("skip-me")).unwrap();

    let stdout = String::from_utf8(
        tidy(tmp.path())
            .arg(dir.as_os_str())
            .assert()
            .success()
            .get_output()
            .stdout
            .clone(),
    )
    .unwrap();
    assert!(stdout.contains("Report.pages"), "{stdout}");
    assert!(stdout.contains("Documents"), "{stdout}");
    assert!(stdout.contains("skip-me"), "{stdout}");
    assert!(pages.join("Contents/Data").is_file());

    tidy(tmp.path())
        .args(["--apply"])
        .arg(&dir)
        .assert()
        .success();
    let dest = tmp.path().join("home/Documents/Downloads/Report.pages");
    assert!(!pages.exists());
    assert_eq!(fs::read(dest.join("index.xml")).unwrap(), b"<doc/>");
    assert_eq!(fs::read(dest.join("Contents/Data")).unwrap(), b"inside");
    assert!(dir.join("skip-me").is_dir());

    tidy(tmp.path()).arg("undo").assert().success();
    assert_eq!(fs::read(pages.join("index.xml")).unwrap(), b"<doc/>");
    assert_eq!(fs::read(pages.join("Contents/Data")).unwrap(), b"inside");
    assert!(!dest.exists());
}

#[cfg(unix)]
#[test]
fn watch_install_print_writes_rules_and_age_into_the_plist() {
    let tmp = sandbox();
    let root = tmp.path();
    let folder = downloads(root);
    let rules = root.join("custom.toml");
    fs::write(
        &rules,
        r#"
[[rules]]
name = "PDFs"
extensions = ["pdf"]
action = "move"
destination = "Filed"
"#,
    )
    .unwrap();

    let printed = tidy(root)
        .args([
            "watch",
            "--install",
            "--print",
            "--older-than",
            "7d",
            "--rules",
        ])
        .arg(&rules)
        .arg(&folder)
        .assert()
        .success();
    let xml = String::from_utf8(printed.get_output().stdout.clone()).unwrap();
    assert!(xml.contains("<string>--rules</string>"), "{xml}");
    assert!(xml.contains("<string>--older-than</string>"), "{xml}");
    assert!(xml.contains("<string>7d</string>"), "{xml}");
    let rules_abs = rules.canonicalize().unwrap();
    assert!(
        xml.contains(&format!("<string>{}</string>", rules_abs.display())),
        "{xml}"
    );

    tidy(root)
        .args(["watch", "--install", "--json"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("--json"));

    tidy(root)
        .args(["--apply", "watch", "--install", "--print"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("--apply"));
}

#[cfg(unix)]
#[test]
fn watch_install_persists_flags_and_status_reads_them() {
    let tmp = sandbox();
    let root = tmp.path();
    let log = root.join("launchctl.log");
    let script = root.join("launchctl");
    let body = "if [ \"$1\" = print ]; then\ncat <<'EOF'\nstate = running\npid = 7\nlast exit code = 0\nEOF\nfi\nif [ \"$1\" = bootout ]; then\nexit 3\nfi\nexit 0\n";
    fs::write(
        &script,
        format!(
            "#!/bin/sh\nprintf '%s\\n' \"$*\" >> '{}'\n{body}",
            log.display()
        ),
    )
    .unwrap();
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();

    let folder = root.join("inbox & drafts");
    fs::create_dir_all(&folder).unwrap();
    let rules = root.join("rules & <mine>.toml");
    fs::write(
        &rules,
        "[[rules]]\nname = \"T\"\nextensions = [\"txt\"]\naction = \"trash\"\n",
    )
    .unwrap();

    let mut cmd = tidy(root);
    cmd.env("TIDY_LAUNCHCTL", &script)
        .env("TIDY_UID", "501")
        .args(["watch", "--install", "--older-than", "12h", "--rules"])
        .arg(&rules)
        .arg(&folder)
        .assert()
        .success();

    let plist = root.join("home/Library/LaunchAgents/com.github.jherin89.tidy.plist");
    let written = fs::read_to_string(&plist).unwrap();
    assert!(written.contains("--rules"), "{written}");
    assert!(written.contains("--older-than"), "{written}");
    assert!(written.contains("12h"), "{written}");
    assert!(written.contains("&amp;"), "{written}");
    assert!(written.contains("&lt;mine&gt;"), "{written}");

    let status = String::from_utf8(
        tidy(root)
            .env("TIDY_LAUNCHCTL", &script)
            .env("TIDY_UID", "501")
            .args(["watch", "--status"])
            .assert()
            .success()
            .get_output()
            .stdout
            .clone(),
    )
    .unwrap();
    assert!(status.contains("12h"), "{status}");
    assert!(status.contains("rules"), "{status}");

    let json = tidy(root)
        .env("TIDY_LAUNCHCTL", &script)
        .env("TIDY_UID", "501")
        .args(["watch", "--status", "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let json: serde_json::Value = serde_json::from_slice(&json).unwrap();
    assert_eq!(json["older_than"], "12h");
    assert!(json["rules"].as_str().unwrap().contains("rules"));
}

#[test]
fn explain_reports_the_age_source() {
    let tmp = sandbox();
    let dir = downloads(tmp.path());
    write_aged(
        &dir.join("Docker.dmg"),
        b"dmg",
        Duration::from_secs(5 * 86_400),
    );
    let stdout = String::from_utf8(
        tidy(tmp.path())
            .args(["explain"])
            .arg(dir.join("Docker.dmg"))
            .assert()
            .success()
            .get_output()
            .stdout
            .clone(),
    )
    .unwrap();
    assert!(
        stdout.contains("(created)")
            || stdout.contains("(modified)")
            || stdout.contains("(date added)"),
        "{stdout}"
    );
    assert!(stdout.contains("needs ≥ 30d"), "{stdout}");
}

fn trash_has(dir: &Path, bytes: &[u8]) -> bool {
    if !dir.exists() {
        return false;
    }
    let mut stack = vec![dir.to_path_buf()];
    while let Some(path) = stack.pop() {
        let Ok(reader) = fs::read_dir(&path) else {
            continue;
        };
        for entry in reader.flatten() {
            let child = entry.path();
            if child.is_dir() {
                stack.push(child);
            } else if fs::read(&child).ok().as_deref() == Some(bytes) {
                return true;
            }
        }
    }
    false
}
