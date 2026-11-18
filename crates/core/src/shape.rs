//! QuickShape: recognizes a hand-drawn line, arc, polyline, ellipse or polygon so a held stroke
//! can snap to a clean one, which can then be edited by its handles or made perfect.

use std::f32::consts::TAU;

type P = (f32, f32);

#[derive(Debug, Clone, PartialEq)]
pub enum Shape {
  Line(P, P),
  /// A circular arc from the first point, through the second, to the third.
  Arc(P, P, P),
  /// Open corners, start to end.
  Polyline(Vec<P>),
  Ellipse {
    center: P,
    radii: (f32, f32),
    angle: f32,
  },
  /// Closed corners.
  Polygon(Vec<P>),
}

impl Shape {
  /// Recognizes the shape `points` (pointer samples, canvas pixels) were drawn as; `None` when it
  /// matches nothing well (a scribble, a curve).
  pub fn recognize(points: &[P]) -> Option<Shape> {
    let path = resample(points, 128)?;
    let (min, max) =
      path.iter().fold(((f32::MAX, f32::MAX), (f32::MIN, f32::MIN)), |(lo, hi), p| ((lo.0.min(p.0), lo.1.min(p.1)), (hi.0.max(p.0), hi.1.max(p.1))));
    let size = (max.0 - min.0).hypot(max.1 - min.1);
    if size < 8.0 {
      return None;
    }
    let (first, last) = (path[0], path[path.len() - 1]);
    let closed = dist(first, last) < 0.2 * size;
    let corners = corners(&path, 0.035 * size, closed);
    let polygon_error = (3..=6).contains(&corners.len()).then(|| error(&path, &corners, closed) / size).filter(|&e| e < 0.015);
    if !closed {
      if error(&path, &[first, last], false) / size < 0.025 {
        return Some(Shape::Line(first, last));
      }
      let mid = path[path.len() / 2];
      let arc_error = circle(first, mid, last)
        .map(|(c, r)| path.iter().map(|&p| (dist(p, c) - r).abs()).sum::<f32>() / path.len() as f32 / size)
        .filter(|&e| e < 0.012);
      return match (polygon_error, arc_error) {
        (Some(p), Some(a)) if p <= a => Some(Shape::Polyline(corners)),
        (_, Some(_)) => Some(Shape::Arc(first, mid, last)),
        (Some(_), None) => Some(Shape::Polyline(corners)),
        (None, None) => None,
      };
    }
    let ellipse = fit_ellipse(&path).filter(|e| e.1 / size < 0.03);
    match (polygon_error, ellipse) {
      (Some(p), Some((shape, e))) => Some(if e / size < p { shape } else { Shape::Polygon(corners) }),
      (Some(_), None) => Some(Shape::Polygon(corners)),
      (None, Some((shape, _))) => Some(shape),
      (None, None) => None,
    }
  }

  /// The shape as a path to stroke; closed shapes end where they start.
  pub fn outline(&self) -> Vec<P> {
    match self {
      Shape::Line(a, b) => vec![*a, *b],
      Shape::Arc(a, m, b) => {
        let Some((c, r)) = circle(*a, *m, *b) else { return vec![*a, *b] };
        let angle = |p: P| (p.1 - c.1).atan2(p.0 - c.0);
        let (start, through, end) = (angle(*a), angle(*m), angle(*b));
        // The way round from start to end that passes the middle point.
        let ccw = (end - start).rem_euclid(TAU);
        let sweep = if (through - start).rem_euclid(TAU) <= ccw { ccw } else { ccw - TAU };
        let steps = (r * sweep.abs() / 4.0).clamp(8.0, 720.0) as usize;
        (0..=steps)
          .map(|i| {
            let t = start + sweep * i as f32 / steps as f32;
            (c.0 + r * t.cos(), c.1 + r * t.sin())
          })
          .collect()
      }
      Shape::Polyline(points) => points.clone(),
      Shape::Polygon(points) => points.iter().chain(points.first()).copied().collect(),
      Shape::Ellipse { center, radii, angle } => {
        // About one point every 4 px, at least 24.
        let steps = ((radii.0 + radii.1) * std::f32::consts::PI / 4.0).clamp(24.0, 720.0) as usize;
        let (sin, cos) = angle.sin_cos();
        (0..=steps)
          .map(|i| {
            let t = i as f32 / steps as f32 * TAU;
            let (u, v) = (radii.0 * t.cos(), radii.1 * t.sin());
            (center.0 + u * cos - v * sin, center.1 + u * sin + v * cos)
          })
          .collect()
      }
    }
  }
}

