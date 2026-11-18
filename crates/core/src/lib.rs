//! The document model: a canvas of tiled RGBA8 raster layers with undo (ADR 0005). No GPU here.

pub mod blend;
mod brush;
mod file;
mod fill;
mod history;
mod shape;
pub mod tip;

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

pub use brush::{Brush, Stroke};
pub use file::{FORMAT, FileError};
pub use fill::{Gradient, GradientKind};
pub use history::History;
use history::Snapshot;
pub use shape::Shape;
pub use tip::{Grain, Tip};

/// Tile side in pixels.
pub const TILE: u32 = 256;
pub const TILE_BYTES: usize = (TILE * TILE * 4) as usize;
/// Largest canvas side (ADR 0005); the GPU may allow less.
pub const MAX_SIDE: u32 = 16384;

/// Premultiplied RGBA8 pixels of one tile, row-major, `TILE_BYTES` long. Shared between the
/// document and its history and copied on write.
pub type Tile = Arc<[u8]>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum BlendMode {
  #[default]
  Normal,
  Multiply,
  Screen,
  Overlay,
  Darken,
  Lighten,
  Add,
  ColorDodge,
  ColorBurn,
}

#[derive(Debug, Clone)]
pub struct Layer {
  /// Stable for the document's life; names the layer's PNG in `.rcreate` files.
  pub id: u64,
  pub name: String,
  pub opacity: f32,
  pub hidden: bool,
  pub locked: bool,
  pub blend: BlendMode,
  /// Only tiles that were painted exist; a missing tile is fully transparent.
  tiles: HashMap<(u32, u32), Tile>,
  /// Tiles written through `tile_mut` since the renderer last looked (see `take_dirty`).
  dirty: HashSet<(u32, u32)>,
}

impl Layer {
  fn new(id: u64, name: String) -> Self {
    Self { id, name, opacity: 1.0, hidden: false, locked: false, blend: BlendMode::Normal, tiles: HashMap::new(), dirty: HashSet::new() }
  }

  pub fn tile(&self, tx: u32, ty: u32) -> Option<&[u8]> {
    self.tiles.get(&(tx, ty)).map(|tile| &tile[..])
  }

  /// The tile at `(tx, ty)` for painting, created transparent if empty. Copies it first if the
  /// history still holds it, so call [`Document::checkpoint`] before a stroke, not per tile.
  pub fn tile_mut(&mut self, tx: u32, ty: u32) -> &mut [u8] {
    self.dirty.insert((tx, ty));
    Arc::make_mut(self.tiles.entry((tx, ty)).or_insert_with(|| Arc::from(vec![0; TILE_BYTES])))
  }

  /// Coordinates of the painted tiles.
  pub fn tile_coords(&self) -> impl Iterator<Item = (u32, u32)> + '_ {
    self.tiles.keys().copied()
  }

  /// The tile's shared buffer, to tell whether it was replaced (undo, open) without comparing
  /// pixels.
  pub fn tile_ptr(&self, tx: u32, ty: u32) -> Option<*const u8> {
    self.tiles.get(&(tx, ty)).map(|tile| tile.as_ptr())
  }

  /// Tiles painted in place since the last call. Replaced tiles are found with `tile_ptr`.
  pub fn take_dirty(&mut self) -> HashSet<(u32, u32)> {
    std::mem::take(&mut self.dirty)
  }

  pub fn painted_tiles(&self) -> usize {
    self.tiles.len()
  }
}

#[derive(Debug, Clone)]
pub struct Document {
  pub width: u32,
  pub height: u32,
  /// Bottom to top. Never empty.
  layers: Vec<Layer>,
  /// Index into `layers` of the layer being painted.
  active: usize,
  next_id: u64,
  history: History,
  /// Bumped by every undoable change, undo and redo: compare with the value at save time to
  /// know whether there are unsaved changes.
  revision: u64,
}

impl Document {
  /// A canvas with one empty layer. Sides are clamped to `1..=MAX_SIDE`.
  pub fn new(width: u32, height: u32) -> Self {
    Self::with_history(width, height, History::default())
  }

