//! `rustcreate.log` next to the settings (`%APPDATA%\rustcreate` on Windows): the steps of this
//! run, every panic with its backtrace, and a note when the UI stops answering. A release build on
//! Windows has no console, so this file is how a failure gets reported. The previous run's log is
//! kept as `rustcreate.previous.log`, so restarting after a hang does not lose it.

use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use crate::settings;

/// The UI counts as stuck after this long.
const STALL: Duration = Duration::from_secs(15);

/// Milliseconds since the start when the UI pass last began or ended.
static LAST_CHANGE: AtomicU64 = AtomicU64::new(0);
/// A UI pass is running (between `ui_started` and `ui_finished`).
static IN_UI: AtomicBool = AtomicBool::new(false);
/// The first frame was painted, so the window shows (eframe keeps it hidden until then).
static SHOWN: AtomicBool = AtomicBool::new(false);
/// The last step `line` wrote, named when the UI is stuck.
static LAST_STEP: Mutex<String> = Mutex::new(String::new());
static BEGIN: OnceLock<Instant> = OnceLock::new();

pub fn path() -> Option<PathBuf> {
  settings::config_file("rustcreate.log")
}

/// Starts this run's log (keeping the last one), logs panics, and watches the UI thread.
pub fn start() {
  if let Some(path) = path() {
    if let Some(dir) = path.parent() {
      let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::rename(&path, path.with_file_name("rustcreate.previous.log"));
  }
  line(&format!("start: rustcreate {} on {} {}", env!("CARGO_PKG_VERSION"), std::env::consts::OS, std::env::consts::ARCH));

  let default_hook = std::panic::take_hook();
  std::panic::set_hook(Box::new(move |info| {
    let thread = std::thread::current().name().unwrap_or("unnamed").to_owned();
    line(&format!("panic on thread '{thread}': {info}\n{}", std::backtrace::Backtrace::force_capture()));
    default_hook(info);
    // On Windows, winit catches a panic inside its window procedure and keeps the process alive
    // with no window (eframe shows it only after the first frame): say what broke, then end it.
    if thread == "main" {
      let log = path().map_or_else(String::new, |p| format!("\n\n{}", p.display()));
      rfd::MessageDialog::new().set_level(rfd::MessageLevel::Error).set_title("rustcreate").set_description(format!("{info}{log}")).show();
      std::process::exit(1);
    }
  }));

  let begin = *BEGIN.get_or_init(Instant::now);
  std::thread::Builder::new()
    .name("watchdog".into())
    .spawn(move || {
      let mut stuck = false;
      loop {
        std::thread::sleep(Duration::from_secs(1));
        let idle = Duration::from_millis(begin.elapsed().as_millis() as u64 - LAST_CHANGE.load(Ordering::Relaxed));
        // eframe sleeps while nothing changes, so only a window not shown yet or a UI pass that
        // does not end counts as stuck.
        let busy = !SHOWN.load(Ordering::Relaxed) || IN_UI.load(Ordering::Relaxed);
        if busy && idle > STALL && !stuck {
          stuck = true;
          let step = LAST_STEP.lock().map(|s| s.clone()).unwrap_or_default();
          line(&format!(
            "stuck: nothing for {} s, the window is {} (last step: {step})",
            idle.as_secs(),
            if SHOWN.load(Ordering::Relaxed) { "shown" } else { "not shown yet" }
          ));
        } else if stuck && !busy {
          stuck = false;
          line("unstuck");
        }
      }
    })
    .ok();
}

fn touch() {
  if let Some(begin) = BEGIN.get() {
    LAST_CHANGE.store(begin.elapsed().as_millis() as u64, Ordering::Relaxed);
  }
}

/// The start of `App::ui`; `frame` is egui's frame number. Frame 1 means frame 0 was painted
/// and the window shown.
pub fn ui_started(frame: u64) {
  IN_UI.store(true, Ordering::Relaxed);
  touch();
  if frame == 0 {
    line("first frame: UI");
  } else if frame == 1 && !SHOWN.swap(true, Ordering::Relaxed) {
    line("first frame painted: window shown");
  }
}

pub fn ui_finished() {
  IN_UI.store(false, Ordering::Relaxed);
  touch();
}
/// Appends a line, with the Unix time.
pub fn line(text: &str) {
  if let Ok(mut step) = LAST_STEP.lock() {
    *step = text.lines().next().unwrap_or_default().to_owned();
  }
  let Some(path) = path() else { return };
  let Ok(mut file) = std::fs::OpenOptions::new().create(true).append(true).open(path) else { return };
  let seconds = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs());
  let _ = writeln!(file, "{seconds} {text}");
}
