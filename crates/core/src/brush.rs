//! The brush engine (ADR 0001: dabs stamped on the CPU). A stroke stamps round dabs along the
//! pointer path at a fixed spacing. Dabs accumulate into a per-stroke coverage mask, and every
//! touched pixel is recomposited from its pre-stroke value: overlapping dabs never exceed the
//! stroke's opacity (like Procreate and Photoshop), unless the brush builds up (airbrush).

use std::collections::HashMap;

use crate::{Document, TILE};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Brush {
  /// Diameter in canvas pixels at full pressure.
  pub size: f32,
  /// 0..=1, the most a stroke can cover.
  pub opacity: f32,
  /// 0 = soft falloff from the center, 1 = hard edge.
  pub hardness: f32,
  /// Distance between dabs as a fraction of the current diameter.
  pub spacing: f32,
  /// 0..=1, how much one dab adds.
  pub flow: f32,
  /// Coverage keeps adding up while painting over the same place (airbrush) instead of capping
  /// at the strongest dab.
  pub buildup: bool,
  /// 0..=1, how much pen pressure scales the size (0 = always full size).
  pub pressure_size: f32,
  /// 0..=1, how much pen pressure scales the flow.
  pub pressure_flow: f32,
  /// Stroke length in pixels over which the start grows from a point to full size.
  pub taper: f32,
  /// 0..=1, how much the path is smoothed (StreamLine). Higher lags more.
  pub streamline: f32,
}

impl Brush {
  pub const fn pencil() -> Self {
    Self {
      size: 4.0,
      opacity: 1.0,
      hardness: 0.8,
      spacing: 0.15,
      flow: 0.7,
      buildup: false,
      pressure_size: 0.6,
      pressure_flow: 0.8,
      taper: 0.0,
      streamline: 0.1,
    }
  }

  pub const fn ink() -> Self {
    Self {
      size: 12.0,
      opacity: 1.0,
      hardness: 1.0,
      spacing: 0.08,
      flow: 1.0,
      buildup: false,
      pressure_size: 0.9,
      pressure_flow: 0.0,
      taper: 40.0,
      streamline: 0.4,
    }
  }

  pub const fn airbrush() -> Self {
    Self {
      size: 120.0,
      opacity: 1.0,
      hardness: 0.0,
      spacing: 0.1,
      flow: 0.08,
      buildup: true,
      pressure_size: 0.0,
      pressure_flow: 1.0,
      taper: 0.0,
      streamline: 0.0,
    }
  }
}

/// A tile touched by the stroke: its pixels before the stroke and the stroke's coverage.
struct StrokeTile {
  before: Vec<u8>,
  coverage: Vec<f32>,
}

pub struct Stroke {
  brush: Brush,
  /// Straight (not premultiplied) sRGB.
  color: [u8; 3],
  erase: bool,
  layer_id: u64,
  tiles: HashMap<(u32, u32), StrokeTile>,
  /// Last smoothed point: x, y, pressure.
  last: Option<(f32, f32, f32)>,
  /// Path length stamped so far, for the taper.
  travelled: f32,
  /// Distance left over from the previous segment until the next dab.
  carry: f32,
}

impl Document {
  /// Starts a stroke on the active layer, as one undo step. `None` if the layer is locked.
  /// `erase` removes paint with the brush's shape instead of adding `color`.
  pub fn begin_stroke(&mut self, brush: Brush, color: [u8; 3], erase: bool) -> Option<Stroke> {
    let layer = &self.layers[self.active];
    if layer.locked {
      return None;
    }
    let layer_id = layer.id;
    self.checkpoint();
    Some(Stroke { brush, color, erase, layer_id, tiles: HashMap::new(), last: None, travelled: 0.0, carry: 0.0 })
  }
}

