//! The document model: a canvas of tiled RGBA8 raster layers with undo (ADR 0005). No GPU here.

mod adjust;
mod animation;
pub mod blend;
mod brush;
mod file;
mod fill;
mod group;
mod guide;
mod history;
mod import;
mod resize;
mod selection;
mod shape;
mod swatches;
mod text;
pub mod tip;
mod transform;

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

pub use adjust::{Adjust, Adjustment, curve};
pub use animation::{Animation, Playback};
pub use brush::{Brush, Stroke};
pub use file::{FORMAT, FileError};
pub use fill::{Gradient, GradientKind};
pub use group::group_span;
pub use guide::Symmetry;
pub use history::History;
use history::Snapshot;
pub use import::{BrushImage, Imported, export_brushes, import_abr, import_brushset, load_brush_image};
pub use selection::{SelectMode, SelectShape, Selection};
pub use shape::Shape;
pub use swatches::{SwatchFormat, Swatches, export_swatches, import_swatches};
pub use text::{Text, font_families, text_bounds};
pub use tip::{Grain, Tip};
pub use transform::{Affine, IDENTITY, Transform};

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
  // Added later, so files and the GPU keep the numbers above.
  LinearBurn,
  Subtract,
  Divide,
  SoftLight,
  HardLight,
  VividLight,
  LinearLight,
  PinLight,
  Difference,
  Exclusion,
  Hue,
  Saturation,
  Color,
  Luminosity,
}

impl BlendMode {
  /// Every mode, in declaration order (the GPU's mode numbers).
  pub const ALL: [BlendMode; 23] = [
    BlendMode::Normal,
    BlendMode::Multiply,
    BlendMode::Screen,
    BlendMode::Overlay,
    BlendMode::Darken,
    BlendMode::Lighten,
    BlendMode::Add,
    BlendMode::ColorDodge,
    BlendMode::ColorBurn,
    BlendMode::LinearBurn,
    BlendMode::Subtract,
    BlendMode::Divide,
    BlendMode::SoftLight,
    BlendMode::HardLight,
    BlendMode::VividLight,
    BlendMode::LinearLight,
    BlendMode::PinLight,
    BlendMode::Difference,
    BlendMode::Exclusion,
    BlendMode::Hue,
    BlendMode::Saturation,
    BlendMode::Color,
    BlendMode::Luminosity,
  ];
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
  /// Alpha Lock: painting keeps the transparency there is (only painted pixels change color).
  pub alpha_lock: bool,
  /// Clipping mask: shows only where the base layer below (the first one not clipped) has paint.
  pub clip: bool,
  /// Layer mask: grayscale, black hides and white shows. Stored like a layer whose missing
  /// (transparent) pixels count as white, so an empty mask shows everything.
  pub mask: Option<Box<Layer>>,
  /// A group: no pixels of its own, its children sit right below it (see `group.rs`).
  pub(crate) group: bool,
  /// The group this layer is in.
  pub(crate) parent: Option<u64>,
  /// A text layer: its pixels are this text, drawn by `Document::set_text`. Any other edit of
  /// its pixels turns it into a plain layer (Photoshop's Rasterize).
  pub(crate) text: Option<Box<Text>>,
  /// Only tiles that were painted exist; a missing tile is fully transparent.
  tiles: HashMap<(u32, u32), Tile>,
  /// Tiles written through `tile_mut` since the renderer last looked (see `take_dirty`).
  dirty: HashSet<(u32, u32)>,
}

