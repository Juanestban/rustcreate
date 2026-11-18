//! Free Transform (Ctrl+T): the selected pixels of the active layer (all of it without a
//! selection) are lifted, then drawn again through an affine map while the user drags; the
//! selection follows. One undo step; cancelling puts everything back.

use std::collections::{HashMap, HashSet};

use crate::{Document, Selection, TILE, Tile};

/// `[a, b, c, d, tx, ty]`: canvas point (x, y) goes to (a·x + b·y + tx, c·x + d·y + ty).
pub type Affine = [f32; 6];

pub const IDENTITY: Affine = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];

pub struct Transform {
  layer_id: u64,
  /// The layer with the lifted pixels removed: what every update draws over.
  lifted: HashMap<(u32, u32), Tile>,
  /// The lifted pixels, premultiplied 0..=255 floats, `size` from `origin`.
  source: Vec<[f32; 4]>,
  origin: (u32, u32),
  size: (u32, u32),
  /// Tiles the last update drew, to put back before the next one.
  drawn: HashSet<(u32, u32)>,
  selection: Option<Selection>,
}

impl Document {
  /// Lifts the active layer's selected pixels for a Free Transform. `None` if the layer is
  /// locked or nothing is there to move.
  // shortcut: moves the layer's pixels only; a layer mask stays where it is. Transform the
  // mask too (or target it, like painting) when that is missed.
  pub fn begin_transform(&mut self) -> Option<Transform> {
    let layer = &self.layers[self.active];
    if layer.locked {
      return None;
    }
    let selection = self.selection.clone();
    // The painted area, inside the selection.
    let mut bounds: Option<(u32, u32, u32, u32)> = None;
    for (tx, ty) in layer.tile_coords() {
      let pixels = layer.tile(tx, ty).expect("listed tile");
      for py in ty * TILE..((ty + 1) * TILE).min(self.height) {
        for px in tx * TILE..((tx + 1) * TILE).min(self.width) {
          let a = pixels[(((py % TILE) * TILE + px % TILE) * 4 + 3) as usize];
          if a > 0 && Selection::weight(selection.as_ref(), px, py) > 0.0 {
            bounds = Some(match bounds {
              None => (px, py, px, py),
              Some((x0, y0, x1, y1)) => (x0.min(px), y0.min(py), x1.max(px), y1.max(py)),
            });
          }
        }
      }
    }
    let (x0, y0, x1, y1) = bounds?;
    let (w, h) = (x1 - x0 + 1, y1 - y0 + 1);
    self.checkpoint();
    let layer = &mut self.layers[self.active];
    let mut source = vec![[0.0f32; 4]; (w * h) as usize];
    for ty in y0 / TILE..=y1 / TILE {
      for tx in x0 / TILE..=x1 / TILE {
        if layer.tile(tx, ty).is_none() {
          continue;
        }
        let pixels = layer.tile_mut(tx, ty);
        for py in y0.max(ty * TILE)..=y1.min(ty * TILE + TILE - 1) {
          for px in x0.max(tx * TILE)..=x1.min(tx * TILE + TILE - 1) {
            let weight = Selection::weight(selection.as_ref(), px, py);
            let p = (((py % TILE) * TILE + px % TILE) * 4) as usize;
            let out = &mut pixels[p..p + 4];
            let taken: [f32; 4] = std::array::from_fn(|ch| out[ch] as f32 * weight);
            source[((py - y0) * w + (px - x0)) as usize] = taken;
            for ch in 0..4 {
              out[ch] = (out[ch] as f32 - taken[ch]).round().clamp(0.0, 255.0) as u8;
            }
          }
        }
      }
    }
    let lifted = layer.tiles.clone();
    Some(Transform { layer_id: layer.id, lifted, source, origin: (x0, y0), size: (w, h), drawn: HashSet::new(), selection })
  }
}

impl Transform {
  /// The lifted area in canvas pixels: left, top, right, bottom.
  pub fn bounds(&self) -> (f32, f32, f32, f32) {
    let (x, y) = (self.origin.0 as f32, self.origin.1 as f32);
    (x, y, x + self.size.0 as f32, y + self.size.1 as f32)
  }

