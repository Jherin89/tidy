//! Library-level tests. Every path is under a temp directory, and trash goes
//! through FakeTrasher so the system Trash is never touched.

use std::fs::{self, File};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use tidy::apply::apply_plan;
use tidy::plan::{plan_folder, ActionKind, ActionStatus};
use tidy::rules::{load_rules, Ruleset};
use tidy::trash::FakeTrasher;
use tidy::undo::{undo_latest, UndoOutcome};
use tidy::Env;

fn env_at(root: &Path) -> Env {
    Env {
        home: root.join("home"),
        config_dir: root.join("config"),
        config_from_env: true,
        data_dir: root.join("data"),
        fake_trash: Some(root.join("trash")),
    }
}

fn write_aged(path: &Path, bytes: &[u8], age: Duration) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(path, bytes).unwrap();
    let file = File::options().write(true).open(path).unwrap();
    file.set_modified(SystemTime::now() - age).unwrap();
}

fn builtin(env: &Env) -> Ruleset {
    load_rules(env, None, false).unwrap()
}

#[test]
fn builtin_plan_apply_and_undo_round_trip() {
    let tmp = tempfile::tempdir().unwrap();
    let env = env_at(tmp.path());
    let folder = env.downloads();
    fs::create_dir_all(&folder).unwrap();
    fs::create_dir_all(folder.join("nested")).unwrap();

    write_aged(
        &folder.join("Screenshot 1.png"),
        b"shot",
        Duration::from_secs(3 * 86400),
    );
    write_aged(
        &folder.join("photo.png"),
        b"img",
        Duration::from_secs(86400),
    );
    write_aged(
        &folder.join("report.pdf"),
        b"pdf-bytes",
        Duration::from_secs(86400),
    );
    write_aged(&folder.join("song.mp3"), b"mp3", Duration::from_secs(86400));
    write_aged(&folder.join("clip.mp4"), b"mp4", Duration::from_secs(86400));
    write_aged(
        &folder.join("notes.txt"),
        b"hello",
        Duration::from_secs(86400),
    );
    write_aged(
        &folder.join("budget.csv"),
        b"a,b",
        Duration::from_secs(86400),
    );
    write_aged(
        &folder.join("book.epub"),
        b"epub",
        Duration::from_secs(86400),
    );
    write_aged(
        &folder.join("old.zip"),
        b"zip",
        Duration::from_secs(20 * 86400),
    );
    write_aged(
        &folder.join("new.zip"),
        b"fresh",
        Duration::from_secs(86400),
    );
    write_aged(
        &folder.join("setup.dmg"),
        b"dmg-old",
        Duration::from_secs(40 * 86400),
    );
    write_aged(
        &folder.join("fresh.dmg"),
        b"dmg-new",
        Duration::from_secs(86400),
    );
    write_aged(
        &folder.join(".secret.pdf"),
        b"hide",
        Duration::from_secs(86400),
    );
    write_aged(
        &folder.join("video.mp4.part"),
        b"partial",
        Duration::from_secs(86400),
    );
    write_aged(
        &folder.join("nested/inner.pdf"),
        b"inner",
        Duration::from_secs(86400),
    );
    #[cfg(unix)]
    std::os::unix::fs::symlink(folder.join("report.pdf"), folder.join("link.pdf")).unwrap();

    let rules = builtin(&env);
    assert!(matches!(rules.source, tidy::rules::RulesSource::BuiltIn));

    let plan = plan_folder(&env, &folder, &rules, None, SystemTime::now()).unwrap();
    assert!(
        !env.home.join("Documents/PDFs").exists(),
        "planning must not create destination folders"
    );

    let rule_for = |name: &str| {
        plan.actions
            .iter()
            .find(|action| action.file.name == name)
            .unwrap_or_else(|| panic!("{name} was not matched"))
    };
    assert_eq!(rule_for("Screenshot 1.png").rule_name, "Screenshots");
    assert_eq!(rule_for("photo.png").rule_name, "Images");
    assert_eq!(rule_for("report.pdf").rule_name, "PDFs");
    assert_eq!(rule_for("song.mp3").rule_name, "Audio");
    assert_eq!(rule_for("clip.mp4").rule_name, "Videos");
    assert_eq!(rule_for("notes.txt").rule_name, "Documents");
    assert_eq!(rule_for("budget.csv").rule_name, "Spreadsheets");
    assert_eq!(rule_for("book.epub").rule_name, "Ebooks");
    assert_eq!(rule_for("old.zip").rule_name, "Old archives");
    assert_eq!(rule_for("setup.dmg").kind, ActionKind::Trash);
    assert!(plan.unmatched.iter().any(|file| file.name == "new.zip"));
    assert!(plan.unmatched.iter().any(|file| file.name == "fresh.dmg"));
    assert!(plan.skipped.iter().any(|skip| skip.name == ".secret.pdf"));
    assert!(plan
        .skipped
        .iter()
        .any(|skip| skip.name == "video.mp4.part"));
    assert!(plan.skipped.iter().any(|skip| skip.name == "nested"));
    #[cfg(unix)]
    assert!(plan.skipped.iter().any(|skip| skip.name == "link.pdf"));

    let shot = rule_for("Screenshot 1.png");
    let bucket = tidy::plan::month_bucket(shot.file.modified);
    assert_eq!(
        shot.destination.as_ref().unwrap(),
        &env.home
            .join("Pictures/Screenshots")
            .join(bucket)
            .join("Screenshot 1.png")
    );

    let mut plan = plan;
    let trasher = FakeTrasher {
        root: env.fake_trash.clone().unwrap(),
    };
    apply_plan(&env, &mut plan, &trasher).unwrap();
    assert!(plan.applied);
    assert!(plan.log_path.is_some());
    assert!(!folder.join("report.pdf").exists());
    assert_eq!(
        fs::read(env.home.join("Documents/PDFs/report.pdf")).unwrap(),
        b"pdf-bytes"
    );
    assert!(!folder.join("setup.dmg").exists());
    assert!(trash_contains(&env.fake_trash.clone().unwrap(), b"dmg-old"));
    assert_eq!(fs::read(folder.join("new.zip")).unwrap(), b"fresh");
    assert_eq!(fs::read(folder.join("fresh.dmg")).unwrap(), b"dmg-new");
    assert_eq!(fs::read(folder.join(".secret.pdf")).unwrap(), b"hide");
    assert_eq!(fs::read(folder.join("nested/inner.pdf")).unwrap(), b"inner");

    let report = undo_latest(&env, &trasher).unwrap();
    assert!(report.fully_undone);
    assert!(report
        .items
        .iter()
        .all(|item| { matches!(item.outcome, UndoOutcome::MovedBack | UndoOutcome::Restored) }));
    assert_eq!(fs::read(folder.join("report.pdf")).unwrap(), b"pdf-bytes");
    assert_eq!(fs::read(folder.join("setup.dmg")).unwrap(), b"dmg-old");
    assert!(!env.home.join("Documents/PDFs/report.pdf").exists());
    assert!(!trash_contains(
        &env.fake_trash.clone().unwrap(),
        b"dmg-old"
    ));
    assert_eq!(fs::read(folder.join("notes.txt")).unwrap(), b"hello");
    assert_eq!(fs::read(folder.join("old.zip")).unwrap(), b"zip");
}

