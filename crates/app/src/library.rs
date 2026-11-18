//! The brush library: built-in brushes in categories (all original, ADR 0006), the user's own
//! brushes, and which brush each tool (paint, smudge, erase) uses. Edits and the user's brushes
//! are kept in `brushes.json` next to the settings.

use std::collections::HashMap;

use std::path::Path;

use rustcreate_core::{Brush, FileError, Grain, Tip};
use serde::{Deserialize, Serialize};

use crate::settings::config_file;

/// Category name tokens, in library order. The last one holds the user's brushes.
pub const CATEGORIES: [&str; 6] = ["ui-cat-sketching", "ui-cat-inking", "ui-cat-painting", "ui-cat-airbrushing", "ui-cat-textures", "ui-cat-mine"];
pub const MINE: usize = CATEGORIES.len() - 1;

#[derive(Debug, Clone, PartialEq)]
pub enum Name {
  /// A built-in brush's name token.
  Token(&'static str),
  /// A name the user typed.
  Custom(String),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
  /// Stable id, the key in `brushes.json`.
  pub id: String,
  pub name: Name,
  pub category: usize,
  pub brush: Brush,
}

/// Index of each tool's brush: paint, smudge, erase (`Tool as usize`).
pub type Selection = [usize; 3];

pub struct Library {
  pub entries: Vec<Entry>,
  pub selection: Selection,
  /// What is on disk, to save only when something changed.
  saved: Saved,
}

/// The file format: only what differs from the built-ins.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
struct Saved {
  /// Built-in brushes the user changed, by id.
  edited: HashMap<String, Brush>,
  /// The user's own brushes.
  custom: Vec<Custom>,
  /// Brush ids for paint, smudge and erase.
  selected: [String; 3],
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct Custom {
  id: String,
  name: String,
  brush: Brush,
}

fn builtins() -> Vec<Entry> {
  let base = Brush::ink();
  let flat = Brush { taper_start: 0.0, taper_end: 0.0, streamline: 0.1, ..base };
  let list: [(&str, usize, Brush); 20] = [
    ("graphite", 0, Brush::pencil()),
    ("soft-graphite", 0, Brush { size: 10.0, hardness: 0.3, flow: 0.5, grain_depth: 0.75, ..Brush::pencil() }),
    ("technical-pencil", 0, Brush { size: 3.0, hardness: 1.0, flow: 0.9, pressure_size: 0.2, grain: Grain::None, ..Brush::pencil() }),
    (
      "sketch-chalk",
      0,
      Brush {
        size: 16.0,
        tip: Tip::Chalk,
        angle_jitter: 1.0,
        flow: 0.6,
        pressure_size: 0.5,
        pressure_flow: 0.8,
        grain: Grain::Paper,
        grain_depth: 0.4,
        ..flat
      },
    ),
    ("ink", 1, base),
    ("fineliner", 1, Brush { size: 5.0, pressure_size: 0.15, taper_start: 0.0, taper_end: 0.0, streamline: 0.3, ..base }),
    ("brush-pen", 1, Brush { size: 24.0, pressure_size: 1.0, taper_start: 80.0, taper_end: 80.0, streamline: 0.5, ..base }),
    ("dry-ink", 1, Brush { size: 30.0, tip: Tip::Bristles, follow: true, spacing: 0.05, pressure_size: 0.6, ..base }),
    (
      "flat-brush",
      2,
      Brush {
        size: 40.0,
        tip: Tip::Square,
        hardness: 0.7,
        follow: true,
        angle: 90.0,
        spacing: 0.05,
        flow: 0.9,
        pressure_size: 0.3,
        pressure_flow: 0.5,
        ..flat
      },
    ),
    (
      "round-bristle",
      2,
      Brush { size: 36.0, tip: Tip::Bristles, follow: true, spacing: 0.04, flow: 0.8, pressure_size: 0.4, pressure_flow: 0.6, ..flat },
    ),
    ("gouache", 2, Brush { size: 30.0, hardness: 0.75, flow: 0.85, pressure_flow: 0.4, grain: Grain::Canvas, grain_depth: 0.3, ..flat }),
    (
      "dabbing-brush",
      2,
      Brush { size: 50.0, tip: Tip::Chalk, angle_jitter: 1.0, size_jitter: 0.3, spacing: 0.3, flow: 0.7, pressure_size: 0.5, ..flat },
    ),
    ("soft-airbrush", 3, Brush::airbrush()),
    ("hard-airbrush", 3, Brush { size: 60.0, hardness: 0.7, flow: 0.15, ..Brush::airbrush() }),
    (
      "spray",
      3,
      Brush { size: 80.0, tip: Tip::Splatter, angle_jitter: 1.0, size_jitter: 0.5, scatter: 0.4, spacing: 0.15, flow: 0.4, ..Brush::airbrush() },
    ),
    ("blender", 3, Brush { size: 90.0, hardness: 0.0, flow: 0.3, pressure_flow: 1.0, ..Brush::airbrush() }),
    (
      "chalk",
      4,
      Brush {
        size: 40.0,
        tip: Tip::Chalk,
        angle_jitter: 1.0,
        spacing: 0.15,
        flow: 0.8,
        pressure_flow: 0.7,
        grain: Grain::Paper,
        grain_depth: 0.5,
        ..flat
      },
    ),
    (
      "charcoal",
      4,
      Brush { size: 28.0, tip: Tip::Chalk, follow: true, flow: 0.9, pressure_size: 0.4, grain: Grain::Noise, grain_depth: 0.6, ..flat },
    ),
    (
      "pastel",
      4,
      Brush { size: 45.0, hardness: 0.4, flow: 0.6, pressure_flow: 0.8, grain: Grain::Paper, grain_depth: 0.85, grain_scale: 1.5, ..flat },
    ),
    ("canvas-texture", 4, Brush { size: 150.0, hardness: 0.0, flow: 0.5, grain: Grain::Canvas, grain_depth: 1.0, ..Brush::airbrush() }),
  ];
  list.into_iter().map(|(id, category, brush)| Entry { id: id.to_owned(), name: Name::Token(token(id)), category, brush }).collect()
}

/// The name token of a built-in brush id.
fn token(id: &str) -> &'static str {
  match id {
    "graphite" => "ui-brush-graphite",
    "soft-graphite" => "ui-brush-soft-graphite",
    "technical-pencil" => "ui-brush-technical-pencil",
    "sketch-chalk" => "ui-brush-sketch-chalk",
    "ink" => "ui-brush-ink",
    "fineliner" => "ui-brush-fineliner",
    "brush-pen" => "ui-brush-brush-pen",
    "dry-ink" => "ui-brush-dry-ink",
    "flat-brush" => "ui-brush-flat-brush",
    "round-bristle" => "ui-brush-round-bristle",
    "gouache" => "ui-brush-gouache",
    "dabbing-brush" => "ui-brush-dabbing-brush",
    "soft-airbrush" => "ui-brush-soft-airbrush",
    "hard-airbrush" => "ui-brush-hard-airbrush",
    "spray" => "ui-brush-spray",
    "blender" => "ui-brush-blender",
    "chalk" => "ui-brush-chalk",
    "charcoal" => "ui-brush-charcoal",
    "pastel" => "ui-brush-pastel",
    "canvas-texture" => "ui-brush-canvas-texture",
    _ => "ui-brush-ink",
  }
}

/// The folder, next to the settings, that keeps imported tips and grains.
const IMAGES: &str = "brush-images";

fn image_file(grain: bool, id: u32) -> String {
  format!("{}-{id:08x}.png", if grain { "grain" } else { "tip" })
}

/// Registers the tips and grains kept from earlier imports. A damaged one is skipped: its
/// brushes paint round and without grain.
fn register_images() {
  let Some(entries) = config_file(IMAGES).and_then(|dir| std::fs::read_dir(dir).ok()) else { return };
  for path in entries.flatten().map(|e| e.path()) {
    let grain = path.file_name().is_some_and(|n| n.to_string_lossy().starts_with("grain-"));
    if let Ok(bytes) = std::fs::read(&path) {
      let _ = rustcreate_core::load_brush_image(&bytes, grain);
    }
  }
}

/// Paint with Ink, smudge with the Blender, erase with the Hard Airbrush.
const DEFAULT_SELECTION: [&str; 3] = ["ink", "blender", "hard-airbrush"];

impl Default for Library {
  /// The built-ins only, nothing read from disk.
  fn default() -> Self {
    Self::from_saved(Saved::default())
  }
}

impl Library {
  pub fn load() -> Self {
    register_images();
    let saved: Saved =
      config_file("brushes.json").and_then(|path| std::fs::read(path).ok()).and_then(|bytes| serde_json::from_slice(&bytes).ok()).unwrap_or_default();
    Self::from_saved(saved)
  }

