//! The paint bucket (flood fill with a color threshold) and the gradient tool. Both paint the
//! active layer as one undo step and refuse locked layers.

use std::collections::HashMap;
use std::sync::Arc;

use crate::tip::unit;
use crate::{Document, TILE, Tile};

/// Reads the colors the bucket compares: the active layer, or every visible layer composited.
struct Reference<'a> {
  doc: &'a Document,
  all_layers: bool,
  composites: HashMap<(u32, u32), Option<Arc<[u8]>>>,
}

impl Reference<'_> {
  fn tile(&mut self, tx: u32, ty: u32) -> Option<Arc<[u8]>> {
    if !self.all_layers {
      return self.doc.layers[self.doc.active].tiles.get(&(tx, ty)).cloned();
    }
    let doc = self.doc;
    self.composites.entry((tx, ty)).or_insert_with(|| doc.composite_tile(tx, ty).map(Arc::from)).clone()
  }
}

/// A bit per canvas pixel.
struct Mask {
  width: u32,
  bits: Vec<u64>,
}

impl Mask {
  fn new(width: u32, height: u32) -> Self {
    Self { width, bits: vec![0; (width as usize * height as usize).div_ceil(64)] }
  }

  fn get(&self, x: u32, y: u32) -> bool {
    let i = y as usize * self.width as usize + x as usize;
    self.bits[i / 64] >> (i % 64) & 1 == 1
  }

  fn set(&mut self, x: u32, y: u32) {
    let i = y as usize * self.width as usize + x as usize;
    self.bits[i / 64] |= 1 << (i % 64);
  }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum GradientKind {
  #[default]
  Linear,
  Radial,
}

/// A gradient being dragged: every update repaints it over the layer as it was before.
pub struct Gradient {
  layer_id: u64,
  before: HashMap<(u32, u32), Tile>,
  kind: GradientKind,
  /// Straight sRGB with alpha, start and end.
  colors: [[u8; 4]; 2],
}

impl Document {
  /// Flood-fills the area around (x, y) whose color is within `threshold` (0..=1 of the channel
  /// range) of the clicked color, with opaque `color`. The filled area grows one pixel under
  /// antialiased edges, painted behind what is there, so line art keeps its edges without a
  /// light halo. `false` if nothing was filled (outside, locked).
  pub fn fill(&mut self, x: u32, y: u32, color: [u8; 3], threshold: f32, all_layers: bool) -> bool {
    if x >= self.width || y >= self.height || self.layers[self.active].locked {
      return false;
    }
    let mask = {
      let mut reference = Reference { doc: self, all_layers, composites: HashMap::new() };
      flood(&mut reference, x, y, threshold)
    };
    let Some((x0, y0, x1, y1)) = mask.1 else { return false };
    let mask = mask.0;
    self.checkpoint();
    let (width, height) = (self.width, self.height);
    let layer = &mut self.layers[self.active];
    // One pixel of margin for the grown edge.
    let (x0, y0, x1, y1) = (x0.saturating_sub(1), y0.saturating_sub(1), (x1 + 1).min(width - 1), (y1 + 1).min(height - 1));
    let inside = |x: u32, y: u32| mask.get(x, y);
    let edge = |x: u32, y: u32| {
      (x > 0 && inside(x - 1, y)) || (x + 1 < width && inside(x + 1, y)) || (y > 0 && inside(x, y - 1)) || (y + 1 < height && inside(x, y + 1))
    };
    for ty in y0 / TILE..=y1 / TILE {
      for tx in x0 / TILE..=x1 / TILE {
        let pixels = layer.tile_mut(tx, ty);
        for py in y0.max(ty * TILE)..=y1.min(ty * TILE + TILE - 1) {
          for px in x0.max(tx * TILE)..=x1.min(tx * TILE + TILE - 1) {
            let p = (((py % TILE) * TILE + px % TILE) * 4) as usize;
            let out = &mut pixels[p..p + 4];
            if inside(px, py) {
              out.copy_from_slice(&[color[0], color[1], color[2], 255]);
            } else if edge(px, py) {
              // Behind: the color shows only where the pixel is not already opaque.
              let behind = 1.0 - out[3] as f32 / 255.0;
              for ch in 0..3 {
                out[ch] = (out[ch] as f32 + color[ch] as f32 * behind).round() as u8;
              }
              out[3] = 255;
            }
          }
        }
      }
    }
    true
  }

  /// Starts a gradient on the active layer, as one undo step. `None` if it is locked. `colors`
  /// are straight sRGB + alpha for the start and the end.
  pub fn begin_gradient(&mut self, kind: GradientKind, colors: [[u8; 4]; 2]) -> Option<Gradient> {
    let layer = &self.layers[self.active];
    if layer.locked {
      return None;
    }
    let (layer_id, before) = (layer.id, layer.tiles.clone());
    self.checkpoint();
    Some(Gradient { layer_id, before, kind, colors })
  }

