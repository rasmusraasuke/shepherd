use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::thread;
use std::time::Duration;

use eframe::egui;

use crate::git::{Changes, Repo, Status};

const REFRESH_INTERVAL: Duration = Duration::from_secs(2);

#[derive(Clone, PartialEq)]
pub struct Snapshot {
    pub changes: Changes,
    pub files: Vec<String>,
    pub status: Status,
}

impl Snapshot {
    pub fn take(repo: &Repo) -> Result<Self, String> {
        Ok(Self {
            changes: repo.changes()?,
            files: repo.list_files()?,
            status: repo.status(),
        })
    }
}

pub struct Watcher {
    snapshots: Receiver<(u64, Result<Snapshot, String>)>,
    wake: Sender<()>,
    refreshes: Arc<AtomicU64>,
}

impl Watcher {
    pub fn spawn(repo: Repo, ctx: egui::Context) -> Self {
        let (sender, snapshots) = mpsc::channel();
        let (wake, woken) = mpsc::channel();
        let refreshes = Arc::new(AtomicU64::new(0));
        let asked = refreshes.clone();
        thread::spawn(move || {
            let mut last = None;
            loop {
                let refreshes = asked.load(Ordering::SeqCst);
                let snapshot = (refreshes, Snapshot::take(&repo));
                if last.as_ref() != Some(&snapshot) {
                    if sender.send(snapshot.clone()).is_err() {
                        break;
                    }
                    ctx.request_repaint();
                    last = Some(snapshot);
                }
                if woken.recv_timeout(REFRESH_INTERVAL) == Err(RecvTimeoutError::Disconnected) {
                    break;
                }
            }
        });
        Self {
            snapshots,
            wake,
            refreshes,
        }
    }

    pub fn changed(&self) -> Option<Result<Snapshot, String>> {
        let refreshes = self.refreshes.load(Ordering::SeqCst);
        self.snapshots
            .try_iter()
            .filter(|(started_after, _)| *started_after == refreshes)
            .last()
            .map(|(_, snapshot)| snapshot)
    }

    pub fn refresh_now(&self) {
        self.refreshes.fetch_add(1, Ordering::SeqCst);
        let _ = self.wake.send(());
    }
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use super::*;
    use crate::git::tests::temp_repo;

    fn next(watcher: &Watcher) -> Option<Snapshot> {
        let start = Instant::now();
        while start.elapsed() < Duration::from_secs(1) {
            if let Some(snapshot) = watcher.changed() {
                return snapshot.ok();
            }
            thread::sleep(Duration::from_millis(5));
        }
        None
    }

    #[test]
    fn hands_over_a_refresh_even_when_it_was_seen_before_it_was_asked_for() {
        let (repo, dir) = temp_repo("watcher-refresh", &[("a.txt", "a\n")]);
        let watcher = Watcher::spawn(repo, egui::Context::default());
        assert!(next(&watcher).is_some());
        dir.write("b.txt", "b\n");
        watcher.refresh_now();
        thread::sleep(Duration::from_millis(300));
        watcher.refresh_now();
        let snapshot = next(&watcher).expect("no snapshot after the refresh");
        assert!(snapshot.files.contains(&"b.txt".to_string()));
    }
}
