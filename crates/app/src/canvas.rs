//! The canvas area: view transform (zoom, pan, rotate), painting with the pointer, their
//! gestures and shortcuts, and the paint callback that hands drawing to `rustcreate_render`.

use eframe::egui::{self, Event, Key, Modifiers, Pos2, Rect, Vec2, vec2};
use eframe::egui_wgpu::{self, wgpu};
use rustcreate_core::{Document, Gradient, Shape, Stroke};

/// QuickShape: how long the pointer must rest, and how far (points) it may drift meanwhile.
const HOLD_SECONDS: f64 = 0.6;
const HOLD_SLOP: f32 = 4.0;
use rustcreate_render::{CanvasRenderer, Placement};

use crate::tools::{Tool, Tools};

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
  pub tools: Tools,
  stroke: Option<Stroke>,
  /// Where the last stroke ended, in canvas pixels, for Shift+click lines.
  last_point: Option<(f32, f32)>,
  /// The pen's pressure while it touches (Windows Ink); `None` means mouse.
  pen: Option<f32>,
  /// A gradient being dragged: from where it started (screen, canvas) to where it is now.
  gradient: Option<(Gradient, Pos2, Pos2, Pos2)>,
  /// QuickShape: when and where the pointer last moved during the stroke, and whether the
  /// stroke already snapped to a shape (it then ignores the pointer until released).
  hold: (f64, Pos2),
  snapped: bool,
  /// Drawing Assist: where the stroke started (canvas pixels) and the grid direction it keeps.
  assist: (Pos2, Option<f32>),
  /// Eyedropper loupe while picking: where (screen), and the color before picking started.
  loupe: Option<(Pos2, [u8; 3])>,
  /// The guide point being dragged while editing the guide.
  handle: Option<usize>,
  /// QuickShape: the shape the stroke snapped to and its pressure. After release the stroke
  /// stays open while the shape is edited by its handles (`shape_edit`).
  shape: Option<(Shape, f32)>,
  shape_edit: bool,
  shape_handle: Option<usize>,
  /// Shift on the previous frame, to make the shape perfect once per press.
  shift: bool,
}

/// A ring by the pointer: the color under it on top, the one before picking below.
fn loupe(painter: &egui::Painter, at: Pos2, now: [u8; 3], before: [u8; 3]) {
  let center = at + vec2(0.0, -70.0);
  let half = |from: f32, color: [u8; 3]| {
    let points: Vec<Pos2> = (0..=32).map(|i| center + 38.0 * Vec2::angled(from + i as f32 / 32.0 * std::f32::consts::PI)).collect();
    painter.add(egui::Shape::line(points, egui::Stroke::new(16.0, egui::Color32::from_rgb(color[0], color[1], color[2]))));
  };
  half(std::f32::consts::PI, now);
  half(0.0, before);
  for radius in [29.0, 47.0] {
    painter.circle_stroke(center, radius, egui::Stroke::new(1.0, egui::Color32::from_black_alpha(120)));
  }
}

/// ColorDrop: the color being dragged from the color button onto the canvas.
pub struct ColorDrop(pub [u8; 3]);

impl Canvas {
  /// Registers the GPU renderer for `doc` with eframe's wgpu state.
  pub fn new(render_state: &egui_wgpu::RenderState, doc: &Document) -> Self {
    let renderer = CanvasRenderer::new(&render_state.device, render_state.target_format, (doc.width, doc.height));
    render_state.renderer.write().callback_resources.insert(renderer);
    Self {
      view: None,
      tools: Tools::default(),
      stroke: None,
      last_point: None,
      pen: None,
      gradient: None,
      hold: (0.0, Pos2::ZERO),
      snapped: false,
      assist: (Pos2::ZERO, None),
      loupe: None,
      handle: None,
      shape: None,
      shape_edit: false,
      shape_handle: None,
      shift: false,
    }
  }

