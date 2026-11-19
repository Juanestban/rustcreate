// Layer compositing and canvas display. Pixels are premultiplied, sRGB-encoded RGBA8, blended
// on the encoded values exactly like `rustcreate_core::blend` (the CPU reference).

struct FullscreenOut {
  @builtin(position) position: vec4<f32>,
}

// One triangle that covers the whole target.
@vertex
fn fullscreen(@builtin(vertex_index) index: u32) -> FullscreenOut {
  let uv = vec2<f32>(f32((index << 1u) & 2u), f32(index & 2u));
  return FullscreenOut(vec4<f32>(uv * 2.0 - 1.0, 0.0, 1.0));
}

// ---- Compositing: `below` is the stack so far, `layer` goes on top of it. ----

struct LayerParams {
  opacity: f32,
  mode: u32,
  // 1 = clipping mask: the layer only shows where `base` has paint.
  clip: u32,
  // 1 = the layer has a mask: gray shows, black hides, transparent counts as white.
  has_mask: u32,
}

@group(0) @binding(0) var below: texture_2d<f32>;
@group(0) @binding(1) var layer: texture_2d<f32>;
@group(0) @binding(2) var<uniform> params: LayerParams;
@group(0) @binding(3) var base: texture_2d<f32>;
@group(0) @binding(4) var mask: texture_2d<f32>;

// Written on whole RGB vectors with `select`, no loops or per-channel indexing: Windows'
// default shader compiler (FXC, used when DXC is not shipped) can take minutes on a loop that
// calls a `switch`, and the window only shows once this shader is built.

fn hard_light(cs: vec3<f32>, cd: vec3<f32>) -> vec3<f32> {
  let s = 2.0 * cs - 1.0;
  return select(cd + s - cd * s, cd * 2.0 * cs, cs <= vec3<f32>(0.5));
}

// Color dodge: 0 where the backdrop is black, else 1 where the source is white.
fn dodge(cs: vec3<f32>, cd: vec3<f32>) -> vec3<f32> {
  let one = vec3<f32>(1.0);
  let r = min(cd / max(one - cs, vec3<f32>(1e-6)), one);
  return select(select(r, one, cs >= one), vec3<f32>(0.0), cd <= vec3<f32>(0.0));
}

// Color burn: 1 where the backdrop is white, else 0 where the source is black.
fn burn(cs: vec3<f32>, cd: vec3<f32>) -> vec3<f32> {
  let one = vec3<f32>(1.0);
  let r = one - min((one - cd) / max(cs, vec3<f32>(1e-6)), one);
  return select(select(r, vec3<f32>(0.0), cs <= vec3<f32>(0.0)), one, cd >= one);
}

fn lum(c: vec3<f32>) -> f32 {
  return dot(c, vec3<f32>(0.3, 0.59, 0.11));
}

fn sat(c: vec3<f32>) -> f32 {
  return max(max(c.r, c.g), c.b) - min(min(c.r, c.g), c.b);
}

// The color with luminosity `l`, pulled back into gamut keeping its luminosity.
fn set_lum(color: vec3<f32>, l: f32) -> vec3<f32> {
  var c = color + (l - lum(color));
  let m = lum(c);
  let n = min(min(c.r, c.g), c.b);
  let x = max(max(c.r, c.g), c.b);
  if n < 0.0 {
    c = m + (c - m) * m / (m - n);
  }
  if x > 1.0 {
    c = m + (c - m) * (1.0 - m) / (x - m);
  }
  return c;
}

// The color stretched so its saturation (max − min) is `s`, with its minimum at 0.
fn set_sat(c: vec3<f32>, s: f32) -> vec3<f32> {
  let n = min(min(c.r, c.g), c.b);
  let x = max(max(c.r, c.g), c.b);
  return select(vec3<f32>(0.0), (c - n) * s / max(x - n, 1e-6), x > n);
}

