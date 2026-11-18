//! The brush engine (ADR 0001: dabs stamped on the CPU). A stroke stamps round dabs along the
//! pointer path at a fixed spacing. Dabs accumulate into a per-stroke coverage mask, and every
//! touched pixel is recomposited from its pre-stroke value: overlapping dabs never exceed the
//! stroke's opacity (like Procreate and Photoshop), unless the brush builds up (airbrush).
//!
//! Smudge strokes work differently: each dab drags the paint that was under the previous dab
//! into its own place (patch smudging, as in Krita), with the brush's shape and opacity as
//! strength. They never add color, so smudging an empty layer leaves it empty.

use std::collections::{HashMap, HashSet};

use crate::guide::Mirror;
use crate::tip::{self, Grain, Tip};
use crate::{Document, Selection, Symmetry, TILE};

/// Serializable for saved brush libraries; fields missing from older files take the defaults.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
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
  pub taper_start: f32,
  /// Stroke length in pixels over which the end thins to a point (applied when it ends).
  pub taper_end: f32,
  /// 0..=1, how much the path is smoothed (StreamLine). Higher lags more.
  pub streamline: f32,
  /// Pulled-string stabilizer (Photoshop): the brush only moves once the pointer is this many
  /// pixels away, so small wobbles never reach the canvas. 0 = off.
  pub string: f32,
  /// Motion filtering 0..=1: removes jitter from slow, shaky movement without slowing fast
  /// strokes down (adaptive smoothing, after the 1€ filter).
  pub motion_filter: f32,
  /// Wet mixing 0..=1: how much of the color already on the layer the brush picks up and
  /// blends into its own as it moves. 0 = paints only its color.
  pub wet_mix: f32,
  /// Paint load 0..=1: how long the brush lasts before it runs dry (thinner, then more of the
  /// picked-up color). 1 = never runs out.
  pub charge: f32,
  /// Pen tilt: how much leaning the pen over widens the dab (up to 3×), and how much it thins
  /// the paint, for shading with the side of a pencil. 0..=1 each.
  pub tilt_size: f32,
  pub tilt_flow: f32,
  /// Tip roundness 0.05..=1: below 1 the tip is squashed across its angle, a flat nib for
  /// calligraphy or a ribbon.
  pub roundness: f32,
  /// The dab's shape.
  pub tip: Tip,
  /// Tip rotation in degrees.
  pub angle: f32,
  /// Turn the tip with the stroke's direction (added to `angle`).
  pub follow: bool,
  /// 0..=1, random rotation per dab (1 = up to ±180°).
  pub angle_jitter: f32,
  /// Random offset per dab, as a fraction of the diameter.
  pub scatter: f32,
  /// 0..=1, random shrink per dab.
  pub size_jitter: f32,
  /// Texture the paint catches on, fixed to the canvas.
  pub grain: Grain,
  /// Grain zoom: 1 = one texture pixel per canvas pixel.
  pub grain_scale: f32,
  /// 0..=1, how much the grain holds paint back.
  pub grain_depth: f32,
}

impl Brush {
  /// Every setting brought into the range Brush Studio offers, and a non-number back to the
  /// default: brushes from a file must not paint gigantic or endless dabs.
  pub fn sanitized(self) -> Self {
    let d = Brush::ink();
    let fit = |v: f32, lo: f32, hi: f32, default: f32| if v.is_finite() { v.clamp(lo, hi) } else { default };
    Brush {
      size: fit(self.size, 1.0, 1000.0, d.size),
      opacity: fit(self.opacity, 0.0, 1.0, d.opacity),
      hardness: fit(self.hardness, 0.0, 1.0, d.hardness),
      spacing: fit(self.spacing, 0.01, 2.0, d.spacing),
      flow: fit(self.flow, 0.0, 1.0, d.flow),
      pressure_size: fit(self.pressure_size, 0.0, 1.0, 0.0),
      pressure_flow: fit(self.pressure_flow, 0.0, 1.0, 0.0),
      taper_start: fit(self.taper_start, 0.0, 1000.0, 0.0),
      taper_end: fit(self.taper_end, 0.0, 1000.0, 0.0),
      streamline: fit(self.streamline, 0.0, 1.0, 0.0),
      string: fit(self.string, 0.0, 200.0, 0.0),
      motion_filter: fit(self.motion_filter, 0.0, 1.0, 0.0),
      wet_mix: fit(self.wet_mix, 0.0, 1.0, 0.0),
      charge: fit(self.charge, 0.0, 1.0, 1.0),
      tilt_size: fit(self.tilt_size, 0.0, 1.0, 0.0),
      tilt_flow: fit(self.tilt_flow, 0.0, 1.0, 0.0),
      roundness: fit(self.roundness, 0.05, 1.0, 1.0),
      angle: fit(self.angle, -180.0, 180.0, 0.0),
      angle_jitter: fit(self.angle_jitter, 0.0, 1.0, 0.0),
      scatter: fit(self.scatter, 0.0, 2.0, 0.0),
      size_jitter: fit(self.size_jitter, 0.0, 1.0, 0.0),
      grain_scale: fit(self.grain_scale, 0.25, 8.0, 1.0),
      grain_depth: fit(self.grain_depth, 0.0, 1.0, 0.0),
      ..self
    }
  }
}

