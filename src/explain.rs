//! `tidy explain <FILE>` — why a file would be skipped, matched, or left alone.
//!
//! The walk uses [`crate::rules::Rule::explain`], which is the same check
//! [`crate::rules::Rule::matches`] uses to build a plan. The destination,
//! including a collision suffix, comes from that plan.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use crate::env::{contract_home, Env};
use crate::error::Error;
use crate::plan::{plan_entries, ActionKind, Plan};
use crate::rules::{ConditionExplain, Ruleset};
use crate::scan::{self, Inspect, SkipReason};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuleMark {
    Wins,
    Miss,
    NotEvaluated,
}

impl RuleMark {
    pub fn as_str(self) -> &'static str {
        match self {
            RuleMark::Wins => "wins",
            RuleMark::Miss => "no-match",
            RuleMark::NotEvaluated => "not evaluated",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExplainedRule {
    pub name: String,
    pub mark: RuleMark,
    pub conditions: Vec<ConditionExplain>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    Skipped {
        reason: String,
        detail: String,
    },
    Matched {
        rule: String,
        action: String,
        destination: String,
        destination_path: Option<PathBuf>,
        copy_of: Option<String>,
    },
    Unmatched,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Explanation {
    pub file_name: String,
    pub path: PathBuf,
    pub folder: PathBuf,
    pub rules: Vec<ExplainedRule>,
    pub outcome: Outcome,
}

impl Explanation {
    /// Winning rule name. `None` when the file is skipped or matches nothing.
    pub fn winner(&self) -> Option<&str> {
        match &self.outcome {
            Outcome::Matched { rule, .. } => Some(rule.as_str()),
            _ => None,
        }
    }

    pub fn destination(&self) -> Option<&str> {
        match &self.outcome {
            Outcome::Matched { destination, .. } => Some(destination.as_str()),
            _ => None,
        }
    }
}

/// Evaluate `path` as if tidying its parent folder. Does not create or move anything.
pub fn explain_path(
    env: &Env,
    rules: &Ruleset,
    path: &Path,
    now: SystemTime,
) -> Result<Explanation, Error> {
    explain_path_with(env, rules, path, now, &crate::age::SystemAge)
}

pub fn explain_path_with(
    env: &Env,
    rules: &Ruleset,
    path: &Path,
    now: SystemTime,
    ages: &dyn crate::age::AgeProbe,
) -> Result<Explanation, Error> {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .filter(|n| !n.is_empty())
        .ok_or_else(|| Error::msg(format!("path has no file name: {}", path.display())))?;

    match std::fs::symlink_metadata(path) {
        Ok(_) => {}
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            return Err(Error::msg(format!("file not found: {}", path.display())));
        }
        Err(err) => {
            return Err(crate::privacy::annotate(
                parent_dir(path),
                format!("reading {}", path.display()),
                err,
            ));
        }
    }

    let folder = scan::resolve_folder(parent_dir(path))?;
    let child = folder.join(&name);
    let bundles = rules.bundle_extensions();
    let inspected = scan::inspect_with(&folder, &child, None, now, &bundles, ages)?;
    match inspected {
        Inspect::Skip(skip) => {
            return Ok(skipped_explanation(
                skip.name,
                skip.path,
                folder,
                skip.reason,
            ));
        }
        Inspect::Gone => {
            return Err(Error::msg(format!("file not found: {}", path.display())));
        }
        Inspect::Outside => {
            return Err(Error::msg(format!(
                "{} is not a file in {}",
                path.display(),
                folder.display()
            )));
        }
        Inspect::Candidate(_) => {}
    }

    let view = crate::plan::view_folder_with(&folder, None, now, &bundles, ages)?;
    let plan = plan_entries(
        env,
        &folder,
        view.entries.clone(),
        view.skipped.clone(),
        rules,
        now,
        &view.duplicates,
    )?;
    let entry = plan
        .actions
        .iter()
        .find(|action| action.file.name == name)
        .map(|action| action.file.clone())
        .or_else(|| {
            plan.unmatched
                .iter()
                .find(|file| file.name == name)
                .cloned()
        })
        .ok_or_else(|| Error::msg(format!("could not evaluate {}", child.display())))?;

    let explained = walk_rules(rules, &entry, now, &view.duplicates);
    let winner = explained.iter().find(|rule| rule.mark == RuleMark::Wins);
    let outcome = outcome_for(&plan, &name, winner.map(|rule| rule.name.as_str()));
    Ok(Explanation {
        file_name: name,
        path: child,
        folder,
        rules: explained,
        outcome,
    })
}

fn walk_rules(
    rules: &Ruleset,
    file: &scan::FileEntry,
    now: SystemTime,
    duplicates: &crate::duplicates::DuplicateIndex,
) -> Vec<ExplainedRule> {
    let mut won = false;
    let mut out = Vec::with_capacity(rules.rules.len());
    for rule in &rules.rules {
        if won {
            out.push(ExplainedRule {
                name: rule.name.clone(),
                mark: RuleMark::NotEvaluated,
                conditions: Vec::new(),
            });
            continue;
        }
        let why = rule.explain(file, now, duplicates);
        let mark = if why.matched {
            won = true;
            RuleMark::Wins
        } else {
            RuleMark::Miss
        };
        out.push(ExplainedRule {
            name: rule.name.clone(),
            mark,
            conditions: why.conditions,
        });
    }
    out
}

fn outcome_for(plan: &Plan, name: &str, winner: Option<&str>) -> Outcome {
    let Some(rule) = winner else {
        return Outcome::Unmatched;
    };
    let Some(action) = plan.actions.iter().find(|action| action.file.name == name) else {
        return Outcome::Matched {
            rule: rule.to_string(),
            action: "move".to_string(),
            destination: String::new(),
            destination_path: None,
            copy_of: None,
        };
    };
    Outcome::Matched {
        rule: rule.to_string(),
        action: match action.kind {
            ActionKind::Move => "move",
            ActionKind::Trash => "trash",
        }
        .to_string(),
        destination: action.destination_display.clone(),
        destination_path: action.destination.clone(),
        copy_of: action.copy_of.clone(),
    }
}

fn skipped_explanation(
    name: String,
    path: PathBuf,
    folder: PathBuf,
    reason: SkipReason,
) -> Explanation {
    Explanation {
        file_name: name,
        path,
        folder,
        rules: Vec::new(),
        outcome: Outcome::Skipped {
            reason: reason.as_str().to_string(),
            detail: skip_detail(reason).to_string(),
        },
    }
}

fn skip_detail(reason: SkipReason) -> &'static str {
    match reason {
        SkipReason::Hidden => "hidden files are left in place",
        SkipReason::PartialDownload => "partial downloads are left in place",
        SkipReason::Directory => {
            "directories are left in place (a package whose extension is on a rule is filed as one item)"
        }
        SkipReason::Symlink => "symlinks are left in place",
        SkipReason::NotRegular => "only regular files are tidied",
        SkipReason::NewerThanFilter => "newer than --older-than, so it is left out of the plan",
    }
}

fn parent_dir(path: &Path) -> &Path {
    match path.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent,
        _ => Path::new("."),
    }
}

