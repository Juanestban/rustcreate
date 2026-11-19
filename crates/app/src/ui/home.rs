//! The home screen before the canvas (and after File > Close): a new canvas form with size
//! templates, and the recent projects with their previews.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use eframe::egui::{self, Align, Frame, Layout, Margin, RichText, Sense, Stroke, Vec2, vec2};

use super::theme;
use crate::settings::{SavedSize, Settings, Unit};
use crate::{App, Pending};

const CARD: Vec2 = Vec2::new(200.0, 150.0);

/// Suggested sizes: name token, width, height (pixels) and the resolution they are meant at.
const TEMPLATES: [(&str, u32, u32, u32); 11] = [
  ("ui-canvas-square", 2048, 2048, 132),
  ("ui-canvas-hd", 1920, 1080, 72),
  ("ui-canvas-4k", 3840, 2160, 72),
  ("ui-canvas-a4", 2480, 3508, 300),
  ("ui-canvas-letter", 2550, 3300, 300),
  ("ui-canvas-a5", 1748, 2480, 300),
  ("ui-canvas-post", 1080, 1080, 72),
  ("ui-canvas-portrait", 1080, 1350, 72),
  ("ui-canvas-story", 1080, 1920, 72),
  ("ui-canvas-video-thumbnail", 1280, 720, 72),
  ("ui-canvas-banner", 1500, 500, 72),
];

/// The home screen's state: the size being set up and whether there is a canvas to go back to.
pub struct Home {
  pub width: u32,
  pub height: u32,
  /// The canvas is still open behind (File > New): offer to go back to it.
  pub back: bool,
  size_name: String,
  /// Previews of the recent files, read once each time the home screen opens.
  previews: HashMap<PathBuf, Option<egui::TextureHandle>>,
}

impl Home {
  pub fn new(settings: &Settings, back: bool) -> Self {
    Self { width: settings.canvas.0, height: settings.canvas.1, back, size_name: String::new(), previews: HashMap::new() }
  }
}

/// Pinned files first, then the rest by when they were last opened or saved.
pub fn projects(settings: &Settings) -> Vec<(PathBuf, bool)> {
  let pinned = settings.pinned.iter().map(|p| (p.clone(), true));
  let recent = settings.recent.iter().filter(|p| !settings.pinned.contains(p)).map(|p| (p.clone(), false));
  pinned.chain(recent).collect()
}

impl App {
  pub(crate) fn open_home(&mut self, back: bool) {
    self.stop_playback();
    self.popover = None;
    self.home = Some(Home::new(&self.settings, back));
  }

