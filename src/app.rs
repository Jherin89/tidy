//! Command dispatch. The binary's `main` prints fatal errors and exits.

use std::io::{self, IsTerminal};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use crate::apply::apply_plan;
use crate::cli::{Cli, Command};
use crate::env::{contract_home, Env};
use crate::error::Error;
use crate::explain;
use crate::launchd::{self, LaunchPaths};
use crate::parse::parse_duration;
use crate::plan::{plan_folder, Plan};
use crate::report::{self, render_plan};
use crate::rules::{self, load_rules};
use crate::scan::resolve_folder;
use crate::trash::{self, Trasher};
use crate::undo::{self, UndoOutcome};
use crate::watch;

/// Run the CLI. `Ok(0)` is success. `Ok(1)` means the run finished but at
/// least one file failed, or a Trash item has to be restored by hand.
pub fn run(cli: Cli) -> Result<u8, Error> {
    let env = Env::from_process()?;
    let trasher = trash::from_env(&env);
    let macos = cfg!(target_os = "macos");

    match cli.command {
        Some(Command::Init { force }) => {
            init_rules(&env, None, force, macos)?;
            Ok(0)
        }
        Some(Command::Config { init, show, force }) => {
            if !init && !show {
                return Err(Error::msg(
                    "pass --init, --show, or both (`tidy config --help`)",
                ));
            }
            if init {
                init_rules(&env, None, force, macos)?;
            }
            if show {
                let json = cli.json;
                print_rules(&env, None, macos, json, color_enabled() && !json)?;
            }
            Ok(0)
        }
        Some(Command::Rules { rules, json }) => {
            let json = json || cli.json;
            print_rules(
                &env,
                rules.as_deref().or(cli.rules.as_deref()),
                macos,
                json,
                color_enabled() && !json,
            )?;
            Ok(0)
        }
        Some(Command::Undo { json }) => {
            let json = json || cli.json;
            undo(&env, trasher.as_ref(), json, color_enabled() && !json)
        }
        Some(Command::Explain { file, rules, json }) => {
            let json = json || cli.json;
            explain_cmd(
                &env,
                &file,
                rules.as_deref().or(cli.rules.as_deref()),
                macos,
                json,
            )
        }
        Some(Command::Watch {
            path,
            rules,
            older_than,
            install,
            uninstall,
            status,
            print,
            force,
            json,
        }) => {
            if print || install || uninstall || status || force {
                return watch_service(
                    &env,
                    WatchRequest {
                        path,
                        install,
                        uninstall,
                        status,
                        print,
                        force,
                        json: json || cli.json,
                    },
                );
            }
            let folder = resolve_folder(&folder_or_default(path, &env))?;
            let older = parse_older(older_than.as_deref().or(cli.older_than.as_deref()))?;
            let ruleset = load_rules(&env, rules.as_deref().or(cli.rules.as_deref()), macos)?;
            let settle = watch_settle()?;
            let json = json || cli.json;
            let color = color_enabled() && !json;
            if !json {
                println!("Watching {}", contract_home(&folder, &env.home));
                println!("Rules  {}", rules_label(&ruleset));
                println!(
                    "New files are tidied once their size stays the same for {}. Ctrl-C stops.",
                    format_settle(settle)
                );
            }
            watch::watch_folder(
                &env,
                &folder,
                &ruleset,
                older,
                settle,
                trasher.as_ref(),
                |plan| {
                    if json {
                        match report::render_plan_json(plan) {
                            Ok(text) => println!("{text}"),
                            Err(err) => eprintln!("tidy: {err}"),
                        }
                    } else {
                        print!("{}", render_plan(plan, &env, color));
                    }
                },
            )?;
            if !json {
                println!("tidy: stopped watching");
            }
            Ok(0)
        }
        None => {
            let folder = resolve_folder(&folder_or_default(cli.path.clone(), &env))?;
            let older = parse_older(cli.older_than.as_deref())?;
            let ruleset = load_rules(&env, cli.rules.as_deref(), macos)?;
            let mut plan = plan_folder(&env, &folder, &ruleset, older, SystemTime::now())?;
            if cli.apply {
                apply_plan(&env, &mut plan, trasher.as_ref())?;
            }
            let json = cli.json;
            emit_plan(&plan, &env, json, color_enabled() && !json)?;
            if plan.failure_count() > 0 {
                Ok(1)
            } else {
                Ok(0)
            }
        }
    }
}

fn explain_cmd(
    env: &Env,
    file: &Path,
    rules_path: Option<&Path>,
    macos: bool,
    json: bool,
) -> Result<u8, Error> {
    let ruleset = load_rules(env, rules_path, macos)?;
    let explanation = explain::explain_path(env, &ruleset, file, SystemTime::now())?;
    if json {
        println!("{}", explain::render_json(&explanation)?);
    } else {
        print!(
            "{}",
            explain::render(&explanation, env, color_enabled() && !json)
        );
    }
    Ok(0)
}

struct WatchRequest {
    path: Option<PathBuf>,
    install: bool,
    uninstall: bool,
    status: bool,
    print: bool,
    force: bool,
    json: bool,
}

