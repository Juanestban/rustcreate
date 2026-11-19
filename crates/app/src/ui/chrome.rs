//! The Procreate-like layout: a thin top bar (gallery, actions, adjustments, selection,
//! transform on the left; brush, smudge, eraser, layers and color on the right), a left sidebar
//! with the size and opacity sliders, the eyedropper and undo/redo, and the floating panels the
//! top bar opens.

use eframe::egui::{self, Align, Align2, Color32, Frame, Layout, Margin, Pos2, RichText, Sense, Stroke, Vec2, vec2};
use rustcreate_core::BlendMode;

use super::icons::{self, Icon};
use super::studio;
use super::theme;
use crate::canvas::ColorDrop;
use crate::guides::Kind;
use crate::i18n::Language;
use crate::library::{CATEGORIES, MINE, Name};
use crate::settings::Theme;
use crate::tools::{SelectKind, Tool};
use crate::{App, Export, Pending};
use rustcreate_core::{GradientKind, Symmetry};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Popover {
  File,
  Actions,
  Brushes,
  Fill,
  Layers,
  Color,
  Select,
  Adjust,
}

/// The layer blend modes in Photoshop's order; a separator goes before each index in
/// `BLEND_GROUPS` (darken, lighten, contrast, compare, color).
const BLEND_MODES: [(BlendMode, &str); 23] = [
  (BlendMode::Normal, "ui-blend-normal"),
  (BlendMode::Darken, "ui-blend-darken"),
  (BlendMode::Multiply, "ui-blend-multiply"),
  (BlendMode::ColorBurn, "ui-blend-color-burn"),
  (BlendMode::LinearBurn, "ui-blend-linear-burn"),
  (BlendMode::Lighten, "ui-blend-lighten"),
  (BlendMode::Screen, "ui-blend-screen"),
  (BlendMode::ColorDodge, "ui-blend-color-dodge"),
  (BlendMode::Add, "ui-blend-add"),
  (BlendMode::Overlay, "ui-blend-overlay"),
  (BlendMode::SoftLight, "ui-blend-soft-light"),
  (BlendMode::HardLight, "ui-blend-hard-light"),
  (BlendMode::VividLight, "ui-blend-vivid-light"),
  (BlendMode::LinearLight, "ui-blend-linear-light"),
  (BlendMode::PinLight, "ui-blend-pin-light"),
  (BlendMode::Difference, "ui-blend-difference"),
  (BlendMode::Exclusion, "ui-blend-exclusion"),
  (BlendMode::Subtract, "ui-blend-subtract"),
  (BlendMode::Divide, "ui-blend-divide"),
  (BlendMode::Hue, "ui-blend-hue"),
  (BlendMode::Saturation, "ui-blend-saturation"),
  (BlendMode::Color, "ui-blend-color"),
  (BlendMode::Luminosity, "ui-blend-luminosity"),
];

const BLEND_GROUPS: [usize; 5] = [1, 5, 9, 15, 19];

