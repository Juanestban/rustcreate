# ADR 0008: Photoshop's layout instead of Procreate's

- Status: accepted (replaces the layout part of DESIGN's first interface decision)
- Date: 2026-11-19

## Context

The first interface copied Procreate's tablet layout: a thin top bar of icons and floating
panels that close when the canvas is touched. On a desktop with a mouse it got in the way:
layers, brushes and color could not stay open next to the canvas, panels overflowed when the
window was maximized, and people coming from Photoshop looked for its menus.

## Decision

- A menu bar on top: File, Edit, Image, Layer, Select, Filter, View, Window, Help. Every command
  of the old Actions and Adjustments panels lives in a menu.
- The tools on the left, with the color, the size and opacity sliders and undo/redo.
- A resizable dock on the right with folding windows: tool options, color, brushes and layers.
  Which ones show is a setting (`Settings::panels`), changed from the Window menu.
- Settings get their own window (Edit > Settings, `Ctrl+K`, the home screen).
- The canvas, the brush engine, the gestures and the look (ADR 0006) stay the same.

## Consequences

- No floating panels: no cover over the canvas, and Esc goes straight to the canvas.
- Layer changes are disabled in the dock while a transform or text is in progress; Enter
  applies them first.
- Touch-first platforms (iPad, Android) will need their own, Procreate-like layout later.
