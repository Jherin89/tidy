//! Command dispatch. The binary's `main` prints fatal errors and exits.

use std::io::{self, IsTerminal};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use crate::apply::apply_plan;
use crate::cli::{Cli, Command};
use crate::env::{contract_home, Env};
use crate::error::Error;
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
    let color = color_enabled() && !cli.json;
    let macos = cfg!(target_os = "macos");

    match cli.command {
        Some(Command::Init { force }) => {
            init_rules(&env, cli.rules.as_deref(), force, macos)?;
            Ok(0)
        }
        Some(Command::Config { init, show, force }) => {
            if !init && !show {
                return Err(Error::msg(
                    "pass --init, --show, or both (`tidy config --help`)",
                ));
            }
            if init {
                init_rules(&env, cli.rules.as_deref(), force, macos)?;
            }
            if show {
                print_rules(&env, cli.rules.as_deref(), macos, cli.json, color)?;
            }
            Ok(0)
        }
        Some(Command::Rules) => {
            print_rules(&env, cli.rules.as_deref(), macos, cli.json, color)?;
            Ok(0)
        }
        Some(Command::Undo) => undo(&env, trasher.as_ref(), cli.json, color),
        Some(Command::Watch { path }) => {
            let folder = resolve_folder(&folder_or_default(path, &env))?;
            let older = parse_older(cli.older_than.as_deref())?;
            let ruleset = load_rules(&env, cli.rules.as_deref(), macos)?;
            let settle = watch_settle()?;
            if !cli.json {
                println!("Watching {}", contract_home(&folder, &env.home));
                println!("Rules  {}", rules_label(&ruleset));
                println!(
                    "New files are tidied once their size stays the same for {}. Ctrl-C stops.",
                    format_settle(settle)
                );
            }
            let json = cli.json;
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
            let folder = resolve_folder(&folder_or_default(cli.path, &env))?;
            let older = parse_older(cli.older_than.as_deref())?;
            let ruleset = load_rules(&env, cli.rules.as_deref(), macos)?;
            let mut plan = plan_folder(&env, &folder, &ruleset, older, SystemTime::now())?;
            if cli.apply {
                apply_plan(&env, &mut plan, trasher.as_ref())?;
            }
            emit_plan(&plan, &env, cli.json, color)?;
            if plan.failure_count() > 0 {
                Ok(1)
            } else {
                Ok(0)
            }
        }
    }
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