  /// A new or opened document: a renderer of its size, the view fitted again. Tools stay.
  pub fn reset(&mut self, render_state: &egui_wgpu::RenderState, doc: &Document) {
    let tools = std::mem::take(&mut self.tools);
    *self = Self { tools, ..Self::new(render_state, doc) };
  }

  /// Draws the canvas filling `ui`, after handling painting, view gestures and shortcuts.
  /// `keys` is false while a text field has the keyboard.
  pub fn show(&mut self, ui: &mut egui::Ui, render_state: &egui_wgpu::RenderState, doc: &mut Document, keys: bool) {
    let (rect, response) = ui.allocate_exact_size(ui.available_size(), egui::Sense::click_and_drag());
    let canvas = vec2(doc.width as f32, doc.height as f32);
    let view = self.view.get_or_insert_with(|| View::fit(rect.size(), canvas));
    handle_input(ui, &response, rect, canvas, view, self.tools.hand);
    let view = *view;
    // Editing a QuickShape ends with any key or a press outside the canvas, before shortcuts
    // (undo, layer changes) touch the document under the open stroke.
    if self.shape_edit
      && ui.input(|i| {
        i.events.iter().any(|e| matches!(e, Event::Key { pressed: true, .. })) || (i.pointer.any_pressed() && !response.contains_pointer())
      })
    {
      self.end_shape(doc);
    }
    let painting = self.stroke.is_some() || self.gradient.is_some();
    if keys {
      ui.input_mut(|input| self.tools.shortcuts(input, doc, painting));
    }
    self.paint(ui, &response, |p| view.to_canvas(rect.center(), canvas, p), doc);

    if let Some(renderer) = render_state.renderer.write().callback_resources.get_mut::<CanvasRenderer>() {
      renderer.sync(&render_state.device, &render_state.queue, doc);
    }
    let placement = view.placement(rect, canvas, ui.ctx().pixels_per_point());
    ui.painter().add(egui_wgpu::Callback::new_paint_callback(rect, CanvasCallback { placement }));
    let to_screen = |p: Pos2| view.to_screen(rect.center(), canvas, p);
    self.tools.guides.draw(&ui.painter_at(rect), canvas, crate::ui::theme::ACCENT.gamma_multiply(0.75), to_screen);
    if let Some((at, before)) = self.loupe {
      loupe(ui.painter(), at, self.tools.primary, before);
    }
    if self.shape_edit
      && let Some((shape, _)) = &self.shape
    {
      let (fill, line) = (crate::ui::theme::ACCENT, egui::Stroke::new(2.0, egui::Color32::WHITE));
      for (x, y) in shape.handles() {
        ui.painter().circle(to_screen(Pos2::new(x, y)), 7.0, fill, line);
      }
    }
    // The gradient's guide line while dragging.
    if let Some((_, start, _, end)) = &self.gradient {
      let (line, shadow) = (egui::Stroke::new(2.0, egui::Color32::WHITE), egui::Color32::from_black_alpha(90));
      ui.painter().line_segment([*start, *end], egui::Stroke::new(4.0, shadow));
      ui.painter().line_segment([*start, *end], line);
      for point in [*start, *end] {
        ui.painter().circle(point, 5.0, shadow, line);
      }
    }
  }

