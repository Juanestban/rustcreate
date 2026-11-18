//! The Procreate-like layout: a thin top bar (gallery, actions, adjustments, selection,
//! transform on the left; brush, smudge, eraser, layers and color on the right), a left sidebar
//! with the size and opacity sliders, the eyedropper and undo/redo, and the floating panels the
//! top bar opens.

use eframe::egui::{self, Align, Align2, Color32, Frame, Layout, Margin, Pos2, RichText, Sense, Stroke, Vec2, vec2};
use rustcreate_core::BlendMode;

use super::icons::{self, Icon};
use super::studio;
use super::theme;
use crate::i18n::Language;
use crate::library::{CATEGORIES, MINE, Name};
use crate::settings::Theme;
use crate::tools::Tool;
use crate::{App, Pending};
use rustcreate_core::GradientKind;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Popover {
  Actions,
  Brushes,
  Fill,
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
            // Right to left: gradient, bucket, eraser, smudge, brush. Clicking the active tool
            // opens its panel: brushes, or the bucket and gradient options.
            for (tool, icon, tooltip) in [
              (Tool::Gradient, Icon::Gradient, "ui-tooltip-gradient"),
              (Tool::Fill, Icon::Bucket, "ui-tooltip-bucket"),
              (Tool::Eraser, Icon::Eraser, "ui-tooltip-eraser"),
              (Tool::Smudge, Icon::Smudge, "ui-tooltip-smudge"),
              (Tool::Brush, Icon::Brush, "ui-tooltip-brush"),
            ] {
              let active = self.canvas.tools.tool == tool;
              if icons::button(ui, icon, active, true, &self.i18n.tr(tooltip)).clicked() {
                if active {
                  self.toggle(if matches!(tool, Tool::Fill | Tool::Gradient) { Popover::Fill } else { Popover::Brushes });
                }
                self.canvas.tools.tool = tool;
              }
            }
          });
        });
      },
    );
  }

  fn toggle(&mut self, popover: Popover) {
    self.popover = if self.popover == Some(popover) { None } else { Some(popover) };
    self.brush_category = None;
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
        ui.set_width(match popover {
          Popover::Color => 240.0,
          Popover::Brushes => 380.0,
          _ => 280.0,
        });
        match popover {
          Popover::Actions => self.actions_panel(ui),
          Popover::Brushes => self.brushes_panel(ui),
          Popover::Fill => self.fill_panel(ui),
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
      self.new_canvas = Some(self.settings.canvas);
      self.popover = None;
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
    // Settings: applied and saved as soon as they change.
    let mut settings = self.settings.clone();
    let muted = theme::palette(ui).muted;
    ui.label(RichText::new(self.i18n.tr("ui-settings")).strong());
    ui.label(RichText::new(self.i18n.tr("ui-theme")).color(muted));
    ui.horizontal(|ui| {
      ui.selectable_value(&mut settings.theme, Theme::System, self.i18n.tr("ui-theme-system"));
      ui.selectable_value(&mut settings.theme, Theme::Dark, self.i18n.tr("ui-theme-dark"));
      ui.selectable_value(&mut settings.theme, Theme::Light, self.i18n.tr("ui-theme-light"));
    });
    ui.label(RichText::new(self.i18n.tr("ui-language")).color(muted));
    ui.horizontal(|ui| {
      ui.selectable_value(&mut settings.language, None, self.i18n.tr("ui-theme-system"));
      // Language names are written in their own language, not translated.
      ui.selectable_value(&mut settings.language, Some(Language::En), "English");
      ui.selectable_value(&mut settings.language, Some(Language::Es), "Español");
    });
    ui.checkbox(&mut settings.check_updates, self.i18n.tr("ui-check-updates"));
    ui.checkbox(&mut settings.quickshape, self.i18n.tr("ui-quickshape")).on_hover_text(self.i18n.tr("ui-quickshape-help"));
    if settings != self.settings {
      self.apply(settings);
    }
    ui.separator();
    ui.label(RichText::new(format!("rustcreate {}", env!("CARGO_PKG_VERSION"))).small().color(theme::palette(ui).muted));
  }

  /// Bucket and gradient options.
  fn fill_panel(&mut self, ui: &mut egui::Ui) {
    let tools = &mut self.canvas.tools;
    let i18n = &self.i18n;
    ui.label(RichText::new(i18n.tr("ui-tooltip-bucket")).strong());
    ui.horizontal(|ui| {
      ui.label(i18n.tr("ui-fill-threshold"));
      ui.add(egui::Slider::new(&mut tools.fill_threshold, 0.0..=1.0).custom_formatter(|v, _| format!("{:.0} %", v * 100.0)));
    });
    ui.checkbox(&mut tools.fill_all_layers, i18n.tr("ui-fill-all-layers"));
    ui.separator();
    ui.label(RichText::new(i18n.tr("ui-tooltip-gradient")).strong());
    ui.horizontal(|ui| {
      ui.selectable_value(&mut tools.gradient, GradientKind::Linear, i18n.tr("ui-gradient-linear"));
      ui.selectable_value(&mut tools.gradient, GradientKind::Radial, i18n.tr("ui-gradient-radial"));
    });
    ui.checkbox(&mut tools.gradient_to_clear, i18n.tr("ui-gradient-to-clear"));
  }

  /// The brush library: categories on the left, the category's brushes on the right with
  /// previews painted by the engine. Picking a brush sets it for the current tool.
  fn brushes_panel(&mut self, ui: &mut egui::Ui) {
    self.title(ui, "ui-brushes");
    let palette = theme::palette(ui);
    let selected = self.canvas.tools.selected();
    let category = *self.brush_category.get_or_insert(self.canvas.tools.library.entries[selected].category);
    ui.horizontal_top(|ui| {
      ui.vertical(|ui| {
        ui.set_width(118.0);
        for (index, id) in CATEGORIES.iter().enumerate() {
          if ui.selectable_label(category == index, self.i18n.tr(id)).clicked() {
            self.brush_category = Some(index);
          }
        }
      });
      ui.separator();
      ui.vertical(|ui| {
        egui::ScrollArea::vertical().max_height(360.0).show(ui, |ui| {
          let indices: Vec<usize> =
            (0..self.canvas.tools.library.entries.len()).filter(|&i| self.canvas.tools.library.entries[i].category == category).collect();
          if indices.is_empty() {
            ui.label(RichText::new(self.i18n.tr("ui-brushes-mine-empty")).color(palette.muted));
          }
          for index in indices {
            let thumbnail = self.thumbnail(ui, index);
            let response = Frame::NONE
              .fill(if index == selected { palette.selected } else { Color32::TRANSPARENT })
              .corner_radius(6)
              .inner_margin(Margin::symmetric(8, 4))
              .show(ui, |ui| {
                ui.set_width(220.0);
                ui.label(RichText::new(self.brush_name(index)).strong());
                ui.add(egui::Image::new(&thumbnail).fit_to_exact_size(vec2(220.0, 44.0)));
              });
            if ui.interact(response.response.rect, ui.scope_id().with(("brush", index)), Sense::click()).clicked() {
              self.canvas.tools.select(index);
            }
          }
        });
      });
    });
    ui.separator();
    ui.horizontal(|ui| {
      if icons::button(ui, Icon::Duplicate, false, true, &self.i18n.tr("ui-brush-duplicate")).clicked() {
        let name = self.i18n.tr_args("ui-brush-copy-name", &[("name", self.brush_name(selected).into())]);
        let copy = self.canvas.tools.library.duplicate(selected, name);
        self.canvas.tools.select(copy);
        self.brush_category = Some(MINE);
      }
      let custom = !self.canvas.tools.library.is_builtin(selected);
      if icons::button(ui, Icon::Trash, false, custom, &self.i18n.tr("ui-brush-delete")).clicked() {
        self.canvas.tools.library.delete(selected);
      }
      if ui.button(self.i18n.tr("ui-studio")).clicked() {
        self.studio = true;
        self.popover = None;
      }
    });
  }

  /// A brush's display name: its translated token, or what the user called it.
  pub(crate) fn brush_name(&self, index: usize) -> String {
    match &self.canvas.tools.library.entries[index].name {
      Name::Token(id) => self.i18n.tr(id),
      Name::Custom(name) => name.clone(),
    }
  }

  /// The engine-painted preview of a library brush, re-rendered when it or the theme changes.
  fn thumbnail(&mut self, ui: &egui::Ui, index: usize) -> egui::TextureHandle {
    let entry = &self.canvas.tools.library.entries[index];
    let dark = ui.visuals().dark_mode;
    match self.thumbnails.get(&entry.id) {
      Some((brush, was_dark, texture)) if *brush == entry.brush && *was_dark == dark => texture.clone(),
      _ => {
        let image = studio::render(entry.brush, theme::palette(ui).ink, (256, 52));
        let texture = ui.ctx().load_texture(format!("brush-{}", entry.id), image, egui::TextureOptions::LINEAR);
        self.thumbnails.insert(entry.id.clone(), (entry.brush, dark, texture.clone()));
        texture
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
