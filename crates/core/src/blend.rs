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
    // Premultiplied channels, scaled by the layer opacity like alpha.
    let sc = [0, 1, 2].map(|c| s[c] as f32 / 255.0 * opacity);
    let dc = [0, 1, 2].map(|c| d[c] as f32 / 255.0);
    // B() works on straight (unpremultiplied) colors.
    // Clamped: rounding can push a straight color just past 1, which breaks dodge and burn.
    let cs = sc.map(|v| (v / sa).min(1.0));
    let cd = dc.map(|v| if da > 0.0 { (v / da).min(1.0) } else { 0.0 });
    let b = blend(mode, cs, cd);
    for c in 0..3 {
      let mixed = (1.0 - sa) * dc[c] + (1.0 - da) * sc[c] + sa * da * b[c];
      d[c] = (mixed.clamp(0.0, ra) * 255.0).round() as u8;
    }
    d[3] = (ra * 255.0).round() as u8;
  }
}

/// A layer's tile with its mask applied (see [`crate::Layer::mask`]): every channel scaled by
/// the mask's gray, where transparent counts as white.
pub(crate) fn masked<'a>(tile: &'a [u8], mask: Option<&crate::Layer>, tx: u32, ty: u32) -> std::borrow::Cow<'a, [u8]> {
  let Some(m) = mask.and_then(|m| m.tile(tx, ty)) else { return std::borrow::Cow::Borrowed(tile) };
  let show = |p: &[u8; 4]| (p[0] as u32 * 54 + p[1] as u32 * 183 + p[2] as u32 * 19) / 256 + 255 - p[3] as u32;
  std::borrow::Cow::Owned(
    tile.as_chunks::<4>().0.iter().zip(m.as_chunks::<4>().0).flat_map(|(p, m)| p.map(|v| (v as u32 * show(m).min(255) / 255) as u8)).collect(),
  )
}

/// A clipped layer's tile limited to where `base` has paint: every channel scaled by the base's
/// alpha (both premultiplied).
pub(crate) fn clip(tile: &[u8], base: &[u8]) -> Vec<u8> {
  tile.as_chunks::<4>().0.iter().zip(base.as_chunks::<4>().0).flat_map(|(p, b)| p.map(|v| (v as u32 * b[3] as u32 / 255) as u8)).collect()
}

/// The blend function B(Cs, Cd) on straight RGB in 0..=1 (W3C Compositing and Blending, plus
/// Photoshop's Linear Burn, Subtract, Divide, Vivid, Linear and Pin Light).
fn blend(mode: BlendMode, cs: [f32; 3], cd: [f32; 3]) -> [f32; 3] {
  match mode {
    BlendMode::Hue => set_lum(set_sat(cs, sat(cd)), lum(cd)),
    BlendMode::Saturation => set_lum(set_sat(cd, sat(cs)), lum(cd)),
    BlendMode::Color => set_lum(cs, lum(cd)),
    BlendMode::Luminosity => set_lum(cd, lum(cs)),
    _ => [0, 1, 2].map(|c| separable(mode, cs[c], cd[c])),
  }
}

/// The modes that treat each channel on its own.
fn separable(mode: BlendMode, cs: f32, cd: f32) -> f32 {
  match mode {
    BlendMode::Normal => cs,
    BlendMode::Multiply => cs * cd,
    BlendMode::Screen => cs + cd - cs * cd,
    BlendMode::Overlay => hard_light(cd, cs),
    BlendMode::Darken => cs.min(cd),
    BlendMode::Lighten => cs.max(cd),
    BlendMode::Add => (cs + cd).min(1.0),
    BlendMode::ColorDodge => dodge(cs, cd),
    BlendMode::ColorBurn => burn(cs, cd),
    BlendMode::LinearBurn => (cs + cd - 1.0).max(0.0),
    BlendMode::Subtract => (cd - cs).max(0.0),
    BlendMode::Divide if cd <= 0.0 => 0.0,
    BlendMode::Divide if cs <= 0.0 => 1.0,
    BlendMode::Divide => (cd / cs).min(1.0),
    BlendMode::SoftLight if cs <= 0.5 => cd - (1.0 - 2.0 * cs) * cd * (1.0 - cd),
    BlendMode::SoftLight => {
      let d = if cd <= 0.25 { ((16.0 * cd - 12.0) * cd + 4.0) * cd } else { cd.sqrt() };
      cd + (2.0 * cs - 1.0) * (d - cd)
    }
    BlendMode::HardLight => hard_light(cs, cd),
    BlendMode::VividLight if cs <= 0.5 => burn(2.0 * cs, cd),
    BlendMode::VividLight => dodge(2.0 * cs - 1.0, cd),
    BlendMode::LinearLight => (cd + 2.0 * cs - 1.0).clamp(0.0, 1.0),
    BlendMode::PinLight if cs <= 0.5 => cd.min(2.0 * cs),
    BlendMode::PinLight => cd.max(2.0 * cs - 1.0),
    BlendMode::Difference => (cs - cd).abs(),
    BlendMode::Exclusion => cs + cd - 2.0 * cs * cd,
    BlendMode::Hue | BlendMode::Saturation | BlendMode::Color | BlendMode::Luminosity => unreachable!("not separable"),
  }
}

