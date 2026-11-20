//! User preferences, kept as a small JSON file in the per-user config folder. A missing or
//! unreadable file means the defaults; a failed write is ignored (the app still works).

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::i18n::Language;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Theme {
  System,
  #[default]
  Dark,
  Light,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
  /// `None` follows the system language.
  pub language: Option<Language>,
  pub theme: Theme,
  /// Check GitHub Releases at startup and every 5 hours (ADR 0002).
  pub check_updates: bool,
  /// The last size picked in "New canvas".
  pub canvas: (u32, u32),
  /// Holding still at the end of a stroke snaps it to a clean shape.
  pub quickshape: bool,
  /// Files opened or saved lately, newest first (File > Open recent).
  pub recent: Vec<PathBuf>,
  /// Files kept first on the home screen.
  pub pinned: Vec<PathBuf>,
  /// How the home screen's new canvas form measures, and its resolution.
  pub unit: Unit,
  pub dpi: u32,
  /// A new canvas's paper color; `None` = transparent.
  pub background: Option<[u8; 3]>,
  /// Sizes the user saved on the home screen.
  pub sizes: Vec<SavedSize>,
  /// Windows: read the pen through Wintab (older tablets) besides Windows Ink.
  pub wintab: bool,
  /// The right dock's windows (Window menu).
  pub panels: Panels,
}

/// Which windows the right dock shows, top to bottom.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Panels {
  pub options: bool,
  pub color: bool,
  pub brushes: bool,
  pub layers: bool,
}

impl Default for Panels {
  fn default() -> Self {
    Self { options: true, color: true, brushes: true, layers: true }
  }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Unit {
  #[default]
  Pixels,
  Inches,
  Centimeters,
  Millimeters,
}

impl Unit {
  pub const ALL: [Unit; 4] = [Unit::Pixels, Unit::Inches, Unit::Centimeters, Unit::Millimeters];

  /// Units in one inch.
  fn per_inch(self, dpi: u32) -> f64 {
    match self {
      Unit::Pixels => dpi as f64,
      Unit::Inches => 1.0,
      Unit::Centimeters => 2.54,
      Unit::Millimeters => 25.4,
    }
  }

  pub fn to_pixels(self, value: f64, dpi: u32) -> u32 {
    (value * dpi.max(1) as f64 / self.per_inch(dpi.max(1))).round().max(1.0) as u32
  }

  pub fn in_units(self, pixels: u32, dpi: u32) -> f64 {
    pixels as f64 * self.per_inch(dpi.max(1)) / dpi.max(1) as f64
  }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SavedSize {
  pub name: String,
  pub width: u32,
  pub height: u32,
  pub dpi: u32,
}

impl Default for Settings {
  fn default() -> Self {
    Self {
      language: None,
      theme: Theme::Dark,
      check_updates: true,
      canvas: (2048, 1536),
      quickshape: true,
      recent: Vec::new(),
      pinned: Vec::new(),
      unit: Unit::Pixels,
      dpi: 300,
      background: None,
      sizes: Vec::new(),
      wintab: false,
      panels: Panels::default(),
    }
  }
}

impl Settings {
  pub fn load() -> Self {
    path().and_then(|path| std::fs::read(path).ok()).and_then(|bytes| serde_json::from_slice(&bytes).ok()).unwrap_or_default()
  }

  pub fn save(&self) {
    let Some(path) = path() else { return };
    if let Some(dir) = path.parent() {
      let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(path, serde_json::to_vec_pretty(self).expect("settings serialize"));
  }

  pub fn language(&self) -> Language {
    self.language.unwrap_or_else(Language::system)
  }
}

fn path() -> Option<PathBuf> {
  config_file("settings.json")
}

/// `name` in `%APPDATA%\rustcreate`, `~/Library/Application Support/rustcreate` or
/// `$XDG_CONFIG_HOME/rustcreate`.
pub fn config_file(name: &str) -> Option<PathBuf> {
  let base = if cfg!(windows) {
    std::env::var_os("APPDATA").map(PathBuf::from)
  } else if cfg!(target_os = "macos") {
    std::env::home_dir().map(|home| home.join("Library/Application Support"))
  } else {
    std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from).or_else(|| std::env::home_dir().map(|home| home.join(".config")))
  };
  Some(base?.join("rustcreate").join(name))
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn round_trips_and_fills_missing_fields() {
    let settings = Settings {
      language: Some(Language::Es),
      theme: Theme::Light,
      check_updates: false,
      canvas: (800, 600),
      quickshape: false,
      recent: vec!["a.rcreate".into()],
      pinned: vec!["a.rcreate".into()],
      unit: Unit::Centimeters,
      dpi: 150,
      background: None,
      sizes: vec![SavedSize { name: "Card".into(), width: 1050, height: 600, dpi: 300 }],
      wintab: true,
      panels: Panels { color: false, ..Panels::default() },
    };
    let json = serde_json::to_string(&settings).unwrap();
    assert_eq!(serde_json::from_str::<Settings>(&json).unwrap(), settings);
    // An older file without the newer fields still loads.
    let old: Settings = serde_json::from_str(r#"{"theme":"light"}"#).unwrap();
    assert_eq!(old, Settings { theme: Theme::Light, ..Settings::default() });
  }

  #[test]
  fn units_convert_through_the_resolution() {
    // A4 at 300 DPI.
    assert_eq!(Unit::Millimeters.to_pixels(210.0, 300), 2480);
    assert_eq!(Unit::Centimeters.to_pixels(29.7, 300), 3508);
    assert_eq!(Unit::Inches.to_pixels(8.5, 300), 2550);
    assert_eq!(Unit::Pixels.to_pixels(1920.0, 72), 1920);
    assert!((Unit::Inches.in_units(600, 300) - 2.0).abs() < 1e-9);
  }
}
