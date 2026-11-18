//! The canvas area: view transform (zoom, pan, rotate), painting with the pointer, their
//! gestures and shortcuts, and the paint callback that hands drawing to `rustcreate_render`.

use eframe::egui::{self, Event, Key, Modifiers, Pos2, Rect, Vec2, vec2};
use eframe::egui_wgpu::{self, wgpu};
use rustcreate_core::{Document, Stroke};
use rustcreate_render::{CanvasRenderer, Placement};

use crate::i18n::I18n;
use crate::tools::Tools;

const MIN_ZOOM: f32 = 0.01;
const MAX_ZOOM: f32 = 64.0;
const ZOOM_STEP: f32 = 1.25;

/// Where the canvas sits on screen, in egui points: canvas pixel `p` is drawn at
/// `area center + offset + R(rotation) · zoom · (p − canvas size / 2)`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct View {
  /// Screen points per canvas pixel.
  pub zoom: f32,
  /// Radians, clockwise on screen.
  pub rotation: f32,
  pub offset: Vec2,
}

impl View {
  /// The whole canvas centered with a margin, unrotated.
  pub fn fit(area: Vec2, canvas: Vec2) -> Self {
    let zoom = (area.x / canvas.x).min(area.y / canvas.y) * 0.9;
    Self { zoom: zoom.clamp(MIN_ZOOM, MAX_ZOOM), rotation: 0.0, offset: Vec2::ZERO }
  }

  fn rotate(v: Vec2, angle: f32) -> Vec2 {
    let (sin, cos) = angle.sin_cos();
    vec2(v.x * cos - v.y * sin, v.x * sin + v.y * cos)
  }

  #[cfg(test)]
  pub fn to_screen(self, center: Pos2, canvas: Vec2, p: Pos2) -> Pos2 {
    center + self.offset + Self::rotate((p.to_vec2() - canvas / 2.0) * self.zoom, self.rotation)
  }

  pub fn to_canvas(self, center: Pos2, canvas: Vec2, screen: Pos2) -> Pos2 {
    (Self::rotate(screen - center - self.offset, -self.rotation) / self.zoom + canvas / 2.0).to_pos2()
  }

  /// Zooms by `factor`, keeping the canvas point under `anchor` (relative to the area center)
  /// in place.
  pub fn zoom_at(&mut self, factor: f32, anchor: Vec2) {
    let zoom = (self.zoom * factor).clamp(MIN_ZOOM, MAX_ZOOM);
    let factor = zoom / self.zoom;
    self.offset = anchor - (anchor - self.offset) * factor;
    self.zoom = zoom;
  }

  /// Rotates by `angle` around `anchor` (relative to the area center).
  pub fn rotate_at(&mut self, angle: f32, anchor: Vec2) {
    self.offset = anchor + Self::rotate(self.offset - anchor, angle);
    self.rotation = (self.rotation + angle).rem_euclid(std::f32::consts::TAU);
  }

  /// The renderer's placement for an area of `rect` at `pixels_per_point`.
  fn placement(&self, rect: Rect, canvas: Vec2, pixels_per_point: f32) -> Placement {
    let x_axis = Self::rotate(vec2(self.zoom, 0.0), self.rotation) * pixels_per_point;
    let y_axis = Self::rotate(vec2(0.0, self.zoom), self.rotation) * pixels_per_point;
    let origin = (rect.size() / 2.0 + self.offset + Self::rotate(-canvas / 2.0 * self.zoom, self.rotation)) * pixels_per_point;
    Placement {
      m: [[x_axis.x, x_axis.y], [y_axis.x, y_axis.y]],
      t: [origin.x, origin.y],
      area: [rect.width() * pixels_per_point, rect.height() * pixels_per_point],
    }
  }
}

pub struct Canvas {
  view: Option<View>,
  tools: Tools,
  stroke: Option<Stroke>,
  /// Where the last stroke ended, in canvas pixels, for Shift+click lines.
  last_point: Option<(f32, f32)>,
}

impl Canvas {
  /// Registers the GPU renderer for `doc` with eframe's wgpu state.
  pub fn new(render_state: &egui_wgpu::RenderState, doc: &Document) -> Self {
    let renderer = CanvasRenderer::new(&render_state.device, render_state.target_format, (doc.width, doc.height));
    render_state.renderer.write().callback_resources.insert(renderer);
    Self { view: None, tools: Tools::default(), stroke: None, last_point: None }
  }