  /// Draws the lifted pixels through `m` over the layer they were lifted from.
  pub fn update(&mut self, doc: &mut Document, m: Affine) {
    // shortcut: redraws every covered pixel with bilinear sampling on each call (a full 4096²
    // layer took 76 ms in release, M-series Mac); move to the GPU if transforming big layers lags.
    let Some(inverse) = invert(m) else { return };
    let (width, height) = (doc.width, doc.height);
    let Some(layer) = doc.find_mut(self.layer_id) else { return };
    for key in self.drawn.drain() {
      layer.set_tile(key, self.lifted.get(&key).cloned());
    }
    let (l, t, r, b) = self.bounds();
    let corners = [(l, t), (r, t), (r, b), (l, b)].map(|p| apply(m, p));
    let (min_x, max_x) = corners.iter().fold((f32::MAX, f32::MIN), |(lo, hi), p| (lo.min(p.0), hi.max(p.0)));
    let (min_y, max_y) = corners.iter().fold((f32::MAX, f32::MIN), |(lo, hi), p| (lo.min(p.1), hi.max(p.1)));
    if max_x < 0.0 || max_y < 0.0 || min_x >= width as f32 || min_y >= height as f32 {
      return;
    }
    let (x0, y0) = (min_x.floor().max(0.0) as u32, min_y.floor().max(0.0) as u32);
    let (x1, y1) = ((max_x.ceil() as u32).min(width - 1), (max_y.ceil() as u32).min(height - 1));
    for ty in y0 / TILE..=y1 / TILE {
      for tx in x0 / TILE..=x1 / TILE {
        self.drawn.insert((tx, ty));
        let pixels = layer.tile_mut(tx, ty);
        for py in y0.max(ty * TILE)..=y1.min(ty * TILE + TILE - 1) {
          for px in x0.max(tx * TILE)..=x1.min(tx * TILE + TILE - 1) {
            let (sx, sy) = apply(inverse, (px as f32 + 0.5, py as f32 + 0.5));
            let src = self.sample(sx - self.origin.0 as f32 - 0.5, sy - self.origin.1 as f32 - 0.5);
            if src[3] <= 0.0 {
              continue;
            }
            let p = (((py % TILE) * TILE + px % TILE) * 4) as usize;
            let out = &mut pixels[p..p + 4];
            let keep = 1.0 - src[3] / 255.0;
            for ch in 0..4 {
              out[ch] = (src[ch] + out[ch] as f32 * keep).round().clamp(0.0, 255.0) as u8;
            }
          }
        }
      }
    }
  }

  /// Bilinear sample of the lifted pixels; transparent outside.
  fn sample(&self, x: f32, y: f32) -> [f32; 4] {
    let (w, h) = (self.size.0 as i64, self.size.1 as i64);
    let (fx, fy) = (x.floor(), y.floor());
    let (ax, ay) = (x - fx, y - fy);
    let at = |i: i64, j: i64| if i < 0 || j < 0 || i >= w || j >= h { [0.0; 4] } else { self.source[(j * w + i) as usize] };
    let (i, j) = (fx as i64, fy as i64);
    let (p00, p10, p01, p11) = (at(i, j), at(i + 1, j), at(i, j + 1), at(i + 1, j + 1));
    std::array::from_fn(|ch| {
      let top = p00[ch] + (p10[ch] - p00[ch]) * ax;
      let bottom = p01[ch] + (p11[ch] - p01[ch]) * ax;
      top + (bottom - top) * ay
    })
  }

  /// Keeps the transform drawn with `m`; the selection moves with it.
  pub fn finish(mut self, doc: &mut Document, m: Affine) {
    self.update(doc, m);
    let (Some(selection), Some(inverse)) = (self.selection.take(), invert(m)) else { return };
    let (w, h) = (selection.width(), selection.height());
    let data = (0..h)
      .flat_map(|y| (0..w).map(move |x| (x, y)))
      .map(|(x, y)| {
        let (sx, sy) = apply(inverse, (x as f32 + 0.5, y as f32 + 0.5));
        if sx < 0.0 || sy < 0.0 { 0 } else { selection.at(sx as u32, sy as u32) }
      })
      .collect();
    doc.selection = Some(Selection::new(w, h, data));
  }

  /// Puts the layer back as it was before `begin_transform`, with no undo step left behind.
  pub fn cancel(self, doc: &mut Document) {
    doc.rollback();
  }
}

