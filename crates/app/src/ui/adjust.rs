//! The Adjustments menu and its dialogs: Hue/Saturation, Brightness/Contrast, Levels and
//! Curves preview live on the canvas; Invert and Desaturate apply at once.

use eframe::egui::{self, Align2, Color32, Pos2, Sense, Vec2, vec2};
use rustcreate_core::{Adjust, Adjustment};

use crate::App;

/// An open adjustment dialog.
pub(crate) struct Adjusting {
  live: Adjust,
  settings: Adjustment,
  /// What the canvas shows now, to redraw only on a change.
  shown: Adjustment,
  /// The Curves point being dragged.
  drag: Option<usize>,
}

/// The menu: name token and the settings it starts from (no change).
pub(crate) fn menu() -> [(&'static str, Adjustment); 6] {
  [
    ("ui-adjust-hue-saturation", Adjustment::HueSaturation { hue: 0.0, saturation: 0.0, lightness: 0.0 }),
    ("ui-adjust-brightness-contrast", Adjustment::BrightnessContrast { brightness: 0.0, contrast: 0.0 }),
    ("ui-adjust-levels", Adjustment::Levels { black: 0.0, gamma: 1.0, white: 1.0 }),
    ("ui-adjust-curves", Adjustment::Curves(vec![(0.0, 0.0), (1.0, 1.0)])),
    ("ui-adjust-invert", Adjustment::Invert),
    ("ui-adjust-desaturate", Adjustment::Desaturate),
  ]
}

fn same_kind(a: &Adjustment, b: &Adjustment) -> bool {
  std::mem::discriminant(a) == std::mem::discriminant(b)
}

impl App {
  pub(crate) fn adjust_menu(&mut self, ui: &mut egui::Ui) {
    ui.set_width(220.0);
    for (id, settings) in menu() {
      if ui.add_sized([ui.available_width(), 28.0], egui::Button::new(self.i18n.tr(id))).clicked() {
        self.start_adjust(settings);
      }
    }
  }

  /// Opens an adjustment's dialog, or applies it at once if it has no settings.
  pub(crate) fn start_adjust(&mut self, settings: Adjustment) {
    self.canvas.end_transform(&mut self.doc, true);
    self.adjusting = None;
    self.popover = None;
    if matches!(settings, Adjustment::Invert | Adjustment::Desaturate) {
      self.doc.adjust(&settings);
    } else if let Some(live) = self.doc.begin_adjust() {
      self.adjusting = Some(Adjusting { live, shown: settings.clone(), settings, drag: None });
    }
  }

  // shortcut: every change recomputes the whole layer on the CPU cores (Hue/Saturation on a
  // full 4096² layer is the slowest); preview on the GPU if dragging lags.
  pub(crate) fn adjust_dialog(&mut self, ctx: &egui::Context) {
    let Some(adjusting) = &mut self.adjusting else { return };
    let i18n = &self.i18n;
    let title = menu().into_iter().find(|(_, s)| same_kind(s, &adjusting.settings)).map_or("ui-adjustments", |(id, _)| id);
    let mut done = None;
    // No backdrop, off to the side: the canvas stays visible while it previews.
    let area = egui::Modal::default_area(egui::Id::unique("adjust")).anchor(Align2::RIGHT_TOP, vec2(-72.0, 72.0));
    egui::Modal::new(egui::Id::unique("adjust")).area(area).backdrop_color(Color32::TRANSPARENT).show(ctx, |ui| {
      ui.set_width(280.0);
      ui.heading(i18n.tr(title));
      ui.add_space(6.0);
      let slider = |ui: &mut egui::Ui, id: &str, value: &mut f32, range: std::ops::RangeInclusive<f32>| {
        ui.label(i18n.tr(id));
        ui.add(egui::Slider::new(value, range).fixed_decimals(2));
      };
      match &mut adjusting.settings {
        Adjustment::HueSaturation { hue, saturation, lightness } => {
          ui.label(i18n.tr("ui-adjust-hue"));
          ui.add(egui::Slider::new(hue, -180.0..=180.0).suffix("°").fixed_decimals(0));
          slider(ui, "ui-adjust-saturation", saturation, -1.0..=1.0);
          slider(ui, "ui-adjust-lightness", lightness, -1.0..=1.0);
        }
        Adjustment::BrightnessContrast { brightness, contrast } => {
          slider(ui, "ui-adjust-brightness", brightness, -1.0..=1.0);
          slider(ui, "ui-adjust-contrast", contrast, -1.0..=1.0);
        }
        Adjustment::Levels { black, gamma, white } => {
          slider(ui, "ui-adjust-black", black, 0.0..=1.0);
          ui.label(i18n.tr("ui-adjust-gamma"));
          ui.add(egui::Slider::new(gamma, 0.1..=10.0).logarithmic(true).fixed_decimals(2));
          slider(ui, "ui-adjust-white", white, 0.0..=1.0);
          *black = black.min(*white - 0.01);
        }
        Adjustment::Curves(points) => {
          curves(ui, points, &mut adjusting.drag);
          ui.label(egui::RichText::new(i18n.tr("ui-adjust-curves-help")).small());
        }
        Adjustment::Invert | Adjustment::Desaturate => {}
      }
      ui.add_space(8.0);
      ui.horizontal(|ui| {
        if ui.button(i18n.tr("ui-adjust-reset")).clicked() {
          adjusting.settings = menu().into_iter().find(|(_, s)| same_kind(s, &adjusting.settings)).map(|(_, s)| s).expect("listed");
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
          if ui.button(i18n.tr("ui-adjust-apply")).clicked() {
            done = Some(true);
          }
          if ui.button(i18n.tr("ui-cancel")).clicked() {
            done = Some(false);
          }
        });
      });
    });
    // Clicking outside does nothing (it is easy to do by mistake); keys decide.
    ctx.input_mut(|i| {
      if i.consume_key(egui::Modifiers::NONE, egui::Key::Escape) {
        done = Some(false);
      } else if i.consume_key(egui::Modifiers::NONE, egui::Key::Enter) {
        done = done.or(Some(true));
      }
    });
    if adjusting.settings != adjusting.shown {
      adjusting.live.update(&mut self.doc, &adjusting.settings);
      adjusting.shown = adjusting.settings.clone();
    }
    match done {
      Some(true) => self.adjusting = None,
      Some(false) => self.adjusting.take().expect("open").live.cancel(&mut self.doc),
      None => {}
    }
  }
}