  /// The bucket fills on press; the gradient follows the drag and stays when released.
  fn fill_or_gradient(
    &mut self,
    ui: &egui::Ui,
    response: &egui::Response,
    to_canvas: &impl Fn(Pos2) -> Pos2,
    doc: &mut Document,
    pressed: bool,
    released: bool,
  ) {
    let tools = &mut self.tools;
    if pressed
      && response.hovered()
      && let Some(origin) = ui.input(|i| i.pointer.press_origin())
    {
      let p = to_canvas(origin);
      let on_canvas = p.x >= 0.0 && p.y >= 0.0 && p.x < doc.width as f32 && p.y < doc.height as f32;
      match tools.tool {
        Tool::Fill if on_canvas => {
          if doc.fill(p.x as u32, p.y as u32, tools.primary, tools.fill_threshold, tools.fill_all_layers) {
            tools.used_color();
          }
        }
        Tool::Gradient => {
          let [r, g, b] = tools.primary;
          let [sr, sg, sb] = tools.secondary;
          let end = if tools.gradient_to_clear { [r, g, b, 0] } else { [sr, sg, sb, 255] };
          if let Some(gradient) = doc.begin_gradient(tools.gradient, [[r, g, b, 255], end]) {
            tools.used_color();
            self.gradient = Some((gradient, origin, p, origin));
          }
        }
        _ => {}
      }
    }
    if let Some((gradient, _, from, end)) = &mut self.gradient
      && let Some(now) = ui.input(|i| i.pointer.interact_pos())
      && now != *end
    {
      *end = now;
      let to = to_canvas(now);
      gradient.update(doc, (from.x, from.y), (to.x, to.y));
    }
    if released {
      self.gradient = None;
    }
  }

