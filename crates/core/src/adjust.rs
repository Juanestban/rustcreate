//! Adjustments and filters (Photoshop's Image > Adjustments and Filter): color changes, blur,
//! sharpening and noise to the active layer, or to its
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
  /// Radius in pixels (about one standard deviation).
  GaussianBlur {
    radius: f32,
  },
  /// Sharpens by `amount` (0..=5) times the difference from a blur of `radius` pixels, where
  /// that difference is at least `threshold` (0..=1).
  UnsharpMask {
    radius: f32,
    amount: f32,
    threshold: f32,
  },
  /// `amount` 0..=1; monochrome adds the same noise to every channel.
  AddNoise {
    amount: f32,
    monochrome: bool,
  },
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
      Adjustment::GaussianBlur { .. } | Adjustment::UnsharpMask { .. } | Adjustment::AddNoise { .. } => {
        unreachable!("filters read neighbors and run in Adjust::filter")
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
    if !self.paintable() {
      return None;
    }
    let mask = self.editing_mask();
    let layer = self.target();
    if !mask && layer.tiles.is_empty() {
      return None;
    }
    // Every tile: a blur spreads paint into empty ones.
    let (cols, rows) = (self.width.div_ceil(TILE), self.height.div_ceil(TILE));
    let original: HashMap<_, _> =
      (0..rows).flat_map(|ty| (0..cols).map(move |tx| (tx, ty))).map(|key| (key, layer.tiles.get(&key).cloned())).collect();
    let layer_id = layer.id;
    self.checkpoint();
    self.rasterize_text();
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
    if matches!(adjustment, Adjustment::GaussianBlur { .. } | Adjustment::UnsharpMask { .. } | Adjustment::AddNoise { .. }) {
      return self.filter(doc, adjustment);
    }
    let compiled = adjustment.compile();
    // Color changes leave empty tiles empty.
    let entries: Vec<_> = self.original.iter().filter(|(_, before)| self.mask || before.is_some()).collect();
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

  /// Blur, sharpening and noise read neighbors, so they work on the whole layer at once.
  // shortcut: redraws the whole canvas on the CPU on every change; move it to the GPU if big
  // canvases drag.
  fn filter(&self, doc: &mut Document, adjustment: &Adjustment) {
    let (w, h) = (doc.width as usize, doc.height as usize);
    // The layer as it was, premultiplied; a mask as opaque gray over white.
    let mut image = vec![[0u8; 4]; w * h];
    for (&(tx, ty), before) in &self.original {
      for y in 0..TILE as usize {
        for x in 0..TILE as usize {
          let (px, py) = (tx as usize * TILE as usize + x, ty as usize * TILE as usize + y);
          if px >= w || py >= h {
            continue;
          }
          let p: [u8; 4] = before.as_deref().map_or([0; 4], |t| t[(y * TILE as usize + x) * 4..][..4].try_into().expect("4 bytes"));
          image[py * w + px] = if self.mask {
            let gray = |c: usize| (p[c] as u32 + 255 - p[3] as u32).min(255) as u8;
            [gray(0), gray(1), gray(2), 255]
          } else {
            p
          };
        }
      }
    }
    let result: Vec<[u8; 4]> = match *adjustment {
      Adjustment::GaussianBlur { radius } => blur(&image, w, h, radius),
      Adjustment::UnsharpMask { radius, amount, threshold } => {
        let blurred = blur(&image, w, h, radius);
        let (amount, threshold) = (amount.clamp(0.0, 5.0), threshold.clamp(0.0, 1.0) * 255.0);
        image
          .iter()
          .zip(&blurred)
          .map(|(o, b)| {
            let mut out = *o;
            for c in 0..3 {
              let d = o[c] as f32 - b[c] as f32;
              if d.abs() >= threshold.max(0.5) {
                out[c] = (o[c] as f32 + amount * d).round().clamp(0.0, o[3] as f32) as u8;
              }
            }
            out
          })
          .collect()
      }
      Adjustment::AddNoise { amount, monochrome } => {
        let amount = amount.clamp(0.0, 1.0);
        image
          .iter()
          .enumerate()
          .map(|(i, o)| {
            let mut out = *o;
            for c in 0..3 {
              let n = noise(i, if monochrome { 0 } else { c });
              out[c] = (o[c] as f32 + n * amount * o[3] as f32).round().clamp(0.0, o[3] as f32) as u8;
            }
            out
          })
          .collect()
      }
      _ => unreachable!("only filters come here"),
    };
    let selection = self.selection.as_ref();
    let Some(layer) = doc.find_mut(self.layer_id) else { return };
    for (&(tx, ty), before) in &self.original {
      let mut out = before.as_deref().map_or_else(|| vec![0; TILE_BYTES], <[u8]>::to_vec);
      let mut painted = false;
      for y in 0..TILE as usize {
        for x in 0..TILE as usize {
          let (px, py) = (tx as usize * TILE as usize + x, ty as usize * TILE as usize + y);
          if px >= w || py >= h {
            continue;
          }
          let (from, to) = (image[py * w + px], result[py * w + px]);
          let weight = Selection::weight(selection, px as u32, py as u32);
          let o = &mut out[(y * TILE as usize + x) * 4..][..4];
          for c in 0..4 {
            o[c] = (from[c] as f32 + (to[c] as f32 - from[c] as f32) * weight).round() as u8;
          }
          painted |= o[3] > 0;
        }
      }
      if before.is_none() && !painted && !self.mask {
        continue;
      }
      layer.set_tile((tx, ty), Some(Arc::from(out)));
    }
  }

  /// Puts every pixel back and drops the undo step.
  pub fn cancel(self, doc: &mut Document) {
    doc.rollback();
  }
}

