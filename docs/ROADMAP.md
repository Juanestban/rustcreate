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
  v0.1 features are in.** Phase 2 started: Smudge tool, end taper, pulled-string stabilizer, brush tips and grains,
  Brush Studio, the brush library with saved brushes and a brush per tool, the paint bucket and the
  gradient, QuickShape, the color disc with harmonies and palettes, drawing guides with symmetry,
  brush import from `.brushset` and `.abr`, ColorDrop and the eyedropper loupe,
  motion filtering, perspective guides, QuickShape editing and arcs, wet
  mixing, pen tilt, ColorDrop threshold, palette import/export, the guide saved in the file,
  Calligraphy/Abstract/Elements brushes, `.rcbrushes` sets and the brush outline. **Phase 2 is
  done.** Phase 3 started: selections, Free Transform, Alpha Lock, clipping masks, layer masks, adjustments, filters, all 23 blend modes, crop and canvas/image size, layer groups.
- **Start-up freeze (fixed):** since the brush outline (v0.5.0) the first frame deadlocked: the
  outline asked egui for a drag payload inside `ui.input`, which already held the context's lock.
  eframe shows the window only after the first frame, so the app ran with no window on every
  platform (a debug build panics after 10 s with "Failed to acquire RwLock read"). Earlier guesses
  (FXC and the shader) were not the cause; the shader change stays as harmless FXC hardening, and a
  panic on the main thread shows a dialog and ends the process. `%APPDATA%\rustcreate\rustcreate.log` records each run's steps, panics with
  backtraces and UI stalls; the previous run stays in `rustcreate.previous.log`.
- **Not verified yet:** MSI packaging and the `.ico` (`build.rs`) only run on Windows/CI.
- **UI pass (2026-11-18):** Free Transform previews on the GPU (the lifted pixels upload once,
  each drag frame only moves a map; pixels are drawn once, on Done) with cursors for move, scale
  and turn; its bar stays above the panels, and brushes/color panels no longer apply it. Panels
  close with Esc, a press on the canvas or their ×. The canvas display averages four taps when
  zoomed out and shows crisp pixels only from 3×; the hue ring has soft edges. Layer rows are
  taller with centered names, rename by double click or right click. `rustcreate-core` builds
  optimized in dev too.
- **Text layers (2026-11-18):** `core::text` draws text with the system's fonts (fontdb +
  ab_glyph) into the layer; the text is kept on the layer and in the file, so it stays editable
  until a pixel edit rasterizes it.
- **Next:** v0.1.0 still waits on a Windows test with a pen. Phase 3: PSD import/export and
  JPEG/WebP export next.

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

- [x] End taper: the tail is re-stamped thinner when the stroke ends

- [x] Brush library by category: 20 original brushes in Sketching, Inking, Painting,
      Airbrushing and Textures, plus My brushes (duplicate, rename, delete); previews painted by
      the engine; each tool (paint, smudge, erase) keeps its own brush
- [x] Edited and new brushes saved to `brushes.json` (only the differences from the built-ins)
- [x] Calligraphy and Abstract categories (8 new brushes); tip roundness for flat nibs and
      ribbons; rustcreate brush sets (`.rcbrushes`: every setting plus the images) exported per
      category and imported back, with values brought into range
- [x] Elements category: Clouds, Smoke, Water, Flame, Sparks, with two new procedural grains
      (clouds, ripples)
- [x] Brush outline under the pointer: the brush, smudge or eraser size at the current zoom,
      turned like a fixed-angle tip; a cross for tiny brushes
- [x] Brush Studio: every parameter (shape, grain, dynamics, stroke), live preview painted by
      the engine, reset
- [x] Procedurally generated tips (round, square, chalk, bristles, splatter) and grains (paper,
      canvas, noise), all original (ADR 0006); rotation, follow-the-stroke, scatter, size jitter
- [x] Pen tilt (Windows Ink): leaning the pen widens the dab and thins the paint, per brush;
      Graphite, Soft Graphite and Charcoal shade with it
- [x] Smudge tool (`S`): patch smudging with the brush's shape and opacity, pressure-aware
- [x] Wet mixing per brush: picks up the layer's color under the brush and carries it along;
      paint charge runs the brush dry (thinner, more picked-up color); Flat Brush, Round Bristle
      and Gouache use it
- [x] Stabilization: StreamLine plus a pulled-string stabilizer; strokes catch up with the
      pointer when they end; per-brush settings in the brushes panel
- [x] Motion filtering per brush: adaptive smoothing (after the 1€ filter) that removes slow
      jitter from shaky hands or noisy tablets without making fast strokes lag