  /// Turns primary-button presses and drags into strokes. Space and R drags belong to the view.
  fn paint(&mut self, ui: &egui::Ui, response: &egui::Response, to_canvas: impl Fn(Pos2) -> Pos2, doc: &mut Document) {
    let pen = crate::pen::buttons();
    let (pressed, released, shift, alt, view_drag, origin, moves) = ui.input(|i| {
      let moves = samples(&i.events, &mut self.pen);
      (
        i.pointer.primary_pressed(),
        i.pointer.primary_released(),
        i.modifiers.shift,
        i.modifiers.alt,
        i.key_down(Key::Space) || i.key_down(Key::R) || self.tools.hand,
        i.pointer.press_origin(),
        moves,
      )
    });
    let shift_down = shift && !self.shift;
    self.shift = shift;
    if self.shape_edit {
      self.edit_shape(ui, &to_canvas, doc, pressed, released, shift_down, origin);
      return;
    }
    // Editing the guide: drags move its points; nothing paints.
    if self.tools.guides.editing && self.tools.guides.kind != crate::guides::Kind::Off {
      let size = vec2(doc.width as f32, doc.height as f32);
      if pressed && let Some(origin) = origin {
        let at = to_canvas(origin);
        // 18 points around the pointer, in canvas pixels.
        let reach = (to_canvas(origin + vec2(18.0, 0.0)) - at).length();
        self.handle = self.tools.guides.handles(size).iter().position(|h| (*h - at).length() < reach);
      }
      if let Some(index) = self.handle
        && let Some(p) = ui.input(|i| i.pointer.interact_pos())
      {
        self.tools.guides.move_handle(index, to_canvas(p), size);
      }
      if released {
        self.handle = None;
      }
      return;
    }
    // Eyedropper: I, Alt held or the pen's barrel button.
    if (self.tools.picking || alt || pen.barrel) && self.stroke.is_none() && !view_drag {
      if response.is_pointer_button_down_on()
        && let Some(p) = response.interact_pointer_pos()
        && let p = to_canvas(p)
        && p.x >= 0.0
        && p.y >= 0.0
      {
        let before = self.loupe.map_or(self.tools.primary, |(_, before)| before);
        let [r, g, b, a] = doc.pixel(p.x as u32, p.y as u32);
        if a > 0 {
          self.tools.primary = [r, g, b];
        }
        self.loupe = response.interact_pointer_pos().map(|screen| (screen, before));
      }
      if released {
        self.tools.picking = false;
        self.loupe = None;
      }
      return;
    }
    self.loupe = None;
    // ColorDrop: a color dropped from the color button fills like the bucket.
    if let Some(drop) = response.dnd_release_payload::<ColorDrop>()
      && let Some(p) = ui.input(|i| i.pointer.interact_pos()).map(&to_canvas)
      && p.x >= 0.0
      && p.y >= 0.0
      && doc.fill(p.x as u32, p.y as u32, drop.0, self.tools.fill_threshold, self.tools.fill_all_layers)
    {
      self.tools.primary = drop.0;
      self.tools.used_color();
      return;
    }
    if matches!(self.tools.tool, Tool::Fill | Tool::Gradient) && !pen.eraser {
      if !view_drag || self.gradient.is_some() {
        self.fill_or_gradient(ui, response, &to_canvas, doc, pressed, released);
      }
      return;
    }
    if pressed
      && self.stroke.is_none()
      && !view_drag
      && response.hovered()
      && let Some(origin) = origin
    {
      let (_, color, tool) = self.tools.current();
      // Flipping the pen to its eraser end erases with the eraser's brush.
      let tool = if pen.eraser { Tool::Eraser } else { tool };
      let brush = self.tools.brush_for(tool);
      let pressure = moves.first().map_or(self.pen.unwrap_or(1.0), |&(_, pressure)| pressure);
      let stroke = match tool {
        Tool::Smudge => doc.begin_smudge(brush),
        _ => doc.begin_stroke(brush, color, tool == Tool::Eraser),
      };
      let center = self.tools.guides.center(vec2(doc.width as f32, doc.height as f32));
      let center = (center.x, center.y);
      let stroke = stroke.map(|s| match self.tools.guides.symmetry() {
        Some(symmetry) => s.with_symmetry(symmetry, center),
        None => s,
      });
      // A locked layer refuses the stroke.
      if let Some(mut stroke) = stroke {
        if tool == Tool::Brush {
          self.tools.used_color();
        }
        let p = to_canvas(origin);
        match self.last_point {
          Some(from) if shift => stroke.line(doc, from, (p.x, p.y), pressure),
          _ => stroke.add(doc, p.x, p.y, pressure),
        }
        self.stroke = Some(stroke);
        self.hold = (ui.input(|i| i.time), origin);
        self.shape = None;
        self.assist = (p, None);
        self.snapped = false;
      }
    }
    if let Some(stroke) = &mut self.stroke
      && !self.snapped
    {
      let now = ui.input(|i| i.time);
      let pressure = moves.last().map_or(self.pen.unwrap_or(1.0), |&(_, pressure)| pressure);
      for (p, pressure) in moves {
        if p.distance(self.hold.1) > HOLD_SLOP {
          self.hold = (now, p);
        }
        let Some(p) = self.tools.guides.assist(self.assist.0, to_canvas(p), vec2(doc.width as f32, doc.height as f32), &mut self.assist.1) else {
          continue;
        };
        stroke.add(doc, p.x, p.y, pressure);
      }
      // QuickShape: holding still at the end snaps the stroke to the shape it looks like.
      if self.tools.quickshape && self.tools.tool != Tool::Smudge && now - self.hold.0 >= HOLD_SECONDS {
        if let Some(shape) = Shape::recognize(stroke.points()) {
          // Shift held: the perfect version (circle, square, 15° lines).
          let shape = if shift { shape.perfect() } else { shape };
          stroke.redraw(doc, &shape.outline(), pressure);
          self.shape = Some((shape, pressure));
          self.snapped = true;
        }
        // Not a shape: try again after the next hold.
        self.hold.0 = now;
      }
      ui.ctx().request_repaint_after(std::time::Duration::from_millis(100));
    }
    if released && let Some(stroke) = self.stroke.take() {
      if self.shape.is_some() {
        // A snapped shape stays editable until a tap elsewhere or a key.
        self.stroke = Some(stroke);
        self.shape_edit = true;
      } else {
        self.last_point = stroke.finish(doc);
      }
    }
  }

