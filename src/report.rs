//! Text tables and JSON for plans and undo reports.

use std::collections::BTreeMap;

use unicode_width::UnicodeWidthStr;

use crate::env::{contract_home, Env};
use crate::error::Error;
use crate::parse::format_size;
use crate::plan::{ActionKind, ActionStatus, Plan};
use crate::scan::SkipReason;
use crate::undo::{UndoOutcome, UndoReport};

pub fn render_plan(plan: &Plan, env: &Env, color: bool) -> String {
    let mut out = String::new();
    line(
        &mut out,
        &format!(
            "{}  {}",
            dye("Folder", color, "2"),
            contract_home(&plan.folder, &env.home)
        ),
    );
    line(
        &mut out,
        &format!("{}   {}", dye("Rules", color, "2"), plan.rules_source_label),
    );
    out.push('\n');

    if plan.actions.is_empty() {
        line(&mut out, "No files matched a rule.");
    } else {
        out.push_str(&render_table(plan, color));
        out.push('\n');
    }

    out.push_str(&render_summary(plan, color));

    if !plan.unmatched.is_empty() {
        out.push('\n');
        line(&mut out, &dye("Unmatched  (left in place)", color, "1"));
        for file in plan.unmatched.iter().take(20) {
            line(
                &mut out,
                &format!("  {:<24} {}", file.name, format_size(file.size)),
            );
        }
        if plan.unmatched.len() > 20 {
            line(
                &mut out,
                &format!("  … and {} more", plan.unmatched.len() - 20),
            );
        }
    }

    if !plan.skipped.is_empty() {
        out.push('\n');
        line(&mut out, &dye("Skipped", color, "1"));
        for skip in plan.skipped.iter().take(20) {
            let name = match skip.reason {
                SkipReason::Directory => format!("{}/", skip.name),
                _ => skip.name.clone(),
            };
            line(
                &mut out,
                &format!("  {:<24} {}", name, skip.reason.as_str()),
            );
        }
        if plan.skipped.len() > 20 {
            line(
                &mut out,
                &format!("  … and {} more", plan.skipped.len() - 20),
            );
        }
    }

    let errors: Vec<_> = plan
        .actions
        .iter()
        .filter_map(|action| {
            action
                .error
                .as_ref()
                .map(|err| (action.file.name.clone(), err))
        })
        .collect();
    if !errors.is_empty() {
        out.push('\n');
        line(&mut out, &dye("Errors", color, "1;33"));
        for (name, err) in errors {
            line(&mut out, &format!("  {name}: {err}"));
        }
    }

    out.push('\n');
    if !plan.applied {
        if plan.actions.is_empty() {
            line(&mut out, "Nothing to do.");
        } else {
            line(&mut out, &dye("Dry run. Nothing was changed.", color, "1"));
            line(&mut out, "Re-run with --apply to perform these actions.");
        }
    } else {
        let applied = plan
            .actions
            .iter()
            .filter(|a| a.status == ActionStatus::Applied)
            .count();
        let failed = plan.failure_count();
        if applied == 0 && failed == 0 {
            line(&mut out, "Nothing to do.");
        } else {
            line(
                &mut out,
                &format!(
                    "Applied {applied} {} ({}).",
                    plural(applied, "action", "actions"),
                    format_size(plan.applied_size())
                ),
            );
            if failed > 0 {
                line(
                    &mut out,
                    &format!("{failed} {}.", plural(failed, "error", "errors")),
                );
            }
            if let Some(path) = &plan.log_path {
                line(&mut out, &format!("Undo log  {}", path.display()));
                line(&mut out, "Run `tidy undo` to reverse this run.");
            }
        }
    }
    out
}

