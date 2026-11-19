//! The canvas area: view transform (zoom, pan, rotate), painting with the pointer, their
//! gestures and shortcuts, and the paint callback that hands drawing to `rustcreate_render`.

use eframe::egui::{self, Event, Key, Modifiers, Pos2, Rect, Vec2, vec2};
use eframe::egui_wgpu::{self, wgpu};
use rustcreate_core::{Affine, Document, Gradient, SelectMode, SelectShape, Selection, Shape, Stroke, Text, Transform, text_bounds};

/// QuickShape: how long the pointer must rest, and how far (points) it may drift meanwhile.
const HOLD_SECONDS: f64 = 0.6;
const HOLD_SLOP: f32 = 4.0;
use rustcreate_render::{CanvasRenderer, Placement};

use crate::tools::{SelectKind, Tool, Tools};

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
  /// ColorDrop held still over the canvas: where it filled (canvas pixels), the pointer's x then
  /// (sliding sideways changes the threshold) and the threshold of the fill on screen.
  drop: Option<(Pos2, f32, f32)>,
  /// The selection being drawn: how it combines (from Shift/Alt at the press) and its points
  /// in canvas pixels (the corners for a rectangle or ellipse, the outline for the lasso).
  selecting: Option<(SelectMode, Vec<Pos2>)>,
  /// The selection's outline for the marching ants, in canvas pixels, and the mask it was
  /// traced from (by address), so it is only traced again when the selection changes.
  ants: (usize, Vec<[Pos2; 2]>),
  /// Free Transform in progress (Ctrl+T).
  transform: Option<Free>,
  /// The text layer being typed into, with the Text tool.
  text: Option<Typing>,
  /// The system's font families, listed the first time the Text tool is used.
  fonts: Option<Vec<String>>,
}

struct Typing {
  layer: u64,
  /// Added by this edit: finishing it empty removes it without an undo step.
  new: bool,
  /// A drag moving the text: where it started (canvas pixels) and the text's position then.
  drag: Option<(Pos2, (f32, f32))>,
  /// The editor takes the keyboard on its next frame.
  focus: bool,
}

/// What a Free Transform drag holds.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Grab {
  Move,
  Rotate,
  /// A box handle: -1, 0 or 1 along each axis (0 = the middle of a side).
  Handle(f32, f32),
}

/// A Free Transform: the lifted pixels and how they are moved, scaled and turned around the
/// center of what was lifted.
pub(crate) struct Free {
  lifted: Transform,
  center: Pos2,
  half: Vec2,
  offset: Vec2,
  /// Negative to flip.
  scale: Vec2,
  angle: f32,
  /// The drag: what it holds, where it started (canvas) and the state then.
  drag: Option<(Grab, Pos2, (Vec2, Vec2, f32))>,
}

impl Free {
  fn matrix(&self) -> Affine {
    let (sin, cos) = self.angle.sin_cos();
    let (a, b, c, d) = (cos * self.scale.x, -sin * self.scale.y, sin * self.scale.x, cos * self.scale.y);
    let to = self.center + self.offset;
    [a, b, c, d, to.x - (a * self.center.x + b * self.center.y), to.y - (c * self.center.x + d * self.center.y)]
  }

  /// A point of the box, in units of its half size from the center (-1..=1), on the canvas.
  fn point(&self, hx: f32, hy: f32) -> Pos2 {
    let local = vec2(hx * self.half.x * self.scale.x, hy * self.half.y * self.scale.y);
    self.center + self.offset + rotate(local, self.angle)
  }

  /// The pointer at canvas point `p` relative to the box, unturned and unscaled.
  fn local(&self, p: Pos2) -> Vec2 {
    rotate(p - (self.center + self.offset), -self.angle)
  }

  /// What a press at canvas point `p` takes hold of; `reach` is how near a handle counts.
  fn grab_at(&self, p: Pos2, reach: f32) -> Grab {
    let handle = Self::HANDLES.iter().find(|&&(hx, hy)| self.point(hx, hy).distance(p) < reach);
    let local = self.local(p);
    let inside = local.x.abs() <= (self.half.x * self.scale.x).abs() && local.y.abs() <= (self.half.y * self.scale.y).abs();
    match handle {
      Some(&(hx, hy)) => Grab::Handle(hx, hy),
      None if inside => Grab::Move,
      None => Grab::Rotate,
    }
  }

  const HANDLES: [(f32, f32); 8] = [(-1.0, -1.0), (0.0, -1.0), (1.0, -1.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0), (-1.0, 1.0), (-1.0, 0.0)];
}

