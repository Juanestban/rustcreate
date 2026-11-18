//! Brush tip shapes and grain textures. The built-in ones are generated here from deterministic
//! noise (ADR 0006: no third-party brush images), once, and cached. Images from brushes the user
//! imports are registered at run time. Brushes refer to them by name or id, so `Brush` stays a
//! small `Copy` value.

use std::sync::{OnceLock, RwLock};

/// The shape a dab stamps.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Tip {
  /// A disc with a soft edge set by the brush's hardness.
  #[default]
  Round,
  /// A square, softened by hardness like the disc.
  Square,
  /// A disc eaten at by grit, like chalk or a dry pastel.
  Chalk,
  /// Many fine hairs in a disc, for dry-brush and bristle strokes.
  Bristles,
  /// Uneven drops scattered in a disc.
  Splatter,
  /// An imported shape, by the id `register` gave it. Round if it is not registered.
  Image(u32),
}

impl Tip {
  pub const ALL: [Tip; 5] = [Tip::Round, Tip::Square, Tip::Chalk, Tip::Bristles, Tip::Splatter];

  /// How strongly the dab covers the point (x, y), in the dab's own frame where the edge of the
  /// tip is at distance 1 from the center. `hardness` only applies to the analytic shapes.
  pub fn coverage(self, x: f32, y: f32, hardness: f32, radius: f32) -> f32 {
    self.covers(self.mask(), x, y, hardness, radius)
  }

  /// The tip's mask, looked up once per dab: `None` for the analytic shapes.
  pub(crate) fn mask(self) -> Option<&'static [f32]> {
    match self {
      Tip::Round | Tip::Square => None,
      Tip::Chalk => Some(mask(0, chalk)),
      Tip::Bristles => Some(mask(1, bristles)),
      Tip::Splatter => Some(mask(2, splatter)),
      Tip::Image(id) => image(id, false),
    }
  }

  /// `coverage` with the mask already looked up.
  pub(crate) fn covers(self, mask: Option<&[f32]>, x: f32, y: f32, hardness: f32, radius: f32) -> f32 {
    match (mask, self) {
      (Some(mask), _) => sample(mask, x, y),
      (None, Tip::Square) => edge(x.abs().max(y.abs()), hardness, radius),
      (None, _) => edge(x.hypot(y), hardness, radius),
    }
  }

  /// How far from the center (in radii) the shape can reach when rotated.
  pub fn reach(self) -> f32 {
    if self == Tip::Square { std::f32::consts::SQRT_2 } else { 1.0 }
  }
}

/// A paper texture the paint catches on, fixed to the canvas (like real paper under a pencil).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Grain {
  #[default]
  None,
  /// Soft, irregular paper tooth.
  Paper,
  /// A woven canvas.
  Canvas,
  /// Fine, even speckle.
  Noise,
  /// Big, soft billows: clouds and smoke.
  Clouds,
  /// Wavy bands, like light on water.
  Ripples,
  /// An imported texture, by the id `register` gave it. No grain if it is not registered.
  Image(u32),
}

impl Grain {
  pub const ALL: [Grain; 6] = [Grain::None, Grain::Paper, Grain::Canvas, Grain::Noise, Grain::Clouds, Grain::Ripples];

  /// 0..=1 at canvas pixel (x, y); `scale` 1 = one texture pixel per canvas pixel.
  pub fn value(self, x: f32, y: f32, scale: f32) -> f32 {
    self.texture().map_or(1.0, |texture| Self::at(texture, x, y, scale))
  }

  /// The texture, looked up once per dab: `None` means no grain.
  pub(crate) fn texture(self) -> Option<&'static [f32]> {
    match self {
      Grain::None => None,
      Grain::Paper => Some(grain(0, paper)),
      Grain::Canvas => Some(grain(1, canvas)),
      Grain::Noise => Some(grain(2, speckle)),
      Grain::Clouds => Some(grain(3, clouds)),
      Grain::Ripples => Some(grain(4, ripples)),
      Grain::Image(id) => image(id, true),
    }
  }

  /// `value` with the texture already looked up.
  pub(crate) fn at(texture: &[f32], x: f32, y: f32, scale: f32) -> f32 {
    let scale = scale.max(0.05);
    let (u, v) = ((x / scale) as i64, (y / scale) as i64);
    let n = GRAIN_SIZE as i64;
    texture[(v.rem_euclid(n) * n + u.rem_euclid(n)) as usize]
  }
}