pub fn render_plan_json(plan: &Plan) -> Result<String, Error> {
    let entries: Vec<serde_json::Value> = plan
        .actions
        .iter()
        .map(|action| {
            serde_json::json!({
                "file": action.file.name,
                "path": action.file.path.display().to_string(),
                "size": action.file.size,
                "rule": action.rule_name,
                "action": match action.kind {
                    ActionKind::Move => "move",
                    ActionKind::Trash => "trash",
                },
                "destination": action.destination.as_ref().map(|p| p.display().to_string()),
                "destination_display": action.destination_display,
                "status": match action.status {
                    ActionStatus::Planned => "planned",
                    ActionStatus::Applied => "applied",
                    ActionStatus::Failed => "error",
                },
                "error": action.error,
            })
        })
        .collect();
    let unmatched: Vec<serde_json::Value> = plan
        .unmatched
        .iter()
        .map(|file| {
            serde_json::json!({
                "file": file.name,
                "path": file.path.display().to_string(),
                "size": file.size,
            })
        })
        .collect();
    let skipped: Vec<serde_json::Value> = plan
        .skipped
        .iter()
        .map(|skip| {
            serde_json::json!({
                "file": skip.name,
                "path": skip.path.display().to_string(),
                "reason": skip.reason.as_str(),
            })
        })
        .collect();

    let mut by_rule = BTreeMap::<String, usize>::new();
    let mut by_action = BTreeMap::<String, usize>::new();
    for action in &plan.actions {
        if action.status == ActionStatus::Failed {
            continue;
        }
        *by_rule.entry(action.rule_name.clone()).or_insert(0) += 1;
        let key = match action.kind {
            ActionKind::Move => "move",
            ActionKind::Trash => "trash",
        };
        *by_action.entry(key.to_string()).or_insert(0) += 1;
    }

    let value = serde_json::json!({
        "version": 1,
        "mode": if plan.applied { "applied" } else { "dry-run" },
        "folder": plan.folder.display().to_string(),
        "rules_source": plan.rules_source_label,
        "rules_path": plan.rules_path.as_ref().map(|p| p.display().to_string()),
        "entries": entries,
        "unmatched": unmatched,
        "skipped": skipped,
        "summary": {
            "matched": plan.actions.len(),
            "unmatched": plan.unmatched.len(),
            "skipped": plan.skipped.len(),
            "total_size": plan.matched_size(),
            "applied": plan.actions.iter().filter(|a| a.status == ActionStatus::Applied).count(),
            "errors": plan.failure_count(),
            "by_rule": by_rule,
            "by_action": by_action,
        },
        "log_path": plan.log_path.as_ref().map(|p| p.display().to_string()),
        "errors": plan.actions.iter().filter_map(|a| a.error.clone()).collect::<Vec<_>>(),
    });
    serde_json::to_string_pretty(&value).map_err(|err| Error::msg(format!("json: {err}")))
}

pub fn render_undo(report: &UndoReport, env: &Env, color: bool) -> String {
    let mut out = String::new();
    let done = report
        .items
        .iter()
        .filter(|item| matches!(item.outcome, UndoOutcome::MovedBack | UndoOutcome::Restored))
        .count();
    line(
        &mut out,
        &format!(
            "Undid {} from {} ({} put back).",
            dye(&report.log_id, color, "1"),
            contract_home(&report.source_folder, &env.home),
            done
        ),
    );
    for item in &report.items {
        let label = match item.outcome {
            UndoOutcome::MovedBack => dye("moved back", color, "32"),
            UndoOutcome::Restored => dye("restored", color, "32"),
            UndoOutcome::Manual => dye("manual", color, "33"),
            UndoOutcome::Failed => dye("failed", color, "33"),
        };
        line(&mut out, &format!("  {:<24} {}", item.file_name, label));
        line(&mut out, &format!("    {}", item.message));
    }
    out.push('\n');
    if report.manual_count() > 0 {
        line(
            &mut out,
            "Some files are still in the Trash. On macOS, tidy cannot put them back for you.",
        );
        line(
            &mut out,
            "Open the Trash and move those files back to their original location.",
        );
    }
    if report.fully_undone {
        line(
            &mut out,
            &format!("Log marked undone: {}", report.log_path.display()),
        );
    } else {
        line(
            &mut out,
            "Some files were not reversed. Run `tidy undo` again to retry the ones that failed.",
        );
    }
    out
}

pub fn render_undo_json(report: &UndoReport) -> Result<String, Error> {
    let entries: Vec<serde_json::Value> = report
        .items
        .iter()
        .map(|item| {
            serde_json::json!({
                "file": item.file_name,
                "path": item.path,
                "result": match item.outcome {
                    UndoOutcome::MovedBack => "moved-back",
                    UndoOutcome::Restored => "restored",
                    UndoOutcome::Manual => "manual",
                    UndoOutcome::Failed => "failed",
                },
                "message": item.message,
            })
        })
        .collect();
    let value = serde_json::json!({
        "version": 1,
        "log": report.log_path.display().to_string(),
        "id": report.log_id,
        "source_folder": report.source_folder.display().to_string(),
        "undone": report.fully_undone,
        "entries": entries,
        "manual": report.manual_count(),
        "errors": report.failure_count(),
    });
    serde_json::to_string_pretty(&value).map_err(|err| Error::msg(format!("json: {err}")))
}

