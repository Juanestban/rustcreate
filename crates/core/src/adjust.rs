//! Adjustments (Photoshop's Image > Adjustments): color changes to the active layer, or to its
//! mask while editing it, inside the selection. Each change redraws from the pixels there were
//! when the adjustment began, so a dialog can preview live; it is one undo step, and cancelling
//! puts everything back.

use std::collections::HashMap;
use std::sync::Arc;

use crate::{Document, Selection, TILE, TILE_BYTES, Tile};

#[derive(Debug, Clone, PartialEq)]
pub enum Adjustment {
  /// Hue in degrees (-180..=180); saturation and lightness -1..=1.
  HueSaturation {
    hue: f32,
    saturation: f32,
    lightness: f32,
  },
  /// Both -1..=1.
  BrightnessContrast {
    brightness: f32,
    contrast: f32,
  },
  /// Input levels: `black` and `white` 0..=1, `gamma` 0.1..=10 (1 = unchanged).
  Levels {
    black: f32,
    gamma: f32,
    white: f32,
  },
  /// Points (input, output) in 0..=1, joined by a smooth monotone curve.
  Curves(Vec<(f32, f32)>),
  Invert,
  Desaturate,
}

/// An adjustment ready to run per pixel: a table for the ones that treat each channel alike.
// One per adjustment run, so the table's size does not matter.
#[allow(clippy::large_enum_variant)]
enum Compiled {
  Table([u8; 256]),
  HueSaturation(f32, f32, f32),
  Desaturate,
}

impl Adjustment {
  fn compile(&self) -> Compiled {
    let table = |f: &dyn Fn(f32) -> f32| Compiled::Table(std::array::from_fn(|i| (f(i as f32 / 255.0).clamp(0.0, 1.0) * 255.0).round() as u8));
    match self {
      Adjustment::HueSaturation { hue, saturation, lightness } => {
        Compiled::HueSaturation(*hue, saturation.clamp(-1.0, 1.0), lightness.clamp(-1.0, 1.0))
      }
      Adjustment::Desaturate => Compiled::Desaturate,
      Adjustment::Invert => table(&|v| 1.0 - v),
      Adjustment::BrightnessContrast { brightness, contrast } => {
        // Contrast turns the line around mid gray: -1 flat, 0 unchanged, 1 almost a step.
        let slope = ((contrast.clamp(-1.0, 1.0) + 1.0) * std::f32::consts::FRAC_PI_4).min(1.5).tan();
        let shift = brightness.clamp(-1.0, 1.0) * 0.5;
        table(&|v| (v - 0.5) * slope + 0.5 + shift)
      }
      Adjustment::Levels { black, gamma, white } => {
        let (black, white) = (black.clamp(0.0, 1.0), white.clamp(0.0, 1.0));
        let span = (white - black).max(1.0 / 255.0);
        let power = 1.0 / gamma.clamp(0.1, 10.0);
        table(&|v| ((v - black) / span).clamp(0.0, 1.0).powf(power))
      }
      Adjustment::Curves(points) => {
        let curve = curve(points);
        table(&|v| curve(v))
      }
    }
  }
}

