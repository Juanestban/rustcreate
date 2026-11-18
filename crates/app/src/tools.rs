//! The painting tools (brushes, eraser, colors) and their Photoshop shortcuts
//! (docs/SHORTCUTS.md).

use eframe::egui::{InputState, Key, Modifiers};
use rustcreate_core::{Brush, Document, GradientKind};

use crate::library::Library;

const MIN_SIZE: f32 = 1.0;
const MAX_SIZE: f32 = 1000.0;
const RECENT_COLORS: usize = 10;

/// The painting tools of the top bar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tool {
  Brush,
  Smudge,
  Eraser,
  /// Paint bucket.
  Fill,
  Gradient,
  Select,
}

/// How the selection tool selects (M, L, W in Photoshop).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelectKind {
  Rectangle,
  Ellipse,
  Lasso,
  /// Magic wand: the area of similar color, with the bucket's threshold.
  Wand,
}

impl Tool {
  /// Which library brush slot the tool uses; the bucket and the gradient show the paint brush.
  fn slot(self) -> usize {
    match self {
      Tool::Smudge => 1,
      Tool::Eraser => 2,
      _ => 0,
    }
  }
}

pub struct Tools {
  /// Every brush keeps its own size and opacity, and each tool its own brush, like Procreate.
  pub library: Library,
  pub tool: Tool,
  /// The eyedropper (I): the next press picks a color, then the brush comes back.
  pub picking: bool,
  /// The hand (H): primary drags pan instead of painting.
  pub hand: bool,
  /// Straight sRGB.
  pub primary: [u8; 3],
  pub secondary: [u8; 3],
  /// Paint bucket: 0..=1 color difference it spreads over, and whether it sees every visible
  /// layer (to color line art on another layer) or only the active one.
  pub fill_threshold: f32,
  pub fill_all_layers: bool,
  pub gradient: GradientKind,
  /// The gradient fades the primary color out instead of blending into the secondary.
  pub gradient_to_clear: bool,
  /// QuickShape: a held stroke snaps to a clean shape (a setting).
  pub quickshape: bool,
  pub guides: crate::guides::Guides,
  /// Colors painted with, newest first.
  pub recent: Vec<[u8; 3]>,
  pub select_kind: SelectKind,
  /// Feather radius in canvas pixels for the Feather button.
  pub feather: f32,
}

impl Default for Tools {
  fn default() -> Self {
    Self {
      library: Library::default(),
      tool: Tool::Brush,
      picking: false,
      hand: false,
      primary: [0, 0, 0],
      secondary: [255, 255, 255],
      fill_threshold: 0.1,
      fill_all_layers: true,
      gradient: GradientKind::Linear,
      gradient_to_clear: false,
      quickshape: true,
      guides: Default::default(),
      recent: Vec::new(),
      select_kind: SelectKind::Rectangle,
      feather: 10.0,
    }
  }
}

impl Tools {
  /// The brush, color and mode a new stroke uses.
  pub fn current(&self) -> (Brush, [u8; 3], Tool) {
    (self.brush_for(self.tool), self.primary, self.tool)
  }

  /// The brush `tool` paints with.
  pub fn brush_for(&self, tool: Tool) -> Brush {
    self.library.entries[self.library.selection[tool.slot()]].brush
  }

  /// The library index of the current tool's brush.
  pub fn selected(&self) -> usize {
    self.library.selection[self.tool.slot()]
  }

  pub fn select(&mut self, index: usize) {
    self.library.selection[self.tool.slot()] = index;
  }

  pub fn brush_mut(&mut self) -> &mut Brush {
    let index = self.selected();
    &mut self.library.entries[index].brush
  }

  /// Remembers the color a stroke starts with, for the color panel's history.
  pub fn used_color(&mut self) {
    let color = self.primary;
    self.recent.retain(|c| *c != color);
    self.recent.insert(0, color);
    self.recent.truncate(RECENT_COLORS);
  }

