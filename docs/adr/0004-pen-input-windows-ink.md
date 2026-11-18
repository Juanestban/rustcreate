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
  every message on unchanged. The eraser end erases with the current brush; the barrel button
  is the eyedropper. The same module turns off press-and-hold right click and the tap ripples.
- **Tilt** is not read yet: no brush uses it. It joins the subclass when Brush Studio adds tilt
  parameters.
- The mouse is the fallback, with a fixed pressure of 1.0.
- **Wintab** goes to the roadmap for older tablets that do not support Windows Ink.

## Consequences

- Works with Wacom, Surface, XP-Pen and Huion in Windows Ink mode.
- Input code is Windows-specific and isolated in `crates/app`; other platforms get their own
  backends later.
