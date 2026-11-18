//! Symmetry for the drawing guide: a stroke can paint mirrored or rotated copies of every dab.

use std::f32::consts::TAU;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Symmetry {
  /// Mirrored left and right of a vertical axis.
  #[default]
  Vertical,
  /// Mirrored above and below a horizontal axis.
  Horizontal,
  /// Both axes: four copies.
  Quadrant,
  /// `n` copies rotated around the center.
  Radial(u8),
}

/// A linear map around a center, applied to every dab of a symmetric stroke.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Mirror {
  m: [f32; 4],
  center: (f32, f32),
}

impl Mirror {
  pub(crate) fn point(&self, (x, y): (f32, f32)) -> (f32, f32) {
    let ([a, b, c, d], (cx, cy)) = (self.m, self.center);
    let (dx, dy) = (x - cx, y - cy);
    (cx + a * dx + b * dy, cy + c * dx + d * dy)
  }

  /// A direction in radians, mapped (tips that follow the stroke turn with their copy).
  pub(crate) fn angle(&self, angle: f32) -> f32 {
    let [a, b, c, d] = self.m;
    let (sin, cos) = angle.sin_cos();
    (c * cos + d * sin).atan2(a * cos + b * sin)
  }
}

impl Symmetry {
  /// The copies besides the original, around `center` (canvas pixels).
  pub(crate) fn mirrors(self, center: (f32, f32)) -> Vec<Mirror> {
    let mirror = |m| Mirror { m, center };
    match self {
      Symmetry::Vertical => vec![mirror([-1.0, 0.0, 0.0, 1.0])],
      Symmetry::Horizontal => vec![mirror([1.0, 0.0, 0.0, -1.0])],
      Symmetry::Quadrant => vec![mirror([-1.0, 0.0, 0.0, 1.0]), mirror([1.0, 0.0, 0.0, -1.0]), mirror([-1.0, 0.0, 0.0, -1.0])],
      Symmetry::Radial(n) => (1..n.max(1))
        .map(|k| {
          let (sin, cos) = (k as f32 / n as f32 * TAU).sin_cos();
          mirror([cos, -sin, sin, cos])
        })
        .collect(),
    }
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn mirrors_map_points_and_angles() {
    let [vertical] = Symmetry::Vertical.mirrors((100.0, 50.0))[..] else { panic!("one copy") };
    assert_eq!(vertical.point((90.0, 10.0)), (110.0, 10.0));
    assert!((vertical.angle(0.0) - std::f32::consts::PI).abs() < 1e-5, "heading right mirrors to heading left");
    let radial = Symmetry::Radial(4).mirrors((0.0, 0.0));
    assert_eq!(radial.len(), 3);
    let (x, y) = radial[0].point((10.0, 0.0));
    assert!(x.abs() < 1e-4 && (y - 10.0).abs() < 1e-4, "a quarter turn");
    assert_eq!(Symmetry::Quadrant.mirrors((0.0, 0.0)).len(), 3);
  }
}
