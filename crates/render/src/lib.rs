//! GPU side of the canvas (ADR 0001): uploads changed tiles of each layer, composites the layer
//! stack with the blend-mode shader and draws the result into a view. Knows nothing about egui:
//! the app passes the wgpu device, queue and render pass it gets from eframe.

use std::collections::HashMap;

use rustcreate_core::{BlendMode, Document, Layer, TILE};
use wgpu::util::DeviceExt;

const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

/// How the canvas sits in the view: canvas pixel `p` lands at `m * p + t`, in physical pixels
/// of the drawing area, y down.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Placement {
  pub m: [[f32; 2]; 2],
  pub t: [f32; 2],
  /// Drawing area size in physical pixels.
  pub area: [f32; 2],
}

struct LayerTexture {
  texture: wgpu::Texture,
  view: wgpu::TextureView,
  params: wgpu::Buffer,
  /// The tile buffers last uploaded, to spot tiles replaced by undo.
  uploaded: HashMap<(u32, u32), *const u8>,
}

// The raw pointers are only compared, never dereferenced.
unsafe impl Send for LayerTexture {}
unsafe impl Sync for LayerTexture {}

pub struct CanvasRenderer {
  size: (u32, u32),
  layers: HashMap<u64, LayerTexture>,
  /// Ping-pong targets; `result` indexes the finished composite.
  stack: [(wgpu::Texture, wgpu::TextureView); 2],
  result: usize,
  /// What the last composite was made from: (id, opacity, hidden, blend) bottom to top.
  composed: Vec<(u64, u32, bool, BlendMode)>,
  composite_pipeline: wgpu::RenderPipeline,
  composite_layout: wgpu::BindGroupLayout,
  display_pipeline: wgpu::RenderPipeline,
  display_layout: wgpu::BindGroupLayout,
  display_group: wgpu::BindGroup,
  view_buffer: wgpu::Buffer,
  sampler: wgpu::Sampler,
  srgb_target: bool,
}

