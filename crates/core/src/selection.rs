//! Selections: a coverage mask over the canvas (0 outside, 255 inside, soft in between). Brush,
//! eraser, smudge, fill and gradient only change pixels inside it. Changing it is an undo step.

use std::sync::Arc;

use crate::Document;

/// How a new shape combines with the selection there is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SelectMode {
  #[default]
  Replace,
  /// Shift in Photoshop.
  Add,
  /// Alt in Photoshop.
  Subtract,
}

/// A shape to select, in canvas pixels.
#[derive(Debug, Clone, PartialEq)]
pub enum SelectShape {
  /// Two opposite corners.
  Rectangle((f32, f32), (f32, f32)),
  /// The ellipse inside the box with these opposite corners.
  Ellipse((f32, f32), (f32, f32)),
  /// A closed freehand outline (lasso).
  Polygon(Vec<(f32, f32)>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Selection {
  width: u32,
  height: u32,
  /// Row-major coverage; shared with the undo history.
  data: Arc<Vec<u8>>,
}

impl Selection {
  pub(crate) fn new(width: u32, height: u32, data: Vec<u8>) -> Self {
    Self { width, height, data: Arc::new(data) }
  }

  pub fn width(&self) -> u32 {
    self.width
  }

  pub fn height(&self) -> u32 {
    self.height
  }

  /// Coverage at a pixel, 0..=255; 0 outside the canvas.
  pub fn at(&self, x: u32, y: u32) -> u8 {
    if x < self.width && y < self.height { self.data[(y * self.width + x) as usize] } else { 0 }
  }

  /// Coverage as 0..=1, for multiplying a paint amount.
  pub(crate) fn weight(selection: Option<&Self>, x: u32, y: u32) -> f32 {
    selection.map_or(1.0, |s| s.at(x, y) as f32 / 255.0)
  }

  pub fn data(&self) -> &[u8] {
    &self.data
  }
}

/// Antialiased scanline fill of a closed polygon (even-odd), 4 sub-rows per pixel row.
fn rasterize(points: &[(f32, f32)], width: u32, height: u32) -> Vec<u8> {
  const SUB: usize = 4;
  let mut coverage = vec![0f32; (width * height) as usize];
  if points.len() < 3 {
    return vec![0; coverage.len()];
  }
  let (min_y, max_y) = points.iter().fold((f32::MAX, f32::MIN), |(lo, hi), p| (lo.min(p.1), hi.max(p.1)));
  let (y0, y1) = (min_y.floor().max(0.0) as u32, (max_y.ceil().max(0.0) as u32).min(height));
  let mut crossings = Vec::new();
  for row in y0..y1 {
    for sub in 0..SUB {
      let y = row as f32 + (sub as f32 + 0.5) / SUB as f32;
      crossings.clear();
      for (i, &(ax, ay)) in points.iter().enumerate() {
        let (bx, by) = points[(i + 1) % points.len()];
        if (ay <= y) != (by <= y) {
          crossings.push(ax + (y - ay) / (by - ay) * (bx - ax));
        }
      }
      crossings.sort_by(f32::total_cmp);
      for span in crossings.as_chunks::<2>().0 {
        let (from, to) = (span[0].clamp(0.0, width as f32), span[1].clamp(0.0, width as f32));
        if to <= from {
          continue;
        }
        let base = (row * width) as usize;
        let (first, last) = (from.floor() as u32, (to.ceil() as u32).min(width));
        for x in first..last {
          // The part of pixel x inside the span.
          let inside = (to.min(x as f32 + 1.0) - from.max(x as f32)).max(0.0);
          coverage[base + x as usize] += inside / SUB as f32;
        }
      }
    }
  }
  coverage.iter().map(|c| (c.min(1.0) * 255.0).round() as u8).collect()
}

impl SelectShape {
  fn outline(&self) -> Vec<(f32, f32)> {
    match self {
      SelectShape::Rectangle(a, b) => vec![(a.0, a.1), (b.0, a.1), (b.0, b.1), (a.0, b.1)],
      SelectShape::Ellipse(a, b) => {
        let (cx, cy, rx, ry) = ((a.0 + b.0) / 2.0, (a.1 + b.1) / 2.0, (b.0 - a.0).abs() / 2.0, (b.1 - a.1).abs() / 2.0);
        let n = ((rx + ry) * 0.5).clamp(32.0, 720.0) as usize;
        (0..n)
          .map(|i| {
            let t = i as f32 / n as f32 * std::f32::consts::TAU;
            (cx + rx * t.cos(), cy + ry * t.sin())
          })
          .collect()
      }
      SelectShape::Polygon(points) => points.clone(),
    }
  }
}

impl Document {
  pub fn selection(&self) -> Option<&Selection> {
    self.selection.as_ref()
  }

  /// Selects a shape, combined with the current selection by `mode`.
  pub fn select_shape(&mut self, shape: &SelectShape, mode: SelectMode) {
    let data = rasterize(&shape.outline(), self.width, self.height);
    self.combine(data, mode);
  }

  /// The selection becomes `data` combined by `mode`; empty means nothing selected.
  pub(crate) fn combine(&mut self, data: Vec<u8>, mode: SelectMode) {
    let old = self.selection.as_ref().map(|s| s.data.clone());
    let data: Vec<u8> = match (mode, old) {
      (SelectMode::Replace, _) | (SelectMode::Add, None) => data,
      (SelectMode::Add, Some(old)) => old.iter().zip(&data).map(|(&a, &b)| a.max(b)).collect(),
      // Nothing selected means everything is: subtracting cuts a hole in the whole canvas.
      (SelectMode::Subtract, None) => data.iter().map(|&b| 255 - b).collect(),
      (SelectMode::Subtract, Some(old)) => old.iter().zip(&data).map(|(&a, &b)| (a as u32 * (255 - b as u32) / 255) as u8).collect(),
    };
    self.checkpoint();
    self.selection = data.iter().any(|&v| v > 0).then(|| Selection::new(self.width, self.height, data));
  }

  /// Ctrl+A.
  pub fn select_all(&mut self) {
    self.checkpoint();
    self.selection = Some(Selection::new(self.width, self.height, vec![255; (self.width * self.height) as usize]));
  }

  /// Ctrl+D. `false` if nothing was selected.
  pub fn deselect(&mut self) -> bool {
    if self.selection.is_none() {
      return false;
    }
    self.checkpoint();
    self.selection = None;
    true
  }

  /// Ctrl+Shift+I. Nothing selected stays nothing selected.
  pub fn invert_selection(&mut self) -> bool {
    let Some(old) = &self.selection else { return false };
    let data: Vec<u8> = old.data.iter().map(|&v| 255 - v).collect();
    self.checkpoint();
    self.selection = data.iter().any(|&v| v > 0).then(|| Selection::new(self.width, self.height, data));
    true
  }

  /// Softens the selection's edge over about `radius` pixels (three box blurs ≈ a Gaussian).
  pub fn feather_selection(&mut self, radius: f32) -> bool {
    let Some(old) = &self.selection else { return false };
    let r = (radius / 1.7).round().max(0.0) as usize;
    if r == 0 {
      return false;
    }
    let (w, h) = (self.width as usize, self.height as usize);
    let mut values: Vec<f32> = old.data.iter().map(|&v| v as f32).collect();
    let mut scratch = vec![0f32; values.len()];
    for _ in 0..3 {
      box_blur(&values, &mut scratch, w, h, r, true);
      box_blur(&scratch, &mut values, w, h, r, false);
    }
    self.checkpoint();
    self.selection = Some(Selection::new(self.width, self.height, values.iter().map(|v| v.round().clamp(0.0, 255.0) as u8).collect()));
    true
  }
}

/// One running-sum box blur pass, along rows or columns, edges clamped.
fn box_blur(from: &[f32], to: &mut [f32], w: usize, h: usize, r: usize, rows: bool) {
  let (lines, len) = if rows { (h, w) } else { (w, h) };
  let index = |line: usize, i: usize| if rows { line * w + i } else { i * w + line };
  let size = (2 * r + 1) as f32;
  for line in 0..lines {
    let at = |i: isize| from[index(line, i.clamp(0, len as isize - 1) as usize)];
    let mut sum: f32 = (-(r as isize)..=r as isize).map(at).sum();
    for i in 0..len {
      to[index(line, i)] = sum / size;
      sum += at(i as isize + r as isize + 1) - at(i as isize - r as isize);
    }
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::Brush;

  #[test]
  fn shapes_combine_and_invert() {
    let mut doc = Document::new(100, 100);
    doc.select_shape(&SelectShape::Rectangle((10.0, 10.0), (50.0, 50.0)), SelectMode::Replace);
    let sel = doc.selection().unwrap();
    assert_eq!((sel.at(30, 30), sel.at(5, 5), sel.at(49, 49), sel.at(50, 50)), (255, 0, 255, 0));
    doc.select_shape(&SelectShape::Rectangle((40.0, 40.0), (80.0, 80.0)), SelectMode::Add);
    assert_eq!(doc.selection().unwrap().at(70, 70), 255);
    doc.select_shape(&SelectShape::Rectangle((0.0, 0.0), (100.0, 35.0)), SelectMode::Subtract);
    assert_eq!((doc.selection().unwrap().at(30, 30), doc.selection().unwrap().at(30, 40)), (0, 255));
    assert!(doc.invert_selection());
    assert_eq!(doc.selection().unwrap().at(30, 30), 255);
    doc.undo();
    assert_eq!(doc.selection().unwrap().at(30, 30), 0, "selection changes are undoable");
    assert!(doc.deselect() && doc.selection().is_none());
  }

  #[test]
  fn ellipse_edges_are_soft_and_feather_spreads() {
    let mut doc = Document::new(100, 100);
    doc.select_shape(&SelectShape::Ellipse((0.0, 0.0), (100.0, 100.0)), SelectMode::Replace);
    let sel = doc.selection().unwrap();
    assert_eq!((sel.at(50, 50), sel.at(2, 2)), (255, 0));
    assert!((1..255).contains(&sel.at(50, 0)) || (1..255).contains(&sel.at(14, 14)), "antialiased edge");
    doc.select_shape(&SelectShape::Rectangle((40.0, 0.0), (60.0, 100.0)), SelectMode::Replace);
    assert!(doc.feather_selection(10.0));
    let sel = doc.selection().unwrap();
    assert!(sel.at(37, 50) > 20 && sel.at(37, 50) < 200 && sel.at(50, 50) > 200, "{} {}", sel.at(37, 50), sel.at(50, 50));
  }

  #[test]
  fn painting_stays_inside_the_selection() {
    let mut doc = Document::new(100, 100);
    doc.select_shape(&SelectShape::Rectangle((0.0, 0.0), (50.0, 100.0)), SelectMode::Replace);
    let mut stroke = doc.begin_stroke(Brush { size: 20.0, ..Brush::ink() }, [0, 0, 0], false).unwrap();
    stroke.line(&mut doc, (10.0, 50.0), (90.0, 50.0), 1.0);
    stroke.finish(&mut doc);
    let alpha = |doc: &Document, x: u32| doc.layers()[0].tile(0, 0).map_or(0, |t| t[((50 * 256 + x) * 4 + 3) as usize]);
    assert!(alpha(&doc, 30) > 200 && alpha(&doc, 70) == 0, "{} {}", alpha(&doc, 30), alpha(&doc, 70));
    assert!(doc.fill(80, 80, [255, 0, 0], 0.1, true));
    assert_eq!(alpha(&doc, 70), 0, "the bucket stays inside too");
    assert!(alpha(&doc, 20) == 255);
  }
}
