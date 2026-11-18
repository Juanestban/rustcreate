//! Design tokens for the dark (default) and light themes: quiet near-black or near-white chrome
//! so the artwork is the brightest thing on screen, one accent taken from the app icon's
//! cool-to-warm stroke, 8 px corners on floating panels.

use eframe::egui::{self, Color32, CornerRadius, Stroke, Theme, Visuals};

const fn hex(rgb: u32) -> Color32 {
  let [_, r, g, b] = rgb.to_be_bytes();
  Color32::from_rgb(r, g, b)
}

/// The icon's gradient, cool to warm. Used sparingly: the active color ring, selection marks.
pub const GRADIENT: [Color32; 4] = [hex(0x22d3ee), hex(0x4f6bff), hex(0xec4899), hex(0xff7a3d)];
pub const ACCENT: Color32 = hex(0x4f6bff);

pub const BAR_HEIGHT: f32 = 44.0;
pub const SIDEBAR_WIDTH: f32 = 52.0;
pub const PANEL_RADIUS: CornerRadius = CornerRadius::same(10);

/// Colors that change with the theme.
#[derive(Debug, Clone, Copy)]
pub struct Palette {
  /// Top bar and sidebar.
  pub chrome: Color32,
  /// Around the canvas.
  pub backdrop: Color32,
  /// Floating panels and dialogs.
  pub panel: Color32,
  /// Rows and buttons inside panels.
  pub control: Color32,
  pub ink: Color32,
  pub muted: Color32,
  pub line: Color32,
  /// Highlighted row (the active layer, the selected brush).
  pub selected: Color32,
}

pub const DARK: Palette = Palette {
  chrome: hex(0x111114),
  backdrop: hex(0x1a1a1e),
  panel: hex(0x1f1f24),
  control: hex(0x2a2a31),
  ink: hex(0xf2f2f5),
  muted: hex(0x9a9aa6),
  line: hex(0x34343c),
  selected: hex(0x2b3566),
};

pub const LIGHT: Palette = Palette {
  chrome: hex(0xf6f6f8),
  backdrop: hex(0xdcdce2),
  panel: hex(0xffffff),
  control: hex(0xeeeef2),
  ink: hex(0x1b1b20),
  muted: hex(0x6b6b78),
  line: hex(0xd6d6de),
  selected: hex(0xdce3ff),
};

pub fn palette(ui: &egui::Ui) -> Palette {
  if ui.visuals().dark_mode { DARK } else { LIGHT }
}

/// Installs both themes; egui switches between them with the system (or the user's choice).
pub fn install(ctx: &egui::Context) {
  for (theme, palette) in [(Theme::Dark, DARK), (Theme::Light, LIGHT)] {
    let mut visuals = if theme == Theme::Dark { Visuals::dark() } else { Visuals::light() };
    visuals.panel_fill = palette.chrome;
    visuals.window_fill = palette.panel;
    visuals.extreme_bg_color = palette.backdrop;
    visuals.faint_bg_color = palette.control;
    visuals.window_corner_radius = PANEL_RADIUS;
    visuals.menu_corner_radius = PANEL_RADIUS;
    visuals.window_stroke = Stroke::new(1.0, palette.line);
    visuals.selection.bg_fill = ACCENT;
    visuals.selection.stroke = Stroke::new(1.0, palette.ink);
    visuals.hyperlink_color = ACCENT;
    visuals.override_text_color = Some(palette.ink);
    for widget in [&mut visuals.widgets.inactive, &mut visuals.widgets.hovered, &mut visuals.widgets.active] {
      widget.corner_radius = CornerRadius::same(6);
    }
    visuals.widgets.inactive.weak_bg_fill = palette.control;
    visuals.widgets.inactive.bg_fill = palette.control;
    visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0, palette.line);
    ctx.set_visuals_of(theme, visuals);
  }
}