impl CanvasRenderer {
  pub fn new(device: &wgpu::Device, target: wgpu::TextureFormat, (width, height): (u32, u32)) -> Self {
    let shader = device.create_shader_module(wgpu::include_wgsl!("canvas.wgsl"));
    let texture_entry = |binding| wgpu::BindGroupLayoutEntry {
      binding,
      visibility: wgpu::ShaderStages::FRAGMENT,
      ty: wgpu::BindingType::Texture {
        sample_type: wgpu::TextureSampleType::Float { filterable: true },
        view_dimension: wgpu::TextureViewDimension::D2,
        multisampled: false,
      },
      count: None,
    };
    let uniform_entry = |binding, visibility| wgpu::BindGroupLayoutEntry {
      binding,
      visibility,
      ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
      count: None,
    };
    let composite_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
      label: Some("composite"),
      entries: &[texture_entry(0), texture_entry(1), uniform_entry(2, wgpu::ShaderStages::FRAGMENT)],
    });
    let display_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
      label: Some("display"),
      entries: &[
        texture_entry(0),
        wgpu::BindGroupLayoutEntry {
          binding: 1,
          visibility: wgpu::ShaderStages::FRAGMENT,
          ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
          count: None,
        },
        uniform_entry(2, wgpu::ShaderStages::VERTEX_FRAGMENT),
      ],
    });
    let pipeline = |layout: &wgpu::BindGroupLayout, vertex: &str, fragment: &str, format, topology| {
      let layout =
        device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: None, bind_group_layouts: &[Some(layout)], immediate_size: 0 });
      device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(fragment),
        layout: Some(&layout),
        vertex: wgpu::VertexState { module: &shader, entry_point: Some(vertex), compilation_options: Default::default(), buffers: &[] },
        fragment: Some(wgpu::FragmentState {
          module: &shader,
          entry_point: Some(fragment),
          compilation_options: Default::default(),
          targets: &[Some(wgpu::ColorTargetState { format, blend: None, write_mask: wgpu::ColorWrites::ALL })],
        }),
        primitive: wgpu::PrimitiveState { topology, ..Default::default() },
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        multiview_mask: None,
        cache: None,
      })
    };
    let composite_pipeline = pipeline(&composite_layout, "fullscreen", "composite", FORMAT, wgpu::PrimitiveTopology::TriangleList);
    let display_pipeline = pipeline(&display_layout, "display_vertex", "display_fragment", target, wgpu::PrimitiveTopology::TriangleStrip);
    let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
      mag_filter: wgpu::FilterMode::Linear,
      min_filter: wgpu::FilterMode::Linear,
      ..Default::default()
    });
    let view_buffer = device.create_buffer(&wgpu::BufferDescriptor {
      label: Some("view"),
      size: 48,
      usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
      mapped_at_creation: false,
    });
    let stack = [stack_texture(device, width, height), stack_texture(device, width, height)];
    let display_group = display_group(device, &display_layout, &stack[0].1, &sampler, &view_buffer);
    Self {
      size: (width, height),
      layers: HashMap::new(),
      stack,
      result: 0,
      composed: Vec::new(),
      composite_pipeline,
      composite_layout,
      display_pipeline,
      display_layout,
      display_group,
      view_buffer,
      sampler,
      srgb_target: target.is_srgb(),
    }
  }

  /// Brings the GPU copy up to date with `doc`: uploads changed tiles and re-composites if
  /// anything changed. Call once per frame before drawing. The document's size must match the
  /// renderer's (make a new renderer for a new document).
  pub fn sync(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, doc: &mut Document) {
    assert_eq!((doc.width, doc.height), self.size, "a CanvasRenderer draws one canvas size");
    let mut changed = false;
    for layer in doc.layers_mut() {
      let gpu = self.layers.entry(layer.id).or_insert_with(|| layer_texture(device, self.size));
      changed |= upload(queue, gpu, layer, self.size);
    }
    let live: Vec<u64> = doc.layers().iter().map(|l| l.id).collect();
    self.layers.retain(|id, _| live.contains(id));

    let composed: Vec<_> = doc.layers().iter().map(|l| (l.id, l.opacity.to_bits(), l.hidden, l.blend)).collect();
    if changed || composed != self.composed {
      self.composite(device, queue, doc.layers());
      self.composed = composed;
    }
  }

  fn composite(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, layers: &[Layer]) {
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("composite") });
    let mut target = 0;
    pass(&mut encoder, &self.stack[target].1, None);
    for layer in layers.iter().filter(|l| !l.hidden) {
      let gpu = &self.layers[&layer.id];
      let mut params = [0u8; 16];
      params[..4].copy_from_slice(&layer.opacity.clamp(0.0, 1.0).to_ne_bytes());
      params[4..8].copy_from_slice(&(layer.blend as u32).to_ne_bytes());
      queue.write_buffer(&gpu.params, 0, &params);
      let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: None,
        layout: &self.composite_layout,
        entries: &[
          wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&self.stack[target].1) },
          wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&gpu.view) },
          wgpu::BindGroupEntry { binding: 2, resource: gpu.params.as_entire_binding() },
        ],
      });
      target = 1 - target;
      pass(&mut encoder, &self.stack[target].1, Some((&self.composite_pipeline, &group)));
    }
    queue.submit([encoder.finish()]);
    if self.result != target {
      self.result = target;
      self.display_group = display_group(device, &self.display_layout, &self.stack[target].1, &self.sampler, &self.view_buffer);
    }
  }

  /// Sets where the canvas is drawn. Call before `paint`, each frame the view changes.
  pub fn place(&self, queue: &wgpu::Queue, placement: Placement) {
    let [w, h] = placement.area;
    // Physical pixels (y down) -> clip space (y up).
    let to_clip = |x: f32, y: f32| [x * 2.0 / w, -y * 2.0 / h];
    let m = [to_clip(placement.m[0][0], placement.m[0][1]), to_clip(placement.m[1][0], placement.m[1][1])];
    let t = {
      let [x, y] = to_clip(placement.t[0], placement.t[1]);
      [x - 1.0, y + 1.0]
    };
    // Zoomed in (a canvas pixel covers a screen pixel or more): show crisp pixels.
    let scale = placement.m[0][0].hypot(placement.m[0][1]);
    let mut bytes = Vec::with_capacity(48);
    for value in [m[0][0], m[0][1], m[1][0], m[1][1], t[0], t[1], self.size.0 as f32, self.size.1 as f32] {
      bytes.extend(value.to_ne_bytes());
    }
    bytes.extend(u32::from(self.srgb_target).to_ne_bytes());
    bytes.extend(u32::from(scale >= 1.0).to_ne_bytes());
    bytes.resize(48, 0);
    queue.write_buffer(&self.view_buffer, 0, &bytes);
  }

  /// Draws the canvas into the current render pass (egui's, through a paint callback).
  pub fn paint(&self, pass: &mut wgpu::RenderPass<'_>) {
    pass.set_pipeline(&self.display_pipeline);
    pass.set_bind_group(0, &self.display_group, &[]);
    pass.draw(0..4, 0..1);
  }

  /// The finished composite, for tests and future readback (export of what is on screen).
  pub fn composite_texture(&self) -> &wgpu::Texture {
    &self.stack[self.result].0
  }
}