  /// Draws the canvas filling `ui`, after handling painting, view gestures and shortcuts.
  pub fn show(&mut self, ui: &mut egui::Ui, render_state: &egui_wgpu::RenderState, doc: &mut Document, i18n: &I18n) {
    let (rect, response) = ui.allocate_exact_size(ui.available_size(), egui::Sense::click_and_drag());
    let canvas = vec2(doc.width as f32, doc.height as f32);
    let view = self.view.get_or_insert_with(|| View::fit(rect.size(), canvas));
    handle_input(ui, &response, rect, canvas, view);
    let view = *view;
    let painting = self.stroke.is_some();
    ui.input_mut(|input| self.tools.shortcuts(input, doc, painting));
    self.paint(ui, &response, |p| view.to_canvas(rect.center(), canvas, p), doc);

    if let Some(renderer) = render_state.renderer.write().callback_resources.get_mut::<CanvasRenderer>() {
      renderer.sync(&render_state.device, &render_state.queue, doc);
    }
    let placement = view.placement(rect, canvas, ui.ctx().pixels_per_point());
    ui.painter().add(egui_wgpu::Callback::new_paint_callback(rect, CanvasCallback { placement }));
    // shortcut: a status line until the brush and color panels exist.
    let status = ui.painter().layout_no_wrap(self.tools.status(i18n), egui::FontId::proportional(13.0), ui.visuals().weak_text_color());
    ui.painter().galley(rect.left_bottom() + vec2(12.0, -12.0 - status.size().y), status, ui.visuals().weak_text_color());
  }

  /// Turns primary-button presses and drags into strokes. Space and R drags belong to the view.
  fn paint(&mut self, ui: &egui::Ui, response: &egui::Response, to_canvas: impl Fn(Pos2) -> Pos2, doc: &mut Document) {
    let (pressed, released, shift, view_drag, origin, moves) = ui.input(|i| {
      let moves: Vec<Pos2> = i.events.iter().filter_map(|e| if let Event::PointerMoved(p) = e { Some(*p) } else { None }).collect();
      (
        i.pointer.primary_pressed(),
        i.pointer.primary_released(),
        i.modifiers.shift,
        i.key_down(Key::Space) || i.key_down(Key::R),
        i.pointer.press_origin(),
        moves,
      )
    });
    // shortcut: mouse and trackpad paint at full pressure; pen pressure comes with Windows Ink (ADR 0004).
    const PRESSURE: f32 = 1.0;
    if pressed
      && self.stroke.is_none()
      && !view_drag
      && response.hovered()
      && let Some(origin) = origin
    {
      let (brush, color, erase) = self.tools.current();
      // A locked layer refuses the stroke.
      if let Some(mut stroke) = doc.begin_stroke(brush, color, erase) {
        let p = to_canvas(origin);
        match self.last_point {
          Some(from) if shift => stroke.line(doc, from, (p.x, p.y), PRESSURE),
          _ => stroke.add(doc, p.x, p.y, PRESSURE),
        }
        self.stroke = Some(stroke);
      }
    }
    if let Some(stroke) = &mut self.stroke {
      for p in moves {
        let p = to_canvas(p);
        stroke.add(doc, p.x, p.y, PRESSURE);
      }
      if released {
        self.last_point = stroke.position();
        self.stroke = None;
      }
    }
  }
}

fn handle_input(ui: &egui::Ui, response: &egui::Response, rect: Rect, canvas: Vec2, view: &mut View) {
  let center = rect.center();
  let pointer = response.hover_pos();
  let anchor = pointer.map_or(Vec2::ZERO, |p| p - center);
  ui.input_mut(|input| {
    if response.hovered() {
      // Pinch or Ctrl+wheel zooms; two-finger scroll or the wheel pans.
      let zoom = input.zoom_delta();
      if zoom != 1.0 {
        view.zoom_at(zoom, anchor);
      }
      view.offset += input.smooth_scroll_delta;
      if let Some(touch) = input.multi_touch() {
        view.rotate_at(touch.rotation_delta, anchor);
      }
    }

    // Photoshop view shortcuts (docs/SHORTCUTS.md).
    if input.consume_key(Modifiers::COMMAND, Key::Num0) {
      *view = View::fit(rect.size(), canvas);
    }
    if input.consume_key(Modifiers::COMMAND, Key::Num1) {
      // 100 %: one canvas pixel per physical pixel.
      let actual = 1.0 / input.pixels_per_point;
      view.zoom_at(actual / view.zoom, Vec2::ZERO);
    }
    if input.consume_key(Modifiers::COMMAND, Key::Plus) || input.consume_key(Modifiers::COMMAND, Key::Equals) {
      view.zoom_at(ZOOM_STEP, Vec2::ZERO);
    }
    if input.consume_key(Modifiers::COMMAND, Key::Minus) {
      view.zoom_at(1.0 / ZOOM_STEP, Vec2::ZERO);
    }
    if input.consume_key(Modifiers::NONE, Key::Escape) {
      view.rotate_at(-view.rotation, Vec2::ZERO);
    }
  });

  if response.dragged() {
    let (space, rotate) = ui.input(|i| (i.key_down(Key::Space), i.key_down(Key::R)));
    let middle = response.dragged_by(egui::PointerButton::Middle);
    let delta = response.drag_delta();
    if rotate && let Some(p) = pointer {
      // Hold R and drag: turn by the angle the pointer swept around the area center.
      let (now, before) = (p - center, p - delta - center);
      view.rotate_at(now.angle() - before.angle(), Vec2::ZERO);
    } else if space || middle {
      // Hold Space (or the middle button) and drag: pan.
      view.offset += delta;
    }
  }
}

