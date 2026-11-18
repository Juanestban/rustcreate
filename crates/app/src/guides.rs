//! The drawing guide: a 2D or isometric grid, or symmetry axes, drawn over the canvas. With
//! Drawing Assist, grid strokes lock to the grid's directions and symmetry strokes are mirrored.

use std::f32::consts::{FRAC_PI_2, PI};

use eframe::egui::{Color32, Painter, Pos2, Stroke, Vec2, pos2};
use rustcreate_core::Symmetry;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Kind {
  #[default]
  Off,
  Grid,
  Isometric,
  Symmetry,
}

/// shortcut: the guide belongs to the session, not the file; save it in `.rcp` when files grow
/// per-canvas settings.
pub struct Guides {
  pub kind: Kind,
  /// Grid spacing in canvas pixels.
  pub spacing: f32,
  pub symmetry: Symmetry,
  pub assist: bool,
}

impl Default for Guides {
  fn default() -> Self {
    Self { kind: Kind::Off, spacing: 100.0, symmetry: Symmetry::Vertical, assist: true }
  }
}

/// Assisted strokes pick their direction after moving this far (canvas pixels).
const DECIDE: f32 = 6.0;

impl Guides {
  /// Line directions in radians (mod π): the grid's, which assisted strokes follow.
  fn directions(&self) -> &'static [f32] {
    match self.kind {
      Kind::Grid => &[0.0, FRAC_PI_2],
      Kind::Isometric => &[PI / 6.0, FRAC_PI_2, PI * 5.0 / 6.0],
      Kind::Off | Kind::Symmetry => &[],
    }
  }

  /// The symmetry new strokes use, if any.
  pub fn symmetry(&self) -> Option<Symmetry> {
    (self.kind == Kind::Symmetry && self.assist).then_some(self.symmetry)
  }

  /// Drawing Assist on a grid: once the stroke has moved a little from `start`, it keeps to the
  /// grid direction closest to that first movement. `None` while it is still deciding.
  pub fn assist(&self, start: Pos2, p: Pos2, axis: &mut Option<f32>) -> Option<Pos2> {
    let directions = self.directions();
    if !self.assist || directions.is_empty() {
      return Some(p);
    }
    let d = p - start;
    if axis.is_none() {
      if d.length() < DECIDE {
        return None;
      }
      let heading = d.y.atan2(d.x).rem_euclid(PI);
      let off = |a: f32| {
        let t = (heading - a).rem_euclid(PI);
        t.min(PI - t)
      };
      *axis = directions.iter().copied().min_by(|a, b| off(*a).total_cmp(&off(*b)));
    }
    let along = Vec2::angled(axis.expect("just chosen"));
    Some(start + along * d.dot(along))
  }

  /// Draws the guide over a canvas of `size` pixels; `to_screen` maps canvas pixels to points.
  pub fn draw(&self, painter: &Painter, size: Vec2, color: Color32, to_screen: impl Fn(Pos2) -> Pos2) {
    let stroke = Stroke::new(1.0, color);
    let line = |a: Pos2, b: Pos2| painter.line_segment([to_screen(a), to_screen(b)], stroke);
    let center = (size / 2.0).to_pos2();
    match self.kind {
      Kind::Off => {}
      Kind::Grid | Kind::Isometric => {
        for &angle in self.directions() {
          // Lines along `angle`, `spacing` apart, through the center.
          let (along, normal) = (Vec2::angled(angle), Vec2::angled(angle + FRAC_PI_2));
          let corners = [pos2(0.0, 0.0), pos2(size.x, 0.0), pos2(0.0, size.y), size.to_pos2()];
          let reach = corners.iter().map(|c| (*c - center).dot(normal).abs()).fold(0.0, f32::max);
          // Too dense to read: skip rather than paint thousands of lines.
          let count = (reach / self.spacing) as i32;
          if count > 400 {
            continue;
          }
          for k in -count..=count {
            let origin = center + normal * (k as f32 * self.spacing);
            if let Some((a, b)) = clip(origin, along, size) {
              line(a, b);
            }
          }
        }
      }
      Kind::Symmetry => {
        let axes: Vec<f32> = match self.symmetry {
          Symmetry::Vertical => vec![FRAC_PI_2],
          Symmetry::Horizontal => vec![0.0],
          Symmetry::Quadrant => vec![0.0, FRAC_PI_2],
          // Spokes between the copies, the first one straight up.
          Symmetry::Radial(n) => (0..n.max(1)).map(|k| -FRAC_PI_2 + k as f32 / n.max(1) as f32 * 2.0 * PI).collect(),
        };
        let radial = matches!(self.symmetry, Symmetry::Radial(_));
        for angle in axes {
          if let Some((a, b)) = clip(center, Vec2::angled(angle), size) {
            // A spoke starts at the center; an axis crosses the whole canvas.
            line(if radial { center } else { a }, b);
          }
        }
        painter.circle_stroke(to_screen(center), 5.0, stroke);
      }
    }
  }
}

