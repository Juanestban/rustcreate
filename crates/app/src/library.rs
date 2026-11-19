//! The brush library: built-in brushes in categories (all original, ADR 0006), the user's own
//! brushes, and which brush each tool (paint, smudge, erase) uses. Edits and the user's brushes
//! are kept in `brushes.json` next to the settings.

use std::collections::HashMap;

use std::path::Path;

use rustcreate_core::{Brush, FileError, Grain, Tip};
use serde::{Deserialize, Serialize};

use crate::settings::config_file;

/// Category name tokens, in library order. The last one holds the user's brushes.
pub const CATEGORIES: [&str; 9] = [
  "ui-cat-sketching",
  "ui-cat-inking",
  "ui-cat-painting",
  "ui-cat-airbrushing",
  "ui-cat-textures",
  "ui-cat-calligraphy",
  "ui-cat-abstract",
  "ui-cat-elements",
  "ui-cat-mine",
];
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
  let list: [(&str, usize, Brush); 35] = [
    ("graphite", 0, Brush { tilt_size: 0.7, tilt_flow: 0.5, ..Brush::pencil() }),
    ("soft-graphite", 0, Brush { tilt_size: 1.0, tilt_flow: 0.6, size: 10.0, hardness: 0.3, flow: 0.5, grain_depth: 0.75, ..Brush::pencil() }),
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
        wet_mix: 0.3,
        charge: 0.9,
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
      Brush {
        wet_mix: 0.5,
        charge: 0.8,
        size: 36.0,
        tip: Tip::Bristles,
        follow: true,
        spacing: 0.04,
        flow: 0.8,
        pressure_size: 0.4,
        pressure_flow: 0.6,
        ..flat
      },
    ),
    (
      "gouache",
      2,
      Brush { wet_mix: 0.35, size: 30.0, hardness: 0.75, flow: 0.85, pressure_flow: 0.4, grain: Grain::Canvas, grain_depth: 0.3, ..flat },
    ),
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
    // Erasers like Photoshop's: full flow and no build-up, so one pass clears what it covers
    // (the soft one fades only at its edge). Pressure changes the size, not how much is removed.
    (
      "hard-eraser",
      3,
      Brush { size: 40.0, hardness: 0.9, flow: 1.0, buildup: false, pressure_flow: 0.0, pressure_size: 0.3, spacing: 0.08, ..Brush::airbrush() },
    ),
    (
      "soft-eraser",
      3,
      Brush { size: 90.0, hardness: 0.0, flow: 1.0, buildup: false, pressure_flow: 0.0, pressure_size: 0.3, spacing: 0.08, ..Brush::airbrush() },
    ),
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
      Brush {
        tilt_size: 0.8,
        tilt_flow: 0.4,
        size: 28.0,
        tip: Tip::Chalk,
        follow: true,
        flow: 0.9,
        pressure_size: 0.4,
        grain: Grain::Noise,
        grain_depth: 0.6,
        ..flat
      },
    ),
    (
      "pastel",
      4,
      Brush { size: 45.0, hardness: 0.4, flow: 0.6, pressure_flow: 0.8, grain: Grain::Paper, grain_depth: 0.85, grain_scale: 1.5, ..flat },
    ),
    ("canvas-texture", 4, Brush { size: 150.0, hardness: 0.0, flow: 0.5, grain: Grain::Canvas, grain_depth: 1.0, ..Brush::airbrush() }),
    // Calligraphy: flat nibs at a fixed angle, so the width follows the direction.
    (
      "chisel-nib",
      5,
      Brush { size: 24.0, roundness: 0.18, angle: 45.0, hardness: 0.95, spacing: 0.03, pressure_size: 0.35, streamline: 0.4, ..flat },
    ),
    (
      "script-pen",
      5,
      Brush {
        size: 14.0,
        roundness: 0.35,
        angle: 30.0,
        spacing: 0.03,
        pressure_size: 0.8,
        taper_start: 20.0,
        taper_end: 30.0,
        streamline: 0.5,
        ..base
      },
    ),
    ("monoline", 5, Brush { size: 10.0, pressure_size: 0.0, streamline: 0.7, ..flat }),
    (
      "brush-lettering",
      5,
      Brush {
        size: 30.0,
        roundness: 0.7,
        angle: 30.0,
        spacing: 0.03,
        pressure_size: 1.0,
        taper_start: 40.0,
        taper_end: 60.0,
        streamline: 0.6,
        ..base
      },
    ),
    // Abstract: scattered and ribbon-like marks.
    ("confetti", 6, Brush { size: 18.0, hardness: 1.0, scatter: 2.0, size_jitter: 0.8, spacing: 1.5, pressure_size: 0.0, streamline: 0.0, ..flat }),
    ("ribbon", 6, Brush { size: 50.0, roundness: 0.1, follow: true, angle: 60.0, spacing: 0.02, pressure_size: 0.3, ..flat }),
    ("stipple", 6, Brush { size: 6.0, hardness: 0.9, scatter: 2.0, size_jitter: 0.6, spacing: 0.9, flow: 0.9, pressure_size: 0.5, ..flat }),
    (
      "splatter",
      6,
      Brush { size: 60.0, tip: Tip::Splatter, scatter: 1.0, size_jitter: 0.7, angle_jitter: 1.0, spacing: 0.6, pressure_size: 0.4, ..flat },
    ),
    // Elements: soft, textured masses and streaks.
    (
      "clouds",
      7,
      Brush {
        size: 200.0,
        hardness: 0.0,
        flow: 0.45,
        scatter: 0.5,
        size_jitter: 0.5,
        spacing: 0.2,
        pressure_flow: 0.8,
        grain: Grain::Clouds,
        grain_depth: 0.9,
        grain_scale: 2.0,
        ..Brush::airbrush()
      },
    ),
    (
      "smoke",
      7,
      Brush {
        size: 120.0,
        hardness: 0.0,
        flow: 0.22,
        scatter: 0.8,
        size_jitter: 0.6,
        spacing: 0.15,
        grain: Grain::Clouds,
        grain_depth: 1.0,
        ..Brush::airbrush()
      },
    ),
    (
      "water",
      7,
      Brush {
        size: 60.0,
        wet_mix: 0.6,
        charge: 0.85,
        roundness: 0.5,
        follow: true,
        hardness: 0.4,
        flow: 0.5,
        grain: Grain::Ripples,
        grain_depth: 0.6,
        ..flat
      },
    ),
    (
      "flame",
      7,
      Brush {
        size: 40.0,
        follow: true,
        roundness: 0.4,
        hardness: 0.3,
        flow: 0.7,
        taper_start: 30.0,
        taper_end: 80.0,
        angle_jitter: 0.08,
        size_jitter: 0.4,
        scatter: 0.2,
        spacing: 0.06,
        grain: Grain::Clouds,
        grain_depth: 0.5,
        grain_scale: 0.5,
        ..base
      },
    ),
    (
      "sparks",
      7,
      Brush { size: 9.0, hardness: 1.0, roundness: 0.35, follow: true, scatter: 1.5, size_jitter: 0.7, spacing: 0.7, pressure_size: 0.2, ..flat },
    ),
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
    "hard-eraser" => "ui-brush-hard-eraser",
    "soft-eraser" => "ui-brush-soft-eraser",
    "chalk" => "ui-brush-chalk",
    "charcoal" => "ui-brush-charcoal",
    "pastel" => "ui-brush-pastel",
    "canvas-texture" => "ui-brush-canvas-texture",
    "chisel-nib" => "ui-brush-chisel-nib",
    "script-pen" => "ui-brush-script-pen",
    "monoline" => "ui-brush-monoline",
    "brush-lettering" => "ui-brush-brush-lettering",
    "confetti" => "ui-brush-confetti",
    "ribbon" => "ui-brush-ribbon",
    "stipple" => "ui-brush-stipple",
    "splatter" => "ui-brush-splatter",
    "clouds" => "ui-brush-clouds",
    "smoke" => "ui-brush-smoke",
    "water" => "ui-brush-water",
    "flame" => "ui-brush-flame",
    "sparks" => "ui-brush-sparks",
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
const DEFAULT_SELECTION: [&str; 3] = ["ink", "blender", "hard-eraser"];

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
      // The eraser used to default to the Hard Airbrush, which takes many passes: move those
      // who never changed it to the Hard Eraser.
      let selected = if tool == 2 && saved.selected[tool] == "hard-airbrush" { "" } else { &saved.selected[tool] };
      library.selection[tool] = library.find(selected).or_else(|| library.find(default)).unwrap_or(0);
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

  /// The brushes of one category, named as the user sees them, for exporting a set.
  pub fn category(&self, category: usize, name: impl Fn(&Entry) -> String) -> Vec<(String, Brush)> {
    self.entries.iter().filter(|e| e.category == category).map(|e| (name(e), e.brush)).collect()
  }

  /// Imports a `.rcbrushes`, `.brushset`, `.brush` or `.abr` into My brushes and keeps its images, so the
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
  fn the_default_eraser_clears_in_one_pass() {
    let library = Library::from_saved(Saved::default());
    let eraser = library.entries[library.selection[2]].brush;
    let mut doc = rustcreate_core::Document::new(64, 64);
    doc.layer_mut(0).unwrap().tile_mut(0, 0).fill(255);
    let mut stroke = doc.begin_stroke(eraser, [0, 0, 0], true).unwrap();
    // A light mouse-like pressure: still all gone under the middle of the stroke.
    for x in [4.0, 20.0, 40.0, 60.0] {
      stroke.add(&mut doc, x, 32.0, 0.3);
    }
    stroke.finish(&mut doc);
    let tile = doc.layers()[0].tile(0, 0).unwrap();
    assert!((10..54).all(|x| tile[(32 * rustcreate_core::TILE as usize + x) * 4 + 3] == 0));
  }

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