  fn from_saved(saved: Saved) -> Self {
    let mut entries = builtins();
    for entry in &mut entries {
      if let Some(brush) = saved.edited.get(&entry.id) {
        entry.brush = *brush;
      }
    }
    entries.extend(saved.custom.iter().map(|c| Entry { id: c.id.clone(), name: Name::Custom(c.name.clone()), category: MINE, brush: c.brush }));
    let mut library = Self { entries, selection: [0; 3], saved: Saved::default() };
    for (tool, default) in DEFAULT_SELECTION.iter().enumerate() {
      library.selection[tool] = library.find(&saved.selected[tool]).or_else(|| library.find(default)).unwrap_or(0);
    }
    library.saved = library.to_saved();
    library
  }

  fn find(&self, id: &str) -> Option<usize> {
    self.entries.iter().position(|e| e.id == id)
  }

  fn to_saved(&self) -> Saved {
    let defaults: HashMap<String, Brush> = builtins().into_iter().map(|e| (e.id, e.brush)).collect();
    let edited = self.entries.iter().filter(|e| defaults.get(&e.id).is_some_and(|b| *b != e.brush)).map(|e| (e.id.clone(), e.brush)).collect();
    let custom = self
      .entries
      .iter()
      .filter_map(|e| match &e.name {
        Name::Custom(name) => Some(Custom { id: e.id.clone(), name: name.clone(), brush: e.brush }),
        Name::Token(_) => None,
      })
      .collect();
    Saved { edited, custom, selected: self.selection.map(|i| self.entries[i].id.clone()) }
  }