impl Layer {
  fn new(id: u64, name: String) -> Self {
    Self {
      id,
      name,
      opacity: 1.0,
      hidden: false,
      locked: false,
      blend: BlendMode::Normal,
      alpha_lock: false,
      clip: false,
      mask: None,
      group: false,
      parent: None,
      text: None,
      tiles: HashMap::new(),
      dirty: HashSet::new(),
    }
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

  /// Puts a tile back (or removes it) as a whole, for operations that redraw from a saved state.
  pub(crate) fn set_tile(&mut self, key: (u32, u32), tile: Option<Tile>) {
    self.dirty.insert(key);
    match tile {
      Some(tile) => self.tiles.insert(key, tile),
      None => self.tiles.remove(&key),
    };
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
  /// The app's drawing guide for this canvas, saved in the file as is. Core does not read it;
  /// changing it is not an undoable edit.
  pub guide: Option<serde_json::Value>,
  /// Animation Assist: frames, speed and playback, saved in the file; changing it is not an
  /// undoable edit (like the guide).
  pub animation: Animation,
  /// The frame shown while playing (a layer index), instead of the active layer's frame.
  pub(crate) playing: Option<usize>,
  /// What painting is limited to; `None` = the whole canvas. Not saved in the file.
  selection: Option<Selection>,
  /// Brush, fill and gradient go to the active layer's mask instead of its pixels.
  editing_mask: bool,
}

impl Document {
  /// A canvas with one empty layer. Sides are clamped to `1..=MAX_SIDE`.
  pub fn new(width: u32, height: u32) -> Self {
    Self::with_history(width, height, History::default())
  }

  pub fn with_history(width: u32, height: u32, history: History) -> Self {
    let mut doc = Self {
      width: width.clamp(1, MAX_SIDE),
      height: height.clamp(1, MAX_SIDE),
      layers: Vec::new(),
      active: 0,
      next_id: 1,
      history,
      revision: 0,
      guide: None,
      animation: Animation::default(),
      playing: None,
      selection: None,
      editing_mask: false,
    };
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
    self.history.push(Snapshot::new(self.layers.clone(), self.active, self.selection.clone(), (self.width, self.height)));
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

  /// Goes back to the state before the last `checkpoint` and forgets that step, as if the
  /// change never happened (a cancelled transform).
  pub fn rollback(&mut self) {
    if let Some(state) = self.history.pop() {
      self.layers = state.layers;
      self.active = state.active;
      self.selection = state.selection;
      (self.width, self.height) = state.size;
      self.revision = self.revision.saturating_sub(1);
    }
  }

  fn restore(&mut self, step: impl FnOnce(&mut History, Snapshot) -> Result<Snapshot, Snapshot>) -> bool {
    let current = Snapshot::new(std::mem::take(&mut self.layers), self.active, self.selection.take(), (self.width, self.height));
    let (state, moved) = match step(&mut self.history, current) {
      Ok(state) => (state, true),
      Err(current) => (current, false),
    };
    self.layers = state.layers;
    self.active = state.active;
    self.selection = state.selection;
    (self.width, self.height) = state.size;
    self.revision += u64::from(moved);
    moved
  }

  /// A new empty layer above the active one, which becomes active. With a group active it goes
  /// in the group, on top; otherwise in the active layer's group.
  pub fn add_layer(&mut self) {
    self.checkpoint();
    let mut layer = self.new_layer();
    let active = &self.layers[self.active];
    if active.group {
      layer.parent = Some(active.id);
    } else {
      layer.parent = active.parent;
      self.active += 1;
    }
    self.layers.insert(self.active, layer);
    self.editing_mask = false;
  }

  /// Moves a layer to another position in the stack; the active layer follows its content.
  /// Only without groups (it could break one apart): use `move_up` and `move_down`.
  pub fn move_layer(&mut self, from: usize, to: usize) -> bool {
    let len = self.layers.len();
    if from >= len || to >= len || from == to || self.layers.iter().any(|l| l.group || l.parent.is_some()) {
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
    // Two plain layers side by side in the same group.
    let (upper, lower) = (&self.layers[index], &self.layers[index - 1]);
    if upper.group || lower.group || upper.parent != lower.parent {
      return false;
    }
    self.checkpoint();
    let upper = self.layers.remove(index);
    let lower = &mut self.layers[index - 1];
    lower.text = None;
    if !upper.hidden {
      for (&(tx, ty), tile) in &upper.tiles {
        let tile = &*blend::masked(tile, upper.mask.as_deref(), tx, ty);
        if upper.clip {
          // A clipped layer merges only where its base has paint.
          let Some(base) = lower.tile(tx, ty) else { continue };
          let masked = blend::clip(tile, base);
          blend::composite(lower.tile_mut(tx, ty), &masked, upper.opacity, upper.blend);
        } else {
          blend::composite(lower.tile_mut(tx, ty), tile, upper.opacity, upper.blend);
        }
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
    self.layers[index].text = None;
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

  /// Gives a layer an empty mask (everything shown) and starts editing it. `false` if it has one.
  pub fn add_mask(&mut self, index: usize) -> bool {
    if self.layers.get(index).is_none_or(|l| l.mask.is_some()) {
      return false;
    }
    self.checkpoint();
    let id = self.take_id();
    self.layers[index].mask = Some(Box::new(Layer::new(id, "Mask".into())));
    self.active = index;
    self.editing_mask = true;
    true
  }

  /// Removes a layer's mask; with `apply`, its pixels are cut by the mask first.
  pub fn remove_mask(&mut self, index: usize, apply: bool) -> bool {
    if self.layers.get(index).is_none_or(|l| l.mask.is_none()) {
      return false;
    }
    self.checkpoint();
    let layer = &mut self.layers[index];
    let mask = layer.mask.take().expect("checked");
    if apply {
      for key in layer.tiles.keys().copied().collect::<Vec<_>>() {
        let masked = blend::masked(&layer.tiles[&key], Some(&mask), key.0, key.1).into_owned();
        layer.set_tile(key, Some(Arc::from(masked)));
      }
    }
    if index == self.active {
      self.editing_mask = false;
    }
    true
  }

  /// Whether brush, fill and gradient paint the active layer's mask.
  pub fn editing_mask(&self) -> bool {
    self.editing_mask && self.layers[self.active].mask.is_some()
  }

  pub fn set_editing_mask(&mut self, on: bool) {
    self.editing_mask = on;
  }

  /// What painting changes: the active layer, or its mask while editing it.
  pub(crate) fn target(&self) -> &Layer {
    match &self.layers[self.active].mask {
      Some(mask) if self.editing_mask => mask,
      _ => &self.layers[self.active],
    }
  }

  pub(crate) fn target_mut(&mut self) -> &mut Layer {
    if self.editing_mask() { self.layers[self.active].mask.as_deref_mut().expect("editing a mask") } else { &mut self.layers[self.active] }
  }

  /// A layer or a mask by id (strokes and gradients remember which they paint).
  pub(crate) fn find_mut(&mut self, id: u64) -> Option<&mut Layer> {
    self.layers.iter_mut().find_map(|l| if l.id == id { Some(l) } else { l.mask.as_deref_mut().filter(|m| m.id == id) })
  }

  pub(crate) fn find(&self, id: u64) -> Option<&Layer> {
    self.layers.iter().find_map(|l| if l.id == id { Some(l) } else { l.mask.as_deref().filter(|m| m.id == id) })
  }

  /// A mask is painted in gray: color becomes how much it shows.
  pub(crate) fn paint_color(&self, color: [u8; 3]) -> [u8; 3] {
    if self.editing_mask() {
      let gray = ((color[0] as u32 * 54 + color[1] as u32 * 183 + color[2] as u32 * 19) / 256) as u8;
      [gray; 3]
    } else {
      color
    }
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

  #[test]
  fn alpha_lock_keeps_transparency() {
    let mut doc = Document::new(100, 100);
    doc.layer_mut(0).unwrap().tile_mut(0, 0)[..4 * 50].copy_from_slice(&[0, 0, 255, 255].repeat(50));
    doc.layer_mut(0).unwrap().alpha_lock = true;
    let mut stroke = doc.begin_stroke(Brush { size: 10.0, ..Brush::ink() }, [255, 0, 0], false).unwrap();
    stroke.line(&mut doc, (0.0, 0.0), (90.0, 0.0), 1.0);
    stroke.finish(&mut doc);
    let px = |doc: &Document, x: usize| doc.layers()[0].tile(0, 0).unwrap()[x * 4..x * 4 + 4].to_vec();
    assert_eq!(px(&doc, 20)[3], 255);
    assert!(px(&doc, 20)[0] > 200, "painted pixels take the color: {:?}", px(&doc, 20));
    assert_eq!(px(&doc, 70), vec![0, 0, 0, 0], "transparent pixels stay transparent");
    assert!(doc.fill(80, 50, [0, 255, 0], 0.1, false), "the bucket runs");
    assert_eq!(px(&doc, 70)[3], 0, "and keeps transparency too");
  }

  #[test]
  fn clipped_layers_show_only_over_their_base() {
    let mut doc = Document::new(10, 1);
    doc.layer_mut(0).unwrap().tile_mut(0, 0)[..4 * 5].copy_from_slice(&[0, 0, 255, 255].repeat(5));
    doc.add_layer();
    let top = doc.layer_mut(1).unwrap();
    top.tile_mut(0, 0)[..4 * 10].copy_from_slice(&[255, 0, 0, 255].repeat(10));
    top.clip = true;
    let tile = doc.composite_tile(0, 0).unwrap();
    assert_eq!((&tile[8..12], &tile[28..32]), (&[255u8, 0, 0, 255][..], &[0u8, 0, 0, 0][..]), "red only over the blue base");
    doc.layer_mut(0).unwrap().hidden = true;
    assert!(doc.composite_tile(0, 0).is_none_or(|t| t.iter().all(|&v| v == 0)), "a hidden base hides its clipped layers");
  }

  #[test]
  fn masks_hide_with_black_and_apply() {
    let mut doc = Document::new(10, 1);
    doc.layer_mut(0).unwrap().tile_mut(0, 0)[..4 * 10].copy_from_slice(&[0, 0, 255, 255].repeat(10));
    assert!(doc.add_mask(0) && doc.editing_mask() && !doc.add_mask(0));
    assert_eq!(doc.composite_tile(0, 0).unwrap()[3], 255, "an empty mask shows everything");
    doc.select_shape(&SelectShape::Rectangle((0.0, 0.0), (5.0, 1.0)), SelectMode::Replace);
    // A colored fill paints the mask in gray: pure red is dark, so it mostly hides.
    assert!(doc.fill(0, 0, [255, 0, 0], 0.1, false));
    let mask = doc.layers()[0].mask.as_ref().unwrap().tile(0, 0).unwrap();
    assert_eq!((mask[0], mask[1], mask[2]), (53, 53, 53), "masks are gray");
    assert!(doc.fill(0, 0, [0, 0, 0], 0.1, false));
    let tile = doc.composite_tile(0, 0).unwrap();
    assert_eq!((tile[3], tile[4 * 8 + 3]), (0, 255), "black hides, the rest shows");
    assert_eq!(doc.layers()[0].tile(0, 0).unwrap()[3], 255, "the layer's own pixels are untouched");
    assert!(doc.remove_mask(0, true) && !doc.editing_mask());
    assert_eq!(doc.layers()[0].tile(0, 0).unwrap()[3], 0, "applying cuts the pixels");
    doc.undo();
    assert!(doc.layers()[0].mask.is_some(), "undo brings the mask back");
  }
}
