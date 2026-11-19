# Keyboard shortcuts

rustcreate follows Photoshop's shortcuts. On macOS (later), `Ctrl` becomes `Cmd` and `Alt`
becomes `Option`. Status: ✅ in v0.1 · 🕓 planned (see [ROADMAP.md](ROADMAP.md)).

## Tools

| Shortcut | Action | Status |
|---|---|---|
| `B` | Brush | ✅ |
| `E` | Eraser | ✅ |
| `Shift+B` | Next brush in the same library category | ✅ |
| `I` | Eyedropper (also `Alt` held while painting) | ✅ |
| `H` (toggle) / hold `Space` | Hand (pan) | ✅ |
| `Z` | Zoom tool | 🕓 |
| `R` | Rotate view (`Esc` resets rotation) | ✅ |
| `Esc` | Close the open panel (brushes, layers, color…); a press on the canvas closes it too | ✅ |
| `S` | Smudge | ✅ |
| `G` | Paint bucket; again for the gradient | ✅ |
| `M` | Marquee selection (again: ellipse) | ✅ |
| `L` | Lasso selection | ✅ |
| `W` | Magic wand | ✅ |
| `Shift` / `Alt` + selection drag | Add to / subtract from the selection | ✅ |
| `V` | Move | 🕓 |
| `T` | Text | 🕓 |
| `C` | Crop tool (drag a frame; for now: Actions → Crop to selection) | 🕓 |

## Brush

| Shortcut | Action | Status |
|---|---|---|
| `[` / `]` | Decrease / increase brush size | ✅ |
| `Shift+[` / `Shift+]` | Decrease / increase brush hardness | 🕓 |
| `1` … `9`, `0` | Brush opacity 10 % … 90 %, 100 % | ✅ |
| `X` | Swap primary and secondary colors | ✅ |
| `D` | Reset colors to black and white | ✅ |
| `Shift` + click | Straight line from the last point | ✅ |
| `Shift` while a QuickShape snaps or is edited | Perfect shape: circle, regular polygon, 15° lines | ✅ |

## Edit and history

| Shortcut | Action | Status |
|---|---|---|
| `Ctrl+Z` | Undo | ✅ |
| `Ctrl+Shift+Z` / `Ctrl+Y` | Redo | ✅ |
| `Ctrl+Alt+Z` | Step backward (Photoshop legacy) | ✅ |
| `Ctrl+X` / `Ctrl+C` / `Ctrl+V` | Cut / copy / paste | 🕓 |
| `Ctrl+T` | Free transform (Enter or Ctrl+T applies, Esc or Ctrl+Z cancels; Shift: free scale, 15° turns) | ✅ |
| `Ctrl+A` / `Ctrl+D` / `Ctrl+Shift+I` | Select all / deselect / invert selection | ✅ |
| `Backspace` / `Delete` | Clear layer (or selection) | ✅ |
| `Alt+Backspace` | Fill with primary color | ✅ |

## Adjustments

| Shortcut | Action | Status |
|---|---|---|
| `Ctrl+U` | Hue/Saturation | ✅ |
| `Ctrl+L` | Levels | ✅ |
| `Ctrl+M` | Curves | ✅ |
| `Ctrl+I` | Invert | ✅ |
| `Ctrl+Shift+U` | Desaturate | ✅ |
| `Enter` / `Esc` | Apply / cancel the open adjustment | ✅ |
| `Ctrl+Alt+C` | Canvas Size | ✅ |
| `Ctrl+Alt+I` | Image Size | ✅ |

## Layers

| Shortcut | Action | Status |
|---|---|---|
| `Ctrl+Shift+N` | New layer | ✅ |
| `Ctrl+J` | Duplicate layer | ✅ |
| `Ctrl+E` | Merge down | ✅ |
| `Ctrl+G` | Group layers | ✅ |
| `Ctrl+Shift+G` | Ungroup | ✅ |
| `Ctrl+Alt+G` | Clipping mask | ✅ |
| `/` | Alpha Lock (lock transparent pixels) | ✅ |
| `Ctrl+[` / `Ctrl+]` | Move layer down / up | ✅ |
| `Alt+[` / `Alt+]` | Select layer below / above | ✅ |

## View

| Shortcut | Action | Status |
|---|---|---|
| `Ctrl++` / `Ctrl+-` | Zoom in / out | ✅ |
| `Ctrl+0` | Fit to screen | ✅ |
| `Ctrl+1` | Actual pixels (100 %) | ✅ |
| `Tab` | Hide / show the interface | ✅ |
| `F` | Cycle screen modes (full screen) | 🕓 |

## File

| Shortcut | Action | Status |
|---|---|---|
| `Ctrl+N` | New canvas | ✅ |
| `Ctrl+O` | Open | ✅ |
| `Ctrl+S` | Save | ✅ |
| `Ctrl+Shift+S` | Save as | ✅ |
| `Ctrl+Alt+Shift+W` | Export PNG ("Export As") | ✅ |

## Animation (later)

| Shortcut | Action | Status |
|---|---|---|
| `,` / `.` | Previous / next frame | 🕓 |
| `Enter` (timeline focused) | Play / pause | 🕓 |
| `Ctrl+Shift+F` | New frame | 🕓 |
| `O` | Toggle onion skin | 🕓 |