- [x] QuickShape: holding still at the end of a stroke snaps it to a line, polyline, ellipse
      or polygon drawn with the same brush (setting to turn it off)
- [x] QuickShape editing: after lifting, drag the shape's handles (ends, corners, ellipse center
      and axes); arcs; Shift makes it perfect (circle, regular polygon, 15° lines)
- [x] Paint bucket with threshold, sampling the active layer or all visible layers, growing
      under antialiased edges; gradient tool (linear/radial, to the secondary color or to clear,
      dithered, live while dragging); Alt+Backspace fills the layer
- [x] ColorDrop: drag the color button onto the canvas to fill there (bucket threshold and
      sampling)
- [x] ColorDrop threshold: hold still over the canvas to fill, slide sideways to change the
      threshold live (one undo step); the threshold is kept for the bucket
- [x] Color panel: own hue ring with saturation/brightness square, classic picker, harmony
      modes (complementary, split, analogous, triadic, tetradic), hex field, palettes saved to
      `palettes.json` (new, rename, delete, add/remove colors)
- [x] Eyedropper loupe: a ring by the pointer, the picked color over the previous one
- [x] Palette import/export: Procreate `.swatches`, Photoshop `.aco` (v1/v2) and Adobe `.ase`
      (one palette per group); CMYK and Lab convert without a profile until color management
- [x] Drawing guides: 2D grid and isometric with Drawing Assist (strokes keep to the grid's
      directions); symmetry (vertical, horizontal, quadrant, radial 2–16) painted by the engine
      in the same stroke, one undo step
- [x] Perspective guides with 1, 2 or 3 vanishing points: assisted strokes head for the nearest
      vanishing point (or stay horizontal/vertical where the perspective keeps them); "Edit
      guide" drags the vanishing points or the grid/symmetry center
- [x] Guide saved in the file: `document.json` keeps it per canvas (optional field, same
      format version); a new canvas starts without one
- [x] Import brushes the user owns: Procreate `.brushset`/`.brush` (shape and grain images)
      and Photoshop `.abr` v6+ (sampled tips, raw or PackBits) into My brushes; imported images
      kept in `brush-images/` next to the settings
- Skipped: reading the rest of an imported brush's settings (Procreate `Brush.archive`, ABR
  descriptors). They are closed binary formats; imported brushes keep their shape and grain.

## Phase 3 — Photoshop-grade editing

The Photoshop features people use most, in this order:

- [x] Selections: rectangle, ellipse, lasso and magic wand; add/subtract with Shift/Alt; select
      all, deselect, invert (Ctrl+A, Ctrl+D, Ctrl+Shift+I); feather; painting, fill and
      gradient stay inside the selection;
      undoable; marching ants
- [x] Free Transform (Ctrl+T): move, scale, rotate and flip the selection or the layer;
      bilinear, one undo step, the selection follows; Esc cancels
- [x] Alpha Lock and clipping masks (CPU and GPU, merge down respects clipping, saved in the file)
- [x] Layer masks: add, edit (brush, eraser, fill and gradient paint gray), apply, delete;
      CPU and GPU, saved as `masks/{id}.png`
- [x] Adjustments: Hue/Saturation (Ctrl+U), Levels (Ctrl+L), Curves (Ctrl+M),
      Brightness/Contrast, Invert (Ctrl+I), Desaturate (Ctrl+Shift+U), live preview; on the
      layer or its mask, inside the selection, one undo step
- [x] Filters: Gaussian blur, Unsharp Mask, Add Noise (in the Adjustments menu, live preview,
      premultiplied blur so colors do not bleed from transparency)
- [x] The remaining blend modes: Linear Burn, Subtract, Divide, Soft/Hard/Vivid/Linear/Pin
      Light, Difference, Exclusion, Hue, Saturation, Color, Luminosity (23 in all, CPU and GPU,
      grouped as in Photoshop)
- [x] Crop to selection, Canvas Size (with anchor) and Image Size (resampled, averaging when
      shrinking): layers, masks and the selection; undo restores the old size
- [x] Layer groups with their own blend mode, opacity and mask, nestable; Ctrl+[ / Ctrl+] step
      in and out of groups; a group composites its children on their own (no pass-through yet)
- [x] Text layers (T): system fonts, size, color, multi-line; click to add, click one to edit,
      drag to move; painting, filling, adjusting or transforming one rasterizes it (no shaping
      for complex scripts yet)
- [ ] PSD import/export, JPEG/WebP export

Later: adjustment layers, warp/distort and liquify, motion blur and chromatic aberration,
16-bit channels and color management (Display P3, ICC), TIFF export, reference window,
time-lapse recording.

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
