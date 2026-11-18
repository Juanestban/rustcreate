//! Per-pixel compositing of premultiplied RGBA8 (W3C Compositing and Blending, source-over).
//! The GPU compositor in `render` must match these results; this is the reference.

use crate::BlendMode;

/// Composites `src` over `dst` in place. Both are premultiplied RGBA8 of the same length.
pub fn composite(dst: &mut [u8], src: &[u8], opacity: f32, mode: BlendMode) {
  debug_assert_eq!(dst.len(), src.len());
  let opacity = opacity.clamp(0.0, 1.0);
  for (d, s) in dst.as_chunks_mut::<4>().0.iter_mut().zip(src.as_chunks::<4>().0) {
    let sa = s[3] as f32 / 255.0 * opacity;
    if sa == 0.0 {
      continue;
    }
    let da = d[3] as f32 / 255.0;
    let ra = sa + da - sa * da;
    for c in 0..3 {
      // Premultiplied channels, scaled by the layer opacity like alpha.
      let sc = s[c] as f32 / 255.0 * opacity;
      let dc = d[c] as f32 / 255.0;
      // B() works on straight (unpremultiplied) colors.
      let (cs, cd) = (sc / sa, if da > 0.0 { dc / da } else { 0.0 });
      let mixed = (1.0 - sa) * dc + (1.0 - da) * sc + sa * da * blend(mode, cs, cd);
      d[c] = (mixed.clamp(0.0, ra) * 255.0).round() as u8;
    }
    d[3] = (ra * 255.0).round() as u8;
  }
}

/// The blend function B(Cs, Cd) on straight colors in 0..=1.
fn blend(mode: BlendMode, cs: f32, cd: f32) -> f32 {
  match mode {
    BlendMode::Normal => cs,
    BlendMode::Multiply => cs * cd,
    BlendMode::Screen => cs + cd - cs * cd,
    BlendMode::Overlay => hard_light(cd, cs),
    BlendMode::Darken => cs.min(cd),
    BlendMode::Lighten => cs.max(cd),
    BlendMode::Add => (cs + cd).min(1.0),
    BlendMode::ColorDodge => match (cd, cs) {
      (0.0, _) => 0.0,
      (_, 1.0) => 1.0,
      _ => (cd / (1.0 - cs)).min(1.0),
    },
    BlendMode::ColorBurn => match (cd, cs) {
      (1.0, _) => 1.0,
      (_, 0.0) => 0.0,
      _ => 1.0 - ((1.0 - cd) / cs).min(1.0),
    },
  }
}

fn hard_light(cs: f32, cd: f32) -> f32 {
  if cs <= 0.5 {
    cd * 2.0 * cs
  } else {
    let s = 2.0 * cs - 1.0;
    cd + s - cd * s
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  fn over(dst: [u8; 4], src: [u8; 4], opacity: f32, mode: BlendMode) -> [u8; 4] {
    let mut d = dst;
    composite(&mut d, &src, opacity, mode);
    d
  }

  #[test]
  fn normal_is_source_over() {
    assert_eq!(over([0, 0, 255, 255], [255, 0, 0, 255], 1.0, BlendMode::Normal), [255, 0, 0, 255]);
    assert_eq!(over([0, 0, 0, 0], [128, 0, 0, 128], 1.0, BlendMode::Normal), [128, 0, 0, 128], "onto transparency");
    assert_eq!(over([0, 0, 255, 255], [255, 0, 0, 255], 0.5, BlendMode::Normal), [128, 0, 128, 255], "half opacity");
    assert_eq!(over([1, 2, 3, 4], [0, 0, 0, 0], 1.0, BlendMode::Multiply), [1, 2, 3, 4], "transparent source");
  }

  #[test]
  fn modes_on_opaque_pixels() {
    let gray = [128, 128, 128, 255];
    let white = [255, 255, 255, 255];
    assert_eq!(over(white, gray, 1.0, BlendMode::Multiply), gray);
    assert_eq!(over([0, 0, 0, 255], gray, 1.0, BlendMode::Screen), gray);
    assert_eq!(over(gray, white, 1.0, BlendMode::Add), white);
    assert_eq!(over(gray, [0, 0, 0, 255], 1.0, BlendMode::Lighten), gray);
    assert_eq!(over(gray, white, 1.0, BlendMode::Darken), gray);
    // Overlay with a 50 % gray destination leaves the source's look: 2·cs·cd.
    assert_eq!(over([100, 100, 100, 255], [255, 0, 0, 255], 1.0, BlendMode::Overlay), [200, 0, 0, 255]);
    assert_eq!(over(white, [0, 0, 0, 255], 1.0, BlendMode::ColorBurn), white);
    assert_eq!(over([0, 0, 0, 255], white, 1.0, BlendMode::ColorDodge), [0, 0, 0, 255]);
  }
}