  /// Writes `brushes.json` if anything changed since the last save.
  pub fn save_if_changed(&mut self) {
    let now = self.to_saved();
    if now == self.saved {
      return;
    }
    if let Some(path) = config_file("brushes.json") {
      if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
      }
      let _ = std::fs::write(path, serde_json::to_vec_pretty(&now).expect("brushes serialize"));
    }
    self.saved = now;
  }

  pub fn is_builtin(&self, index: usize) -> bool {
    matches!(self.entries[index].name, Name::Token(_))
  }

  /// The built-in brush as shipped, to reset an edited one.
  pub fn original(&self, index: usize) -> Option<Brush> {
    builtins().into_iter().find(|e| e.id == self.entries[index].id).map(|e| e.brush)
  }

  /// Copies a brush into "My brushes" and returns the copy's index.
  pub fn duplicate(&mut self, index: usize, name: String) -> usize {
    self.push_custom(name, self.entries[index].brush)
  }

  fn push_custom(&mut self, name: String, brush: Brush) -> usize {
    let id = (1..).map(|n| format!("custom-{n}")).find(|id| self.find(id).is_none()).expect("a free id");
    self.entries.push(Entry { id, name: Name::Custom(name), category: MINE, brush });
    self.entries.len() - 1
  }

  /// Imports a `.brushset`, `.brush` or `.abr` into My brushes and keeps its images, so the
  /// brushes still work after a restart. Returns the first new brush.
  pub fn import(&mut self, path: &Path) -> Result<usize, FileError> {
    let bytes = std::fs::read(path)?;
    let set = path.file_stem().map_or("Brush".into(), |s| s.to_string_lossy().into_owned());
    let imported = match path.extension().map(|e| e.to_string_lossy().to_lowercase()).as_deref() {
      Some("abr") => rustcreate_core::import_abr(&bytes, &set)?,
      _ => rustcreate_core::import_brushset(&bytes, &set)?,
    };
    if let Some(dir) = config_file(IMAGES) {
      std::fs::create_dir_all(&dir)?;
      for image in &imported.images {
        std::fs::write(dir.join(image_file(image.grain, image.id)), &image.png)?;
      }
    }
    let first = self.entries.len();
    for (name, brush) in imported.brushes {
      self.push_custom(name, brush);
    }
    Ok(first)
  }

