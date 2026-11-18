//! The user's color palettes, kept in `palettes.json` next to the settings.

use serde::{Deserialize, Serialize};

use crate::settings::config_file;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Palette {
  pub name: String,
  pub colors: Vec<[u8; 3]>,
}

pub struct Palettes {
  pub list: Vec<Palette>,
  /// What is on disk, to save only when something changed.
  saved: Vec<Palette>,
}

impl Palettes {
  /// The saved palettes, or one starter palette named `starter` the first time.
  pub fn load(starter: &str) -> Self {
    let list =
      config_file("palettes.json").and_then(|path| std::fs::read(path).ok()).and_then(|bytes| serde_json::from_slice::<Vec<Palette>>(&bytes).ok());
    Self::from_saved(list, starter)
  }

  fn from_saved(list: Option<Vec<Palette>>, starter: &str) -> Self {
    let saved = list.unwrap_or_default();
    let list = if saved.is_empty() { vec![Self::starter(starter)] } else { saved.clone() };
    Self { list, saved }
  }

  /// A small original set: neutrals, then warm to cool.
  fn starter(name: &str) -> Palette {
    let colors = vec![
      [20, 20, 24],
      [96, 96, 104],
      [200, 200, 206],
      [250, 248, 244],
      [214, 58, 52],
      [240, 132, 48],
      [246, 206, 72],
      [92, 168, 84],
      [44, 150, 160],
      [56, 102, 206],
      [118, 76, 186],
      [214, 92, 156],
    ];
    Palette { name: name.to_owned(), colors }
  }

  /// Writes `palettes.json` if anything changed since the last save.
  pub fn save_if_changed(&mut self) {
    if self.list == self.saved {
      return;
    }
    if let Some(path) = config_file("palettes.json") {
      if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
      }
      let _ = std::fs::write(path, serde_json::to_vec_pretty(&self.list).expect("palettes serialize"));
    }
    self.saved = self.list.clone();
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn starts_with_one_palette_and_keeps_edits() {
    let mut palettes = Palettes::from_saved(None, "Basics");
    assert_eq!(palettes.list.len(), 1);
    assert_ne!(palettes.list, palettes.saved, "the starter is saved on the first change check");
    palettes.list[0].colors.push([1, 2, 3]);
    let reloaded = Palettes::from_saved(Some(palettes.list.clone()), "Basics");
    assert_eq!(reloaded.list[0].colors.last(), Some(&[1, 2, 3]));
    assert_eq!(reloaded.list, reloaded.saved, "nothing to save after loading");
  }
}