fn dodge(cs: f32, cd: f32) -> f32 {
  if cd <= 0.0 {
    0.0
  } else if cs >= 1.0 {
    1.0
  } else {
    (cd / (1.0 - cs)).min(1.0)
  }
}

fn burn(cs: f32, cd: f32) -> f32 {
  if cd >= 1.0 {
    1.0
  } else if cs <= 0.0 {
    0.0
  } else {
    1.0 - ((1.0 - cd) / cs).min(1.0)
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

fn lum(c: [f32; 3]) -> f32 {
  0.3 * c[0] + 0.59 * c[1] + 0.11 * c[2]
}

fn sat(c: [f32; 3]) -> f32 {
  c[0].max(c[1]).max(c[2]) - c[0].min(c[1]).min(c[2])
}

/// The color with luminosity `l`, pulled back into gamut keeping its luminosity.
fn set_lum(c: [f32; 3], l: f32) -> [f32; 3] {
  let d = l - lum(c);
  let c = c.map(|v| v + d);
  let l = lum(c);
  let (n, x) = (c[0].min(c[1]).min(c[2]), c[0].max(c[1]).max(c[2]));
  let c = if n < 0.0 { c.map(|v| l + (v - l) * l / (l - n)) } else { c };
  if x > 1.0 { c.map(|v| l + (v - l) * (1.0 - l) / (x - l)) } else { c }
}

/// The color stretched so its saturation (max − min) is `s`, with its minimum at 0.
fn set_sat(c: [f32; 3], s: f32) -> [f32; 3] {
  let (n, x) = (c[0].min(c[1]).min(c[2]), c[0].max(c[1]).max(c[2]));
  if x > n { c.map(|v| (v - n) * s / (x - n)) } else { [0.0; 3] }
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

  #[test]
  fn the_photoshop_modes() {
    let (black, white, gray) = ([0, 0, 0, 255], [255, 255, 255, 255], [128, 128, 128, 255]);
    let red = [255, 0, 0, 255];
    assert_eq!(over(gray, white, 1.0, BlendMode::Difference), [127, 127, 127, 255]);
    assert_eq!(over(white, white, 1.0, BlendMode::Exclusion), black);
    assert_eq!(over(gray, gray, 1.0, BlendMode::Subtract), black);
    assert_eq!(over(gray, white, 1.0, BlendMode::Divide), gray, "dividing by white changes nothing");
    assert_eq!(over(gray, black, 1.0, BlendMode::LinearBurn), black);
    assert_eq!(over(gray, gray, 1.0, BlendMode::SoftLight), gray, "50 % gray is neutral in the light modes");
    assert_eq!(over(gray, gray, 1.0, BlendMode::LinearLight), [129, 129, 129, 255]);
    assert_eq!(over([40, 90, 200, 255], gray, 1.0, BlendMode::PinLight), [40, 90, 200, 255]);
    assert_eq!(over(white, black, 1.0, BlendMode::HardLight), black);
    assert_eq!(over(gray, white, 1.0, BlendMode::VividLight), white);
    // Color keeps the backdrop's luminosity: red over gray is a gray-bright red.
    let tinted = over(gray, red, 1.0, BlendMode::Color);
    assert!(tinted[0] > tinted[1] && tinted[1] == tinted[2], "{tinted:?}");
    assert_eq!(over(red, gray, 1.0, BlendMode::Saturation), [77, 77, 77, 255], "no saturation: a gray of red's luminosity (0.3)");
    assert_eq!(over(gray, red, 1.0, BlendMode::Luminosity)[3], 255);
    let hue = over([0, 0, 255, 255], [0, 255, 0, 255], 1.0, BlendMode::Hue);
    assert!(hue[1] > hue[0] && hue[1] > hue[2], "takes green's hue: {hue:?}");
  }
}
