use super::*;

/// A headless device, or `None` on machines without a usable GPU (the test then passes
/// vacuously and says so).
fn device() -> Option<(wgpu::Device, wgpu::Queue)> {
  let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
  let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default())).ok()?;
  pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())).ok()
}

/// The composite texture's pixels, row-major RGBA8. `width * 4` must be a multiple of 256.
fn read_back(device: &wgpu::Device, queue: &wgpu::Queue, texture: &wgpu::Texture, (width, height): (u32, u32)) -> Vec<u8> {
  let buffer = device.create_buffer(&wgpu::BufferDescriptor {
    label: None,
    size: (width * height * 4) as u64,
    usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
    mapped_at_creation: false,
  });
  let mut encoder = device.create_command_encoder(&Default::default());
  encoder.copy_texture_to_buffer(
    texture.as_image_copy(),
    wgpu::TexelCopyBufferInfo {
      buffer: &buffer,
      layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(width * 4), rows_per_image: Some(height) },
    },
    wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
  );
  queue.submit([encoder.finish()]);
  buffer.slice(..).map_async(wgpu::MapMode::Read, |result| result.expect("map the readback buffer"));
  device.poll(wgpu::PollType::wait_indefinitely()).expect("GPU finished");
  buffer.slice(..).get_mapped_range().expect("mapped").to_vec()
}

/// Every blend mode on the GPU matches the CPU reference (`core::blend`) within rounding.
#[test]
fn gpu_composite_matches_the_cpu_reference() {
  let Some((device, queue)) = device() else {
    eprintln!("no GPU adapter: skipping");
    return;
  };
  // 64 px wide: one 256-byte row, as copies require. One row per mode.
  let modes = BlendMode::ALL;
  let (width, height) = (64, modes.len() as u32);
  let mut doc = Document::new(width, height);
  // Bottom: a gradient of colors and alphas, one row per mode below.
  let bottom = doc.layer_mut(0).unwrap().tile_mut(0, 0);
  for y in 0..height as usize {
    for x in 0..width as usize {
      let a = (x * 4) as u8 | 3;
      let p = (y * TILE as usize + x) * 4;
      bottom[p..p + 4].copy_from_slice(&[a / 2, a / 3, a, a]);
    }
  }
  for (row, &mode) in modes.iter().enumerate() {
    doc.add_layer();
    let top = doc.layer_mut(row + 1).unwrap();
    top.blend = mode;
    top.opacity = 0.8;
    let tile = top.tile_mut(0, 0);
    for x in 0..width as usize {
      let a = 255 - (x * 4) as u8;
      let p = (row * TILE as usize + x) * 4;
      tile[p..p + 4].copy_from_slice(&[a, a / 2, a / 4, a]);
    }
  }

  let mut renderer = CanvasRenderer::new(&device, wgpu::TextureFormat::Bgra8Unorm, (width, height));
  renderer.sync(&device, &queue, &mut doc);
  let gpu = read_back(&device, &queue, renderer.composite_texture(), (width, height));
  let cpu = doc.composite_tile(0, 0).unwrap();
  for y in 0..height as usize {
    for x in 0..width as usize {
      let g = &gpu[(y * width as usize + x) * 4..][..4];
      let c = &cpu[(y * TILE as usize + x) * 4..][..4];
      let close = g.iter().zip(c).all(|(g, c)| g.abs_diff(*c) <= 2);
      assert!(close, "{:?} at ({x}, {y}): GPU {g:?}, CPU {c:?}", modes[y]);
    }
  }

  // A clipping mask: the last layer only shows over the one below it.
  doc.layer_mut(modes.len()).unwrap().clip = true;
  renderer.sync(&device, &queue, &mut doc);
  let gpu = read_back(&device, &queue, renderer.composite_texture(), (width, height));
  let cpu = doc.composite_tile(0, 0).unwrap();
  assert!(
    gpu.iter().zip(cpu.chunks(TILE as usize * 4).flat_map(|row| &row[..width as usize * 4])).all(|(g, c)| g.abs_diff(*c) <= 2),
    "clipping matches"
  );
  doc.layer_mut(modes.len()).unwrap().clip = false;

  // A layer mask: half of the bottom layer hidden with black, half shown with mid gray.
  assert!(doc.add_mask(0));
  let mask = doc.layer_mut(0).unwrap().mask.as_mut().unwrap().tile_mut(0, 0);
  for x in 0..width as usize {
    let v = if x < 32 { 0 } else { 128 };
    for y in 0..height as usize {
      mask[(y * TILE as usize + x) * 4..][..4].copy_from_slice(&[v, v, v, 255]);
    }
  }
  renderer.sync(&device, &queue, &mut doc);
  let gpu = read_back(&device, &queue, renderer.composite_texture(), (width, height));
  let cpu = doc.composite_tile(0, 0).unwrap();
  assert!(gpu.iter().zip(cpu.chunks(TILE as usize * 4).flat_map(|row| &row[..width as usize * 4])).all(|(g, c)| g.abs_diff(*c) <= 2), "masks match");
  assert!(doc.remove_mask(0, false));

  // Hiding a layer re-composites; painting re-uploads only what changed.
  doc.layer_mut(1).unwrap().hidden = true;
  doc.layer_mut(2).unwrap().tile_mut(0, 0)[TILE as usize * 4..][..4].copy_from_slice(&[0, 0, 0, 0]);
  renderer.sync(&device, &queue, &mut doc);
  let gpu = read_back(&device, &queue, renderer.composite_texture(), (width, height));
  let cpu = doc.composite_tile(0, 0).unwrap();
  assert!(gpu.iter().zip(cpu.chunks(TILE as usize * 4).flat_map(|row| &row[..width as usize * 4])).all(|(g, c)| g.abs_diff(*c) <= 2));
}

