# ADR 0001: egui for the interface, wgpu for the canvas (not Skia)

- Status: accepted
- Date: 2026-11-18

## Context

rustcreate must feel instant: a painting app stamps thousands of brush dabs per second, blends
many layers every frame and has to keep up with a 200+ Hz pen. Everything must be Rust, and the
app must later run on Windows, macOS, iOS, Android and Linux.

Options considered:

- **Skia via `skia-safe`**: mature, but it links prebuilt C++ (not Rust), makes builds heavy, and
  is optimized for vector and UI drawing rather than raster dab stamping and tiled layers.
  Procreate itself does not use Skia: it has its own engine on Metal.
- **Vello**: pure Rust and very fast, but designed for vector scenes, not raster painting.
- **Tauri + web canvas (Konva)**, like ArtCraft's web app: not Rust, slower input path.
- **wgpu with our own shaders**: pure Rust, maps to DX12/Vulkan/Metal/GLES, full control of the
  brush and compositing pipeline.

## Decision

- The interface (toolbars, panels, sliders, dialogs) is **egui** through **eframe with the wgpu
  backend**, the same stack as archi-games and ArtCraft's native launcher.
- The canvas, brush engine and layer compositor are our own **wgpu** code in `crates/render`,
  sharing the device eframe creates and drawn into the egui frame with a paint callback.
- No Skia, no web view.

## Consequences

- We write the brush and blend shaders ourselves (WGSL). More work up front, but nothing stands
  between the pen and the GPU.
- One GPU device for UI and canvas: no copies between contexts.
- Porting to other platforms is mostly windowing and input; rendering stays the same.