  pub fn with_history(width: u32, height: u32, history: History) -> Self {
    let mut doc =
      Self { width: width.clamp(1, MAX_SIDE), height: height.clamp(1, MAX_SIDE), layers: Vec::new(), active: 0, next_id: 1, history, revision: 0 };
    let first = doc.new_layer();
    doc.layers.push(first);
    doc
  }

  /// Tiles across and down.
  pub fn tiles(&self) -> (u32, u32) {
    (self.width.div_ceil(TILE), self.height.div_ceil(TILE))
  }

  pub fn layers(&self) -> &[Layer] {
    &self.layers
  }

  pub fn active(&self) -> usize {
    self.active
  }

  /// Selecting a layer is not an undo step (as in Photoshop).
  pub fn select(&mut self, index: usize) {
    self.active = index.min(self.layers.len() - 1);
  }

  pub fn history(&self) -> &History {
    &self.history
  }

  /// Records the current state as an undo step. Call it once before painting a stroke or
  /// changing a layer property through [`Document::layer_mut`]; the layer operations below
  /// call it themselves.
  pub fn checkpoint(&mut self) {
    self.history.push(Snapshot::new(self.layers.clone(), self.active));
    self.revision += 1;
  }

  pub fn revision(&self) -> u64 {
    self.revision
  }

  /// A layer to paint on or edit. Call [`Document::checkpoint`] first to make it undoable.
  pub fn layer_mut(&mut self, index: usize) -> Option<&mut Layer> {
    self.layers.get_mut(index)
  }

  pub fn layers_mut(&mut self) -> impl Iterator<Item = &mut Layer> {
    self.layers.iter_mut()
  }

  pub fn undo(&mut self) -> bool {
    self.restore(|history, current| history.undo(current))
  }

  pub fn redo(&mut self) -> bool {
    self.restore(|history, current| history.redo(current))
  }

  fn restore(&mut self, step: impl FnOnce(&mut History, Snapshot) -> Result<Snapshot, Snapshot>) -> bool {
    let current = Snapshot::new(std::mem::take(&mut self.layers), self.active);
    let (state, moved) = match step(&mut self.history, current) {
      Ok(state) => (state, true),
      Err(current) => (current, false),
    };
    self.layers = state.layers;
    self.active = state.active;
    self.revision += u64::from(moved);
    moved
  }

  /// A new empty layer above the active one, which becomes active.
  pub fn add_layer(&mut self) {
    self.checkpoint();
    let layer = self.new_layer();
    self.active += 1;
    self.layers.insert(self.active, layer);
  }

  /// Deletes a layer. The last remaining layer can't be deleted.
  pub fn delete_layer(&mut self, index: usize) -> bool {
    if self.layers.len() == 1 || index >= self.layers.len() {
      return false;
    }
    self.checkpoint();
    self.layers.remove(index);
    if self.active > index || self.active == self.layers.len() {
      self.active -= 1;
    }
    true
  }

  /// A copy above the original, which becomes active. Tiles are shared until painted.
  pub fn duplicate_layer(&mut self, index: usize) -> bool {
    let Some(original) = self.layers.get(index) else { return false };
    let mut copy = original.clone();
    self.checkpoint();
    copy.id = self.take_id();
    copy.name = format!("{} copy", copy.name);
    self.layers.insert(index + 1, copy);
    self.active = index + 1;
    true
  }

  /// Moves a layer to another position in the stack; the active layer follows its content.
  pub fn move_layer(&mut self, from: usize, to: usize) -> bool {
    let len = self.layers.len();
    if from >= len || to >= len || from == to {
      return false;
    }
    let active_id = self.layers[self.active].id;
    self.checkpoint();
    let layer = self.layers.remove(from);
    self.layers.insert(to, layer);
    self.active = self.layers.iter().position(|layer| layer.id == active_id).expect("the active layer is still in the stack");
    true
  }