/// The smoothstep edge of the analytic tips: 1 inside the hard core, 0 at distance 1. The soft
/// band is at least one pixel wide so hard edges stay antialiased.
fn edge(distance: f32, hardness: f32, radius: f32) -> f32 {
  let inner = hardness.clamp(0.0, 1.0).min(1.0 - 1.0 / radius.max(1.0)).max(0.0);
  if distance <= inner {
    1.0
  } else {
    let t = ((distance - inner) / (1.0 - inner).max(1e-3)).clamp(0.0, 1.0);
    1.0 - t * t * (3.0 - 2.0 * t)
  }
}

pub(crate) const MASK_SIZE: usize = 128;
pub(crate) const GRAIN_SIZE: usize = 256;

/// Imported images: id, whether it is a grain, and its values. Never freed: a brush may still
/// point at one, and they are few and small.
static IMAGES: RwLock<Vec<(u32, bool, &'static [f32])>> = RwLock::new(Vec::new());

pub(crate) fn image(id: u32, grain: bool) -> Option<&'static [f32]> {
  IMAGES.read().expect("image registry").iter().find(|(i, g, _)| *i == id && *g == grain).map(|(_, _, v)| *v)
}

/// Registers an imported tip (`MASK_SIZE`²) or grain (`GRAIN_SIZE`²) of 8-bit values and returns
/// its id, a hash of the pixels: the same image always gets the same id.
pub(crate) fn register(grain: bool, pixels: &[u8]) -> u32 {
  let side = if grain { GRAIN_SIZE } else { MASK_SIZE };
  assert_eq!(pixels.len(), side * side, "registered images are normalized first");
  // FNV-1a.
  let id = pixels.iter().fold(0x811c_9dc5u32 ^ grain as u32, |h, &b| (h ^ b as u32).wrapping_mul(0x0100_0193));
  if image(id, grain).is_none() {
    let values: &'static [f32] = Box::leak(pixels.iter().map(|&b| b as f32 / 255.0).collect());
    IMAGES.write().expect("image registry").push((id, grain, values));
  }
  id
}

fn mask(slot: usize, make: fn() -> Vec<f32>) -> &'static [f32] {
  static MASKS: [OnceLock<Vec<f32>>; 3] = [OnceLock::new(), OnceLock::new(), OnceLock::new()];
  MASKS[slot].get_or_init(make)
}

fn grain(slot: usize, make: fn() -> Vec<f32>) -> &'static [f32] {
  static GRAINS: [OnceLock<Vec<f32>>; 5] = [const { OnceLock::new() }; 5];
  GRAINS[slot].get_or_init(make)
}

/// Bilinear sample of a mask at (x, y) in -1..=1; 0 outside.
fn sample(mask: &[f32], x: f32, y: f32) -> f32 {
  let n = MASK_SIZE as f32;
  let (u, v) = ((x + 1.0) * 0.5 * n - 0.5, (y + 1.0) * 0.5 * n - 0.5);
  if !(-1.0..n).contains(&u) || !(-1.0..n).contains(&v) {
    return 0.0;
  }
  let at = |i: f32, j: f32| -> f32 { if i < 0.0 || j < 0.0 || i >= n || j >= n { 0.0 } else { mask[j as usize * MASK_SIZE + i as usize] } };
  let (i, j) = (u.floor(), v.floor());
  let (fx, fy) = (u - i, v - j);
  let top = at(i, j) + (at(i + 1.0, j) - at(i, j)) * fx;
  let bottom = at(i, j + 1.0) + (at(i + 1.0, j + 1.0) - at(i, j + 1.0)) * fx;
  top + (bottom - top) * fy
}

/// A well-mixed 32-bit hash (lowbias32), for repeatable noise and stroke jitter.
pub(crate) fn hash(mut x: u32) -> u32 {
  x ^= x >> 16;
  x = x.wrapping_mul(0x7feb_352d);
  x ^= x >> 15;
  x = x.wrapping_mul(0x846c_a68b);
  x ^ (x >> 16)
}

