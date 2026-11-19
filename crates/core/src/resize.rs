//! Crop, Canvas Size and Image Size: the canvas changes size, with every layer, mask and the
//! selection. Each is one undo step, and undo brings the old size back.

use std::collections::HashMap;
use std::sync::Arc;

use crate::{Document, Layer, MAX_SIDE, Selection, TILE, Tile};

impl Document {
  /// The new canvas shows the area of the old one at (`x`, `y`) of `width` × `height`; it may
  /// reach outside, where it is transparent. Paint outside it is cut. `false` if the size is
  /// out of range or nothing changes.
  pub fn reframe(&mut self, x: i64, y: i64, width: u32, height: u32) -> bool {
    if !(1..=MAX_SIDE).contains(&width) || !(1..=MAX_SIDE).contains(&height) || (x, y, width, height) == (0, 0, self.width, self.height) {
      return false;
    }
    self.checkpoint();
    // shortcut: text layers become plain pixels when the canvas changes size; move and scale the
    // text instead if re-editing after a crop is missed.
    self.layers.iter_mut().for_each(|l| l.text = None);
    let (old_w, old_h) = (self.width as usize, self.height as usize);
    let (w, h) = (width as usize, height as usize);
    // Copies the overlapping rows of a `channels`-per-pixel image into the new frame.
    let shift = |from: &[u8], channels: usize| {
      let mut to = vec![0u8; w * h * channels];
      let (x0, x1) = (x.max(0), (x + w as i64).min(old_w as i64));
      if x0 < x1 {
        for ny in 0..h {
          let oy = ny as i64 + y;
          if (0..old_h as i64).contains(&oy) {
            let src = (oy as usize * old_w + x0 as usize) * channels;
            let dst = (ny * w + (x0 - x) as usize) * channels;
            let len = (x1 - x0) as usize * channels;
            to[dst..dst + len].copy_from_slice(&from[src..src + len]);
          }
        }
      }
      to
    };
    for layer in &mut self.layers {
      layer.tiles = to_tiles(&shift(&image(layer, old_w, old_h), 4), w, h);
      if let Some(mask) = &mut layer.mask {
        mask.tiles = to_tiles(&shift(&image(mask, old_w, old_h), 4), w, h);
      }
    }
    self.selection = self.selection.take().and_then(|s| {
      let data = shift(s.data(), 1);
      data.iter().any(|&v| v > 0).then(|| Selection::new(width, height, data))
    });
    (self.width, self.height) = (width, height);
    true
  }

  /// Crops the canvas to the selected area's bounds. `false` without a selection.
  pub fn crop_to_selection(&mut self) -> bool {
    let Some(selection) = &self.selection else { return false };
    let w = self.width as usize;
    let mut bounds: Option<(usize, usize, usize, usize)> = None;
    for (i, _) in selection.data().iter().enumerate().filter(|&(_, &v)| v > 0) {
      let (x, y) = (i % w, i / w);
      bounds = Some(bounds.map_or((x, y, x, y), |(x0, y0, x1, y1)| (x0.min(x), y0.min(y), x1.max(x), y1.max(y))));
    }
    let Some((x0, y0, x1, y1)) = bounds else { return false };
    self.reframe(x0 as i64, y0 as i64, (x1 - x0 + 1) as u32, (y1 - y0 + 1) as u32)
  }

  /// Canvas Size: `anchor` (0..=1 each way) is the point of the old canvas that stays put, so
  /// (0.5, 0.5) grows or shrinks around the center and (0, 0) keeps the top left corner.
  pub fn resize_canvas(&mut self, width: u32, height: u32, anchor: (f32, f32)) -> bool {
    let x = ((self.width as f32 - width as f32) * anchor.0.clamp(0.0, 1.0)).round() as i64;
    let y = ((self.height as f32 - height as f32) * anchor.1.clamp(0.0, 1.0)).round() as i64;
    self.reframe(x, y, width, height)
  }

