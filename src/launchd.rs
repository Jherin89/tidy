//! macOS launchd helper for `tidy watch`.
//!
//! `--uninstall` deletes one file: the LaunchAgent plist this module wrote
//! (`~/Library/LaunchAgents/com.github.jherin89.tidy.plist`). That is tidy's
//! own config, not a user download. Nothing else in tidy removes a file
//! except the cross-device move in `relocate`.

use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::error::Error;

pub const LABEL: &str = "com.github.jherin89.tidy";

/// Launchd label. `TIDY_LAUNCHD_LABEL` overrides the default so a smoke
/// test can install a throwaway agent without touching the real one.
pub fn job_label() -> Result<String, Error> {
    match std::env::var("TIDY_LAUNCHD_LABEL") {
        Ok(value) if !value.is_empty() => validate_label(&value),
        _ => Ok(LABEL.to_string()),
    }
}

fn validate_label(label: &str) -> Result<String, Error> {
    let ok = label.len() <= 64
        && label
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_alphabetic())
        && label
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-' || c == '_');
    if ok {
        Ok(label.to_string())
    } else {
        Err(Error::msg(format!(
            "TIDY_LAUNCHD_LABEL `{label}` is not a valid launchd label (letters, digits, '.', '-', '_'; max 64)"
        )))
    }
}

#[derive(Debug, Clone)]
pub struct LaunchPaths {
    pub home: PathBuf,
    pub uid: u32,
    pub launchctl: PathBuf,
    pub binary: PathBuf,
    pub label: String,
}