/// A smooth curve through `points` that never overshoots between them (Fritsch–Carlson);
/// flat beyond the first and last point. No points is the identity.
pub fn curve(points: &[(f32, f32)]) -> impl Fn(f32) -> f32 {
  let mut p: Vec<(f32, f32)> = points.iter().map(|&(x, y)| (x.clamp(0.0, 1.0), y.clamp(0.0, 1.0))).collect();
  p.sort_by(|a, b| a.0.total_cmp(&b.0));
  p.dedup_by(|a, b| (a.0 - b.0).abs() < 1e-4);
  if p.is_empty() {
    p = vec![(0.0, 0.0), (1.0, 1.0)];
  }
  let n = p.len();
  let secant: Vec<f32> = p.windows(2).map(|w| (w[1].1 - w[0].1) / (w[1].0 - w[0].0)).collect();
  let mut tangent = vec![0.0f32; n];
  if n > 1 {
    tangent[0] = secant[0];
    tangent[n - 1] = secant[n - 2];
    for i in 1..n - 1 {
      tangent[i] = if secant[i - 1] * secant[i] <= 0.0 { 0.0 } else { (secant[i - 1] + secant[i]) / 2.0 };
    }
    for i in 0..n - 1 {
      if secant[i] == 0.0 {
        (tangent[i], tangent[i + 1]) = (0.0, 0.0);
        continue;
      }
      let (a, b) = (tangent[i] / secant[i], tangent[i + 1] / secant[i]);
      let h = a.hypot(b);
      if h > 3.0 {
        (tangent[i], tangent[i + 1]) = (3.0 / h * a * secant[i], 3.0 / h * b * secant[i]);
      }
    }
  }
  move |x: f32| {
    if x <= p[0].0 {
      return p[0].1;
    }
    if x >= p[n - 1].0 {
      return p[n - 1].1;
    }
    let i = p.windows(2).position(|w| x < w[1].0).unwrap_or(n - 2);
    let (x0, y0, x1, y1) = (p[i].0, p[i].1, p[i + 1].0, p[i + 1].1);
    let h = x1 - x0;
    let t = (x - x0) / h;
    let (t2, t3) = (t * t, t * t * t);
    (2.0 * t3 - 3.0 * t2 + 1.0) * y0 + (t3 - 2.0 * t2 + t) * h * tangent[i] + (-2.0 * t3 + 3.0 * t2) * y1 + (t3 - t2) * h * tangent[i + 1]
  }
}

impl Compiled {
  /// Straight 0..=1 RGB in and out.
  fn apply(&self, rgb: [f32; 3]) -> [f32; 3] {
    match self {
      Compiled::Table(t) => rgb.map(|v| t[(v * 255.0).round() as usize] as f32 / 255.0),
      Compiled::Desaturate => [0.299 * rgb[0] + 0.587 * rgb[1] + 0.114 * rgb[2]; 3],
      Compiled::HueSaturation(hue, sat, light) => {
        let (h, s, l) = to_hsl(rgb);
        let s = if *sat >= 0.0 { s + (1.0 - s) * sat * s.min(1.0) } else { s * (1.0 + sat) };
        let rgb = from_hsl((h + hue / 360.0).rem_euclid(1.0), s.clamp(0.0, 1.0), l);
        // Lightness blends toward white or black, as Photoshop does.
        rgb.map(|v| if *light >= 0.0 { v + (1.0 - v) * light } else { v * (1.0 + light) })
      }
    }
  }
}

fn to_hsl([r, g, b]: [f32; 3]) -> (f32, f32, f32) {
  let (max, min) = (r.max(g).max(b), r.min(g).min(b));
  let l = (max + min) / 2.0;
  let d = max - min;
  if d <= 0.0 {
    return (0.0, 0.0, l);
  }
  let s = d / (1.0 - (2.0 * l - 1.0).abs()).max(1e-6);
  let h = if max == r {
    ((g - b) / d).rem_euclid(6.0)
  } else if max == g {
    (b - r) / d + 2.0
  } else {
    (r - g) / d + 4.0
  };
  (h / 6.0, s, l)
}

fn from_hsl(h: f32, s: f32, l: f32) -> [f32; 3] {
  let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
  let x = c * (1.0 - ((h * 6.0).rem_euclid(2.0) - 1.0).abs());
  let m = l - c / 2.0;
  let (r, g, b) = match (h * 6.0) as u32 {
    0 => (c, x, 0.0),
    1 => (x, c, 0.0),
    2 => (0.0, c, x),
    3 => (0.0, x, c),
    4 => (x, 0.0, c),
    _ => (c, 0.0, x),
  };
  [r + m, g + m, b + m]
}

