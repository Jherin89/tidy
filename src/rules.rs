//! Loading, validating, and matching rules.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use globset::{GlobBuilder, GlobSet, GlobSetBuilder};
use serde::Deserialize;

use crate::duplicates::DuplicateIndex;
use crate::env::Env;
use crate::error::Error;
use crate::names::{extension_matches, last_extension};
use crate::parse::{format_age, format_size, parse_duration, parse_size};
use crate::scan::FileEntry;
use crate::DEFAULT_RULES;

/// Where the active rules came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RulesSource {
    BuiltIn,
    File,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    Move { destination: String, by_date: bool },
    Trash,
}

/// One validated rule. Conditions that are `None` or empty are not checked.
/// Every condition that is set must match. Several extensions, or several
/// globs, match if any one of them matches (OR within the list, AND across
/// different conditions).
#[derive(Debug, Clone)]
pub struct Rule {
    pub name: String,
    pub extensions: Vec<String>,
    pub globs: Vec<String>,
    globset: Option<GlobSet>,
    pub older_than: Option<Duration>,
    pub older_than_text: Option<String>,
    pub larger_than: Option<u64>,
    pub larger_than_text: Option<String>,
    pub smaller_than: Option<u64>,
    pub smaller_than_text: Option<String>,
    /// `Some(true)` matches an extra copy. `Some(false)` matches everything
    /// that is not an extra copy. `None` does not look at duplicates.
    pub duplicate: Option<bool>,
    pub action: Action,
}

/// One condition, and why it passed or failed. `tidy explain` prints these.
/// [`Rule::matches`] is this result's `matched` flag, so the two cannot disagree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConditionExplain {
    pub name: &'static str,
    pub passed: bool,
    pub detail: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuleExplain {
    pub matched: bool,
    pub conditions: Vec<ConditionExplain>,
}

impl Rule {
    /// Evaluate every condition. The rule matches when all of them pass.
    pub fn explain(
        &self,
        file: &FileEntry,
        now: SystemTime,
        duplicates: &DuplicateIndex,
    ) -> RuleExplain {
        let mut conditions = Vec::new();

        if !self.extensions.is_empty() {
            let matched = matching_extension(&file.name, &self.extensions);
            let passed = matched.is_some();
            let detail = extension_detail(&file.name, &self.extensions, matched.as_deref());
            conditions.push(ConditionExplain {
                name: "extensions",
                passed,
                detail,
            });
        }

        if self.globset.is_some() {
            let passed = self
                .globset
                .as_ref()
                .is_some_and(|set| set.is_match(&file.name));
            let list = self.globs.join(", ");
            let detail = if passed {
                format!("matches {list}")
            } else {
                format!("does not match {list}")
            };
            conditions.push(ConditionExplain {
                name: "glob",
                passed,
                detail,
            });
        }

        if let Some(min_age) = self.older_than {
            let passed = is_older_than(file.modified, now, min_age);
            let need = self
                .older_than_text
                .clone()
                .unwrap_or_else(|| format_age(min_age));
            let detail = match now.duration_since(file.modified) {
                Ok(age) => format!("{} old, needs ≥ {need}", format_age(age)),
                Err(_) => format!("modified in the future, needs ≥ {need}"),
            };
            conditions.push(ConditionExplain {
                name: "older_than",
                passed,
                detail,
            });
        }

        if let Some(min_size) = self.larger_than {
            let passed = file.size >= min_size;
            let limit = self
                .larger_than_text
                .clone()
                .unwrap_or_else(|| format_size(min_size));
            let actual = format_size(file.size);
            let detail = if passed {
                format!("{actual} ≥ {limit}")
            } else {
                format!("{actual} < {limit} (needs ≥ {limit})")
            };
            conditions.push(ConditionExplain {
                name: "larger_than",
                passed,
                detail,
            });
        }

        if let Some(max_size) = self.smaller_than {
            let passed = file.size <= max_size;
            let limit = self
                .smaller_than_text
                .clone()
                .unwrap_or_else(|| format_size(max_size));
            let actual = format_size(file.size);
            let detail = if passed {
                format!("{actual} ≤ {limit}")
            } else {
                format!("{actual} > {limit} (needs ≤ {limit})")
            };
            conditions.push(ConditionExplain {
                name: "smaller_than",
                passed,
                detail,
            });
        }

        if let Some(want_extra) = self.duplicate {
            let keeper = duplicates.keeper_of(&file.path);
            let is_extra = keeper.is_some();
            let passed = is_extra == want_extra;
            let detail = duplicate_detail(file, duplicates, keeper);
            conditions.push(ConditionExplain {
                name: "duplicate",
                passed,
                detail,
            });
        }

        RuleExplain {
            matched: conditions.iter().all(|condition| condition.passed),
            conditions,
        }
    }

