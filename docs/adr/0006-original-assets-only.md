# ADR 0006: Original assets only

- Status: accepted
- Date: 2026-11-18

## Context

rustcreate is inspired by Procreate and Photoshop. Their names, icons, brush textures and
artwork are protected. Shipping any of them would put the project at risk of takedown.

## Decision

- "Procreate" and "Photoshop" are never used as our brand; the README only says "inspired by"
  and states we are not affiliated.
- **No copied assets**: icons, brush shapes and grains, textures, sounds and fonts are original
  or come with a license that allows redistribution (credited in `NOTICE.md`).
- The interface follows Procreate's *layout and workflow* (top bar, side size/opacity sliders,
  floating layers panel), drawn with our own minimalist icon set.
- Brushes reproduce the *categories and behavior* (spacing, jitter, taper, pressure curves) with
  our own parameters and textures. Importing users' own `.brushset` / `.abr` files comes later.
- Keyboard shortcuts follow Photoshop's conventions; shortcuts are not copyrightable.

## Consequences

- Every asset added to the repository must be original or have its license recorded.