pub fn render(explanation: &Explanation, env: &Env, color: bool) -> String {
    let mut out = String::new();
    line(&mut out, &dye(&explanation.file_name, color, "1"));
    line(
        &mut out,
        &format!(
            "{}  {}",
            dye("Folder", color, "2"),
            contract_home(&explanation.folder, &env.home)
        ),
    );
    out.push('\n');

    if let Outcome::Skipped { reason, detail } = &explanation.outcome {
        line(&mut out, &dye("Skipped", color, "1"));
        line(&mut out, &format!("  {reason}: {detail}"));
        out.push('\n');
        line(&mut out, &dye("Outcome", color, "1"));
        line(&mut out, &format!("  skipped ({reason}); left in place"));
        return out;
    }

    for (index, rule) in explanation.rules.iter().enumerate() {
        let mark = match rule.mark {
            RuleMark::Wins => format!("  {}", dye("wins", color, "1;32")),
            RuleMark::NotEvaluated => format!("  {}", dye("not evaluated", color, "2")),
            RuleMark::Miss => String::new(),
        };
        line(
            &mut out,
            &format!("{}. {}{mark}", index + 1, dye(&rule.name, color, "1;36")),
        );
        for condition in &rule.conditions {
            let flag = if condition.passed { "pass" } else { "fail" };
            let code = if condition.passed { "32" } else { "33" };
            line(
                &mut out,
                &format!(
                    "   {:<12}  {}  {}",
                    condition.name,
                    dye(&format!("{flag:<4}"), color, code),
                    condition.detail
                ),
            );
        }
    }

    out.push('\n');
    line(&mut out, &dye("Outcome", color, "1"));
    match &explanation.outcome {
        Outcome::Matched {
            rule,
            action,
            destination,
            ..
        } => {
            line(&mut out, &format!("  {:<12} {rule}", "rule"));
            line(&mut out, &format!("  {:<12} {action}", "action"));
            line(&mut out, &format!("  {:<12} {destination}", "destination"));
        }
        Outcome::Unmatched => line(&mut out, "  no rule matches; left in place"),
        Outcome::Skipped { .. } => {}
    }
    out
}