    pub fn matches(&self, file: &FileEntry, now: SystemTime, duplicates: &DuplicateIndex) -> bool {
        self.explain(file, now, duplicates).matched
    }
}

fn matching_extension(name: &str, extensions: &[String]) -> Option<String> {
    if !extension_matches(name, extensions) {
        return None;
    }
    let lower = name.to_ascii_lowercase();
    let last = last_extension(name);
    extensions
        .iter()
        .find(|ext| {
            if ext.contains('.') {
                lower.ends_with(&format!(".{ext}"))
            } else {
                last.as_deref() == Some(ext.as_str())
            }
        })
        .cloned()
}

fn extension_detail(name: &str, extensions: &[String], matched: Option<&str>) -> String {
    let list = extensions.join(", ");
    if let Some(ext) = matched {
        return format!("{ext} is in {list}");
    }
    match last_extension(name) {
        Some(ext) => format!("{ext} is not in {list}"),
        None => format!("no extension; needs one of {list}"),
    }
}

fn duplicate_detail(file: &FileEntry, duplicates: &DuplicateIndex, keeper: Option<&str>) -> String {
    if let Some(name) = keeper {
        return format!("extra copy of {name}");
    }
    if file.size == 0 {
        return "not an extra copy (zero-byte files are ignored)".to_string();
    }
    if duplicates.is_keeper(&file.path) {
        return "not an extra copy (this file is the one that is kept)".to_string();
    }
    "not an extra copy".to_string()
}

/// Inclusive: a file whose age is exactly `min_age` matches.
pub fn is_older_than(modified: SystemTime, now: SystemTime, min_age: Duration) -> bool {
    match now.duration_since(modified) {
        Ok(age) => age >= min_age,
        Err(_) => false,
    }
}

#[derive(Debug, Clone)]
pub struct Ruleset {
    pub source: RulesSource,
    /// File the rules were read from. `None` for the built-in set.
    pub path: Option<PathBuf>,
    /// Path tidy would read when `--rules` is not passed. Shown in `tidy rules`
    /// so a built-in run still tells you where a file would go.
    pub candidate_path: PathBuf,
    pub rules: Vec<Rule>,
}

impl Ruleset {
    pub fn parse(text: &str) -> Result<Vec<Rule>, String> {
        parse_rules(text)
    }
}

/// Load rules for a run.
///
/// `override_path` is `--rules`. When it is set, the file must exist.
/// Otherwise a missing default file selects the built-in rules.
pub fn load_rules(
    env: &Env,
    override_path: Option<&Path>,
    macos_prefers_xdg: bool,
) -> Result<Ruleset, Error> {
    let candidate = env.rules_read_path(macos_prefers_xdg);
    if let Some(path) = override_path {
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
        let rules = read_rules_file(path)?;
        return Ok(Ruleset {
            source: RulesSource::File,
            path: Some(path.to_path_buf()),
            candidate_path: candidate,
            rules,
        });
    }
    if candidate.is_file() {
        let rules = read_rules_file(&candidate)?;
        return Ok(Ruleset {
            source: RulesSource::File,
            path: Some(candidate.clone()),
            candidate_path: candidate,
            rules,
        });
    }
    let rules = parse_rules(DEFAULT_RULES).map_err(|message| {
        Error::msg(format!(
            "internal error: built-in rules failed to parse: {message}"
        ))
    })?;
    Ok(Ruleset {
        source: RulesSource::BuiltIn,
        path: None,
        candidate_path: candidate,
        rules,
    })
}

