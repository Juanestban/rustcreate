//! The Procreate-like layout: a thin top bar (gallery, actions, adjustments, selection,
//! transform on the left; brush, smudge, eraser, layers and color on the right), a left sidebar
//! with the size and opacity sliders, the eyedropper and undo/redo, and the floating panels the
//! top bar opens.

use eframe::egui::{self, Align, Align2, Color32, Frame, Layout, Margin, Pos2, Rect, RichText, Sense, Stroke, Vec2, vec2};
use rustcreate_core::BlendMode;

use super::icons::{self, Icon};
use super::theme;
use crate::tools::BRUSHES;
use crate::{App, Pending};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Popover {
  Actions,
  Brushes,
  Layers,
  Color,
}

const BLEND_MODES: [(BlendMode, &str); 9] = [
  (BlendMode::Normal, "ui-blend-normal"),
  (BlendMode::Multiply, "ui-blend-multiply"),
  (BlendMode::Screen, "ui-blend-screen"),
  (BlendMode::Overlay, "ui-blend-overlay"),
  (BlendMode::Darken, "ui-blend-darken"),
  (BlendMode::Lighten, "ui-blend-lighten"),
  (BlendMode::Add, "ui-blend-add"),
  (BlendMode::ColorDodge, "ui-blend-color-dodge"),
  (BlendMode::ColorBurn, "ui-blend-color-burn"),
];

