# ADR 0004: Pen input through Windows Ink (WM_POINTER)

- Status: accepted
- Date: 2026-11-18

## Context

Pressure and tilt are essential for painting. winit (under eframe) does not reliably expose pen
pressure and tilt on Windows (true of older winit versions; 0.30 reads pressure). Tablets speak two APIs on Windows: the modern Pointer API
(Windows Ink) and the legacy Wacom Wintab.

## Decision

- v0.1 reads pen input from **`WM_POINTER` messages** (Windows Ink). Updated 2026-11-18: winit
  0.30 already handles them (`GetPointerFrameInfoHistory` + `GetPointerPenInfo`) and egui
  delivers each coalesced sample as a touch event with its pressure, so pressure comes from
  there with no unsafe code.
- What winit drops (the **eraser end** and the **barrel button**) is read through `windows-sys` in
  a window subclass (`crates/app/src/pen.rs`) that only looks at `GetPointerPenInfo` and passes
  every message on unchanged. The eraser end erases with the eraser tool's brush; the barrel
  button is the eyedropper. The same module turns off press-and-hold right click and the tap ripples.
- **Tilt** is read in the same subclass (`tiltX`/`tiltY`, when the pen reports them) as one lean
  value, 0 upright to 1 flat. Brushes use it for size and flow (Brush Studio). It is sampled once
  per frame, not per coalesced sample.
- The mouse is the fallback, with a fixed pressure of 1.0.
- **Wintab** (updated 2026-11-19) is an opt-in setting for older tablets without Windows Ink.
  `wintab32.dll` comes with the tablet driver and is loaded at run time (no build dependency);
  a missing driver only turns the setting back off. The context is opened in message mode with
  the system cursor, so the tablet keeps moving the mouse: strokes come from mouse events and
  take their pressure from the latest `WT_PACKET`; tilt (altitude), the eraser end
  (`TPS_INVERT`) and the barrel button go to the same pen state as Windows Ink's. It stays off
  by default because opening a Wintab context next to Windows Ink can switch some drivers' mode.

## Consequences

- Works with Wacom, Surface, XP-Pen and Huion in Windows Ink mode, and with Wintab-only tablets
  once the setting is on. Wintab pressure is paired with mouse moves by time, not matched per
  sample.
- Input code is Windows-specific and isolated in `crates/app`; other platforms get their own
  backends later.
