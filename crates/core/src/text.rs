//! Text layers: a text, its font, size, color and place, drawn into the layer's pixels with the
//! system's fonts. The pixels are kept and saved like any layer's; the text makes them editable.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use ab_glyph::{Font, FontArc, PxScale, ScaleFont, point};
use serde::{Deserialize, Serialize};

use crate::{Document, TILE};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Text {
  pub text: String,
  /// A font family installed on the system; another sans serif is used if it is missing.
  pub font: String,
  /// Pixels from one line to the next, roughly the height of a capital plus descenders.
  pub size: f32,
  /// Straight sRGB.
  pub color: [u8; 3],
  /// The top left corner of the first line, in canvas pixels.
  pub position: (f32, f32),
}

fn database() -> &'static fontdb::Database {
  static DB: OnceLock<fontdb::Database> = OnceLock::new();
  DB.get_or_init(|| {
    let mut db = fontdb::Database::new();
    db.load_system_fonts();
    db
  })
}

/// The installed font families, sorted, for a font menu.
pub fn font_families() -> Vec<String> {
  let mut names: Vec<String> = database().faces().filter_map(|face| face.families.first().map(|(name, _)| name.clone())).collect();
  names.sort_by_key(|n| n.to_lowercase());
  names.dedup();
  names
}

/// The regular face of `family` (or of a sans serif, or any font), loaded once.
fn font(family: &str) -> Option<FontArc> {
  static LOADED: OnceLock<Mutex<HashMap<String, Option<FontArc>>>> = OnceLock::new();
  let mut loaded = LOADED.get_or_init(Default::default).lock().ok()?;
  loaded
    .entry(family.to_owned())
    .or_insert_with(|| {
      let db = database();
      let families = [fontdb::Family::Name(family), fontdb::Family::SansSerif];
      let id = db.query(&fontdb::Query { families: &families, ..Default::default() }).or_else(|| db.faces().next().map(|f| f.id))?;
      db.with_face_data(id, |data, index| ab_glyph::FontVec::try_from_vec_and_index(data.to_vec(), index).ok()).flatten().map(FontArc::new)
    })
    .clone()
}

/// Every glyph's coverage, as (canvas x, canvas y, 0..1), in drawing order.
fn coverage(text: &Text, mut each: impl FnMut(i32, i32, f32)) {
  let Some(font) = font(&text.font) else { return };
  let font = font.as_scaled(PxScale::from(text.size.max(1.0)));
  let line_height = font.height() + font.line_gap();
  // shortcut: glyphs placed one after another with kerning, no shaping; scripts that need it
  // (Arabic, Indic) come out unjoined. Use rustybuzz if those are asked for.
  for (row, line) in text.text.lines().enumerate() {
    let mut x = text.position.0;
    let baseline = text.position.1 + font.ascent() + row as f32 * line_height;
    let mut previous = None;
    for c in line.chars() {
      let id = font.glyph_id(c);
      if let Some(previous) = previous {
        x += font.kern(previous, id);
      }
      let glyph = id.with_scale_and_position(font.scale(), point(x, baseline));
      x += font.h_advance(id);
      previous = Some(id);
      if let Some(outlined) = font.outline_glyph(glyph) {
        let min = outlined.px_bounds().min;
        outlined.draw(|gx, gy, cover| each(min.x as i32 + gx as i32, min.y as i32 + gy as i32, cover));
      }
    }
  }
}

/// The area `text` covers (x0, y0, x1, y1), in canvas pixels, for picking a text layer.
pub fn text_bounds(text: &Text) -> Option<(f32, f32, f32, f32)> {
  let font = font(&text.font)?;
  let font = font.as_scaled(PxScale::from(text.size.max(1.0)));
  let lines = text.text.lines().count().max(1) as f32;
  let width = text.text.lines().map(|line| line.chars().map(|c| font.h_advance(font.glyph_id(c))).sum::<f32>()).fold(0.0, f32::max);
  let (x, y) = text.position;
  Some((x, y, x + width.max(text.size * 0.5), y + lines * (font.height() + font.line_gap())))
}