/// Arguments stored in the agent's ProgramArguments.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WatchProgram {
    pub folder: PathBuf,
    pub rules: Option<PathBuf>,
    pub older_than: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JobInfo {
    pub loaded: bool,
    pub state: Option<String>,
    pub pid: Option<u32>,
    pub last_exit_code: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaunchStatus {
    pub plist_path: PathBuf,
    pub plist_exists: bool,
    pub binary: Option<String>,
    pub folder: Option<String>,
    pub rules: Option<String>,
    pub older_than: Option<String>,
    pub arguments: Vec<String>,
    pub job: JobInfo,
    pub log_path: PathBuf,
    pub log_tail: Vec<String>,
}

pub fn plist_path(home: &Path) -> PathBuf {
    plist_path_for(home, LABEL)
}

pub fn plist_path_for(home: &Path, label: &str) -> PathBuf {
    home.join("Library")
        .join("LaunchAgents")
        .join(format!("{label}.plist"))
}

pub fn log_path(home: &Path) -> PathBuf {
    home.join("Library").join("Logs").join("tidy.log")
}

/// On macOS, or when `TIDY_LAUNCHCTL` is set so tests can inject a fake.
pub fn ensure_supported() -> Result<(), Error> {
    if cfg!(target_os = "macos") || std::env::var_os("TIDY_LAUNCHCTL").is_some() {
        return Ok(());
    }
    Err(Error::msg(
        "launchd is macOS-only; on Linux use a systemd user service — see README",
    ))
}

pub fn launchctl_program() -> PathBuf {
    std::env::var_os("TIDY_LAUNCHCTL")
        .map(PathBuf::from)
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or_else(|| PathBuf::from("launchctl"))
}

/// User id from `TIDY_UID`, otherwise `id -u`. No libc.
pub fn current_uid() -> Result<u32, Error> {
    if let Some(value) = std::env::var_os("TIDY_UID") {
        let text = value.to_string_lossy();
        if text.trim().is_empty() {
            return Err(Error::msg("TIDY_UID is set but empty"));
        }
        return text
            .trim()
            .parse::<u32>()
            .map_err(|_| Error::msg(format!("TIDY_UID is not a user id: {}", text.trim())));
    }
    let output = Command::new("id")
        .arg("-u")
        .output()
        .map_err(|err| Error::msg(format!("could not run `id -u` to learn the user id: {err}")))?;
    if !output.status.success() {
        return Err(Error::msg(format!(
            "could not learn the user id from `id -u`: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    let text = String::from_utf8_lossy(&output.stdout);
    text.trim()
        .parse::<u32>()
        .map_err(|_| Error::msg(format!("`id -u` returned {text:?}, which is not a user id")))
}

/// Absolute tidy binary. A Homebrew Cellar path becomes the stable prefix link
/// so `brew upgrade` does not leave the agent pointing at an old version.
pub fn resolved_tidy_binary() -> Result<PathBuf, Error> {
    let exe = std::env::current_exe()
        .map_err(|err| Error::msg(format!("could not locate the tidy binary: {err}")))?;
    let canon = fs::canonicalize(&exe).unwrap_or(exe);
    Ok(stable_brew_binary(&canon))
}

/// `…/Cellar/tidy/<version>/bin/tidy` → `<prefix>/bin/tidy`, or
/// `<prefix>/opt/tidy/bin/tidy` when that file exists and the prefix `bin`
/// link does not. Any other path is returned unchanged.
pub fn stable_brew_binary(exe: &Path) -> PathBuf {
    let comps: Vec<_> = exe.components().collect();
    for (index, comp) in comps.iter().enumerate() {
        if comp.as_os_str() != "Cellar" || comps.len() < index + 5 {
            continue;
        }
        if comps[index + 1].as_os_str() != "tidy" || comps[index + 3].as_os_str() != "bin" {
            continue;
        }
        let prefix: PathBuf = comps[..index].iter().collect();
        let linked = prefix.join("bin").join("tidy");
        let opt = prefix.join("opt").join("tidy").join("bin").join("tidy");
        if linked.is_file() {
            return linked;
        }
        if opt.is_file() {
            return opt;
        }
        return linked;
    }
    exe.to_path_buf()
}

pub fn render_plist(
    binary: &Path,
    program: &WatchProgram,
    log: &Path,
    label: &str,
    env_vars: &[(String, String)],
) -> String {
    let mut args = String::new();
    args.push_str(&xml_string(&binary.display().to_string()));
    args.push_str(&xml_string("watch"));
    args.push_str(&xml_string(&program.folder.display().to_string()));
    if let Some(rules) = &program.rules {
        args.push_str(&xml_string("--rules"));
        args.push_str(&xml_string(&rules.display().to_string()));
    }
    if let Some(older) = &program.older_than {
        args.push_str(&xml_string("--older-than"));
        args.push_str(&xml_string(older));
    }

    let mut env_block = String::new();
    if !env_vars.is_empty() {
        env_block.push_str("  <key>EnvironmentVariables</key>\n  <dict>\n");
        for (key, value) in env_vars {
            env_block.push_str(&format!(
                "    <key>{}</key>\n    <string>{}</string>\n",
                xml_escape(key),
                xml_escape(value)
            ));
        }
        env_block.push_str("  </dict>\n");
    }

    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key>
  <string>{label}</string>
  <key>ProgramArguments</key>
  <array>
{args}  </array>
  <key>RunAtLoad</key>
  <true/>
  <key>KeepAlive</key>
  <true/>
  <key>StandardOutPath</key>
  <string>{log}</string>
  <key>StandardErrorPath</key>
  <string>{log}</string>
  <key>ProcessType</key>
  <string>Background</string>
{env} </dict>
</plist>
"#,
        label = xml_escape(label),
        args = args,
        log = xml_escape(&log.display().to_string()),
        env = env_block,
    )
}

fn xml_string(text: &str) -> String {
    format!("    <string>{}</string>\n", xml_escape(text))
}

/// Snapshot of TIDY_* overrides so a launchd agent uses the same isolation
/// the installing process had (tests, the Mac smoke script, demos).
pub fn snapshot_override_env() -> Vec<(String, String)> {
    const KEYS: &[&str] = &[
        "TIDY_HOME",
        "TIDY_CONFIG_DIR",
        "TIDY_DATA_DIR",
        "TIDY_FAKE_TRASH",
        "TIDY_WATCH_SETTLE_MS",
    ];
    let mut out = Vec::new();
    for key in KEYS {
        if let Ok(value) = std::env::var(key) {
            if !value.is_empty() {
                out.push(((*key).to_string(), value));
            }
        }
    }
    out
}

/// Make `--rules` absolute so the agent still finds the file after launchd
/// starts with a different working directory.
pub fn absolute_rules_path(path: &Path) -> Result<PathBuf, Error> {
    if !path.exists() {
        return Err(Error::msg(format!(
            "rules file not found: {}",
            path.display()
        )));
    }
    if !path.is_file() {
        return Err(Error::msg(format!(
            "rules path is not a file: {}",
            path.display()
        )));
    }
    fs::canonicalize(path)
        .map_err(|err| Error::io(format!("resolving rules file {}", path.display()), err))
}

/// Write the plist and load it. Refuses to replace a different plist unless
/// `force` is set. Returns the human-readable summary.
pub fn install(
    paths: &LaunchPaths,
    program: &WatchProgram,
    env_vars: &[(String, String)],
    force: bool,
) -> Result<String, Error> {
    let plist = plist_path_for(&paths.home, &paths.label);
    let log = log_path(&paths.home);
    let xml = render_plist(&paths.binary, program, &log, &paths.label, env_vars);
    if let Some(parent) = plist.parent() {
        fs::create_dir_all(parent)
            .map_err(|err| Error::io(format!("creating {}", parent.display()), err))?;
    }
    if plist.is_file() {
        let existing = fs::read_to_string(&plist)
            .map_err(|err| Error::io(format!("reading {}", plist.display()), err))?;
        if existing != xml && !force {
            return Err(Error::msg(diff_hint(&plist, &existing, &xml)));
        }
    } else if plist.exists() {
        return Err(Error::msg(format!(
            "{} exists and is not a file",
            plist.display()
        )));
    }
    fs::write(&plist, &xml)
        .map_err(|err| Error::io(format!("writing {}", plist.display()), err))?;

    let domain = format!("gui/{}", paths.uid);
    let target = format!("{domain}/{}", paths.label);
    // A missing job makes bootout fail. That is the first install, not an error.
    let _ = run_launchctl(paths, &["bootout".to_string(), target.clone()])?;
    let boot = run_launchctl(
        paths,
        &["bootstrap".to_string(), domain, plist.display().to_string()],
    )?;
    if !boot.status.success() {
        let stderr = String::from_utf8_lossy(&boot.stderr);
        let stdout = String::from_utf8_lossy(&boot.stdout);
        return Err(Error::msg(format!(
            "launchctl bootstrap failed: {}{}",
            stderr.trim(),
            stdout.trim()
        )));
    }
    Ok(format!(
        "Using binary {}\nWrote {}\nLoaded {target}\nWatching {}\nLog {}\n",
        paths.binary.display(),
        plist.display(),
        program.folder.display(),
        log.display()
    ))
}

/// Boot the job out, then delete the plist tidy installed.
pub fn uninstall(paths: &LaunchPaths) -> Result<String, Error> {
    let plist = plist_path_for(&paths.home, &paths.label);
    let target = format!("gui/{}/{}", paths.uid, paths.label);
    let _ = run_launchctl(paths, &["bootout".to_string(), target.clone()])?;
    if plist.is_file() {
        fs::remove_file(&plist)
            .map_err(|err| Error::io(format!("removing {}", plist.display()), err))?;
        Ok(format!("Unloaded {target}\nRemoved {}\n", plist.display()))
    } else if plist.exists() {
        Err(Error::msg(format!(
            "{} exists and is not a file",
            plist.display()
        )))
    } else {
        Ok(format!(
            "Unloaded {target}\nNo LaunchAgent plist at {}\n",
            plist.display()
        ))
    }
}

pub fn status(paths: &LaunchPaths) -> Result<LaunchStatus, Error> {
    let plist = plist_path_for(&paths.home, &paths.label);
    let log = log_path(&paths.home);
    let parsed = if plist.is_file() {
        let text = fs::read_to_string(&plist)
            .map_err(|err| Error::io(format!("reading {}", plist.display()), err))?;
        parse_watch_program(&text)
    } else {
        ParsedWatchProgram::default()
    };
    let target = format!("gui/{}/{}", paths.uid, paths.label);
    let output = run_launchctl(paths, &["print".to_string(), target])?;
    let job = parse_launchctl_print(
        output.status.success(),
        &String::from_utf8_lossy(&output.stdout),
    );
    Ok(LaunchStatus {
        plist_path: plist,
        plist_exists: plist_path_for(&paths.home, &paths.label).is_file(),
        binary: parsed.binary,
        folder: parsed.folder,
        rules: parsed.rules,
        older_than: parsed.older_than,
        arguments: parsed.arguments,
        job,
        log_path: log.clone(),
        log_tail: log_tail(&log, 8)?,
    })
}

pub fn render_status(status: &LaunchStatus, color: bool) -> String {
    let mut out = String::new();
    let presence = if status.plist_exists {
        "present"
    } else {
        "absent"
    };
    line(
        &mut out,
        &format!("Plist   {} ({presence})", status.plist_path.display()),
    );
    line(
        &mut out,
        &format!(
            "Binary  {}",
            status.binary.as_deref().unwrap_or("(unknown)")
        ),
    );
    line(
        &mut out,
        &format!(
            "Folder  {}",
            status.folder.as_deref().unwrap_or("(unknown)")
        ),
    );
    line(
        &mut out,
        &format!("Rules   {}", status.rules.as_deref().unwrap_or("(default)")),
    );
    line(
        &mut out,
        &format!(
            "Age     {}",
            status.older_than.as_deref().unwrap_or("(none)")
        ),
    );
    if !status.arguments.is_empty() {
        line(&mut out, &format!("Args    {}", status.arguments.join(" ")));
    }
    let job = if !status.job.loaded {
        "not loaded".to_string()
    } else {
        let state = status.job.state.as_deref().unwrap_or("loaded");
        let pid = status
            .job
            .pid
            .map(|pid| format!("pid {pid}"))
            .unwrap_or_else(|| "no pid".to_string());
        let code = status
            .job
            .last_exit_code
            .map(|code| format!("last exit {code}"))
            .unwrap_or_else(|| "no exit code".to_string());
        format!("{state} ({pid}, {code})")
    };
    let job_code = if status.job.loaded { "32" } else { "33" };
    line(&mut out, &format!("Job     {}", dye(&job, color, job_code)));
    line(&mut out, &format!("Log     {}", status.log_path.display()));
    if !status.log_tail.is_empty() {
        out.push('\n');
        line(&mut out, &dye("Log tail", color, "1"));
        for entry in &status.log_tail {
            line(&mut out, &format!("  {entry}"));
        }
    }
    out
}

pub fn render_status_json(status: &LaunchStatus) -> Result<String, Error> {
    let value = serde_json::json!({
        "version": 1,
        "plist": status.plist_path.display().to_string(),
        "plist_exists": status.plist_exists,
        "binary": status.binary,
        "folder": status.folder,
        "rules": status.rules,
        "older_than": status.older_than,
        "arguments": status.arguments,
        "loaded": status.job.loaded,
        "state": status.job.state,
        "pid": status.job.pid,
        "last_exit_code": status.job.last_exit_code,
        "log_path": status.log_path.display().to_string(),
        "log_tail": status.log_tail,
    });
    serde_json::to_string_pretty(&value).map_err(|err| Error::msg(format!("json: {err}")))
}

/// Parse `launchctl print` stdout. A failed command means the job is not loaded.
pub fn parse_launchctl_print(command_ok: bool, stdout: &str) -> JobInfo {
    if !command_ok {
        return JobInfo {
            loaded: false,
            state: None,
            pid: None,
            last_exit_code: None,
        };
    }
    let mut state = None;
    let mut pid = None;
    let mut last_exit_code = None;
    for line in stdout.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("state = ") {
            if state.is_none() {
                state = Some(rest.trim().to_string());
            }
        } else if let Some(rest) = line.strip_prefix("pid = ") {
            if pid.is_none() {
                pid = rest.trim().parse().ok();
            }
        } else if let Some(rest) = line.strip_prefix("last exit code = ") {
            if last_exit_code.is_none() {
                last_exit_code = rest.trim().parse().ok();
            }
        }
    }
    JobInfo {
        loaded: true,
        state,
        pid,
        last_exit_code,
    }
}

pub fn program_from_plist(xml: &str) -> (Option<String>, Option<String>) {
    let parsed = parse_watch_program(xml);
    (parsed.binary, parsed.folder)
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ParsedWatchProgram {
    pub binary: Option<String>,
    pub folder: Option<String>,
    pub rules: Option<String>,
    pub older_than: Option<String>,
    pub arguments: Vec<String>,
}

/// Read ProgramArguments from an installed plist.
pub fn parse_watch_program(xml: &str) -> ParsedWatchProgram {
    let args = plist_array_after_key(xml, "ProgramArguments");
    let mut parsed = ParsedWatchProgram {
        arguments: args.clone(),
        ..ParsedWatchProgram::default()
    };
    if args.is_empty() {
        return parsed;
    }
    parsed.binary = args.first().cloned();
    let rest = if args.get(1).map(String::as_str) == Some("watch") {
        &args[2..]
    } else {
        &args[1..]
    };
    let mut i = 0;
    while i < rest.len() {
        match rest[i].as_str() {
            "--rules" => {
                if let Some(value) = rest.get(i + 1) {
                    parsed.rules = Some(value.clone());
                    i += 2;
                    continue;
                }
            }
            "--older-than" => {
                if let Some(value) = rest.get(i + 1) {
                    parsed.older_than = Some(value.clone());
                    i += 2;
                    continue;
                }
            }
            flag if flag.starts_with('-') => {
                i += 1;
                continue;
            }
            _ => {
                if parsed.folder.is_none() {
                    parsed.folder = Some(rest[i].clone());
                }
            }
        }
        i += 1;
    }
    parsed
}

fn plist_array_after_key(xml: &str, key: &str) -> Vec<String> {
    let marker = format!("<key>{key}</key>");
    let Some(start) = xml.find(&marker) else {
        return Vec::new();
    };
    let rest = &xml[start + marker.len()..];
    let Some(array_start) = rest.find("<array>") else {
        return Vec::new();
    };
    let rest = &rest[array_start + "<array>".len()..];
    let Some(array_end) = rest.find("</array>") else {
        return Vec::new();
    };
    xml_strings(&rest[..array_end])
}

fn xml_strings(xml: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = xml;
    while let Some(start) = rest.find("<string>") {
        rest = &rest[start + "<string>".len()..];
        let Some(end) = rest.find("</string>") else {
            break;
        };
        out.push(xml_unescape(&rest[..end]));
        rest = &rest[end + "</string>".len()..];
    }
    out
}

fn xml_escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            _ => out.push(ch),
        }
    }
    out
}

fn xml_unescape(text: &str) -> String {
    text.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&amp;", "&")
}

fn diff_hint(path: &Path, old: &str, new: &str) -> String {
    let mut msg = format!(
        "{} already exists and differs from the plist tidy would write. Pass --force to replace it.",
        path.display()
    );
    let mut shown = 0;
    for (number, (before, after)) in old.lines().zip(new.lines()).enumerate() {
        if before != after {
            msg.push_str(&format!("\n  line {}: `{before}` → `{after}`", number + 1));
            shown += 1;
            if shown >= 6 {
                msg.push_str("\n  …");
                break;
            }
        }
    }
    if shown == 0 && old != new {
        msg.push_str("\n  the files differ in length or trailing whitespace");
    }
    msg
}

fn run_launchctl(paths: &LaunchPaths, args: &[String]) -> Result<std::process::Output, Error> {
    Command::new(&paths.launchctl)
        .args(args)
        .output()
        .map_err(|err| {
            Error::msg(format!(
                "could not run {}: {err}",
                paths.launchctl.display()
            ))
        })
}

fn log_tail(path: &Path, lines: usize) -> Result<Vec<String>, Error> {
    if !path.is_file() || lines == 0 {
        return Ok(Vec::new());
    }
    let mut file =
        File::open(path).map_err(|err| Error::io(format!("reading {}", path.display()), err))?;
    let len = file
        .metadata()
        .map_err(|err| Error::io(format!("reading {}", path.display()), err))?
        .len();
    let take = len.min(16 * 1024);
    let start = len - take;
    file.seek(SeekFrom::Start(start))
        .map_err(|err| Error::io(format!("reading {}", path.display()), err))?;
    let mut buf = String::new();
    file.read_to_string(&mut buf)
        .map_err(|err| Error::io(format!("reading {}", path.display()), err))?;
    let mut collected: Vec<String> = buf.lines().map(str::to_string).collect();
    if start > 0 && !collected.is_empty() {
        collected.remove(0);
    }
    if collected.len() > lines {
        collected = collected.split_off(collected.len() - lines);
    }
    Ok(collected)
}

fn dye(text: &str, enabled: bool, code: &str) -> String {
    if !enabled || text.is_empty() {
        return text.to_string();
    }
    format!("\u{1b}[{code}m{text}\u{1b}[0m")
}

fn line(out: &mut String, text: &str) {
    out.push_str(text);
    out.push('\n');
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plist_escapes_and_lists_the_watch_arguments() {
        let binary = Path::new("/opt/homebrew/bin/tidy");
        let folder = Path::new("/Users/a&b/Downloads <fun>");
        let log = Path::new("/Users/a&b/Library/Logs/tidy.log");
        let program = WatchProgram {
            folder: folder.to_path_buf(),
            rules: Some(PathBuf::from("/tmp/rules & <mine>.toml")),
            older_than: Some("30d".into()),
        };
        let xml = render_plist(
            binary,
            &program,
            log,
            LABEL,
            &[("TIDY_HOME".into(), "/Users/a&b".into())],
        );
        assert!(xml.contains("<key>Label</key>"));
        assert!(xml.contains("<string>com.github.jherin89.tidy</string>"));
        assert!(xml.contains("<key>RunAtLoad</key>"));
        assert!(xml.contains("<true/>"));
        assert!(xml.contains("<key>KeepAlive</key>"));
        assert!(xml.contains("<key>ProcessType</key>"));
        assert!(xml.contains("<string>Background</string>"));
        assert!(xml.contains("<string>/opt/homebrew/bin/tidy</string>"));
        assert!(xml.contains("<string>watch</string>"));
        assert!(xml.contains("<string>--rules</string>"));
        assert!(xml.contains("<string>--older-than</string>"));
        assert!(xml.contains("<string>30d</string>"));
        assert!(xml.contains("&amp;"));
        assert!(xml.contains("&lt;"));
        assert!(!xml.contains("Downloads <fun>"));
        assert!(!xml.contains("rules & <mine>"));
        assert!(xml.contains("rules &amp; &lt;mine&gt;.toml"));
        assert!(xml.contains("<key>EnvironmentVariables</key>"));
        assert!(xml.contains("<key>TIDY_HOME</key>"));
        let (bin, watched) = program_from_plist(&xml);
        assert_eq!(bin.as_deref(), Some("/opt/homebrew/bin/tidy"));
        assert_eq!(watched.as_deref(), Some("/Users/a&b/Downloads <fun>"));
        let parsed = parse_watch_program(&xml);
        assert_eq!(parsed.rules.as_deref(), Some("/tmp/rules & <mine>.toml"));
        assert_eq!(parsed.older_than.as_deref(), Some("30d"));
        assert!(xml.contains("StandardOutPath"));
        assert!(xml.contains("StandardErrorPath"));
    }

    #[test]
    fn cellar_paths_prefer_the_stable_prefix_link() {
        let cellar = Path::new("/opt/homebrew/Cellar/tidy/0.3.0/bin/tidy");
        assert_eq!(
            stable_brew_binary(cellar),
            PathBuf::from("/opt/homebrew/bin/tidy")
        );
        let intel = Path::new("/usr/local/Cellar/tidy/0.3.0/bin/tidy");
        assert_eq!(
            stable_brew_binary(intel),
            PathBuf::from("/usr/local/bin/tidy")
        );
        let plain = Path::new("/usr/local/bin/tidy");
        assert_eq!(stable_brew_binary(plain), plain);

        let tmp = tempfile::tempdir().unwrap();
        let exe = tmp.path().join("Cellar/tidy/0.3.0/bin/tidy");
        fs::create_dir_all(exe.parent().unwrap()).unwrap();
        fs::write(&exe, b"cellar").unwrap();
        let opt = tmp.path().join("opt/tidy/bin/tidy");
        fs::create_dir_all(opt.parent().unwrap()).unwrap();
        fs::write(&opt, b"opt").unwrap();
        assert_eq!(stable_brew_binary(&exe), opt);

        let linked = tmp.path().join("bin/tidy");
        fs::create_dir_all(linked.parent().unwrap()).unwrap();
        fs::write(&linked, b"bin").unwrap();
        assert_eq!(stable_brew_binary(&exe), linked);
    }

    #[test]
    fn parses_launchctl_print_output() {
        let running = r#"
gui/501/com.github.jherin89.tidy = {
	active count = 1
	path = /Users/me/Library/LaunchAgents/com.github.jherin89.tidy.plist
	state = running
	program = /opt/homebrew/bin/tidy
	pid = 4242
	last exit code = 0
}
"#;
        let job = parse_launchctl_print(true, running);
        assert!(job.loaded);
        assert_eq!(job.state.as_deref(), Some("running"));
        assert_eq!(job.pid, Some(4242));
        assert_eq!(job.last_exit_code, Some(0));

        let stopped = "state = not running\nlast exit code = 78\n";
        let job = parse_launchctl_print(true, stopped);
        assert!(job.loaded);
        assert_eq!(job.state.as_deref(), Some("not running"));
        assert_eq!(job.pid, None);
        assert_eq!(job.last_exit_code, Some(78));

        let never = "state = running\nlast exit code = (never exited)\n";
        let job = parse_launchctl_print(true, never);
        assert_eq!(job.last_exit_code, None);

        let missing = parse_launchctl_print(false, "Could not find service\n");
        assert!(!missing.loaded);
        assert!(missing.state.is_none());
    }

    fn fake_launchctl(dir: &Path, body: &str) -> (PathBuf, PathBuf) {
        let log = dir.join("launchctl.log");
        let script = dir.join("launchctl");
        let mut text = format!("#!/bin/sh\nprintf '%s\\n' \"$*\" >> '{}'\n", log.display());
        text.push_str(body);
        if !body.ends_with('\n') {
            text.push('\n');
        }
        fs::write(&script, text).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = fs::metadata(&script).unwrap().permissions();
            perms.set_mode(0o755);
            fs::set_permissions(&script, perms).unwrap();
        }
        (script, log)
    }

    fn paths(home: &Path, launchctl: PathBuf) -> LaunchPaths {
        LaunchPaths {
            home: home.to_path_buf(),
            uid: 501,
            launchctl,
            binary: PathBuf::from("/opt/homebrew/bin/tidy"),
            label: LABEL.to_string(),
        }
    }

    fn watch_at(folder: &Path) -> WatchProgram {
        WatchProgram {
            folder: folder.to_path_buf(),
            rules: None,
            older_than: None,
        }
    }

    #[test]
    fn install_refuses_a_different_plist_until_forced() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join("home");
        fs::create_dir_all(&home).unwrap();
        let (script, log) = fake_launchctl(
            tmp.path(),
            "if [ \"$1\" = bootout ]; then exit 3; fi\nexit 0\n",
        );
        let paths = paths(&home, script);
        let folder = PathBuf::from("/tmp/Downloads");
        let summary = install(&paths, &watch_at(&folder), &[], false).unwrap();
        assert!(summary.contains("/opt/homebrew/bin/tidy"), "{summary}");
        let plist = plist_path(&home);
        let first = fs::read_to_string(&plist).unwrap();
        assert!(first.contains("<string>watch</string>"));

        fs::write(&plist, "different\n").unwrap();
        let err = install(&paths, &watch_at(&folder), &[], false).unwrap_err();
        assert!(err.to_string().contains("--force"), "{err}");
        assert_eq!(fs::read_to_string(&plist).unwrap(), "different\n");

        install(&paths, &watch_at(&folder), &[], true).unwrap();
        let replaced = fs::read_to_string(&plist).unwrap();
        assert!(replaced.contains("/tmp/Downloads"));
        let recorded = fs::read_to_string(&log).unwrap();
        assert!(
            recorded.contains("bootout gui/501/com.github.jherin89.tidy"),
            "{recorded}"
        );
        assert!(
            recorded.contains(&format!("bootstrap gui/501 {}", plist.display())),
            "{recorded}"
        );
    }

    #[test]
    fn uninstall_and_status_call_launchctl() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join("home");
        let fixture = "state = running\npid = 99\nlast exit code = 0\n";
        let body = format!(
            "if [ \"$1\" = print ]; then\ncat <<'EOF'\n{fixture}EOF\nfi\nexit 0\n",
            fixture = fixture
        );
        let (script, log) = fake_launchctl(tmp.path(), &body);
        let paths = paths(&home, script);
        let folder = home.join("Downloads");
        install(
            &paths,
            &WatchProgram {
                folder: folder.clone(),
                rules: Some(PathBuf::from("/tmp/custom.toml")),
                older_than: Some("7d".into()),
            },
            &[],
            false,
        )
        .unwrap();
        let log_file = log_path(&home);
        fs::create_dir_all(log_file.parent().unwrap()).unwrap();
        fs::write(&log_file, "one\ntwo\nthree\n").unwrap();

        let status = status(&paths).unwrap();
        assert!(status.plist_exists);
        assert_eq!(status.folder.as_deref(), Some(folder.to_str().unwrap()));
        assert_eq!(status.binary.as_deref(), Some("/opt/homebrew/bin/tidy"));
        assert_eq!(status.rules.as_deref(), Some("/tmp/custom.toml"));
        assert_eq!(status.older_than.as_deref(), Some("7d"));
        let text = render_status(&status, false);
        assert!(text.contains("/tmp/custom.toml"), "{text}");
        assert!(text.contains("7d"), "{text}");
        assert!(text.contains("--rules"), "{text}");
        assert!(status.job.loaded);
        assert_eq!(status.job.state.as_deref(), Some("running"));
        assert_eq!(status.job.pid, Some(99));
        assert_eq!(status.job.last_exit_code, Some(0));
        assert_eq!(status.log_tail, vec!["one", "two", "three"]);
        let text = render_status(&status, false);
        assert!(text.contains("running"), "{text}");
        assert!(text.contains("pid 99"), "{text}");
        let json: serde_json::Value =
            serde_json::from_str(&render_status_json(&status).unwrap()).unwrap();
        assert_eq!(json["state"], "running");
        assert_eq!(json["pid"], 99);

        let summary = uninstall(&paths).unwrap();
        assert!(summary.contains("Removed"), "{summary}");
        assert!(!plist_path(&home).exists());
        let recorded = fs::read_to_string(&log).unwrap();
        assert!(
            recorded.contains("bootout gui/501/com.github.jherin89.tidy"),
            "{recorded}"
        );
        assert!(
            recorded.contains("print gui/501/com.github.jherin89.tidy"),
            "{recorded}"
        );
    }
}