impl Default for Brush {
  fn default() -> Self {
    Self::ink()
  }
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
      taper_start: 0.0,
      taper_end: 0.0,
      streamline: 0.1,
      motion_filter: 0.0,
      wet_mix: 0.0,
      charge: 1.0,
      tilt_size: 0.0,
      tilt_flow: 0.0,
      roundness: 1.0,
      string: 0.0,
      tip: Tip::Round,
      angle: 0.0,
      follow: false,
      angle_jitter: 0.0,
      scatter: 0.0,
      size_jitter: 0.0,
      grain: Grain::Paper,
      grain_scale: 1.0,
      grain_depth: 0.55,
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
      taper_start: 40.0,
      taper_end: 40.0,
      streamline: 0.4,
      motion_filter: 0.0,
      wet_mix: 0.0,
      charge: 1.0,
      tilt_size: 0.0,
      tilt_flow: 0.0,
      roundness: 1.0,
      string: 0.0,
      tip: Tip::Round,
      angle: 0.0,
      follow: false,
      angle_jitter: 0.0,
      scatter: 0.0,
      size_jitter: 0.0,
      grain: Grain::None,
      grain_scale: 1.0,
      grain_depth: 0.0,
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
      taper_start: 0.0,
      taper_end: 0.0,
      streamline: 0.0,
      motion_filter: 0.0,
      wet_mix: 0.0,
      charge: 1.0,
      tilt_size: 0.0,
      tilt_flow: 0.0,
      roundness: 1.0,
      string: 0.0,
      tip: Tip::Round,
      angle: 0.0,
      follow: false,
      angle_jitter: 0.0,
      scatter: 0.0,
      size_jitter: 0.0,
      grain: Grain::None,
      grain_scale: 1.0,
      grain_depth: 0.0,
    }
  }
}

/// A stamped dab, kept so the end taper can re-stamp the tail when the stroke ends.
#[derive(Debug, Clone, Copy)]
struct Dab {
  x: f32,
  y: f32,
  pressure: f32,
  /// Path length at this dab, for the tapers.
  at: f32,
  /// Tip rotation in radians, after direction and jitter.
  angle: f32,
  /// Size multiplier from the size jitter.
  size: f32,
  /// Straight sRGB this dab paints with: the brush color, or a wet mix with the canvas.
  color: [f32; 3],
  /// Paint left in the brush, 0..=1: thins the dab as it runs dry.
  load: f32,
  /// Pen tilt when the dab was placed, 0 upright ..= 1 flat.
  tilt: f32,
}

type Textures = (Option<&'static [f32]>, Option<&'static [f32]>);

/// Adaptive smoothing (the 1€ filter, per sample instead of per second): the cutoff rises with
/// speed, so slow jitter is smoothed hard and fast movement passes almost untouched.
#[derive(Debug, Default, Clone, Copy)]
struct MotionFilter {
  /// Filtered position and filtered velocity (pixels per sample).
  state: Option<((f32, f32), (f32, f32))>,
}

impl MotionFilter {
  fn apply(&mut self, p: (f32, f32), amount: f32) -> (f32, f32) {
    let amount = amount.clamp(0.0, 1.0);
    let Some((last, velocity)) = self.state.filter(|_| amount > 0.0) else {
      self.state = Some((p, (0.0, 0.0)));
      return p;
    };
    // Smoothing factor for a cutoff in cycles per sample.
    let alpha = |cutoff: f32| 1.0 / (1.0 + 1.0 / (std::f32::consts::TAU * cutoff));
    let a = alpha(1.0);
    let velocity = (velocity.0 + a * (p.0 - last.0 - velocity.0), velocity.1 + a * (p.1 - last.1 - velocity.1));
    let cutoff = 1.0 + (0.03 - 1.0) * amount + 0.06 * velocity.0.hypot(velocity.1);
    let a = alpha(cutoff);
    let out = (last.0 + a * (p.0 - last.0), last.1 + a * (p.1 - last.1));
    self.state = Some((out, velocity));
    out
  }
}

/// A tile touched by the stroke: its pixels before the stroke and the stroke's coverage.
struct StrokeTile {
  before: Vec<u8>,
  coverage: Vec<f32>,
  /// Wet mixing: the stroke's own color per pixel, since each dab may carry a different one.
  paint: Vec<[f32; 3]>,
}

pub struct Stroke {
  brush: Brush,
  /// Straight (not premultiplied) sRGB.
  color: [u8; 3],
  erase: bool,
  smudge: bool,
  layer_id: u64,
  tiles: HashMap<(u32, u32), StrokeTile>,
  /// Last smoothed point: x, y, pressure.
  last: Option<(f32, f32, f32)>,
  /// Path length stamped so far, for the taper.
  travelled: f32,
  /// Distance left over from the previous segment until the next dab.
  carry: f32,
  /// Smudge: where the previous dab was, the paint is picked up from.
  smudged: Option<(f32, f32)>,
  dabs: Vec<Dab>,
  /// The latest pointer sample before any stabilizing, which the stroke reaches when it ends.
  pointer: Option<(f32, f32, f32)>,
  /// Pulled string: the end of the string the pointer drags.
  pulled: Option<(f32, f32)>,
  motion: MotionFilter,
  /// The stroke's direction in radians, for tips that follow it.
  direction: f32,
  /// Dabs placed so far, the seed of each dab's jitter.
  placed: u32,
  /// Wet mixing: the color the brush carries, and the paint left in it.
  carried: Option<[f32; 3]>,
  load: f32,
  /// The pen's tilt now, 0 upright ..= 1 flat (0 for a mouse).
  tilt: f32,
  /// Raw pointer samples, for QuickShape.
  points: Vec<(f32, f32)>,
  /// Symmetry: every dab is also stamped through each of these.
  mirrors: Vec<Mirror>,
  /// The selection when the stroke began: paint only lands inside it.
  selection: Option<Selection>,
  /// The layer's Alpha Lock when the stroke began.
  alpha_lock: bool,
}

