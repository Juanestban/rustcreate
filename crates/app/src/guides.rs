//! The drawing guide: a 2D or isometric grid, perspective with 1 to 3 vanishing points, or
//! symmetry axes, drawn over the canvas. With Drawing Assist, grid and perspective strokes lock to
//! the guide's lines and symmetry strokes are mirrored. Its points can be dragged while editing.

use std::f32::consts::{FRAC_PI_2, PI};

use eframe::egui::{Color32, Painter, Pos2, Stroke, Vec2, pos2};
use rustcreate_core::Symmetry;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Kind {
  #[default]
  Off,
  Grid,
  Isometric,
  Perspective,
  Symmetry,
}

/// Saved in the `.rcreate` file with `to_json`, restored with `from_json`.
pub struct Guides {
  pub kind: Kind,
  /// Grid spacing in canvas pixels.
  pub spacing: f32,
  pub symmetry: Symmetry,
  /// Vanishing points in use, 1 to 3.
  pub points: usize,
  pub assist: bool,
  /// Dragging on the canvas moves the guide's points instead of painting.
  pub editing: bool,
  /// Center of the grid and the symmetry, as a fraction of the canvas size.
  center: Pos2,
  /// Vanishing points, as fractions of the canvas size (they may lie outside it).
  vanishing: [Pos2; 3],
}

impl Default for Guides {
  fn default() -> Self {
    Self {
      kind: Kind::Off,
      spacing: 100.0,
      symmetry: Symmetry::Vertical,
      points: 1,
      assist: true,
      editing: false,
      center: pos2(0.5, 0.5),
      vanishing: [pos2(0.5, 0.4), pos2(1.1, 0.4), pos2(0.5, 1.6)],
    }
  }
}

/// The guide as stored in `document.json`.
#[derive(serde::Serialize, serde::Deserialize)]
struct Saved {
  kind: Kind,
  spacing: f32,
  symmetry: Symmetry,
  points: usize,
  assist: bool,
  center: [f32; 2],
  vanishing: [[f32; 2]; 3],
}

impl Guides {
  pub fn to_json(&self) -> serde_json::Value {
    let saved = Saved {
      kind: self.kind,
      spacing: self.spacing,
      symmetry: self.symmetry,
      points: self.points,
      assist: self.assist,
      center: [self.center.x, self.center.y],
      vanishing: self.vanishing.map(|p| [p.x, p.y]),
    };
    serde_json::to_value(saved).expect("guide serializes")
  }

  /// The guide saved in a file, or no guide. Values out of range (an edited or damaged file)
  /// are brought back in.
  pub fn from_json(value: Option<&serde_json::Value>) -> Self {
    let Some(saved) = value.and_then(|v| <Saved as serde::Deserialize>::deserialize(v).ok()) else { return Self::default() };
    let point =
      |[x, y]: [f32; 2], fallback: Pos2| if x.is_finite() && y.is_finite() { pos2(x.clamp(-10.0, 10.0), y.clamp(-10.0, 10.0)) } else { fallback };
    let default = Self::default();
    Self {
      kind: saved.kind,
      spacing: if saved.spacing.is_finite() { saved.spacing.clamp(10.0, 1000.0) } else { default.spacing },
      symmetry: match saved.symmetry {
        Symmetry::Radial(n) => Symmetry::Radial(n.clamp(2, 16)),
        other => other,
      },
      points: saved.points.clamp(1, 3),
      assist: saved.assist,
      editing: false,
      center: point(saved.center, default.center),
      vanishing: std::array::from_fn(|i| point(saved.vanishing[i], default.vanishing[i])),
    }
  }
}

/// Assisted strokes pick their direction after moving this far (canvas pixels).
const DECIDE: f32 = 6.0;

impl Guides {
  /// The draggable points, in canvas pixels: the vanishing points, or the center.
  pub fn handles(&self, size: Vec2) -> Vec<Pos2> {
    let scale = |p: Pos2| pos2(p.x * size.x, p.y * size.y);
    match self.kind {
      Kind::Off => Vec::new(),
      Kind::Perspective => self.vanishing[..self.points].iter().map(|&p| scale(p)).collect(),
      Kind::Grid | Kind::Isometric | Kind::Symmetry => vec![scale(self.center)],
    }
  }

  /// Moves handle `index` (from `handles`) to `p`, in canvas pixels.
  pub fn move_handle(&mut self, index: usize, p: Pos2, size: Vec2) {
    let p = pos2(p.x / size.x, p.y / size.y);
    match self.kind {
      Kind::Perspective => self.vanishing[index] = p,
      _ => self.center = p,
    }
  }

