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
}

impl Default for Settings {
  fn default() -> Self {
    Self { language: None, theme: Theme::Dark, check_updates: true, canvas: (2048, 1536) }
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
    let settings = Settings { language: Some(Language::Es), theme: Theme::Light, check_updates: false, canvas: (800, 600) };
    let json = serde_json::to_string(&settings).unwrap();
    assert_eq!(serde_json::from_str::<Settings>(&json).unwrap(), settings);
    // An older file without the newer fields still loads.
    let old: Settings = serde_json::from_str(r#"{"theme":"light"}"#).unwrap();
    assert_eq!(old, Settings { theme: Theme::Light, ..Settings::default() });
  }
}
