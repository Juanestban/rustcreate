//! Brush Studio: every parameter of the selected brush, grouped like Procreate's (shape, grain,
//! dynamics, stroke), with a live preview painted by the real brush engine.

use eframe::egui::{self, Color32, ColorImage, RichText, TextureHandle, TextureOptions};
use rustcreate_core::{Brush, Document, Grain, Tip};

use super::theme;
use crate::App;
use crate::library::Name;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Section {
  #[default]
  Shape,
  Grain,
  Dynamics,
  Stroke,
}

/// The preview image and the brush it shows, re-rendered only when the brush changes.
pub struct Preview {
  brush: Brush,
  dark: bool,
  texture: TextureHandle,
}

const PREVIEW: (u32, u32) = (256, 96);

fn tip_name(tip: Tip) -> &'static str {
  match tip {
    Tip::Round => "ui-tip-round",
    Tip::Square => "ui-tip-square",
    Tip::Chalk => "ui-tip-chalk",
    Tip::Bristles => "ui-tip-bristles",
    Tip::Splatter => "ui-tip-splatter",
    Tip::Image(_) => "ui-tip-imported",
  }
}

fn grain_name(grain: Grain) -> &'static str {
  match grain {
    Grain::None => "ui-grain-none",
    Grain::Paper => "ui-grain-paper",
    Grain::Canvas => "ui-grain-canvas",
    Grain::Noise => "ui-grain-noise",
    Grain::Image(_) => "ui-grain-imported",
  }
}

/// An S-curve that goes light → heavy → light, so pressure, taper and texture all show.
pub fn render(brush: Brush, ink: Color32, (w, h): (u32, u32)) -> ColorImage {
  let mut doc = Document::new(w, h);
  // Big brushes would only show a blob: scale them to fit the strip.
  let brush = Brush { size: brush.size.min(h as f32 * 0.45), ..brush };
  if let Some(mut stroke) = doc.begin_stroke(brush, [ink.r(), ink.g(), ink.b()], false) {
    for i in 0..=120 {
      let t = i as f32 / 120.0;
      let x = 16.0 + t * (w as f32 - 32.0);
      let y = h as f32 / 2.0 + (t * std::f32::consts::TAU).sin() * h as f32 * 0.22;
      stroke.add(&mut doc, x, y, (t * std::f32::consts::PI).sin().max(0.05));
    }
    stroke.finish(&mut doc);
  }
  let tile = doc.composite_tile(0, 0).unwrap_or_else(|| vec![0; 256 * 256 * 4]);
  ColorImage::from_rgba_premultiplied([w as usize, h as usize], &tile[..(w * h * 4) as usize])
}