struct CanvasCallback {
  placement: Placement,
}

impl egui_wgpu::CallbackTrait for CanvasCallback {
  fn prepare(
    &self,
    _device: &wgpu::Device,
    queue: &wgpu::Queue,
    _screen: &egui_wgpu::ScreenDescriptor,
    _encoder: &mut wgpu::CommandEncoder,
    resources: &mut egui_wgpu::CallbackResources,
  ) -> Vec<wgpu::CommandBuffer> {
    if let Some(renderer) = resources.get::<CanvasRenderer>() {
      renderer.place(queue, self.placement);
    }
    Vec::new()
  }

  fn paint(&self, _info: egui::PaintCallbackInfo, pass: &mut wgpu::RenderPass<'static>, resources: &egui_wgpu::CallbackResources) {
    if let Some(renderer) = resources.get::<CanvasRenderer>() {
      renderer.paint(pass);
    }
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  fn close(a: Pos2, b: Pos2) -> bool {
    (a - b).length() < 1e-3
  }

  #[test]
  fn screen_and_canvas_coordinates_round_trip() {
    let view = View { zoom: 0.5, rotation: 0.7, offset: vec2(30.0, -12.0) };
    let (center, canvas) = (Pos2::new(400.0, 300.0), vec2(2048.0, 1536.0));
    let p = Pos2::new(100.0, 900.0);
    assert!(close(view.to_canvas(center, canvas, view.to_screen(center, canvas, p)), p));
    assert!(close(View::fit(vec2(800.0, 600.0), canvas).to_screen(center, canvas, (canvas / 2.0).to_pos2()), center), "fit centers the canvas");
  }

  #[test]
  fn zoom_and_rotation_keep_the_anchor_still() {
    let (center, canvas) = (Pos2::new(400.0, 300.0), vec2(1000.0, 1000.0));
    let mut view = View { zoom: 1.0, rotation: 0.3, offset: vec2(10.0, 20.0) };
    let cursor = Pos2::new(520.0, 260.0);
    let under = view.to_canvas(center, canvas, cursor);
    view.zoom_at(2.5, cursor - center);
    assert!(close(view.to_canvas(center, canvas, cursor), under));
    view.rotate_at(1.1, cursor - center);
    assert!(close(view.to_canvas(center, canvas, cursor), under));
    view.zoom_at(1e9, Vec2::ZERO);
    assert_eq!(view.zoom, MAX_ZOOM);
  }

  #[test]
  fn placement_matches_to_screen() {
    let view = View { zoom: 0.8, rotation: 0.4, offset: vec2(-50.0, 25.0) };
    let rect = Rect::from_min_size(Pos2::new(0.0, 40.0), vec2(800.0, 600.0));
    let canvas = vec2(640.0, 480.0);
    let ppp = 2.0;
    let pl = view.placement(rect, canvas, ppp);
    let p = Pos2::new(123.0, 456.0);
    let physical = vec2(pl.m[0][0] * p.x + pl.m[1][0] * p.y + pl.t[0], pl.m[0][1] * p.x + pl.m[1][1] * p.y + pl.t[1]);
    let expected = (view.to_screen(rect.center(), canvas, p) - rect.min) * ppp;
    assert!((physical - expected).length() < 1e-2, "{physical:?} vs {expected:?}");
  }
}