  /// Tool, brush, layer and history shortcuts. The document is left alone while a stroke is in
  /// progress. egui ignores extra Shift/Alt, so the longer combinations are checked first.
  pub fn shortcuts(&mut self, input: &mut InputState, doc: &mut Document, painting: bool) {
    if input.consume_key(Modifiers::SHIFT, Key::B) {
      (self.tool, self.hand) = (Tool::Brush, false);
      self.select(self.library.next_in_category(self.selected()));
    }
    if input.consume_key(Modifiers::NONE, Key::B) {
      (self.tool, self.hand) = (Tool::Brush, false);
    }
    if input.consume_key(Modifiers::NONE, Key::S) {
      (self.tool, self.hand) = (Tool::Smudge, false);
    }
    if input.consume_key(Modifiers::NONE, Key::E) {
      (self.tool, self.hand) = (Tool::Eraser, false);
    }
    // G: paint bucket, again for the gradient (Photoshop keeps both under G).
    if input.consume_key(Modifiers::NONE, Key::G) {
      self.tool = if self.tool == Tool::Fill { Tool::Gradient } else { Tool::Fill };
      self.hand = false;
    }
    if input.consume_key(Modifiers::NONE, Key::I) {
      self.picking = !self.picking;
    }
    // M: rectangle, again for the ellipse; L: lasso; W: magic wand.
    for (key, kind) in [(Key::M, SelectKind::Rectangle), (Key::L, SelectKind::Lasso), (Key::W, SelectKind::Wand)] {
      if input.consume_key(Modifiers::NONE, key) {
        let again = self.tool == Tool::Select && self.select_kind == SelectKind::Rectangle && kind == SelectKind::Rectangle;
        (self.tool, self.hand, self.select_kind) = (Tool::Select, false, if again { SelectKind::Ellipse } else { kind });
      }
    }
    if input.consume_key(Modifiers::NONE, Key::H) {
      self.hand = !self.hand;
    }
    if !painting {
      // Before the plain Backspace (clear layer), which Alt+Backspace would match too.
      if input.consume_key(Modifiers::ALT, Key::Backspace) {
        doc.fill_layer(self.primary);
      }
      layer_shortcuts(input, doc);
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
    if input.consume_key(Modifiers::COMMAND | Modifiers::SHIFT, Key::I) {
      doc.invert_selection();
    }
    if input.consume_key(Modifiers::COMMAND, Key::A) {
      doc.select_all();
    }
    if input.consume_key(Modifiers::COMMAND, Key::D) {
      doc.deselect();
    }
    // Check the longer combinations first: consume_key ignores extra Shift/Alt on Ctrl+Z.
    if input.consume_key(Modifiers::COMMAND | Modifiers::SHIFT, Key::Z) || input.consume_key(Modifiers::COMMAND, Key::Y) {
      doc.redo();
    }
    if input.consume_key(Modifiers::COMMAND | Modifiers::ALT, Key::Z) || input.consume_key(Modifiers::COMMAND, Key::Z) {
      doc.undo();
    }
  }
}

fn layer_shortcuts(input: &mut InputState, doc: &mut Document) {
  let active = doc.active();
  // Ctrl+Alt+G: clipping mask (Photoshop); /: Alpha Lock ("lock transparent pixels").
  let clip = active > 0 && input.consume_key(Modifiers::COMMAND | Modifiers::ALT, Key::G);
  let lock = input.consume_key(Modifiers::NONE, Key::Slash);
  if clip || lock {
    doc.checkpoint();
    let layer = doc.layer_mut(active).expect("active layer");
    if clip {
      layer.clip = !layer.clip;
    }
    if lock {
      layer.alpha_lock = !layer.alpha_lock;
    }
  }
  let top = doc.layers().len() - 1;
  if input.consume_key(Modifiers::COMMAND | Modifiers::SHIFT, Key::N) {
    doc.add_layer();
  }
  if input.consume_key(Modifiers::COMMAND, Key::J) {
    doc.duplicate_layer(active);
  }
  if input.consume_key(Modifiers::COMMAND, Key::E) {
    doc.merge_down(active);
  }
  if input.consume_key(Modifiers::COMMAND, Key::OpenBracket) && active > 0 {
    doc.move_layer(active, active - 1);
  }
  if input.consume_key(Modifiers::COMMAND, Key::CloseBracket) && active < top {
    doc.move_layer(active, active + 1);
  }
  // Before the plain [ ]: Alt+[ would match them too.
  if input.consume_key(Modifiers::ALT, Key::OpenBracket) {
    doc.select(active.saturating_sub(1));
  }
  if input.consume_key(Modifiers::ALT, Key::CloseBracket) {
    doc.select((active + 1).min(top));
  }
  if input.consume_key(Modifiers::NONE, Key::Backspace) || input.consume_key(Modifiers::NONE, Key::Delete) {
    doc.clear_layer(doc.active());
  }
}

#[cfg(test)]
mod tests {
  use super::*;
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
    assert_eq!(tools.current().2, Tool::Eraser, "E = eraser");
    press(&mut tools, &mut doc, Modifiers::NONE, Key::S);
    assert_eq!(tools.current().2, Tool::Smudge, "S = smudge");
    press(&mut tools, &mut doc, Modifiers::NONE, Key::B);
    assert_eq!(tools.current().2, Tool::Brush, "B = brush");
    press(&mut tools, &mut doc, Modifiers::NONE, Key::Num5);
    assert_eq!(tools.current().0.opacity, 0.5);
    press(&mut tools, &mut doc, Modifiers::NONE, Key::CloseBracket);
    assert!(tools.current().0.size > Brush::ink().size);
    press(&mut tools, &mut doc, Modifiers::NONE, Key::X);
    assert_eq!(tools.current().1, [255, 255, 255]);
    press(&mut tools, &mut doc, Modifiers::NONE, Key::D);
    assert_eq!(tools.current().1, [0, 0, 0]);
    press(&mut tools, &mut doc, Modifiers::SHIFT, Key::B);
    assert_eq!(tools.current().0.size, 5.0, "Shift+B: the next inking brush, the Fineliner");
  }