  /// Image Size: every layer, mask and the selection resampled to `width` × `height`.
  pub fn resize_image(&mut self, width: u32, height: u32) -> bool {
    if !(1..=MAX_SIDE).contains(&width) || !(1..=MAX_SIDE).contains(&height) || (width, height) == (self.width, self.height) {
      return false;
    }
    self.checkpoint();
    // shortcut: text layers become plain pixels when the canvas changes size; move and scale the
    // text instead if re-editing after a crop is missed.
    self.layers.iter_mut().for_each(|l| l.text = None);
    let (old, new) = ((self.width as usize, self.height as usize), (width as usize, height as usize));
    for layer in &mut self.layers {
      if !layer.tiles.is_empty() {
        layer.tiles = to_tiles(&resample(&image(layer, old.0, old.1), old, new, 4), new.0, new.1);
      }
      if let Some(mask) = &mut layer.mask {
        mask.tiles = to_tiles(&resample(&image(mask, old.0, old.1), old, new, 4), new.0, new.1);
      }
    }
    self.selection = self.selection.take().and_then(|s| {
      let data = resample(s.data(), old, new, 1);
      data.iter().any(|&v| v > 0).then(|| Selection::new(width, height, data))
    });
    (self.width, self.height) = (width, height);
    true
  }
}

/// A layer as one row-major RGBA image of the canvas size.
fn image(layer: &Layer, w: usize, h: usize) -> Vec<u8> {
  let mut out = vec![0u8; w * h * 4];
  let t = TILE as usize;
  for (&(tx, ty), tile) in &layer.tiles {
    let (x0, y0) = (tx as usize * t, ty as usize * t);
    if x0 >= w || y0 >= h {
      continue;
    }
    let len = t.min(w - x0) * 4;
    for row in 0..t.min(h - y0) {
      out[((y0 + row) * w + x0) * 4..][..len].copy_from_slice(&tile[row * t * 4..][..len]);
    }
  }
  out
}

/// Back to tiles; fully transparent ones are left out.
fn to_tiles(image: &[u8], w: usize, h: usize) -> HashMap<(u32, u32), Tile> {
  let t = TILE as usize;
  let mut tiles = HashMap::new();
  for ty in 0..h.div_ceil(t) {
    for tx in 0..w.div_ceil(t) {
      let (x0, y0) = (tx * t, ty * t);
      let len = t.min(w - x0) * 4;
      let mut tile = vec![0u8; t * t * 4];
      for row in 0..t.min(h - y0) {
        tile[row * t * 4..][..len].copy_from_slice(&image[((y0 + row) * w + x0) * 4..][..len]);
      }
      if tile.iter().any(|&v| v > 0) {
        tiles.insert((tx as u32, ty as u32), Arc::from(tile));
      }
    }
  }
  tiles
}

/// Separable resampling with a triangle filter that widens when shrinking (so it averages
/// instead of skipping pixels), across the CPU's cores. Premultiplied pixels, so colors do not
/// bleed from transparency.
fn resample(image: &[u8], (sw, sh): (usize, usize), (dw, dh): (usize, usize), channels: usize) -> Vec<u8> {
  // First along rows (sw → dw), then down columns (sh → dh). The 8-bit middle step is what
  // Pillow does too.
  let rows = weights(sw, dw);
  let wide = parallel_rows(sh, dw * channels, |y, out| {
    let src = &image[y * sw * channels..][..sw * channels];
    for (x, (first, w)) in rows.iter().enumerate() {
      for c in 0..channels {
        let sum: f32 = w.iter().enumerate().map(|(k, w)| src[(first + k) * channels + c] as f32 * w).sum();
        out[x * channels + c] = sum.round().clamp(0.0, 255.0) as u8;
      }
    }
  });
  let columns = weights(sh, dh);
  parallel_rows(dh, dw * channels, |y, out| {
    let (first, w) = &columns[y];
    let mut sums = vec![0f32; out.len()];
    for (k, w) in w.iter().enumerate() {
      for (s, v) in sums.iter_mut().zip(&wide[(first + k) * dw * channels..][..dw * channels]) {
        *s += *v as f32 * w;
      }
    }
    for (o, s) in out.iter_mut().zip(sums) {
      *o = s.round().clamp(0.0, 255.0) as u8;
    }
  })
}