/// Three box blurs ≈ a Gaussian of standard deviation `radius`, on premultiplied pixels (so
/// color does not bleed from transparent areas).
fn blur(image: &[[u8; 4]], w: usize, h: usize, radius: f32) -> Vec<[u8; 4]> {
  let r = ((((4.0 * radius.clamp(0.0, 250.0).powi(2) + 1.0).sqrt() - 1.0) / 2.0).round()) as usize;
  if r == 0 {
    return image.to_vec();
  }
  let mut out = image.to_vec();
  let (mut values, mut scratch) = (vec![0f32; image.len()], vec![0f32; image.len()]);
  for c in 0..4 {
    for (v, p) in values.iter_mut().zip(image) {
      *v = p[c] as f32;
    }
    for _ in 0..3 {
      blur_rows(&values, &mut scratch, w, r);
      blur_columns(&scratch, &mut values, w, h, r);
    }
    for (o, v) in out.iter_mut().zip(&values) {
      o[c] = v.round().clamp(0.0, 255.0) as u8;
    }
  }
  for o in &mut out {
    for c in 0..3 {
      o[c] = o[c].min(o[3]);
    }
  }
  out
}

/// One running-sum box blur of radius `r` along every row of `len` values, edges clamped; the
/// rows are split across the CPU's cores.
fn blur_rows(from: &[f32], to: &mut [f32], len: usize, r: usize) {
  let threads = std::thread::available_parallelism().map_or(1, |n| n.get());
  let chunk = (from.len() / len).div_ceil(threads).max(1) * len;
  std::thread::scope(|scope| {
    for (src, dst) in from.chunks(chunk).zip(to.chunks_mut(chunk)) {
      scope.spawn(move || {
        for (row, out) in src.chunks(len).zip(dst.chunks_mut(len)) {
          let n = row.len() as isize;
          let at = |i: isize| row[i.clamp(0, n - 1) as usize];
          let size = (2 * r + 1) as f32;
          let mut sum: f32 = (-(r as isize)..=r as isize).map(at).sum();
          for (i, o) in out.iter_mut().enumerate() {
            let i = i as isize;
            *o = sum / size;
            sum += at(i + r as isize + 1) - at(i - r as isize);
          }
        }
      });
    }
  });
}

