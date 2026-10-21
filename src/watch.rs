//! Watch a folder and tidy files once their size stops changing.
//!
//! Events are debounced with `notify-debouncer-mini`. A file is eligible only
//! after its size has stayed the same for the settle duration (2 seconds
//! unless the caller passes another value). Partial downloads, hidden files,
//! directories, symlinks, and anything that is not a direct child of the
//! folder are ignored. Watch mode always applies the rules.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

use notify_debouncer_mini::notify::RecursiveMode;
use notify_debouncer_mini::{new_debouncer, DebounceEventResult};

use crate::apply::apply_plan;
use crate::env::Env;
use crate::error::Error;
use crate::plan::{plan_entries, Plan};
use crate::rules::Ruleset;
use crate::scan::{self, Inspect};
use crate::trash::Trasher;

/// Tracks paths until their size has been unchanged for `settle`.
#[derive(Debug)]
pub struct Stabilizer {
    pending: HashMap<PathBuf, Pending>,
    settle: Duration,
}

#[derive(Debug, Clone, Copy)]
struct Pending {
    size: u64,
    since: Instant,
}

impl Stabilizer {
    pub fn new(settle: Duration) -> Self {
        Self {
            pending: HashMap::new(),
            settle,
        }
    }

    /// Record a fresh stat.
    ///
    /// `None` drops the path (it disappeared, or it is not eligible). A size
    /// change restarts the settle timer.
    pub fn observe(&mut self, path: PathBuf, size: Option<u64>, now: Instant) {
        let Some(size) = size else {
            self.pending.remove(&path);
            return;
        };
        match self.pending.get_mut(&path) {
            Some(pending) if pending.size == size => {}
            Some(pending) => {
                pending.size = size;
                pending.since = now;
            }
            None => {
                self.pending.insert(path, Pending { size, since: now });
            }
        }
    }

    /// Paths that have been stable long enough.
    ///
    /// `stat` returns the current size, or `None` when the path should be
    /// dropped. A size that no longer matches restarts the timer.
    pub fn drain_ready(
        &mut self,
        now: Instant,
        mut stat: impl FnMut(&Path) -> Option<u64>,
    ) -> Vec<PathBuf> {
        let settle = self.settle;
        let mut ready = Vec::new();
        self.pending.retain(|path, pending| {
            if now.duration_since(pending.since) < settle {
                return true;
            }
            match stat(path) {
                None => false,
                Some(size) if size != pending.size => {
                    pending.size = size;
                    pending.since = now;
                    true
                }
                Some(_) => {
                    ready.push(path.clone());
                    false
                }
            }
        });
        ready.sort();
        ready
    }

    pub fn len(&self) -> usize {
        self.pending.len()
    }

    pub fn is_empty(&self) -> bool {
        self.pending.is_empty()
    }
}

