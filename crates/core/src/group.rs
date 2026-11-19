//! Layer groups. A group is a layer without pixels; its children sit right below it in the
//! stack (bottom to top), so the stack stays one list. A group composites its children on their
//! own, then goes on what is below with its own opacity, blend mode and mask (Photoshop's
//! groups in Normal mode).

use std::collections::HashMap;
use std::ops::Range;

use crate::{Document, Layer};

impl Layer {
  pub fn is_group(&self) -> bool {
    self.group
  }

  /// The group this layer is in, by id.
  pub fn parent(&self) -> Option<u64> {
    self.parent
  }
}

/// The children (and theirs) of the group at `index`: the run right below it. Empty for a
/// plain layer.
pub fn group_span(layers: &[Layer], index: usize) -> Range<usize> {
  if !layers[index].group {
    return index..index;
  }
  let id = layers[index].id;
  let mut start = index;
  while start > 0 && inside(layers, start - 1, id) {
    start -= 1;
  }
  start..index
}

/// A layer with everything in it, if it is a group: what moves, is deleted or copied together.
fn block(layers: &[Layer], index: usize) -> Range<usize> {
  group_span(layers, index).start..index + 1
}

/// Whether the layer at `index` is in the group `id`, directly or deeper.
fn inside(layers: &[Layer], index: usize, id: u64) -> bool {
  let mut parent = layers[index].parent;
  // Bounded: a damaged stack must not loop forever.
  for _ in 0..layers.len() {
    match parent {
      Some(p) if p == id => return true,
      Some(p) => parent = layers.iter().find(|l| l.id == p).and_then(|l| l.parent),
      None => return false,
    }
  }
  false
}

/// Every parent is a group above its children, with all of them right below it. Files are
/// checked with this before they are trusted.
pub(crate) fn groups_valid(layers: &[Layer]) -> bool {
  layers.iter().enumerate().all(|(i, layer)| {
    let Some(p) = layer.parent else { return true };
    let Some(j) = layers.iter().position(|l| l.id == p) else { return false };
    layers[j].group && j > i && (i..j).all(|k| inside(layers, k, p))
  })
}

impl Document {
  /// How many groups the layer at `index` is in.
  pub fn depth(&self, index: usize) -> usize {
    let mut depth = 0;
    let mut parent = self.layers[index].parent;
    while let Some(p) = parent
      && depth < self.layers.len()
    {
      depth += 1;
      parent = self.layers.iter().find(|l| l.id == p).and_then(|l| l.parent);
    }
    depth
  }

  /// Brush, fill and gradient can work on the active layer: not locked, and not a group
  /// (unless its mask is being edited).
  pub(crate) fn paintable(&self) -> bool {
    let layer = &self.layers[self.active];
    !layer.locked && (!layer.group || self.editing_mask())
  }

  /// Ctrl+G: puts the layer at `index` (with all in it, if a group) in a new group, which
  /// becomes active.
  pub fn group_layer(&mut self, index: usize) -> bool {
    if index >= self.layers.len() {
      return false;
    }
    self.checkpoint();
    let id = self.take_id();
    let mut group = Layer::new(id, format!("Group {id}"));
    group.group = true;
    group.parent = self.layers[index].parent;
    self.layers[index].parent = Some(id);
    self.layers.insert(index + 1, group);
    self.active = index + 1;
    self.editing_mask = false;
    true
  }

  /// Ctrl+Shift+G: removes a group, keeping its children where they are.
  pub fn ungroup(&mut self, index: usize) -> bool {
    if !self.layers.get(index).is_some_and(|l| l.group) {
      return false;
    }
    self.checkpoint();
    let (id, parent) = (self.layers[index].id, self.layers[index].parent);
    for layer in &mut self.layers {
      if layer.parent == Some(id) {
        layer.parent = parent;
      }
    }
    self.layers.remove(index);
    if self.active >= index {
      self.active = self.active.saturating_sub(1).min(self.layers.len() - 1);
    }
    true
  }