impl Stroke {
  /// Adds a pointer sample in canvas pixels, pressure 0..=1 (1 for a mouse).
  pub fn add(&mut self, doc: &mut Document, x: f32, y: f32, pressure: f32) {
    let pressure = pressure.clamp(0.0, 1.0);
    let Some((lx, ly, lp)) = self.last else {
      self.last = Some((x, y, pressure));
      self.dab(doc, x, y, pressure);
      return;
    };
    // StreamLine: follow the pointer only part of the way.
    let follow = 1.0 - self.brush.streamline.clamp(0.0, 0.95);
    let (x, y, pressure) = (lx + (x - lx) * follow, ly + (y - ly) * follow, lp + (pressure - lp) * follow);
    self.segment(doc, (lx, ly, lp), (x, y, pressure));
    self.last = Some((x, y, pressure));
  }

  /// A straight line from `from` to `to`, unsmoothed (Shift+click from the last point).
  pub fn line(&mut self, doc: &mut Document, from: (f32, f32), to: (f32, f32), pressure: f32) {
    let pressure = pressure.clamp(0.0, 1.0);
    if self.last.is_none() {
      self.dab(doc, from.0, from.1, pressure);
    }
    self.segment(doc, (from.0, from.1, pressure), (to.0, to.1, pressure));
    self.last = Some((to.0, to.1, pressure));
  }

  /// Where the stroke is now, in canvas pixels.
  pub fn position(&self) -> Option<(f32, f32)> {
    self.last.map(|(x, y, _)| (x, y))
  }

  /// Dabs from `a` to `b` every `spacing × diameter`, interpolating the pressure.
  fn segment(&mut self, doc: &mut Document, a: (f32, f32, f32), b: (f32, f32, f32)) {
    let length = (b.0 - a.0).hypot(b.1 - a.1);
    let mut at = self.carry;
    while at <= length {
      let t = if length > 0.0 { at / length } else { 1.0 };
      let pressure = a.2 + (b.2 - a.2) * t;
      self.dab(doc, a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t, pressure);
      let step = (self.diameter(pressure) * self.brush.spacing).max(0.5);
      at += step;
      self.travelled += step;
    }
    self.carry = at - length;
  }

  fn diameter(&self, pressure: f32) -> f32 {
    let taper = if self.brush.taper > 0.0 { (self.travelled / self.brush.taper).min(1.0) } else { 1.0 };
    let pressure = 1.0 - self.brush.pressure_size * (1.0 - pressure);
    // shortcut: tapers only the start; the end needs the stroke's length, known only when it ends.
    (self.brush.size * pressure * (0.15 + 0.85 * taper)).max(1.0)
  }

