# ADR 0007: Animation frames are layers

- Status: accepted
- Date: 2026-11-19

## Context

Phase 4 adds animation. There are two common models: Procreate's Animation Assist, where the
layer stack itself is the timeline, and a separate timeline (Photoshop, Krita) where every frame
has its own layers. Procreate is the app rustcreate follows, and its model fits everything built
so far (layers, groups, masks, the `.rcreate` format).

## Decision

- **Every top-level layer or group is a frame**, bottom (first) to top. A frame drawn with several
  layers is a group.
- With Animation Assist on, only one frame is in the picture: the active layer's, or the one
  playing. `Document::hides` decides it, and the CPU composite and the GPU renderer both follow it.
- The settings (`enabled`, `fps` 1–60, `playback`: loop, ping-pong or one shot) are saved in
  `document.json` as an optional `animation` field, so the format version stays 1. Changing them
  is not an undo step (like the guide).
- Export works frame by frame from `Document::frame_tile`: GIF, animated PNG and PNG sequences
  first; MP4 later with OpenH264.

## Consequences

- No new document structure: painting, undo, files and the layers panel work on frames as they
  are.
- A separate timeline (several tracks, frames of different lengths) would need a new model. Hold
  frames and background/foreground frames come as per-frame flags instead.