impl Shape {
  /// The points that can be dragged to edit the shape. An ellipse has its center, then the end
  /// of each axis.
  pub fn handles(&self) -> Vec<P> {
    match self {
      Shape::Line(a, b) => vec![*a, *b],
      Shape::Arc(a, m, b) => vec![*a, *m, *b],
      Shape::Polyline(points) | Shape::Polygon(points) => points.clone(),
      Shape::Ellipse { center, radii, angle } => {
        let (sin, cos) = angle.sin_cos();
        vec![*center, (center.0 + radii.0 * cos, center.1 + radii.0 * sin), (center.0 - radii.1 * sin, center.1 + radii.1 * cos)]
      }
    }
  }

  /// Moves handle `index` (from `handles`) to `p`. Dragging an ellipse's first axis turns it too.
  pub fn move_handle(&mut self, index: usize, p: P) {
    match self {
      Shape::Line(a, b) => *[a, b][index.min(1)] = p,
      Shape::Arc(a, m, b) => *[a, m, b][index.min(2)] = p,
      Shape::Polyline(points) | Shape::Polygon(points) => {
        if let Some(point) = points.get_mut(index) {
          *point = p;
        }
      }
      Shape::Ellipse { center, radii, angle } => {
        let (dx, dy) = (p.0 - center.0, p.1 - center.1);
        match index {
          0 => *center = p,
          1 => (radii.0, *angle) = (dx.hypot(dy).max(1.0), dy.atan2(dx)),
          _ => radii.1 = (-dx * angle.sin() + dy * angle.cos()).abs().max(1.0),
        }
      }
    }
  }

  /// The perfect version: a circle, a regular polygon (square, equilateral triangle…), lines at
  /// multiples of 15°, an arc symmetric about its middle.
  pub fn perfect(&self) -> Shape {
    let snap = |from: P, to: P| {
      let step = 15f32.to_radians();
      let (length, a) = (dist(from, to), ((to.1 - from.1).atan2(to.0 - from.0) / step).round() * step);
      (from.0 + length * a.cos(), from.1 + length * a.sin())
    };
    match self {
      Shape::Line(a, b) => Shape::Line(*a, snap(*a, *b)),
      Shape::Polyline(points) => {
        let mut out = vec![points[0]];
        for pair in points.windows(2) {
          let from = *out.last().expect("one point");
          out.push(snap(from, (from.0 + pair[1].0 - pair[0].0, from.1 + pair[1].1 - pair[0].1)));
        }
        Shape::Polyline(out)
      }
      Shape::Polygon(points) => {
        let n = points.len() as f32;
        let c = points.iter().fold((0.0, 0.0), |c, p| (c.0 + p.0 / n, c.1 + p.1 / n));
        let r = points.iter().map(|&p| dist(p, c)).sum::<f32>() / n;
        let start = (points[0].1 - c.1).atan2(points[0].0 - c.0);
        Shape::Polygon((0..points.len()).map(|k| (start + k as f32 / n * TAU).sin_cos()).map(|(s, co)| (c.0 + r * co, c.1 + r * s)).collect())
      }
      Shape::Ellipse { center, radii, angle } => {
        let r = (radii.0 + radii.1) / 2.0;
        Shape::Ellipse { center: *center, radii: (r, r), angle: *angle }
      }
      Shape::Arc(a, _, b) => {
        let outline = self.outline();
        let Some((c, r)) = circle(outline[0], outline[outline.len() / 2], outline[outline.len() - 1]) else { return self.clone() };
        // The middle point halfway round, so both halves match.
        let m = outline[outline.len() / 2];
        let t = (m.1 - c.1).atan2(m.0 - c.0);
        Shape::Arc(*a, (c.0 + r * t.cos(), c.1 + r * t.sin()), *b)
      }
    }
  }
}

