//! Where tidy looks for home, config, data, and an optional fake Trash.
//!
//! Process environment variables, when set, replace the platform directories
//! from the `dirs` crate:
//!
//! * `TIDY_HOME` — home directory used for `~/` and the default Downloads folder
//! * `TIDY_CONFIG_DIR` — replaces `dirs::config_dir()`
//! * `TIDY_DATA_DIR` — replaces `dirs::data_dir()`
//! * `TIDY_FAKE_TRASH` — move "trashed" files into this directory instead of
//!   the system Trash (tests and demos)
//! * `TIDY_LAUNCHD_LABEL` — launchd job label (default `com.github.jherin89.tidy`)

use std::path::{Path, PathBuf};

use crate::error::Error;

/// Resolved locations for one tidy process.
///
/// Construct this with [`Env::from_process`] in the binary. Tests build an
/// `Env` directly so they never read the real home directory.
#[derive(Debug, Clone)]
pub struct Env {
    pub home: PathBuf,
    /// Platform config directory (`dirs::config_dir()` or `TIDY_CONFIG_DIR`).
    /// This is the parent of the `tidy` folder, not the folder itself.
    pub config_dir: PathBuf,
    /// `true` when `config_dir` came from `TIDY_CONFIG_DIR`.
    /// An explicit config dir disables the macOS `~/.config` preference.
    pub config_from_env: bool,
    /// Platform data directory (`dirs::data_dir()` or `TIDY_DATA_DIR`).
    pub data_dir: PathBuf,
    /// When set, trash operations move files here instead of calling the OS.
    pub fake_trash: Option<PathBuf>,
}

impl Env {
    /// Read locations from the process environment, then the `dirs` crate.
    pub fn from_process() -> Result<Self, Error> {
        let home = if let Some(value) = std::env::var_os("TIDY_HOME") {
            let path = PathBuf::from(value);
            if path.as_os_str().is_empty() {
                return Err(Error::msg("TIDY_HOME is set but empty"));
            }
            path
        } else {
            dirs::home_dir().ok_or_else(|| {
                Error::msg("could not determine the home directory; set TIDY_HOME")
            })?
        };

        let (config_dir, config_from_env) = if let Some(value) = std::env::var_os("TIDY_CONFIG_DIR")
        {
            let path = PathBuf::from(value);
            if path.as_os_str().is_empty() {
                return Err(Error::msg("TIDY_CONFIG_DIR is set but empty"));
            }
            (path, true)
        } else {
            let path = dirs::config_dir().ok_or_else(|| {
                Error::msg("could not determine the config directory; set TIDY_CONFIG_DIR")
            })?;
            (path, false)
        };

        let data_dir = if let Some(value) = std::env::var_os("TIDY_DATA_DIR") {
            let path = PathBuf::from(value);
            if path.as_os_str().is_empty() {
                return Err(Error::msg("TIDY_DATA_DIR is set but empty"));
            }
            path
        } else {
            dirs::data_dir().ok_or_else(|| {
                Error::msg("could not determine the data directory; set TIDY_DATA_DIR")
            })?
        };

        let fake_trash = std::env::var_os("TIDY_FAKE_TRASH").and_then(|value| {
            let path = PathBuf::from(&value);
            if path.as_os_str().is_empty() {
                None
            } else {
                Some(path)
            }
        });

        Ok(Env {
            home,
            config_dir,
            config_from_env,
            data_dir,
            fake_trash,
        })
    }

    pub fn downloads(&self) -> PathBuf {
        self.home.join("Downloads")
    }

    pub fn logs_dir(&self) -> PathBuf {
        self.data_dir.join("tidy").join("logs")
    }

    /// Rules file that would be read when `--rules` is not passed.
    ///
    /// Lookup, when `macos_prefers_xdg` is true (the CLI sets this on macOS):
    ///
    /// 1. `$TIDY_CONFIG_DIR/tidy/rules.toml` when `TIDY_CONFIG_DIR` is set.
    ///    The macOS preference is skipped, because the variable is explicit.
    /// 2. `$HOME/.config/tidy/rules.toml` when that file exists.
    /// 3. `dirs::config_dir()/tidy/rules.toml`
    ///    (`~/Library/Application Support/tidy/rules.toml` on macOS).
    ///
    /// On Linux and Windows `macos_prefers_xdg` is false, so the path is
    /// `config_dir/tidy/rules.toml`. On Linux that is already
    /// `~/.config/tidy/rules.toml` unless `XDG_CONFIG_HOME` says otherwise.
    pub fn rules_read_path(&self, macos_prefers_xdg: bool) -> PathBuf {
        if self.config_from_env {
            return self.rules_under_config();
        }
        if macos_prefers_xdg {
            let xdg = self.xdg_rules_path();
            if xdg.is_file() {
                return xdg;
            }
        }
        self.rules_under_config()
    }