pub fn write_default_rules(path: &Path, force: bool) -> Result<(), Error> {
    if path.exists() && !force {
        return Err(Error::msg(format!(
            "rules file already exists: {} (pass --force to overwrite)",
            path.display()
        )));
    }
    if path.is_dir() {
        return Err(Error::msg(format!(
            "rules path is a directory: {}",
            path.display()
        )));
    }
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent)
                .map_err(|source| Error::io(format!("creating {}", parent.display()), source))?;
        }
    }
    fs::write(path, DEFAULT_RULES)
        .map_err(|source| Error::io(format!("writing {}", path.display()), source))?;
    Ok(())
}

fn read_rules_file(path: &Path) -> Result<Vec<Rule>, Error> {
    let text = fs::read_to_string(path)
        .map_err(|source| Error::io(format!("reading {}", path.display()), source))?;
    parse_rules(&text).map_err(|message| Error::Rules {
        path: path.to_path_buf(),
        message,
    })
}

fn parse_rules(text: &str) -> Result<Vec<Rule>, String> {
    let file: RulesFile = toml::from_str(text).map_err(|err| err.to_string())?;
    let mut rules = Vec::with_capacity(file.rules.len());
    for (index, raw) in file.rules.into_iter().enumerate() {
        rules.push(compile_rule(index, raw)?);
    }
    Ok(rules)
}

fn compile_rule(index: usize, raw: RawRule) -> Result<Rule, String> {
    let label = format!("rule {} ({})", index + 1, raw.name.trim());
    let name = raw.name.trim().to_string();
    if name.is_empty() {
        return Err(format!("rule {}: name must not be empty", index + 1));
    }

    let extensions =
        normalize_extensions(raw.extensions.unwrap_or_else(|| OneOrMany::Many(vec![])))
            .map_err(|err| format!("{label}: {err}"))?;
    let globs = string_list(raw.glob);
    if globs.iter().any(|g| g.trim().is_empty()) {
        return Err(format!("{label}: glob patterns must not be empty"));
    }
    let globset = build_globset(&globs).map_err(|err| format!("{label}: {err}"))?;

    let older_than_text = blank_to_none(raw.older_than);
    let older_than = match &older_than_text {
        Some(text) => Some(parse_duration(text).map_err(|err| format!("{label}: {err}"))?),
        None => None,
    };
    let larger_than_text = blank_to_none(raw.larger_than);
    let larger_than = match &larger_than_text {
        Some(text) => Some(parse_size(text).map_err(|err| format!("{label}: {err}"))?),
        None => None,
    };
    let smaller_than_text = blank_to_none(raw.smaller_than);
    let smaller_than = match &smaller_than_text {
        Some(text) => Some(parse_size(text).map_err(|err| format!("{label}: {err}"))?),
        None => None,
    };

    let duplicate = raw.duplicate;
    if extensions.is_empty()
        && globs.is_empty()
        && older_than.is_none()
        && larger_than.is_none()
        && smaller_than.is_none()
        && duplicate.is_none()
    {
        return Err(format!(
            "{label}: set at least one condition (extensions, glob, older_than, larger_than, smaller_than, or duplicate)"
        ));
    }

    if let (Some(min), Some(max)) = (larger_than, smaller_than) {
        if min > max {
            return Err(format!(
                "{label}: larger_than is greater than smaller_than, so the rule can never match"
            ));
        }
    }

    let action_name = raw.action.trim().to_ascii_lowercase();
    let action = match action_name.as_str() {
        "move" => {
            let destination = raw
                .destination
                .as_deref()
                .map(str::trim)
                .filter(|d| !d.is_empty())
                .ok_or_else(|| format!("{label}: a move rule needs destination"))?
                .to_string();
            Action::Move {
                destination,
                by_date: raw.by_date,
            }
        }
        "trash" => {
            if raw
                .destination
                .as_deref()
                .is_some_and(|d| !d.trim().is_empty())
            {
                return Err(format!(
                    "{label}: destination is only valid when action = \"move\""
                ));
            }
            if raw.by_date {
                return Err(format!(
                    "{label}: by_date is only valid when action = \"move\""
                ));
            }
            Action::Trash
        }
        "delete" | "remove" | "rm" => {
            return Err(format!(
                "{label}: action `{action_name}` is not supported. Use action = \"trash\" to send files to the Trash. tidy never permanently deletes files"
            ));
        }
        other => {
            return Err(format!(
                "{label}: unknown action `{other}` (expected \"move\" or \"trash\")"
            ));
        }
    };

    Ok(Rule {
        name,
        extensions,
        globs,
        globset,
        older_than,
        older_than_text,
        larger_than,
        larger_than_text,
        smaller_than,
        smaller_than_text,
        duplicate,
        action,
    })
}