impl Document {
  /// Starts a stroke on the active layer, as one undo step. `None` if the layer is locked.
  /// `erase` removes paint with the brush's shape instead of adding `color`.
  pub fn begin_stroke(&mut self, brush: Brush, color: [u8; 3], erase: bool) -> Option<Stroke> {
    if self.layers[self.active].locked {
      return None;
    }
    let color = self.paint_color(color);
    let layer = self.target();
    let (layer_id, alpha_lock) = (layer.id, layer.alpha_lock);
    self.checkpoint();
    Some(Stroke {
      brush,
      color,
      erase,
      smudge: false,
      layer_id,
      tiles: HashMap::new(),
      last: None,
      travelled: 0.0,
      carry: 0.0,
      smudged: None,
      dabs: Vec::new(),
      pointer: None,
      pulled: None,
      direction: 0.0,
      placed: 0,
      carried: None,
      load: 1.0,
      tilt: 0.0,
      points: Vec::new(),
      mirrors: Vec::new(),
      motion: MotionFilter::default(),
      selection: self.selection.clone(),
      alpha_lock,
    })
  }

  /// Starts a smudge stroke on the active layer, as one undo step. `None` if it is locked.
  pub fn begin_smudge(&mut self, brush: Brush) -> Option<Stroke> {
    let mut stroke = self.begin_stroke(brush, [0; 3], false)?;
    stroke.smudge = true;
    Some(stroke)
  }
}

impl Stroke {
  /// Adds a pointer sample in canvas pixels, pressure 0..=1 (1 for a mouse).
  pub fn add(&mut self, doc: &mut Document, x: f32, y: f32, pressure: f32) {
    let pressure = pressure.clamp(0.0, 1.0);
    self.pointer = Some((x, y, pressure));
    self.points.push((x, y));
    let (x, y) = self.motion.apply((x, y), self.brush.motion_filter);
    // Pulled string: drag the string's end only as far as the pointer pulls it.
    let (x, y) = match self.pulled {
      Some((sx, sy)) if self.brush.string > 0.0 => {
        let distance = (x - sx).hypot(y - sy);
        if distance <= self.brush.string {
          return;
        }
        let t = (distance - self.brush.string) / distance;
        (sx + (x - sx) * t, sy + (y - sy) * t)
      }
      _ => (x, y),
    };
    self.pulled = Some((x, y));
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
    self.pointer = self.last;
    self.pulled = Some(to);
  }

  /// Ends the stroke: catches up with the pointer (StreamLine and the string lag behind it), then
  /// thins the end. The taper re-stamps the tail's tiles from their pre-stroke pixels. Returns
  /// where the stroke ended, for the next Shift+click line.
  pub fn finish(mut self, doc: &mut Document) -> Option<(f32, f32)> {
    if let (Some(last), Some(pointer)) = (self.last, self.pointer) {
      self.segment(doc, last, pointer);
      self.last = Some(pointer);
    }
    let taper = self.brush.taper_end;
    let end = self.position();
    let Some(total) = self.dabs.last().map(|d| d.at) else { return end };
    if self.smudge || taper <= 0.0 {
      return end;
    }
    let tail = self.dabs.iter().position(|d| total - d.at < taper).unwrap_or(self.dabs.len());
    let mut affected = HashSet::new();
    for d in self.dabs[tail..].iter().flat_map(|&d| self.copies(d)) {
      let radius = self.diameter(d.pressure, d.at) * d.size / 2.0;
      for ty in ((d.y - radius).max(0.0) as u32 / TILE)..=((d.y + radius).max(0.0) as u32 / TILE) {
        for tx in ((d.x - radius).max(0.0) as u32 / TILE)..=((d.x + radius).max(0.0) as u32 / TILE) {
          if self.tiles.contains_key(&(tx, ty)) {
            affected.insert((tx, ty));
          }
        }
      }
    }
    let Some(layer) = doc.find_mut(self.layer_id) else { return end };
    for key in &affected {
      let tile = self.tiles.get_mut(key).expect("affected tiles were stamped");
      tile.coverage.fill(0.0);
      layer.tile_mut(key.0, key.1).copy_from_slice(&tile.before);
    }
    for (i, d) in std::mem::take(&mut self.dabs).into_iter().enumerate() {
      let scale = if i >= tail { 0.15 + 0.85 * ((total - d.at) / taper).clamp(0.0, 1.0) } else { 1.0 };
      for copy in self.copies(d) {
        self.stamp(doc, copy, scale, Some(&affected));
      }
    }
    end
  }

  /// Paints mirrored or rotated copies around `center` (canvas pixels). Call before painting;
  /// smudge strokes stay single.
  pub fn with_symmetry(mut self, symmetry: Symmetry, center: (f32, f32)) -> Self {
    if !self.smudge {
      self.mirrors = symmetry.mirrors(center);
    }
    self
  }

  /// The pen's tilt for the next samples: 0 upright ..= 1 lying flat.
  pub fn set_tilt(&mut self, tilt: f32) {
    self.tilt = tilt.clamp(0.0, 1.0);
  }

  /// How much the current tilt widens the dabs.
  fn tilt_scale(&self) -> f32 {
    1.0 + 2.0 * self.brush.tilt_size.clamp(0.0, 1.0) * self.tilt
  }

  /// The pointer samples so far, unsmoothed.
  pub fn points(&self) -> &[(f32, f32)] {
    &self.points
  }