fn apply(m: Affine, (x, y): (f32, f32)) -> (f32, f32) {
  (m[0] * x + m[1] * y + m[4], m[2] * x + m[3] * y + m[5])
}

fn invert(m: Affine) -> Option<Affine> {
  let det = m[0] * m[3] - m[1] * m[2];
  if det.abs() < 1e-8 {
    return None;
  }
  let (a, b, c, d) = (m[3] / det, -m[1] / det, -m[2] / det, m[0] / det);
  Some([a, b, c, d, -(a * m[4] + b * m[5]), -(c * m[4] + d * m[5])])
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::{SelectMode, SelectShape};

  fn alpha(doc: &Document, x: u32, y: u32) -> u8 {
    doc.layers()[doc.active()].tile(x / TILE, y / TILE).map_or(0, |t| t[(((y % TILE) * TILE + x % TILE) * 4 + 3) as usize])
  }

  fn square(doc: &mut Document) {
    let layer = doc.layer_mut(0).unwrap();
    for y in 10..20 {
      for x in 10..20 {
        let t = layer.tile_mut(0, 0);
        t[((y * TILE + x) * 4) as usize..][..4].copy_from_slice(&[255, 0, 0, 255]);
      }
    }
  }

  #[test]
  fn moves_scales_and_is_one_undo_step() {
    let mut doc = Document::new(300, 300);
    square(&mut doc);
    let steps = doc.history().steps();
    let mut transform = doc.begin_transform().unwrap();
    assert_eq!(transform.bounds(), (10.0, 10.0, 20.0, 20.0));
    // Twice as big, moved by (100, 100).
    transform.update(&mut doc, [2.0, 0.0, 0.0, 2.0, 100.0, 100.0]);
    transform.update(&mut doc, [1.0, 0.0, 0.0, 1.0, 50.0, 0.0]);
    assert_eq!((alpha(&doc, 15, 15), alpha(&doc, 65, 15), alpha(&doc, 130, 130)), (0, 255, 0), "updates replace each other");
    transform.finish(&mut doc, [2.0, 0.0, 0.0, 2.0, 100.0, 100.0]);
    assert_eq!((alpha(&doc, 15, 15), alpha(&doc, 125, 125), alpha(&doc, 138, 138)), (0, 255, 255));
    assert_eq!(doc.history().steps(), steps + 1);
    doc.undo();
    assert_eq!((alpha(&doc, 15, 15), alpha(&doc, 125, 125)), (255, 0));
  }

  #[test]
  fn only_the_selection_moves_and_cancel_restores() {
    let mut doc = Document::new(100, 100);
    square(&mut doc);
    doc.select_shape(&SelectShape::Rectangle((10.0, 10.0), (15.0, 20.0)), SelectMode::Replace);
    let steps = doc.history().steps();
    let mut transform = doc.begin_transform().unwrap();
    assert_eq!(transform.bounds(), (10.0, 10.0, 15.0, 20.0));
    transform.update(&mut doc, [1.0, 0.0, 0.0, 1.0, 40.0, 0.0]);
    assert_eq!((alpha(&doc, 12, 12), alpha(&doc, 17, 12), alpha(&doc, 52, 12)), (0, 255, 255));
    transform.cancel(&mut doc);
    assert_eq!((alpha(&doc, 12, 12), alpha(&doc, 52, 12)), (255, 0));
    assert_eq!(doc.history().steps(), steps, "cancel leaves no step");

    let transform = doc.begin_transform().unwrap();
    transform.finish(&mut doc, [1.0, 0.0, 0.0, 1.0, 40.0, 0.0]);
    assert_eq!(doc.selection().unwrap().at(52, 12), 255, "the selection moved along");
    assert_eq!(doc.selection().unwrap().at(12, 12), 0);
    assert!(Document::new(10, 10).begin_transform().is_none(), "an empty layer has nothing to move");
  }

  #[test]
  fn inverse_undoes_the_map() {
    let m = [0.8, -0.6, 0.6, 0.8, 12.0, -3.0];
    let p = apply(invert(m).unwrap(), apply(m, (7.0, 9.0)));
    assert!((p.0 - 7.0).abs() < 1e-4 && (p.1 - 9.0).abs() < 1e-4);
  }
}
