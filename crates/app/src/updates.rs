//! Runs the updater on a background thread: checks at startup and every 5 hours, and installs
//! only when asked (ADR 0002). The UI reads `status()` each frame.

use std::sync::atomic::{AtomicBool, Ordering};
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
  /// Settings → About → "Check now" is waiting for GitHub.
  Checking,
  /// The last check asked for found nothing newer.
  UpToDate,
  Available(Update),
  Downloading(f32),
  Failed(String),
  /// The new version is starting: close now so its files can be replaced.
  Quit,
}

enum Request {
  Check,
  Install(Update),
}

pub struct Updates {
  status: Arc<Mutex<Status>>,
  requests: Sender<Request>,
  /// Settings → "Check for updates" (ADR 0002).
  checks: Arc<AtomicBool>,
}

impl Updates {
  pub fn start(ctx: egui::Context, checks: bool) -> Self {
    let checks = Arc::new(AtomicBool::new(checks));
    let enabled = checks.clone();
    let status = Arc::new(Mutex::new(Status::Idle));
    let (sender, requests) = mpsc::channel::<Request>();
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
        // shortcut: turning checks back on waits for the next 5 h round, add a wake-up if users ask.
        if enabled.load(Ordering::Relaxed)
          && let Ok(Some(found)) = runtime.block_on(updater.check(kind))
        {
          set(Status::Available(found));
        }
        match requests.recv_timeout(CHECK_EVERY) {
          // Asked for: says so either way, and a failure is shown.
          Ok(Request::Check) => match runtime.block_on(updater.check(kind)) {
            Ok(Some(found)) => set(Status::Available(found)),
            Ok(None) => set(Status::UpToDate),
            Err(error) => set(Status::Failed(error.to_string())),
          },
          Ok(Request::Install(chosen)) => {
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
    Self { status, requests: sender, checks }
  }

  pub fn status(&self) -> Status {
    self.status.lock().unwrap().clone()
  }

  pub fn install(&self, update: Update) {
    let _ = self.requests.send(Request::Install(update));
  }

  /// Checks now, whatever the setting says; `status()` tells the result.
  pub fn check_now(&self) {
    *self.status.lock().unwrap() = Status::Checking;
    let _ = self.requests.send(Request::Check);
  }

  pub fn set_checks(&self, on: bool) {
    self.checks.store(on, Ordering::Relaxed);
  }

  /// "Later": hide the banner until the next check finds a version.
  pub fn dismiss(&self) {
    *self.status.lock().unwrap() = Status::Idle;
  }
}
