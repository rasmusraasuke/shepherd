use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::thread;
use std::time::Duration;

use eframe::egui;

use crate::git::{Changes, Repo};

/// How often the working tree is looked at, so changes made by an agent show up on their own.
const REFRESH_INTERVAL: Duration = Duration::from_secs(2);

/// What the editor shows of the working tree.
#[derive(Clone, PartialEq)]
pub struct Snapshot {
    pub changes: Changes,
    pub files: Vec<String>,
}

impl Snapshot {
    pub fn take(repo: &Repo) -> Result<Self, String> {
        Ok(Self { changes: repo.changes()?, files: repo.list_files()? })
    }
}

/// Runs git on a background thread, so a slow repository never blocks the UI.
pub struct Watcher {
    /// The snapshots, each with the number of refreshes asked for before it was started.
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
                let snapshot = Snapshot::take(&repo);
                if last.as_ref() != Some(&snapshot) {
                    if sender.send((refreshes, snapshot.clone())).is_err() {
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
        Self { snapshots, wake, refreshes }
    }

    /// The newest snapshot, if the working tree changed since the last call. Snapshots started
    /// before the last refresh was asked for are left out: they may miss what the editor just
    /// changed, such as what is staged.
    pub fn changed(&self) -> Option<Result<Snapshot, String>> {
        let refreshes = self.refreshes.load(Ordering::SeqCst);
        self.snapshots.try_iter().filter(|(started_after, _)| *started_after == refreshes).last().map(|(_, snapshot)| snapshot)
    }

    pub fn refresh_now(&self) {
        self.refreshes.fetch_add(1, Ordering::SeqCst);
        let _ = self.wake.send(());
    }
}
