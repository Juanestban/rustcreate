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
- Export works frame by frame from `Document::frame_tile`: GIF, animated PNG, PNG sequences and
  MP4. MP4 is H.264 from OpenH264 compiled from source (the `openh264` crate, BSD-2-Clause) in an
  `mp4` container, on white, padded to an even size of at least 16 px; a held frame is one
  sample with a longer duration.

## Consequences

- H.264 is patented. Cisco pays the royalties only for its own prebuilt OpenH264 binaries;
  an OpenH264 compiled into the app is not covered. Fine for a free open-source app for now;
  before a commercial release, load Cisco's binary at run time (the crate's `libloading`
  feature) instead.
- `openh264-sys2` compiles C++ in its build script: the Windows CI (MSVC) builds it, but a
  Windows clippy check from macOS needs a stand-in C/C++ compiler.

- No new document structure: painting, undo, files and the layers panel work on frames as they
  are.
- A separate timeline (several tracks) would need a new model. Frames of different lengths are
  a hold duration on the frame's layer (`hold`, saved with it); the background and foreground
  frames are the first and last frame, switched on in the animation settings.
- Onion skin is only on screen: the renderer fades and tints nearby frames (red before, green
  after); the CPU composite, the eyedropper and exports never see it.
