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
}

@group(0) @binding(0) var below: texture_2d<f32>;
@group(0) @binding(1) var layer: texture_2d<f32>;
@group(0) @binding(2) var<uniform> params: LayerParams;

fn hard_light(cs: f32, cd: f32) -> f32 {
  if cs <= 0.5 {
    return cd * 2.0 * cs;
  }
  let s = 2.0 * cs - 1.0;
  return cd + s - cd * s;
}

// Same order as `BlendMode`.
fn blend(mode: u32, cs: f32, cd: f32) -> f32 {
  switch mode {
    case 1u: { return cs * cd; }
    case 2u: { return cs + cd - cs * cd; }
    case 3u: { return hard_light(cd, cs); }
    case 4u: { return min(cs, cd); }
    case 5u: { return max(cs, cd); }
    case 6u: { return min(cs + cd, 1.0); }
    case 7u: {
      if cd <= 0.0 { return 0.0; }
      if cs >= 1.0 { return 1.0; }
      return min(cd / (1.0 - cs), 1.0);
    }
    case 8u: {
      if cd >= 1.0 { return 1.0; }
      if cs <= 0.0 { return 0.0; }
      return 1.0 - min((1.0 - cd) / cs, 1.0);
    }
    default: { return cs; }
  }
}

@fragment
fn composite(in: FullscreenOut) -> @location(0) vec4<f32> {
  let p = vec2<i32>(in.position.xy);
  let d = textureLoad(below, p, 0);
  let s = textureLoad(layer, p, 0) * params.opacity;
  let sa = s.a;
  if sa == 0.0 {
    return d;
  }
  let da = d.a;
  let ra = sa + da - sa * da;
  var out = vec4<f32>(0.0, 0.0, 0.0, ra);
  for (var c = 0; c < 3; c++) {
    // Clamped like the CPU reference: rounding can push a straight color just past 1.
    let cs = min(s[c] / sa, 1.0);
    var cd = 0.0;
    if da > 0.0 {
      cd = min(d[c] / da, 1.0);
    }
    out[c] = clamp((1.0 - sa) * d[c] + (1.0 - da) * s[c] + sa * da * blend(params.mode, cs, cd), 0.0, ra);
  }
  return out;
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
