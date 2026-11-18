# ADR 0005: A tiled RGBA8 canvas

- Status: accepted
- Date: 2026-11-18

## Context

Large canvases with many layers must fit in memory, undo cheaply and upload to the GPU only
what changed.

## Decision

- Every raster layer is a grid of **256×256 tiles**, allocated lazily (empty tiles cost nothing).
- Pixels are **RGBA, 8 bits per channel, premultiplied alpha, sRGB**. Procreate also defaults to
  8 bits.
- Maximum canvas: **16384×16384**, further limited by the GPU's texture limits and VRAM.
- **History** stores snapshots of only the tiles a stroke touched: **250 steps** with a
  configurable memory cap (2 GB by default); the oldest steps are dropped first.
- 16-bit channels and color management (Display P3, ICC profiles) come later.

## Consequences

- Undo and save cost is proportional to what changed, not to canvas size.
- 8-bit blending can band in very soft gradients; the 16-bit option will address it.