/// The same box blur down every column. Bands of rows run in parallel, each sliding a row
/// of column sums, so memory is read in order.
fn blur_columns(from: &[f32], to: &mut [f32], w: usize, h: usize, r: usize) {
  let threads = std::thread::available_parallelism().map_or(1, |n| n.get());
  let band = h.div_ceil(threads).max(1);
  let row = |y: isize| &from[y.clamp(0, h as isize - 1) as usize * w..][..w];
  let size = (2 * r + 1) as f32;
  std::thread::scope(|scope| {
    for (k, dst) in to.chunks_mut(band * w).enumerate() {
      scope.spawn(move || {
        let y0 = (k * band) as isize;
        let mut sums = vec![0f32; w];
        for dy in -(r as isize)..=r as isize {
          for (s, v) in sums.iter_mut().zip(row(y0 + dy)) {
            *s += v;
          }
        }
        for (j, out) in dst.chunks_mut(w).enumerate() {
          let y = y0 + j as isize;
          for (o, s) in out.iter_mut().zip(&sums) {
            *o = s / size;
          }
          for ((s, add), sub) in sums.iter_mut().zip(row(y + r as isize + 1)).zip(row(y - r as isize)) {
            *s += add - sub;
          }
        }
      });
    }
  });
}

/// The same noise for a pixel every time (so a slider does not flicker), -1..1.
fn noise(i: usize, channel: usize) -> f32 {
  let mut x = (i as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ (channel as u64 + 1).wrapping_mul(0xBF58_476D_1CE4_E5B9);
  x ^= x >> 31;
  x = x.wrapping_mul(0x94D0_49BB_1331_11EB);
  x ^= x >> 29;
  (x >> 40) as f32 / (1u64 << 24) as f32 * 2.0 - 1.0
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
  fn blur_spreads_sharpen_steepens_and_noise_keeps_transparency() {
    let mut doc = Document::new(20, 20);
    // One opaque white pixel in the middle of an empty layer.
    doc.layer_mut(0).unwrap().tile_mut(0, 0)[(10 * 256 + 10) * 4..][..4].copy_from_slice(&[255; 4]);
    let a = |doc: &Document, x: usize, y: usize| doc.layers()[0].tile(0, 0).unwrap()[(y * 256 + x) * 4 + 3];
    let adjust = doc.begin_adjust().unwrap();
    adjust.update(&mut doc, &Adjustment::GaussianBlur { radius: 2.0 });
    assert!(a(&doc, 10, 10) < 255 && a(&doc, 12, 10) > 0 && a(&doc, 0, 0) == 0, "{} {}", a(&doc, 10, 10), a(&doc, 12, 10));
    adjust.update(&mut doc, &Adjustment::AddNoise { amount: 1.0, monochrome: false });
    assert_eq!((a(&doc, 10, 10), a(&doc, 0, 0)), (255, 0), "noise keeps alpha and leaves transparency alone");
    adjust.cancel(&mut doc);

    // A soft edge between gray levels gets steeper.
    let mut doc = Document::new(20, 1);
    let tile = doc.layer_mut(0).unwrap().tile_mut(0, 0);
    for x in 0..20 {
      let v = if x < 10 { 100 } else { 150 };
      tile[x * 4..x * 4 + 4].copy_from_slice(&[v, v, v, 255]);
    }
    assert!(doc.adjust(&Adjustment::UnsharpMask { radius: 2.0, amount: 1.0, threshold: 0.0 }));
    let px = |x: usize| doc.layers()[0].tile(0, 0).unwrap()[x * 4];
    assert!(px(9) < 100 && px(10) > 150 && px(0) == 100, "{} {} {}", px(9), px(10), px(0));
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