  /// Wipes what the stroke painted and paints `path` instead, unsmoothed, at `pressure`
  /// (QuickShape). The stroke stays one undo step and can still be finished.
  pub fn redraw(&mut self, doc: &mut Document, path: &[(f32, f32)], pressure: f32) {
    if let Some(layer) = doc.find_mut(self.layer_id) {
      for (key, tile) in self.tiles.drain() {
        layer.tile_mut(key.0, key.1).copy_from_slice(&tile.before);
      }
    }
    (self.last, self.pointer, self.pulled, self.smudged) = (None, None, None, None);
    (self.travelled, self.carry, self.placed) = (0.0, 0.0, 0);
    self.dabs.clear();
    for pair in path.windows(2) {
      self.line(doc, pair[0], pair[1], pressure);
    }
  }

  /// Where the stroke is now, in canvas pixels.
  pub fn position(&self) -> Option<(f32, f32)> {
    self.last.map(|(x, y, _)| (x, y))
  }

  /// Dabs from `a` to `b` every `spacing × diameter`, interpolating the pressure.
  fn segment(&mut self, doc: &mut Document, a: (f32, f32, f32), b: (f32, f32, f32)) {
    let length = (b.0 - a.0).hypot(b.1 - a.1);
    if length > 0.0 {
      self.direction = (b.1 - a.1).atan2(b.0 - a.0);
    }
    let mut at = self.carry;
    while at <= length {
      let t = if length > 0.0 { at / length } else { 1.0 };
      let pressure = a.2 + (b.2 - a.2) * t;
      self.dab(doc, a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t, pressure);
      let step = (self.diameter(pressure, self.travelled) * self.tilt_scale() * self.brush.spacing).max(0.5);
      at += step;
      self.travelled += step;
    }
    self.carry = at - length;
  }

  /// The dab diameter at `pressure` and path length `at`, with the start taper.
  fn diameter(&self, pressure: f32, at: f32) -> f32 {
    let taper = if self.brush.taper_start > 0.0 { (at / self.brush.taper_start).min(1.0) } else { 1.0 };
    let pressure = 1.0 - self.brush.pressure_size * (1.0 - pressure);
    (self.brush.size * pressure * (0.15 + 0.85 * taper)).max(1.0)
  }

  /// Places one dab at (x, y) with the brush's jitter, stamps it and remembers it.
  fn dab(&mut self, doc: &mut Document, x: f32, y: f32, pressure: f32) {
    let b = self.brush;
    let seed = self.placed.wrapping_mul(4);
    self.placed += 1;
    // -1..1 from the dab's seed; the same stroke always gets the same jitter.
    let random = |k: u32| tip::unit(seed + k) * 2.0 - 1.0;
    let diameter = self.diameter(pressure, self.travelled);
    let (r, a) = (b.scatter * diameter * random(0).abs().sqrt(), random(1) * std::f32::consts::PI);
    let angle = b.angle.to_radians() + if b.follow { self.direction } else { 0.0 } + random(2) * b.angle_jitter * std::f32::consts::PI;
    let size = (1.0 - b.size_jitter.clamp(0.0, 1.0) * (random(3) * 0.5 + 0.5)) * self.tilt_scale();
    let (x, y) = (x + r * a.cos(), y + r * a.sin());
    let (color, load) = self.wet(doc, x, y, diameter / 2.0);
    let dab = Dab { x, y, pressure, at: self.travelled, angle, size, color, load, tilt: self.tilt };
    if self.smudge {
      return self.smudge_dab(doc, dab);
    }
    self.dabs.push(dab);
    for copy in self.copies(dab) {
      self.stamp(doc, copy, 1.0, None);
    }
  }

  /// Wet mixing and charge: the color this dab paints with and the paint left. The brush picks
  /// up the layer's color under it (as it was before the stroke) and carries it along.
  fn wet(&mut self, doc: &Document, x: f32, y: f32, radius: f32) -> ([f32; 3], f32) {
    let own = self.color.map(|c| c as f32);
    let b = self.brush;
    if self.erase || self.smudge || (b.wet_mix <= 0.0 && b.charge >= 1.0) {
      return (own, 1.0);
    }
    self.load = (self.load - (1.0 - b.charge.clamp(0.0, 1.0)) * 0.03).max(0.0);
    let wet = b.wet_mix.clamp(0.0, 1.0);
    if wet <= 0.0 {
      return (own, self.load);
    }
    if let Some(picked) = self.pick(doc, x, y, radius * 0.5) {
      let carried = self.carried.unwrap_or(own);
      self.carried = Some(std::array::from_fn(|ch| carried[ch] + (picked[ch] - carried[ch]) * wet * 0.5));
    }
    let carried = self.carried.unwrap_or(own);
    // Fresh paint shows more of the brush color; a dry brush shows what it picked up.
    let mix = wet * (1.0 - 0.5 * self.load);
    (std::array::from_fn(|ch| own[ch] + (carried[ch] - own[ch]) * mix), self.load)
  }

  /// The average straight color of the layer around (x, y), weighted by alpha, from before the
  /// stroke; `None` where it is empty.
  fn pick(&self, doc: &Document, x: f32, y: f32, reach: f32) -> Option<[f32; 3]> {
    let layer = doc.find(self.layer_id)?;
    let (mut sum, mut weight) = ([0.0f32; 3], 0.0f32);
    for (dx, dy) in [(0.0, 0.0), (-1.0, -1.0), (1.0, -1.0), (-1.0, 1.0), (1.0, 1.0), (0.0, -1.0), (0.0, 1.0), (-1.0, 0.0), (1.0, 0.0)] {
      let (px, py) = (x + dx * reach, y + dy * reach);
      if px < 0.0 || py < 0.0 || px >= doc.width as f32 || py >= doc.height as f32 {
        continue;
      }
      let (px, py) = (px as u32, py as u32);
      let key = (px / TILE, py / TILE);
      let tile = match self.tiles.get(&key) {
        Some(stroke) => &stroke.before[..],
        None => match layer.tile(key.0, key.1) {
          Some(pixels) => pixels,
          None => continue,
        },
      };
      let p = (((py % TILE) * TILE + px % TILE) * 4) as usize;
      // Premultiplied, so summing the color channels already weights them by alpha.
      for ch in 0..3 {
        sum[ch] += tile[p + ch] as f32;
      }
      weight += tile[p + 3] as f32;
    }
    (weight > 0.0).then(|| sum.map(|v| v / weight * 255.0))
  }

