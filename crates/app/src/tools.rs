//! The painting tools (brushes, eraser, colors) and their Photoshop shortcuts
//! (docs/SHORTCUTS.md).

use eframe::egui::{InputState, Key, Modifiers};
use rustcreate_core::{Brush, Document};

use crate::i18n::I18n;

/// v0.1 brushes, in Shift+B order, with their name tokens.
const BRUSHES: [(&str, Brush); 3] = [("ui-brush-pencil", Brush::pencil()), ("ui-brush-ink", Brush::ink()), ("ui-brush-airbrush", Brush::airbrush())];
const MIN_SIZE: f32 = 1.0;
const MAX_SIZE: f32 = 1000.0;

pub struct Tools {
  /// Each brush keeps its own size and opacity, like Procreate.
  brushes: [Brush; 3],
  selected: usize,
  erasing: bool,
  /// Straight sRGB.
  primary: [u8; 3],
  secondary: [u8; 3],
}

impl Default for Tools {
  fn default() -> Self {
    Self { brushes: BRUSHES.map(|(_, brush)| brush), selected: 1, erasing: false, primary: [0, 0, 0], secondary: [255, 255, 255] }
  }
}

impl Tools {
  /// The brush, color and mode a new stroke uses.
  pub fn current(&self) -> (Brush, [u8; 3], bool) {
    (self.brushes[self.selected], self.primary, self.erasing)
  }

  fn brush_mut(&mut self) -> &mut Brush {
    &mut self.brushes[self.selected]
  }

  /// Tool, brush and history shortcuts. History is left alone while a stroke is in progress.
  pub fn shortcuts(&mut self, input: &mut InputState, doc: &mut Document, painting: bool) {
    if input.consume_key(Modifiers::SHIFT, Key::B) {
      self.selected = (self.selected + 1) % self.brushes.len();
      self.erasing = false;
    }
    if input.consume_key(Modifiers::NONE, Key::B) {
      self.erasing = false;
    }
    if input.consume_key(Modifiers::NONE, Key::E) {
      self.erasing = true;
    }
    if input.consume_key(Modifiers::NONE, Key::OpenBracket) {
      self.brush_mut().size = (self.brush_mut().size / 1.1).max(MIN_SIZE);
    }
    if input.consume_key(Modifiers::NONE, Key::CloseBracket) {
      self.brush_mut().size = (self.brush_mut().size * 1.1).min(MAX_SIZE);
    }
    // 1 … 9 = 10 % … 90 %, 0 = 100 %.
    let digits = [Key::Num0, Key::Num1, Key::Num2, Key::Num3, Key::Num4, Key::Num5, Key::Num6, Key::Num7, Key::Num8, Key::Num9];
    for (n, key) in digits.into_iter().enumerate() {
      if input.consume_key(Modifiers::NONE, key) {
        self.brush_mut().opacity = if n == 0 { 1.0 } else { n as f32 / 10.0 };
      }
    }
    if input.consume_key(Modifiers::NONE, Key::X) {
      std::mem::swap(&mut self.primary, &mut self.secondary);
    }
    if input.consume_key(Modifiers::NONE, Key::D) {
      (self.primary, self.secondary) = ([0, 0, 0], [255, 255, 255]);
    }
    if painting {
      return;
    }
    // Check the longer combinations first: consume_key ignores extra Shift/Alt on Ctrl+Z.
    if input.consume_key(Modifiers::COMMAND | Modifiers::SHIFT, Key::Z) || input.consume_key(Modifiers::COMMAND, Key::Y) {
      doc.redo();
    }
    if input.consume_key(Modifiers::COMMAND | Modifiers::ALT, Key::Z) || input.consume_key(Modifiers::COMMAND, Key::Z) {
      doc.undo();
    }
  }

  /// "Ink · 12 px · 100 %", or the eraser with the brush it uses.
  pub fn status(&self, i18n: &I18n) -> String {
    let brush = self.brushes[self.selected];
    let name = i18n.tr(BRUSHES[self.selected].0);
    let tool = if self.erasing { i18n.tr_args("ui-tool-eraser", &[("brush", name.into())]) } else { name };
    i18n.tr_args(
      "ui-brush-status",
      &[("tool", tool.into()), ("size", (brush.size.round() as i64).into()), ("opacity", ((brush.opacity * 100.0).round() as i64).into())],
    )
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::i18n::Language;
  use eframe::egui::{Event, RawInput};

  fn press(tools: &mut Tools, doc: &mut Document, modifiers: Modifiers, key: Key) {
    let ctx = eframe::egui::Context::default();
    let event = Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers };
    let raw = RawInput { events: vec![event], ..Default::default() };
    let mut output = ctx.run_ui(raw, |ui| ui.input_mut(|input| tools.shortcuts(input, doc, false)));
    // No renderer here: drop the font texture egui wants uploaded.
    output.textures_delta.clear();
  }

  #[test]
  fn photoshop_tool_shortcuts() {
    let (mut tools, mut doc) = (Tools::default(), Document::new(10, 10));
    press(&mut tools, &mut doc, Modifiers::NONE, Key::E);
    assert!(tools.current().2, "E = eraser");
    press(&mut tools, &mut doc, Modifiers::NONE, Key::B);
    assert!(!tools.current().2, "B = brush");
    press(&mut tools, &mut doc, Modifiers::NONE, Key::Num5);
    assert_eq!(tools.current().0.opacity, 0.5);
    press(&mut tools, &mut doc, Modifiers::NONE, Key::CloseBracket);
    assert!(tools.current().0.size > Brush::ink().size);
    press(&mut tools, &mut doc, Modifiers::NONE, Key::X);
    assert_eq!(tools.current().1, [255, 255, 255]);
    press(&mut tools, &mut doc, Modifiers::NONE, Key::D);
    assert_eq!(tools.current().1, [0, 0, 0]);
    press(&mut tools, &mut doc, Modifiers::SHIFT, Key::B);
    assert_eq!(tools.current().0.size, Brush::airbrush().size, "Shift+B cycles to the airbrush");
    assert_eq!(tools.status(&I18n::new(Language::En)), "Airbrush · 120 px · 100 %");
  }

  #[test]
  fn undo_and_redo_shortcuts() {
    let (mut tools, mut doc) = (Tools::default(), Document::new(10, 10));
    doc.add_layer();
    press(&mut tools, &mut doc, Modifiers::COMMAND, Key::Z);
    assert_eq!(doc.layers().len(), 1);
    press(&mut tools, &mut doc, Modifiers::COMMAND | Modifiers::SHIFT, Key::Z);
    assert_eq!(doc.layers().len(), 2);
    press(&mut tools, &mut doc, Modifiers::COMMAND | Modifiers::ALT, Key::Z);
    assert_eq!(doc.layers().len(), 1);
    press(&mut tools, &mut doc, Modifiers::COMMAND, Key::Y);
    assert_eq!(doc.layers().len(), 2);
  }
}