// Same order as `BlendMode`.
fn blend(mode: u32, cs: vec3<f32>, cd: vec3<f32>) -> vec3<f32> {
  let one = vec3<f32>(1.0);
  let zero = vec3<f32>(0.0);
  let half = vec3<f32>(0.5);
  // One return after the switch: FXC (DX12 without DXC) is strict about functions whose
  // returns are all inside a `switch`.
  var out = cs;
  switch mode {
    case 1u: { out = cs * cd; }
    case 2u: { out = cs + cd - cs * cd; }
    case 3u: { out = hard_light(cd, cs); }
    case 4u: { out = min(cs, cd); }
    case 5u: { out = max(cs, cd); }
    case 6u: { out = min(cs + cd, one); }
    case 7u: { out = dodge(cs, cd); }
    case 8u: { out = burn(cs, cd); }
    case 9u: { out = max(cs + cd - one, zero); }
    case 10u: { out = max(cd - cs, zero); }
    case 11u: {
      // Divide: 0 where the backdrop is black, else 1 where the source is black.
      let r = min(cd / max(cs, vec3<f32>(1e-6)), one);
      out = select(select(r, one, cs <= zero), zero, cd <= zero);
    }
    case 12u: {
      let d = select(sqrt(cd), ((16.0 * cd - 12.0) * cd + 4.0) * cd, cd <= vec3<f32>(0.25));
      out = select(cd + (2.0 * cs - 1.0) * (d - cd), cd - (1.0 - 2.0 * cs) * cd * (1.0 - cd), cs <= half);
    }
    case 13u: { out = hard_light(cs, cd); }
    case 14u: { out = select(dodge(2.0 * cs - 1.0, cd), burn(2.0 * cs, cd), cs <= half); }
    case 15u: { out = clamp(cd + 2.0 * cs - 1.0, zero, one); }
    case 16u: { out = select(max(cd, 2.0 * cs - 1.0), min(cd, 2.0 * cs), cs <= half); }
    case 17u: { out = abs(cs - cd); }
    case 18u: { out = cs + cd - 2.0 * cs * cd; }
    case 19u: { out = set_lum(set_sat(cs, sat(cd)), lum(cd)); }
    case 20u: { out = set_lum(set_sat(cd, sat(cs)), lum(cd)); }
    case 21u: { out = set_lum(cs, lum(cd)); }
    case 22u: { out = set_lum(cd, lum(cs)); }
    default: {}
  }
  return out;
}

@fragment
fn composite(in: FullscreenOut) -> @location(0) vec4<f32> {
  let p = vec2<i32>(in.position.xy);
  let d = textureLoad(below, p, 0);
  var s = textureLoad(layer, p, 0) * params.opacity;
  if params.has_mask == 1u {
    let m = textureLoad(mask, p, 0);
    s *= min(dot(m.rgb, vec3<f32>(54.0, 183.0, 19.0) / 256.0) + 1.0 - m.a, 1.0);
  }
  if params.clip == 1u {
    s *= textureLoad(base, p, 0).a;
  }
  let sa = s.a;
  if sa == 0.0 {
    return d;
  }
  let da = d.a;
  let ra = sa + da - sa * da;
  // Clamped like the CPU reference: rounding can push a straight color just past 1.
  let cs = min(s.rgb / sa, vec3<f32>(1.0));
  let cd = select(vec3<f32>(0.0), min(d.rgb / max(da, 1e-6), vec3<f32>(1.0)), da > 0.0);
  let rgb = (1.0 - sa) * d.rgb + (1.0 - da) * s.rgb + sa * da * blend(params.mode, cs, cd);
  return vec4<f32>(clamp(rgb, vec3<f32>(0.0), vec3<f32>(ra)), ra);
}

// ---- Display: the composited canvas transformed into the view, over a checkerboard. ----

struct View {
  // Canvas pixel -> clip space: clip = m * (x, y) + t.
  m: mat2x2<f32>,
  t: vec2<f32>,
  size: vec2<f32>,
  // Output is decoded to linear when the target format is sRGB.
  srgb_target: u32,
  // 1 = nearest (zoomed in), 0 = linear.
  nearest: u32,
}

@group(0) @binding(0) var canvas: texture_2d<f32>;
@group(0) @binding(1) var canvas_sampler: sampler;
@group(0) @binding(2) var<uniform> view: View;

struct DisplayOut {
  @builtin(position) position: vec4<f32>,
  @location(0) pixel: vec2<f32>,
}

@vertex
fn display_vertex(@builtin(vertex_index) index: u32) -> DisplayOut {
  let corner = vec2<f32>(f32(index & 1u), f32((index >> 1u) & 1u));
  let pixel = corner * view.size;
  return DisplayOut(vec4<f32>(view.m * pixel + view.t, 0.0, 1.0), pixel);
}

fn to_linear(c: vec3<f32>) -> vec3<f32> {
  return select(pow((c + 0.055) / 1.055, vec3<f32>(2.4)), c / 12.92, c <= vec3<f32>(0.04045));
}

@fragment
fn display_fragment(in: DisplayOut) -> @location(0) vec4<f32> {
  var color: vec4<f32>;
  if view.nearest == 1u {
    color = textureLoad(canvas, vec2<i32>(clamp(in.pixel, vec2<f32>(0.0), view.size - 1.0)), 0);
  } else {
    color = textureSampleLevel(canvas, canvas_sampler, in.pixel / view.size, 0.0);
  }
  // 16-pixel checkerboard in canvas space shows transparency.
  let cell = vec2<i32>(floor(in.pixel / 16.0));
  let checker = select(1.0, 0.85, ((cell.x + cell.y) & 1) == 1);
  var rgb = color.rgb + vec3<f32>(checker) * (1.0 - color.a);
  if view.srgb_target == 1u {
    rgb = to_linear(rgb);
  }
  return vec4<f32>(rgb, 1.0);
}
