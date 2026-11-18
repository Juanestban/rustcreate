//! Runs the updater on a background thread: checks at startup and every 5 hours, and installs
//! only when asked (ADR 0002). The UI reads `status()` each frame.

use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use eframe::egui;

use crate::update::{self, Install, Update, Updater};

const CHECK_EVERY: Duration = Duration::from_secs(5 * 60 * 60);

#[derive(Debug, Clone, Default)]
pub enum Status {
  #[default]
  Idle,
  Available(Update),
  Downloading(f32),
  Failed(String),
  /// The new version is starting: close now so its files can be replaced.
  Quit,
}

pub struct Updates {
  status: Arc<Mutex<Status>>,
  install: Sender<Update>,
}

impl Updates {
  pub fn start(ctx: egui::Context) -> Self {
    let status = Arc::new(Mutex::new(Status::Idle));
    let (install, requests) = mpsc::channel::<Update>();
    let shared = status.clone();
    std::thread::spawn(move || {
      let Ok(runtime) = tokio::runtime::Builder::new_current_thread().enable_all().build() else { return };
      let Ok(updater) = Updater::new() else { return };
      let kind = Install::detect();
      let set = |value: Status| {
        *shared.lock().unwrap() = value;
        ctx.request_repaint();
      };
      loop {
        // A failed check stays quiet: it is retried on the next round.
        if let Ok(Some(found)) = runtime.block_on(updater.check(kind)) {
          set(Status::Available(found));
        }
        match requests.recv_timeout(CHECK_EVERY) {
          Ok(chosen) => {
            set(Status::Downloading(0.0));
            let result = runtime.block_on(updater.download(&chosen, |p| set(Status::Downloading(p))));
            match result.and_then(|path| update::apply(&chosen, &path)) {
              Ok(()) => return set(Status::Quit),
              Err(error) => set(Status::Failed(error.to_string())),
            }
          }
          Err(RecvTimeoutError::Timeout) => {}
          Err(RecvTimeoutError::Disconnected) => return,
        }
      }
    });
    Self { status, install }
  }

  pub fn status(&self) -> Status {
    self.status.lock().unwrap().clone()
  }

  pub fn install(&self, update: Update) {
    let _ = self.install.send(update);
  }

  /// "Later": hide the banner until the next check finds a version.
  pub fn dismiss(&self) {
    *self.status.lock().unwrap() = Status::Idle;
  }
}