  /// Deletes a layer, or a group with everything in it. At least one layer that can be painted
  /// on is always kept.
  pub fn delete_layer(&mut self, index: usize) -> bool {
    if index >= self.layers.len() {
      return false;
    }
    let block = block(&self.layers, index);
    if !self.layers.iter().enumerate().any(|(i, l)| !block.contains(&i) && !l.group) {
      return false;
    }
    self.checkpoint();
    self.layers.drain(block.clone());
    if self.active >= block.end {
      self.active -= block.len();
    } else if self.active >= block.start {
      self.active = block.start.min(self.layers.len() - 1);
    }
    true
  }

  /// A copy above the original (a group with all in it), which becomes active. Tiles are shared
  /// until painted.
  pub fn duplicate_layer(&mut self, index: usize) -> bool {
    if index >= self.layers.len() {
      return false;
    }
    let block = block(&self.layers, index);
    let mut copies = self.layers[block.clone()].to_vec();
    self.checkpoint();
    let mut ids = HashMap::new();
    for copy in &mut copies {
      let id = self.take_id();
      ids.insert(copy.id, id);
      copy.id = id;
      if let Some(mask) = &mut copy.mask {
        mask.id = self.take_id();
      }
    }
    for copy in &mut copies {
      copy.parent = copy.parent.map(|p| ids.get(&p).copied().unwrap_or(p));
    }
    let top = copies.last_mut().expect("the layer itself");
    top.name = format!("{} copy", top.name);
    self.layers.splice(index + 1..index + 1, copies);
    self.active = index + block.len();
    true
  }

  /// Ctrl+]: one step up among the layers next to it; a group's whole content moves with it,
  /// and a layer at the top of its group steps out of it.
  pub fn move_up(&mut self, index: usize) -> bool {
    let Some(layer) = self.layers.get(index) else { return false };
    let (block, parent) = (block(&self.layers, index), layer.parent);
    let Some(above) = self.layers.get(index + 1) else { return false };
    let (above_id, grandparent) = (above.id, above.parent);
    let active_id = self.layers[self.active].id;
    self.checkpoint();
    if Some(above_id) == parent {
      // Out of the group, just above it.
      self.layers[index].parent = grandparent;
      let moved: Vec<Layer> = self.layers.drain(block.clone()).collect();
      let header = index + 1 - block.len();
      self.layers.splice(header + 1..header + 1, moved);
    } else {
      // Over the next block at this level (a layer, or a group with its content).
      let mut top = index + 1;
      while self.layers[top].parent != parent {
        top += 1;
      }
      let moved: Vec<Layer> = self.layers.drain(block.clone()).collect();
      let at = top + 1 - block.len();
      self.layers.splice(at..at, moved);
    }
    self.active = self.layers.iter().position(|l| l.id == active_id).expect("still in the stack");
    true
  }