  #[test]
  fn photoshop_layer_shortcuts() {
    let (mut tools, mut doc) = (Tools::default(), Document::new(10, 10));
    press(&mut tools, &mut doc, Modifiers::COMMAND | Modifiers::SHIFT, Key::N);
    assert_eq!((doc.layers().len(), doc.active()), (2, 1));
    let top = doc.layers()[1].id;
    press(&mut tools, &mut doc, Modifiers::ALT, Key::OpenBracket);
    assert_eq!(doc.active(), 0, "Alt+[ selects below and does not shrink the brush");
    assert_eq!(tools.current().0.size, Brush::ink().size);
    press(&mut tools, &mut doc, Modifiers::COMMAND, Key::CloseBracket);
    assert_eq!(doc.layers()[0].id, top, "Ctrl+] moves the layer up");
    press(&mut tools, &mut doc, Modifiers::COMMAND, Key::J);
    assert_eq!(doc.layers().len(), 3);
    press(&mut tools, &mut doc, Modifiers::COMMAND, Key::E);
    assert_eq!(doc.layers().len(), 2);
  }

  #[test]
  fn bucket_and_gradient_shortcuts() {
    let (mut tools, mut doc) = (Tools::default(), Document::new(10, 10));
    press(&mut tools, &mut doc, Modifiers::NONE, Key::G);
    assert_eq!(tools.tool, Tool::Fill);
    press(&mut tools, &mut doc, Modifiers::NONE, Key::G);
    assert_eq!(tools.tool, Tool::Gradient);
    assert_eq!(tools.current().0, tools.brush_for(Tool::Brush), "they show the paint brush");
    tools.primary = [9, 8, 7];
    press(&mut tools, &mut doc, Modifiers::ALT, Key::Backspace);
    assert_eq!(doc.layers()[0].painted_tiles(), 1, "Alt+Backspace fills the layer, not clears it");
  }

  #[test]
  fn recent_colors_are_unique_newest_first() {
    let mut tools = Tools::default();
    for color in [[1, 0, 0], [2, 0, 0], [1, 0, 0]] {
      tools.primary = color;
      tools.used_color();
    }
    assert_eq!(tools.recent, [[1, 0, 0], [2, 0, 0]]);
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
