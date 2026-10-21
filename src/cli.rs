//! Command-line interface.

use std::path::PathBuf;

use clap::{Parser, Subcommand};

/// Rules-driven cleanup for a messy Downloads folder.
#[derive(Debug, Parser)]
#[command(
    name = "tidy",
    version,
    about = "Rules-driven cleanup for a messy Downloads folder",
    after_help = "\
By default tidy only prints a preview. Pass --apply to move files and send matches to the Trash.\n\
Run `tidy init` to write an editable rules file, `tidy rules` to see which rules are active,\n\
and `tidy explain FILE` to see why one file matched.",
    propagate_version = true
)]
pub struct Cli {
    /// Folder to tidy. Defaults to ~/Downloads.
    #[arg(value_name = "PATH")]
    pub path: Option<PathBuf>,

    /// Perform the plan. Without this flag, tidy only prints a preview.
    #[arg(long)]
    pub apply: bool,

    /// Print the plan or the result as JSON.
    #[arg(long)]
    pub json: bool,

    /// Use this rules file instead of the one in the config directory.
    #[arg(long, value_name = "FILE")]
    pub rules: Option<PathBuf>,

    /// Only consider files whose modified time is at least this old.
    ///
    /// Examples: 30d, 12h, 2w. This filter is applied before the rules.
    /// Duplicate detection still sees the whole folder.
    #[arg(long, value_name = "DURATION")]
    pub older_than: Option<String>,

    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Write the built-in rules to the config file.
    Init {
        /// Overwrite an existing rules file.
        #[arg(long)]
        force: bool,
    },
    /// Show the active rules, or write the default rules file.
    ///
    /// Hidden alias of `tidy init` and `tidy rules`.
    #[command(hide = true)]
    Config {
        /// Write the default rules file. Same as `tidy init`.
        #[arg(long)]
        init: bool,
        /// Print the active rules and where they were loaded from.
        #[arg(long)]
        show: bool,
        /// Overwrite an existing rules file. Used with --init.
        #[arg(long)]
        force: bool,
    },
    /// Print the active rules and where they were loaded from.
    Rules {
        /// Use this rules file instead of the one in the config directory.
        #[arg(long, value_name = "FILE")]
        rules: Option<PathBuf>,
        /// Print the rules as JSON.
        #[arg(long)]
        json: bool,
    },
    /// Reverse the most recent run that has not already been undone.
    Undo {
        /// Print the undo report as JSON.
        #[arg(long)]
        json: bool,
    },
    /// Show why one file would be skipped, matched, or left in place.
    ///
    /// Read-only. Rules are applied as if tidying the file's parent folder.
    Explain {
        /// File to evaluate.
        file: PathBuf,
        /// Use this rules file instead of the one in the config directory.
        #[arg(long, value_name = "FILE")]
        rules: Option<PathBuf>,
        /// Print the explanation as JSON.
        #[arg(long)]
        json: bool,
    },
    /// Watch a folder and apply rules to new files once they settle.
    ///
    /// Watch mode always applies. Ctrl-C stops it. Each batch is written to
    /// the undo log, so `tidy undo` can reverse it.
    ///
    /// `--install`, `--uninstall`, and `--status` manage a macOS launchd agent.
    Watch {
        /// Folder to watch. Defaults to ~/Downloads.
        path: Option<PathBuf>,
        /// Use this rules file instead of the one in the config directory.
        #[arg(long, value_name = "FILE")]
        rules: Option<PathBuf>,
        /// Only consider files whose modified time is at least this old.
        #[arg(long, value_name = "DURATION")]
        older_than: Option<String>,
        /// Register a launchd agent that runs `tidy watch` (macOS).
        #[arg(long)]
        install: bool,
        /// Unload the launchd agent and delete its plist.
        #[arg(long)]
        uninstall: bool,
        /// Show whether the launchd agent is installed and running.
        #[arg(long)]
        status: bool,
        /// Print the launchd plist and exit. No files are written.
        ///
        /// Use with `--install`: `tidy watch --install --print`.
        #[arg(long)]
        print: bool,
        /// Replace an existing launchd plist.
        #[arg(long)]
        force: bool,
        /// With `--status`, print JSON. While watching, print each batch as JSON.
        #[arg(long)]
        json: bool,
    },
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;

    #[test]
    fn parses_a_dry_run_and_an_apply() {
        let cli = Cli::try_parse_from(["tidy", "/tmp/box"]).unwrap();
        assert_eq!(cli.path.unwrap(), PathBuf::from("/tmp/box"));
        assert!(!cli.apply);
        assert!(cli.command.is_none());

        let cli =
            Cli::try_parse_from(["tidy", "--apply", "--json", "--older-than", "30d"]).unwrap();
        assert!(cli.apply);
        assert!(cli.json);
        assert_eq!(cli.older_than.as_deref(), Some("30d"));
    }

    #[test]
    fn parses_subcommands() {
        let cli = Cli::try_parse_from(["tidy", "init", "--force"]).unwrap();
        assert!(matches!(cli.command, Some(Command::Init { force: true })));

        let cli = Cli::try_parse_from(["tidy", "config", "--init", "--show"]).unwrap();
        assert!(matches!(
            cli.command,
            Some(Command::Config {
                init: true,
                show: true,
                force: false
            })
        ));

        let cli =
            Cli::try_parse_from(["tidy", "rules", "--rules", "custom.toml", "--json"]).unwrap();
        assert!(matches!(
            cli.command,
            Some(Command::Rules {
                json: true,
                ref rules
            }) if rules.as_deref() == Some(Path::new("custom.toml"))
        ));

        let cli = Cli::try_parse_from(["tidy", "undo", "--json"]).unwrap();
        assert!(matches!(cli.command, Some(Command::Undo { json: true })));

        let cli = Cli::try_parse_from(["tidy", "explain", "invoice.pdf", "--json"]).unwrap();
        assert!(matches!(
            cli.command,
            Some(Command::Explain { json: true, ref file, .. }) if file == Path::new("invoice.pdf")
        ));

        let cli =
            Cli::try_parse_from(["tidy", "watch", "/tmp/inbox", "--older-than", "7d"]).unwrap();
        assert!(matches!(
            cli.command,
            Some(Command::Watch {
                path: Some(folder),
                ref older_than,
                install: false,
                ..
            }) if folder == PathBuf::from("/tmp/inbox") && older_than.as_deref() == Some("7d")
        ));

        let cli = Cli::try_parse_from(["tidy", "watch", "--install", "--print"]).unwrap();
        assert!(matches!(
            cli.command,
            Some(Command::Watch {
                install: true,
                print: true,
                ..
            })
        ));
    }

    #[test]
    fn init_does_not_accept_the_plan_flags() {
        let err = Cli::try_parse_from(["tidy", "init", "--apply"]).unwrap_err();
        let text = err.to_string();
        assert!(
            text.contains("unexpected")
                || text.contains("unrecognized")
                || text.contains("--apply"),
            "{text}"
        );
    }
}