impl Document {
  /// Makes the active layer a plain one: its text stays as pixels, no longer editable. Every
  /// other pixel edit calls this after its checkpoint.
  pub(crate) fn rasterize_text(&mut self) {
    if !self.editing_mask() {
      self.layers[self.active].text = None;
    }
  }

  /// A new text layer above the active one, as one undo step.
  pub fn add_text_layer(&mut self, text: Text) {
    self.add_layer();
    self.set_text(self.active, text);
  }

  /// Redraws layer `index` as `text` and names it after it. Not an undo step by itself:
  /// `checkpoint` once before a round of edits. `false` for a group, a locked layer or an
  /// index out of range.
  pub fn set_text(&mut self, index: usize, text: Text) -> bool {
    let (width, height) = (self.width as i32, self.height as i32);
    let Some(layer) = self.layers.get_mut(index).filter(|l| !l.group && !l.locked) else { return false };
    for key in layer.tile_coords().collect::<Vec<_>>() {
      layer.set_tile(key, None);
    }
    let [r, g, b] = text.color;
    coverage(&text, |x, y, cover| {
      if !(0..width).contains(&x) || !(0..height).contains(&y) {
        return;
      }
      let (x, y) = (x as u32, y as u32);
      let tile = layer.tile_mut(x / TILE, y / TILE);
      let p = (((y % TILE) * TILE + x % TILE) * 4) as usize;
      // Overlapping glyphs add up like paint over paint of the same color.
      let a = tile[p + 3] as f32 / 255.0;
      let a = a + cover.clamp(0.0, 1.0) * (1.0 - a);
      tile[p..p + 4].copy_from_slice(&[
        (r as f32 * a).round() as u8,
        (g as f32 * a).round() as u8,
        (b as f32 * a).round() as u8,
        (a * 255.0).round() as u8,
      ]);
    });
    let first = text.text.lines().find(|l| !l.trim().is_empty()).unwrap_or("Text").trim();
    layer.name = first.chars().take(40).collect();
    layer.text = Some(Box::new(text));
    true
  }
}

impl crate::Layer {
  /// The text a text layer shows.
  pub fn text(&self) -> Option<&Text> {
    self.text.as_deref()
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  fn sample() -> Text {
    Text { text: "Hi\nthere".into(), font: "no such font".into(), size: 24.0, color: [200, 40, 10], position: (10.0, 8.0) }
  }

  #[test]
  fn text_layers_draw_rasterize_and_save() {
    if font_families().is_empty() {
      eprintln!("no system fonts: skipping");
      return;
    }
    let mut doc = Document::new(128, 96);
    doc.add_text_layer(sample());
    let layer = &doc.layers()[1];
    assert_eq!(layer.name, "Hi");
    assert!(layer.text().is_some());
    // Paint where the text is, in its color, and none far from it.
    let tile = layer.tile(0, 0).expect("drawn");
    let alpha = |x: usize, y: usize| tile[(y * TILE as usize + x) * 4 + 3];
    assert!((8..40).flat_map(|y| (10..40).map(move |x| (x, y))).any(|(x, y)| alpha(x, y) == 255));
    assert_eq!(alpha(120, 90), 0);
    let (x0, y0, x1, y1) = text_bounds(layer.text().unwrap()).unwrap();
    assert!(x0 == 10.0 && y0 == 8.0 && x1 > 30.0 && y1 > 50.0);

    // Saved and opened, the text is still editable.
    let path = std::env::temp_dir().join(format!("rustcreate-text-{}.rcreate", std::process::id()));
    doc.save(&path).unwrap();
    let opened = Document::open(&path).unwrap();
    std::fs::remove_file(&path).ok();
    assert_eq!(opened.layers()[1].text(), Some(&sample()));

    // Editing the text redraws; a stroke makes it plain pixels, and undo brings the text back.
    doc.checkpoint();
    assert!(doc.set_text(1, Text { text: "Bye".into(), ..sample() }));
    assert_eq!(doc.layers()[1].name, "Bye");
    let stroke = doc.begin_stroke(crate::Brush::default(), [0, 0, 0], false);
    assert!(stroke.is_some() && doc.layers()[1].text().is_none());
    drop(stroke);
    doc.undo();
    assert_eq!(doc.layers()[1].text().map(|t| t.text.as_str()), Some("Bye"));
  }
}