/// The cursor over a Free Transform for `grab` at screen point `at`.
fn transform_cursor(ui: &egui::Ui, at: Pos2, grab: Grab, free: &Free, to_canvas: &impl Fn(Pos2) -> Pos2, dragging: bool) {
  use egui::CursorIcon;
  let icon = match grab {
    Grab::Move if dragging => CursorIcon::Grabbing,
    Grab::Move => CursorIcon::Move,
    Grab::Handle(hx, hy) => {
      // The handle's direction on screen (the box and the view may both be turned): canvas
      // vectors are mapped back through the screen's unit steps.
      let (c0, cx, cy) = (to_canvas(at), to_canvas(at + vec2(1.0, 0.0)), to_canvas(at + vec2(0.0, 1.0)));
      let (ex, ey) = (cx - c0, cy - c0);
      let d = free.point(hx, hy) - free.point(0.0, 0.0);
      let det = ex.x * ey.y - ex.y * ey.x;
      let screen = vec2(d.x * ey.y - d.y * ey.x, ex.x * d.y - ex.y * d.x) / det.abs().max(1e-6) * det.signum();
      let degrees = screen.y.atan2(screen.x).to_degrees().rem_euclid(180.0);
      match degrees {
        d if !(22.5..157.5).contains(&d) => CursorIcon::ResizeHorizontal,
        d if d < 67.5 => CursorIcon::ResizeNwSe,
        d if d < 112.5 => CursorIcon::ResizeVertical,
        _ => CursorIcon::ResizeNeSw,
      }
    }
    Grab::Rotate => {
      // No system cursor turns: a curved arrow by the pointer says it.
      let painter = ui.ctx().layer_painter(egui::LayerId::new(egui::Order::Tooltip, egui::Id::unique("rotate-cursor")));
      let center = at + vec2(16.0, 16.0);
      let arc: Vec<Pos2> = (0..=20).map(|i| center + 8.0 * Vec2::angled((-200.0 + i as f32 * 11.0).to_radians())).collect();
      let tip = *arc.last().expect("arc points");
      let back = (tip - arc[arc.len() - 3]).normalized();
      let side = vec2(-back.y, back.x);
      for (width, color) in [(4.0, egui::Color32::from_black_alpha(160)), (2.0, egui::Color32::WHITE)] {
        painter.add(egui::Shape::line(arc.clone(), egui::Stroke::new(width, color)));
        painter.add(egui::Shape::line(vec![tip - back * 5.0 + side * 4.0, tip, tip - back * 5.0 - side * 4.0], egui::Stroke::new(width, color)));
      }
      CursorIcon::Crosshair
    }
  };
  ui.ctx().set_cursor_icon(icon);
}

/// Where the layer with id `id` is in the stack.
fn layer_index(doc: &Document, id: u64) -> Option<usize> {
  doc.layers().iter().position(|l| l.id == id)
}

