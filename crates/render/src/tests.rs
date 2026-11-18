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
  // 64 px wide: one 256-byte row, as copies require.
  let (width, height) = (64, 9);
  let modes = [
    BlendMode::Normal,
    BlendMode::Multiply,
    BlendMode::Screen,
    BlendMode::Overlay,
    BlendMode::Darken,
    BlendMode::Lighten,
    BlendMode::Add,
    BlendMode::ColorDodge,
    BlendMode::ColorBurn,
  ];
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

  // Hiding a layer re-composites; painting re-uploads only what changed.
  doc.layer_mut(1).unwrap().hidden = true;
  doc.layer_mut(2).unwrap().tile_mut(0, 0)[TILE as usize * 4..][..4].copy_from_slice(&[0, 0, 0, 0]);
  renderer.sync(&device, &queue, &mut doc);
  let gpu = read_back(&device, &queue, renderer.composite_texture(), (width, height));
  let cpu = doc.composite_tile(0, 0).unwrap();
  assert!(gpu.iter().zip(cpu.chunks(TILE as usize * 4).flat_map(|row| &row[..width as usize * 4])).all(|(g, c)| g.abs_diff(*c) <= 2));
}
