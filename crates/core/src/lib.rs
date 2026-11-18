//! The document model: a canvas of tiled RGBA8 raster layers (ADR 0005). No GPU here.

use std::collections::HashMap;

/// Tile side in pixels.
pub const TILE: u32 = 256;
/// Largest canvas side (ADR 0005); the GPU may allow less.
pub const MAX_SIDE: u32 = 16384;

/// Premultiplied RGBA8 pixels of one tile, row-major.
pub type TilePixels = Box<[u8; (TILE * TILE * 4) as usize]>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
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
  pub name: String,
  pub opacity: f32,
  pub hidden: bool,
  pub locked: bool,
  pub blend: BlendMode,
  /// Only tiles that were painted exist; a missing tile is fully transparent.
  tiles: HashMap<(u32, u32), TilePixels>,
}

impl Layer {
  pub fn new(name: impl Into<String>) -> Self {
    Self { name: name.into(), opacity: 1.0, hidden: false, locked: false, blend: BlendMode::Normal, tiles: HashMap::new() }
  }

  pub fn tile(&self, tx: u32, ty: u32) -> Option<&TilePixels> {
    self.tiles.get(&(tx, ty))
  }

  /// The tile at `(tx, ty)`, created transparent if it was empty.
  pub fn tile_mut(&mut self, tx: u32, ty: u32) -> &mut TilePixels {
    self.tiles.entry((tx, ty)).or_insert_with(|| Box::new([0; (TILE * TILE * 4) as usize]))
  }

  pub fn painted_tiles(&self) -> usize {
    self.tiles.len()
  }
}

#[derive(Debug, Clone)]
pub struct Document {
  pub width: u32,
  pub height: u32,
  /// Bottom to top.
  pub layers: Vec<Layer>,
}

impl Document {
  /// A canvas with one empty layer. Sides are clamped to `1..=MAX_SIDE`.
  pub fn new(width: u32, height: u32) -> Self {
    Self { width: width.clamp(1, MAX_SIDE), height: height.clamp(1, MAX_SIDE), layers: vec![Layer::new("Layer 1")] }
  }

  /// Tiles across and down.
  pub fn tiles(&self) -> (u32, u32) {
    (self.width.div_ceil(TILE), self.height.div_ceil(TILE))
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn tiles_are_allocated_lazily() {
    let mut layer = Layer::new("ink");
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
}