  /// Deletes one of the user's brushes; built-ins stay. Tools that used it go back to defaults.
  pub fn delete(&mut self, index: usize) {
    if self.is_builtin(index) {
      return;
    }
    self.entries.remove(index);
    for (tool, selected) in self.selection.iter_mut().enumerate() {
      if *selected == index {
        *selected = self.entries.iter().position(|e| e.id == DEFAULT_SELECTION[tool]).unwrap_or(0);
      } else if *selected > index {
        *selected -= 1;
      }
    }
  }

  /// The next brush in the same category, wrapping (Shift+B).
  pub fn next_in_category(&self, index: usize) -> usize {
    let category = self.entries[index].category;
    let same: Vec<usize> = (0..self.entries.len()).filter(|&i| self.entries[i].category == category).collect();
    let at = same.iter().position(|&i| i == index).unwrap_or(0);
    same[(at + 1) % same.len()]
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn builtins_are_named_and_in_every_category() {
    let entries = builtins();
    let tokens: std::collections::HashSet<_> = entries.iter().map(|e| e.name.clone()).map(|n| format!("{n:?}")).collect();
    assert_eq!(tokens.len(), entries.len(), "every built-in has its own name token");
    for category in 0..MINE {
      assert!(entries.iter().any(|e| e.category == category), "category {category} has brushes");
    }
  }

  #[test]
  fn edits_custom_brushes_and_selection_survive_a_round_trip() {
    let mut library = Library::from_saved(Saved::default());
    assert_eq!(library.entries[library.selection[0]].id, "ink");
    let ink = library.selection[0];
    library.entries[ink].brush.size = 77.0;
    let copy = library.duplicate(ink, "Mine".into());
    library.selection[2] = copy;

    let reloaded = Library::from_saved(library.to_saved());
    assert_eq!(reloaded.entries[ink].brush.size, 77.0, "edits kept");
    assert_eq!(reloaded.entries[copy].name, Name::Custom("Mine".into()));
    assert_eq!(reloaded.selection[2], copy, "the eraser still uses the copy");
    assert_eq!(library.to_saved().edited.len(), 1, "only changed built-ins are saved");
    assert_eq!(reloaded.original(ink).map(|b| b.size), Some(Brush::ink().size));
  }

  #[test]
  fn deleting_a_custom_brush_fixes_the_selection() {
    let mut library = Library::from_saved(Saved::default());
    let copy = library.duplicate(0, "A".into());
    library.selection = [copy, copy, 0];
    library.delete(copy);
    assert_eq!(library.entries[library.selection[0]].id, "ink");
    assert_eq!(library.entries[library.selection[1]].id, "blender");
    library.delete(0);
    assert_eq!(library.entries[0].id, "graphite", "built-ins cannot be deleted");
  }

  #[test]
  fn shift_b_cycles_inside_the_category() {
    let library = Library::from_saved(Saved::default());
    let ink = library.find("ink").unwrap();
    let mut i = ink;
    for _ in 0..4 {
      i = library.next_in_category(i);
      assert_eq!(library.entries[i].category, 1);
    }
    assert_eq!(i, ink, "four inking brushes, back to the start");
  }
}