/// The part of the infinite line through `origin` along `along` inside the canvas.
fn clip(origin: Pos2, along: Vec2, size: Vec2) -> Option<(Pos2, Pos2)> {
  let (mut t0, mut t1) = (f32::MIN, f32::MAX);
  for (o, d, max) in [(origin.x, along.x, size.x), (origin.y, along.y, size.y)] {
    if d.abs() < 1e-6 {
      if o < 0.0 || o > max {
        return None;
      }
      continue;
    }
    let (a, b) = ((0.0 - o) / d, (max - o) / d);
    (t0, t1) = (t0.max(a.min(b)), t1.min(a.max(b)));
  }
  (t0 < t1).then(|| (origin + along * t0, origin + along * t1))
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn assist_locks_to_the_nearest_grid_direction() {
    let guides = Guides { kind: Kind::Grid, ..Guides::default() };
    let (start, mut axis) = (pos2(100.0, 100.0), None);
    assert_eq!(guides.assist(start, pos2(102.0, 101.0), &mut axis), None, "still deciding");
    let p = guides.assist(start, pos2(110.0, 102.0), &mut axis).unwrap();
    assert_eq!(p, pos2(110.0, 100.0), "horizontal");
    let p = guides.assist(start, pos2(150.0, 140.0), &mut axis).unwrap();
    assert_eq!(p.y, 100.0, "stays horizontal for the whole stroke");

    let iso = Guides { kind: Kind::Isometric, ..Guides::default() };
    let mut axis = None;
    let p = iso.assist(start, pos2(120.0, 108.0), &mut axis).unwrap();
    assert!((axis.unwrap() - PI / 6.0).abs() < 1e-5 && (p - start).angle() - PI / 6.0 < 1e-4);

    let off = Guides { assist: false, ..guides };
    assert_eq!(off.assist(start, pos2(103.0, 107.0), &mut None), Some(pos2(103.0, 107.0)));
  }

  #[test]
  fn clipping_keeps_lines_inside_the_canvas() {
    let size = Vec2::new(200.0, 100.0);
    let (a, b) = clip(pos2(100.0, 50.0), Vec2::X, size).unwrap();
    assert_eq!((a, b), (pos2(0.0, 50.0), pos2(200.0, 50.0)));
    assert_eq!(clip(pos2(100.0, 150.0), Vec2::X, size), None);
    let (a, b) = clip(pos2(100.0, 50.0), Vec2::angled(PI / 4.0), size).unwrap();
    assert!((a - pos2(50.0, 0.0)).length() < 1e-3 && (b - pos2(150.0, 100.0)).length() < 1e-3);
  }

  #[test]
  fn symmetry_needs_assist() {
    let mut guides = Guides { kind: Kind::Symmetry, symmetry: Symmetry::Radial(6), ..Guides::default() };
    assert_eq!(guides.symmetry(), Some(Symmetry::Radial(6)));
    guides.assist = false;
    assert_eq!(guides.symmetry(), None);
  }
}
