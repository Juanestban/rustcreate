# ADR 0003: The `.rcreate` file format

- Status: accepted
- Date: 2026-11-18

## Context

Documents need to store layers losslessly with their properties, open fast, be easy to inspect
and grow later to animation frames, masks and adjustment layers. Optional per-layer fields
(`alpha_lock`, `clip`, `mask`) default to `false` when missing, so they did not bump FORMAT. A
layer with `mask: true` has its mask in `masks/{id}.png` (gray; transparent counts as white).
A group has `group: true` and no `layers/{id}.png`; a layer in a group names it in `parent`.
Children sit right below their group in the list; files where they do not are rejected.

## Decision

A `.rcreate` file is a **zip archive**:

```
document.json        format version, canvas size, layer tree and layer properties
                     (name, opacity, blend mode, hidden, locked…), the drawing guide,
                     later animation frames
layers/<id>.png      one RGBA PNG per raster layer, full canvas size, unpremultiplied
thumbnail.png        a small preview for file browsers and the start screen
```

- `document.json` has a `"format": 1` field; readers refuse newer major formats with a clear
  message instead of guessing.
- New optional fields (like `guide`) do not bump the format: older readers ignore them and
  older files simply lack them.
- Images are stored uncompressed inside the zip (`Stored`): PNG is already compressed.
- Saving writes to a temporary file next to the target and renames it, so a crash never leaves
  a half-written document.
- PSD import and export come later and do not change this format.

## Consequences

- Anyone can unzip a document and get their layers as PNGs.
- Full-size PNGs per layer are larger than tiles on disk for sparse layers; acceptable for v0.1,
  revisit with tile chunks if files get too big.
