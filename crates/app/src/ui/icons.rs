//! rustcreate's own minimalist line icons (ADR 0006), painted with vectors on a 24 px grid with
//! 1.6 px rounded strokes, so they follow the theme color and stay crisp at any scale.

use std::f32::consts::{PI, TAU};

use eframe::egui::{self, Color32, Painter, Pos2, Rect, Response, Sense, Shape, Stroke, StrokeKind, Ui, Vec2, vec2};

use super::theme;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Icon {
  Selection,
  Transform,
  Brush,
  Smudge,
  Eraser,
  Bucket,
  Gradient,
  Layers,
  Eyedropper,
  Undo,
  Redo,
  Plus,
  Eye,
  EyeOff,
  Lock,
  Trash,
  Duplicate,
  MergeDown,
  Text,
  Play,
  Pause,
}

fn arc(center: Pos2, radius: f32, a0: f32, a1: f32) -> Vec<Pos2> {
  (0..=20).map(|i| center + radius * Vec2::angled(a0 + (a1 - a0) * i as f32 / 20.0)).collect()
}

pub fn paint(p: &Painter, rect: Rect, icon: Icon, color: Color32) {
  let s = rect.width() / 24.0;
  let at = |x: f32, y: f32| rect.min + vec2(x * s, y * s);
  let stroke = Stroke::new(1.6 * s.max(0.7), color);
  let line = |points: Vec<Pos2>| p.add(Shape::line(points, stroke));
  let seg = |a: (f32, f32), b: (f32, f32)| p.line_segment([at(a.0, a.1), at(b.0, b.1)], stroke);
  let rounded = |min: (f32, f32), max: (f32, f32), r: f32| {
    p.rect_stroke(Rect::from_min_max(at(min.0, min.1), at(max.0, max.1)), r * s, stroke, StrokeKind::Middle)
  };

  match icon {
    Icon::Selection => {
      // A dashed lasso loop.
      let points = arc(at(12.0, 10.5), 7.5 * s, 0.0, TAU);
      for pair in points.chunks(2) {
        if let [a, b] = pair {
          p.line_segment([*a, *b], stroke);
        }
      }
      seg((9.0, 17.0), (7.5, 21.0));
    }
    Icon::Transform => {
      // An arrow cursor.
      let points = vec![at(6.0, 3.5), at(6.0, 19.0), at(10.0, 15.0), at(13.0, 21.0), at(15.5, 20.0), at(12.5, 14.0), at(18.0, 14.0), at(6.0, 3.5)];
      line(points);
    }
    Icon::Brush => {
      // A brush head (no handle, like the app icon) and its stroke.
      let tip = vec![at(20.0, 4.0), at(11.0, 13.0), at(13.0, 15.0), at(20.0, 4.0)];
      line(tip);
      line(vec![at(11.0, 13.0), at(7.5, 14.5), at(4.0, 20.0), at(9.5, 16.5), at(13.0, 15.0)]);
    }
    Icon::Smudge => {
      // A fingertip.
      line(arc(at(12.0, 9.0), 4.5 * s, PI, TAU));
      seg((7.5, 9.0), (7.5, 18.0));
      seg((16.5, 9.0), (16.5, 18.0));
      line(arc(at(12.0, 18.0), 4.5 * s, 0.0, PI));
    }
    Icon::Eraser => {
      let body = vec![at(14.0, 4.0), at(21.0, 11.0), at(11.0, 21.0), at(6.0, 21.0), at(3.0, 18.0), at(14.0, 4.0)];
      line(body);
      seg((8.5, 10.5), (15.5, 17.5));
      seg((11.0, 21.0), (21.0, 21.0));
    }
    Icon::Bucket => {
      // A tipped pail and the drop falling from its lip.
      line(vec![at(9.0, 3.5), at(17.5, 12.0), at(11.0, 18.5), at(2.5, 10.0), at(9.0, 3.5)]);
      seg((4.5, 8.0), (14.5, 9.0));
      line(vec![at(17.5, 12.0), at(19.5, 15.5), at(19.5, 17.5)]);
      line(arc(at(19.5, 19.0), 1.6 * s, 0.0, TAU));
    }
    Icon::Gradient => {
      // A square with hatching that thins out from left to right.
      rounded((3.5, 3.5), (20.5, 20.5), 3.0);
      for (x, dy) in [(6.5, 0.0), (9.0, 0.0), (11.5, 3.0), (14.5, 6.0), (17.5, 9.0)] {
        seg((x, 6.5 + dy), (x, 17.5 - dy));
      }
    }
    Icon::Layers => {
      line(vec![at(12.0, 3.5), at(21.0, 8.0), at(12.0, 12.5), at(3.0, 8.0), at(12.0, 3.5)]);
      line(vec![at(3.0, 12.5), at(12.0, 17.0), at(21.0, 12.5)]);
      line(vec![at(3.0, 16.5), at(12.0, 21.0), at(21.0, 16.5)]);
    }
    Icon::Eyedropper => {
      line(arc(at(17.0, 7.0), 3.0 * s, 0.0, TAU));
      line(vec![at(14.5, 9.5), at(5.0, 19.0), at(5.0, 19.5), at(4.0, 21.0), at(5.5, 20.0), at(6.0, 20.0), at(15.5, 10.5)]);
      seg((12.0, 7.0), (18.0, 13.0));
    }
    Icon::Undo | Icon::Redo => {
      let flip = |x: f32| if icon == Icon::Undo { x } else { 24.0 - x };
      let c = at(flip(13.0), 14.0);
      let curve = if icon == Icon::Undo { arc(c, 6.0 * s, -PI / 2.0, PI / 2.0) } else { arc(c, 6.0 * s, PI * 1.5, PI / 2.0) };
      line(curve);
      seg((flip(13.0), 8.0), (flip(5.0), 8.0));
      line(vec![at(flip(8.5), 4.5), at(flip(5.0), 8.0), at(flip(8.5), 11.5)]);
      seg((flip(13.0), 20.0), (flip(8.0), 20.0));
    }
    Icon::Plus => {
      seg((12.0, 5.0), (12.0, 19.0));
      seg((5.0, 12.0), (19.0, 12.0));
    }
    Icon::Eye | Icon::EyeOff => {
      line(arc(at(12.0, 19.0), 10.5 * s, PI * 1.2, PI * 1.8));
      line(arc(at(12.0, 5.0), 10.5 * s, PI * 0.2, PI * 0.8));
      p.circle_stroke(at(12.0, 12.0), 2.8 * s, stroke);
      if icon == Icon::EyeOff {
        seg((4.0, 4.0), (20.0, 20.0));
      }
    }
    Icon::Lock => {
      rounded((5.0, 11.0), (19.0, 21.0), 2.0);
      line(arc(at(12.0, 8.5), 4.0 * s, PI, TAU));
      seg((8.0, 8.5), (8.0, 11.0));
      seg((16.0, 8.5), (16.0, 11.0));
    }
    Icon::Trash => {
      seg((4.0, 7.0), (20.0, 7.0));
      line(vec![at(9.0, 7.0), at(9.0, 4.0), at(15.0, 4.0), at(15.0, 7.0)]);
      line(vec![at(6.0, 7.0), at(7.0, 20.0), at(17.0, 20.0), at(18.0, 7.0)]);
    }
    Icon::Duplicate => {
      rounded((8.0, 8.0), (20.0, 20.0), 2.5);
      line(vec![at(16.0, 8.0), at(16.0, 4.0), at(4.0, 4.0), at(4.0, 16.0), at(8.0, 16.0)]);
    }
    Icon::MergeDown => {
      seg((12.0, 3.0), (12.0, 14.0));
      line(vec![at(7.5, 9.5), at(12.0, 14.0), at(16.5, 9.5)]);
      seg((4.0, 19.5), (20.0, 19.5));
    }
    Icon::Play => {
      line(vec![at(8.0, 5.0), at(19.0, 12.0), at(8.0, 19.0), at(8.0, 5.0)]);
    }
    Icon::Pause => {
      seg((8.5, 5.5), (8.5, 18.5));
      seg((15.5, 5.5), (15.5, 18.5));
    }
    Icon::Text => {
      // A capital T with turned-down ends and a foot.
      line(vec![at(5.0, 7.5), at(5.0, 4.5), at(19.0, 4.5), at(19.0, 7.5)]);
      seg((12.0, 4.5), (12.0, 19.5));
      seg((9.0, 19.5), (15.0, 19.5));
    }
  }
}

/// A square icon button. `active` draws it in the accent color (the current tool or open panel).
pub fn button(ui: &mut Ui, icon: Icon, active: bool, enabled: bool, tooltip: &str) -> Response {
  let size = 32.0;
  let (rect, response) = ui.allocate_exact_size(Vec2::splat(size), if enabled { Sense::click() } else { Sense::hover() });
  let palette = theme::palette(ui);
  if enabled && response.hovered() {
    ui.painter().rect_filled(rect, 8.0, palette.control);
  }
  let color = match (enabled, active) {
    (false, _) => palette.muted.gamma_multiply(0.5),
    (true, true) => theme::ACCENT,
    (true, false) => palette.ink,
  };
  paint(ui.painter(), Rect::from_center_size(rect.center(), Vec2::splat(22.0)), icon, color);
  response.on_hover_text(tooltip).on_hover_cursor(if enabled { egui::CursorIcon::PointingHand } else { egui::CursorIcon::Default })
}