fn rotate(v: Vec2, angle: f32) -> Vec2 {
  let (sin, cos) = angle.sin_cos();
  vec2(v.x * cos - v.y * sin, v.x * sin + v.y * cos)
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
      drop: None,
      selecting: None,
      ants: (0, Vec::new()),
      transform: None,
      text: None,
      fonts: None,
    }
  }

  /// A new or opened document: a renderer of its size, the view fitted again. Tools stay.
  /// Where the canvas sits on screen, kept per document tab.
  pub fn view(&self) -> Option<View> {
    self.view
  }

  pub fn set_view(&mut self, view: Option<View>) {
    self.view = view;
  }

  pub fn reset(&mut self, render_state: &egui_wgpu::RenderState, doc: &Document) {
    let tools = std::mem::take(&mut self.tools);
    *self = Self { tools, ..Self::new(render_state, doc) };
  }

  /// Draws the canvas filling `ui`, after handling painting, view gestures and shortcuts.
  /// `keys` is false while a text field has the keyboard.
  pub fn show(&mut self, ui: &mut egui::Ui, render_state: &egui_wgpu::RenderState, doc: &mut Document, keys: bool, i18n: &crate::i18n::I18n) {
    // Crop, Canvas Size, Image Size (or undoing one) changed the size: a renderer of the new
    // size, and the view fitted to it.
    let size = (doc.width, doc.height);
    if render_state.renderer.read().callback_resources.get::<CanvasRenderer>().is_some_and(|r| r.size() != size) {
      let renderer = CanvasRenderer::new(&render_state.device, render_state.target_format, size);
      render_state.renderer.write().callback_resources.insert(renderer);
      self.view = None;
    }
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
    if self.transform.is_some() && keys {
      // While transforming only these keys count: the document is in the middle of a change.
      let (enter, escape) = ui.input_mut(|i| {
        let enter = i.consume_key(Modifiers::NONE, Key::Enter) || i.consume_key(Modifiers::COMMAND, Key::T);
        (enter, i.consume_key(Modifiers::NONE, Key::Escape) || i.consume_key(Modifiers::COMMAND, Key::Z))
      });
      if enter {
        self.end_transform(doc, true);
      } else if escape {
        self.end_transform(doc, false);
      }
    } else if keys && ui.input_mut(|i| i.consume_key(Modifiers::COMMAND, Key::T)) && self.stroke.is_none() && self.gradient.is_none() {
      self.start_transform(doc);
    }
    if self.tools.tool != Tool::Text || self.transform.is_some() {
      self.end_text(doc);
    }
    let painting = self.stroke.is_some() || self.gradient.is_some() || self.transform.is_some();
    if keys {
      ui.input_mut(|input| self.tools.shortcuts(input, doc, painting));
    }
    self.paint(ui, &response, |p| view.to_canvas(rect.center(), canvas, p), doc);

    if let Some(renderer) = render_state.renderer.write().callback_resources.get_mut::<CanvasRenderer>() {
      // A Free Transform drags on the GPU: its pixels go up once, then only the map changes.
      match &self.transform {
        Some(free) => {
          if renderer.floating_layer() != Some(free.lifted.layer_id()) {
            let (origin, size) = free.lifted.area();
            renderer.begin_floating(&render_state.device, &render_state.queue, free.lifted.layer_id(), &free.lifted.pixels(), origin, size);
          }
          renderer.move_floating(&render_state.queue, free.matrix());
        }
        None => renderer.end_floating(),
      }
      renderer.sync(&render_state.device, &render_state.queue, doc);
    }
    let placement = view.placement(rect, canvas, ui.ctx().pixels_per_point());
    ui.painter().add(egui_wgpu::Callback::new_paint_callback(rect, CanvasCallback { placement }));
    let to_screen = |p: Pos2| view.to_screen(rect.center(), canvas, p);
    self.tools.guides.draw(&ui.painter_at(rect), canvas, crate::ui::theme::ACCENT.gamma_multiply(0.75), to_screen);
    self.draw_selection(ui, rect, doc, to_screen);
    self.draw_transform(ui, rect, doc, to_screen, i18n);
    self.draw_text(ui, rect, doc, to_screen, i18n);
    if let Some((at, before)) = self.loupe {
      loupe(ui.painter(), at, self.tools.primary, before);
    }
    // Only where the canvas is on top (not under a popover or the Studio).
    self.outline(ui, rect, view, response.contains_pointer());
    if let (Some((_, _, threshold)), Some(at)) = (self.drop, ui.input(|i| i.pointer.interact_pos())) {
      let text = format!("{:.0} %", threshold * 100.0);
      ui.painter().text(at + vec2(0.0, -28.0), egui::Align2::CENTER_BOTTOM, text, egui::FontId::proportional(15.0), egui::Color32::WHITE);
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

  /// Text tool presses: on the text being typed a drag moves it; on another text layer it
  /// starts editing that one; elsewhere on the canvas it adds a new text layer there.
  fn text_input(
    &mut self,
    ui: &egui::Ui,
    response: &egui::Response,
    to_canvas: &impl Fn(Pos2) -> Pos2,
    doc: &mut Document,
    (pressed, released): (bool, bool),
    origin: Option<Pos2>,
  ) {
    let inside = |text: &Text, p: Pos2| text_bounds(text).is_some_and(|(x0, y0, x1, y1)| (x0..=x1).contains(&p.x) && (y0..=y1).contains(&p.y));
    let typing = self.text.as_ref().and_then(|t| layer_index(doc, t.layer));
    if pressed
      && response.contains_pointer()
      && let Some(origin) = origin
    {
      let p = to_canvas(origin);
      if let (Some(typing), Some(index)) = (&mut self.text, typing)
        && let Some(text) = doc.layers()[index].text().filter(|t| inside(t, p))
      {
        typing.drag = Some((p, text.position));
      } else {
        self.end_text(doc);
        let hit = (0..doc.layers().len()).rev().find(|&i| {
          let layer = &doc.layers()[i];
          !layer.hidden && !layer.locked && layer.text().is_some_and(|t| inside(t, p))
        });
        let on_canvas = p.x >= 0.0 && p.y >= 0.0 && p.x < doc.width as f32 && p.y < doc.height as f32;
        if let Some(index) = hit {
          doc.select(index);
          doc.checkpoint();
          // Its color becomes the current one, so editing does not recolor it.
          self.tools.primary = doc.layers()[index].text().expect("hit a text").color;
          self.text = Some(Typing { layer: doc.layers()[index].id, new: false, drag: None, focus: true });
        } else if on_canvas {
          let (font, size, color) = (self.tools.text_font.clone(), self.tools.text_size, self.tools.primary);
          doc.add_text_layer(Text { text: String::new(), font, size, color, position: (p.x, p.y) });
          self.text = Some(Typing { layer: doc.layers()[doc.active()].id, new: true, drag: None, focus: true });
        }
      }
    }
    if let Some(typing) = &self.text
      && let Some((start, from)) = typing.drag
      && let Some(index) = layer_index(doc, typing.layer)
      && let Some(now) = ui.input(|i| i.pointer.interact_pos())
      && let Some(text) = doc.layers()[index].text()
    {
      let moved = to_canvas(now) - start;
      let position = (from.0 + moved.x, from.1 + moved.y);
      if text.position != position {
        let text = Text { position, ..text.clone() };
        doc.set_text(index, text);
      }
    }
    if released && let Some(typing) = &mut self.text {
      typing.drag = None;
    }
    // A text cursor over the canvas, a move cursor over the text being typed.
    if let Some(at) = ui.input(|i| i.pointer.hover_pos()).filter(|_| response.contains_pointer()) {
      let over = typing.and_then(|i| doc.layers()[i].text()).is_some_and(|t| inside(t, to_canvas(at)));
      ui.ctx().set_cursor_icon(if over { egui::CursorIcon::Move } else { egui::CursorIcon::Text });
    }
  }

  /// Stops typing: an empty text layer goes away (one added just now leaves no undo step).
  pub fn end_text(&mut self, doc: &mut Document) {
    let Some(typing) = self.text.take() else { return };
    let Some(index) = layer_index(doc, typing.layer) else { return };
    if doc.layers()[index].text().is_some_and(|t| t.text.trim().is_empty()) {
      if typing.new {
        doc.rollback();
      } else {
        doc.delete_layer(index);
      }
    }
  }

  /// The box around the text being typed and its editor: the text, font, size and Done. The
  /// text takes the current color, so picking a color while typing recolors it.
  fn draw_text(&mut self, ui: &egui::Ui, rect: Rect, doc: &mut Document, to_screen: impl Fn(Pos2) -> Pos2, i18n: &crate::i18n::I18n) {
    let Some(typing) = &mut self.text else { return };
    // Undo can take the layer, or its text, away.
    let Some(index) = layer_index(doc, typing.layer).filter(|&i| doc.layers()[i].text().is_some()) else {
      self.text = None;
      return;
    };
    let mut text = doc.layers()[index].text().expect("a text layer").clone();
    let (x0, y0, x1, y1) = text_bounds(&text).unwrap_or((text.position.0, text.position.1, text.position.0, text.position.1));
    let corners: Vec<Pos2> = [(x0, y0), (x1, y0), (x1, y1), (x0, y1), (x0, y0)]
      .iter()
      .map(|&(x, y)| to_screen(Pos2::new(x - 4.0, y) + vec2(if x == x1 { 8.0 } else { 0.0 }, 0.0)))
      .collect();
    let painter = ui.painter_at(rect);
    painter.add(egui::Shape::line(corners.clone(), egui::Stroke::new(3.0, egui::Color32::from_black_alpha(90))));
    painter.add(egui::Shape::dashed_line(&corners, egui::Stroke::new(1.5, crate::ui::theme::ACCENT), 6.0, 4.0));
    let below = corners[2].max(corners[3]).max(corners[0]).max(corners[1]);
    let fonts = self.fonts.get_or_insert_with(rustcreate_core::font_families);
    let mut done = false;
    egui::Area::new(egui::Id::unique("text-editor"))
      .order(egui::Order::Foreground)
      .fixed_pos(Pos2::new(to_screen(Pos2::new(x0, y1)).x.min(below.x).max(rect.left() + 8.0), below.y + 12.0))
      .constrain_to(rect)
      .show(ui.ctx(), |ui| {
        egui::Frame::popup(ui.style()).show(ui, |ui| {
          ui.set_width(300.0);
          let edit =
            ui.add(egui::TextEdit::multiline(&mut text.text).desired_rows(3).desired_width(f32::INFINITY).hint_text(i18n.tr("ui-text-hint")));
          if std::mem::take(&mut typing.focus) {
            edit.request_focus();
          }
          if edit.has_focus() && ui.input(|i| i.modifiers.command && i.key_pressed(Key::Enter)) {
            done = true;
          }
          if ui.input(|i| i.key_pressed(Key::Escape)) {
            done = true;
          }
          ui.horizontal(|ui| {
            ui.label(i18n.tr("ui-text-font"));
            egui::ComboBox::from_id_salt("text-font").selected_text(&text.font).width(150.0).height(320.0).show_ui(ui, |ui| {
              for family in fonts.iter() {
                ui.selectable_value(&mut text.font, family.clone(), egui::RichText::new(family));
              }
            });
          });
          ui.horizontal(|ui| {
            ui.label(i18n.tr("ui-text-size"));
            ui.add(egui::DragValue::new(&mut text.size).range(4.0..=2000.0).speed(0.5).suffix(" px"));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
              if ui.button(i18n.tr("ui-text-done")).clicked() {
                done = true;
              }
            });
          });
        });
      });
    text.color = self.tools.primary;
    if doc.layers()[index].text() != Some(&text) {
      (self.tools.text_font, self.tools.text_size) = (text.font.clone(), text.size);
      doc.set_text(index, text);
    }
    if done {
      self.end_text(doc);
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
    if self.transform.is_some() {
      // A press on the transform bar (above the canvas) is not a drag.
      self.transform_input(ui, &to_canvas, (pressed && response.contains_pointer(), released), shift, origin, view_drag);
      return;
    }
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
    if self.tools.tool == Tool::Text {
      if !view_drag {
        self.text_input(ui, response, &to_canvas, doc, (pressed, released), origin);
      }
      return;
    }
    // Selections: Alt subtracts here instead of picking a color.
    if self.tools.tool == Tool::Select {
      if !view_drag || self.selecting.is_some() {
        self.select_input(ui, &to_canvas, doc, (pressed, released), (shift, alt), origin);
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
    // ColorDrop: a color dropped from the color button fills like the bucket. Held still over
    // the canvas it fills right away, and sliding sideways redoes the fill with a higher or lower
    // threshold (one undo step); releasing keeps it.
    if let Some(drop) = response.dnd_release_payload::<ColorDrop>() {
      let filled = match self.drop.take() {
        Some((_, _, threshold)) => {
          self.tools.fill_threshold = threshold;
          true
        }
        None => ui.input(|i| i.pointer.interact_pos()).map(&to_canvas).is_some_and(|p| {
          p.x >= 0.0 && p.y >= 0.0 && doc.fill(p.x as u32, p.y as u32, drop.0, self.tools.fill_threshold, self.tools.fill_all_layers)
        }),
      };
      if filled {
        self.tools.primary = drop.0;
        self.tools.used_color();
      }
      return;
    }
    if let Some(drop) = response.dnd_hover_payload::<ColorDrop>()
      && let Some(screen) = ui.input(|i| i.pointer.interact_pos())
    {
      let all_layers = self.tools.fill_all_layers;
      match &mut self.drop {
        Some((at, start_x, threshold)) => {
          let wanted = (self.tools.fill_threshold + (screen.x - *start_x) / 400.0).clamp(0.0, 1.0);
          if (wanted - *threshold).abs() >= 0.01 && doc.undo() {
            *threshold = wanted;
            doc.fill(at.x as u32, at.y as u32, drop.0, wanted, all_layers);
          }
        }
        None => {
          let now = ui.input(|i| i.time);
          let at = to_canvas(screen);
          if screen.distance(self.hold.1) > HOLD_SLOP {
            self.hold = (now, screen);
          } else if now - self.hold.0 >= HOLD_SECONDS {
            self.hold.0 = now;
            if at.x >= 0.0 && at.y >= 0.0 && doc.fill(at.x as u32, at.y as u32, drop.0, self.tools.fill_threshold, all_layers) {
              self.drop = Some((at, screen.x, self.tools.fill_threshold));
            }
          }
          ui.ctx().request_repaint_after(std::time::Duration::from_millis(100));
        }
      }
      return;
    }
    // The drag left the canvas or ended elsewhere: the fill stays as it is.
    self.drop = None;
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
        stroke.set_tilt(pen.tilt);
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
      // shortcut: one tilt per frame (the latest), not per coalesced sample; sample it in the
      // subclass with the pointer id if strokes show steps.
      stroke.set_tilt(if self.pen.is_some() { pen.tilt } else { 0.0 });
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

  /// Starts a Free Transform of the active layer's selection (or all of it). Nothing happens on a
  /// locked or empty layer.
  pub fn start_transform(&mut self, doc: &mut Document) {
    if self.transform.is_some() || self.stroke.is_some() || self.gradient.is_some() {
      return;
    }
    let Some(lifted) = doc.begin_transform() else { return };
    let (l, t, r, b) = lifted.bounds();
    let (center, half) = (Pos2::new((l + r) / 2.0, (t + b) / 2.0), vec2((r - l) / 2.0, (b - t) / 2.0));
    self.transform = Some(Free { lifted, center, half, offset: Vec2::ZERO, scale: Vec2::splat(1.0), angle: 0.0, drag: None });
  }

  pub fn transforming(&self) -> bool {
    self.transform.is_some()
  }

  /// Applies the transform, or puts everything back.
  pub fn end_transform(&mut self, doc: &mut Document, apply: bool) {
    let Some(free) = self.transform.take() else { return };
    if apply {
      let m = free.matrix();
      free.lifted.finish(doc, m);
    } else {
      free.lifted.cancel(doc);
    }
  }

  /// Flips, quarter turns and reset from the transform bar.
  fn adjust_transform(&mut self, change: impl FnOnce(&mut Free)) {
    if let Some(free) = &mut self.transform {
      change(free);
    }
  }

  /// Free Transform drags: inside the box moves, a handle scales (corners keep the proportions
  /// unless Shift is held), outside the box turns (Shift: 15° steps).
  #[allow(clippy::too_many_arguments)]
  fn transform_input(
    &mut self,
    ui: &egui::Ui,
    to_canvas: &impl Fn(Pos2) -> Pos2,
    (pressed, released): (bool, bool),
    shift: bool,
    origin: Option<Pos2>,
    view_drag: bool,
  ) {
    let Some(free) = &mut self.transform else { return };
    // 10 points around the pointer, in canvas pixels.
    let reach = |at: Pos2| (to_canvas(at + vec2(10.0, 0.0)) - to_canvas(at)).length();
    if pressed
      && !view_drag
      && let Some(origin) = origin
    {
      let p = to_canvas(origin);
      free.drag = Some((free.grab_at(p, reach(origin)), p, (free.offset, free.scale, free.angle)));
    }
    // The pointer shows what a drag does there: move, scale along a handle, or turn.
    if let Some(at) = ui.input(|i| i.pointer.hover_pos()).filter(|_| ui.ui_contains_pointer() && !view_drag) {
      let grab = free.drag.map_or_else(|| free.grab_at(to_canvas(at), reach(at)), |(grab, ..)| grab);
      transform_cursor(ui, at, grab, free, to_canvas, free.drag.is_some());
    }
    let Some((grab, start, (offset, scale, angle))) = free.drag else { return };
    let Some(p) = ui.input(|i| i.pointer.interact_pos()).map(to_canvas) else { return };
    match grab {
      Grab::Move => free.offset = offset + (p - start),
      Grab::Rotate => {
        let pivot = free.center + offset;
        let turned = angle + (p - pivot).angle() - (start - pivot).angle();
        let step = 15f32.to_radians();
        free.angle = if shift { (turned / step).round() * step } else { turned };
      }
      Grab::Handle(hx, hy) => {
        let local = rotate(p - (free.center + offset), -angle);
        let corner = vec2(hx * free.half.x * scale.x, hy * free.half.y * scale.y);
        free.scale = if hx != 0.0 && hy != 0.0 && !shift {
          // Proportional: how far along its diagonal the corner was pulled.
          scale * (local.dot(corner) / corner.length_sq().max(1e-6))
        } else {
          let axis = |h: f32, half: f32, now: f32, old: f32| if h == 0.0 || half <= 0.0 { old } else { now / (h * half) };
          vec2(axis(hx, free.half.x, local.x, scale.x), axis(hy, free.half.y, local.y, scale.y))
        };
      }
    }
    // The GPU previews the new map (see `show`); the pixels are drawn once, on Done.
    if released {
      free.drag = None;
    }
  }

  /// The transform box, its handles and the bar with flips, turns, reset, cancel and done.
  fn draw_transform(&mut self, ui: &egui::Ui, rect: Rect, doc: &mut Document, to_screen: impl Fn(Pos2) -> Pos2, i18n: &crate::i18n::I18n) {
    let Some(free) = &self.transform else { return };
    let painter = ui.painter_at(rect);
    let corners: Vec<Pos2> = [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)].iter().map(|&(x, y)| to_screen(free.point(x, y))).collect();
    painter.add(egui::Shape::closed_line(corners, egui::Stroke::new(1.5, crate::ui::theme::ACCENT)));
    for (hx, hy) in Free::HANDLES {
      let at = to_screen(free.point(hx, hy));
      painter.rect(
        Rect::from_center_size(at, Vec2::splat(9.0)),
        2.0,
        egui::Color32::WHITE,
        egui::Stroke::new(1.5, crate::ui::theme::ACCENT),
        egui::StrokeKind::Middle,
      );
    }
    let mut action = None;
    // Above the floating panels: the transform waits on Done or Cancel, so they stay in reach.
    egui::Area::new(egui::Id::unique("transform-bar"))
      .order(egui::Order::Tooltip)
      .pivot(egui::Align2::CENTER_BOTTOM)
      .fixed_pos(rect.center_bottom() - vec2(0.0, 16.0))
      .show(ui.ctx(), |ui| {
        egui::Frame::popup(ui.style()).show(ui, |ui| {
          ui.horizontal(|ui| {
            for (id, a) in [
              ("ui-transform-flip-h", 0),
              ("ui-transform-flip-v", 1),
              ("ui-transform-rotate", 2),
              ("ui-transform-reset", 3),
              ("ui-transform-cancel", 4),
              ("ui-transform-done", 5),
            ] {
              if ui.button(i18n.tr(id)).clicked() {
                action = Some(a);
              }
            }
          });
        });
      });
    match action {
      Some(0) => self.adjust_transform(|f| f.scale.x = -f.scale.x),
      Some(1) => self.adjust_transform(|f| f.scale.y = -f.scale.y),
      Some(2) => self.adjust_transform(|f| f.angle += std::f32::consts::FRAC_PI_2),
      Some(3) => self.adjust_transform(|f| (f.offset, f.scale, f.angle) = (Vec2::ZERO, Vec2::splat(1.0), 0.0)),
      Some(4) => self.end_transform(doc, false),
      Some(5) => self.end_transform(doc, true),
      _ => {}
    }
  }

  /// The selection tool: a drag draws a rectangle, ellipse or lasso, a click with the wand picks
  /// an area; Shift adds and Alt subtracts. A click without a drag deselects.
  fn select_input(
    &mut self,
    ui: &egui::Ui,
    to_canvas: &impl Fn(Pos2) -> Pos2,
    doc: &mut Document,
    (pressed, released): (bool, bool),
    (shift, alt): (bool, bool),
    origin: Option<Pos2>,
  ) {
    let kind = self.tools.select_kind;
    let mode = if shift {
      SelectMode::Add
    } else if alt {
      SelectMode::Subtract
    } else {
      SelectMode::Replace
    };
    if pressed && let Some(origin) = origin {
      let p = to_canvas(origin);
      if kind == SelectKind::Wand {
        if p.x >= 0.0 && p.y >= 0.0 {
          doc.select_wand(p.x as u32, p.y as u32, self.tools.fill_threshold, self.tools.fill_all_layers, mode);
        }
        return;
      }
      self.selecting = Some((mode, vec![p]));
    }
    let Some((_, points)) = &mut self.selecting else { return };
    if let Some(p) = ui.input(|i| i.pointer.interact_pos()).map(to_canvas) {
      if kind == SelectKind::Lasso {
        if points.last().is_none_or(|last| last.distance(p) >= 1.0) {
          points.push(p);
        }
      } else {
        points.truncate(1);
        points.push(p);
      }
    }
    if !released {
      return;
    }
    let Some((mode, points)) = self.selecting.take() else { return };
    let (a, b) = (points[0], *points.last().expect("starts with the press"));
    let shape = match kind {
      SelectKind::Lasso if points.len() >= 3 => Some(SelectShape::Polygon(points.iter().map(|p| (p.x, p.y)).collect())),
      SelectKind::Rectangle if (b.x - a.x).abs() >= 1.0 && (b.y - a.y).abs() >= 1.0 => Some(SelectShape::Rectangle((a.x, a.y), (b.x, b.y))),
      SelectKind::Ellipse if (b.x - a.x).abs() >= 1.0 && (b.y - a.y).abs() >= 1.0 => Some(SelectShape::Ellipse((a.x, a.y), (b.x, b.y))),
      _ => None,
    };
    match shape {
      Some(shape) => doc.select_shape(&shape, mode),
      None if mode == SelectMode::Replace => {
        doc.deselect();
      }
      None => {}
    }
  }

  /// Marching ants around the selection, and the shape being drawn.
  fn draw_selection(&mut self, ui: &egui::Ui, rect: Rect, doc: &Document, to_screen: impl Fn(Pos2) -> Pos2) {
    let painter = ui.painter_at(rect);
    let dark = egui::Stroke::new(1.0, egui::Color32::from_black_alpha(200));
    let light = egui::Stroke::new(1.0, egui::Color32::WHITE);
    if let Some((_, points)) = &self.selecting {
      let outline: Vec<Pos2> = match self.tools.select_kind {
        SelectKind::Lasso => points.clone(),
        kind => {
          let (a, b) = (points[0], *points.last().expect("starts with the press"));
          if kind == SelectKind::Ellipse {
            let (center, radius) = (a.lerp(b, 0.5), (b - a).abs() / 2.0);
            (0..64).map(|i| center + Vec2::angled(i as f32 / 64.0 * std::f32::consts::TAU) * radius).collect()
          } else {
            vec![a, Pos2::new(b.x, a.y), b, Pos2::new(a.x, b.y)]
          }
        }
      };
      let screen: Vec<Pos2> = outline.into_iter().map(&to_screen).collect();
      painter.add(egui::Shape::closed_line(screen.clone(), dark));
      painter.add(egui::Shape::dashed_line(&[screen.as_slice(), &screen[..1]].concat(), light, 4.0, 4.0));
    }
    let Some(selection) = doc.selection() else {
      self.ants = (0, Vec::new());
      return;
    };
    let key = selection.data().as_ptr() as usize;
    if self.ants.0 != key {
      self.ants = (key, trace(selection));
    }
    // Dashes that march: the offset moves with time.
    let offset = (ui.input(|i| i.time) * 12.0 % 8.0) as f32;
    for [a, b] in &self.ants.1 {
      let (a, b) = (to_screen(*a), to_screen(*b));
      painter.line_segment([a, b], dark);
      painter.extend(egui::Shape::dashed_line_with_offset(&[a, b], light, &[4.0], &[4.0], offset));
    }
    ui.ctx().request_repaint_after(std::time::Duration::from_millis(120));
  }

  /// The brush outline under the pointer: where and how big the brush (or the eraser, also when
  /// the pen is flipped) paints at full pressure, turned like the tip. Tiny brushes get a cross.
  fn outline(&self, ui: &egui::Ui, rect: egui::Rect, view: View, over: bool) {
    let pen = crate::pen::buttons();
    // Not inside `ui.input`: that holds the context's lock, which `has_any_payload` takes again;
    // the first frame then never finished and the window (shown after it) never appeared.
    let alt = ui.input(|i| i.modifiers.alt);
    let dragging_color = egui::DragAndDrop::has_any_payload(ui.ctx());
    let tool = if pen.eraser { Tool::Eraser } else { self.tools.tool };
    let busy = self.transform.is_some()
      || self.tools.picking
      || self.tools.hand
      || alt
      || pen.barrel
      || self.tools.guides.editing
      || self.shape_edit
      || dragging_color;
    let Some(at) = ui.input(|i| i.pointer.hover_pos()).filter(|&p| rect.contains(p)) else { return };
    if tool == Tool::Select && over && !busy {
      ui.ctx().set_cursor_icon(egui::CursorIcon::Crosshair);
    }
    if busy || !matches!(tool, Tool::Brush | Tool::Smudge | Tool::Eraser) || !over {
      return;
    }
    let brush = self.tools.brush_for(tool);
    let radius = brush.size / 2.0 * view.zoom;
    let painter = ui.painter_at(rect);
    let (dark, light) = (egui::Stroke::new(3.0, egui::Color32::from_black_alpha(150)), egui::Stroke::new(1.0, egui::Color32::WHITE));
    ui.ctx().set_cursor_icon(egui::CursorIcon::None);
    if radius < 4.0 {
      for d in [vec2(1.0, 0.0), vec2(0.0, 1.0)] {
        for stroke in [dark, light] {
          painter.line_segment([at - d * 6.0, at + d * 6.0], stroke);
        }
      }
      return;
    }
    // A tip that follows the stroke has no fixed angle to show.
    let angle = if brush.follow { 0.0 } else { brush.angle.to_radians() } + view.rotation;
    let squash = brush.roundness.clamp(0.05, 1.0);
    let (sin, cos) = angle.sin_cos();
    let points: Vec<Pos2> = (0..64)
      .map(|i| {
        let t = i as f32 / 64.0 * std::f32::consts::TAU;
        let (x, y) = (t.cos() * radius, t.sin() * radius * squash);
        at + vec2(x * cos - y * sin, x * sin + y * cos)
      })
      .collect();
    for stroke in [dark, light] {
      painter.add(egui::Shape::closed_line(points.clone(), stroke));
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

/// The selection's edge as straight runs in canvas pixels, traced on a grid of at most 1024
/// cells a side (a cell is inside when its corner pixel is at least half selected).
fn trace(selection: &Selection) -> Vec<[Pos2; 2]> {
  let (w, h) = (selection.width(), selection.height());
  let step = w.max(h).div_ceil(1024).max(1);
  let (gw, gh) = (w.div_ceil(step), h.div_ceil(step));
  let inside = |cx: i64, cy: i64| cx >= 0 && cy >= 0 && cx < gw as i64 && cy < gh as i64 && selection.at(cx as u32 * step, cy as u32 * step) >= 128;
  let at = |x: i64, y: i64| Pos2::new((x as f32 * step as f32).min(w as f32), (y as f32 * step as f32).min(h as f32));
  let mut runs = Vec::new();
  // Vertical edges between columns cx-1 and cx, merged along y; then horizontal ones.
  for cx in 0..=gw as i64 {
    let mut start = None;
    for cy in 0..=gh as i64 {
      let edge = cy < gh as i64 && inside(cx - 1, cy) != inside(cx, cy);
      match (edge, start) {
        (true, None) => start = Some(cy),
        (false, Some(from)) => {
          runs.push([at(cx, from), at(cx, cy)]);
          start = None;
        }
        _ => {}
      }
    }
  }
  for cy in 0..=gh as i64 {
    let mut start = None;
    for cx in 0..=gw as i64 {
      let edge = cx < gw as i64 && inside(cx, cy - 1) != inside(cx, cy);
      match (edge, start) {
        (true, None) => start = Some(cx),
        (false, Some(from)) => {
          runs.push([at(from, cy), at(cx, cy)]);
          start = None;
        }
        _ => {}
      }
    }
  }
  runs
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn a_rectangle_traces_to_four_edges() {
    let mut doc = Document::new(100, 80);
    doc.select_shape(&SelectShape::Rectangle((10.0, 20.0), (60.0, 50.0)), SelectMode::Replace);
    let mut runs = trace(doc.selection().unwrap());
    runs.sort_by(|a, b| (a[0].x, a[0].y).partial_cmp(&(b[0].x, b[0].y)).unwrap());
    assert_eq!(runs.len(), 4, "{runs:?}");
    assert!(runs.contains(&[Pos2::new(10.0, 20.0), Pos2::new(10.0, 50.0)]), "left edge {runs:?}");
    assert!(runs.contains(&[Pos2::new(10.0, 20.0), Pos2::new(60.0, 20.0)]), "top edge {runs:?}");
  }

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
