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
                        rules: rules.or(cli.rules.clone()),
                        older_than: older_than.or(cli.older_than.clone()),
                        install,
                        uninstall,
                        status,
                        print,
                        force,
                        json: json || cli.json,
                        apply: cli.apply,
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
        Some(Command::Schedule {
            path,
            rules,
            older_than,
            at,
            install,
            uninstall,
            status,
            print,
            force,
            json,
        }) => schedule_service(
            &env,
            ScheduleRequest {
                path,
                rules: rules.or(cli.rules.clone()),
                older_than: older_than.or(cli.older_than.clone()),
                at,
                install,
                uninstall,
                status,
                print,
                force,
                json: json || cli.json,
                apply: cli.apply,
            },
        ),
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
    rules: Option<PathBuf>,
    older_than: Option<String>,
    install: bool,
    uninstall: bool,
    status: bool,
    print: bool,
    force: bool,
    json: bool,
    apply: bool,
}

fn watch_service(env: &Env, request: WatchRequest) -> Result<u8, Error> {
    let WatchRequest {
        path,
        rules,
        older_than,
        install,
        uninstall,
        status,
        print,
        force,
        json,
        apply,
    } = request;
    validate_agent_flags(&AgentFlags {
        command: "watch",
        install,
        uninstall,
        status,
        print,
        force,
        json,
        apply,
        has_rules: rules.is_some(),
        has_older_than: older_than.is_some(),
        has_folder: path.is_some(),
        has_at: false,
        print_requires_install: true,
        extra_flag_words: "",
        apply_message: "`--apply` is not stored in the launchd agent; `tidy watch` always applies",
    })?;

    let program = if install {
        Some(install_program(path, rules, older_than, env)?)
    } else {
        None
    };

    if print {
        let binary = launchd::resolved_tidy_binary()?;
        let program = program.expect("install program");
        let xml = launchd::render_plist(
            &binary,
            &program,
            &launchd::log_path(&env.home),
            &launchd::job_label()?,
            &launchd::snapshot_override_env(),
        );
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
        label: launchd::job_label()?,
    };
    if install {
        let program = program.expect("install program");
        print!(
            "{}",
            launchd::install(&paths, &program, &launchd::snapshot_override_env(), force,)?
        );
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

struct ScheduleRequest {
    path: Option<PathBuf>,
    rules: Option<PathBuf>,
    older_than: Option<String>,
    at: Option<String>,
    install: bool,
    uninstall: bool,
    status: bool,
    print: bool,
    force: bool,
    json: bool,
    apply: bool,
}

fn schedule_service(env: &Env, request: ScheduleRequest) -> Result<u8, Error> {
    let ScheduleRequest {
        path,
        rules,
        older_than,
        at,
        install,
        uninstall,
        status,
        print,
        force,
        json,
        apply,
    } = request;
    validate_agent_flags(&AgentFlags {
        command: "schedule",
        install,
        uninstall,
        status,
        print,
        force,
        json,
        apply,
        has_rules: rules.is_some(),
        has_older_than: older_than.is_some(),
        has_folder: path.is_some(),
        has_at: at.is_some(),
        print_requires_install: false,
        extra_flag_words: ", --at",
        apply_message:
            "`--apply` is already what the daily job runs; do not pass it to `tidy schedule`",
    })?;

    let program = if install || print {
        Some(install_daily_program(path, rules, older_than, at, env)?)
    } else {
        None
    };

    if print {
        let binary = launchd::resolved_tidy_binary()?;
        let program = program.expect("daily program");
        let xml = launchd::render_daily_plist(
            &binary,
            &program,
            &launchd::log_path_for_label(&env.home, &launchd::daily_job_label()?),
            &launchd::daily_job_label()?,
            &launchd::snapshot_override_env(),
        );
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
        label: launchd::daily_job_label()?,
    };
    if install {
        let program = program.expect("daily program");
        print!(
            "{}",
            launchd::install_daily(&paths, &program, &launchd::snapshot_override_env(), force,)?
        );
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

struct PreparedAgent {
    folder: PathBuf,
    rules: Option<PathBuf>,
    older_than: Option<String>,
}

fn prepare_agent(
    path: Option<PathBuf>,
    rules: Option<PathBuf>,
    older_than: Option<String>,
    env: &Env,
) -> Result<PreparedAgent, Error> {
    if let Some(text) = older_than.as_deref() {
        parse_duration(text).map_err(|err| Error::msg(format!("--older-than: {err}")))?;
    }
    let rules = match rules {
        Some(path) => Some(launchd::absolute_rules_path(&path)?),
        None => None,
    };
    Ok(PreparedAgent {
        folder: service_folder(path, env)?,
        rules,
        older_than,
    })
}

fn install_daily_program(
    path: Option<PathBuf>,
    rules: Option<PathBuf>,
    older_than: Option<String>,
    at: Option<String>,
    env: &Env,
) -> Result<launchd::DailyProgram, Error> {
    let (hour, minute) = crate::parse::parse_clock(at.as_deref().unwrap_or("10:00"))
        .map_err(|err| Error::msg(format!("--at: {err}")))?;
    let prepared = prepare_agent(path, rules, older_than, env)?;
    Ok(launchd::DailyProgram {
        folder: prepared.folder,
        rules: prepared.rules,
        older_than: prepared.older_than,
        hour,
        minute,
    })
}

fn install_program(
    path: Option<PathBuf>,
    rules: Option<PathBuf>,
    older_than: Option<String>,
    env: &Env,
) -> Result<launchd::WatchProgram, Error> {
    let prepared = prepare_agent(path, rules, older_than, env)?;
    Ok(launchd::WatchProgram {
        folder: prepared.folder,
        rules: prepared.rules,
        older_than: prepared.older_than,
    })
}

/// Flags that mean the same thing for `tidy watch` and `tidy schedule`.
///
/// `print_requires_install` keeps the intentional difference: `watch --print`
/// must be paired with `--install`, while `schedule --print` works alone.
/// `--at` is schedule-only (`has_at`). Error text stays command-specific.
struct AgentFlags {
    command: &'static str,
    install: bool,
    uninstall: bool,
    status: bool,
    print: bool,
    force: bool,
    json: bool,
    apply: bool,
    has_rules: bool,
    has_older_than: bool,
    has_folder: bool,
    has_at: bool,
    print_requires_install: bool,
    /// Inserted after `--older-than` in uninstall/status errors. Watch is
    /// empty. Schedule is `, --at`.
    extra_flag_words: &'static str,
    apply_message: &'static str,
}

fn validate_agent_flags(flags: &AgentFlags) -> Result<(), Error> {
    if flags.print && flags.print_requires_install && !flags.install {
        return Err(Error::msg(format!(
            "`--print` is only valid with `tidy {} --install --print`",
            flags.command
        )));
    }
    if flags.print && !flags.print_requires_install && (flags.uninstall || flags.status) {
        return Err(Error::msg(format!(
            "`--print` is only valid alone or with `tidy {} --install --print`",
            flags.command
        )));
    }
    let modes = [flags.install, flags.uninstall, flags.status]
        .iter()
        .filter(|flag| **flag)
        .count();
    if modes > 1 {
        return Err(Error::msg(
            "pass only one of --install, --uninstall, or --status",
        ));
    }
    if flags.force && !flags.install {
        return Err(Error::msg(format!(
            "`--force` is only valid with `tidy {} --install`",
            flags.command
        )));
    }
    let print_is_a_mode = flags.print && !flags.print_requires_install;
    if !flags.install && !flags.uninstall && !flags.status && !print_is_a_mode {
        if flags.print_requires_install {
            return Err(Error::msg(format!(
                "pass --install, --uninstall, or --status (`tidy {} --help`)",
                flags.command
            )));
        }
        return Err(Error::msg(format!(
            "pass --install, --uninstall, --status, or --print (`tidy {} --help`)",
            flags.command
        )));
    }
    if flags.apply {
        return Err(Error::msg(flags.apply_message));
    }
    if flags.json && (flags.install || flags.uninstall || flags.print) {
        return Err(Error::msg(format!(
            "`--json` cannot be stored in the launchd agent (its output goes to the log file). Use `tidy {} --status --json`.",
            flags.command
        )));
    }
    let extras = flags.has_rules || flags.has_older_than || flags.has_folder || flags.has_at;
    if flags.uninstall && extras {
        return Err(Error::msg(format!(
            "`tidy {} --uninstall` does not take --rules, --older-than{}, or a folder; those flags belong on --install",
            flags.command, flags.extra_flag_words
        )));
    }
    if flags.status && extras {
        return Err(Error::msg(format!(
            "`tidy {} --status` reads the installed plist; it does not take --rules, --older-than{}, or a folder",
            flags.command, flags.extra_flag_words
        )));
    }
    Ok(())
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

#[cfg(test)]
mod tests {
    use super::*;

    fn flags(command: &'static str) -> AgentFlags {
        let schedule = command == "schedule";
        AgentFlags {
            command,
            install: false,
            uninstall: false,
            status: false,
            print: false,
            force: false,
            json: false,
            apply: false,
            has_rules: false,
            has_older_than: false,
            has_folder: false,
            has_at: false,
            print_requires_install: !schedule,
            extra_flag_words: if schedule { ", --at" } else { "" },
            apply_message: if schedule {
                "`--apply` is already what the daily job runs; do not pass it to `tidy schedule`"
            } else {
                "`--apply` is not stored in the launchd agent; `tidy watch` always applies"
            },
        }
    }

    fn err(flags: &AgentFlags) -> String {
        validate_agent_flags(flags).unwrap_err().to_string()
    }

    #[test]
    fn shared_validation_rejects_the_same_bad_flag_combos() {
        for command in ["watch", "schedule"] {
            let mut both = flags(command);
            both.install = true;
            both.uninstall = true;
            assert_eq!(
                err(&both),
                "pass only one of --install, --uninstall, or --status"
            );

            let mut forced = flags(command);
            forced.force = true;
            assert_eq!(
                err(&forced),
                format!("`--force` is only valid with `tidy {command} --install`")
            );

            let mut apply = flags(command);
            apply.install = true;
            apply.apply = true;
            let message = err(&apply);
            assert!(message.contains("`--apply`"), "{message}");
            assert!(message.contains(command), "{message}");

            let mut json_install = flags(command);
            json_install.install = true;
            json_install.json = true;
            assert!(
                err(&json_install).contains(&format!("Use `tidy {command} --status --json`.")),
                "{}",
                err(&json_install)
            );

            let mut uninstall_rules = flags(command);
            uninstall_rules.uninstall = true;
            uninstall_rules.has_rules = true;
            let message = err(&uninstall_rules);
            assert!(message.contains("--rules"), "{message}");
            assert!(message.contains("--uninstall"), "{message}");
            assert!(
                message.contains("those flags belong on --install"),
                "{message}"
            );

            let mut status_folder = flags(command);
            status_folder.status = true;
            status_folder.has_folder = true;
            status_folder.has_older_than = true;
            let message = err(&status_folder);
            assert!(message.contains("--status"), "{message}");
            assert!(message.contains("--older-than"), "{message}");
            assert!(message.contains("or a folder"), "{message}");

            let mut status_json = flags(command);
            status_json.status = true;
            status_json.json = true;
            assert!(validate_agent_flags(&status_json).is_ok());
        }

        let mut watch_print = flags("watch");
        watch_print.print = true;
        assert_eq!(
            err(&watch_print),
            "`--print` is only valid with `tidy watch --install --print`"
        );
        watch_print.install = true;
        assert!(validate_agent_flags(&watch_print).is_ok());

        let mut schedule_print = flags("schedule");
        schedule_print.print = true;
        assert!(validate_agent_flags(&schedule_print).is_ok());
        schedule_print.install = true;
        assert!(validate_agent_flags(&schedule_print).is_ok());
        schedule_print.uninstall = true;
        assert_eq!(
            err(&schedule_print),
            "`--print` is only valid alone or with `tidy schedule --install --print`"
        );

        let mut schedule_at = flags("schedule");
        schedule_at.uninstall = true;
        schedule_at.has_at = true;
        assert!(err(&schedule_at).contains("--at"), "{}", err(&schedule_at));

        let watch_plain = flags("watch");
        assert_eq!(
            err(&watch_plain),
            "pass --install, --uninstall, or --status (`tidy watch --help`)"
        );
        let schedule_plain = flags("schedule");
        assert_eq!(
            err(&schedule_plain),
            "pass --install, --uninstall, --status, or --print (`tidy schedule --help`)"
        );
    }
}
