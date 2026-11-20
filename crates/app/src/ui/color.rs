//! The color panel: an original hue ring with a saturation/brightness square, the classic picker,
//! color harmonies and the user's palettes.

use std::f32::consts::{FRAC_1_SQRT_2, TAU};

use eframe::egui::{self, Color32, Mesh, Pos2, RichText, Sense, Shape, Stroke, Vec2, ecolor::Hsva};

use super::theme;
use crate::App;
use rustcreate_core::{SwatchFormat, Swatches, export_swatches, import_swatches};

use crate::palettes::Palette;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Tab {
  #[default]
  Disc,
  Classic,
  Harmony,
  Palettes,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Harmony {
  #[default]
  Complementary,
  SplitComplementary,
  Analogous,
  Triadic,
  Tetradic,
}

impl Harmony {
  const ALL: [Harmony; 5] = [Harmony::Complementary, Harmony::SplitComplementary, Harmony::Analogous, Harmony::Triadic, Harmony::Tetradic];

  /// Hue offsets in turns, the picked color first.
  fn offsets(self) -> &'static [f32] {
    match self {
      Harmony::Complementary => &[0.0, 0.5],
      Harmony::SplitComplementary => &[0.0, 5.0 / 12.0, 7.0 / 12.0],
      Harmony::Analogous => &[0.0, -1.0 / 12.0, 1.0 / 12.0],
      Harmony::Triadic => &[0.0, 1.0 / 3.0, 2.0 / 3.0],
      Harmony::Tetradic => &[0.0, 0.25, 0.5, 0.75],
    }
  }

  fn name(self) -> &'static str {
    match self {
      Harmony::Complementary => "ui-harmony-complementary",
      Harmony::SplitComplementary => "ui-harmony-split",
      Harmony::Analogous => "ui-harmony-analogous",
      Harmony::Triadic => "ui-harmony-triadic",
      Harmony::Tetradic => "ui-harmony-tetradic",
    }
  }

  fn colors(self, base: Hsva) -> Vec<Hsva> {
    self.offsets().iter().map(|o| Hsva { h: (base.h + o).rem_euclid(1.0), ..base }).collect()
  }
}

fn rgb(hsv: Hsva) -> [u8; 3] {
  hsv.to_srgb()
}

