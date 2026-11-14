use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::thread;
use std::time::Duration;

use eframe::egui;

use crate::git::Repo;

/// How often the working tree is looked at, so changes made by an agent show up on their own.
const REFRESH_INTERVAL: Duration = Duration::from_secs(2);

/// What the editor shows of the working tree.
#[derive(Clone, PartialEq)]
pub struct Snapshot {
    pub diff: String,
    pub files: Vec<String>,
}

impl Snapshot {
    pub fn take(repo: &Repo) -> Result<Self, String> {
        Ok(Self { diff: repo.raw_diff()?, files: repo.list_files()? })
    }
}

/// Runs git on a background thread, so a slow repository never blocks the UI.
pub struct Watcher {
    snapshots: Receiver<Result<Snapshot, String>>,
    wake: Sender<()>,
}

impl Watcher {
    pub fn spawn(repo: Repo, ctx: egui::Context) -> Self {
        let (sender, snapshots) = mpsc::channel();
        let (wake, woken) = mpsc::channel();
        thread::spawn(move || {
            let mut last = None;
            loop {
                let snapshot = Snapshot::take(&repo);
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
        Self { snapshots, wake }
    }

    /// The newest snapshot, if the working tree changed since the last call.
    pub fn changed(&self) -> Option<Result<Snapshot, String>> {
        self.snapshots.try_iter().last()
    }

    pub fn refresh_now(&self) {
        let _ = self.wake.send(());
    }
}