/// The circle through three points, or `None` when they are in a line.
fn circle(a: P, b: P, c: P) -> Option<(P, f32)> {
  let d = 2.0 * (a.0 * (b.1 - c.1) + b.0 * (c.1 - a.1) + c.0 * (a.1 - b.1));
  let scale = dist(a, b).max(dist(b, c)).max(dist(a, c));
  if d.abs() < 1e-3 * scale * scale {
    return None;
  }
  let (a2, b2, c2) = (a.0 * a.0 + a.1 * a.1, b.0 * b.0 + b.1 * b.1, c.0 * c.0 + c.1 * c.1);
  let center = ((a2 * (b.1 - c.1) + b2 * (c.1 - a.1) + c2 * (a.1 - b.1)) / d, (a2 * (c.0 - b.0) + b2 * (a.0 - c.0) + c2 * (b.0 - a.0)) / d);
  Some((center, dist(center, a)))
}

fn dist(a: P, b: P) -> f32 {
  (a.0 - b.0).hypot(a.1 - b.1)
}

fn to_segment(p: P, a: P, b: P) -> f32 {
  let (dx, dy) = (b.0 - a.0, b.1 - a.1);
  let length2 = dx * dx + dy * dy;
  if length2 == 0.0 {
    return dist(p, a);
  }
  let t = (((p.0 - a.0) * dx + (p.1 - a.1) * dy) / length2).clamp(0.0, 1.0);
  dist(p, (a.0 + dx * t, a.1 + dy * t))
}

/// `n` points evenly spaced along the path, so slow and fast parts weigh the same.
fn resample(points: &[P], n: usize) -> Option<Vec<P>> {
  let length: f32 = points.windows(2).map(|w| dist(w[0], w[1])).sum();
  if points.len() < 2 || length == 0.0 {
    return None;
  }
  let step = length / (n - 1) as f32;
  let mut out = vec![points[0]];
  let (mut i, mut at, mut from) = (1, 0.0, points[0]);
  while out.len() < n - 1 && i < points.len() {
    let segment = dist(from, points[i]);
    if at + segment >= step {
      let t = (step - at) / segment;
      from = (from.0 + (points[i].0 - from.0) * t, from.1 + (points[i].1 - from.1) * t);
      out.push(from);
      at = 0.0;
    } else {
      at += segment;
      from = points[i];
      i += 1;
    }
  }
  out.push(points[points.len() - 1]);
  Some(out)
}

/// Ramer–Douglas–Peucker corners, then drops the ones that barely turn (a closed shape usually
/// starts in the middle of a side).
fn corners(path: &[P], epsilon: f32, closed: bool) -> Vec<P> {
  fn rdp(path: &[P], epsilon: f32, out: &mut Vec<P>) {
    let (a, b) = (path[0], path[path.len() - 1]);
    let far = (1..path.len() - 1).map(|i| (i, to_segment(path[i], a, b))).max_by(|x, y| x.1.total_cmp(&y.1));
    match far {
      Some((i, d)) if d > epsilon => {
        rdp(&path[..=i], epsilon, out);
        out.pop();
        rdp(&path[i..], epsilon, out);
      }
      _ => out.extend([a, b]),
    }
  }
  let mut out = Vec::new();
  rdp(path, epsilon, &mut out);
  if !closed {
    return out;
  }
  out.pop();
  // Remove the straightest corner while any turns less than 30°.
  while out.len() > 3 {
    let n = out.len();
    let turn = |i: usize| {
      let (a, b, c) = (out[(i + n - 1) % n], out[i], out[(i + 1) % n]);
      let (d1, d2) = ((b.1 - a.1).atan2(b.0 - a.0), (c.1 - b.1).atan2(c.0 - b.0));
      let t = (d2 - d1).rem_euclid(TAU);
      t.min(TAU - t)
    };
    let (i, t) = (0..n).map(|i| (i, turn(i))).min_by(|x, y| x.1.total_cmp(&y.1)).expect("corners");
    if t > 30f32.to_radians() {
      break;
    }
    out.remove(i);
  }
  out
}