/// An adjustment in progress, from [`Document::begin_adjust`].
pub struct Adjust {
  layer_id: u64,
  /// Masks are gray over white: every tile counts, missing ones too.
  mask: bool,
  /// The tiles as they were, by position (`None`: not painted).
  original: HashMap<(u32, u32), Option<Tile>>,
  selection: Option<Selection>,
}

impl Document {
  /// Starts adjusting the active layer (or the mask being edited), as one undo step. `None` if
  /// the layer is locked or empty.
  pub fn begin_adjust(&mut self) -> Option<Adjust> {
    if self.layers[self.active].locked {
      return None;
    }
    let mask = self.editing_mask();
    let layer = self.target();
    let original: HashMap<_, _> = if mask {
      let (cols, rows) = (self.width.div_ceil(TILE), self.height.div_ceil(TILE));
      (0..rows).flat_map(|ty| (0..cols).map(move |tx| (tx, ty))).map(|key| (key, layer.tiles.get(&key).cloned())).collect()
    } else {
      layer.tiles.iter().map(|(&key, tile)| (key, Some(tile.clone()))).collect()
    };
    if original.is_empty() {
      return None;
    }
    let layer_id = layer.id;
    self.checkpoint();
    Some(Adjust { layer_id, mask, original, selection: self.selection.clone() })
  }

  /// Applies an adjustment at once (Invert, Desaturate). `false` if nothing changed.
  pub fn adjust(&mut self, adjustment: &Adjustment) -> bool {
    let Some(adjust) = self.begin_adjust() else { return false };
    adjust.update(self, adjustment);
    true
  }
}

impl Adjust {
  /// Redraws the adjusted pixels for these settings. Tiles are split across the CPU's cores.
  pub fn update(&self, doc: &mut Document, adjustment: &Adjustment) {
    let compiled = adjustment.compile();
    let entries: Vec<_> = self.original.iter().collect();
    let threads = std::thread::available_parallelism().map_or(1, |n| n.get()).min(entries.len().max(1));
    let tiles: Vec<((u32, u32), Vec<u8>)> = std::thread::scope(|scope| {
      let workers: Vec<_> = entries
        .chunks(entries.len().div_ceil(threads).max(1))
        .map(|chunk| scope.spawn(|| chunk.iter().map(|&(&key, before)| (key, self.redraw(&compiled, key, before.as_deref()))).collect::<Vec<_>>()))
        .collect();
      workers.into_iter().flat_map(|w| w.join().expect("adjustment worker")).collect()
    });
    let Some(layer) = doc.find_mut(self.layer_id) else { return };
    for (key, pixels) in tiles {
      layer.set_tile(key, Some(Arc::from(pixels)));
    }
  }

  fn redraw(&self, compiled: &Compiled, (tx, ty): (u32, u32), before: Option<&[u8]>) -> Vec<u8> {
    let selection = self.selection.as_ref();
    let empty = [0u8; TILE_BYTES];
    let src = before.unwrap_or(&empty);
    let mut out = src.to_vec();
    for (i, (p, o)) in src.as_chunks::<4>().0.iter().zip(out.as_chunks_mut::<4>().0).enumerate() {
      let (px, py) = (tx * TILE + i as u32 % TILE, ty * TILE + i as u32 / TILE);
      let w = Selection::weight(selection, px, py);
      if w <= 0.0 || (!self.mask && p[3] == 0) {
        continue;
      }
      let (rgb, alpha) = if self.mask {
        // Over white, so a transparent mask pixel is white (shows) like everywhere else.
        let a = p[3] as f32 / 255.0;
        ([0, 1, 2].map(|c| p[c] as f32 / 255.0 + 1.0 - a), 1.0)
      } else {
        let a = p[3] as f32;
        ([0, 1, 2].map(|c| (p[c] as f32 / a).min(1.0)), a / 255.0)
      };
      let new = compiled.apply(rgb);
      let target = [new[0] * alpha, new[1] * alpha, new[2] * alpha, alpha].map(|v| v.clamp(0.0, 1.0) * 255.0);
      for c in 0..4 {
        o[c] = (p[c] as f32 + (target[c] - p[c] as f32) * w).round() as u8;
      }
    }
    out
  }