/// The hue ring and the saturation/brightness square inside it. `marks` are extra hues shown on
/// the ring (harmonies). True when the color changed.
fn disc(ui: &mut egui::Ui, hsv: &mut Hsva, marks: &[f32]) -> bool {
  let size = 236.0;
  let (rect, response) = ui.allocate_exact_size(Vec2::splat(size), Sense::click_and_drag());
  let center = rect.center();
  let (outer, inner) = (size / 2.0, size / 2.0 - 20.0);
  let half = inner * FRAC_1_SQRT_2 - 6.0;
  let mut changed = false;
  if let Some(p) = response.interact_pointer_pos()
    && let Some(origin) = ui.input(|i| i.pointer.press_origin())
  {
    let d = p - center;
    // Where the drag started decides what it changes, so it can leave the ring freely.
    if (origin - center).length() > inner {
      hsv.h = (d.y.atan2(d.x) / TAU).rem_euclid(1.0);
    } else {
      hsv.s = ((d.x + half) / (2.0 * half)).clamp(0.0, 1.0);
      hsv.v = 1.0 - ((d.y + half) / (2.0 * half)).clamp(0.0, 1.0);
    }
    changed = true;
  }

  let painter = ui.painter();
  let mut mesh = Mesh::default();
  let at = |turns: f32, radius: f32| center + radius * Vec2::angled(turns * TAU);
  // Both edges fade out over one physical pixel: a mesh gets none of egui's own anti-aliasing,
  // and a hard edge shows steps around the ring.
  let feather = 1.0 / ui.ctx().pixels_per_point();
  const SEGMENTS: u32 = 128;
  for i in 0..=SEGMENTS {
    let t = i as f32 / SEGMENTS as f32;
    let color = Color32::from(Hsva::new(t, 1.0, 1.0, 1.0));
    for (radius, color) in [(outer, Color32::TRANSPARENT), (outer - feather, color), (inner + feather, color), (inner, Color32::TRANSPARENT)] {
      mesh.colored_vertex(at(t, radius), color);
    }
    if i > 0 {
      let (now, before) = (i * 4, (i - 1) * 4);
      for band in 0..3 {
        mesh.add_triangle(before + band, before + band + 1, now + band);
        mesh.add_triangle(before + band + 1, now + band, now + band + 1);
      }
    }
  }
  // The square, as a grid so the colors blend like HSV and not like RGB corners.
  const GRID: u32 = 16;
  let start = mesh.vertices.len() as u32;
  for j in 0..=GRID {
    for i in 0..=GRID {
      let (s, v) = (i as f32 / GRID as f32, 1.0 - j as f32 / GRID as f32);
      let pos = center + Vec2::new(-half + 2.0 * half * s, -half + 2.0 * half * (1.0 - v));
      mesh.colored_vertex(pos, Color32::from(Hsva::new(hsv.h, s, v, 1.0)));
      if i > 0 && j > 0 {
        let k = start + j * (GRID + 1) + i;
        mesh.add_triangle(k - GRID - 2, k - GRID - 1, k - 1);
        mesh.add_triangle(k - GRID - 1, k - 1, k);
      }
    }
  }
  painter.add(Shape::mesh(mesh));

  let marker = |pos: Pos2, fill: Color32, radius: f32| {
    painter.circle(pos, radius, fill, Stroke::new(2.0, Color32::WHITE));
    painter.circle_stroke(pos, radius + 1.5, Stroke::new(1.0, Color32::from_black_alpha(140)));
  };
  let ring = (outer + inner) / 2.0;
  for &mark in marks {
    marker(at(mark, ring), Color32::from(Hsva::new(mark, 1.0, 1.0, 1.0)), 5.0);
  }
  marker(at(hsv.h, ring), Color32::from(Hsva::new(hsv.h, 1.0, 1.0, 1.0)), 8.0);
  let picked = center + Vec2::new(-half + 2.0 * half * hsv.s, -half + 2.0 * half * (1.0 - hsv.v));
  marker(picked, Color32::from(*hsv), 7.0);
  changed
}

/// A clickable color square, outlined when hovered or when it is the current color.
fn swatch(ui: &mut egui::Ui, [r, g, b]: [u8; 3], size: f32, current: bool) -> egui::Response {
  let (rect, response) = ui.allocate_exact_size(Vec2::splat(size), Sense::click());
  let radius = (size * 0.2).min(6.0);
  ui.painter().rect_filled(rect, radius, Color32::from_rgb(r, g, b));
  // A faint edge so near-panel colors (black on dark, white on light) still show their shape.
  ui.painter().rect_stroke(rect, radius, Stroke::new(1.0, theme::palette(ui).line), egui::StrokeKind::Inside);
  if response.hovered() || current {
    ui.painter().rect_stroke(
      rect.expand(1.5),
      radius + 1.5,
      Stroke::new(2.0, if current { theme::ACCENT } else { ui.visuals().strong_text_color() }),
      egui::StrokeKind::Outside,
    );
  }
  response
}

/// Equal-width buttons in a row, one of them selected.
fn segmented<T: PartialEq + Copy>(ui: &mut egui::Ui, value: &mut T, items: &[(T, String)]) {
  let gap = 4.0;
  let width = (ui.available_width() - gap * (items.len() as f32 - 1.0)) / items.len() as f32;
  ui.horizontal(|ui| {
    ui.spacing_mut().item_spacing.x = gap;
    for (item, label) in items {
      if ui.add(egui::Button::new(label.as_str()).selected(*value == *item).min_size(Vec2::new(width, 30.0))).clicked() {
        *value = *item;
      }
    }
  });
}