/// Block until Ctrl-C, applying rules to files that settle in `folder`.
///
/// `on_plan` receives each batch after it has been applied, including batches
/// whose files matched no rule.
pub fn watch_folder(
    env: &Env,
    folder: &Path,
    rules: &Ruleset,
    older_than: Option<Duration>,
    settle: Duration,
    trasher: &dyn Trasher,
    mut on_plan: impl FnMut(&Plan),
) -> Result<(), Error> {
    let (tx, rx) = mpsc::channel::<PathBuf>();
    let mut debouncer = new_debouncer(
        Duration::from_millis(400),
        move |result: DebounceEventResult| match result {
            Ok(events) => {
                for event in events {
                    let _ = tx.send(event.path);
                }
            }
            Err(err) => {
                eprintln!("tidy: watch: {err}");
            }
        },
    )
    .map_err(|err| Error::msg(format!("could not start the file watcher: {err}")))?;

    debouncer
        .watcher()
        .watch(folder, RecursiveMode::NonRecursive)
        .map_err(|err| Error::msg(format!("could not watch {}: {err}", folder.display())))?;

    let stop = Arc::new(AtomicBool::new(false));
    let stop_flag = Arc::clone(&stop);
    ctrlc::set_handler(move || {
        stop_flag.store(true, Ordering::SeqCst);
    })
    .map_err(|err| Error::msg(format!("could not install Ctrl-C handler: {err}")))?;

    let mut stabilizer = Stabilizer::new(settle);
    loop {
        if stop.load(Ordering::SeqCst) {
            return Ok(());
        }
        match rx.recv_timeout(Duration::from_millis(200)) {
            Ok(path) => observe_path(&mut stabilizer, folder, &path, older_than),
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => break,
        }
        while let Ok(path) = rx.try_recv() {
            observe_path(&mut stabilizer, folder, &path, older_than);
        }

        let now = SystemTime::now();
        let ready = stabilizer.drain_ready(Instant::now(), |path| {
            eligible_size(folder, path, older_than, now)
        });
        if ready.is_empty() {
            continue;
        }
        let mut entries = Vec::new();
        for path in ready {
            if let Inspect::Candidate(file) = scan::inspect(folder, &path, older_than, now)? {
                entries.push(file);
            }
        }
        if entries.is_empty() {
            continue;
        }
        let mut plan = plan_entries(env, folder, entries, Vec::new(), rules, now)?;
        apply_plan(env, &mut plan, trasher)?;
        on_plan(&plan);
    }

    // `debouncer` has to live until here. Dropping it stops the watcher.
    drop(debouncer);
    Ok(())
}

fn observe_path(
    stabilizer: &mut Stabilizer,
    folder: &Path,
    path: &Path,
    older_than: Option<Duration>,
) {
    let size = eligible_size(folder, path, older_than, SystemTime::now());
    stabilizer.observe(path.to_path_buf(), size, Instant::now());
}

fn eligible_size(
    folder: &Path,
    path: &Path,
    older_than: Option<Duration>,
    now: SystemTime,
) -> Option<u64> {
    match scan::inspect(folder, path, older_than, now).ok()? {
        Inspect::Candidate(file) => Some(file.size),
        Inspect::Skip(_) | Inspect::Outside | Inspect::Gone => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn size_must_stay_still_for_the_settle_window() {
        let mut stabilizer = Stabilizer::new(Duration::from_secs(2));
        let path = PathBuf::from("/tmp/downloads/report.pdf");
        let start = Instant::now();
        stabilizer.observe(path.clone(), Some(10), start);
        assert!(stabilizer
            .drain_ready(start + Duration::from_millis(1900), |_| Some(10))
            .is_empty());
        assert_eq!(stabilizer.len(), 1);

        let ready = stabilizer.drain_ready(start + Duration::from_secs(2), |_| Some(10));
        assert_eq!(ready, vec![path]);
        assert!(stabilizer.is_empty());
    }

    #[test]
    fn a_growing_file_restarts_the_timer() {
        let mut stabilizer = Stabilizer::new(Duration::from_secs(2));
        let path = PathBuf::from("/tmp/downloads/video.mp4");
        let start = Instant::now();
        stabilizer.observe(path.clone(), Some(10), start);
        stabilizer.observe(path.clone(), Some(50), start + Duration::from_millis(1500));
        assert!(stabilizer
            .drain_ready(start + Duration::from_secs(2), |_| Some(50))
            .is_empty());
        let ready = stabilizer.drain_ready(start + Duration::from_millis(3500), |_| Some(50));
        assert_eq!(ready, vec![path]);
    }

    #[test]
    fn ineligible_and_disappeared_paths_are_dropped() {
        let mut stabilizer = Stabilizer::new(Duration::from_millis(10));
        let start = Instant::now();
        let partial = PathBuf::from("/tmp/downloads/file.crdownload");
        let gone = PathBuf::from("/tmp/downloads/gone.pdf");
        stabilizer.observe(partial.clone(), None, start);
        stabilizer.observe(gone.clone(), Some(4), start);
        assert!(stabilizer.len() == 1);
        let ready = stabilizer.drain_ready(start + Duration::from_millis(20), |path| {
            if path == gone {
                None
            } else {
                Some(4)
            }
        });
        assert!(ready.is_empty());
        assert!(stabilizer.is_empty());
    }
}