  /// Fills the whole active layer with `color` (Alt+Backspace), as one undo step.
  pub fn fill_layer(&mut self, color: [u8; 3]) -> bool {
    let rgba = [color[0], color[1], color[2], 255];
    self.begin_gradient(GradientKind::Linear, [rgba, rgba]).map(|mut g| g.update(self, (0.0, 0.0), (1.0, 0.0))).is_some()
  }
}

impl Gradient {
  /// Paints the gradient from `from` to `to` (canvas pixels) over the layer as it was when the
  /// gradient began. Linear: across the line; radial: `from` is the center, `to` the edge.
  pub fn update(&mut self, doc: &mut Document, from: (f32, f32), to: (f32, f32)) {
    // shortcut: repaints the whole layer on every move (~50 ms at 4096² in release); move it to
    // the GPU if dragging feels slow on big canvases.
    let (width, height) = (doc.width, doc.height);
    let (tiles_x, tiles_y) = doc.tiles();
    let Some(layer) = doc.layers.iter_mut().find(|l| l.id == self.layer_id) else { return };
    let (dx, dy) = (to.0 - from.0, to.1 - from.1);
    let length2 = (dx * dx + dy * dy).max(1e-6);
    let [a, b] = self.colors.map(|c| c.map(|v| v as f32));
    for ty in 0..tiles_y {
      for tx in 0..tiles_x {
        let pixels = layer.tile_mut(tx, ty);
        match self.before.get(&(tx, ty)) {
          Some(before) => pixels.copy_from_slice(before),
          None => pixels.fill(0),
        }
        for py in ty * TILE..((ty + 1) * TILE).min(height) {
          for px in tx * TILE..((tx + 1) * TILE).min(width) {
            let (x, y) = (px as f32 + 0.5 - from.0, py as f32 + 0.5 - from.1);
            let t = match self.kind {
              GradientKind::Linear => (x * dx + y * dy) / length2,
              GradientKind::Radial => (x * x + y * y).sqrt() / length2.sqrt(),
            }
            .clamp(0.0, 1.0);
            // Under half a level of noise keeps 8-bit gradients free of bands.
            let dither = unit(px.wrapping_mul(73_856_093) ^ py.wrapping_mul(19_349_663)) - 0.5;
            let alpha = (a[3] + (b[3] - a[3]) * t) / 255.0;
            let p = (((py % TILE) * TILE + px % TILE) * 4) as usize;
            let out = &mut pixels[p..p + 4];
            // Normal blending over the layer, in premultiplied color.
            for ch in 0..3 {
              let src = (a[ch] + (b[ch] - a[ch]) * t) * alpha;
              out[ch] = (src + out[ch] as f32 * (1.0 - alpha) + dither).round().clamp(0.0, 255.0) as u8;
            }
            out[3] = (alpha * 255.0 + out[3] as f32 * (1.0 - alpha) + dither).round().clamp(0.0, 255.0) as u8;
          }
        }
      }
    }
  }
}

/// The pixels connected to (x, y) within `threshold` of its color, and their bounding box.
fn flood(reference: &mut Reference, x: u32, y: u32, threshold: f32) -> (Mask, Option<(u32, u32, u32, u32)>) {
  let (width, height) = (reference.doc.width, reference.doc.height);
  let mut mask = Mask::new(width, height);
  // The last tile read, so neighbouring pixels skip the map lookup.
  let mut cached: ((u32, u32), Option<Arc<[u8]>>) = ((u32::MAX, u32::MAX), None);
  let mut read = |x: u32, y: u32| -> [u8; 4] {
    let key = (x / TILE, y / TILE);
    if cached.0 != key {
      cached = (key, reference.tile(key.0, key.1));
    }
    cached.1.as_ref().map_or([0; 4], |t| {
      let p = (((y % TILE) * TILE + x % TILE) * 4) as usize;
      t[p..p + 4].try_into().expect("4 bytes")
    })
  };
  let seed = read(x, y);
  let tolerance = (threshold.clamp(0.0, 1.0) * 255.0).round() as i32;
  let mut matches = |x: u32, y: u32| read(x, y).iter().zip(seed).all(|(&a, b)| (a as i32 - b as i32).abs() <= tolerance);
  let mut bounds: Option<(u32, u32, u32, u32)> = None;
  let mut stack = vec![(x, y)];
  while let Some((x, y)) = stack.pop() {
    if mask.get(x, y) || !matches(x, y) {
      continue;
    }
    // The whole run on this row, then seeds for the runs above and below it.
    let (mut left, mut right) = (x, x);
    while left > 0 && !mask.get(left - 1, y) && matches(left - 1, y) {
      left -= 1;
    }
    while right + 1 < width && !mask.get(right + 1, y) && matches(right + 1, y) {
      right += 1;
    }
    for i in left..=right {
      mask.set(i, y);
    }
    let b = bounds.get_or_insert((left, y, right, y));
    *b = (b.0.min(left), b.1.min(y), b.2.max(right), b.3.max(y));
    for ny in [y.checked_sub(1), (y + 1 < height).then_some(y + 1)].into_iter().flatten() {
      let mut in_run = false;
      for i in left..=right {
        let open = !mask.get(i, ny) && matches(i, ny);
        if open && !in_run {
          stack.push((i, ny));
        }
        in_run = open;
      }
    }
  }
  (mask, bounds)
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::Brush;

  fn pixel(doc: &Document, layer: usize, x: u32, y: u32) -> [u8; 4] {
    doc.layers()[layer].tile(x / TILE, y / TILE).map_or([0; 4], |t| {
      let p = (((y % TILE) * TILE + x % TILE) * 4) as usize;
      t[p..p + 4].try_into().unwrap()
    })
  }

  /// A closed square outline of hard ink, 100..200, on layer 0.
  fn outlined() -> Document {
    let mut doc = Document::new(300, 300);
    let brush = Brush { size: 4.0, taper_start: 0.0, taper_end: 0.0, streamline: 0.0, ..Brush::ink() };
    let mut stroke = doc.begin_stroke(brush, [0, 0, 0], false).unwrap();
    for (x, y) in [(100.0, 100.0), (200.0, 100.0), (200.0, 200.0), (100.0, 200.0), (100.0, 100.0)] {
      stroke.line(&mut doc, stroke.position().unwrap_or((x, y)), (x, y), 1.0);
    }
    stroke.finish(&mut doc);
    doc
  }

  #[test]
  fn bucket_fills_inside_the_lines_on_another_layer() {
    let mut doc = outlined();
    doc.add_layer();
    assert!(doc.fill(150, 150, [255, 0, 0], 0.1, true));
    assert_eq!(pixel(&doc, 1, 150, 150), [255, 0, 0, 255], "inside is filled");
    assert_eq!(pixel(&doc, 1, 50, 50), [0; 4], "outside the outline is not");
    assert_eq!(pixel(&doc, 1, 103, 150)[3], 255, "the fill reaches under the line's soft edge");
    assert!(doc.undo());
    assert_eq!(pixel(&doc, 1, 150, 150), [0; 4]);

    // Sampling only the empty new layer, the outline is invisible: everything fills.
    assert!(doc.fill(150, 150, [0, 0, 255], 0.1, false));
    assert_eq!(pixel(&doc, 1, 50, 50), [0, 0, 255, 255]);
  }

  #[test]
  fn bucket_threshold_and_refusals() {
    let mut doc = Document::new(64, 64);
    doc.layer_mut(0).unwrap().tile_mut(0, 0)[..4].copy_from_slice(&[20, 20, 20, 20]);
    doc.fill(10, 10, [9, 9, 9], 0.0, false);
    // Threshold 0 stops at the different pixel: it keeps its color, with the fill only behind it.
    assert_eq!(pixel(&doc, 0, 0, 0), [28, 28, 28, 255]);
    assert_eq!(pixel(&doc, 0, 63, 63), [9, 9, 9, 255]);
    assert!(!doc.fill(64, 0, [0; 3], 0.5, false), "outside the canvas");
    doc.layer_mut(0).unwrap().locked = true;
    assert!(!doc.fill(5, 5, [0; 3], 0.5, false), "locked layer");
  }

  #[test]
  fn gradients_blend_and_redraw_while_dragged() {
    let mut doc = Document::new(256, 16);
    let mut gradient = doc.begin_gradient(GradientKind::Linear, [[0, 0, 0, 255], [255, 255, 255, 255]]).unwrap();
    gradient.update(&mut doc, (0.0, 0.0), (64.0, 0.0));
    gradient.update(&mut doc, (0.0, 0.0), (256.0, 0.0));
    let (start, middle, end) = (pixel(&doc, 0, 0, 8)[0], pixel(&doc, 0, 128, 8)[0], pixel(&doc, 0, 255, 8)[0]);
    assert!(start <= 1 && (126..=130).contains(&middle) && end >= 254, "{start} {middle} {end}: redrawn, not stacked");
    assert!(doc.undo());
    assert_eq!(pixel(&doc, 0, 128, 8), [0; 4], "one undo step");

    let mut radial = doc.begin_gradient(GradientKind::Radial, [[255, 0, 0, 255], [255, 0, 0, 0]]).unwrap();
    radial.update(&mut doc, (128.0, 8.0), (228.0, 8.0));
    assert!(pixel(&doc, 0, 128, 8)[3] >= 250 && pixel(&doc, 0, 0, 8)[3] <= 1, "red center fading to clear");

    assert!(doc.fill_layer([1, 2, 3]));
    assert_eq!(pixel(&doc, 0, 200, 3)[..3], [1, 2, 3]);
  }
}