fn stack_texture(device: &wgpu::Device, width: u32, height: u32) -> (wgpu::Texture, wgpu::TextureView) {
  let texture = device.create_texture(&wgpu::TextureDescriptor {
    label: Some("stack"),
    size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
    mip_level_count: 1,
    sample_count: 1,
    dimension: wgpu::TextureDimension::D2,
    format: FORMAT,
    usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_SRC,
    view_formats: &[],
  });
  let view = texture.create_view(&Default::default());
  (texture, view)
}

// shortcut: one full-canvas texture per layer (64 MB at 4096², 1 GB at 16384²); move to a sparse
// tile atlas when large canvases with many layers run out of VRAM.
fn layer_texture(device: &wgpu::Device, (width, height): (u32, u32)) -> LayerTexture {
  let texture = device.create_texture(&wgpu::TextureDescriptor {
    label: Some("layer"),
    size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
    mip_level_count: 1,
    sample_count: 1,
    dimension: wgpu::TextureDimension::D2,
    format: FORMAT,
    usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
    view_formats: &[],
  });
  let view = texture.create_view(&Default::default());
  let params = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
    label: Some("layer params"),
    contents: &[0; 16],
    usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
  });
  LayerTexture { texture, view, params, uploaded: HashMap::new() }
}

fn display_group(
  device: &wgpu::Device,
  layout: &wgpu::BindGroupLayout,
  canvas: &wgpu::TextureView,
  sampler: &wgpu::Sampler,
  view: &wgpu::Buffer,
) -> wgpu::BindGroup {
  device.create_bind_group(&wgpu::BindGroupDescriptor {
    label: Some("display"),
    layout,
    entries: &[
      wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(canvas) },
      wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(sampler) },
      wgpu::BindGroupEntry { binding: 2, resource: view.as_entire_binding() },
    ],
  })
}

/// A render pass into `target`: clears it to transparent, then draws a fullscreen triangle if a
/// pipeline is given.
fn pass(encoder: &mut wgpu::CommandEncoder, target: &wgpu::TextureView, draw: Option<(&wgpu::RenderPipeline, &wgpu::BindGroup)>) {
  let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
    label: None,
    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
      view: target,
      depth_slice: None,
      resolve_target: None,
      ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT), store: wgpu::StoreOp::Store },
    })],
    ..Default::default()
  });
  if let Some((pipeline, group)) = draw {
    pass.set_pipeline(pipeline);
    pass.set_bind_group(0, group, &[]);
    pass.draw(0..3, 0..1);
  }
}

/// Uploads the layer's tiles painted in place or replaced since last time, and clears removed
/// ones. Returns whether anything was uploaded.
fn upload(queue: &wgpu::Queue, gpu: &mut LayerTexture, layer: &mut Layer, (width, height): (u32, u32)) -> bool {
  let dirty = layer.take_dirty();
  let mut changed = false;
  let mut write = |gpu: &LayerTexture, (tx, ty): (u32, u32), pixels: &[u8]| {
    let size = wgpu::Extent3d { width: TILE.min(width - tx * TILE), height: TILE.min(height - ty * TILE), depth_or_array_layers: 1 };
    queue.write_texture(
      wgpu::TexelCopyTextureInfo {
        texture: &gpu.texture,
        mip_level: 0,
        origin: wgpu::Origin3d { x: tx * TILE, y: ty * TILE, z: 0 },
        aspect: wgpu::TextureAspect::All,
      },
      pixels,
      wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(TILE * 4), rows_per_image: Some(TILE) },
      size,
    );
    changed = true;
  };
  for coord in layer.tile_coords().collect::<Vec<_>>() {
    let ptr = layer.tile_ptr(coord.0, coord.1);
    if dirty.contains(&coord) || gpu.uploaded.get(&coord).copied() != ptr {
      write(gpu, coord, layer.tile(coord.0, coord.1).expect("listed tile exists"));
      gpu.uploaded.insert(coord, ptr.expect("listed tile exists"));
    }
  }
  let removed: Vec<_> = gpu.uploaded.keys().copied().filter(|&(tx, ty)| layer.tile(tx, ty).is_none()).collect();
  if !removed.is_empty() {
    let clear = vec![0u8; (TILE * TILE * 4) as usize];
    for coord in removed {
      write(gpu, coord, &clear);
      gpu.uploaded.remove(&coord);
    }
  }
  changed
}

#[cfg(test)]
mod tests;
