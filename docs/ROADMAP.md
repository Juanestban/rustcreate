# Roadmap

## Current status

- **Done:** project foundation (licenses, docs, ADRs 0001–0006, graphify setup), app icon,
  CI and release packaging copied from archi-games. App skeleton: eframe window with the icon,
  system dark/light theme, Fluent `en`/`es`, updater (startup + every 5 h, banner, verified
  install). `core`: tiled layers, layer stack operations, blend modes, undo/redo history, `.rcreate`
  save/open and PNG export. `render`: wgpu compositor and canvas view with zoom, pan and rotation.
  Painting with the mouse: brush engine, Pencil/Ink/Airbrush/Eraser. Procreate-like interface:
  top bar, sidebar sliders, brush/layers/color/actions panels, own icons, dark/light palette,
  file dialogs, unsaved-changes prompt, all v0.1 Photoshop shortcuts. Pen input through
  Windows Ink (pressure, eraser end, barrel button). Settings (language, theme, update checks)
  and the "New canvas" dialog. A native error dialog when no DX12/Vulkan GPU is found. **All
  v0.1 features are in.**
- **Not verified yet:** Windows build, MSI packaging and the `.ico` (`build.rs`) only run on
  Windows/CI; the first push to GitHub will exercise them.
- **Next:** test on a Windows machine with a pen (build, MSI, portable EXE, pressure, eraser
  end), fix what shows up, then tag `v0.1.0` (see `RELEASING.md`). After that, Phase 2.

## Phase 0 — Foundation

- [x] Licenses (MIT OR Apache-2.0), README, RELEASING, NOTICE
- [x] `docs/DESIGN.md`, `docs/SHORTCUTS.md`, ADRs 0001–0006
- [x] `CLAUDE.md`, graphify `Stop` hook, `.gitignore` for graphify output
- [x] App icon (SVG source, PNG, `.ico` from `build.rs`)
- [x] CI (`ci.yml`) and release (`release.yml`, `scripts/package.ps1`, WiX MSI)

## Phase 1 — v0.1 (Windows)

### Core (`crates/core`)
- [x] Tiled RGBA8 layers (256×256, lazy allocation)
- [x] Layer stack: add, delete, duplicate, reorder, merge down, rename
- [x] Layer properties: opacity, hidden, locked, blend mode (Normal, Multiply, Screen, Overlay,
      Darken, Lighten, Add, Color Dodge, Color Burn), with a CPU reference compositor
      (`core::blend`) that merge down uses and the GPU shaders must match
- [x] History: copy-on-write tile snapshots, 250 steps, 2 GB memory cap
- [x] `.rcreate` save/open (atomic save, validated on open, thumbnail), PNG export

### Render (`crates/render`)
- [x] wgpu canvas sharing eframe's device (paint callback)
- [x] Zoom, pan, rotate view (pinch/Ctrl+wheel, scroll, Space/middle drag, hold R, Ctrl+0/1/+/-, Esc)
- [x] Brush engine (CPU, in `core`): dab stamping with spacing, size/flow from pressure, start taper,
      streamline, per-stroke opacity cap, airbrush build-up
- [x] Brushes: Pencil, Ink, Airbrush, Eraser (erases with the selected brush)
- [x] Layer compositor with blend-mode shaders (GPU test checks every mode against `core::blend`)
- [x] Dirty-tile uploads

### App (`crates/app`)
- [x] Procreate-like layout: top bar, side size/opacity sliders, floating panels, `Tab` hides it
- [x] Dark and light themes with an own palette (dark by default, chosen in Settings)
- [x] Own minimalist icon set (vector line icons painted in code, `ui/icons.rs`)
- [x] Brush picker, layers panel (visibility, lock, rename, opacity, blend, duplicate, merge,
      delete), color picker (HSV square + history) and eyedropper
- [x] Pen input through Windows Ink (`WM_POINTER`): pressure per coalesced sample, eraser end,
      barrel button = eyedropper, no press-and-hold ripples; mouse fallback (ADR 0004)
- [x] Photoshop shortcuts marked ✅ in `SHORTCUTS.md`
- [x] Fluent texts in English and Spanish (system language; test checks both files match)
- [x] Updater: check at startup and every 5 h, banner, verified install (ADR 0002)
- [x] Settings (in Actions, saved to `settings.json` in the user config folder): language,
      theme, automatic update checks
- [x] "New canvas" dialog: presets, custom size capped by the GPU, remembers the last size
- [x] Unsaved-changes prompt on close, new, open and before updating; Open/Save/Export dialogs
- [x] Clear error when no DX12/Vulkan GPU is available (native dialog in the user's language)

## Phase 2 — Painting depth

- [ ] End taper (needs the stroke length when it ends)

- [ ] Full brush library by category (Sketching, Inking, Drawing, Painting, Artistic,
      Calligraphy, Airbrushing, Textures, Abstract, Charcoals, Elements, Spraypaints…)
- [ ] Brush Studio: edit every parameter, grain and shape textures, live preview, pen tilt
- [ ] Smudge tool and wet mixing
- [ ] Stabilization modes (StreamLine, motion filtering)
- [ ] QuickShape (hold to snap lines, ellipses, polygons)
- [ ] Paint bucket (ColorDrop) with threshold, gradient tool
- [ ] Color disc, eyedropper loupe, color palettes, harmony modes
- [ ] Drawing guides: grid, perspective (1/2/3 point), symmetry
- [ ] Import `.brushset` and `.abr` brushes the user owns

## Phase 3 — Photoshop-grade editing

- [ ] Selections: marquee, lasso, magic wand, feather, invert, save
- [ ] Transform: free, uniform, distort, warp, liquify
- [ ] Layer masks and clipping masks, Alpha Lock
- [ ] Layer groups with their own blend mode and opacity
- [ ] Adjustment layers: levels, curves, hue/saturation, color balance, gradient map
- [ ] Filters: Gaussian/motion blur, sharpen, noise, chromatic aberration
- [ ] Text layers
- [ ] All blend modes (Soft/Hard/Vivid/Linear Light, Difference, Exclusion, Hue, Saturation,
      Color, Luminosity…)
- [ ] 16-bit channels and color management (Display P3, ICC)
- [ ] PSD import/export, JPEG/WebP/TIFF export
- [ ] Reference window, time-lapse recording

## Phase 4 — Animation

- [ ] Animation Assist: timeline of frames (one layer or group per frame)
- [ ] Onion skin (frames, opacity, colors), hold frames, background/foreground frames
- [ ] Playback: loop, ping-pong, one-shot; frames per second
- [ ] Export GIF, MP4, animated PNG, PNG sequence

## Phase 5 — More platforms

- [ ] Wintab pen input (older tablets)
- [ ] macOS (Metal), Apple Pencil on iPad/iPhone
- [ ] Android (Vulkan, stylus)
- [ ] Linux (Wayland/X11 tablets)
- [ ] Touch gestures: two-finger undo, three-finger redo, pinch zoom/rotate
