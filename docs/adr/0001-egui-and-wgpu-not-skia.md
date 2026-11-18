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
- The layer compositor and canvas view are our own **wgpu** code in `crates/render`, sharing the
  device eframe creates and drawn into the egui frame with a paint callback.
- **Brush dabs are stamped on the CPU**, into the document's tiles in `crates/core` (decided
  2026-11-18). The document stays the single source of truth, so undo and saving need no GPU
  readback; the GPU only uploads changed tiles and composites, as Krita does. If a benchmark shows
  large brushes or wet mixing falling behind, stamping can move to multithreading or the GPU later.
- No Skia, no web view.

## Consequences

- We write the blend shaders ourselves (WGSL); `core::blend` is the CPU reference they must match
  (a GPU test checks it).
- One GPU device for UI and canvas: no copies between contexts.
- Porting to other platforms is mostly windowing and input; rendering stays the same.