impl App {
  pub(crate) fn color_panel(&mut self, ui: &mut egui::Ui) {
    let primary = self.canvas.tools.primary;
    // Keep the hue while saturation or brightness is 0, where RGB forgets it.
    let mut hsv = self.color_hsv.filter(|hsv| rgb(*hsv) == primary).unwrap_or_else(|| Hsva::from_srgb(primary));
    let mut picked = None;

    // The current color large, the secondary beside it (click or X swaps), and its hex code.
    ui.horizontal(|ui| {
      swatch(ui, primary, 48.0, false);
      let secondary = self.canvas.tools.secondary;
      ui.vertical(|ui| {
        ui.add_space(14.0);
        if swatch(ui, secondary, 28.0, false).on_hover_text(self.i18n.tr("ui-color-swap")).clicked() {
          let tools = &mut self.canvas.tools;
          std::mem::swap(&mut tools.primary, &mut tools.secondary);
          picked = Some(tools.primary);
          hsv = Hsva::from_srgb(tools.primary);
        }
      });
      ui.add_space(6.0);
      ui.vertical(|ui| {
        ui.add_space(4.0);
        // Hex, edited as text; shows the current color whenever it is not being typed in.
        let id = egui::Id::unique("color-hex");
        if !ui.memory(|m| m.has_focus(id)) {
          let [r, g, b] = picked.unwrap_or(primary);
          self.color_hex = format!("{r:02X}{g:02X}{b:02X}");
        }
        ui.horizontal(|ui| {
          ui.label(RichText::new("#").color(theme::palette(ui).muted));
          let response =
            ui.add(egui::TextEdit::singleline(&mut self.color_hex).id(id).desired_width(90.0).char_limit(7).font(egui::TextStyle::Monospace));
          if response.changed()
            && let Some(color) = parse_hex(&self.color_hex)
          {
            picked = Some(color);
            hsv = Hsva::from_srgb(color);
          }
        });
        let [r, g, b] = picked.unwrap_or(primary);
        ui.label(RichText::new(format!("R {r}  G {g}  B {b}")).small().color(theme::palette(ui).muted));
      });
    });
    ui.add_space(10.0);
    let tabs =
      [(Tab::Disc, "ui-color-disc"), (Tab::Classic, "ui-color-classic"), (Tab::Harmony, "ui-color-harmony"), (Tab::Palettes, "ui-color-palettes")];
    segmented(ui, &mut self.color_tab, &tabs.map(|(tab, id)| (tab, self.i18n.tr(id))));
    ui.add_space(10.0);
    match self.color_tab {
      Tab::Disc => {
        if ui.vertical_centered(|ui| disc(ui, &mut hsv, &[])).inner {
          picked = Some(rgb(hsv));
        }
      }
      Tab::Classic => {
        let [r, g, b] = primary;
        let mut color = Color32::from_rgb(r, g, b);
        if egui::color_picker::color_picker_color32(ui, &mut color, egui::color_picker::Alpha::Opaque) {
          picked = Some([color.r(), color.g(), color.b()]);
          hsv = Hsva::from_srgb(picked.expect("just set"));
        }
      }
      Tab::Harmony => {
        egui::ComboBox::from_id_salt("harmony").selected_text(self.i18n.tr(self.harmony.name())).width(ui.available_width()).show_ui(ui, |ui| {
          for harmony in Harmony::ALL {
            ui.selectable_value(&mut self.harmony, harmony, self.i18n.tr(harmony.name()));
          }
        });
        let marks: Vec<f32> = self.harmony.colors(hsv).iter().skip(1).map(|c| c.h).collect();
        ui.add_space(6.0);
        if ui.vertical_centered(|ui| disc(ui, &mut hsv, &marks)).inner {
          picked = Some(rgb(hsv));
        }
        ui.add_space(6.0);
        ui.horizontal(|ui| {
          ui.spacing_mut().item_spacing.x = 8.0;
          for color in self.harmony.colors(hsv) {
            if swatch(ui, rgb(color), 40.0, rgb(color) == primary).clicked() {
              picked = Some(rgb(color));
              hsv = color;
            }
          }
        });
      }
      Tab::Palettes => {
        if let Some(color) = self.palettes_tab(ui, primary) {
          picked = Some(color);
          hsv = Hsva::from_srgb(color);
        }
      }
    }
    if let Some(color) = picked {
      self.canvas.tools.primary = color;
    }
    self.color_hsv = Some(hsv);

    let recent = self.canvas.tools.recent.clone();
    if !recent.is_empty() {
      ui.add_space(10.0);
      ui.separator();
      ui.label(RichText::new(self.i18n.tr("ui-color-history")).small().color(theme::palette(ui).muted));
      ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = Vec2::splat(6.0);
        for color in recent {
          if swatch(ui, color, 26.0, color == self.canvas.tools.primary).clicked() {
            self.canvas.tools.primary = color;
          }
        }
      });
    }
  }

  /// The palettes tab: pick one, rename it, add the current color, remove with a right-click.
  fn palettes_tab(&mut self, ui: &mut egui::Ui, current: [u8; 3]) -> Option<[u8; 3]> {
    let list = &mut self.palettes.list;
    self.palette = self.palette.min(list.len() - 1);
    let mut picked = None;
    // The buttons first (right to left), then the list takes exactly what is left: a row wider
    // than the dock made the dock grow, and then the row again, on every frame.
    let row = Vec2::new(ui.available_width(), ui.spacing().interact_size.y);
    ui.allocate_ui_with_layout(row, egui::Layout::right_to_left(egui::Align::Center), |ui| {
      if ui.add_enabled(list.len() > 1, egui::Button::new("−")).on_hover_text(self.i18n.tr("ui-palette-delete")).clicked() {
        list.remove(self.palette);
        self.palette = self.palette.saturating_sub(1);
      }
      if ui.button("+").on_hover_text(self.i18n.tr("ui-palette-new")).clicked() {
        list.push(Palette { name: self.i18n.tr("ui-palette-new-name"), colors: Vec::new() });
        self.palette = list.len() - 1;
      }
      // A long name is cut short instead of widening the list.
      ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Truncate);
      egui::ComboBox::from_id_salt("palette").selected_text(list[self.palette].name.clone()).width(ui.available_width()).show_ui(ui, |ui| {
        for (i, palette) in list.iter().enumerate() {
          ui.selectable_value(&mut self.palette, i, &palette.name);
        }
      });
    });
    let palette = &mut list[self.palette];
    ui.add(egui::TextEdit::singleline(&mut palette.name).desired_width(f32::INFINITY));
    ui.add_space(8.0);
    let mut remove = None;
    ui.horizontal_wrapped(|ui| {
      ui.spacing_mut().item_spacing = Vec2::splat(6.0);
      for (i, &color) in palette.colors.iter().enumerate() {
        let response = swatch(ui, color, 30.0, color == current);
        if response.clicked() {
          picked = Some(color);
        }
        if response.secondary_clicked() {
          remove = Some(i);
        }
      }
      let (rect, response) = ui.allocate_exact_size(Vec2::splat(30.0), Sense::click());
      ui.painter().rect_stroke(rect, 4.0, Stroke::new(1.0, ui.visuals().weak_text_color()), egui::StrokeKind::Inside);
      ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, "+", egui::FontId::proportional(16.0), ui.visuals().text_color());
      if response.on_hover_text(self.i18n.tr("ui-palette-add-color")).clicked() && !palette.colors.contains(&current) {
        palette.colors.push(current);
      }
    });
    if let Some(i) = remove {
      palette.colors.remove(i);
    }
    ui.add_space(4.0);
    ui.label(RichText::new(self.i18n.tr("ui-palette-remove-help")).small().color(theme::palette(ui).muted));
    ui.add_space(4.0);
    ui.horizontal_wrapped(|ui| {
      if ui.button(self.i18n.tr("ui-palette-import")).on_hover_text(self.i18n.tr("ui-palette-import-help")).clicked() {
        self.import_palettes();
      }
      if ui.button(self.i18n.tr("ui-palette-export")).clicked() {
        self.export_palette();
      }
    });
    picked
  }

  /// Adds the palettes of a `.swatches`, `.aco` or `.ase` file and shows the first one.
  fn import_palettes(&mut self) {
    let Some(path) = rfd::FileDialog::new().add_filter(self.i18n.tr("ui-palette-filter"), &["swatches", "aco", "ase"]).pick_file() else { return };
    let name = path.file_stem().map_or_else(|| self.i18n.tr("ui-palette-new-name"), |s| s.to_string_lossy().into_owned());
    let result = std::fs::read(&path).map_err(rustcreate_core::FileError::from).and_then(|bytes| {
      let format = path
        .extension()
        .and_then(|e| SwatchFormat::from_extension(&e.to_string_lossy()))
        .ok_or_else(|| rustcreate_core::FileError::Invalid("unknown palette format".into()))?;
      import_swatches(&bytes, format, &name)
    });
    match result {
      Ok(imported) => {
        self.palette = self.palettes.list.len();
        self.palettes.list.extend(imported.into_iter().map(|s| Palette { name: s.name, colors: s.colors }));
      }
      Err(error) => self.error = Some(self.i18n.tr_args("ui-palette-import-failed", &[("error", error.to_string().into())])),
    }
  }

  /// Saves the shown palette as `.swatches`, `.aco` or `.ase`, by the extension picked.
  fn export_palette(&mut self) {
    let palette = &self.palettes.list[self.palette];
    let Some(path) = rfd::FileDialog::new()
      .add_filter("Procreate", &["swatches"])
      .add_filter("Photoshop", &["aco"])
      .add_filter("Adobe Swatch Exchange", &["ase"])
      .set_file_name(format!("{}.swatches", palette.name))
      .save_file()
    else {
      return;
    };
    let format = path.extension().and_then(|e| SwatchFormat::from_extension(&e.to_string_lossy())).unwrap_or(SwatchFormat::Procreate);
    let path = if path.extension().is_some_and(|e| SwatchFormat::from_extension(&e.to_string_lossy()).is_some()) {
      path
    } else {
      path.with_extension("swatches")
    };
    let swatches = Swatches { name: palette.name.clone(), colors: palette.colors.clone() };
    if let Err(error) = export_swatches(&swatches, format).and_then(|bytes| Ok(std::fs::write(&path, bytes)?)) {
      self.error = Some(self.i18n.tr_args("ui-palette-export-failed", &[("error", error.to_string().into())]));
    }
  }
}

