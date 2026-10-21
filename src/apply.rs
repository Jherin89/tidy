//! Perform a plan. Directories are created here, not during planning.

use std::collections::HashSet;
use std::path::Path;

use crate::env::{contract_home, Env};
use crate::error::Error;
use crate::logbook;
use crate::plan::{choose_destination, occupied, ActionKind, ActionStatus, Plan};
use crate::relocate::relocate;
use crate::trash::Trasher;

/// Execute `plan`. Successful actions are recorded in an undo log.
/// Per-file failures are stored on the action and do not abort the rest.
pub fn apply_plan(env: &Env, plan: &mut Plan, trasher: &dyn Trasher) -> Result<(), Error> {
    let mut reserved = HashSet::new();
    for action in &mut plan.actions {
        match action.kind {
            ActionKind::Move => apply_move(env, action, &mut reserved),
            ActionKind::Trash => apply_trash(action, trasher),
        }
    }
    plan.applied = true;
    if let Some(path) = logbook::write_run_log(env, plan)? {
        plan.log_path = Some(path);
    }
    Ok(())
}

fn apply_move(
    env: &Env,
    action: &mut crate::plan::PlannedAction,
    reserved: &mut HashSet<std::path::PathBuf>,
) {
    let Some(planned) = action.destination.clone() else {
        fail(action, "move is missing a destination");
        return;
    };
    let parent = planned
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| Path::new(".").to_path_buf());
    let dest = if !reserved.contains(&planned) && !occupied(&planned) {
        reserved.insert(planned.clone());
        planned
    } else {
        match choose_destination(&parent, &action.file.name, reserved) {
            Ok(path) => path,
            Err(err) => {
                fail(action, err.to_string());
                return;
            }
        }
    };
    match relocate(&action.file.path, &dest) {
        Ok(()) => {
            action.destination_display = contract_home(&dest, &env.home);
            action.destination = Some(dest);
            action.status = ActionStatus::Applied;
            action.error = None;
        }
        Err(err) => fail(action, err.to_string()),
    }
}

fn apply_trash(action: &mut crate::plan::PlannedAction, trasher: &dyn Trasher) {
    match trasher.trash(&action.file.path) {
        Ok(record) => {
            action.destination = record.fake_path;
            action.trashed_at_unix = Some(record.trashed_at_unix);
            action.status = ActionStatus::Applied;
            action.error = None;
        }
        Err(err) => fail(action, err.to_string()),
    }
}

fn fail(action: &mut crate::plan::PlannedAction, message: impl Into<String>) {
    action.status = ActionStatus::Failed;
    action.error = Some(message.into());
}