  /// The center symmetric strokes turn around, in canvas pixels.
  pub fn center(&self, size: Vec2) -> Pos2 {
    pos2(self.center.x * size.x, self.center.y * size.y)
  }

  /// Line directions in radians (mod π) a stroke starting at `start` may follow.
  fn directions(&self, start: Pos2, size: Vec2) -> Vec<f32> {
    match self.kind {
      Kind::Grid => vec![0.0, FRAC_PI_2],
      Kind::Isometric => vec![PI / 6.0, FRAC_PI_2, PI * 5.0 / 6.0],
      Kind::Perspective => {
        let toward = self.handles(size).into_iter().filter(|&vp| (vp - start).length() > 1.0).map(|vp| (vp - start).angle());
        // One point keeps horizontals and verticals, two points keep verticals.
        let straight: &[f32] = match self.points {
          1 => &[0.0, FRAC_PI_2],
          2 => &[FRAC_PI_2],
          _ => &[],
        };
        toward.chain(straight.iter().copied()).collect()
      }
      Kind::Off | Kind::Symmetry => Vec::new(),
    }
  }

  /// The symmetry new strokes use, if any.
  pub fn symmetry(&self) -> Option<Symmetry> {
    (self.kind == Kind::Symmetry && self.assist).then_some(self.symmetry)
  }

  /// Drawing Assist: once the stroke has moved a little from `start`, it keeps to the guide line
  /// closest to that first movement. `None` while it is still deciding.
  pub fn assist(&self, start: Pos2, p: Pos2, size: Vec2, axis: &mut Option<f32>) -> Option<Pos2> {
    if !self.assist {
      return Some(p);
    }
    let d = p - start;
    if axis.is_none() {
      let directions = self.directions(start, size);
      if directions.is_empty() {
        return Some(p);
      }
      if d.length() < DECIDE {
        return None;
      }
      let heading = d.y.atan2(d.x).rem_euclid(PI);
      let off = |a: f32| {
        let t = (heading - a).rem_euclid(PI);
        t.min(PI - t)
      };
      *axis = directions.into_iter().min_by(|a, b| off(*a).total_cmp(&off(*b)));
    }
    let along = Vec2::angled(axis.expect("chosen"));
    Some(start + along * d.dot(along))
  }