fn watch_service(env: &Env, request: WatchRequest) -> Result<u8, Error> {
    let WatchRequest {
        path,
        install,
        uninstall,
        status,
        print,
        force,
        json,
    } = request;
    if print && !install {
        return Err(Error::msg(
            "`--print` is only valid with `tidy watch --install --print`",
        ));
    }
    let modes = [install, uninstall, status]
        .iter()
        .filter(|flag| **flag)
        .count();
    if modes > 1 {
        return Err(Error::msg(
            "pass only one of --install, --uninstall, or --status",
        ));
    }
    if force && !install {
        return Err(Error::msg(
            "`--force` is only valid with `tidy watch --install`",
        ));
    }
    if !install && !uninstall && !status {
        return Err(Error::msg(
            "pass --install, --uninstall, or --status (`tidy watch --help`)",
        ));
    }

    if print {
        let binary = launchd::resolved_tidy_binary()?;
        let folder = service_folder(path, env)?;
        let xml = launchd::render_plist(&binary, &folder, &launchd::log_path(&env.home));
        eprintln!("Using binary {}", binary.display());
        print!("{xml}");
        return Ok(0);
    }

    launchd::ensure_supported()?;
    let paths = LaunchPaths {
        home: env.home.clone(),
        uid: launchd::current_uid()?,
        launchctl: launchd::launchctl_program(),
        binary: launchd::resolved_tidy_binary()?,
    };
    if install {
        let folder = service_folder(path, env)?;
        print!("{}", launchd::install(&paths, &folder, force)?);
        return Ok(0);
    }
    if uninstall {
        print!("{}", launchd::uninstall(&paths)?);
        return Ok(0);
    }
    let report = launchd::status(&paths)?;
    if json {
        println!("{}", launchd::render_status_json(&report)?);
    } else {
        print!(
            "{}",
            launchd::render_status(&report, color_enabled() && !json)
        );
    }
    Ok(0)
}

/// Folder embedded in a LaunchAgent. Missing folders are allowed so the agent
/// can be installed before the directory exists. An existing non-directory fails.
fn service_folder(path: Option<PathBuf>, env: &Env) -> Result<PathBuf, Error> {
    let raw = path.unwrap_or_else(|| env.downloads());
    if raw.exists() {
        return resolve_folder(&raw);
    }
    if raw.is_absolute() {
        return Ok(raw);
    }
    let cwd =
        std::env::current_dir().map_err(|err| Error::io("reading the working directory", err))?;
    Ok(cwd.join(raw))
}

fn init_rules(
    env: &Env,
    override_path: Option<&Path>,
    force: bool,
    macos: bool,
) -> Result<(), Error> {
    let path = match override_path {
        Some(path) => path.to_path_buf(),
        None => env.rules_init_path(macos),
    };
    rules::write_default_rules(&path, force)?;
    println!("Wrote {}", path.display());
    Ok(())
}

fn print_rules(
    env: &Env,
    override_path: Option<&Path>,
    macos: bool,
    json: bool,
    color: bool,
) -> Result<(), Error> {
    let ruleset = load_rules(env, override_path, macos)?;
    if json {
        println!("{}", rules::render_rules_json(&ruleset)?);
    } else {
        print!("{}", rules::render_rules(&ruleset, color));
    }
    Ok(())
}

fn undo(env: &Env, trasher: &dyn Trasher, json: bool, color: bool) -> Result<u8, Error> {
    let report = undo::undo_latest(env, trasher)?;
    if json {
        println!("{}", report::render_undo_json(&report)?);
    } else {
        print!("{}", report::render_undo(&report, env, color));
    }
    let needs_attention = report
        .items
        .iter()
        .any(|item| matches!(item.outcome, UndoOutcome::Failed | UndoOutcome::Manual));
    if needs_attention {
        Ok(1)
    } else {
        Ok(0)
    }
}

fn emit_plan(plan: &Plan, env: &Env, json: bool, color: bool) -> Result<(), Error> {
    if json {
        println!("{}", report::render_plan_json(plan)?);
    } else {
        print!("{}", render_plan(plan, env, color));
    }
    Ok(())
}

fn folder_or_default(path: Option<PathBuf>, env: &Env) -> PathBuf {
    path.unwrap_or_else(|| env.downloads())
}

fn parse_older(value: Option<&str>) -> Result<Option<Duration>, Error> {
    match value {
        None => Ok(None),
        Some(text) => parse_duration(text)
            .map(Some)
            .map_err(|err| Error::msg(format!("--older-than: {err}"))),
    }
}

fn rules_label(rules: &rules::Ruleset) -> String {
    match rules.source {
        rules::RulesSource::BuiltIn => "built-in defaults".to_string(),
        rules::RulesSource::File => rules
            .path
            .as_ref()
            .map(|path| path.display().to_string())
            .unwrap_or_else(|| "rules file".to_string()),
    }
}

fn format_settle(settle: Duration) -> String {
    if settle.as_secs() > 0 && settle.subsec_nanos() == 0 {
        format!("{}s", settle.as_secs())
    } else {
        format!("{}ms", settle.as_millis())
    }
}

fn watch_settle() -> Result<Duration, Error> {
    match std::env::var("TIDY_WATCH_SETTLE_MS") {
        Ok(text) if !text.is_empty() => {
            let ms: u64 = text.parse().map_err(|_| {
                Error::msg(format!(
                    "TIDY_WATCH_SETTLE_MS must be a number of milliseconds, got `{text}`"
                ))
            })?;
            if ms == 0 {
                return Err(Error::msg("TIDY_WATCH_SETTLE_MS must be greater than zero"));
            }
            Ok(Duration::from_millis(ms))
        }
        _ => Ok(Duration::from_secs(2)),
    }
}

fn color_enabled() -> bool {
    // https://bixense.com/clicolors/ : CLICOLOR_FORCE wins, even when piped.
    if std::env::var("CLICOLOR_FORCE").is_ok_and(|v| !v.is_empty() && v != "0") {
        return true;
    }
    if std::env::var_os("NO_COLOR").is_some() {
        return false;
    }
    if std::env::var("CLICOLOR").ok().as_deref() == Some("0") {
        return false;
    }
    if std::env::var("TERM").ok().as_deref() == Some("dumb") {
        return false;
    }
    io::stdout().is_terminal()
}