/// 0..1 from a hash.
pub(crate) fn unit(x: u32) -> f32 {
  (hash(x) >> 8) as f32 / (1u32 << 24) as f32
}

/// Smooth value noise that wraps every `period` cells, so textures tile seamlessly.
fn value_noise(x: f32, y: f32, period: u32, seed: u32) -> f32 {
  let (i, j) = (x.floor(), y.floor());
  let (fx, fy) = (x - i, y - j);
  let corner = |a: f32, b: f32| {
    let (a, b) = ((a as i64).rem_euclid(period as i64) as u32, (b as i64).rem_euclid(period as i64) as u32);
    unit(seed ^ hash(a.wrapping_mul(0x9e37) ^ b.wrapping_mul(0x85eb_ca6b)))
  };
  let (sx, sy) = (fx * fx * (3.0 - 2.0 * fx), fy * fy * (3.0 - 2.0 * fy));
  let top = corner(i, j) + (corner(i + 1.0, j) - corner(i, j)) * sx;
  let bottom = corner(i, j + 1.0) + (corner(i + 1.0, j + 1.0) - corner(i, j + 1.0)) * sx;
  top + (bottom - top) * sy
}

/// Octaves of value noise over a `size` square that tiles; 0..1.
fn fractal(px: usize, py: usize, size: usize, cells: u32, octaves: u32, seed: u32) -> f32 {
  let (mut total, mut weight, mut amplitude, mut cells) = (0.0, 0.0, 1.0, cells);
  for octave in 0..octaves {
    let scale = cells as f32 / size as f32;
    total += amplitude * value_noise(px as f32 * scale, py as f32 * scale, cells, seed.wrapping_add(octave * 7919));
    weight += amplitude;
    amplitude *= 0.5;
    cells *= 2;
  }
  total / weight
}

fn disc(px: usize, py: usize) -> f32 {
  let half = MASK_SIZE as f32 / 2.0;
  ((px as f32 + 0.5 - half) / half).hypot((py as f32 + 0.5 - half) / half)
}

fn chalk() -> Vec<f32> {
  let mut out = vec![0.0; MASK_SIZE * MASK_SIZE];
  for (i, v) in out.iter_mut().enumerate() {
    let (px, py) = (i % MASK_SIZE, i / MASK_SIZE);
    let grit = fractal(px, py, MASK_SIZE, 16, 3, 11);
    // The grit eats more of the disc toward its edge.
    let body = edge(disc(px, py), 0.55, 64.0);
    *v = ((grit - (1.0 - body) * 0.7 + 0.1) * 2.5).clamp(0.0, 1.0) * body;
  }
  out
}

fn bristles() -> Vec<f32> {
  let mut out = vec![0.0f32; MASK_SIZE * MASK_SIZE];
  let half = MASK_SIZE as f32 / 2.0;
  for hair in 0..60u32 {
    // Uniform over the disc: radius from the square root of a uniform number.
    let (r, a) = (unit(hair * 2 + 101).sqrt() * 0.85, unit(hair * 2 + 102) * std::f32::consts::TAU);
    let (cx, cy) = (half + r * a.cos() * half, half + r * a.sin() * half);
    let (size, strength) = (2.0 + unit(hair + 303) * 4.0, 0.5 + unit(hair + 404) * 0.5);
    for (i, v) in out.iter_mut().enumerate() {
      let d = ((i % MASK_SIZE) as f32 + 0.5 - cx).hypot((i / MASK_SIZE) as f32 + 0.5 - cy) / size;
      if d < 1.0 {
        *v = v.max(strength * (1.0 - d * d));
      }
    }
  }
  out
}

fn splatter() -> Vec<f32> {
  let mut out = vec![0.0f32; MASK_SIZE * MASK_SIZE];
  let half = MASK_SIZE as f32 / 2.0;
  for drop in 0..22u32 {
    let (r, a) = (unit(drop * 3 + 7).sqrt() * 0.8, unit(drop * 3 + 8) * std::f32::consts::TAU);
    let (cx, cy) = (half + r * a.cos() * half, half + r * a.sin() * half);
    let size = 2.0 + unit(drop * 3 + 9).powi(3) * 16.0;
    for (i, v) in out.iter_mut().enumerate() {
      let d = ((i % MASK_SIZE) as f32 + 0.5 - cx).hypot((i / MASK_SIZE) as f32 + 0.5 - cy) / size;
      *v = v.max(edge(d, 0.7, size));
    }
  }
  out
}