  /// Puts every pixel back and drops the undo step.
  pub fn cancel(self, doc: &mut Document) {
    doc.rollback();
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::{SelectMode, SelectShape};

  fn px(doc: &Document, x: usize) -> [u8; 4] {
    doc.layers()[0].tile(0, 0).unwrap()[x * 4..x * 4 + 4].try_into().unwrap()
  }

  #[test]
  fn invert_and_desaturate_keep_alpha_and_stay_in_the_selection() {
    let mut doc = Document::new(10, 1);
    doc.layer_mut(0).unwrap().tile_mut(0, 0)[..40].copy_from_slice(&[100, 0, 0, 128].repeat(10));
    doc.select_shape(&SelectShape::Rectangle((0.0, 0.0), (5.0, 1.0)), SelectMode::Replace);
    assert!(doc.adjust(&Adjustment::Invert));
    assert_eq!((px(&doc, 0), px(&doc, 8)), ([28, 128, 128, 128], [100, 0, 0, 128]), "straight red 200 inverts to 55, cyan");
    assert!(doc.adjust(&Adjustment::Desaturate));
    let p = px(&doc, 8);
    assert_eq!(p, [100, 0, 0, 128], "outside the selection nothing changes");
    let p = px(&doc, 0);
    assert!(p[0] == p[1] && p[1] == p[2] && p[3] == 128, "{p:?}");
  }

  #[test]
  fn live_updates_start_from_the_original_and_cancel_restores() {
    let mut doc = Document::new(4, 1);
    doc.layer_mut(0).unwrap().tile_mut(0, 0)[..16].copy_from_slice(&[128, 128, 128, 255].repeat(4));
    let adjust = doc.begin_adjust().unwrap();
    adjust.update(&mut doc, &Adjustment::BrightnessContrast { brightness: 0.5, contrast: 0.0 });
    assert!(px(&doc, 0)[0] > 180);
    adjust.update(&mut doc, &Adjustment::Levels { black: 0.0, gamma: 1.0, white: 1.0 });
    assert_eq!(px(&doc, 0), [128, 128, 128, 255], "each update redraws from the start");
    adjust.update(&mut doc, &Adjustment::HueSaturation { hue: 0.0, saturation: 0.0, lightness: -1.0 });
    assert_eq!(px(&doc, 0), [0, 0, 0, 255]);
    adjust.cancel(&mut doc);
    assert_eq!(px(&doc, 0), [128, 128, 128, 255]);
    assert!(!doc.history().can_undo(), "cancelling leaves no undo step");
  }

  #[test]
  fn hue_turns_and_curves_pass_through_their_points() {
    assert_eq!(from_hsl(to_hsl([1.0, 0.0, 0.0]).0 + 1.0 / 3.0, 1.0, 0.5).map(|v| (v * 100.0).round()), [0.0, 100.0, 0.0], "red + 120° = green");
    let c = curve(&[(0.0, 0.0), (0.25, 0.5), (1.0, 1.0)]);
    assert!((c(0.25) - 0.5).abs() < 1e-4 && c(0.0) == 0.0 && c(1.0) == 1.0);
    let values: Vec<f32> = (0..=100).map(|i| c(i as f32 / 100.0)).collect();
    assert!(values.windows(2).all(|w| w[1] >= w[0]), "monotone points give a monotone curve");
    assert!((curve(&[])(0.3) - 0.3).abs() < 1e-5, "no points is the identity");
  }

  #[test]
  fn an_empty_mask_inverts_to_hiding_everything() {
    let mut doc = Document::new(4, 1);
    doc.layer_mut(0).unwrap().tile_mut(0, 0)[..16].copy_from_slice(&[0, 0, 255, 255].repeat(4));
    doc.add_mask(0);
    assert!(doc.adjust(&Adjustment::Invert));
    assert_eq!(doc.composite_tile(0, 0).map_or(0, |t| t[3]), 0);
  }
}
