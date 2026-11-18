//! Undo and redo (ADR 0005). A step is a snapshot of the layer stack. Tiles are `Arc`s that are
//! copied on write, so a snapshot only owns the tiles changed after it: undo memory is
//! proportional to what was painted, not to the canvas size.

use std::collections::VecDeque;
use std::sync::Arc;

use crate::{Layer, TILE_BYTES};

pub const DEFAULT_STEPS: usize = 250;
pub const DEFAULT_MEMORY: usize = 2 * 1024 * 1024 * 1024;

#[derive(Debug, Clone)]
pub(crate) struct Snapshot {
  pub layers: Vec<Layer>,
  pub active: usize,
  /// Bytes of tiles only this snapshot holds, measured once the step is done.
  bytes: usize,
}

impl Snapshot {
  pub fn new(layers: Vec<Layer>, active: usize) -> Self {
    Self { layers, active, bytes: 0 }
  }

  // shortcut: counts tiles nothing else references when measured; a tile later freed by an
  // evicted step is not re-counted. Precise enough for a memory cap; revisit if it drifts.
  fn measure(&mut self) {
    self.bytes = self.layers.iter().flat_map(|layer| layer.tiles.values()).filter(|tile| Arc::strong_count(tile) == 1).count() * TILE_BYTES;
  }
}

#[derive(Debug, Clone)]
pub struct History {
  undo: VecDeque<Snapshot>,
  redo: Vec<Snapshot>,
  max_steps: usize,
  max_bytes: usize,
}

impl Default for History {
  fn default() -> Self {
    Self::new(DEFAULT_STEPS, DEFAULT_MEMORY)
  }
}

impl History {
  pub fn new(max_steps: usize, max_bytes: usize) -> Self {
    Self { undo: VecDeque::new(), redo: Vec::new(), max_steps: max_steps.max(1), max_bytes }
  }

  pub fn can_undo(&self) -> bool {
    !self.undo.is_empty()
  }

  pub fn can_redo(&self) -> bool {
    !self.redo.is_empty()
  }

  pub fn steps(&self) -> usize {
    self.undo.len()
  }

  /// Memory the undo and redo steps hold on their own, in bytes.
  pub fn bytes(&self) -> usize {
    self.undo.iter().chain(&self.redo).map(|s| s.bytes).sum()
  }

  /// Records the state before a change. Clears redo, then drops the oldest steps over the limits.
  pub(crate) fn push(&mut self, before: Snapshot) {
    // The previous step is finished now: measure what it alone holds.
    if let Some(last) = self.undo.back_mut() {
      last.measure();
    }
    self.redo.clear();
    self.undo.push_back(before);
    while self.undo.len() > self.max_steps || (self.undo.len() > 1 && self.bytes() > self.max_bytes) {
      self.undo.pop_front();
    }
  }

  /// Swaps `current` with the previous step; gives `current` back if there is none.
  pub(crate) fn undo(&mut self, current: Snapshot) -> Result<Snapshot, Snapshot> {
    let Some(previous) = self.undo.pop_back() else { return Err(current) };
    self.redo.push(current);
    Ok(previous)
  }

  pub(crate) fn redo(&mut self, current: Snapshot) -> Result<Snapshot, Snapshot> {
    let Some(next) = self.redo.pop() else { return Err(current) };
    self.undo.push_back(current);
    Ok(next)
  }
}