fn render_table(plan: &Plan, color: bool) -> String {
    let headers = ["file", "rule", "action", "destination", "size"];
    let rows: Vec<[String; 5]> = plan
        .actions
        .iter()
        .map(|action| {
            let action_name = match action.kind {
                ActionKind::Move => "move",
                ActionKind::Trash => "trash",
            };
            let action_name = if action.status == ActionStatus::Failed {
                "error"
            } else {
                action_name
            };
            [
                action.file.name.clone(),
                action.rule_name.clone(),
                action_name.to_string(),
                action.destination_display.clone(),
                format_size(action.file.size),
            ]
        })
        .collect();

    let mut widths = headers.map(width);
    for row in &rows {
        for (index, cell) in row.iter().enumerate() {
            widths[index] = widths[index].max(width(cell));
        }
    }

    let mut out = String::new();
    out.push_str(&paint_row(
        &headers.map(str::to_string),
        &widths,
        color,
        true,
    ));
    out.push('\n');
    out.push_str(&separator(&widths));
    out.push('\n');
    for (index, row) in rows.iter().enumerate() {
        let action = &plan.actions[index];
        out.push_str(&paint_action_row(
            row,
            &widths,
            action.kind,
            action.status,
            color,
        ));
        out.push('\n');
    }
    out
}

fn render_summary(plan: &Plan, color: bool) -> String {
    let mut by_rule: BTreeMap<&str, (usize, u64)> = BTreeMap::new();
    let mut moves = (0usize, 0u64);
    let mut trashes = (0usize, 0u64);
    for action in &plan.actions {
        if action.status == ActionStatus::Failed && plan.applied {
            continue;
        }
        let slot = by_rule.entry(action.rule_name.as_str()).or_insert((0, 0));
        slot.0 += 1;
        slot.1 += action.file.size;
        match action.kind {
            ActionKind::Move => {
                moves.0 += 1;
                moves.1 += action.file.size;
            }
            ActionKind::Trash => {
                trashes.0 += 1;
                trashes.1 += action.file.size;
            }
        }
    }

    let mut out = String::new();
    line(&mut out, &dye("Summary", color, "1"));
    line(
        &mut out,
        &format!(
            "  matched    {:>4}   {}",
            plan.actions.len(),
            format_size(plan.matched_size())
        ),
    );
    line(
        &mut out,
        &format!("  unmatched  {:>4}   left in place", plan.unmatched.len()),
    );
    line(&mut out, &format!("  skipped    {:>4}", plan.skipped.len()));
    if moves.0 > 0 || trashes.0 > 0 {
        out.push('\n');
        line(&mut out, &dye("By action", color, "1"));
        line(
            &mut out,
            &format!("  {:<8} {:>4}   {}", "move", moves.0, format_size(moves.1)),
        );
        line(
            &mut out,
            &format!(
                "  {:<8} {:>4}   {}",
                "trash",
                trashes.0,
                format_size(trashes.1)
            ),
        );
    }
    if !by_rule.is_empty() {
        out.push('\n');
        line(&mut out, &dye("By rule", color, "1"));
        let name_width = by_rule.keys().map(|name| width(name)).max().unwrap_or(0);
        for (name, (count, size)) in &by_rule {
            line(
                &mut out,
                &format!(
                    "  {}  {:>4}   {}",
                    pad(name, name_width),
                    count,
                    format_size(*size)
                ),
            );
        }
    }
    out
}

fn paint_row(cells: &[String; 5], widths: &[usize; 5], color: bool, header: bool) -> String {
    let painted: Vec<String> = cells
        .iter()
        .enumerate()
        .map(|(index, cell)| {
            let padded = pad(cell, widths[index]);
            if header {
                dye(&padded, color, "1")
            } else {
                padded
            }
        })
        .collect();
    painted.join("  ")
}

fn paint_action_row(
    cells: &[String; 5],
    widths: &[usize; 5],
    kind: ActionKind,
    status: ActionStatus,
    color: bool,
) -> String {
    let action_code = if status == ActionStatus::Failed {
        "33"
    } else {
        match kind {
            ActionKind::Move => "32",
            ActionKind::Trash => "33",
        }
    };
    let painted: Vec<String> = cells
        .iter()
        .enumerate()
        .map(|(index, cell)| {
            let padded = pad(cell, widths[index]);
            match index {
                1 => dye(&padded, color, "36"),
                2 => dye(&padded, color, action_code),
                4 => dye(&padded, color, "2"),
                _ => padded,
            }
        })
        .collect();
    painted.join("  ")
}

fn separator(widths: &[usize; 5]) -> String {
    widths
        .iter()
        .map(|width| "─".repeat(*width))
        .collect::<Vec<_>>()
        .join("  ")
}

fn pad(text: &str, target: usize) -> String {
    let current = width(text);
    if current >= target {
        text.to_string()
    } else {
        format!("{text}{}", " ".repeat(target - current))
    }
}

fn width(text: &str) -> usize {
    UnicodeWidthStr::width(text)
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

fn plural<'a>(count: usize, one: &'a str, many: &'a str) -> &'a str {
    if count == 1 {
        one
    } else {
        many
    }
}