  /// QuickShape editing: drag a handle to reshape, Shift for the perfect shape, a press away
  /// from the handles ends it.
  #[allow(clippy::too_many_arguments)]
  fn edit_shape(
    &mut self,
    ui: &egui::Ui,
    to_canvas: &impl Fn(Pos2) -> Pos2,
    doc: &mut Document,
    pressed: bool,
    released: bool,
    shift_down: bool,
    origin: Option<Pos2>,
  ) {
    let (Some(stroke), Some((shape, pressure))) = (&mut self.stroke, &mut self.shape) else {
      self.shape_edit = false;
      return;
    };
    let mut changed = false;
    if pressed && let Some(origin) = origin {
      let at = to_canvas(origin);
      // 16 points around the pointer, in canvas pixels.
      let reach = (to_canvas(origin + vec2(16.0, 0.0)) - at).length();
      self.shape_handle = shape.handles().iter().position(|&(x, y)| (Pos2::new(x, y) - at).length() < reach);
      if self.shape_handle.is_none() {
        self.end_shape(doc);
        return;
      }
    }
    if shift_down {
      *shape = shape.perfect();
      changed = true;
    }
    if let Some(index) = self.shape_handle
      && let Some(p) = ui.input(|i| i.pointer.interact_pos()).map(to_canvas)
      && shape.handles().get(index).is_some_and(|&(x, y)| (x, y) != (p.x, p.y))
    {
      shape.move_handle(index, (p.x, p.y));
      changed = true;
    }
    if changed {
      stroke.redraw(doc, &shape.outline(), *pressure);
    }
    if released {
      self.shape_handle = None;
    }
  }

  /// Fixes the edited shape: the stroke ends like any other.
  fn end_shape(&mut self, doc: &mut Document) {
    if let Some(stroke) = self.stroke.take() {
      self.last_point = stroke.finish(doc);
    }
    (self.shape, self.shape_edit, self.shape_handle) = (None, false, None);
  }
}

/// This frame's pointer positions with the pressure each was drawn at. A pen sends a touch event
/// with its force just before the pointer move it causes; the mouse has none and paints at full
/// pressure.
fn samples(events: &[Event], pen: &mut Option<f32>) -> Vec<(Pos2, f32)> {
  let mut out = Vec::new();
  for event in events {
    match event {
      Event::Touch { phase, force, .. } => {
        let lifted = matches!(phase, egui::TouchPhase::End | egui::TouchPhase::Cancel);
        *pen = if lifted { None } else { force.filter(|f| *f > 0.0) };
      }
      Event::PointerMoved(p) => out.push((*p, pen.unwrap_or(1.0))),
      _ => {}
    }
  }
  out
}

fn handle_input(ui: &egui::Ui, response: &egui::Response, rect: Rect, canvas: Vec2, view: &mut View, hand: bool) {
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
    } else if space || middle || hand {
      // Hold Space (or the middle button, or the hand tool) and drag: pan.
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
  fn pen_pressure_follows_touch_force() {
    let touch = |phase, force| Event::Touch { device_id: egui::TouchDeviceId(0), id: egui::TouchId(1), phase, pos: Pos2::ZERO, force };
    let mut pen = None;
    let events = [
      Event::PointerMoved(Pos2::new(1.0, 0.0)),
      touch(egui::TouchPhase::Start, Some(0.25)),
      Event::PointerMoved(Pos2::new(2.0, 0.0)),
      touch(egui::TouchPhase::Move, Some(0.75)),
      Event::PointerMoved(Pos2::new(3.0, 0.0)),
    ];
    let pressures: Vec<f32> = samples(&events, &mut pen).into_iter().map(|(_, p)| p).collect();
    assert_eq!(pressures, [1.0, 0.25, 0.75], "mouse first, then the pen's force");
    assert_eq!(pen, Some(0.75), "kept for the next frame");
    samples(&[touch(egui::TouchPhase::End, Some(0.1))], &mut pen);
    assert_eq!(pen, None, "lifting the pen goes back to the mouse");
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