/// Mean distance from the path to the polyline through `corners`.
fn error(path: &[P], corners: &[P], closed: bool) -> f32 {
  let mut segments: Vec<(P, P)> = corners.windows(2).map(|w| (w[0], w[1])).collect();
  if closed {
    segments.push((corners[corners.len() - 1], corners[0]));
  }
  path.iter().map(|&p| segments.iter().map(|&(a, b)| to_segment(p, a, b)).fold(f32::MAX, f32::min)).sum::<f32>() / path.len() as f32
}

/// The ellipse along the path's principal axes, and the mean distance of the path from it.
fn fit_ellipse(path: &[P]) -> Option<(Shape, f32)> {
  let n = path.len() as f32;
  let mean = path.iter().fold((0.0, 0.0), |m, p| (m.0 + p.0 / n, m.1 + p.1 / n));
  let (mut xx, mut xy, mut yy) = (0.0, 0.0, 0.0);
  for p in path {
    let (dx, dy) = (p.0 - mean.0, p.1 - mean.1);
    (xx, xy, yy) = (xx + dx * dx, xy + dx * dy, yy + dy * dy);
  }
  let angle = 0.5 * (2.0 * xy).atan2(xx - yy);
  let (sin, cos) = angle.sin_cos();
  let local: Vec<P> = path.iter().map(|p| ((p.0 - mean.0) * cos + (p.1 - mean.1) * sin, -(p.0 - mean.0) * sin + (p.1 - mean.1) * cos)).collect();
  let (lo, hi) =
    local.iter().fold(((f32::MAX, f32::MAX), (f32::MIN, f32::MIN)), |(lo, hi), p| ((lo.0.min(p.0), lo.1.min(p.1)), (hi.0.max(p.0), hi.1.max(p.1))));
  let (cu, cv) = ((lo.0 + hi.0) / 2.0, (lo.1 + hi.1) / 2.0);
  let radii = ((hi.0 - lo.0) / 2.0, (hi.1 - lo.1) / 2.0);
  if radii.0 < 1.0 || radii.1 < 1.0 {
    return None;
  }
  let error = local
    .iter()
    .map(|&(u, v)| {
      let (u, v) = (u - cu, v - cv);
      let d = u.hypot(v);
      let r = ((u / radii.0).powi(2) + (v / radii.1).powi(2)).sqrt();
      if r > 0.0 { d * (1.0 - 1.0 / r).abs() } else { radii.0.min(radii.1) }
    })
    .sum::<f32>()
    / n;
  let center = (mean.0 + cu * cos - cv * sin, mean.1 + cu * sin + cv * cos);
  Some((Shape::Ellipse { center, radii, angle }, error))
}

#[cfg(test)]
mod tests {
  use super::*;

  /// A hand-drawn-ish path through `corners`: wobble across the direction of travel.
  fn drawn(corners: &[P]) -> Vec<P> {
    let mut out = Vec::new();
    for (k, w) in corners.windows(2).enumerate() {
      let (a, b) = (w[0], w[1]);
      let (dx, dy) = (b.0 - a.0, b.1 - a.1);
      let length = dx.hypot(dy);
      for i in 0..20 {
        let t = i as f32 / 20.0;
        let wobble = (t * 9.0 + k as f32).sin() * 0.6;
        out.push((a.0 + dx * t - dy / length * wobble, a.1 + dy * t + dx / length * wobble));
      }
    }
    out.push(corners[corners.len() - 1]);
    out
  }

  #[test]
  fn recognizes_arcs_and_draws_them_through_the_middle() {
    let arc: Vec<P> = (0..=60).map(|i| (i as f32 / 60.0 * 2.0).sin_cos()).map(|(s, c)| (200.0 + 100.0 * c, 200.0 + 100.0 * s)).collect();
    let Some(shape @ Shape::Arc(..)) = Shape::recognize(&arc) else { panic!("an arc: {:?}", Shape::recognize(&arc)) };
    let outline = shape.outline();
    assert!(outline.iter().all(|&p| (dist(p, (200.0, 200.0)) - 100.0).abs() < 2.0), "on the circle");
    // Through the middle, not the long way round.
    assert!(outline.iter().all(|&p| p.1 >= 195.0));
  }