  /// Merges a layer into the one below, with its opacity and blend mode, like Photoshop's
  /// Merge Down: the result keeps the lower layer's name and properties.
  pub fn merge_down(&mut self, index: usize) -> bool {
    if index == 0 || index >= self.layers.len() {
      return false;
    }
    self.checkpoint();
    let upper = self.layers.remove(index);
    let lower = &mut self.layers[index - 1];
    if !upper.hidden {
      for (&(tx, ty), tile) in &upper.tiles {
        blend::composite(lower.tile_mut(tx, ty), tile, upper.opacity, upper.blend);
      }
    }
    if self.active >= index {
      self.active -= 1;
    }
    true
  }

  /// Erases everything on a layer (Delete / Backspace).
  pub fn clear_layer(&mut self, index: usize) -> bool {
    match self.layers.get(index) {
      Some(layer) if !layer.locked && layer.painted_tiles() > 0 => {}
      _ => return false,
    }
    self.checkpoint();
    self.layers[index].tiles.clear();
    true
  }

  pub fn rename_layer(&mut self, index: usize, name: impl Into<String>) -> bool {
    if index >= self.layers.len() {
      return false;
    }
    self.checkpoint();
    self.layers[index].name = name.into();
    true
  }

  fn new_layer(&mut self) -> Layer {
    let id = self.take_id();
    Layer::new(id, format!("Layer {id}"))
  }