#[test]
fn gpu_groups_match_the_cpu_reference() {
  let Some((device, queue)) = device() else {
    eprintln!("no GPU adapter: skipping");
    return;
  };
  let (width, height) = (64, 4);
  let mut doc = Document::new(width, height);
  let paint = |doc: &mut Document, index: usize, color: [u8; 3]| {
    let tile = doc.layer_mut(index).unwrap().tile_mut(0, 0);
    for y in 0..height as usize {
      for x in 0..width as usize {
        let a = (x * 4) as u8 | 3;
        let p = (y * TILE as usize + x) * 4;
        let c = color.map(|c| (c as u32 * a as u32 / 255) as u8);
        tile[p..p + 4].copy_from_slice(&[c[0], c[1], c[2], a]);
      }
    }
  };
  paint(&mut doc, 0, [250, 240, 200]);
  // A group holding a Multiply layer, with a group nested in it; then a layer clipped to the group.
  doc.add_layer();
  paint(&mut doc, 1, [40, 120, 220]);
  doc.layer_mut(1).unwrap().blend = BlendMode::Multiply;
  doc.add_layer();
  paint(&mut doc, 2, [220, 60, 40]);
  doc.group_layer(2); // inner group at 3
  doc.select(1);
  doc.group_layer(1); // outer group holds layer 1 only, at 2
  let outer = doc.layers()[2].id;
  doc.layer_mut(2).unwrap().opacity = 0.7;
  doc.layer_mut(2).unwrap().blend = BlendMode::Screen;
  doc.select(4);
  doc.layer_mut(4).unwrap().opacity = 0.6;
  doc.add_layer();
  paint(&mut doc, 5, [10, 200, 90]);
  doc.layer_mut(5).unwrap().clip = true;
  assert!(doc.layers().iter().any(|l| l.parent() == Some(outer)));
  assert!(doc.layers().iter().filter(|l| l.is_group()).count() == 2);

  let mut renderer = CanvasRenderer::new(&device, wgpu::TextureFormat::Bgra8Unorm, (width, height));
  renderer.sync(&device, &queue, &mut doc);
  let gpu = read_back(&device, &queue, renderer.composite_texture(), (width, height));
  let cpu = doc.composite_tile(0, 0).unwrap();
  for y in 0..height as usize {
    for x in 0..width as usize {
      let g = &gpu[(y * width as usize + x) * 4..][..4];
      let c = &cpu[(y * TILE as usize + x) * 4..][..4];
      assert!(g.iter().zip(c).all(|(g, c)| g.abs_diff(*c) <= 2), "({x}, {y}): gpu {g:?} cpu {c:?}");
    }
  }
}

#[test]
fn floating_transform_matches_the_cpu_result() {
  let Some((device, queue)) = device() else {
    eprintln!("no GPU adapter: skipping");
    return;
  };
  let (width, height) = (64, 48);
  let mut doc = Document::new(width, height);
  // A gradient square on the layer, and paint outside it that stays put.
  let tile = doc.layer_mut(0).unwrap().tile_mut(0, 0);
  for y in 4..20usize {
    for x in 6..30usize {
      let p = (y * TILE as usize + x) * 4;
      let a = (40 + x * 7) as u8;
      tile[p..p + 4].copy_from_slice(&[a / 2, (y * 9) as u8 % a, a, a]);
    }
  }
  doc.select_shape(&rustcreate_core::SelectShape::Rectangle((6.0, 4.0), (20.0, 20.0)), rustcreate_core::SelectMode::Replace);
  let mut transform = doc.begin_transform().unwrap();
  // Turned, scaled and moved: [a, b, c, d, tx, ty].
  let (sin, cos) = 0.4f32.sin_cos();
  let m = [1.3 * cos, -1.3 * sin, 1.3 * sin, 1.3 * cos, 14.0, 6.0];

  let mut renderer = CanvasRenderer::new(&device, wgpu::TextureFormat::Bgra8Unorm, (width, height));
  let (origin, size) = transform.area();
  renderer.begin_floating(&device, &queue, transform.layer_id(), &transform.pixels(), origin, size);
  renderer.move_floating(&queue, m);
  renderer.sync(&device, &queue, &mut doc);
  let gpu = read_back(&device, &queue, renderer.composite_texture(), (width, height));

  transform.update(&mut doc, m);
  let cpu = doc.composite_tile(0, 0).unwrap();
  for y in 0..height as usize {
    for x in 0..width as usize {
      let g = &gpu[(y * width as usize + x) * 4..][..4];
      let c = &cpu[(y * TILE as usize + x) * 4..][..4];
      assert!(g.iter().zip(c).all(|(g, c)| g.abs_diff(*c) <= 2), "({x}, {y}): gpu {g:?} cpu {c:?}");
    }
  }
}