  /// Ctrl+[: one step down; a layer at the bottom of its group steps out of it.
  pub fn move_down(&mut self, index: usize) -> bool {
    let Some(layer) = self.layers.get(index) else { return false };
    let (block, parent) = (block(&self.layers, index), layer.parent);
    // At the bottom of its group (or of the whole stack): it can only step out of the group.
    let leaving = block.start == 0 || self.layers[block.start - 1].parent != parent;
    if leaving && parent.is_none() {
      return false;
    }
    let active_id = self.layers[self.active].id;
    self.checkpoint();
    if leaving {
      // Out of the group, in the same place.
      let header = self.layers.iter().position(|l| Some(l.id) == parent).expect("the parent is in the stack");
      self.layers[index].parent = self.layers[header].parent;
    } else {
      let under = self::block(&self.layers, block.start - 1);
      let moved: Vec<Layer> = self.layers.drain(block).collect();
      self.layers.splice(under.start..under.start, moved);
    }
    self.active = self.layers.iter().position(|l| l.id == active_id).expect("still in the stack");
    true
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::BlendMode;

  fn names(doc: &Document) -> Vec<String> {
    doc.layers().iter().map(|l| format!("{}{}", l.name, l.parent.map_or(String::new(), |p| format!("<{p}")))).collect()
  }

  #[test]
  fn group_moves_and_ungroup_keep_the_stack_whole() {
    let mut doc = Document::new(8, 8);
    doc.add_layer(); // Layer 2 above Layer 1
    assert!(doc.group_layer(1)); // Group 3 holds Layer 2
    assert_eq!(names(&doc), ["Layer 1", "Layer 2<3", "Group 3"]);
    doc.add_layer(); // with the group active: inside it, on top
    assert_eq!(names(&doc), ["Layer 1", "Layer 2<3", "Layer 4<3", "Group 3"]);
    assert!(groups_valid(doc.layers()) && doc.depth(2) == 1 && doc.depth(0) == 0);
    // Layer 1 steps up over the whole group.
    assert!(doc.move_up(0));
    assert_eq!(names(&doc), ["Layer 2<3", "Layer 4<3", "Group 3", "Layer 1"]);
    // Layer 4 (top of the group) steps out above it.
    assert!(doc.move_up(1));
    assert_eq!(names(&doc), ["Layer 2<3", "Group 3", "Layer 4", "Layer 1"]);
    // Layer 2 (bottom of the group) steps out below it, in place.
    assert!(doc.move_down(0));
    assert_eq!(names(&doc), ["Layer 2", "Group 3", "Layer 4", "Layer 1"]);
    assert!(groups_valid(doc.layers()));
    assert!(doc.ungroup(1));
    assert_eq!(names(&doc), ["Layer 2", "Layer 4", "Layer 1"]);
  }

  #[test]
  fn duplicate_and_delete_take_the_whole_group() {
    let mut doc = Document::new(8, 8);
    doc.add_layer();
    doc.group_layer(1);
    assert!(doc.duplicate_layer(2));
    assert_eq!(doc.layers().len(), 5);
    assert!(groups_valid(doc.layers()));
    let copy = &doc.layers()[4];
    assert!(copy.group && copy.name.ends_with("copy") && doc.layers()[3].parent == Some(copy.id), "the copy has its own child");
    assert!(doc.delete_layer(4));
    assert_eq!(doc.layers().len(), 3);
    assert!(doc.delete_layer(2) && doc.layers().len() == 1, "a group goes with its children");
    assert!(!doc.delete_layer(0), "the last layer stays");
  }

  #[test]
  fn a_group_composites_its_children_on_their_own() {
    let mut doc = Document::new(2, 1);
    // Bottom: white. In the group: gray in Multiply, which multiplies with nothing inside the
    // group (it is isolated), then the group goes on white at half opacity.
    doc.layer_mut(0).unwrap().tile_mut(0, 0)[..8].copy_from_slice(&[255; 8]);
    doc.add_layer();
    let top = doc.layer_mut(1).unwrap();
    top.tile_mut(0, 0)[..4].copy_from_slice(&[100, 100, 100, 255]);
    top.blend = BlendMode::Multiply;
    doc.group_layer(1);
    doc.layer_mut(2).unwrap().opacity = 0.5;
    let tile = doc.composite_tile(0, 0).unwrap();
    assert_eq!(&tile[..4], &[178, 178, 178, 255], "half of gray over white");
    assert_eq!(&tile[4..8], &[255; 4], "nothing in the group there");
    doc.layer_mut(2).unwrap().hidden = true;
    assert_eq!(&doc.composite_tile(0, 0).unwrap()[..4], &[255; 4], "a hidden group hides its children");
    doc.select(2);
    assert!(!doc.paintable(), "a group is not painted on");
    assert!(doc.begin_stroke(crate::Brush::ink(), [0, 0, 0], false).is_none());
  }

  #[test]
  fn groups_survive_save_and_open() {
    let mut doc = Document::new(8, 8);
    doc.add_layer();
    doc.layer_mut(1).unwrap().tile_mut(0, 0)[..4].copy_from_slice(&[9, 9, 9, 255]);
    doc.group_layer(1);
    let path = std::env::temp_dir().join(format!("rustcreate-groups-{}.rcr", std::process::id()));
    doc.save(&path).unwrap();
    let back = Document::open(&path).unwrap();
    std::fs::remove_file(&path).ok();
    assert_eq!(names(&back), names(&doc));
    assert!(back.layers()[2].is_group() && back.composite_tile(0, 0).unwrap()[..4] == [9, 9, 9, 255]);
  }
}