impl App {
  pub(crate) fn top_bar(&mut self, ui: &mut egui::Ui) {
    let palette = theme::palette(ui);
    egui::Panel::top("top-bar").exact_size(theme::BAR_HEIGHT).frame(Frame::NONE.fill(palette.chrome).inner_margin(Margin::symmetric(10, 6))).show(
      ui,
      |ui| {
        ui.horizontal_centered(|ui| {
          ui.spacing_mut().item_spacing.x = 4.0;
          self.popover_button(ui, Icon::Gallery, Popover::File, "ui-file");
          self.popover_button(ui, Icon::Actions, Popover::Actions, "ui-actions");
          ui.add_space(8.0);
          self.popover_button(ui, Icon::Adjustments, Popover::Adjust, "ui-adjustments");
          // Selection: picks the tool; clicking it again opens its options.
          let selecting = self.canvas.tools.tool == Tool::Select;
          if icons::button(ui, Icon::Selection, selecting, true, &self.i18n.tr("ui-selection")).clicked() {
            if selecting {
              self.toggle(Popover::Select);
            }
            (self.canvas.tools.tool, self.canvas.tools.hand) = (Tool::Select, false);
          }
          // Free Transform: a second click applies it.
          let transforming = self.canvas.transforming();
          if icons::button(ui, Icon::Transform, transforming, true, &self.i18n.tr("ui-transform")).clicked() {
            if transforming { self.canvas.end_transform(&mut self.doc, true) } else { self.canvas.start_transform(&mut self.doc) }
          }

          ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            self.color_button(ui);
            self.popover_button(ui, Icon::Layers, Popover::Layers, "ui-layers");
            // Right to left: gradient, bucket, eraser, smudge, brush. Clicking the active tool
            // opens its panel: brushes, or the bucket and gradient options.
            for (tool, icon, tooltip) in [
              (Tool::Text, Icon::Text, "ui-tooltip-text"),
              (Tool::Gradient, Icon::Gradient, "ui-tooltip-gradient"),
              (Tool::Fill, Icon::Bucket, "ui-tooltip-bucket"),
              (Tool::Eraser, Icon::Eraser, "ui-tooltip-eraser"),
              (Tool::Smudge, Icon::Smudge, "ui-tooltip-smudge"),
              (Tool::Brush, Icon::Brush, "ui-tooltip-brush"),
            ] {
              let active = self.canvas.tools.tool == tool;
              if icons::button(ui, icon, active, true, &self.i18n.tr(tooltip)).clicked() {
                if active && tool != Tool::Text {
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
    // Panels that change the document finish the text being typed and apply a transform in
    // progress first; brushes, color,
    // fill and selection options leave it waiting on its bar.
    if matches!(popover, Popover::Layers | Popover::Actions | Popover::Adjust) {
      self.canvas.end_text(&mut self.doc);
      self.canvas.end_transform(&mut self.doc, true);
    }
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
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(32.0), Sense::click_and_drag());
    let center = rect.center();
    let ring: Vec<Pos2> = (0..=48).map(|i| center + 12.0 * Vec2::angled(i as f32 / 48.0 * std::f32::consts::TAU)).collect();
    for (i, pair) in ring.windows(2).enumerate() {
      let t = i as f32 / 48.0 * (theme::GRADIENT.len() - 1) as f32;
      let (a, b) = (theme::GRADIENT[t as usize], theme::GRADIENT[(t as usize + 1).min(theme::GRADIENT.len() - 1)]);
      ui.painter().line_segment([pair[0], pair[1]], Stroke::new(2.5, a.lerp_to_gamma(b, t.fract())));
    }
    let [r, g, b] = self.canvas.tools.primary;
    ui.painter().circle_filled(center, 9.0, Color32::from_rgb(r, g, b));
    // ColorDrop: drag the color onto the canvas to fill there; a drop follows the pointer.
    response.dnd_set_drag_payload(ColorDrop([r, g, b]));
    if response.dragged()
      && let Some(at) = ui.input(|i| i.pointer.hover_pos())
    {
      let painter = ui.ctx().layer_painter(egui::LayerId::new(egui::Order::Tooltip, egui::Id::unique("color-drop")));
      painter.circle(at, 10.0, Color32::from_rgb(r, g, b), Stroke::new(2.0, Color32::WHITE));
    }
    if response.on_hover_text(self.i18n.tr("ui-color-tooltip")).clicked() {
      self.toggle(Popover::Color);
    }
  }

  pub(crate) fn sidebar(&mut self, ui: &mut egui::Ui) {
    let palette = theme::palette(ui);
    egui::Panel::left("sidebar")
      .exact_size(theme::SIDEBAR_WIDTH)
      .resizable(false)
      // Off the window's edge, so the sliders and buttons are not pressed against it.
      .frame(Frame::NONE.fill(palette.chrome).inner_margin(Margin { left: 12, right: 4, top: 18, bottom: 18 }))
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
          // Undo while transforming cancels the transform, like Ctrl+Z does.
          if icons::button(ui, Icon::Undo, false, can_undo, &self.i18n.tr("ui-undo")).clicked() {
            if self.canvas.transforming() {
              self.canvas.end_transform(&mut self.doc, false)
            } else {
              self.doc.undo();
            }
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
    // A press on the canvas closes the panel and does nothing else (like tapping away in
    // Procreate): a see-through cover over the canvas, under the panel, takes it.
    let screen = ctx.content_rect();
    let canvas = egui::Rect::from_min_max(screen.min + vec2(theme::SIDEBAR_WIDTH, theme::BAR_HEIGHT), screen.max);
    egui::Area::new(egui::Id::unique("popover-cover")).order(egui::Order::Middle).fixed_pos(canvas.min).show(ctx, |ui| {
      let cover = ui.allocate_rect(egui::Rect::from_min_size(canvas.min, canvas.size()), Sense::click_and_drag());
      if cover.clicked() || cover.drag_started() {
        self.popover = None;
      }
    });
    if self.popover.is_none() {
      return;
    }
    let (anchor, pivot) = match popover {
      Popover::File | Popover::Actions => (Pos2::new(screen.left() + 10.0, screen.top() + theme::BAR_HEIGHT + 6.0), Align2::LEFT_TOP),
      _ => (Pos2::new(screen.right() - 10.0, screen.top() + theme::BAR_HEIGHT + 6.0), Align2::RIGHT_TOP),
    };
    egui::Area::new(egui::Id::unique("popover")).order(egui::Order::Foreground).fixed_pos(anchor).pivot(pivot).show(ctx, |ui| {
      let palette = theme::palette(ui);
      Frame::popup(ui.style()).fill(palette.panel).corner_radius(theme::PANEL_RADIUS).inner_margin(Margin::same(12)).show(ui, |ui| {
        ui.set_width(match popover {
          Popover::Color => 280.0,
          Popover::Brushes => 330.0,
          Popover::Layers => 320.0,
          _ => 280.0,
        });
        // A close button in the corner, for those who don't know Esc or tapping away.
        let corner = ui.max_rect().right_top();
        let close = egui::Rect::from_min_size(corner - vec2(22.0, 0.0), Vec2::splat(22.0));
        if ui.put(close, egui::Button::new("×").frame(false)).on_hover_text(format!("{} (Esc)", self.i18n.tr("ui-close"))).clicked() {
          self.popover = None;
        }
        match popover {
          Popover::File => self.file_panel(ui),
          Popover::Actions => self.actions_panel(ui),
          Popover::Brushes => self.brushes_panel(ui),
          Popover::Fill => self.fill_panel(ui),
          Popover::Select => self.select_panel(ui),
          Popover::Adjust => self.adjust_menu(ui),
          Popover::Layers => self.layers_panel(ui),
          Popover::Color => self.color_panel(ui),
        }
      });
    });
  }

  pub(crate) fn title(&self, ui: &mut egui::Ui, id: &str) {
    ui.label(RichText::new(self.i18n.tr(id)).strong().size(15.0));
    ui.add_space(6.0);
  }

  /// Photoshop's File menu: new, open (and recent files), close, save, save as, save a copy,
  /// revert, place, export and quit.
  fn file_panel(&mut self, ui: &mut egui::Ui) {
    self.title(ui, "ui-file");
    let item = |ui: &mut egui::Ui, text: String, shortcut: &str, enabled: bool| {
      ui.add_enabled(enabled, egui::Button::new(text).shortcut_text(shortcut).min_size(vec2(ui.available_width(), 30.0))).clicked()
    };
    let mut close = true;
    if item(ui, self.i18n.tr("ui-file-new"), "Ctrl+N", true) {
      self.new_canvas = Some(self.settings.canvas);
    } else if item(ui, self.i18n.tr("ui-file-open"), "Ctrl+O", true) {
      self.request(Pending::Open(None));
    } else {
      close = false;
    }
    let muted = theme::palette(ui).muted;
    egui::CollapsingHeader::new(self.i18n.tr("ui-file-recent")).id_salt("recent").show(ui, |ui| {
      if self.settings.recent.is_empty() {
        ui.label(RichText::new(self.i18n.tr("ui-file-recent-none")).color(muted));
      }
      for path in self.settings.recent.clone() {
        let name = path.file_stem().map_or_else(|| path.display().to_string(), |n| n.to_string_lossy().into_owned());
        let response =
          ui.add(egui::Button::new(name).frame(false).min_size(vec2(ui.available_width(), 26.0))).on_hover_text(path.display().to_string());
        if response.clicked() {
          self.request(Pending::Open(Some(path.clone())));
          close = true;
        }
        response.context_menu(|ui| {
          if ui.button(self.i18n.tr("ui-file-recent-remove")).clicked() {
            self.forget(&path);
            ui.close();
          }
        });
      }
    });
    ui.separator();
    let saved = self.path.is_some();
    if item(ui, self.i18n.tr("ui-file-close"), "Ctrl+W", true) {
      self.request(Pending::New(self.settings.canvas.0, self.settings.canvas.1));
      close = true;
    }
    if item(ui, self.i18n.tr("ui-file-save"), "Ctrl+S", true) {
      self.save(false);
      close = true;
    }
    if item(ui, self.i18n.tr("ui-file-save-as"), "Ctrl+Shift+S", true) {
      self.save(true);
      close = true;
    }
    if item(ui, self.i18n.tr("ui-file-save-copy"), "Ctrl+Alt+S", true) {
      self.save_copy();
      close = true;
    }
    if item(ui, self.i18n.tr("ui-file-revert"), "F12", saved && self.dirty()) {
      self.request(Pending::Revert);
      close = true;
    }
    ui.separator();
    if item(ui, self.i18n.tr("ui-file-place"), "", true) {
      self.place();
      close = true;
    }
    for (format, id, shortcut) in
      [(Export::Png, "ui-file-export-png", "Ctrl+Alt+Shift+W"), (Export::Jpeg, "ui-file-export-jpeg", ""), (Export::Webp, "ui-file-export-webp", "")]
    {
      if item(ui, self.i18n.tr(id), shortcut, true) {
        self.export(format);
        close = true;
      }
    }
    ui.separator();
    if item(ui, self.i18n.tr("ui-file-quit"), "Ctrl+Q", true) {
      self.request(Pending::Close);
      close = true;
    }
    if close {
      self.popover = None;
    }
  }

  fn actions_panel(&mut self, ui: &mut egui::Ui) {
    self.title(ui, "ui-actions");
    let wide = |ui: &mut egui::Ui, text: String, shortcut: &str| {
      ui.add(egui::Button::new(text).shortcut_text(shortcut).min_size(vec2(ui.available_width(), 30.0))).clicked()
    };
    if ui
      .add_enabled(self.doc.selection().is_some(), egui::Button::new(self.i18n.tr("ui-canvas-crop")).min_size(vec2(ui.available_width(), 30.0)))
      .clicked()
    {
      self.crop();
    }
    if wide(ui, self.i18n.tr("ui-canvas-size"), "Ctrl+Alt+C") {
      self.open_resize(false);
    }
    if wide(ui, self.i18n.tr("ui-image-size"), "Ctrl+Alt+I") {
      self.open_resize(true);
    }
    ui.separator();
    self.guides_section(ui);
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

  /// The drawing guide and Drawing Assist.
  fn guides_section(&mut self, ui: &mut egui::Ui) {
    let (guides, i18n) = (&mut self.canvas.tools.guides, &self.i18n);
    ui.label(RichText::new(i18n.tr("ui-guide")).strong());
    let kinds = [
      (Kind::Off, "ui-guide-off"),
      (Kind::Grid, "ui-guide-grid"),
      (Kind::Isometric, "ui-guide-isometric"),
      (Kind::Perspective, "ui-guide-perspective"),
      (Kind::Symmetry, "ui-guide-symmetry"),
    ];
    let name = |kind: Kind| kinds.iter().find(|(k, _)| *k == kind).map_or(String::new(), |(_, id)| i18n.tr(id));
    egui::ComboBox::from_id_salt("guide").selected_text(name(guides.kind)).show_ui(ui, |ui| {
      for (kind, id) in kinds {
        ui.selectable_value(&mut guides.kind, kind, i18n.tr(id));
      }
    });
    match guides.kind {
      Kind::Off => return,
      Kind::Perspective => {
        ui.horizontal(|ui| {
          for points in 1..=3 {
            ui.selectable_value(&mut guides.points, points, i18n.tr_args("ui-guide-points", &[("count", points.into())]));
          }
        });
      }
      Kind::Grid | Kind::Isometric => {
        ui.horizontal(|ui| {
          ui.label(i18n.tr("ui-guide-spacing"));
          ui.add(egui::Slider::new(&mut guides.spacing, 10.0..=1000.0).logarithmic(true).suffix(" px").fixed_decimals(0));
        });
      }
      Kind::Symmetry => {
        let mut radial = match guides.symmetry {
          Symmetry::Radial(n) => n,
          _ => 6,
        };
        ui.horizontal_wrapped(|ui| {
          for (symmetry, id) in [
            (Symmetry::Vertical, "ui-symmetry-vertical"),
            (Symmetry::Horizontal, "ui-symmetry-horizontal"),
            (Symmetry::Quadrant, "ui-symmetry-quadrant"),
            (Symmetry::Radial(radial), "ui-symmetry-radial"),
          ] {
            let on = std::mem::discriminant(&guides.symmetry) == std::mem::discriminant(&symmetry);
            if ui.selectable_label(on, i18n.tr(id)).clicked() {
              guides.symmetry = symmetry;
            }
          }
        });
        if matches!(guides.symmetry, Symmetry::Radial(_))
          && ui.add(egui::Slider::new(&mut radial, 2..=16).text(i18n.tr("ui-symmetry-copies"))).changed()
        {
          guides.symmetry = Symmetry::Radial(radial);
        }
      }
    }
    ui.checkbox(&mut guides.assist, i18n.tr("ui-guide-assist")).on_hover_text(i18n.tr("ui-guide-assist-help"));
    ui.toggle_value(&mut guides.editing, i18n.tr("ui-guide-edit")).on_hover_text(i18n.tr("ui-guide-edit-help"));
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

  /// The selection tool's kind and the commands on the current selection.
  fn select_panel(&mut self, ui: &mut egui::Ui) {
    self.title(ui, "ui-selection");
    let tools = &mut self.canvas.tools;
    let i18n = &self.i18n;
    ui.horizontal_wrapped(|ui| {
      for (kind, id) in [
        (SelectKind::Rectangle, "ui-select-rectangle"),
        (SelectKind::Ellipse, "ui-select-ellipse"),
        (SelectKind::Lasso, "ui-select-lasso"),
        (SelectKind::Wand, "ui-select-wand"),
      ] {
        ui.selectable_value(&mut tools.select_kind, kind, i18n.tr(id));
      }
    });
    if tools.select_kind == SelectKind::Wand {
      ui.horizontal(|ui| {
        ui.label(i18n.tr("ui-fill-threshold"));
        ui.add(egui::Slider::new(&mut tools.fill_threshold, 0.0..=1.0).custom_formatter(|v, _| format!("{:.0} %", v * 100.0)));
      });
      ui.checkbox(&mut tools.fill_all_layers, i18n.tr("ui-fill-all-layers"));
    }
    ui.label(RichText::new(i18n.tr("ui-select-help")).small().color(theme::palette(ui).muted));
    ui.separator();
    let doc = &mut self.doc;
    let selected = doc.selection().is_some();
    ui.horizontal_wrapped(|ui| {
      if ui.button(i18n.tr("ui-select-all")).clicked() {
        doc.select_all();
      }
      if ui.add_enabled(selected, egui::Button::new(i18n.tr("ui-select-none"))).clicked() {
        doc.deselect();
      }
      if ui.add_enabled(selected, egui::Button::new(i18n.tr("ui-select-invert"))).clicked() {
        doc.invert_selection();
      }
    });
    ui.horizontal(|ui| {
      ui.add(egui::Slider::new(&mut tools.feather, 1.0..=200.0).logarithmic(true).suffix(" px").fixed_decimals(0));
      if ui.add_enabled(selected, egui::Button::new(i18n.tr("ui-select-feather"))).clicked() {
        doc.feather_selection(tools.feather);
      }
    });
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
        ui.set_width(96.0);
        for (index, id) in CATEGORIES.iter().enumerate() {
          if ui.selectable_label(category == index, self.i18n.tr(id)).clicked() {
            self.brush_category = Some(index);
          }
        }
      });
      ui.separator();
      ui.vertical(|ui| {
        // At most half the window tall, so the canvas stays in view.
        let tall = (ui.ctx().content_rect().height() * 0.5).clamp(200.0, 360.0);
        egui::ScrollArea::vertical().max_height(tall).show(ui, |ui| {
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
                ui.set_width(186.0);
                ui.label(RichText::new(self.brush_name(index)).strong());
                ui.add(egui::Image::new(&thumbnail).fit_to_exact_size(vec2(186.0, 37.0)));
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
      if ui.button(self.i18n.tr("ui-brush-import")).on_hover_text(self.i18n.tr("ui-brush-import-help")).clicked()
        && let Some(path) =
          rfd::FileDialog::new().add_filter(self.i18n.tr("ui-brush-import-filter"), &["rcbrushes", "brushset", "brush", "abr"]).pick_file()
      {
        match self.canvas.tools.library.import(&path) {
          Ok(first) => {
            self.canvas.tools.select(first);
            self.brush_category = Some(MINE);
          }
          Err(error) => self.error = Some(self.i18n.tr_args("ui-brush-import-failed", &[("error", error.to_string().into())])),
        }
      }
      if ui.button(self.i18n.tr("ui-brush-export")).on_hover_text(self.i18n.tr("ui-brush-export-help")).clicked() {
        self.export_brushes(category);
      }
    });
  }

  /// Saves the brushes of `category` as a `.rcbrushes` set, every setting included.
  fn export_brushes(&mut self, category: usize) {
    let library = &self.canvas.tools.library;
    let brushes = library.category(category, |e| match &e.name {
      Name::Token(id) => self.i18n.tr(id),
      Name::Custom(name) => name.clone(),
    });
    if brushes.is_empty() {
      return;
    }
    let file = format!("{}.rcbrushes", self.i18n.tr(CATEGORIES[category]));
    let Some(path) = rfd::FileDialog::new().add_filter(self.i18n.tr("ui-brush-import-filter"), &["rcbrushes"]).set_file_name(file).save_file() else {
      return;
    };
    let path = path.with_extension("rcbrushes");
    if let Err(error) = rustcreate_core::export_brushes(&brushes).and_then(|bytes| Ok(std::fs::write(&path, bytes)?)) {
      self.error = Some(self.i18n.tr_args("ui-brush-export-failed", &[("error", error.to_string().into())]));
    }
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
    ui.add_space(8.0);
    // Changes that add, remove or reorder layers wait until the panel is drawn: indices taken
    // while drawing would point past the end or at another layer.
    let mut action: Option<LayerAction> = None;
    let palette = theme::palette(ui);
    let tall = (ui.ctx().content_rect().height() * 0.5).clamp(200.0, 460.0);
    egui::ScrollArea::vertical().max_height(tall).auto_shrink([false, true]).show(ui, |ui| {
      ui.spacing_mut().item_spacing.y = 6.0;
      for index in (0..self.doc.layers().len()).rev() {
        let layer = &self.doc.layers()[index];
        let (id, hidden, locked, group, text) = (layer.id, layer.hidden, layer.locked, layer.is_group(), layer.text().is_some());
        let name = layer.name.clone();
        // A clipped layer is marked with an arrow into the layer below, a mask with ◐.
        let mut shown = if layer.clip && index > 0 { format!("↳ {name}") } else { name.clone() };
        if layer.mask.is_some() {
          shown.push_str("  ◐");
        }
        let indent = self.doc.depth(index) as f32 * 16.0;
        let active = index == self.doc.active();

        // The whole row selects; the eye and the lock sit on top of it.
        let (rect, row) = ui.allocate_exact_size(vec2(ui.available_width(), 40.0), Sense::click());
        let fill = if active {
          palette.selected
        } else if row.hovered() {
          palette.control.gamma_multiply(1.4)
        } else {
          palette.control
        };
        ui.painter().rect_filled(rect, 8, fill);
        let button_at = |n: f32| egui::Rect::from_center_size(egui::pos2(rect.right() - 22.0 - n * 34.0, rect.center().y), Vec2::splat(32.0));
        if ui
          .put(button_at(0.0), |ui: &mut egui::Ui| {
            icons::button(ui, if hidden { Icon::EyeOff } else { Icon::Eye }, false, true, &self.i18n.tr("ui-layer-visible"))
          })
          .clicked()
        {
          self.doc.checkpoint();
          self.doc.layer_mut(index).expect("layer").hidden = !hidden;
        }
        if ui.put(button_at(1.0), |ui: &mut egui::Ui| icons::button(ui, Icon::Lock, locked, true, &self.i18n.tr("ui-layer-lock"))).clicked() {
          self.doc.checkpoint();
          self.doc.layer_mut(index).expect("layer").locked = !locked;
        }
        // A small badge says what kind of layer it is: a group, a text, or plain pixels.
        let badge = egui::Rect::from_center_size(egui::pos2(rect.left() + 22.0 + indent, rect.center().y), Vec2::splat(20.0));
        let ink = if hidden { palette.muted } else { palette.ink };
        if group || text {
          icons::paint(ui.painter(), badge, if group { Icon::Layers } else { Icon::Text }, ink);
        } else {
          ui.painter().rect_stroke(badge.shrink(2.0), 3, Stroke::new(1.2, palette.muted), egui::StrokeKind::Inside);
        }
        let label = egui::Rect::from_min_max(egui::pos2(badge.right() + 10.0, rect.top()), egui::pos2(button_at(1.0).left() - 6.0, rect.bottom()));
        if self.renaming.as_ref().is_some_and(|(layer, _)| *layer == id) {
          let (_, typed) = self.renaming.as_mut().expect("renaming");
          let edit = ui.put(label.shrink2(vec2(0.0, 6.0)), egui::TextEdit::singleline(typed).font(egui::FontId::proportional(14.5)));
          edit.request_focus();
          if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.renaming = None;
          } else if edit.lost_focus() {
            let (_, typed) = self.renaming.take().expect("renaming");
            if !typed.trim().is_empty() && typed != name {
              self.doc.rename_layer(index, typed.trim());
            }
          }
        } else {
          let galley = ui.painter().layout_no_wrap(shown, egui::FontId::proportional(14.5), ink);
          let at = egui::Align2::LEFT_CENTER.align_size_within_rect(galley.size(), label).min;
          ui.painter().with_clip_rect(label).galley(at, galley, ink);
        }
        if row.double_clicked() {
          self.renaming = Some((id, name.clone()));
        } else if row.clicked() {
          self.doc.select(index);
        }
        row.context_menu(|ui| {
          if ui.button(self.i18n.tr("ui-layer-rename")).clicked() {
            self.renaming = Some((id, name.clone()));
            ui.close();
          }
          if ui.button(self.i18n.tr("ui-layer-duplicate")).clicked() {
            action = Some(LayerAction::Duplicate(index));
            ui.close();
          }
          if ui.add_enabled(index > 0, egui::Button::new(self.i18n.tr("ui-layer-merge-down"))).clicked() {
            action = Some(LayerAction::MergeDown(index));
            ui.close();
          }
          if ui.add_enabled(self.doc.layers().len() > 1, egui::Button::new(self.i18n.tr("ui-layer-delete"))).clicked() {
            action = Some(LayerAction::Delete(index));
            ui.close();
          }
        });
      }
    });

    // The active layer's properties and actions.
    ui.add_space(4.0);
    ui.separator();
    ui.add_space(4.0);
    let index = self.doc.active();
    let layer = &self.doc.layers()[index];
    let (mut opacity, mut blend, is_group) = (layer.opacity * 100.0, layer.blend, layer.is_group());
    egui::Grid::new("layer-properties").num_columns(2).spacing(vec2(10.0, 8.0)).show(ui, |ui| {
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
      ui.end_row();
      ui.label(self.i18n.tr("ui-layer-blend"));
      let current = BLEND_MODES.iter().find(|(mode, _)| *mode == blend).map_or("ui-blend-normal", |(_, id)| id);
      egui::ComboBox::from_id_salt("blend").selected_text(self.i18n.tr(current)).width(170.0).show_ui(ui, |ui| {
        for (i, (mode, id)) in BLEND_MODES.into_iter().enumerate() {
          if BLEND_GROUPS.contains(&i) {
            ui.separator();
          }
          ui.selectable_value(&mut blend, mode, self.i18n.tr(id));
        }
      });
      ui.end_row();
    });
    if blend != self.doc.layers()[index].blend {
      self.doc.checkpoint();
      self.doc.layer_mut(index).expect("layer").blend = blend;
    }
    ui.add_space(4.0);
    let (mut alpha_lock, mut clip) = (self.doc.layers()[index].alpha_lock, self.doc.layers()[index].clip);
    ui.horizontal(|ui| {
      ui.checkbox(&mut alpha_lock, self.i18n.tr("ui-layer-alpha-lock")).on_hover_text(self.i18n.tr("ui-layer-alpha-lock-help"));
      ui.add_enabled(index > 0, egui::Checkbox::new(&mut clip, self.i18n.tr("ui-layer-clip"))).on_hover_text(self.i18n.tr("ui-layer-clip-help"));
    });
    if (alpha_lock, clip) != (self.doc.layers()[index].alpha_lock, self.doc.layers()[index].clip) {
      self.doc.checkpoint();
      let layer = self.doc.layer_mut(index).expect("layer");
      (layer.alpha_lock, layer.clip) = (alpha_lock, clip);
    }
    // The layer mask: black hides, white shows; painting goes to it while "Edit mask" is on.
    ui.horizontal_wrapped(|ui| {
      if self.doc.layers()[index].mask.is_none() {
        if ui.button(self.i18n.tr("ui-mask-add")).on_hover_text(self.i18n.tr("ui-mask-help")).clicked() {
          self.doc.add_mask(index);
        }
      } else {
        let mut editing = self.doc.editing_mask();
        if ui.checkbox(&mut editing, self.i18n.tr("ui-mask-edit")).on_hover_text(self.i18n.tr("ui-mask-help")).changed() {
          self.doc.set_editing_mask(editing);
        }
        if ui.button(self.i18n.tr("ui-mask-apply")).clicked() {
          self.doc.remove_mask(index, true);
        }
        if ui.button(self.i18n.tr("ui-mask-delete")).clicked() {
          self.doc.remove_mask(index, false);
        }
      }
    });
    ui.add_space(4.0);
    ui.separator();
    ui.horizontal(|ui| {
      if icons::button(ui, Icon::Duplicate, false, true, &self.i18n.tr("ui-layer-duplicate")).clicked() {
        action = Some(LayerAction::Duplicate(index));
      }
      if icons::button(ui, Icon::MergeDown, false, index > 0, &self.i18n.tr("ui-layer-merge-down")).clicked() {
        action = Some(LayerAction::MergeDown(index));
      }
      if icons::button(ui, Icon::Trash, false, self.doc.layers().len() > 1, &self.i18n.tr("ui-layer-delete")).clicked() {
        action = Some(LayerAction::Delete(index));
      }
      ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
        if is_group && ui.button(self.i18n.tr("ui-layer-ungroup")).on_hover_text("Ctrl+Shift+G").clicked() {
          action = Some(LayerAction::Ungroup(index));
        }
        if ui.button(self.i18n.tr("ui-layer-group")).on_hover_text("Ctrl+G").clicked() {
          action = Some(LayerAction::Group(index));
        }
      });
    });

    match action {
      Some(LayerAction::Duplicate(i)) => self.doc.duplicate_layer(i),
      Some(LayerAction::MergeDown(i)) => self.doc.merge_down(i),
      Some(LayerAction::Delete(i)) => self.doc.delete_layer(i),
      Some(LayerAction::Group(i)) => self.doc.group_layer(i),
      Some(LayerAction::Ungroup(i)) => self.doc.ungroup(i),
      None => false,
    };
  }
}

/// A layer panel change that adds, removes or reorders layers, applied after drawing.
enum LayerAction {
  Duplicate(usize),
  MergeDown(usize),
  Delete(usize),
  Group(usize),
  Ungroup(usize),
}