    /// Where `tidy init` writes a new rules file.
    ///
    /// On macOS this is `~/.config/tidy/rules.toml`, unless a platform config
    /// file already exists and the XDG file does not — then init updates the
    /// file tidy would already be reading. `TIDY_CONFIG_DIR` always wins.
    pub fn rules_init_path(&self, macos_style: bool) -> PathBuf {
        if self.config_from_env {
            return self.rules_under_config();
        }
        let platform = self.rules_under_config();
        if macos_style {
            let xdg = self.xdg_rules_path();
            if platform.is_file() && !xdg.is_file() {
                return platform;
            }
            return xdg;
        }
        platform
    }

    fn rules_under_config(&self) -> PathBuf {
        self.config_dir.join("tidy").join("rules.toml")
    }

    fn xdg_rules_path(&self) -> PathBuf {
        self.home.join(".config").join("tidy").join("rules.toml")
    }
}

/// Expand a leading `~` or `~/` (also `~\`) using `home`.
///
/// `~other` is left unchanged. Relative paths are returned unchanged so the
/// caller can resolve them against the folder being tidied.
pub fn expand_tilde(raw: &str, home: &Path) -> PathBuf {
    if raw == "~" {
        return home.to_path_buf();
    }
    if let Some(rest) = raw.strip_prefix("~/").or_else(|| raw.strip_prefix("~\\")) {
        let mut path = home.to_path_buf();
        for part in rest.split(['/', '\\']) {
            if !part.is_empty() {
                path.push(part);
            }
        }
        return path;
    }
    PathBuf::from(raw)
}

/// Contract `path` to a `~/...` display string when it lives under `home`.
pub fn contract_home(path: &Path, home: &Path) -> String {
    match path.strip_prefix(home) {
        Ok(rest) if rest.as_os_str().is_empty() => "~".to_string(),
        Ok(rest) => {
            let text = rest.to_string_lossy().replace('\\', "/");
            format!("~/{text}")
        }
        Err(_) => path.display().to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn env_at(root: &Path, from_env: bool) -> Env {
        Env {
            home: root.join("home"),
            config_dir: root.join("config"),
            config_from_env: from_env,
            data_dir: root.join("data"),
            fake_trash: None,
        }
    }

    #[test]
    fn tilde_expansion_and_contraction() {
        let home = Path::new("/Users/jordan");
        assert_eq!(expand_tilde("~", home), home);
        assert_eq!(
            expand_tilde("~/Documents/PDFs", home),
            PathBuf::from("/Users/jordan/Documents/PDFs")
        );
        assert_eq!(
            expand_tilde("~\\Documents\\PDFs", home),
            PathBuf::from("/Users/jordan/Documents/PDFs")
        );
        assert_eq!(expand_tilde("~jordan", home), PathBuf::from("~jordan"));
        assert_eq!(expand_tilde("Archives", home), PathBuf::from("Archives"));
        assert_eq!(
            contract_home(Path::new("/Users/jordan/Documents/PDFs"), home),
            "~/Documents/PDFs"
        );
        assert_eq!(contract_home(Path::new("/tmp/other"), home), "/tmp/other");
    }

    #[test]
    fn macos_lookup_prefers_existing_xdg_file() {
        let tmp = tempfile::tempdir().unwrap();
        let env = env_at(tmp.path(), false);
        fs::create_dir_all(env.home.join(".config/tidy")).unwrap();
        fs::write(env.xdg_rules_path(), "rules = []\n").unwrap();

        let read = env.rules_read_path(true);
        assert_eq!(read, env.xdg_rules_path());
        assert_eq!(env.rules_init_path(true), env.xdg_rules_path());
    }

    #[test]
    fn macos_lookup_falls_back_to_platform_config_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let env = env_at(tmp.path(), false);
        assert_eq!(env.rules_read_path(true), env.rules_under_config());
        assert_eq!(env.rules_init_path(true), env.xdg_rules_path());
    }

    #[test]
    fn macos_init_keeps_an_existing_platform_file() {
        let tmp = tempfile::tempdir().unwrap();
        let env = env_at(tmp.path(), false);
        fs::create_dir_all(env.config_dir.join("tidy")).unwrap();
        fs::write(env.rules_under_config(), "rules = []\n").unwrap();
        assert_eq!(env.rules_read_path(true), env.rules_under_config());
        assert_eq!(env.rules_init_path(true), env.rules_under_config());
    }

    #[test]
    fn explicit_config_dir_skips_the_macos_preference() {
        let tmp = tempfile::tempdir().unwrap();
        let env = env_at(tmp.path(), true);
        fs::create_dir_all(env.home.join(".config/tidy")).unwrap();
        fs::write(env.xdg_rules_path(), "rules = []\n").unwrap();
        assert_eq!(env.rules_read_path(true), env.rules_under_config());
        assert_eq!(env.rules_init_path(true), env.rules_under_config());
    }

    #[test]
    fn linux_style_lookup_uses_the_config_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let env = env_at(tmp.path(), false);
        assert_eq!(env.rules_read_path(false), env.rules_under_config());
        assert_eq!(env.rules_init_path(false), env.rules_under_config());
        assert_eq!(env.logs_dir(), tmp.path().join("data/tidy/logs"));
        assert_eq!(env.downloads(), tmp.path().join("home/Downloads"));
    }
}