  /// Draws the guide over a canvas of `size` pixels; `to_screen` maps canvas pixels to points.
  pub fn draw(&self, painter: &Painter, size: Vec2, color: Color32, to_screen: impl Fn(Pos2) -> Pos2) {
    let stroke = Stroke::new(1.0, color);
    let line = |a: Pos2, b: Pos2| painter.line_segment([to_screen(a), to_screen(b)], stroke);
    let center = self.center(size);
    match self.kind {
      Kind::Off => {}
      Kind::Grid | Kind::Isometric => {
        for angle in self.directions(center, size) {
          // Lines along `angle`, `spacing` apart, through the center.
          let (along, normal) = (Vec2::angled(angle), Vec2::angled(angle + FRAC_PI_2));
          let corners = [pos2(0.0, 0.0), pos2(size.x, 0.0), pos2(0.0, size.y), size.to_pos2()];
          let (lo, hi) = corners.iter().map(|c| (*c - center).dot(normal)).fold((f32::MAX, f32::MIN), |(lo, hi), d| (lo.min(d), hi.max(d)));
          let (first, last) = ((lo / self.spacing).floor() as i32, (hi / self.spacing).ceil() as i32);
          // Too dense to read: skip rather than paint thousands of lines.
          if last - first > 800 {
            continue;
          }
          for k in first..=last {
            if let Some((a, b)) = clip(center + normal * (k as f32 * self.spacing), along, size) {
              line(a, b);
            }
          }
        }
      }
      Kind::Perspective => {
        let points = self.handles(size);
        // The horizon, through the first two points (level with one point).
        let horizon = match points[..] {
          [a, b, ..] => b - a,
          _ => Vec2::X,
        };
        if let Some((a, b)) = clip(points[0], horizon, size) {
          painter.line_segment([to_screen(a), to_screen(b)], Stroke::new(1.5, color));
        }
        for &vp in &points {
          for k in 0..18 {
            if let Some((a, b)) = clip(vp, Vec2::angled(k as f32 / 18.0 * PI), size) {
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
      }
    }
    // The points: small rings, bigger and filled while editing so they read as handles.
    for p in self.handles(size) {
      if self.editing {
        painter.circle(to_screen(p), 9.0, color.gamma_multiply(0.35), Stroke::new(2.0, color));
      } else if self.kind != Kind::Grid && self.kind != Kind::Isometric {
        painter.circle_stroke(to_screen(p), 5.0, stroke);
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

  const SIZE: Vec2 = Vec2::new(1000.0, 1000.0);

  #[test]
  fn guide_survives_json_and_rejects_bad_values() {
    let mut guide = Guides { kind: Kind::Perspective, points: 2, spacing: 64.0, ..Guides::default() };
    guide.move_handle(1, pos2(900.0, 200.0), SIZE);
    let back = Guides::from_json(Some(&guide.to_json()));
    assert_eq!((back.kind, back.points, back.spacing, back.vanishing), (guide.kind, 2, 64.0, guide.vanishing));
    let bad = serde_json::json!({ "kind": "grid", "spacing": 1e9, "symmetry": { "radial": 200 }, "points": 9, "assist": true,
      "center": [f32::MAX, 0.5], "vanishing": [[0.5, 0.4], [1.1, 0.4], [0.5, 1.6]] });
    let back = Guides::from_json(Some(&bad));
    assert_eq!((back.kind, back.spacing, back.points, back.symmetry), (Kind::Grid, 1000.0, 3, Symmetry::Radial(16)));
    assert_eq!(Guides::from_json(Some(&serde_json::json!("nonsense"))).kind, Kind::Off);
    assert_eq!(Guides::from_json(None).kind, Kind::Off);
  }

  #[test]
  fn assist_locks_to_the_nearest_grid_direction() {
    let guides = Guides { kind: Kind::Grid, ..Guides::default() };
    let (start, mut axis) = (pos2(100.0, 100.0), None);
    assert_eq!(guides.assist(start, pos2(102.0, 101.0), SIZE, &mut axis), None, "still deciding");
    let p = guides.assist(start, pos2(110.0, 102.0), SIZE, &mut axis).unwrap();
    assert_eq!(p, pos2(110.0, 100.0), "horizontal");
    let p = guides.assist(start, pos2(150.0, 140.0), SIZE, &mut axis).unwrap();
    assert_eq!(p.y, 100.0, "stays horizontal for the whole stroke");

    let iso = Guides { kind: Kind::Isometric, ..Guides::default() };
    let mut axis = None;
    let p = iso.assist(start, pos2(120.0, 108.0), SIZE, &mut axis).unwrap();
    assert!((axis.unwrap() - PI / 6.0).abs() < 1e-5 && (p - start).angle() - PI / 6.0 < 1e-4);

    let off = Guides { assist: false, ..guides };
    assert_eq!(off.assist(start, pos2(103.0, 107.0), SIZE, &mut None), Some(pos2(103.0, 107.0)));
  }

  #[test]
  fn perspective_assist_heads_for_the_vanishing_point() {
    let mut guides = Guides { kind: Kind::Perspective, points: 2, ..Guides::default() };
    // Vanishing points at (0, 400) and (1100, 400) on a 1000×1000 canvas.
    guides.move_handle(0, pos2(0.0, 400.0), SIZE);
    assert_eq!(guides.handles(SIZE), [pos2(0.0, 400.0), pos2(1100.0, 400.0)]);
    let (start, mut axis) = (pos2(500.0, 600.0), None);
    // Heading left and a bit up: toward the left vanishing point.
    let p = guides.assist(start, pos2(480.0, 590.0), SIZE, &mut axis).unwrap();
    let toward = (pos2(0.0, 400.0) - start).normalized();
    assert!(((p - start).normalized() - toward).length() < 1e-4, "{p:?}");
    // Straight up is a vertical with two points.
    let mut axis = None;
    let p = guides.assist(start, pos2(501.0, 560.0), SIZE, &mut axis).unwrap();
    assert!((p.x - 500.0).abs() < 1e-3);
  }

  #[test]
  fn the_center_moves_and_scales_with_the_canvas() {
    let mut guides = Guides { kind: Kind::Symmetry, ..Guides::default() };
    assert_eq!(guides.center(SIZE), pos2(500.0, 500.0));
    guides.move_handle(0, pos2(250.0, 100.0), SIZE);
    assert_eq!(guides.center(Vec2::new(2000.0, 500.0)), pos2(500.0, 50.0));
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