  /// The dab and its symmetric copies.
  fn copies(&self, d: Dab) -> Vec<Dab> {
    let copy = |m: &Mirror| {
      let (x, y) = m.point((d.x, d.y));
      Dab { x, y, angle: m.angle(d.angle), ..d }
    };
    std::iter::once(d).chain(self.mirrors.iter().map(copy)).collect()
  }

  /// How much dab `d` of `radius` covers canvas pixel (px, py): tip shape × grain.
  /// The brush's tip mask and grain texture, looked up once per dab rather than per pixel.
  fn textures(brush: &Brush) -> Textures {
    (brush.tip.mask(), brush.grain.texture())
  }

  fn shape(brush: &Brush, (mask, texture): Textures, d: &Dab, radius: f32, px: u32, py: u32) -> f32 {
    let (dx, dy) = (px as f32 + 0.5 - d.x, py as f32 + 0.5 - d.y);
    let (sin, cos) = d.angle.sin_cos();
    let (x, y) = ((dx * cos + dy * sin) / radius, (dy * cos - dx * sin) / (radius * brush.roundness.clamp(0.05, 1.0)));
    let shape = brush.tip.covers(mask, x, y, brush.hardness, radius);
    let Some(texture) = texture.filter(|_| shape > 0.0) else { return shape };
    let grain = Grain::at(texture, px as f32, py as f32, brush.grain_scale);
    shape * (1.0 - brush.grain_depth.clamp(0.0, 1.0) * (1.0 - grain))
  }

  /// The pixel box a dab of `radius` can touch, clipped to the canvas; `None` if off-canvas.
  fn bounds(&self, d: &Dab, radius: f32, width: u32, height: u32) -> Option<(u32, u32, u32, u32)> {
    let reach = radius * self.brush.tip.reach();
    let (x0, y0) = ((d.x - reach).floor().max(0.0), (d.y - reach).floor().max(0.0));
    let (x1, y1) = ((d.x + reach).ceil().min(width as f32 - 1.0), (d.y + reach).ceil().min(height as f32 - 1.0));
    (x1 >= x0 && y1 >= y0).then_some((x0 as u32, y0 as u32, x1 as u32, y1 as u32))
  }

  /// Stamps `d` at `scale` × its size, only into the `only` tiles when given.
  fn stamp(&mut self, doc: &mut Document, d: Dab, scale: f32, only: Option<&HashSet<(u32, u32)>>) {
    let radius = (self.diameter(d.pressure, d.at) * scale * d.size).max(1.0) / 2.0;
    let Some((x0, y0, x1, y1)) = self.bounds(&d, radius, doc.width, doc.height) else { return };
    let textures = Self::textures(&self.brush);
    let Some(layer) = doc.find_mut(self.layer_id) else { return };
    // A brush running dry thins down to a quarter.
    let flow = self.brush.flow
      * (1.0 - self.brush.pressure_flow * (1.0 - d.pressure))
      * (0.25 + 0.75 * d.load)
      * (1.0 - 0.85 * self.brush.tilt_flow.clamp(0.0, 1.0) * d.tilt);
    let opacity = self.brush.opacity.clamp(0.0, 1.0);
    let wet = self.brush.wet_mix > 0.0 && !self.erase;
    for ty in y0 / TILE..=y1 / TILE {
      for tx in x0 / TILE..=x1 / TILE {
        if only.is_some_and(|only| !only.contains(&(tx, ty))) {
          continue;
        }
        let pixels = layer.tile_mut(tx, ty);
        let tile = self.tiles.entry((tx, ty)).or_insert_with(|| StrokeTile {
          before: pixels.to_vec(),
          coverage: vec![0.0; (TILE * TILE) as usize],
          paint: if wet { vec![[0.0; 3]; (TILE * TILE) as usize] } else { Vec::new() },
        });
        for py in y0.max(ty * TILE)..=y1.min(ty * TILE + TILE - 1) {
          for px in x0.max(tx * TILE)..=x1.min(tx * TILE + TILE - 1) {
            let falloff = Self::shape(&self.brush, textures, &d, radius, px, py);
            if falloff <= 0.0 {
              continue;
            }
            let i = ((py - ty * TILE) * TILE + (px - tx * TILE)) as usize;
            let dab = falloff * flow;
            let c = &mut tile.coverage[i];
            let previous = *c;
            *c = if self.brush.buildup { *c + (1.0 - *c) * dab } else { c.max(dab) };
            let alpha = *c * opacity * Selection::weight(self.selection.as_ref(), px, py);
            // Wet: each dab tints the stroke's color by its share of the coverage.
            let color = if wet {
              let paint = &mut tile.paint[i];
              let share = if previous <= 0.0 { 1.0 } else { dab / (previous + dab) };
              *paint = std::array::from_fn(|ch| paint[ch] + (d.color[ch] - paint[ch]) * share);
              *paint
            } else {
              d.color
            };
            let p = i * 4;
            let before = &tile.before[p..p + 4];
            let out = &mut pixels[p..p + 4];
            if self.alpha_lock {
              // Keep the alpha; only painted pixels take the color (the eraser does nothing).
              if !self.erase {
                let keep = before[3] as f32 / 255.0;
                for ch in 0..3 {
                  out[ch] = (color[ch] * alpha * keep + before[ch] as f32 * (1.0 - alpha)).round() as u8;
                }
              }
            } else if self.erase {
              for ch in 0..4 {
                out[ch] = (before[ch] as f32 * (1.0 - alpha)).round() as u8;
              }
            } else {
              for ch in 0..3 {
                out[ch] = (color[ch] * alpha + before[ch] as f32 * (1.0 - alpha)).round() as u8;
              }
              out[3] = (255.0 * alpha + before[3] as f32 * (1.0 - alpha)).round() as u8;
            }
          }
        }
      }
    }
  }