#[test]
fn apply_and_undo_never_overwrite() {
    let tmp = tempfile::tempdir().unwrap();
    let env = env_at(tmp.path());
    let folder = env.downloads();
    let dest = env.home.join("Documents/Downloads");
    fs::create_dir_all(&dest).unwrap();
    fs::write(dest.join("notes.txt"), b"keep-me").unwrap();
    write_aged(
        &folder.join("notes.txt"),
        b"from-downloads",
        Duration::from_secs(10),
    );

    let rules = builtin(&env);
    let mut plan = plan_folder(&env, &folder, &rules, None, SystemTime::now()).unwrap();
    let trasher = FakeTrasher {
        root: env.fake_trash.clone().unwrap(),
    };
    apply_plan(&env, &mut plan, &trasher).unwrap();
    assert_eq!(fs::read(dest.join("notes.txt")).unwrap(), b"keep-me");
    assert_eq!(
        fs::read(dest.join("notes (1).txt")).unwrap(),
        b"from-downloads"
    );
    assert!(plan
        .actions
        .iter()
        .all(|action| action.status == ActionStatus::Applied));

    fs::write(folder.join("notes.txt"), b"brand-new").unwrap();
    let report = undo_latest(&env, &trasher).unwrap();
    assert_eq!(report.failure_count(), 0);
    assert_eq!(fs::read(folder.join("notes.txt")).unwrap(), b"brand-new");
    assert_eq!(
        fs::read(folder.join("notes (1).txt")).unwrap(),
        b"from-downloads"
    );
    assert!(!dest.join("notes (1).txt").exists());
    assert_eq!(fs::read(dest.join("notes.txt")).unwrap(), b"keep-me");
}

#[test]
fn undo_walks_back_through_runs_that_are_not_yet_undone() {
    let tmp = tempfile::tempdir().unwrap();
    let env = env_at(tmp.path());
    let folder = env.downloads();
    fs::create_dir_all(&folder).unwrap();
    let rules = builtin(&env);
    let trasher = FakeTrasher {
        root: env.fake_trash.clone().unwrap(),
    };

    write_aged(&folder.join("one.pdf"), b"one", Duration::from_secs(5));
    let mut first = plan_folder(&env, &folder, &rules, None, SystemTime::now()).unwrap();
    apply_plan(&env, &mut first, &trasher).unwrap();

    write_aged(&folder.join("two.pdf"), b"two", Duration::from_secs(5));
    let mut second = plan_folder(&env, &folder, &rules, None, SystemTime::now()).unwrap();
    apply_plan(&env, &mut second, &trasher).unwrap();
    assert_eq!(second.actions.len(), 1);

    let report = undo_latest(&env, &trasher).unwrap();
    assert_eq!(report.items.len(), 1);
    assert_eq!(report.items[0].file_name, "two.pdf");
    assert!(folder.join("two.pdf").is_file());
    assert!(!folder.join("one.pdf").exists());

    let report = undo_latest(&env, &trasher).unwrap();
    assert_eq!(report.items[0].file_name, "one.pdf");
    assert_eq!(fs::read(folder.join("one.pdf")).unwrap(), b"one");
    assert!(undo_latest(&env, &trasher).is_err());
}