  #[test]
  fn handles_edit_and_perfect_shapes() {
    let mut line = Shape::Line((0.0, 0.0), (100.0, 3.0));
    line.move_handle(1, (100.0, 8.0));
    let Shape::Line(_, end) = line.perfect() else { unreachable!() };
    assert!(end.1.abs() < 1e-3 && (end.0 - 100.3).abs() < 0.1, "snapped flat: {end:?}");

    let wobbly = Shape::Polygon(vec![(0.0, 0.0), (110.0, 5.0), (104.0, 96.0), (-3.0, 101.0)]);
    let Shape::Polygon(square) = wobbly.perfect() else { unreachable!() };
    let sides: Vec<f32> = (0..4).map(|i| dist(square[i], square[(i + 1) % 4])).collect();
    assert!(sides.iter().all(|s| (s - sides[0]).abs() < 1e-2), "{sides:?}");

    let mut ellipse = Shape::Ellipse { center: (50.0, 50.0), radii: (40.0, 20.0), angle: 0.0 };
    assert_eq!(ellipse.handles()[1], (90.0, 50.0));
    ellipse.move_handle(2, (50.0, 80.0));
    ellipse.move_handle(0, (60.0, 60.0));
    assert_eq!(ellipse, Shape::Ellipse { center: (60.0, 60.0), radii: (40.0, 30.0), angle: 0.0 });
    assert!(matches!(ellipse.perfect(), Shape::Ellipse { radii: (35.0, 35.0), .. }));
  }

  #[test]
  fn recognizes_lines_and_polylines() {
    assert!(matches!(Shape::recognize(&drawn(&[(10.0, 10.0), (200.0, 60.0)])), Some(Shape::Line(..))));
    let Some(Shape::Polyline(points)) = Shape::recognize(&drawn(&[(10.0, 10.0), (200.0, 10.0), (200.0, 150.0)])) else {
      panic!("an L is a polyline")
    };
    assert_eq!(points.len(), 3);
  }

  #[test]
  fn recognizes_polygons_started_mid_side() {
    let square = drawn(&[(100.0, 0.0), (200.0, 0.0), (200.0, 200.0), (0.0, 200.0), (0.0, 0.0), (95.0, 0.0)]);
    let Some(Shape::Polygon(corners)) = Shape::recognize(&square) else { panic!("a square") };
    assert_eq!(corners.len(), 4, "{corners:?}");
    let triangle = drawn(&[(0.0, 200.0), (100.0, 0.0), (200.0, 200.0), (5.0, 198.0)]);
    assert!(matches!(Shape::recognize(&triangle), Some(Shape::Polygon(c)) if c.len() == 3));
  }

  #[test]
  fn recognizes_tilted_ellipses_and_rejects_scribbles() {
    let ellipse: Vec<P> = (0..=100)
      .map(|i| {
        let t = i as f32 / 100.0 * TAU;
        let (u, v) = (120.0 * t.cos() * (1.0 + 0.01 * (t * 7.0).sin()), 60.0 * t.sin());
        (300.0 + u * 0.8 - v * 0.6, 200.0 + u * 0.6 + v * 0.8)
      })
      .collect();
    let Some(Shape::Ellipse { center, radii, .. }) = Shape::recognize(&ellipse) else { panic!("an ellipse") };
    assert!(dist(center, (300.0, 200.0)) < 4.0 && (radii.0 - 120.0).abs() < 5.0 && (radii.1 - 60.0).abs() < 5.0, "{center:?} {radii:?}");

    let scribble: Vec<P> = (0..200).map(|i| (i as f32 * 2.0, ((i * 37) % 23) as f32 * 4.0)).collect();
    assert_eq!(Shape::recognize(&scribble), None);
    let outline = Shape::Ellipse { center: (0.0, 0.0), radii: (50.0, 20.0), angle: 0.0 }.outline();
    assert!(dist(outline[0], *outline.last().unwrap()) < 1e-3, "closed");
  }
}