impl App {
  pub(crate) fn top_bar(&mut self, ui: &mut egui::Ui) {
    let palette = theme::palette(ui);
    egui::Panel::top("top-bar").exact_size(theme::BAR_HEIGHT).frame(Frame::NONE.fill(palette.chrome).inner_margin(Margin::symmetric(10, 6))).show(
      ui,
      |ui| {
        ui.horizontal_centered(|ui| {
          ui.spacing_mut().item_spacing.x = 4.0;
          let soon = self.i18n.tr("ui-coming-soon");
          icons::button(ui, Icon::Gallery, false, false, &format!("{} · {soon}", self.i18n.tr("ui-gallery")));
          self.popover_button(ui, Icon::Actions, Popover::Actions, "ui-actions");
          ui.add_space(8.0);
          icons::button(ui, Icon::Adjustments, false, false, &format!("{} · {soon}", self.i18n.tr("ui-adjustments")));
          icons::button(ui, Icon::Selection, false, false, &format!("{} · {soon}", self.i18n.tr("ui-selection")));
          icons::button(ui, Icon::Transform, false, false, &format!("{} · {soon}", self.i18n.tr("ui-transform")));

          ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            self.color_button(ui);
            self.popover_button(ui, Icon::Layers, Popover::Layers, "ui-layers");
            let tools = &mut self.canvas.tools;
            if icons::button(ui, Icon::Eraser, tools.erasing, true, &self.i18n.tr("ui-tooltip-eraser")).clicked() {
              if tools.erasing {
                self.toggle(Popover::Brushes);
              }
              self.canvas.tools.erasing = true;
            }
            icons::button(ui, Icon::Smudge, false, false, &format!("{} · {soon}", self.i18n.tr("ui-smudge")));
            let erasing = self.canvas.tools.erasing;
            if icons::button(ui, Icon::Brush, !erasing || self.popover == Some(Popover::Brushes), true, &self.i18n.tr("ui-tooltip-brush")).clicked() {
              if !erasing {
                self.toggle(Popover::Brushes);
              }
              self.canvas.tools.erasing = false;
            }
          });
        });
      },
    );
  }

  fn toggle(&mut self, popover: Popover) {
    self.popover = if self.popover == Some(popover) { None } else { Some(popover) };
  }

  fn popover_button(&mut self, ui: &mut egui::Ui, icon: Icon, popover: Popover, tooltip: &str) {
    if icons::button(ui, icon, self.popover == Some(popover), true, &self.i18n.tr(tooltip)).clicked() {
      self.toggle(popover);
    }
  }

  /// The current color as a swatch ringed with the icon's gradient.
  fn color_button(&mut self, ui: &mut egui::Ui) {
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(32.0), Sense::click());
    let center = rect.center();
    let ring: Vec<Pos2> = (0..=48).map(|i| center + 12.0 * Vec2::angled(i as f32 / 48.0 * std::f32::consts::TAU)).collect();
    for (i, pair) in ring.windows(2).enumerate() {
      let t = i as f32 / 48.0 * (theme::GRADIENT.len() - 1) as f32;
      let (a, b) = (theme::GRADIENT[t as usize], theme::GRADIENT[(t as usize + 1).min(theme::GRADIENT.len() - 1)]);
      ui.painter().line_segment([pair[0], pair[1]], Stroke::new(2.5, a.lerp_to_gamma(b, t.fract())));
    }
    let [r, g, b] = self.canvas.tools.primary;
    ui.painter().circle_filled(center, 9.0, Color32::from_rgb(r, g, b));
    if response.on_hover_text(self.i18n.tr("ui-color")).clicked() {
      self.toggle(Popover::Color);
    }
  }

  pub(crate) fn sidebar(&mut self, ui: &mut egui::Ui) {
    let palette = theme::palette(ui);
    egui::Panel::left("sidebar")
      .exact_size(theme::SIDEBAR_WIDTH)
      .resizable(false)
      .frame(Frame::NONE.fill(palette.chrome).inner_margin(Margin::symmetric(0, 16)))
      .show(ui, |ui| {
        ui.vertical_centered(|ui| {
          ui.spacing_mut().slider_width = 170.0;
          let brush = self.canvas.tools.brush_mut();
          ui.add(egui::Slider::new(&mut brush.size, 1.0..=1000.0).logarithmic(true).vertical().show_value(false))
            .on_hover_text(self.i18n.tr_args("ui-size", &[("size", (brush.size.round() as i64).into())]));
          ui.add_space(14.0);
          let picking = self.canvas.tools.picking;
          if icons::button(ui, Icon::Eyedropper, picking, true, &self.i18n.tr("ui-eyedropper")).clicked() {
            self.canvas.tools.picking = !picking;
          }
          ui.add_space(14.0);
          let brush = self.canvas.tools.brush_mut();
          let mut percent = brush.opacity * 100.0;
          let opacity = ui.add(egui::Slider::new(&mut percent, 1.0..=100.0).vertical().show_value(false));
          if opacity.changed() {
            brush.opacity = percent / 100.0;
          }
          opacity.on_hover_text(self.i18n.tr_args("ui-opacity", &[("opacity", (percent.round() as i64).into())]));
          ui.add_space(18.0);
          let (can_undo, can_redo) = (self.doc.history().can_undo(), self.doc.history().can_redo());
          if icons::button(ui, Icon::Undo, false, can_undo, &self.i18n.tr("ui-undo")).clicked() {
            self.doc.undo();
          }
          if icons::button(ui, Icon::Redo, false, can_redo, &self.i18n.tr("ui-redo")).clicked() {
            self.doc.redo();
          }
        });
      });
  }

  /// The open floating panel, under the right end of the top bar (Actions under the left).
  pub(crate) fn popovers(&mut self, ctx: &egui::Context) {
    let Some(popover) = self.popover else { return };
    let screen = ctx.content_rect();
    let (anchor, pivot) = match popover {
      Popover::Actions => (Pos2::new(screen.left() + 10.0, screen.top() + theme::BAR_HEIGHT + 6.0), Align2::LEFT_TOP),
      _ => (Pos2::new(screen.right() - 10.0, screen.top() + theme::BAR_HEIGHT + 6.0), Align2::RIGHT_TOP),
    };
    egui::Area::new(egui::Id::unique("popover")).order(egui::Order::Foreground).fixed_pos(anchor).pivot(pivot).show(ctx, |ui| {
      let palette = theme::palette(ui);
      Frame::popup(ui.style()).fill(palette.panel).corner_radius(theme::PANEL_RADIUS).inner_margin(Margin::same(12)).show(ui, |ui| {
        ui.set_width(if popover == Popover::Color { 240.0 } else { 280.0 });
        match popover {
          Popover::Actions => self.actions_panel(ui),
          Popover::Brushes => self.brushes_panel(ui),
          Popover::Layers => self.layers_panel(ui),
          Popover::Color => self.color_panel(ui),
        }
      });
    });
  }

  fn title(&self, ui: &mut egui::Ui, id: &str) {
    ui.label(RichText::new(self.i18n.tr(id)).strong().size(15.0));
    ui.add_space(6.0);
  }

  fn actions_panel(&mut self, ui: &mut egui::Ui) {
    self.title(ui, "ui-actions");
    let wide = |ui: &mut egui::Ui, text: String, shortcut: &str| {
      ui.add(egui::Button::new(text).shortcut_text(shortcut).min_size(vec2(ui.available_width(), 30.0))).clicked()
    };
    if wide(ui, self.i18n.tr("ui-file-new"), "Ctrl+N") {
      self.request(Pending::New);
    }
    if wide(ui, self.i18n.tr("ui-file-open"), "Ctrl+O") {
      self.request(Pending::Open);
    }
    if wide(ui, self.i18n.tr("ui-file-save"), "Ctrl+S") {
      self.save(false);
    }
    if wide(ui, self.i18n.tr("ui-file-save-as"), "Ctrl+Shift+S") {
      self.save(true);
    }
    if wide(ui, self.i18n.tr("ui-file-export-png"), "Ctrl+Alt+Shift+W") {
      self.export_png();
    }
    ui.separator();
    ui.label(RichText::new(self.i18n.tr("ui-theme")).color(theme::palette(ui).muted));
    ui.horizontal(|ui| {
      let mut preference = ui.ctx().options(|o| o.theme_preference);
      let before = preference;
      ui.selectable_value(&mut preference, egui::ThemePreference::System, self.i18n.tr("ui-theme-system"));
      ui.selectable_value(&mut preference, egui::ThemePreference::Dark, self.i18n.tr("ui-theme-dark"));
      ui.selectable_value(&mut preference, egui::ThemePreference::Light, self.i18n.tr("ui-theme-light"));
      if preference != before {
        ui.ctx().set_theme(preference);
      }
    });
    ui.separator();
    ui.label(RichText::new(format!("rustcreate {}", env!("CARGO_PKG_VERSION"))).small().color(theme::palette(ui).muted));
  }

  fn brushes_panel(&mut self, ui: &mut egui::Ui) {
    self.title(ui, "ui-brushes");
    let palette = theme::palette(ui);
    for (index, (name, _)) in BRUSHES.iter().enumerate() {
      let selected = self.canvas.tools.selected == index;
      let brush = self.canvas.tools.brushes[index];
      let response = Frame::NONE
        .fill(if selected { palette.selected } else { Color32::TRANSPARENT })
        .corner_radius(6)
        .inner_margin(Margin::symmetric(10, 8))
        .show(ui, |ui| {
          ui.set_width(ui.available_width());
          ui.horizontal(|ui| {
            ui.label(RichText::new(self.i18n.tr(name)).strong());
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
              ui.label(RichText::new(format!("{} px", brush.size.round())).color(palette.muted));
            });
          });
          brush_preview(ui, brush, palette.ink);
        });
      if ui.interact(response.response.rect, ui.scope_id().with(("brush", index)), Sense::click()).clicked() {
        self.canvas.tools.selected = index;
      }
    }
  }

  fn layers_panel(&mut self, ui: &mut egui::Ui) {
    ui.horizontal(|ui| {
      ui.label(RichText::new(self.i18n.tr("ui-layers")).strong().size(15.0));
      ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
        if icons::button(ui, Icon::Plus, false, true, &self.i18n.tr("ui-layer-new")).clicked() {
          self.doc.add_layer();
        }
      });
    });
    ui.add_space(6.0);
    let palette = theme::palette(ui);
    egui::ScrollArea::vertical().max_height(320.0).show(ui, |ui| {
      for index in (0..self.doc.layers().len()).rev() {
        let layer = &self.doc.layers()[index];
        let (id, name, hidden, locked) = (layer.id, layer.name.clone(), layer.hidden, layer.locked);
        let active = index == self.doc.active();
        let row = Frame::NONE
          .fill(if active { palette.selected } else { palette.control })
          .corner_radius(6)
          .inner_margin(Margin::symmetric(8, 4))
          .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
              let renaming = self.renaming.as_ref().is_some_and(|(layer, _)| *layer == id);
              if renaming {
                let (_, text) = self.renaming.as_mut().expect("renaming");
                let edit = ui.add(egui::TextEdit::singleline(text).desired_width(150.0));
                edit.request_focus();
                if edit.lost_focus() {
                  let (_, text) = self.renaming.take().expect("renaming");
                  if !text.trim().is_empty() && text != name {
                    self.doc.rename_layer(index, text.trim());
                  }
                }
              } else {
                let label =
                  ui.add(egui::Label::new(RichText::new(&name).color(if hidden { palette.muted } else { palette.ink })).sense(Sense::click()));
                if label.double_clicked() {
                  self.renaming = Some((id, name.clone()));
                } else if label.clicked() {
                  self.doc.select(index);
                }
              }
              ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if icons::button(ui, if hidden { Icon::EyeOff } else { Icon::Eye }, false, true, &self.i18n.tr("ui-layer-visible")).clicked() {
                  self.doc.checkpoint();
                  self.doc.layer_mut(index).expect("layer").hidden = !hidden;
                }
                if icons::button(ui, Icon::Lock, locked, true, &self.i18n.tr("ui-layer-lock")).clicked() {
                  self.doc.checkpoint();
                  self.doc.layer_mut(index).expect("layer").locked = !locked;
                }
              });
            });
          });
        if ui.interact(row.response.rect, ui.scope_id().with(("layer", id)), Sense::click()).clicked() {
          self.doc.select(index);
        }
        ui.add_space(3.0);
      }
    });

    // The active layer's properties and actions.
    ui.separator();
    let index = self.doc.active();
    let layer = &self.doc.layers()[index];
    let (mut opacity, mut blend) = (layer.opacity * 100.0, layer.blend);
    ui.horizontal(|ui| {
      ui.label(self.i18n.tr("ui-layer-opacity"));
      let response = ui.add(egui::Slider::new(&mut opacity, 0.0..=100.0).suffix(" %").fixed_decimals(0));
      if response.changed() {
        // One undo step per drag, not per frame.
        if !self.editing_slider {
          self.doc.checkpoint();
          self.editing_slider = true;
        }
        self.doc.layer_mut(index).expect("layer").opacity = opacity / 100.0;
      }
      if !response.dragged() {
        self.editing_slider = false;
      }
    });
    ui.horizontal(|ui| {
      ui.label(self.i18n.tr("ui-layer-blend"));
      let current = BLEND_MODES.iter().find(|(mode, _)| *mode == blend).map_or("ui-blend-normal", |(_, id)| id);
      egui::ComboBox::from_id_salt("blend").selected_text(self.i18n.tr(current)).show_ui(ui, |ui| {
        for (mode, id) in BLEND_MODES {
          ui.selectable_value(&mut blend, mode, self.i18n.tr(id));
        }
      });
    });
    if blend != self.doc.layers()[index].blend {
      self.doc.checkpoint();
      self.doc.layer_mut(index).expect("layer").blend = blend;
    }
    ui.horizontal(|ui| {
      if icons::button(ui, Icon::Duplicate, false, true, &self.i18n.tr("ui-layer-duplicate")).clicked() {
        self.doc.duplicate_layer(index);
      }
      if icons::button(ui, Icon::MergeDown, false, index > 0, &self.i18n.tr("ui-layer-merge-down")).clicked() {
        self.doc.merge_down(index);
      }
      if icons::button(ui, Icon::Trash, false, self.doc.layers().len() > 1, &self.i18n.tr("ui-layer-delete")).clicked() {
        self.doc.delete_layer(index);
      }
    });
  }

  fn color_panel(&mut self, ui: &mut egui::Ui) {
    self.title(ui, "ui-color");
    let tools = &mut self.canvas.tools;
    let [r, g, b] = tools.primary;
    let mut color = Color32::from_rgb(r, g, b);
    if egui::color_picker::color_picker_color32(ui, &mut color, egui::color_picker::Alpha::Opaque) {
      tools.primary = [color.r(), color.g(), color.b()];
    }
    if !tools.recent.is_empty() {
      ui.add_space(8.0);
      ui.label(RichText::new(self.i18n.tr("ui-color-history")).color(theme::palette(ui).muted));
      ui.horizontal_wrapped(|ui| {
        for &[r, g, b] in &tools.recent.clone() {
          let (rect, response) = ui.allocate_exact_size(Vec2::splat(22.0), Sense::click());
          ui.painter().rect_filled(rect, 4.0, Color32::from_rgb(r, g, b));
          if response.clicked() {
            tools.primary = [r, g, b];
          }
        }
      });
    }
  }
}

/// A short S-shaped sample stroke showing the brush's width and softness.
fn brush_preview(ui: &mut egui::Ui, brush: rustcreate_core::Brush, ink: Color32) {
  let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 26.0), Sense::hover());
  let width = brush.size.clamp(1.0, 14.0);
  let points: Vec<Pos2> = (0..=40)
    .map(|i| {
      let t = i as f32 / 40.0;
      Pos2::new(rect.left() + 8.0 + t * (rect.width() - 16.0), rect.center().y + (t * std::f32::consts::TAU).sin() * 6.0)
    })
    .collect();
  let alpha = if brush.hardness < 0.5 { 0.45 } else { 1.0 } * brush.opacity;
  ui.painter().add(egui::Shape::line(points, Stroke::new(width, ink.gamma_multiply(alpha))));
  let _ = Rect::NOTHING;
}
