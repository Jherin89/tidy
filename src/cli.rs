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
Run `tidy init` to write an editable rules file, and `tidy rules` to see which rules are active.",
    propagate_version = true
)]
pub struct Cli {
    /// Folder to tidy. Defaults to ~/Downloads.
    #[arg(value_name = "PATH")]
    pub path: Option<PathBuf>,

    /// Perform the plan. Without this flag, tidy only prints a preview.
    #[arg(long)]
    pub apply: bool,

    /// Print the plan, the result, or the rules as JSON.
    #[arg(long, global = true)]
    pub json: bool,

    /// Use this rules file instead of the one in the config directory.
    #[arg(long, global = true, value_name = "FILE")]
    pub rules: Option<PathBuf>,

    /// Only consider files whose modified time is at least this old.
    ///
    /// Examples: 30d, 12h, 2w. This filter is applied before the rules.
    #[arg(long, global = true, value_name = "DURATION")]
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
    Rules,
    /// Reverse the most recent run that has not already been undone.
    Undo,
    /// Watch a folder and apply rules to new files once they settle.
    ///
    /// Watch mode always applies. Ctrl-C stops it. Each batch is written to
    /// the undo log, so `tidy undo` can reverse it.
    Watch {
        /// Folder to watch. Defaults to ~/Downloads.
        path: Option<PathBuf>,
    },
}

#[cfg(test)]
mod tests {
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

        let cli = Cli::try_parse_from(["tidy", "rules", "--rules", "custom.toml"]).unwrap();
        assert!(matches!(cli.command, Some(Command::Rules)));
        assert_eq!(cli.rules.unwrap(), PathBuf::from("custom.toml"));

        let cli = Cli::try_parse_from(["tidy", "undo"]).unwrap();
        assert!(matches!(cli.command, Some(Command::Undo)));

        let cli = Cli::try_parse_from(["tidy", "watch", "/tmp/inbox"]).unwrap();
        assert!(matches!(
            cli.command,
            Some(Command::Watch { path: Some(folder) }) if folder == PathBuf::from("/tmp/inbox")
        ));
    }
}