fn paper() -> Vec<f32> {
  (0..GRAIN_SIZE * GRAIN_SIZE).map(|i| (fractal(i % GRAIN_SIZE, i / GRAIN_SIZE, GRAIN_SIZE, 32, 4, 21) * 1.6 - 0.3).clamp(0.0, 1.0)).collect()
}

fn clouds() -> Vec<f32> {
  (0..GRAIN_SIZE * GRAIN_SIZE)
    .map(|i| {
      let v = fractal(i % GRAIN_SIZE, i / GRAIN_SIZE, GRAIN_SIZE, 4, 5, 77);
      // Stretched so there are clear gaps between the billows.
      ((v - 0.35) * 2.2).clamp(0.0, 1.0)
    })
    .collect()
}

fn ripples() -> Vec<f32> {
  (0..GRAIN_SIZE * GRAIN_SIZE)
    .map(|i| {
      let (x, y) = (i % GRAIN_SIZE, i / GRAIN_SIZE);
      // Bands across, bent by noise; 4 whole periods keep the texture seamless.
      let bend = fractal(x, y, GRAIN_SIZE, 4, 3, 91) * 6.0;
      let t = y as f32 / GRAIN_SIZE as f32 * std::f32::consts::TAU * 4.0 + bend;
      t.sin().abs().powf(0.6)
    })
    .collect()
}

fn canvas() -> Vec<f32> {
  (0..GRAIN_SIZE * GRAIN_SIZE)
    .map(|i| {
      let (x, y) = ((i % GRAIN_SIZE) as f32, (i / GRAIN_SIZE) as f32);
      // Threads every 8 px, over and under in a checker, with a little fiber noise.
      let (tx, ty) = ((x / 8.0 * std::f32::consts::PI).sin().abs(), (y / 8.0 * std::f32::consts::PI).sin().abs());
      let over = if ((x / 8.0) as u32 + (y / 8.0) as u32).is_multiple_of(2) { tx } else { ty };
      (over * 0.8 + fractal(i % GRAIN_SIZE, i / GRAIN_SIZE, GRAIN_SIZE, 64, 2, 31) * 0.4).clamp(0.0, 1.0)
    })
    .collect()
}

fn speckle() -> Vec<f32> {
  (0..(GRAIN_SIZE * GRAIN_SIZE) as u32).map(|i| unit(i ^ 0x5bd1_e995)).collect()
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn tips_are_inside_the_disc_and_not_empty() {
    for tip in Tip::ALL {
      let mut covered = 0;
      for j in 0..64 {
        for i in 0..64 {
          let (x, y) = (i as f32 / 32.0 - 1.0, j as f32 / 32.0 - 1.0);
          let c = tip.coverage(x, y, 0.5, 50.0);
          assert!((0.0..=1.0).contains(&c), "{tip:?} out of range");
          covered += (c > 0.2) as u32;
        }
      }
      assert!(covered > 100, "{tip:?} stamps something ({covered})");
      assert_eq!(tip.coverage(1.5, 1.5, 0.5, 50.0), 0.0, "{tip:?} is empty past its reach");
    }
  }

  #[test]
  fn grains_tile_and_vary() {
    for grain in Grain::ALL {
      let a = grain.value(3.0, 5.0, 1.0);
      assert_eq!(a, grain.value(3.0 + GRAIN_SIZE as f32, 5.0 - GRAIN_SIZE as f32, 1.0), "{grain:?} tiles");
      if grain != Grain::None {
        let values: Vec<f32> = (0..200).map(|i| grain.value(i as f32 * 3.7, i as f32 * 1.3, 1.0)).collect();
        let (lo, hi) = values.iter().fold((1.0f32, 0.0f32), |(lo, hi), v| (lo.min(*v), hi.max(*v)));
        assert!(hi - lo > 0.3, "{grain:?} has texture ({lo}..{hi})");
      }
    }
  }
}