  pub(crate) fn home_screen(&mut self, ui: &mut egui::Ui) {
    let palette = theme::palette(ui);
    egui::CentralPanel::default().frame(Frame::NONE.fill(palette.chrome).inner_margin(Margin::symmetric(32, 24))).show(ui, |ui| {
      ui.horizontal(|ui| {
        ui.heading(RichText::new("rustcreate").strong().size(26.0));
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
          if self.home.as_ref().is_some_and(|h| h.back) && ui.button(self.i18n.tr("ui-home-back")).clicked() {
            self.home = None;
          }
          if ui.button(self.i18n.tr("ui-file-open")).clicked() {
            self.request(Pending::Open(None));
          }
        });
      });
      ui.add_space(16.0);
      ui.horizontal_top(|ui| {
        // Outlined, not filled: fields and buttons keep their own fill.
        Frame::NONE.stroke(Stroke::new(1.0, palette.line)).corner_radius(12).inner_margin(Margin::same(18)).show(ui, |ui| {
          ui.vertical(|ui| {
            ui.set_width(380.0);
            self.new_canvas_form(ui);
          });
        });
        ui.add_space(24.0);
        ui.vertical(|ui| {
          ui.set_width(ui.available_width());
          self.recent_projects(ui);
        });
      });
    });
    if self.home.is_some() && !ui.ctx().egui_wants_keyboard_input() && self.confirm.is_none() {
      if ui.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::O)) {
        self.request(Pending::Open(None));
      } else if self.home.as_ref().is_some_and(|h| h.back) && ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
        self.home = None;
      }
    }
  }

  fn new_canvas_form(&mut self, ui: &mut egui::Ui) {
    let Some(home) = &mut self.home else { return };
    let (i18n, max) = (&self.i18n, self.render_state.device.limits().max_texture_dimension_2d.min(rustcreate_core::MAX_SIDE));
    let settings = &mut self.settings;
    let before = settings.clone();
    let muted = theme::palette(ui).muted;
    ui.label(RichText::new(i18n.tr("ui-home-new")).strong().size(18.0));
    ui.add_space(8.0);
    // Templates and the user's own sizes, as chips.
    ui.label(RichText::new(i18n.tr("ui-home-templates")).color(muted));
    let mut remove = None;
    ui.horizontal_wrapped(|ui| {
      ui.spacing_mut().item_spacing = vec2(6.0, 6.0);
      let screen = ui
        .ctx()
        .input(|i| i.viewport().monitor_size)
        .map(|s| ((s.x * ui.ctx().pixels_per_point()) as u32, (s.y * ui.ctx().pixels_per_point()) as u32));
      let screen = screen.filter(|&(w, h)| w > 0 && h > 0).map(|(w, h)| (i18n.tr("ui-canvas-screen"), w, h, 72));
      let templates = TEMPLATES.iter().map(|&(id, w, h, dpi)| (i18n.tr(id), w, h, dpi));
      for (name, w, h, dpi) in screen.into_iter().chain(templates) {
        let chosen = (home.width, home.height) == (w, h);
        if ui.selectable_label(chosen, name).on_hover_text(format!("{w} × {h} px · {dpi} dpi")).clicked() {
          (home.width, home.height, settings.dpi) = (w, h, dpi);
        }
      }
      for (n, size) in settings.sizes.iter().enumerate() {
        let chosen = (home.width, home.height) == (size.width, size.height);
        let response =
          ui.selectable_label(chosen, size.name.clone()).on_hover_text(format!("{} × {} px · {} dpi", size.width, size.height, size.dpi));
        if response.clicked() {
          (home.width, home.height) = (size.width, size.height);
          settings.dpi = size.dpi;
        }
        response.context_menu(|ui| {
          if ui.button(i18n.tr("ui-home-size-remove")).clicked() {
            remove = Some(n);
            ui.close();
          }
        });
      }
    });
    if let Some(n) = remove {
      settings.sizes.remove(n);
    }
    ui.add_space(12.0);
    // Size in the chosen unit; pixels underneath.
    egui::Grid::new("new-canvas-size").num_columns(2).spacing(vec2(10.0, 8.0)).show(ui, |ui| {
      let unit = settings.unit;
      let dpi = settings.dpi;
      let decimals = if unit == Unit::Pixels { 0 } else { 2 };
      for (label, pixels) in [(i18n.tr("ui-canvas-width"), &mut home.width), (i18n.tr("ui-canvas-height"), &mut home.height)] {
        ui.label(label);
        let mut value = unit.in_units(*pixels, dpi);
        if ui.add(egui::DragValue::new(&mut value).range(0.01..=1e6).fixed_decimals(decimals).speed(if decimals == 0 { 1.0 } else { 0.05 })).changed()
        {
          *pixels = unit.to_pixels(value, dpi).clamp(1, max);
        }
        ui.end_row();
      }
      ui.label(i18n.tr("ui-home-unit"));
      ui.horizontal(|ui| {
        egui::ComboBox::from_id_salt("unit").selected_text(i18n.tr(unit_token(unit))).show_ui(ui, |ui| {
          for u in Unit::ALL {
            ui.selectable_value(&mut settings.unit, u, i18n.tr(unit_token(u)));
          }
        });
        if ui.button(i18n.tr("ui-home-swap")).on_hover_text(i18n.tr("ui-canvas-swap")).clicked() {
          std::mem::swap(&mut home.width, &mut home.height);
        }
      });
      ui.end_row();
      ui.label(i18n.tr("ui-home-dpi"));
      ui.add(egui::DragValue::new(&mut settings.dpi).range(36..=1200).suffix(" dpi"));
      ui.end_row();
      ui.label(i18n.tr("ui-home-background"));
      ui.horizontal(|ui| {
        let mut colored = settings.background.is_some();
        ui.selectable_value(&mut colored, false, i18n.tr("ui-home-transparent"));
        ui.selectable_value(&mut colored, true, i18n.tr("ui-home-color"));
        match (colored, &mut settings.background) {
          (true, None) => settings.background = Some([255, 255, 255]),
          (false, Some(_)) => settings.background = None,
          _ => {}
        }
        if let Some(rgb) = &mut settings.background {
          ui.color_edit_button_srgb(rgb);
        }
      });
      ui.end_row();
    });
    ui.add_space(6.0);
    let too_big = home.width.max(home.height) > max;
    let summary = format!("{} × {} px", home.width, home.height);
    ui.label(RichText::new(summary).strong());
    ui.label(RichText::new(i18n.tr_args("ui-canvas-max", &[("max", max.into())])).small().color(if too_big {
      ui.visuals().error_fg_color
    } else {
      muted
    }));
    ui.add_space(8.0);
    // Save the size for later, with a name.
    ui.horizontal(|ui| {
      ui.add(egui::TextEdit::singleline(&mut home.size_name).hint_text(i18n.tr("ui-home-size-name")).desired_width(200.0));
      let name = home.size_name.trim().to_string();
      if ui.add_enabled(!name.is_empty(), egui::Button::new(i18n.tr("ui-home-size-save"))).clicked() {
        settings.sizes.retain(|s| s.name != name);
        settings.sizes.push(SavedSize { name, width: home.width, height: home.height, dpi: settings.dpi });
        home.size_name.clear();
      }
    });
    ui.add_space(12.0);
    let button = egui::Button::new(RichText::new(i18n.tr("ui-canvas-create")).strong().color(egui::Color32::WHITE)).fill(theme::ACCENT);
    let create = ui.add_enabled(!too_big, button.min_size(vec2(ui.available_width(), 38.0))).clicked()
      || (!too_big && !ui.ctx().egui_wants_keyboard_input() && ui.input(|i| i.key_pressed(egui::Key::Enter)));
    let (width, height, background) = (home.width, home.height, settings.background);
    let changed = *settings != before;
    if changed || create {
      let settings = Settings { canvas: (width, height), ..self.settings.clone() };
      self.apply(settings);
    }
    if create {
      self.request(Pending::New(width, height, background));
    }
  }

  fn recent_projects(&mut self, ui: &mut egui::Ui) {
    let palette = theme::palette(ui);
    ui.label(RichText::new(self.i18n.tr("ui-home-recent")).strong().size(18.0));
    ui.add_space(8.0);
    let projects = projects(&self.settings);
    if projects.is_empty() {
      ui.label(RichText::new(self.i18n.tr("ui-home-recent-none")).color(palette.muted));
      return;
    }
    let mut action: Option<(PathBuf, ProjectAction)> = None;
    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
      ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = vec2(16.0, 16.0);
        for (path, pinned) in projects {
          let preview = self.preview(ui.ctx(), &path);
          let (rect, response) = ui.allocate_exact_size(CARD + vec2(0.0, 30.0), Sense::click());
          let picture = egui::Rect::from_min_size(rect.min, CARD);
          ui.painter().rect_filled(picture, 10, palette.control);
          match &preview {
            Some(texture) => {
              let size = texture.size_vec2();
              let fit = ((CARD.x - 16.0) / size.x).min((CARD.y - 16.0) / size.y);
              let image = egui::Rect::from_center_size(picture.center(), size * fit);
              // A light checkerboard stands in for transparency: plain light gray.
              ui.painter().rect_filled(image, 0, egui::Color32::from_gray(225));
              ui.painter().image(texture.id(), image, egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), egui::Color32::WHITE);
            }
            None => {
              ui.painter().text(
                picture.center(),
                egui::Align2::CENTER_CENTER,
                self.i18n.tr("ui-home-missing"),
                egui::FontId::proportional(13.0),
                palette.muted,
              );
            }
          }
          let edge = if response.hovered() { Stroke::new(2.0, theme::ACCENT) } else { Stroke::new(1.0, palette.line) };
          if pinned {
            // Pinned: an accent dot in the corner.
            ui.painter().circle_filled(picture.right_top() + vec2(-12.0, 12.0), 5.0, theme::ACCENT);
          }
          ui.painter().rect_stroke(picture, 10, edge, egui::StrokeKind::Outside);
          let name = path.file_stem().map_or_else(|| path.display().to_string(), |n| n.to_string_lossy().into_owned());
          let label = egui::pos2(rect.left() + 2.0, picture.bottom() + 8.0);
          ui.painter().text(label, egui::Align2::LEFT_TOP, name, egui::FontId::proportional(14.0), palette.ink);
          let response = response.on_hover_text(path.display().to_string()).on_hover_cursor(egui::CursorIcon::PointingHand);
          if response.clicked() {
            action = Some((path.clone(), ProjectAction::Open));
          }
          response.context_menu(|ui| {
            let pin = if pinned { "ui-home-unpin" } else { "ui-home-pin" };
            if ui.button(self.i18n.tr(pin)).clicked() {
              action = Some((path.clone(), ProjectAction::Pin(!pinned)));
              ui.close();
            }
            if ui.button(self.i18n.tr("ui-file-recent-remove")).clicked() {
              action = Some((path.clone(), ProjectAction::Remove));
              ui.close();
            }
          });
        }
      });
    });
    match action {
      Some((path, ProjectAction::Open)) => self.request(Pending::Open(Some(path))),
      Some((path, ProjectAction::Pin(pin))) => {
        let mut settings = self.settings.clone();
        settings.pinned.retain(|p| *p != path);
        if pin {
          settings.pinned.insert(0, path);
        }
        self.apply(settings);
      }
      Some((path, ProjectAction::Remove)) => self.forget(&path),
      None => {}
    }
  }

  /// A recent file's saved preview; `None` if it is gone or unreadable.
  fn preview(&mut self, ctx: &egui::Context, path: &Path) -> Option<egui::TextureHandle> {
    let home = self.home.as_mut()?;
    home
      .previews
      .entry(path.to_path_buf())
      .or_insert_with(|| {
        let (w, h, pixels) = rustcreate_core::Document::read_thumbnail(path).ok()?;
        let image = egui::ColorImage::from_rgba_unmultiplied([w as usize, h as usize], &pixels);
        Some(ctx.load_texture(format!("preview-{}", path.display()), image, egui::TextureOptions::LINEAR))
      })
      .clone()
  }
}

enum ProjectAction {
  Open,
  Pin(bool),
  Remove,
}

fn unit_token(unit: Unit) -> &'static str {
  match unit {
    Unit::Pixels => "ui-unit-px",
    Unit::Inches => "ui-unit-in",
    Unit::Centimeters => "ui-unit-cm",
    Unit::Millimeters => "ui-unit-mm",
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn pinned_projects_come_first_once() {
    let settings = Settings { recent: vec!["a".into(), "b".into(), "c".into()], pinned: vec!["c".into(), "z".into()], ..Settings::default() };
    let order: Vec<_> = projects(&settings).into_iter().map(|(p, pinned)| (p.display().to_string(), pinned)).collect();
    assert_eq!(order, [("c", true), ("z", true), ("a", false), ("b", false)].map(|(p, b)| (p.to_string(), b)));
  }
}