  fn take_id(&mut self) -> u64 {
    self.next_id += 1;
    self.next_id - 1
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  fn names(doc: &Document) -> Vec<&str> {
    doc.layers().iter().map(|layer| layer.name.as_str()).collect()
  }

  fn paint(doc: &mut Document, index: usize, rgba: [u8; 4]) {
    doc.checkpoint();
    let tile = doc.layer_mut(index).unwrap().tile_mut(0, 0);
    tile[..4].copy_from_slice(&rgba);
  }

  fn pixel(doc: &Document, index: usize) -> Option<[u8; 4]> {
    doc.layers()[index].tile(0, 0).map(|tile| tile[..4].try_into().unwrap())
  }

  #[test]
  fn tiles_are_allocated_lazily() {
    let mut doc = Document::new(1000, 1000);
    let layer = doc.layer_mut(0).unwrap();
    assert!(layer.tile(0, 0).is_none());
    layer.tile_mut(3, 1)[0] = 255;
    assert_eq!(layer.painted_tiles(), 1);
    assert_eq!(layer.tile(3, 1).unwrap()[0], 255);
  }

  #[test]
  fn canvas_size_is_clamped_and_tiled() {
    let doc = Document::new(1000, 0);
    assert_eq!((doc.width, doc.height), (1000, 1));
    assert_eq!(doc.tiles(), (4, 1));
    assert_eq!(Document::new(99_999, 256).width, MAX_SIDE);
  }

  #[test]
  fn layer_operations_and_active_layer() {
    let mut doc = Document::new(256, 256);
    doc.add_layer();
    doc.add_layer();
    assert_eq!(names(&doc), ["Layer 1", "Layer 2", "Layer 3"]);
    assert_eq!(doc.active(), 2);

    doc.select(0);
    doc.add_layer();
    assert_eq!(names(&doc), ["Layer 1", "Layer 4", "Layer 2", "Layer 3"], "inserted above the active layer");
    assert_eq!(doc.active(), 1);

    assert!(doc.duplicate_layer(3));
    assert_eq!(names(&doc)[4], "Layer 3 copy");
    assert_eq!(doc.active(), 4);

    assert!(doc.move_layer(4, 0));
    assert_eq!(names(&doc)[0], "Layer 3 copy");
    assert_eq!(doc.active(), 0, "the active layer moves with its content");

    assert!(doc.rename_layer(0, "Sketch"));
    assert!(doc.delete_layer(0));
    assert_eq!(names(&doc), ["Layer 1", "Layer 4", "Layer 2", "Layer 3"]);
    assert_eq!(doc.active(), 0);

    for _ in 0..3 {
      assert!(doc.delete_layer(0));
    }
    assert!(!doc.delete_layer(0), "the last layer stays");
    assert!(!doc.merge_down(0), "nothing below the bottom layer");
  }

  #[test]
  fn undo_and_redo_restore_layers_and_pixels() {
    let mut doc = Document::new(256, 256);
    paint(&mut doc, 0, [255, 0, 0, 255]);
    doc.add_layer();
    paint(&mut doc, 1, [0, 0, 255, 255]);
    assert_eq!(doc.history().steps(), 3);

    assert!(doc.undo());
    assert_eq!(pixel(&doc, 1), None, "the blue stroke is gone");
    assert!(doc.undo());
    assert_eq!(names(&doc), ["Layer 1"]);
    assert_eq!(doc.active(), 0);
    assert!(doc.undo());
    assert_eq!(pixel(&doc, 0), None);
    assert!(!doc.undo(), "nothing left to undo");

    assert!(doc.redo());
    assert!(doc.redo());
    assert!(doc.redo());
    assert_eq!(pixel(&doc, 1), Some([0, 0, 255, 255]));
    assert!(!doc.redo());

    doc.undo();
    paint(&mut doc, 0, [0, 255, 0, 255]);
    assert!(!doc.history().can_redo(), "a new change clears redo");
  }

  #[test]
  fn painting_copies_only_touched_tiles() {
    let mut doc = Document::new(512, 512);
    let layer = doc.layer_mut(0).unwrap();
    layer.tile_mut(0, 0)[0] = 1;
    layer.tile_mut(1, 1)[0] = 1;
    doc.checkpoint();
    doc.layer_mut(0).unwrap().tile_mut(0, 0)[0] = 2;
    doc.checkpoint();
    // The first step holds the old (0, 0) tile alone; (1, 1) is still shared with the document.
    assert_eq!(doc.history().bytes(), TILE_BYTES);
  }

  #[test]
  fn clear_layer_and_revision() {
    let mut doc = Document::new(256, 256);
    let saved = doc.revision();
    assert!(!doc.clear_layer(0), "nothing to clear");
    paint(&mut doc, 0, [1, 2, 3, 255]);
    assert_ne!(doc.revision(), saved, "a change is unsaved");
    assert!(doc.clear_layer(0));
    assert_eq!(pixel(&doc, 0), None);
    assert!(doc.undo());
    assert_eq!(pixel(&doc, 0), Some([1, 2, 3, 255]));
    let before = doc.revision();
    assert!(!doc.redo() || doc.revision() > before);
  }

  #[test]
  fn merge_down_blends_and_is_undoable() {
    let mut doc = Document::new(256, 256);
    paint(&mut doc, 0, [255, 255, 255, 255]);
    doc.add_layer();
    paint(&mut doc, 1, [128, 128, 128, 255]);
    doc.layer_mut(1).unwrap().blend = BlendMode::Multiply;
    assert!(doc.merge_down(1));
    assert_eq!(names(&doc), ["Layer 1"]);
    assert_eq!(pixel(&doc, 0), Some([128, 128, 128, 255]));
    assert_eq!(doc.active(), 0);
    assert!(doc.undo());
    assert_eq!(doc.layers().len(), 2);
    assert_eq!(pixel(&doc, 0), Some([255, 255, 255, 255]));
  }

  #[test]
  fn history_limits_steps_and_memory() {
    let mut doc = Document::with_history(256, 256, History::new(3, usize::MAX));
    for _ in 0..10 {
      doc.add_layer();
    }
    assert_eq!(doc.history().steps(), 3);

    // Room for one tile: older strokes are dropped as new ones settle.
    let mut doc = Document::with_history(256, 256, History::new(250, TILE_BYTES));
    for value in 1..=5 {
      paint(&mut doc, 0, [value, 0, 0, 255]);
    }
    doc.checkpoint();
    assert!(doc.history().bytes() <= TILE_BYTES);
    assert!(doc.history().steps() < 6);
  }
}
