//! The Photoshop-like layout: a menu bar on top (File, Edit, Image, Layer, Select, Filter,
//! View, Window, Help), the tools on the left and the dock of windows on the right (tool options,
//! color, brushes, layers).

use eframe::egui::{self, Align, Color32, Frame, Layout, Margin, Pos2, RichText, Sense, Stroke, Vec2, vec2};
use rustcreate_core::BlendMode;

use super::icons::{self, Icon};
use super::preferences::Page;
use super::studio;
use super::theme;
use crate::canvas::ColorDrop;
use crate::guides::Kind;
use crate::library::{CATEGORIES, MINE, Name};
use crate::settings::Panels;
use crate::tools::{SelectKind, Tool};
use crate::{App, Export, Pending};
use rustcreate_core::{GradientKind, Symmetry};

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
  /// Photoshop's menu bar, the open documents and the settings button.
  pub(crate) fn menu_bar(&mut self, ui: &mut egui::Ui) {
    let palette = theme::palette(ui);
    egui::Panel::top("menu-bar").exact_size(theme::BAR_HEIGHT).frame(Frame::NONE.fill(palette.chrome).inner_margin(Margin::symmetric(8, 4))).show(
      ui,
      |ui| {
        egui::MenuBar::new().ui(ui, |ui| {
          ui.menu_button(self.i18n.tr("ui-menu-file"), |ui| self.file_menu(ui));
          ui.menu_button(self.i18n.tr("ui-menu-edit"), |ui| self.edit_menu(ui));
          ui.menu_button(self.i18n.tr("ui-menu-image"), |ui| self.image_menu(ui));
          ui.menu_button(self.i18n.tr("ui-menu-layer"), |ui| self.layer_menu(ui));
          ui.menu_button(self.i18n.tr("ui-menu-select"), |ui| self.select_menu(ui));
          ui.menu_button(self.i18n.tr("ui-menu-filter"), |ui| self.adjustments(ui, 6..usize::MAX));
          ui.menu_button(self.i18n.tr("ui-menu-view"), |ui| self.view_menu(ui));
          ui.menu_button(self.i18n.tr("ui-menu-window"), |ui| self.window_menu(ui));
          ui.menu_button(self.i18n.tr("ui-menu-help"), |ui| self.help_menu(ui));
          ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if ui.button(self.i18n.tr("ui-preferences")).on_hover_text("Ctrl+K").clicked() {
              self.preferences = Some(Page::General);
            }
            // The open documents, after the menus.
            if self.tabs.len() > 1 {
              ui.add_space(12.0);
              ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                ui.add_space(24.0);
                self.tab_strip(ui);
              });
            }
          });
        });
      },
    );
  }

  fn tab_strip(&mut self, ui: &mut egui::Ui) {
    let palette = theme::palette(ui);
    let mut action = None;
    egui::ScrollArea::horizontal().auto_shrink([false, true]).show(ui, |ui| {
      ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 2.0;
        for (index, (name, dirty)) in self.tab_names().into_iter().enumerate() {
          let shown = index == self.tab;
          let fill = if shown { palette.control } else { Color32::TRANSPARENT };
          Frame::NONE.fill(fill).corner_radius(8).inner_margin(Margin { left: 10, right: 4, top: 3, bottom: 3 }).show(ui, |ui| {
            ui.horizontal(|ui| {
              ui.spacing_mut().item_spacing.x = 4.0;
              let text = if dirty { format!("{name} •") } else { name };
              let color = if shown { palette.ink } else { palette.muted };
              let label = ui.add(egui::Label::new(RichText::new(text).color(color)).truncate().sense(Sense::click()));
              if label.clicked() {
                action = Some((index, false));
              }
              let close =
                ui.add(egui::Button::new(RichText::new("×").color(palette.muted)).frame(false)).on_hover_text(self.i18n.tr("ui-file-close"));
              if close.clicked() {
                action = Some((index, true));
              }
            });
          });
        }
      });
    });
    if let Some((index, close)) = action {
      self.switch_tab(index);
      if close {
        self.request(Pending::Home);
      }
    }
  }

  /// Shows a window of the right dock (and saves that it is open).
  fn show_window(&mut self, set: fn(&mut Panels)) {
    let mut settings = self.settings.clone();
    set(&mut settings.panels);
    if settings != self.settings {
      self.apply(settings);
    }
  }

  /// Undo; while transforming it cancels the transform, like Ctrl+Z does.
  fn undo(&mut self) {
    if self.canvas.transforming() {
      self.canvas.end_transform(&mut self.doc, false)
    } else {
      self.doc.undo();
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
      self.show_window(|p| p.color = true);
    }
  }

  /// Photoshop's toolbar on the left: the tools, the color, the brush size and opacity, and
  /// undo/redo. Clicking the active tool again opens its window in the dock.
  pub(crate) fn toolbar(&mut self, ui: &mut egui::Ui) {
    let palette = theme::palette(ui);
    egui::Panel::left("toolbar")
      .exact_size(theme::SIDEBAR_WIDTH)
      .resizable(false)
      // The same margin on both sides keeps the sliders and buttons on the bar's center line.
      .frame(Frame::NONE.fill(palette.chrome).inner_margin(Margin::symmetric(8, 10)))
      .show(ui, |ui| {
        egui::ScrollArea::vertical().scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden).show(ui, |ui| {
          ui.vertical_centered(|ui| {
            ui.spacing_mut().item_spacing.y = 4.0;
            for (tool, icon, tooltip) in [
              (Tool::Brush, Icon::Brush, "ui-tooltip-brush"),
              (Tool::Smudge, Icon::Smudge, "ui-tooltip-smudge"),
              (Tool::Eraser, Icon::Eraser, "ui-tooltip-eraser"),
              (Tool::Fill, Icon::Bucket, "ui-tooltip-bucket"),
              (Tool::Gradient, Icon::Gradient, "ui-tooltip-gradient"),
              (Tool::Select, Icon::Selection, "ui-selection"),
              (Tool::Text, Icon::Text, "ui-tooltip-text"),
            ] {
              let active = self.canvas.tools.tool == tool;
              if icons::button(ui, icon, active, true, &self.i18n.tr(tooltip)).clicked() {
                if active && tool != Tool::Text {
                  if matches!(tool, Tool::Brush | Tool::Smudge | Tool::Eraser) {
                    self.show_window(|p| p.brushes = true);
                  } else {
                    self.show_window(|p| p.options = true);
                  }
                }
                (self.canvas.tools.tool, self.canvas.tools.hand) = (tool, false);
              }
            }
            // Free Transform: a second click applies it.
            let transforming = self.canvas.transforming();
            if icons::button(ui, Icon::Transform, transforming, true, &self.i18n.tr("ui-transform")).clicked() {
              if transforming { self.canvas.end_transform(&mut self.doc, true) } else { self.canvas.start_transform(&mut self.doc) }
            }
            let picking = self.canvas.tools.picking;
            if icons::button(ui, Icon::Eyedropper, picking, true, &self.i18n.tr("ui-eyedropper")).clicked() {
              self.canvas.tools.picking = !picking;
            }
            ui.add_space(6.0);
            self.color_button(ui);
            ui.add_space(12.0);
            ui.spacing_mut().slider_width = 130.0;
            let brush = self.canvas.tools.brush_mut();
            ui.put(centered_slider(ui), egui::Slider::new(&mut brush.size, 1.0..=1000.0).logarithmic(true).vertical().show_value(false))
              .on_hover_text(self.i18n.tr_args("ui-size", &[("size", (brush.size.round() as i64).into())]));
            let size = brush.size.round() as i64;
            ui.label(RichText::new(self.i18n.tr_args("ui-size-value", &[("size", size.into())])).small().color(palette.muted));
            ui.add_space(8.0);
            let brush = self.canvas.tools.brush_mut();
            let mut percent = brush.opacity * 100.0;
            let opacity = ui.put(centered_slider(ui), egui::Slider::new(&mut percent, 1.0..=100.0).vertical().show_value(false));
            if opacity.changed() {
              brush.opacity = percent / 100.0;
            }
            opacity.on_hover_text(self.i18n.tr_args("ui-opacity", &[("opacity", (percent.round() as i64).into())]));
            let percent = percent.round() as i64;
            ui.label(RichText::new(self.i18n.tr_args("ui-opacity-value", &[("opacity", percent.into())])).small().color(palette.muted));
            ui.add_space(12.0);
            let (can_undo, can_redo) = (self.doc.history().can_undo() || self.canvas.transforming(), self.doc.history().can_redo());
            if icons::button(ui, Icon::Undo, false, can_undo, &self.i18n.tr("ui-undo")).clicked() {
              self.undo();
            }
            if icons::button(ui, Icon::Redo, false, can_redo, &self.i18n.tr("ui-redo")).clicked() {
              self.doc.redo();
            }
          });
        });
      });
  }

  /// Photoshop's panels on the right: tool options, color, brushes and layers, each a window
  /// that folds and that the Window menu shows or hides. Resizable; the content follows its width.
  pub(crate) fn dock(&mut self, ui: &mut egui::Ui) {
    let panels = self.settings.panels;
    if !(panels.options || panels.color || panels.brushes || panels.layers) {
      return;
    }
    let palette = theme::palette(ui);
    egui::Panel::right("dock")
      .resizable(true)
      .default_size(320.0)
      .size_range(260.0..=560.0)
      .frame(Frame::NONE.fill(palette.chrome).inner_margin(Margin::same(10)))
      .show(ui, |ui| {
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
          if panels.options {
            window(self.i18n.tr("ui-window-options"), "options").show(ui, |ui| self.tool_options(ui));
            ui.separator();
          }
          if panels.color {
            window(self.i18n.tr("ui-window-color"), "color").show(ui, |ui| self.color_panel(ui));
            ui.separator();
          }
          if panels.brushes {
            window(self.i18n.tr("ui-brushes"), "brushes").show(ui, |ui| self.brushes_panel(ui));
            ui.separator();
          }
          if panels.layers {
            window(self.i18n.tr("ui-layers"), "layers").show(ui, |ui| {
              // Layer changes wait for the transform or the text being typed to end.
              let busy = self.canvas.transforming() || self.canvas.typing();
              if busy {
                ui.label(RichText::new(self.i18n.tr("ui-layers-busy")).small().color(palette.muted));
              }
              ui.add_enabled_ui(!busy, |ui| self.layers_panel(ui));
            });
          }
        });
      });
  }

  /// The current tool's settings (Photoshop's options bar).
  fn tool_options(&mut self, ui: &mut egui::Ui) {
    match self.canvas.tools.tool {
      Tool::Fill | Tool::Gradient => self.fill_panel(ui),
      Tool::Select => self.select_panel(ui),
      Tool::Brush | Tool::Smudge | Tool::Eraser => {
        let selected = self.canvas.tools.selected();
        ui.horizontal(|ui| {
          ui.label(RichText::new(self.brush_name(selected)).strong());
          ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if ui.button(self.i18n.tr("ui-studio")).clicked() {
              self.studio = true;
            }
          });
        });
        let brush = self.canvas.tools.brush_mut();
        egui::Grid::new("brush-options").num_columns(2).spacing(vec2(10.0, 6.0)).show(ui, |ui| {
          ui.label(self.i18n.tr("ui-brush-size"));
          ui.add(egui::Slider::new(&mut brush.size, 1.0..=1000.0).logarithmic(true).suffix(" px").fixed_decimals(0));
          ui.end_row();
          ui.label(self.i18n.tr("ui-layer-opacity"));
          let mut percent = brush.opacity * 100.0;
          if ui.add(egui::Slider::new(&mut percent, 1.0..=100.0).suffix(" %").fixed_decimals(0)).changed() {
            brush.opacity = percent / 100.0;
          }
          ui.end_row();
        });
      }
      Tool::Text => {
        ui.label(RichText::new(self.i18n.tr("ui-options-none")).color(theme::palette(ui).muted));
      }
    }
  }

  pub(crate) fn title(&self, ui: &mut egui::Ui, id: &str) {
    ui.label(RichText::new(self.i18n.tr(id)).strong().size(15.0));
    ui.add_space(6.0);
  }

  fn edit_menu(&mut self, ui: &mut egui::Ui) {
    let (can_undo, can_redo) = (self.doc.history().can_undo() || self.canvas.transforming(), self.doc.history().can_redo());
    if item(ui, self.i18n.tr("ui-undo"), "Ctrl+Z", can_undo) {
      self.undo();
    }
    if item(ui, self.i18n.tr("ui-redo"), "Ctrl+Shift+Z", can_redo) {
      self.doc.redo();
    }
    ui.separator();
    if item(ui, self.i18n.tr("ui-transform"), "Ctrl+T", true) {
      if self.canvas.transforming() { self.canvas.end_transform(&mut self.doc, true) } else { self.canvas.start_transform(&mut self.doc) }
    }
    ui.separator();
    if item(ui, self.i18n.tr("ui-preferences"), "Ctrl+K", true) {
      self.preferences = Some(Page::General);
    }
  }

  fn image_menu(&mut self, ui: &mut egui::Ui) {
    ui.menu_button(self.i18n.tr("ui-adjustments"), |ui| self.adjustments(ui, 0..6));
    ui.separator();
    if item(ui, self.i18n.tr("ui-canvas-size"), "Ctrl+Alt+C", true) {
      self.open_resize(false);
    }
    if item(ui, self.i18n.tr("ui-image-size"), "Ctrl+Alt+I", true) {
      self.open_resize(true);
    }
    if item(ui, self.i18n.tr("ui-canvas-crop"), "", self.doc.selection().is_some()) {
      self.crop();
    }
  }

  fn layer_menu(&mut self, ui: &mut egui::Ui) {
    let busy = self.canvas.transforming() || self.canvas.typing();
    let index = self.doc.active();
    let (count, group, masked) = (self.doc.layers().len(), self.doc.layers()[index].is_group(), self.doc.layers()[index].mask.is_some());
    let mut action = None;
    ui.add_enabled_ui(!busy, |ui| {
      if item(ui, self.i18n.tr("ui-layer-new"), "Ctrl+Shift+N", true) {
        action = Some(LayerAction::New);
      }
      if item(ui, self.i18n.tr("ui-layer-duplicate"), "Ctrl+J", true) {
        action = Some(LayerAction::Duplicate(index));
      }
      if item(ui, self.i18n.tr("ui-layer-delete"), "", count > 1) {
        action = Some(LayerAction::Delete(index));
      }
      if item(ui, self.i18n.tr("ui-layer-merge-down"), "Ctrl+E", index > 0) {
        action = Some(LayerAction::MergeDown(index));
      }
      ui.separator();
      if item(ui, self.i18n.tr("ui-layer-group"), "Ctrl+G", true) {
        action = Some(LayerAction::Group(index));
      }
      if item(ui, self.i18n.tr("ui-layer-ungroup"), "Ctrl+Shift+G", group) {
        action = Some(LayerAction::Ungroup(index));
      }
      ui.separator();
      if item(ui, self.i18n.tr("ui-layer-forward"), "Ctrl+]", index + 1 < count) {
        action = Some(LayerAction::Up(index));
      }
      if item(ui, self.i18n.tr("ui-layer-backward"), "Ctrl+[", true) {
        action = Some(LayerAction::Down(index));
      }
      ui.separator();
      if item(ui, self.i18n.tr("ui-mask-add"), "", !masked) {
        action = Some(LayerAction::AddMask(index));
      }
    });
    if let Some(action) = action {
      self.layer_action(action);
    }
  }

  fn select_menu(&mut self, ui: &mut egui::Ui) {
    let selected = self.doc.selection().is_some();
    if item(ui, self.i18n.tr("ui-select-all"), "Ctrl+A", true) {
      self.doc.select_all();
    }
    if item(ui, self.i18n.tr("ui-select-none"), "Ctrl+D", selected) {
      self.doc.deselect();
    }
    if item(ui, self.i18n.tr("ui-select-invert"), "Ctrl+Shift+I", selected) {
      self.doc.invert_selection();
    }
  }

  fn view_menu(&mut self, ui: &mut egui::Ui) {
    self.guides_section(ui);
    ui.separator();
    if item(ui, self.i18n.tr("ui-view-hide"), "Tab", true) {
      self.hidden = true;
    }
  }

  fn window_menu(&mut self, ui: &mut egui::Ui) {
    let mut panels = self.settings.panels;
    ui.checkbox(&mut panels.options, self.i18n.tr("ui-window-options"));
    ui.checkbox(&mut panels.color, self.i18n.tr("ui-window-color"));
    ui.checkbox(&mut panels.brushes, self.i18n.tr("ui-brushes"));
    ui.checkbox(&mut panels.layers, self.i18n.tr("ui-layers"));
    if panels != self.settings.panels {
      let mut settings = self.settings.clone();
      settings.panels = panels;
      self.apply(settings);
    }
    ui.separator();
    // Animation Assist (Procreate): every top-level layer or group becomes a frame.
    let mut animation = self.doc.animation.enabled;
    if ui.checkbox(&mut animation, self.i18n.tr("ui-animation-assist")).on_hover_text(self.i18n.tr("ui-animation-help")).changed() {
      self.doc.animation.enabled = animation;
      self.stop_playback();
    }
    if item(ui, self.i18n.tr("ui-studio"), "", true) {
      self.studio = true;
    }
  }

  fn help_menu(&mut self, ui: &mut egui::Ui) {
    if item(ui, self.i18n.tr("ui-about"), "", true) {
      self.preferences = Some(Page::About);
    }
    if item(ui, self.i18n.tr("ui-update-check-now"), "", true) {
      self.updates.check_now();
      self.preferences = Some(Page::About);
    }
    ui.separator();
    ui.hyperlink_to(self.i18n.tr("ui-about-repository"), env!("CARGO_PKG_REPOSITORY"));
  }

  /// Photoshop's File menu: new, open (and recent files), close, save, save as, save a copy,
  /// revert, place, export and quit.
  fn file_menu(&mut self, ui: &mut egui::Ui) {
    if item(ui, self.i18n.tr("ui-file-new"), "Ctrl+N", true) {
      self.open_home(true);
    }
    if item(ui, self.i18n.tr("ui-file-open"), "Ctrl+O", true) {
      self.request(Pending::Open(None));
    }
    let muted = theme::palette(ui).muted;
    ui.menu_button(self.i18n.tr("ui-file-recent"), |ui| {
      if self.settings.recent.is_empty() {
        ui.label(RichText::new(self.i18n.tr("ui-file-recent-none")).color(muted));
      }
      for path in self.settings.recent.clone() {
        let name = path.file_stem().map_or_else(|| path.display().to_string(), |n| n.to_string_lossy().into_owned());
        let response = ui.button(name).on_hover_text(path.display().to_string());
        if response.clicked() {
          self.request(Pending::Open(Some(path.clone())));
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
      self.request(Pending::Home);
    }
    if item(ui, self.i18n.tr("ui-file-save"), "Ctrl+S", true) {
      self.save(false);
    }
    if item(ui, self.i18n.tr("ui-file-save-as"), "Ctrl+Shift+S", true) {
      self.save(true);
    }
    if item(ui, self.i18n.tr("ui-file-save-all"), "", self.tabs.len() > 1) {
      self.save_all();
    }
    if item(ui, self.i18n.tr("ui-file-save-copy"), "Ctrl+Alt+S", true) {
      self.save_copy();
    }
    if item(ui, self.i18n.tr("ui-file-revert"), "F12", saved && self.dirty()) {
      self.request(Pending::Revert);
    }
    ui.separator();
    if item(ui, self.i18n.tr("ui-file-place"), "", true) {
      self.place();
    }
    for (format, id, shortcut) in
      [(Export::Png, "ui-file-export-png", "Ctrl+Alt+Shift+W"), (Export::Jpeg, "ui-file-export-jpeg", ""), (Export::Webp, "ui-file-export-webp", "")]
    {
      if item(ui, self.i18n.tr(id), shortcut, true) {
        self.export(format);
      }
    }
    // Animations: with Animation Assist on.
    let animated = self.doc.animation.enabled;
    for (format, id) in [
      (Export::Gif, "ui-file-export-gif"),
      (Export::AnimatedPng, "ui-file-export-apng"),
      (Export::PngSequence, "ui-file-export-sequence"),
      (Export::Mp4, "ui-file-export-mp4"),
    ] {
      let response = ui.add_enabled(animated, egui::Button::new(self.i18n.tr(id)));
      if response.on_disabled_hover_text(self.i18n.tr("ui-file-animation-only")).clicked() {
        self.export(format);
      }
    }
    ui.separator();
    if item(ui, self.i18n.tr("ui-file-quit"), "Ctrl+Q", true) {
      self.request(Pending::Close);
    }
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
    let palette = theme::palette(ui);
    let selected = self.canvas.tools.selected();
    let category = *self.brush_category.get_or_insert(self.canvas.tools.library.entries[selected].category);
    ui.horizontal_top(|ui| {
      ui.vertical(|ui| {
        ui.set_width(100.0);
        for (index, id) in CATEGORIES.iter().enumerate() {
          if ui.selectable_label(category == index, self.i18n.tr(id)).clicked() {
            self.brush_category = Some(index);
          }
        }
      });
      ui.separator();
      ui.vertical(|ui| {
        // As wide as the dock leaves, so it never spills out when the window grows or shrinks.
        let wide = (ui.available_width() - 16.0).max(80.0);
        egui::ScrollArea::vertical().id_salt("brush-list").max_height(320.0).show(ui, |ui| {
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
                ui.set_width(wide);
                ui.label(RichText::new(self.brush_name(index)).strong());
                // The preview keeps its 256 × 52 shape.
                ui.add(egui::Image::new(&thumbnail).fit_to_exact_size(vec2(wide, wide * 52.0 / 256.0)));
              });
            if ui.interact(response.response.rect, ui.scope_id().with(("brush", index)), Sense::click()).clicked() {
              self.canvas.tools.select(index);
            }
          }
        });
      });
    });
    ui.separator();
    ui.horizontal_wrapped(|ui| {
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
      if icons::button(ui, Icon::Plus, false, true, &self.i18n.tr("ui-layer-new")).clicked() {
        self.doc.add_layer();
      }
      ui.add(egui::Label::new(RichText::new(self.i18n.tr("ui-layers-drag")).small().color(theme::palette(ui).muted)).wrap());
    });
    ui.add_space(4.0);
    // Changes that add, remove or reorder layers wait until the panel is drawn: indices taken
    // while drawing would point past the end or at another layer.
    let mut action: Option<LayerAction> = None;
    let palette = theme::palette(ui);
    let tall = (ui.ctx().content_rect().height() * 0.5).clamp(200.0, 460.0);
    egui::ScrollArea::vertical().id_salt("layer-list").max_height(tall).auto_shrink([false, true]).show(ui, |ui| {
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
        // A group says how many layers it holds; the rows under it are indented behind a line.
        if group {
          let content = self.doc.layers().iter().filter(|l| l.parent() == Some(id)).count();
          shown.push_str(&format!("  ({content})"));
        }
        let depth = self.doc.depth(index);
        let indent = depth as f32 * 18.0;
        let active = index == self.doc.active();

        // The whole row selects; the eye and the lock sit on top of it.
        // Dragging a row onto another moves the layer there (a group with its content).
        let (full, row) = ui.allocate_exact_size(vec2(ui.available_width(), 40.0), Sense::click_and_drag());
        row.dnd_set_drag_payload(LayerDrag(index));
        let rect = full.with_min_x(full.left() + indent);
        // One accent line per level the row is nested, joining it to its group above.
        for level in 0..depth {
          let x = full.left() + 7.0 + level as f32 * 18.0;
          ui.painter().vline(x, full.y_range().expand(3.0), Stroke::new(2.0, theme::ACCENT.gamma_multiply(0.6)));
        }
        let fill = if active {
          palette.selected
        } else if row.hovered() {
          palette.control.gamma_multiply(1.4)
        } else if group {
          // Groups stand out from plain layers.
          palette.control.lerp_to_gamma(theme::ACCENT, 0.18)
        } else {
          palette.control
        };
        ui.painter().rect_filled(rect, 8, fill);
        if group {
          ui.painter().rect_stroke(rect, 8, Stroke::new(1.0, theme::ACCENT.gamma_multiply(0.7)), egui::StrokeKind::Inside);
        }
        if let Some(from) = row.dnd_hover_payload::<LayerDrag>().map(|d| d.0).filter(|&from| from != index) {
          if group {
            // Dropped on a group: it goes inside, so the whole row lights up.
            ui.painter().rect_stroke(rect, 8, Stroke::new(3.0, theme::ACCENT), egui::StrokeKind::Inside);
          } else {
            // Otherwise a line where it lands, at this row's level (inside its group or not).
            let y = if from < index { rect.top() } else { rect.bottom() };
            ui.painter().hline(rect.x_range(), y, Stroke::new(3.0, theme::ACCENT));
          }
        }
        if let Some(from) = row.dnd_release_payload::<LayerDrag>().map(|d| d.0).filter(|&from| from != index) {
          action = Some(LayerAction::Move(from, index));
        }
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
        let badge = egui::Rect::from_center_size(egui::pos2(rect.left() + 22.0, rect.center().y), Vec2::splat(20.0));
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
          // Enter (or a click elsewhere) ends it: focus is only asked for while the field has not
          // given it up, or it would never be lost and the field never close.
          let done = edit.lost_focus() || ui.input(|i| i.key_pressed(egui::Key::Enter));
          if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.renaming = None;
          } else if done {
            let (_, typed) = self.renaming.take().expect("renaming");
            if !typed.trim().is_empty() && typed != name {
              self.doc.rename_layer(index, typed.trim());
            }
          } else {
            edit.request_focus();
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

    if let Some(action) = action {
      self.layer_action(action);
    }
  }

  fn layer_action(&mut self, action: LayerAction) {
    match action {
      LayerAction::New => {
        self.doc.add_layer();
      }
      LayerAction::Duplicate(i) => {
        self.doc.duplicate_layer(i);
      }
      LayerAction::MergeDown(i) => {
        self.doc.merge_down(i);
      }
      LayerAction::Delete(i) => {
        self.doc.delete_layer(i);
      }
      LayerAction::Group(i) => {
        self.doc.group_layer(i);
      }
      LayerAction::Ungroup(i) => {
        self.doc.ungroup(i);
      }
      LayerAction::Up(i) => {
        self.doc.move_up(i);
      }
      LayerAction::Down(i) => {
        self.doc.move_down(i);
      }
      LayerAction::Move(from, to) => {
        self.doc.drop_layer(from, to);
      }
      LayerAction::AddMask(i) => {
        self.doc.add_mask(i);
      }
    }
  }
}

/// A layer change that adds, removes or reorders layers, applied after drawing.
enum LayerAction {
  New,
  Duplicate(usize),
  MergeDown(usize),
  Delete(usize),
  Group(usize),
  Ungroup(usize),
  Up(usize),
  Down(usize),
  /// Dropped on another row: from, to.
  Move(usize, usize),
  AddMask(usize),
}

/// The layer row being dragged in the layers panel.
struct LayerDrag(usize);

/// A menu entry with its shortcut.
fn item(ui: &mut egui::Ui, text: String, shortcut: &str, enabled: bool) -> bool {
  ui.add_enabled(enabled, egui::Button::new(text).shortcut_text(shortcut)).clicked()
}

/// A window of the dock: a header that folds it.
fn window(title: String, id: &str) -> egui::CollapsingHeader {
  egui::CollapsingHeader::new(RichText::new(title).strong().size(14.0)).id_salt(id).default_open(true)
}

/// Where a sidebar slider goes: on the bar's center line (egui lays a vertical slider out from
/// the left otherwise).
fn centered_slider(ui: &egui::Ui) -> egui::Rect {
  let (width, height) = (ui.spacing().interact_size.y, ui.spacing().slider_width);
  egui::Rect::from_min_size(egui::pos2(ui.max_rect().center().x - width / 2.0, ui.cursor().top()), vec2(width, height))
}