  /// Moves the paint under the previous dab to (x, y), blended by the brush's shape × strength.
  fn smudge_dab(&mut self, doc: &mut Document, d: Dab) {
    let Some((from_x, from_y)) = self.smudged.replace((d.x, d.y)) else { return };
    let (width, height) = (doc.width, doc.height);
    let radius = (self.diameter(d.pressure, d.at) * d.size).max(1.0) / 2.0;
    let textures = Self::textures(&self.brush);
    let Some((x0, y0, x1, y1)) = self.bounds(&d, radius, width, height) else { return };
    let Some(layer) = doc.find_mut(self.layer_id) else { return };
    let strength = self.brush.opacity.clamp(0.0, 1.0) * (1.0 - self.brush.pressure_flow * (1.0 - d.pressure));
    let (x, y) = (d.x, d.y);
    // Copy the source patch first: it can overlap the pixels about to change.
    let (dx, dy) = ((from_x - x).round() as i64, (from_y - y).round() as i64);
    let read = |layer: &crate::Layer, px: u32, py: u32| -> [u8; 4] {
      let sx = (px as i64 + dx).clamp(0, width as i64 - 1) as u32;
      let sy = (py as i64 + dy).clamp(0, height as i64 - 1) as u32;
      layer.tile(sx / TILE, sy / TILE).map_or([0; 4], |t| {
        let p = (((sy % TILE) * TILE + sx % TILE) * 4) as usize;
        t[p..p + 4].try_into().expect("4 bytes")
      })
    };
    let columns = (x1 - x0 + 1) as usize;
    let patch: Vec<[u8; 4]> = (y0..=y1).flat_map(|py| (x0..=x1).map(move |px| (px, py))).map(|(px, py)| read(layer, px, py)).collect();
    for ty in y0 / TILE..=y1 / TILE {
      for tx in x0 / TILE..=x1 / TILE {
        let (rows, cols) = (y0.max(ty * TILE)..=y1.min(ty * TILE + TILE - 1), x0.max(tx * TILE)..=x1.min(tx * TILE + TILE - 1));
        // An empty tile only changes if paint is dragged into it.
        let receives = rows.clone().any(|py| cols.clone().any(|px| patch[(py - y0) as usize * columns + (px - x0) as usize][3] > 0));
        if layer.tile(tx, ty).is_none() && !receives {
          continue;
        }
        let pixels = layer.tile_mut(tx, ty);
        for py in rows.clone() {
          for px in cols.clone() {
            let weight = Self::shape(&self.brush, textures, &d, radius, px, py) * strength * Selection::weight(self.selection.as_ref(), px, py);
            if weight <= 0.0 {
              continue;
            }
            let source = patch[(py - y0) as usize * columns + (px - x0) as usize];
            let p = (((py % TILE) * TILE + px % TILE) * 4) as usize;
            let alpha = pixels[p + 3];
            for (out, src) in pixels[p..p + 4].iter_mut().zip(source) {
              *out = (*out as f32 + (src as f32 - *out as f32) * weight).round() as u8;
            }
            if self.alpha_lock {
              // Smudged color, the pixel's own alpha (premultiplied color never exceeds it).
              let old = pixels[p + 3].max(1) as f32;
              for ch in 0..3 {
                pixels[p + ch] = (pixels[p + ch] as f32 * alpha as f32 / old).round().min(alpha as f32) as u8;
              }
              pixels[p + 3] = alpha;
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
  use crate::{Grain, Tip};

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
      taper_start: 0.0,
      taper_end: 0.0,
      streamline: 0.0,
      motion_filter: 0.0,
      wet_mix: 0.0,
      charge: 1.0,
      tilt_size: 0.0,
      tilt_flow: 0.0,
      roundness: 1.0,
      string: 0.0,
      tip: Tip::Round,
      angle: 0.0,
      follow: false,
      angle_jitter: 0.0,
      scatter: 0.0,
      size_jitter: 0.0,
      grain: Grain::None,
      grain_scale: 1.0,
      grain_depth: 0.0,
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
    line(&mut doc, Brush { taper_start: 200.0, ..flat(40.0, 1.0) }, false, (10.0, 50.0), (390.0, 50.0));
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
  fn smudge_drags_paint_and_never_adds_any() {
    let mut doc = Document::new(256, 64);
    // Left half solid red, right half empty.
    let tile = doc.layer_mut(0).unwrap().tile_mut(0, 0);
    for y in 0..64 {
      for x in 0..128 {
        let p = (y * TILE as usize + x) * 4;
        tile[p..p + 4].copy_from_slice(&[255, 0, 0, 255]);
      }
    }
    let mut stroke = doc.begin_smudge(Brush { opacity: 0.8, ..flat(24.0, 1.0) }).unwrap();
    for x in (100..180).step_by(2) {
      stroke.add(&mut doc, x as f32, 32.0, 1.0);
    }
    let smeared = alpha(&doc, 0, 150, 32);
    assert!(smeared > 40, "red pulled into the empty half, got {smeared}");
    assert_eq!(alpha(&doc, 0, 150, 5), 0, "outside the brush nothing moves");
    assert!(doc.undo());
    assert_eq!(alpha(&doc, 0, 150, 32), 0);

    let mut empty = Document::new(64, 64);
    let mut stroke = empty.begin_smudge(flat(20.0, 1.0)).unwrap();
    stroke.add(&mut empty, 10.0, 10.0, 1.0);
    stroke.add(&mut empty, 50.0, 50.0, 1.0);
    assert_eq!(empty.layers()[0].painted_tiles(), 0, "smudging nothing creates no paint");
  }

  #[test]
  fn end_taper_thins_the_tail_when_the_stroke_ends() {
    let mut doc = Document::new(400, 100);
    let mut stroke = doc.begin_stroke(Brush { taper_end: 120.0, ..flat(40.0, 1.0) }, [0, 0, 0], false).unwrap();
    for i in 0..=20 {
      stroke.add(&mut doc, 10.0 + 19.0 * i as f32, 50.0, 1.0);
    }
    assert_eq!(alpha(&doc, 0, 385, 64), 255, "full width while drawing");
    stroke.finish(&mut doc);
    assert_eq!(alpha(&doc, 0, 385, 64), 0, "thin at the end once it ends");
    assert_eq!(alpha(&doc, 0, 385, 50), 255, "the tail is still there");
    assert_eq!(alpha(&doc, 0, 100, 64), 255, "the body keeps its width");
    assert!(doc.undo());
    assert_eq!(alpha(&doc, 0, 100, 50), 0, "still one undo step");
  }

  #[test]
  fn pulled_string_ignores_wobble_and_finish_catches_up() {
    let mut doc = Document::new(400, 100);
    let mut stroke = doc.begin_stroke(Brush { string: 30.0, ..flat(6.0, 1.0) }, [0, 0, 0], false).unwrap();
    stroke.add(&mut doc, 50.0, 50.0, 1.0);
    // A wobble inside the string's length draws nothing.
    stroke.add(&mut doc, 50.0, 75.0, 1.0);
    stroke.add(&mut doc, 50.0, 50.0, 1.0);
    assert_eq!(alpha(&doc, 0, 50, 70), 0);
    stroke.add(&mut doc, 300.0, 50.0, 1.0);
    assert_eq!(alpha(&doc, 0, 290, 50), 0, "the brush trails the pointer by the string");
    assert_eq!(stroke.finish(&mut doc), Some((300.0, 50.0)), "ending reaches the pointer");
    assert_eq!(alpha(&doc, 0, 299, 50), 255);
  }

  #[test]
  fn tips_and_grain_shape_the_dab() {
    let corner = |tip| {
      let mut doc = Document::new(100, 100);
      let mut stroke = doc.begin_stroke(Brush { tip, ..flat(40.0, 1.0) }, [0, 0, 0], false).unwrap();
      stroke.add(&mut doc, 50.0, 50.0, 1.0);
      alpha(&doc, 0, 67, 67)
    };
    assert_eq!(corner(Tip::Round), 0, "a disc misses the corner");
    assert_eq!(corner(Tip::Square), 255, "a square fills it");

    let mut doc = Document::new(300, 60);
    let brush = Brush { grain: Grain::Paper, grain_depth: 1.0, ..flat(40.0, 1.0) };
    line(&mut doc, brush, false, (20.0, 30.0), (280.0, 30.0));
    let row: Vec<u8> = (30..270).map(|x| alpha(&doc, 0, x, 30)).collect();
    assert!(row.iter().any(|&a| a < 128) && row.iter().any(|&a| a > 200), "paper grain leaves gaps");
  }

  #[test]
  fn quickshape_redraw_replaces_the_stroke() {
    let mut doc = Document::new(300, 300);
    let mut stroke = doc.begin_stroke(flat(6.0, 1.0), [0, 0, 0], false).unwrap();
    for i in 0..50 {
      stroke.add(&mut doc, 20.0 + i as f32 * 4.0, 150.0 + (i as f32).sin() * 30.0, 1.0);
    }
    assert_eq!(stroke.points().len(), 50);
    stroke.redraw(&mut doc, &[(20.0, 50.0), (250.0, 50.0)], 1.0);
    stroke.finish(&mut doc);
    assert_eq!(alpha(&doc, 0, 130, 50), 255, "the clean line");
    assert_eq!(alpha(&doc, 0, 100, 150), 0, "the wobbly stroke is gone");
    assert!(doc.undo());
    assert_eq!(alpha(&doc, 0, 130, 50), 0, "still one undo step");
  }

  #[test]
  fn symmetry_paints_copies_in_one_undo_step() {
    let mut doc = Document::new(400, 400);
    let brush = Brush { taper_end: 60.0, ..flat(8.0, 1.0) };
    let mut stroke = doc.begin_stroke(brush, [0, 0, 0], false).unwrap().with_symmetry(Symmetry::Quadrant, (200.0, 200.0));
    for i in 0..=40 {
      stroke.add(&mut doc, 40.0 + i as f32 * 3.0, 60.0, 1.0);
    }
    stroke.finish(&mut doc);
    for (x, y) in [(80, 60), (320, 60), (80, 340), (320, 340)] {
      assert_eq!(alpha(&doc, 0, x, y), 255, "copy at {x},{y}");
    }
    // The end taper thins every copy's tail, not only the original's.
    assert!(alpha(&doc, 0, 160, 64) < 255 && alpha(&doc, 0, 240, 336) < 255);
    assert!(doc.undo());
    assert_eq!(alpha(&doc, 0, 320, 340), 0);
  }

  #[test]
  fn motion_filter_removes_slow_jitter_but_keeps_fast_moves() {
    let jitter = |i: usize| if i.is_multiple_of(2) { 1.5 } else { -1.5 };
    let (mut filter, mut worst) = (MotionFilter::default(), 0.0f32);
    for i in 0..60 {
      let (_, y) = filter.apply((i as f32 * 1.0, 100.0 + jitter(i)), 1.0);
      if i > 10 {
        worst = worst.max((y - 100.0).abs());
      }
    }
    assert!(worst < 0.6, "slow shaky line smoothed: {worst}");
    let mut filter = MotionFilter::default();
    let mut last = (0.0, 0.0);
    for i in 0..20 {
      last = filter.apply((i as f32 * 40.0, 0.0), 1.0);
    }
    assert!(760.0 - last.0 < 40.0, "a fast stroke barely lags: {}", last.0);
    let mut off = MotionFilter::default();
    off.apply((0.0, 0.0), 0.0);
    assert_eq!(off.apply((5.0, 3.0), 0.0), (5.0, 3.0), "0 = off");
  }

  fn rgb(doc: &Document, x: u32, y: u32) -> [u8; 4] {
    let t = doc.layers()[0].tile(x / TILE, y / TILE).unwrap();
    let p = (((y % TILE) * TILE + x % TILE) * 4) as usize;
    t[p..p + 4].try_into().unwrap()
  }

  #[test]
  fn wet_brushes_pick_up_color_and_run_dry() {
    let mut doc = Document::new(400, 100);
    let red = flat(30.0, 1.0);
    let mut stroke = doc.begin_stroke(red, [255, 0, 0], false).unwrap();
    stroke.line(&mut doc, (20.0, 50.0), (150.0, 50.0), 1.0);
    stroke.finish(&mut doc);

    // A wet blue stroke out of the red: it drags red along into the empty part.
    let wet = Brush { wet_mix: 0.8, ..red };
    let mut stroke = doc.begin_stroke(wet, [0, 0, 255], false).unwrap();
    for i in 0..=60 {
      stroke.add(&mut doc, 100.0 + i as f32 * 4.0, 50.0, 1.0);
    }
    stroke.finish(&mut doc);
    let [r, _, b, _] = rgb(&doc, 200, 50);
    assert!(r > 60 && b > 60, "mixed purple past the red: {r} {b}");
    let [r, _, b, _] = rgb(&doc, 330, 50);
    assert!(r > 20 && b > r / 2, "still carrying some red: {r} {b}");

    // A dry brush (no charge) fades along the stroke.
    let mut doc = Document::new(400, 100);
    let mut stroke = doc.begin_stroke(Brush { charge: 0.0, ..flat(20.0, 1.0) }, [0, 0, 0], false).unwrap();
    for i in 0..=80 {
      stroke.add(&mut doc, 10.0 + i as f32 * 4.5, 50.0, 1.0);
    }
    stroke.finish(&mut doc);
    assert!(rgb(&doc, 20, 50)[3] > 200 && rgb(&doc, 360, 50)[3] < 120, "{} → {}", rgb(&doc, 20, 50)[3], rgb(&doc, 360, 50)[3]);
  }

  #[test]
  fn tilting_the_pen_widens_and_thins_the_stroke() {
    let brush = Brush { tilt_size: 1.0, tilt_flow: 0.5, ..flat(10.0, 1.0) };
    let mut doc = Document::new(300, 200);
    for (tilt, y) in [(0.0, 50.0), (1.0, 140.0)] {
      let mut stroke = doc.begin_stroke(brush, [0, 0, 0], false).unwrap();
      stroke.set_tilt(tilt);
      stroke.line(&mut doc, (40.0, y), (260.0, y), 1.0);
      stroke.finish(&mut doc);
    }
    let width = |y: u32| (0..60).filter(|dy| alpha(&doc, 0, 150, y - 30 + dy) > 0).count();
    assert!(width(50) <= 12 && width(140) >= 26, "upright {} vs flat {}", width(50), width(140));
    assert!(alpha(&doc, 0, 150, 50) == 255 && alpha(&doc, 0, 150, 140) < 160, "flat is lighter");
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
    // Smudge back over the painted band: the slowest tool, it reads and writes every pixel.
    let start = std::time::Instant::now();
    let mut stroke = doc.begin_smudge(Brush { size: 150.0, ..Brush::ink() }).unwrap();
    for i in 0..=samples {
      let t = i as f32 / samples as f32;
      stroke.add(&mut doc, 3800.0 - 3600.0 * t, 2048.0 + 800.0 * (t * 12.0).sin(), 1.0);
    }
    let elapsed = start.elapsed();
    eprintln!("{samples} samples of a 150 px smudge: {elapsed:?} ({:?} per sample)", elapsed / samples);
    let brush = Brush { size: 300.0, tip: Tip::Chalk, grain: Grain::Paper, grain_depth: 0.6, follow: true, ..Brush::airbrush() };
    let start = std::time::Instant::now();
    let mut stroke = doc.begin_stroke(brush, [0, 0, 0], false).unwrap();
    for i in 0..=samples {
      let t = i as f32 / samples as f32;
      stroke.add(&mut doc, 200.0 + 3600.0 * t, 1024.0 + 800.0 * (t * 12.0).sin(), 1.0);
    }
    let elapsed = start.elapsed();
    eprintln!("{samples} samples of a 300 px chalk + paper grain: {elapsed:?} ({:?} per sample)", elapsed / samples);
  }
}
