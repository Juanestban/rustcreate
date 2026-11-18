# Design

Living record of the decisions behind rustcreate. Architecture decisions with lasting
consequences are in [`adr/`](adr/). If something here must change, propose it and update this
file; do not silently diverge.

## Product

- **What:** a native raster painting and animation app inspired by Procreate, plus the Photoshop
  features people miss in it (adjustment layers, masks, layer groups with blend modes, precise
  selections, Photoshop shortcuts).
- **Name:** rustcreate. Never branded as Procreate ([ADR 0006](adr/0006-original-assets-only.md)).
- **Platforms:** Windows 10 1809+ x64 first. Then macOS, iPhone/iPad, Android and Linux.
- **Languages:** interface in English and Spanish (Fluent `.ftl` tokens). Docs, code and
  identifiers in English.
- **License:** MIT OR Apache-2.0.

## Technology

- 100% Rust, edition 2024.
- **egui (eframe, wgpu backend)** for the interface; **wgpu** with our own WGSL shaders for the
  canvas ([ADR 0001](adr/0001-egui-and-wgpu-not-skia.md)). No Skia, no web view.
- GPU: DX12 on Windows, Vulkan as fallback. Without a usable GPU the app shows a clear error and
  exits; there is no software renderer.

## Crates

| Crate | Responsibility | Must not know about |
|---|---|---|
| `rustcreate-core` (`crates/core`) | Document model: tiles, layers, blend modes, history, `.rcreate` read/write | GPU, egui |
| `rustcreate-render` (`crates/render`) | wgpu brush engine, compositor, blend-mode shaders, tile uploads | egui |
| `rustcreate` (`crates/app`) | egui interface, themes, icons, shortcuts, pen input, i18n, updater | — |

## Canvas and history

- Raster layers are 256×256 tiles, RGBA8 premultiplied, sRGB; max canvas 16384×16384
  ([ADR 0005](adr/0005-tiled-rgba8-canvas.md)).
- Undo: tile snapshots, 250 steps, 2 GB default memory cap.

## Brushes

- Our own engine with Procreate-like parameters: shape, grain, spacing, jitter (size, opacity,
  angle), taper, streamline/stabilization, pressure and tilt curves, wet mix later.
- Categories modeled after Procreate's library (Sketching, Inking, Drawing, Painting, Airbrushing,
  Textures…), built with our own textures.
- v0.1 brushes: **Pencil, Ink, Airbrush, Eraser** (the eraser can use any brush).

## Files

- Native format `.rcreate`: zip with `document.json`, a PNG per layer and a thumbnail
  ([ADR 0003](adr/0003-rcreate-file-format.md)).
- Export PNG in v0.1; JPEG, PSD, GIF/MP4 (animation) later.

## Input

- Pen through Windows Ink (`WM_POINTER`), mouse fallback; Wintab later
  ([ADR 0004](adr/0004-pen-input-windows-ink.md)).
- Keyboard shortcuts follow Photoshop ([SHORTCUTS.md](SHORTCUTS.md)).

## Interface

- Layout inspired by Procreate: a thin top bar (gallery, actions, adjustments, selection,
  transform on the left; brush, smudge, eraser, layers and color on the right), vertical size and
  opacity sliders on the left edge, floating panels for brushes, layers and color.
- Dark theme (default) and light theme; follows the system theme unless the user picks one.
- Our own minimalist SVG icon set: thin rounded strokes, dark surfaces, gradient accents.
- App icon: no square, dark background with a soft glow, a brush head without handle leaving a
  colorful cool-to-warm gradient stroke.

## Updates and releases

- Installers and update flow copied from archi-games: per-user MSI and portable `.exe` built by
  `scripts/package.ps1` + WiX in `release.yml`, with `SHA256SUMS.txt`
  ([ADR 0002](adr/0002-updates-from-github-releases.md)).
- The app checks GitHub Releases at startup and every 5 hours and shows a banner; it installs only
  when the user clicks **Update**. No repository secrets anywhere.

## Working with Claude Code

- Context comes from the graphify graph (`graphify-out/`, not in git). A `Stop` hook updates it
  for code; after changing `.md` files run `/graphify . --update`.