/// For each destination index: the first source index and the normalized weights from there.
fn weights(from: usize, to: usize) -> Vec<(usize, Vec<f32>)> {
  let scale = from as f32 / to as f32;
  let support = scale.max(1.0);
  (0..to)
    .map(|i| {
      let center = (i as f32 + 0.5) * scale - 0.5;
      let first = ((center - support).floor() + 1.0).max(0.0) as usize;
      let last = ((center + support).ceil() as usize).min(from - 1);
      let mut w: Vec<f32> = (first..=last).map(|j| (1.0 - (j as f32 - center).abs() / support).max(0.0)).collect();
      let total: f32 = w.iter().sum();
      if total > 0.0 {
        w.iter_mut().for_each(|v| *v /= total);
      } else {
        // Nothing in reach (a tiny source): take the nearest pixel.
        w = vec![1.0];
        return (center.round().clamp(0.0, (from - 1) as f32) as usize, w);
      }
      (first, w)
    })
    .collect()
}

/// `rows` output rows of `len` bytes, each made by `row(y, out)`, split across the CPU's cores.
fn parallel_rows(rows: usize, len: usize, row: impl Fn(usize, &mut [u8]) + Sync) -> Vec<u8> {
  let mut out = vec![0u8; rows * len];
  let threads = std::thread::available_parallelism().map_or(1, |n| n.get());
  let band = rows.div_ceil(threads).max(1);
  std::thread::scope(|scope| {
    for (k, chunk) in out.chunks_mut(band * len).enumerate() {
      let row = &row;
      scope.spawn(move || {
        for (j, line) in chunk.chunks_mut(len).enumerate() {
          row(k * band + j, line);
        }
      });
    }
  });
  out
}

#[cfg(test)]
mod tests {
  use crate::{Document, SelectMode, SelectShape};

  #[test]
  fn canvas_size_crop_and_undo() {
    let mut doc = Document::new(300, 200);
    doc.layer_mut(0).unwrap().tile_mut(1, 0)[(10 * 256 + 20) * 4..][..4].copy_from_slice(&[255, 0, 0, 255]);
    // The red pixel is at (276, 10). Grow by 100 on the left (anchor right) and 50 at the bottom.
    assert!(doc.resize_canvas(400, 250, (1.0, 0.0)));
    assert_eq!((doc.width, doc.height), (400, 250));
    assert_eq!(doc.pixel(376, 10), [255, 0, 0, 255], "the paint moved with the anchor");
    assert_eq!(doc.pixel(276, 10), [0; 4]);
    doc.select_shape(&SelectShape::Rectangle((370.0, 5.0), (390.0, 15.0)), SelectMode::Replace);
    assert!(doc.crop_to_selection());
    assert_eq!((doc.width, doc.height), (20, 10));
    assert_eq!(doc.pixel(6, 5), [255, 0, 0, 255]);
    assert!(doc.layers()[0].painted_tiles() == 1);
    doc.undo();
    assert_eq!((doc.width, doc.height), (400, 250), "undoing the crop");
    doc.undo(); // the selection
    doc.undo();
    assert_eq!((doc.width, doc.height), (300, 200), "undo brings the old size back");
    assert_eq!(doc.pixel(276, 10), [255, 0, 0, 255]);
    assert!(!doc.resize_canvas(300, 200, (0.5, 0.5)), "same size changes nothing");
    assert!(!doc.resize_canvas(0, 10, (0.5, 0.5)));
  }

  #[test]
  fn image_size_averages_when_shrinking() {
    let mut doc = Document::new(4, 2);
    // Columns black, white, black, white: halving the width averages them to mid gray.
    let tile = doc.layer_mut(0).unwrap().tile_mut(0, 0);
    for y in 0..2 {
      for x in 0..4 {
        let v = if x % 2 == 0 { 0 } else { 255 };
        tile[(y * 256 + x) * 4..][..4].copy_from_slice(&[v, v, v, 255]);
      }
    }
    assert!(doc.resize_image(2, 1));
    let p = doc.pixel(0, 0);
    assert!(p[3] == 255 && (100..=155).contains(&p[0]), "{p:?}");
    assert!(doc.resize_image(8, 4));
    assert_eq!(doc.pixel(7, 3)[3], 255, "growing fills the new pixels");
  }
}