pub fn render_json(explanation: &Explanation) -> Result<String, Error> {
    let skipped = match &explanation.outcome {
        Outcome::Skipped { reason, detail } => serde_json::json!({
            "reason": reason,
            "detail": detail,
        }),
        _ => serde_json::Value::Null,
    };
    let rules: Vec<serde_json::Value> = explanation
        .rules
        .iter()
        .map(|rule| {
            let conditions: Vec<serde_json::Value> = rule
                .conditions
                .iter()
                .map(|condition| {
                    let mut value = serde_json::json!({
                        "condition": condition.name,
                        "pass": condition.passed,
                        "detail": condition.detail,
                    });
                    if condition.name == "older_than" {
                        if let Some(source) = age_source_from_detail(&condition.detail) {
                            value["age_source"] = serde_json::Value::String(source.to_string());
                        }
                    }
                    value
                })
                .collect();
            serde_json::json!({
                "name": rule.name,
                "result": rule.mark.as_str(),
                "conditions": conditions,
            })
        })
        .collect();
    let outcome = match &explanation.outcome {
        Outcome::Skipped { reason, detail } => serde_json::json!({
            "status": "skipped",
            "reason": reason,
            "detail": detail,
        }),
        Outcome::Matched {
            rule,
            action,
            destination,
            destination_path,
            copy_of,
        } => serde_json::json!({
            "status": "matched",
            "rule": rule,
            "action": action,
            "destination": destination,
            "destination_path": destination_path.as_ref().map(|p| p.display().to_string()),
            "copy_of": copy_of,
        }),
        Outcome::Unmatched => serde_json::json!({
            "status": "unmatched",
            "detail": "no rule matches; left in place",
        }),
    };
    let value = serde_json::json!({
        "version": 1,
        "file": explanation.file_name,
        "path": explanation.path.display().to_string(),
        "folder": explanation.folder.display().to_string(),
        "skipped": skipped,
        "rules": rules,
        "outcome": outcome,
    });
    serde_json::to_string_pretty(&value).map_err(|err| Error::msg(format!("json: {err}")))
}

fn age_source_from_detail(detail: &str) -> Option<&'static str> {
    if detail.contains("(date added)") {
        Some("date added")
    } else if detail.contains("(created)") {
        Some("created")
    } else if detail.contains("(modified)") {
        Some("modified")
    } else {
        None
    }
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