#[test]
fn invalid_rules_file_includes_the_parse_error() {
    let tmp = tempfile::tempdir().unwrap();
    let env = env_at(tmp.path());
    let path = tmp.path().join("broken.toml");
    fs::write(&path, "[[rules]\nname = \"nope\"\n").unwrap();
    let err = load_rules(&env, Some(&path), false).unwrap_err();
    let text = err.to_string();
    assert!(text.contains("invalid rules file"), "{text}");
    assert!(text.contains("broken.toml"), "{text}");
}

#[test]
fn fake_trash_restore_uses_a_suffix_when_the_original_name_is_taken() {
    let tmp = tempfile::tempdir().unwrap();
    let env = env_at(tmp.path());
    let folder = env.downloads();
    write_aged(
        &folder.join("setup.dmg"),
        b"old-installer",
        Duration::from_secs(40 * 86400),
    );
    let rules = builtin(&env);
    let mut plan = plan_folder(&env, &folder, &rules, None, SystemTime::now()).unwrap();
    assert_eq!(plan.actions.len(), 1);
    assert_eq!(plan.actions[0].kind, ActionKind::Trash);
    let trasher = FakeTrasher {
        root: env.fake_trash.clone().unwrap(),
    };
    apply_plan(&env, &mut plan, &trasher).unwrap();
    fs::write(folder.join("setup.dmg"), b"do-not-touch").unwrap();

    let report = undo_latest(&env, &trasher).unwrap();
    assert_eq!(report.items[0].outcome, UndoOutcome::Restored);
    assert_eq!(fs::read(folder.join("setup.dmg")).unwrap(), b"do-not-touch");
    assert_eq!(
        fs::read(folder.join("setup (1).dmg")).unwrap(),
        b"old-installer"
    );
}

fn trash_contains(dir: &Path, bytes: &[u8]) -> bool {
    if !dir.exists() {
        return false;
    }
    walk_files(dir).any(|path| fs::read(path).ok().as_deref() == Some(bytes))
}

fn walk_files(dir: &Path) -> impl Iterator<Item = PathBuf> {
    fn collect(dir: &Path, out: &mut Vec<PathBuf>) {
        let Ok(reader) = fs::read_dir(dir) else {
            return;
        };
        for entry in reader.flatten() {
            let path = entry.path();
            if path.is_dir() {
                collect(&path, out);
            } else if path.is_file() {
                out.push(path);
            }
        }
    }
    let mut out = Vec::new();
    collect(dir, &mut out);
    out.into_iter()
}

#[test]
fn undo_removes_only_the_empty_directories_apply_created() {
    let tmp = tempfile::tempdir().unwrap();
    let env = env_at(tmp.path());
    let folder = env.downloads();
    // Pre-existing (empty) destination parent: must survive undo.
    let pictures = env.home.join("Pictures");
    fs::create_dir_all(&pictures).unwrap();
    write_aged(&folder.join("a.pdf"), b"pdf", Duration::from_secs(10));
    write_aged(&folder.join("b.png"), b"png", Duration::from_secs(10));
    write_aged(&folder.join("c.txt"), b"txt", Duration::from_secs(10));

    let rules = builtin(&env);
    let mut plan = plan_folder(&env, &folder, &rules, None, SystemTime::now()).unwrap();
    let trasher = FakeTrasher {
        root: env.fake_trash.clone().unwrap(),
    };
    apply_plan(&env, &mut plan, &trasher).unwrap();
    assert!(env.home.join("Documents/PDFs/a.pdf").exists());
    assert!(env.home.join("Pictures/Downloads/b.png").exists());

    // Something the user drops into a created folder afterwards is kept.
    fs::write(env.home.join("Documents/Downloads/mine.txt"), b"mine").unwrap();

    let report = undo_latest(&env, &trasher).unwrap();
    assert_eq!(report.failure_count(), 0);
    assert!(folder.join("a.pdf").exists());
    assert!(!env.home.join("Documents/PDFs").exists());
    assert!(!env.home.join("Pictures/Downloads").exists());
    assert!(pictures.is_dir(), "pre-existing directory must be kept");
    assert_eq!(
        fs::read(env.home.join("Documents/Downloads/mine.txt")).unwrap(),
        b"mine"
    );
}