  /// Stamps one dab centered at (x, y).
  fn dab(&mut self, doc: &mut Document, x: f32, y: f32, pressure: f32) {
    let Some(layer) = doc.layers.iter_mut().find(|l| l.id == self.layer_id) else { return };
    let radius = self.diameter(pressure) / 2.0;
    let flow = self.brush.flow * (1.0 - self.brush.pressure_flow * (1.0 - pressure));
    let opacity = self.brush.opacity.clamp(0.0, 1.0);
    // Soft part of the edge; at least one pixel so hard edges are antialiased.
    let inner = (radius * self.brush.hardness.clamp(0.0, 1.0)).min(radius - 1.0).max(0.0);
    let (x0, y0) = ((x - radius).floor().max(0.0) as u32, (y - radius).floor().max(0.0) as u32);
    let (x1, y1) = ((x + radius).ceil().min(doc.width as f32 - 1.0), (y + radius).ceil().min(doc.height as f32 - 1.0));
    if x1 < x0 as f32 || y1 < y0 as f32 || x1 < 0.0 || y1 < 0.0 {
      return;
    }
    let (x1, y1) = (x1 as u32, y1 as u32);
    for ty in y0 / TILE..=y1 / TILE {
      for tx in x0 / TILE..=x1 / TILE {
        let pixels = layer.tile_mut(tx, ty);
        let tile = self.tiles.entry((tx, ty)).or_insert_with(|| StrokeTile { before: pixels.to_vec(), coverage: vec![0.0; (TILE * TILE) as usize] });
        for py in y0.max(ty * TILE)..=y1.min(ty * TILE + TILE - 1) {
          for px in x0.max(tx * TILE)..=x1.min(tx * TILE + TILE - 1) {
            let distance = (px as f32 + 0.5 - x).hypot(py as f32 + 0.5 - y);
            let falloff = if distance <= inner {
              1.0
            } else {
              let t = ((distance - inner) / (radius - inner).max(1e-3)).clamp(0.0, 1.0);
              1.0 - t * t * (3.0 - 2.0 * t)
            };
            if falloff <= 0.0 {
              continue;
            }
            let i = ((py - ty * TILE) * TILE + (px - tx * TILE)) as usize;
            let dab = falloff * flow;
            let c = &mut tile.coverage[i];
            *c = if self.brush.buildup { *c + (1.0 - *c) * dab } else { c.max(dab) };
            let alpha = *c * opacity;
            let p = i * 4;
            let before = &tile.before[p..p + 4];
            let out = &mut pixels[p..p + 4];
            if self.erase {
              for ch in 0..4 {
                out[ch] = (before[ch] as f32 * (1.0 - alpha)).round() as u8;
              }
            } else {
              for ch in 0..3 {
                out[ch] = (self.color[ch] as f32 * alpha + before[ch] as f32 * (1.0 - alpha)).round() as u8;
              }
              out[3] = (255.0 * alpha + before[3] as f32 * (1.0 - alpha)).round() as u8;
            }
          }
        }
      }
    }
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  fn alpha(doc: &Document, layer: usize, x: u32, y: u32) -> u8 {
    doc.layers()[layer].tile(x / TILE, y / TILE).map_or(0, |t| t[(((y % TILE) * TILE + x % TILE) * 4 + 3) as usize])
  }

  fn line(doc: &mut Document, brush: Brush, erase: bool, from: (f32, f32), to: (f32, f32)) {
    let mut stroke = doc.begin_stroke(brush, [0, 0, 0], erase).unwrap();
    for i in 0..=20 {
      let t = i as f32 / 20.0;
      stroke.add(doc, from.0 + (to.0 - from.0) * t, from.1 + (to.1 - from.1) * t, 1.0);
    }
  }

  fn flat(size: f32, opacity: f32) -> Brush {
    Brush {
      size,
      opacity,
      hardness: 1.0,
      spacing: 0.1,
      flow: 1.0,
      buildup: false,
      pressure_size: 0.0,
      pressure_flow: 0.0,
      taper: 0.0,
      streamline: 0.0,
    }
  }

  #[test]
  fn a_line_is_continuous_and_crosses_tiles() {
    let mut doc = Document::new(600, 100);
    line(&mut doc, flat(10.0, 1.0), false, (10.0, 50.0), (590.0, 50.0));
    for x in (10..590).step_by(7) {
      assert_eq!(alpha(&doc, 0, x, 50), 255, "gap at x = {x}");
    }
    assert_eq!(alpha(&doc, 0, 300, 70), 0, "nothing outside the brush");
    assert_eq!(doc.layers()[0].painted_tiles(), 3);
  }

  #[test]
  fn overlapping_dabs_respect_the_stroke_opacity() {
    let mut doc = Document::new(100, 100);
    line(&mut doc, flat(20.0, 0.5), false, (20.0, 50.0), (80.0, 50.0));
    assert_eq!(alpha(&doc, 0, 50, 50), 128, "capped at 50 % despite many dabs");
    // A second stroke goes on top of the first.
    line(&mut doc, flat(20.0, 0.5), false, (20.0, 50.0), (80.0, 50.0));
    assert_eq!(alpha(&doc, 0, 50, 50), 192);
  }

  #[test]
  fn airbrush_builds_up_softly() {
    let mut doc = Document::new(200, 200);
    let mut stroke = doc.begin_stroke(Brush::airbrush(), [255, 0, 0], false).unwrap();
    stroke.add(&mut doc, 100.0, 100.0, 1.0);
    let once = alpha(&doc, 0, 100, 100);
    // Scrub back and forth over the center.
    for i in 0..30 {
      stroke.add(&mut doc, if i % 2 == 0 { 80.0 } else { 120.0 }, 100.0, 1.0);
    }
    let center = alpha(&doc, 0, 100, 100);
    assert!(center > once, "{center} > {once}");
    assert!(alpha(&doc, 0, 140, 100) < center, "softer away from the center");
  }

  #[test]
  fn eraser_removes_paint_and_strokes_undo() {
    let mut doc = Document::new(100, 100);
    line(&mut doc, flat(30.0, 1.0), false, (10.0, 50.0), (90.0, 50.0));
    line(&mut doc, flat(10.0, 1.0), true, (50.0, 0.0), (50.0, 99.0));
    assert_eq!(alpha(&doc, 0, 50, 50), 0);
    assert_eq!(alpha(&doc, 0, 20, 50), 255);
    assert!(doc.undo());
    assert_eq!(alpha(&doc, 0, 50, 50), 255, "the eraser stroke is one undo step");
  }

  #[test]
  fn pressure_and_taper_shape_the_stroke() {
    let mut doc = Document::new(200, 200);
    let brush = Brush { pressure_size: 1.0, ..flat(40.0, 1.0) };
    let mut stroke = doc.begin_stroke(brush, [0, 0, 0], false).unwrap();
    stroke.add(&mut doc, 50.0, 50.0, 0.25);
    assert_eq!(alpha(&doc, 0, 50, 62), 0, "light pressure: about 10 px wide");
    stroke.add(&mut doc, 150.0, 150.0, 1.0);
    assert_eq!(alpha(&doc, 0, 150, 165), 255, "full pressure: 40 px wide");

    let mut doc = Document::new(400, 100);
    line(&mut doc, Brush { taper: 200.0, ..flat(40.0, 1.0) }, false, (10.0, 50.0), (390.0, 50.0));
    assert_eq!(alpha(&doc, 0, 15, 62), 0, "thin at the start");
    assert_eq!(alpha(&doc, 0, 350, 62), 255, "full size after the taper");
  }

  #[test]
  fn shift_click_draws_a_straight_line() {
    let mut doc = Document::new(200, 50);
    let mut stroke = doc.begin_stroke(Brush { streamline: 0.9, ..flat(6.0, 1.0) }, [0, 0, 0], false).unwrap();
    stroke.line(&mut doc, (10.0, 25.0), (190.0, 25.0), 1.0);
    assert_eq!(alpha(&doc, 0, 189, 25), 255, "reaches the end despite StreamLine");
    assert_eq!(stroke.position(), Some((190.0, 25.0)));
  }

  #[test]
  fn locked_layers_refuse_strokes() {
    let mut doc = Document::new(10, 10);
    doc.layer_mut(0).unwrap().locked = true;
    assert!(doc.begin_stroke(Brush::ink(), [0, 0, 0], false).is_none());
    assert!(!doc.history().can_undo());
  }

  #[test]
  fn dabs_off_the_canvas_are_ignored() {
    let mut doc = Document::new(50, 50);
    let mut stroke = doc.begin_stroke(flat(10.0, 1.0), [0, 0, 0], false).unwrap();
    stroke.add(&mut doc, -100.0, -100.0, 1.0);
    stroke.add(&mut doc, 1000.0, -100.0, 1.0);
    assert_eq!(doc.layers()[0].painted_tiles(), 0);
    stroke.add(&mut doc, 49.0, 49.0, 1.0);
    assert!(alpha(&doc, 0, 49, 49) > 0);
  }
  /// Rough speed check: `cargo test -p rustcreate-core --release -- --ignored --nocapture`.
  #[test]
  #[ignore = "timing, run in release"]
  fn bench_large_brush_stroke() {
    let mut doc = Document::new(4096, 4096);
    let brush = Brush { size: 300.0, ..Brush::airbrush() };
    let start = std::time::Instant::now();
    let mut stroke = doc.begin_stroke(brush, [0, 0, 0], false).unwrap();
    let samples = 400;
    for i in 0..=samples {
      let t = i as f32 / samples as f32;
      stroke.add(&mut doc, 200.0 + 3600.0 * t, 2048.0 + 800.0 * (t * 12.0).sin(), 1.0);
    }
    let elapsed = start.elapsed();
    eprintln!("{samples} samples of a 300 px airbrush over 3600 px: {elapsed:?} ({:?} per sample)", elapsed / samples);
  }
}