/// `RRGGBB`, with or without `#`; three-digit `RGB` too.
fn parse_hex(text: &str) -> Option<[u8; 3]> {
  let hex = text.trim().trim_start_matches('#');
  let digits: Vec<u8> = hex.chars().map(|c| c.to_digit(16).map(|d| d as u8)).collect::<Option<_>>()?;
  match digits[..] {
    [r, g, b] => Some([r * 17, g * 17, b * 17]),
    [r1, r0, g1, g0, b1, b0] => Some([r1 * 16 + r0, g1 * 16 + g0, b1 * 16 + b0]),
    _ => None,
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn harmonies_rotate_the_hue_only() {
    let base = Hsva::new(0.9, 0.5, 0.4, 1.0);
    let triad = Harmony::Triadic.colors(base);
    assert_eq!(triad.len(), 3);
    assert!((triad[1].h - (0.9 + 1.0 / 3.0 - 1.0)).abs() < 1e-5, "wraps around");
    assert!(triad.iter().all(|c| c.s == 0.5 && c.v == 0.4));
    assert!((Harmony::Complementary.colors(base)[1].h - 0.4).abs() < 1e-5);
  }

  #[test]
  fn hex_parsing() {
    assert_eq!(parse_hex("#ff8000"), Some([255, 128, 0]));
    assert_eq!(parse_hex("F80"), Some([255, 136, 0]));
    assert_eq!(parse_hex("12345"), None);
    assert_eq!(parse_hex("zzzzzz"), None);
  }
}