/// The Curves editor: drag a point, press anywhere else to add one, drag a point out of the
/// box to remove it (the curve keeps at least two).
fn curves(ui: &mut egui::Ui, points: &mut Vec<(f32, f32)>, drag: &mut Option<usize>) {
  const SIZE: f32 = 256.0;
  let (rect, response) = ui.allocate_exact_size(Vec2::splat(SIZE), Sense::click_and_drag());
  let to_screen = |(x, y): (f32, f32)| Pos2::new(rect.left() + x * SIZE, rect.bottom() - y * SIZE);
  let from_screen = |p: Pos2| (((p.x - rect.left()) / SIZE).clamp(0.0, 1.0), ((rect.bottom() - p.y) / SIZE).clamp(0.0, 1.0));
  if (response.drag_started() || response.clicked())
    && let Some(at) = response.interact_pointer_pos()
  {
    let near = points.iter().position(|&p| to_screen(p).distance(at) < 10.0);
    *drag = Some(near.unwrap_or_else(|| {
      points.push(from_screen(at));
      points.len() - 1
    }));
  }
  if response.dragged()
    && let (Some(i), Some(at)) = (*drag, response.interact_pointer_pos())
  {
    if !rect.expand(24.0).contains(at) && points.len() > 2 {
      points.remove(i);
      *drag = None;
    } else {
      points[i] = from_screen(at);
    }
  }
  if response.drag_stopped() || response.clicked() {
    *drag = None;
  }

  let visuals = ui.visuals();
  let (ink, faint) = (visuals.strong_text_color(), visuals.weak_text_color().gamma_multiply(0.4));
  let painter = ui.painter_at(rect.expand(6.0));
  painter.rect_filled(rect, 4.0, visuals.extreme_bg_color);
  for i in 1..4 {
    let t = i as f32 / 4.0;
    painter.line_segment([to_screen((t, 0.0)), to_screen((t, 1.0))], (1.0, faint));
    painter.line_segment([to_screen((0.0, t)), to_screen((1.0, t))], (1.0, faint));
  }
  painter.line_segment([to_screen((0.0, 0.0)), to_screen((1.0, 1.0))], (1.0, faint));
  let curve = rustcreate_core::curve(points);
  let line: Vec<Pos2> = (0..=64).map(|i| i as f32 / 64.0).map(|x| to_screen((x, curve(x)))).collect();
  painter.add(egui::Shape::line(line, (2.0, ink)));
  for &p in points.iter() {
    painter.circle(to_screen(p), 5.0, visuals.panel_fill, (2.0, ink));
  }
}