fn build_globset(globs: &[String]) -> Result<Option<GlobSet>, String> {
    if globs.is_empty() {
        return Ok(None);
    }
    let mut builder = GlobSetBuilder::new();
    for pattern in globs {
        let glob = GlobBuilder::new(pattern)
            .case_insensitive(true)
            .literal_separator(true)
            .build()
            .map_err(|err| format!("invalid glob `{pattern}`: {err}"))?;
        builder.add(glob);
    }
    builder
        .build()
        .map(Some)
        .map_err(|err| format!("invalid glob: {err}"))
}

fn normalize_extensions(value: OneOrMany) -> Result<Vec<String>, String> {
    let mut out = Vec::new();
    for raw in value.into_vec() {
        let trimmed = raw.trim().trim_start_matches('.').to_ascii_lowercase();
        if trimmed.is_empty() {
            return Err("extensions must not be empty".to_string());
        }
        if !out.contains(&trimmed) {
            out.push(trimmed);
        }
    }
    Ok(out)
}

fn string_list(value: Option<OneOrMany>) -> Vec<String> {
    value
        .map(OneOrMany::into_vec)
        .unwrap_or_default()
        .into_iter()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

fn blank_to_none(value: Option<String>) -> Option<String> {
    value
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RulesFile {
    #[serde(default)]
    rules: Vec<RawRule>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawRule {
    name: String,
    #[serde(default)]
    extensions: Option<OneOrMany>,
    #[serde(default)]
    glob: Option<OneOrMany>,
    #[serde(default)]
    older_than: Option<String>,
    #[serde(default)]
    larger_than: Option<String>,
    #[serde(default)]
    smaller_than: Option<String>,
    #[serde(default)]
    duplicate: Option<bool>,
    action: String,
    #[serde(default)]
    destination: Option<String>,
    #[serde(default)]
    by_date: bool,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum OneOrMany {
    One(String),
    Many(Vec<String>),
}

impl OneOrMany {
    fn into_vec(self) -> Vec<String> {
        match self {
            OneOrMany::One(value) => vec![value],
            OneOrMany::Many(values) => values,
        }
    }
}

/// Human-readable description of the active rules.
pub fn render_rules(ruleset: &Ruleset, color: bool) -> String {
    let mut out = String::new();
    match ruleset.source {
        RulesSource::BuiltIn => {
            push(
                &mut out,
                &dye("Rules source: built-in defaults", color, "1"),
            );
            out.push('\n');
            out.push_str(&format!(
                "No rules file at {}.\nRun `tidy init` to write an editable copy.\n",
                ruleset.candidate_path.display()
            ));
        }
        RulesSource::File => {
            let path = ruleset
                .path
                .as_ref()
                .map(|p| p.display().to_string())
                .unwrap_or_else(|| ruleset.candidate_path.display().to_string());
            push(&mut out, &dye(&format!("Rules source: {path}"), color, "1"));
            out.push('\n');
        }
    }
    if ruleset.rules.is_empty() {
        out.push_str("\n(no rules)\n");
        return out;
    }
    out.push('\n');
    for (index, rule) in ruleset.rules.iter().enumerate() {
        out.push_str(&dye(
            &format!("{}. {}", index + 1, rule.name),
            color,
            "1;36",
        ));
        out.push('\n');
        if !rule.extensions.is_empty() {
            out.push_str(&format!("   extensions: {}\n", rule.extensions.join(", ")));
        }
        if !rule.globs.is_empty() {
            out.push_str(&format!("   glob: {}\n", rule.globs.join(", ")));
        }
        if let Some(text) = &rule.older_than_text {
            out.push_str(&format!("   older_than: {text}\n"));
        }
        if let Some(text) = &rule.larger_than_text {
            out.push_str(&format!("   larger_than: {text}\n"));
        }
        if let Some(text) = &rule.smaller_than_text {
            out.push_str(&format!("   smaller_than: {text}\n"));
        }
        if let Some(flag) = rule.duplicate {
            out.push_str(&format!("   duplicate: {flag}\n"));
        }
        match &rule.action {
            Action::Trash => out.push_str("   action: trash\n"),
            Action::Move {
                destination,
                by_date,
            } => {
                if *by_date {
                    out.push_str(&format!(
                        "   action: move -> {destination} (grouped by month)\n"
                    ));
                } else {
                    out.push_str(&format!("   action: move -> {destination}\n"));
                }
            }
        }
    }
    out
}

pub fn render_rules_json(ruleset: &Ruleset) -> Result<String, Error> {
    let rules: Vec<serde_json::Value> = ruleset
        .rules
        .iter()
        .map(|rule| {
            let (action, destination, by_date) = match &rule.action {
                Action::Trash => ("trash", None, false),
                Action::Move {
                    destination,
                    by_date,
                } => ("move", Some(destination.clone()), *by_date),
            };
            serde_json::json!({
                "name": rule.name,
                "extensions": rule.extensions,
                "glob": rule.globs,
                "older_than": rule.older_than_text,
                "larger_than": rule.larger_than_text,
                "smaller_than": rule.smaller_than_text,
                "duplicate": rule.duplicate,
                "action": action,
                "destination": destination,
                "by_date": by_date,
            })
        })
        .collect();
    let value = serde_json::json!({
        "version": 1,
        "source": match ruleset.source {
            RulesSource::BuiltIn => "built-in",
            RulesSource::File => "file",
        },
        "path": ruleset.path.as_ref().map(|p| p.display().to_string()),
        "candidate_path": ruleset.candidate_path.display().to_string(),
        "rules": rules,
    });
    serde_json::to_string_pretty(&value).map_err(|err| Error::msg(format!("json: {err}")))
}

fn dye(text: &str, enabled: bool, code: &str) -> String {
    if !enabled {
        return text.to_string();
    }
    format!("\u{1b}[{code}m{text}\u{1b}[0m")
}

fn push(out: &mut String, line: &str) {
    out.push_str(line);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration as StdDuration;

    fn entry(name: &str, size: u64, age: Duration) -> FileEntry {
        let now = SystemTime::UNIX_EPOCH + StdDuration::from_secs(1_700_000_000);
        FileEntry {
            path: PathBuf::from(name),
            name: name.to_string(),
            size,
            modified: now - age,
        }
    }

    fn now() -> SystemTime {
        SystemTime::UNIX_EPOCH + StdDuration::from_secs(1_700_000_000)
    }

    fn rules(toml: &str) -> Vec<Rule> {
        Ruleset::parse(toml).unwrap_or_else(|err| panic!("rules failed to parse: {err}"))
    }

    #[test]
    fn builtin_rules_parse_and_cover_the_documented_set() {
        let parsed = Ruleset::parse(DEFAULT_RULES).unwrap();
        let names: Vec<_> = parsed.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(
            names,
            vec![
                "Duplicates",
                "Screenshots",
                "Old installers",
                "PDFs",
                "Images",
                "Videos",
                "Audio",
                "Spreadsheets",
                "Documents",
                "Old archives",
                "Ebooks",
            ]
        );
    }

    #[test]
    fn extension_match_is_case_insensitive() {
        let set = rules(
            r#"
            [[rules]]
            name = "PDFs"
            extensions = ["PDF"]
            action = "move"
            destination = "~/Documents/PDFs"
            "#,
        );
        let none = DuplicateIndex::empty();
        assert!(set[0].matches(
            &entry("Report.PDF", 10, StdDuration::from_secs(1)),
            now(),
            &none
        ));
        assert!(!set[0].matches(
            &entry("Report.txt", 10, StdDuration::from_secs(1)),
            now(),
            &none
        ));
    }

    #[test]
    fn glob_is_case_insensitive_and_combines_with_extension() {
        let set = rules(
            r#"
            [[rules]]
            name = "Shots"
            glob = ["Screenshot*", "Screen Shot*"]
            extensions = "png"
            action = "move"
            destination = "~/Pictures/Screenshots"
            "#,
        );
        let rule = &set[0];
        let none = DuplicateIndex::empty();
        let at = now();
        assert!(rule.matches(&entry("screenshot 1.PNG", 10, StdDuration::ZERO), at, &none));
        assert!(rule.matches(
            &entry("Screen Shot 2024.png", 10, StdDuration::ZERO),
            at,
            &none
        ));
        assert!(!rule.matches(&entry("screenshot 1.jpg", 10, StdDuration::ZERO), at, &none));
        assert!(!rule.matches(&entry("photo.png", 10, StdDuration::ZERO), at, &none));
    }

    #[test]
    fn age_and_size_are_inclusive_and_must_all_match() {
        let set = rules(
            r#"
            [[rules]]
            name = "Big old pdf"
            extensions = ["pdf"]
            older_than = "30d"
            larger_than = "1000B"
            smaller_than = "5000B"
            action = "trash"
            "#,
        );
        let rule = &set[0];
        let day = StdDuration::from_secs(86400);
        let at = now();
        let none = DuplicateIndex::empty();
        let hit = FileEntry {
            path: PathBuf::from("a.pdf"),
            name: "a.pdf".into(),
            size: 1000,
            modified: at - 30 * day,
        };
        assert!(rule.matches(&hit, at, &none));
        let why = rule.explain(&hit, at, &none);
        assert!(why.matched);
        assert!(why
            .conditions
            .iter()
            .any(|c| c.name == "older_than" && c.passed));
        let mut too_new = hit.clone();
        too_new.modified = at - 30 * day + StdDuration::from_secs(1);
        assert!(!rule.matches(&too_new, at, &none));
        let age = rule.explain(&too_new, at, &none);
        let older = age
            .conditions
            .iter()
            .find(|c| c.name == "older_than")
            .unwrap();
        assert!(!older.passed);
        assert!(older.detail.contains("needs ≥ 30d"), "{}", older.detail);
        let mut too_small = hit.clone();
        too_small.size = 999;
        assert!(!rule.matches(&too_small, at, &none));
        let mut too_big = hit.clone();
        too_big.size = 5001;
        assert!(!rule.matches(&too_big, at, &none));
        let mut wrong_ext = hit.clone();
        wrong_ext.name = "a.txt".into();
        assert!(!rule.matches(&wrong_ext, at, &none));
    }

    #[test]
    fn first_matching_rule_wins() {
        let set = rules(
            r#"
            [[rules]]
            name = "First"
            extensions = ["txt"]
            action = "trash"

            [[rules]]
            name = "Second"
            extensions = ["txt"]
            action = "move"
            destination = "~/Documents/Downloads"
            "#,
        );
        let file = entry("notes.txt", 4, StdDuration::from_secs(10));
        let none = DuplicateIndex::empty();
        let winner = set
            .iter()
            .find(|rule| rule.matches(&file, now(), &none))
            .unwrap();
        assert_eq!(winner.name, "First");
    }

    #[test]
    fn duplicate_true_matches_only_the_extra_copy() {
        let set = rules(
            r#"
            [[rules]]
            name = "Duplicates"
            duplicate = true
            action = "trash"

            [[rules]]
            name = "Unique pdf"
            duplicate = false
            extensions = ["pdf"]
            action = "move"
            destination = "~/Documents/PDFs"
            "#,
        );
        let tmp = tempfile::tempdir().unwrap();
        let folder = tmp.path();
        let keeper = entry_at(folder, "invoice.pdf", b"same", StdDuration::from_secs(50));
        let extra = entry_at(
            folder,
            "invoice (1).pdf",
            b"same",
            StdDuration::from_secs(10),
        );
        let index = DuplicateIndex::build(folder, &[keeper.clone(), extra.clone()]).unwrap();
        let at = now();
        assert!(!set[0].matches(&keeper, at, &index));
        assert!(set[0].matches(&extra, at, &index));
        let why = set[0].explain(&extra, at, &index);
        assert_eq!(why.conditions[0].detail, "extra copy of invoice.pdf");
        assert!(set[1].matches(&keeper, at, &index));
        assert!(!set[1].matches(&extra, at, &index));
        let kept = set[0].explain(&keeper, at, &index);
        assert!(kept.conditions[0].detail.contains("kept"));
    }

    fn entry_at(folder: &std::path::Path, name: &str, bytes: &[u8], age: StdDuration) -> FileEntry {
        let path = folder.join(name);
        std::fs::write(&path, bytes).unwrap();
        let at = now();
        FileEntry {
            path,
            name: name.to_string(),
            size: bytes.len() as u64,
            modified: at - age,
        }
    }

    #[test]
    fn rejects_invalid_rules() {
        let cases = [
            (
                "[[rules]]\nname = \"A\"\naction = \"trash\"\n",
                "condition",
            ),
            (
                "[[rules]]\nname = \"A\"\nextensions = [\"pdf\"]\naction = \"move\"\n",
                "destination",
            ),
            (
                "[[rules]]\nname = \"A\"\nextensions = [\"pdf\"]\naction = \"delete\"\n",
                "trash",
            ),
            (
                "[[rules]]\nname = \"A\"\nextensions = [\"pdf\"]\nolder_than = \"nope\"\naction = \"trash\"\n",
                "duration",
            ),
            (
                "[[rules]]\nname = \"A\"\nextensions = [\"pdf\"]\nlarger_than = \"lots\"\naction = \"trash\"\n",
                "size",
            ),
            (
                "[[rules]]\nname = \"\"\nextensions = [\"pdf\"]\naction = \"trash\"\n",
                "name",
            ),
            (
                "[[rules]]\nname = \"A\"\nglob = \"[\"\naction = \"trash\"\n",
                "glob",
            ),
            (
                "[[rules]]\nname = \"A\"\nextensions = [\"pdf\"]\naction = \"trash\"\ndestination = \"~/x\"\n",
                "destination",
            ),
            (
                "[[rules]]\nname = \"A\"\nnope = true\naction = \"trash\"\n",
                "unknown",
            ),
        ];
        for (text, needle) in cases {
            let err = Ruleset::parse(text).unwrap_err();
            assert!(
                err.to_ascii_lowercase().contains(needle),
                "expected `{needle}` in `{err}`"
            );
        }
    }

    #[test]
    fn invalid_toml_reports_a_parse_error() {
        let err = Ruleset::parse("[[rules]\n").unwrap_err();
        assert!(err.to_ascii_lowercase().contains("toml") || err.contains("line"));
    }
}