impl App {
  pub(crate) fn brush_studio(&mut self, ctx: &egui::Context) {
    if !self.studio {
      return;
    }
    let index = self.canvas.tools.selected();
    let mut open = true;
    let title = format!("{} — {}", self.i18n.tr("ui-studio"), self.brush_name(index));
    egui::Window::new(title).id(egui::Id::unique("brush-studio")).open(&mut open).resizable(false).collapsible(false).show(ctx, |ui| {
      ui.set_width(360.0);
      self.studio_preview(ui);
      if let Name::Custom(name) = &mut self.canvas.tools.library.entries[index].name {
        ui.horizontal(|ui| {
          ui.label(self.i18n.tr("ui-studio-name"));
          ui.text_edit_singleline(name);
        });
      }
      ui.add_space(6.0);
      ui.horizontal(|ui| {
        for (section, id) in [
          (Section::Shape, "ui-studio-shape"),
          (Section::Grain, "ui-studio-grain"),
          (Section::Dynamics, "ui-studio-dynamics"),
          (Section::Stroke, "ui-studio-stroke"),
        ] {
          ui.selectable_value(&mut self.studio_section, section, self.i18n.tr(id));
        }
      });
      ui.separator();
      let i18n = &self.i18n;
      let brush = self.canvas.tools.brush_mut();
      // 0..=1 values shown as percentages.
      let percent = |ui: &mut egui::Ui, value: &mut f32, max: f32| {
        ui.add(
          egui::Slider::new(value, 0.0..=max)
            .custom_formatter(|v, _| format!("{:.0} %", v * 100.0))
            .custom_parser(|s| s.trim_end_matches([' ', '%']).parse::<f64>().ok().map(|v| v / 100.0)),
        );
      };
      egui::Grid::new("studio").num_columns(2).spacing([12.0, 8.0]).show(ui, |ui| {
        let row = |ui: &mut egui::Ui, id: &str, add: &mut dyn FnMut(&mut egui::Ui)| {
          ui.label(i18n.tr(id));
          add(ui);
          ui.end_row();
        };
        match self.studio_section {
          Section::Shape => {
            row(ui, "ui-studio-tip", &mut |ui| {
              egui::ComboBox::from_id_salt("tip").selected_text(i18n.tr(tip_name(brush.tip))).show_ui(ui, |ui| {
                for tip in Tip::ALL {
                  ui.selectable_value(&mut brush.tip, tip, i18n.tr(tip_name(tip)));
                }
              });
            });
            row(ui, "ui-studio-size", &mut |ui| {
              ui.add(egui::Slider::new(&mut brush.size, 1.0..=1000.0).logarithmic(true).suffix(" px").fixed_decimals(0));
            });
            row(ui, "ui-studio-hardness", &mut |ui| percent(ui, &mut brush.hardness, 1.0));
            row(ui, "ui-studio-spacing", &mut |ui| percent(ui, &mut brush.spacing, 2.0));
            row(ui, "ui-studio-angle", &mut |ui| {
              ui.add(egui::Slider::new(&mut brush.angle, -180.0..=180.0).suffix("°").fixed_decimals(0));
            });
            row(ui, "ui-studio-follow", &mut |ui| {
              ui.checkbox(&mut brush.follow, "");
            });
            row(ui, "ui-studio-angle-jitter", &mut |ui| percent(ui, &mut brush.angle_jitter, 1.0));
            row(ui, "ui-studio-scatter", &mut |ui| percent(ui, &mut brush.scatter, 2.0));
            row(ui, "ui-studio-size-jitter", &mut |ui| percent(ui, &mut brush.size_jitter, 1.0));
          }
          Section::Grain => {
            row(ui, "ui-studio-texture", &mut |ui| {
              egui::ComboBox::from_id_salt("grain").selected_text(i18n.tr(grain_name(brush.grain))).show_ui(ui, |ui| {
                for grain in Grain::ALL {
                  ui.selectable_value(&mut brush.grain, grain, i18n.tr(grain_name(grain)));
                }
              });
            });
            row(ui, "ui-studio-grain-scale", &mut |ui| {
              ui.add(egui::Slider::new(&mut brush.grain_scale, 0.25..=8.0).logarithmic(true).suffix("×"));
            });
            row(ui, "ui-studio-grain-depth", &mut |ui| percent(ui, &mut brush.grain_depth, 1.0));
          }
          Section::Dynamics => {
            row(ui, "ui-studio-opacity", &mut |ui| percent(ui, &mut brush.opacity, 1.0));
            row(ui, "ui-studio-flow", &mut |ui| percent(ui, &mut brush.flow, 1.0));
            row(ui, "ui-studio-buildup", &mut |ui| {
              ui.checkbox(&mut brush.buildup, "");
            });
            row(ui, "ui-studio-pressure-size", &mut |ui| percent(ui, &mut brush.pressure_size, 1.0));
            row(ui, "ui-studio-pressure-flow", &mut |ui| percent(ui, &mut brush.pressure_flow, 1.0));
            row(ui, "ui-studio-wet-mix", &mut |ui| percent(ui, &mut brush.wet_mix, 1.0));
            row(ui, "ui-studio-charge", &mut |ui| percent(ui, &mut brush.charge, 1.0));
          }
          Section::Stroke => {
            row(ui, "ui-brush-streamline", &mut |ui| percent(ui, &mut brush.streamline, 0.95));
            row(ui, "ui-brush-motion-filter", &mut |ui| {
              percent(ui, &mut brush.motion_filter, 1.0);
            });
            row(ui, "ui-brush-string", &mut |ui| {
              ui.add(egui::Slider::new(&mut brush.string, 0.0..=200.0).suffix(" px").fixed_decimals(0))
                .on_hover_text(i18n.tr("ui-brush-string-help"));
            });
            row(ui, "ui-brush-taper-start", &mut |ui| {
              ui.add(egui::Slider::new(&mut brush.taper_start, 0.0..=500.0).suffix(" px").fixed_decimals(0));
            });
            row(ui, "ui-brush-taper-end", &mut |ui| {
              ui.add(egui::Slider::new(&mut brush.taper_end, 0.0..=500.0).suffix(" px").fixed_decimals(0));
            });
          }
        }
      });
      ui.separator();
      // Built-ins go back to how they ship; the user's own brushes have nothing to reset to.
      if let Some(original) = self.canvas.tools.library.original(index)
        && ui.add_enabled(original != self.canvas.tools.library.entries[index].brush, egui::Button::new(self.i18n.tr("ui-studio-reset"))).clicked()
      {
        self.canvas.tools.library.entries[index].brush = original;
      }
    });
    self.studio &= open;
  }

  fn studio_preview(&mut self, ui: &mut egui::Ui) {
    let brush = self.canvas.tools.current().0;
    let palette = theme::palette(ui);
    let dark = ui.visuals().dark_mode;
    if self.preview.as_ref().is_none_or(|p| p.brush != brush || p.dark != dark) {
      let image = render(brush, palette.ink, PREVIEW);
      let texture = ui.ctx().load_texture("brush-preview", image, TextureOptions::LINEAR);
      self.preview = Some(Preview { brush, dark, texture });
    }
    let texture = &self.preview.as_ref().expect("just rendered").texture;
    egui::Frame::NONE.fill(palette.backdrop).corner_radius(8).show(ui, |ui| {
      ui.add(egui::Image::new(texture).fit_to_exact_size(egui::vec2(360.0, 360.0 * PREVIEW.1 as f32 / PREVIEW.0 as f32)));
    });
    ui.label(RichText::new(self.i18n.tr("ui-studio-preview-help")).small().color(palette.muted));
  }
}
