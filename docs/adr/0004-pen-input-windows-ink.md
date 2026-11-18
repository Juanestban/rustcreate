# ADR 0004: Pen input through Windows Ink (WM_POINTER)

- Status: accepted
- Date: 2026-11-18

## Context

Pressure and tilt are essential for painting. winit (under eframe) does not reliably expose pen
pressure and tilt on Windows. Tablets speak two APIs on Windows: the modern Pointer API
(Windows Ink) and the legacy Wacom Wintab.

## Decision

- v0.1 reads pen input from **`WM_POINTER` messages** (`GetPointerPenInfo`) through `windows-sys`,
  by subclassing the window eframe creates. It provides pressure, tilt, eraser end and barrel
  button, with high-frequency coalesced samples.
- The mouse is the fallback, with a fixed pressure of 1.0.
- **Wintab** goes to the roadmap for older tablets that do not support Windows Ink.

## Consequences

- Works with Wacom, Surface, XP-Pen and Huion in Windows Ink mode.
- Input code is Windows-specific and isolated in `crates/app`; other platforms get their own
  backends later.
