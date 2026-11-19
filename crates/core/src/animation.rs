//! Animation Assist, like Procreate's: every top-level layer or group is a frame, bottom to top.
//! With it on, only one frame shows: the active layer's, or the one playing.

use serde::{Deserialize, Serialize};

use crate::{Document, TILE};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Playback {
  #[default]
  Loop,
  /// Forward, then back.
  PingPong,
  /// Once, stopping on the last frame.
  OneShot,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Animation {
  pub enabled: bool,
  /// Frames per second, 1..=60.
  pub fps: u32,
  pub playback: Playback,
  /// Onion skin: how many frames before and after the shown one show faintly (0..=12), how
  /// strongly the nearest ones show, and whether earlier frames are tinted red and later green.
  pub onion_frames: u32,
  pub onion_opacity: f32,
  pub onion_colors: bool,
  /// The first frame is a background under every frame, and the last a foreground over every
  /// one; neither plays as a frame of its own.
  pub background: bool,
  pub foreground: bool,
}

/// The longest hold duration, in frames.
pub const MAX_HOLD: u32 = 60;
/// Onion skin tints: earlier frames, later frames.
pub const ONION_BEFORE: [u8; 3] = [230, 60, 60];
pub const ONION_AFTER: [u8; 3] = [40, 170, 90];

impl Default for Animation {
  fn default() -> Self {
    Self {
      enabled: false,
      fps: 12,
      playback: Playback::Loop,
      onion_frames: 1,
      onion_opacity: 0.4,
      onion_colors: true,
      background: false,
      foreground: false,
    }
  }
}

impl Animation {
  pub(crate) fn sanitized(self) -> Self {
    let opacity = if self.onion_opacity.is_finite() { self.onion_opacity.clamp(0.0, 1.0) } else { 0.4 };
    Self { fps: self.fps.clamp(1, 60), onion_frames: self.onion_frames.min(12), onion_opacity: opacity, ..self }
  }

  /// Which of `count` frames shows `step` frames after the first, and whether playback is over
  /// (one shot only).
  pub fn frame_at(&self, step: u64, count: usize) -> (usize, bool) {
    let count = count.max(1) as u64;
    match self.playback {
      Playback::Loop => ((step % count) as usize, false),
      Playback::OneShot => (step.min(count - 1) as usize, step >= count - 1),
      Playback::PingPong if count == 1 => (0, false),
      Playback::PingPong => {
        let at = step % (2 * count - 2);
        ((if at < count { at } else { 2 * count - 2 - at }) as usize, false)
      }
    }
  }
}

impl Document {
  /// The frames: layer indices of the top-level layers and groups, bottom (first frame) to top.
  pub fn frames(&self) -> Vec<usize> {
    (0..self.layers.len()).filter(|&i| self.layers[i].parent.is_none()).collect()
  }

  /// The background and foreground frames (layer indices), when they are on.
  fn fixed_frames(&self) -> (Option<usize>, Option<usize>) {
    let frames = self.frames();
    let background = frames.first().copied().filter(|_| self.animation.background && frames.len() > 1);
    let foreground = frames.last().copied().filter(|_| self.animation.foreground && frames.len() > 1 + usize::from(background.is_some()));
    (background, foreground)
  }

  /// The frames that play, in order: all but the background and foreground ones.
  pub fn animated_frames(&self) -> Vec<usize> {
    let (background, foreground) = self.fixed_frames();
    self.frames().into_iter().filter(|&i| Some(i) != background && Some(i) != foreground).collect()
  }

  /// Whether frame `index` is the background (`Some(false)`) or the foreground (`Some(true)`).
  pub fn fixed_frame(&self, index: usize) -> Option<bool> {
    match self.fixed_frames() {
      (Some(b), _) if b == index => Some(false),
      (_, Some(f)) if f == index => Some(true),
      _ => None,
    }
  }

  /// What plays, one entry per step: every animated frame, repeated for its hold duration.
  pub fn playback_sequence(&self) -> Vec<usize> {
    self.animated_frames().into_iter().flat_map(|i| std::iter::repeat_n(i, 1 + self.layers[i].hold.min(MAX_HOLD) as usize)).collect()
  }

  /// Onion skin for frame `index` on screen: its opacity, and its tint if colored. Only while
  /// not playing, for animated frames near the shown one.
  pub fn onion(&self, index: usize) -> Option<(f32, Option<[u8; 3]>)> {
    let a = &self.animation;
    if !a.enabled || self.playing.is_some() || a.onion_frames == 0 || self.layers[index].parent.is_some() || self.layers[index].hidden {
      return None;
    }
    let frames = self.animated_frames();
    let (Some(at), Some(here)) = (frames.iter().position(|&i| i == index), frames.iter().position(|&i| i == self.shown_frame())) else {
      return None;
    };
    let distance = at.abs_diff(here) as u32;
    if distance == 0 || distance > a.onion_frames {
      return None;
    }
    // The nearest at full onion opacity, fading to a third of it at the farthest.
    let fade = 1.0 - (distance - 1) as f32 / a.onion_frames as f32 * 0.67;
    let tint = a.onion_colors.then_some(if at < here { ONION_BEFORE } else { ONION_AFTER });
    Some((a.onion_opacity * fade, tint))
  }

  /// The frame (top-level layer index) that layer `index` belongs to.
  pub fn frame_of(&self, index: usize) -> usize {
    let mut index = index.min(self.layers.len() - 1);
    while let Some(parent) = self.layers[index].parent {
      match self.layers.iter().position(|l| l.id == parent) {
        Some(up) => index = up,
        None => break,
      }
    }
    index
  }

  /// The frame on the canvas with Animation Assist on: the one playing, else the active one's.
  pub fn shown_frame(&self) -> usize {
    self.playing.filter(|&i| i < self.layers.len()).unwrap_or_else(|| self.frame_of(self.active))
  }

  /// A new empty frame right after the active one (a top-level layer), which becomes active.
  /// One undo step.
  pub fn add_frame(&mut self) {
    let frame = self.frame_of(self.active);
    self.checkpoint();
    let layer = self.new_layer();
    self.layers.insert(frame + 1, layer);
    self.active = frame + 1;
    self.editing_mask = false;
  }

  /// Plays frame `index` (a top-level layer index), or `None` to stop and show the active one.
  pub fn set_playing(&mut self, index: Option<usize>) {
    self.playing = index;
  }

  /// Whether layer `index` is left out of the picture: hidden, or (with Animation Assist on)
  /// a frame other than the one shown. `frame` forces which frame shows, for thumbnails and
  /// export.
  pub(crate) fn hidden_in(&self, index: usize, frame: Option<usize>) -> bool {
    let layer = &self.layers[index];
    if layer.hidden {
      return true;
    }
    if layer.parent.is_some() {
      return false;
    }
    // Background and foreground frames show under and over every frame.
    let fixed = (frame.is_some() || self.animation.enabled) && self.fixed_frame(index).is_some();
    match frame {
      Some(frame) => frame != index && !fixed,
      None => self.animation.enabled && self.shown_frame() != index && !fixed,
    }
  }

  /// [`Document::hidden_in`] for the picture on screen.
  pub fn hides(&self, index: usize) -> bool {
    self.hidden_in(index, None)
  }

  /// A small picture of frame `index` (straight RGBA8), at most `side` pixels on its longer
  /// side, for the timeline.
  pub fn frame_thumbnail(&self, index: usize, side: u32) -> (u32, u32, Vec<u8>) {
    let scale = (side as f32 / self.width.max(self.height) as f32).min(1.0);
    let (w, h) = (((self.width as f32 * scale).round() as u32).max(1), ((self.height as f32 * scale).round() as u32).max(1));
    let mut tiles = std::collections::HashMap::new();
    let mut out = vec![0; (w * h * 4) as usize];
    for y in 0..h {
      for x in 0..w {
        let (cx, cy) = ((((x as f32 + 0.5) / scale) as u32).min(self.width - 1), (((y as f32 + 0.5) / scale) as u32).min(self.height - 1));
        let tile = tiles.entry((cx / TILE, cy / TILE)).or_insert_with(|| self.frame_tile(index, cx / TILE, cy / TILE));
        let Some(tile) = tile else { continue };
        let p = &tile[(((cy % TILE) * TILE + cx % TILE) * 4) as usize..][..4];
        let a = p[3] as f32 / 255.0;
        let straight = |c: u8| if a > 0.0 { (c as f32 / a).round().min(255.0) as u8 } else { 0 };
        out[((y * w + x) * 4) as usize..][..4].copy_from_slice(&[straight(p[0]), straight(p[1]), straight(p[2]), p[3]]);
      }
    }
    (w, h, out)
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn playback_orders() {
    let mut a = Animation::default();
    assert_eq!((0..6).map(|s| a.frame_at(s, 3).0).collect::<Vec<_>>(), [0, 1, 2, 0, 1, 2]);
    a.playback = Playback::PingPong;
    assert_eq!((0..8).map(|s| a.frame_at(s, 3).0).collect::<Vec<_>>(), [0, 1, 2, 1, 0, 1, 2, 1]);
    a.playback = Playback::OneShot;
    assert_eq!((0..5).map(|s| a.frame_at(s, 3)).collect::<Vec<_>>(), [(0, false), (1, false), (2, true), (2, true), (2, true)]);
  }

  #[test]
  fn only_the_shown_frame_is_in_the_picture() {
    let mut doc = Document::new(8, 8);
    doc.layer_mut(0).unwrap().tile_mut(0, 0)[..4].copy_from_slice(&[255, 0, 0, 255]);
    doc.add_layer();
    doc.layer_mut(1).unwrap().tile_mut(0, 0)[4..8].copy_from_slice(&[0, 0, 255, 255]);
    // A group holding a layer is one frame.
    doc.add_layer();
    doc.group_layer(2);
    assert_eq!(doc.frames(), vec![0, 1, 3]);
    // A frame added after a group goes after the whole group, not inside it.
    doc.select(2);
    doc.add_frame();
    assert_eq!((doc.frames(), doc.active()), (vec![0, 1, 3, 4], 4));
    doc.undo();
    assert_eq!(doc.frame_of(2), 3);

    let both = doc.composite_tile(0, 0).unwrap();
    assert!(both[3] == 255 && both[7] == 255, "off: every layer shows");
    doc.animation.enabled = true;
    doc.select(0);
    let first = doc.composite_tile(0, 0).unwrap();
    assert!(first[3] == 255 && first[7] == 0, "the active frame only");
    doc.set_playing(Some(1));
    let second = doc.composite_tile(0, 0).unwrap();
    assert!(second[3] == 0 && second[7] == 255, "the playing frame");
    assert_eq!(doc.frame_tile(0, 0, 0).unwrap()[3], 255, "thumbnails pick their frame");
    let (w, h, pixels) = doc.frame_thumbnail(1, 4);
    assert_eq!((w, h, pixels.len()), (4, 4, 64));

    // Hold, background and foreground; onion skin around the shown frame.
    doc.animation.onion_frames = 1;
    doc.set_playing(None);
    doc.select(1);
    assert!(doc.onion(0).is_some_and(|(_, tint)| tint == Some(ONION_BEFORE)));
    assert!(doc.onion(3).is_some_and(|(_, tint)| tint == Some(ONION_AFTER)));
    assert_eq!(doc.onion(1), None, "not the shown frame");
    doc.layer_mut(1).unwrap().hold = 2;
    assert_eq!(doc.playback_sequence(), vec![0, 1, 1, 1, 3]);
    doc.animation.background = true;
    assert_eq!(doc.playback_sequence(), vec![1, 1, 1, 3]);
    assert_eq!(doc.fixed_frame(0), Some(false));
    let with_background = doc.composite_tile(0, 0).unwrap();
    assert!(with_background[3] == 255 && with_background[7] == 255, "the background shows under frame 2");
    assert_eq!(doc.onion(0), None, "the background is no onion skin");

    // Saved and opened, the settings and holds stay.
    doc.animation.fps = 24;
    let path = std::env::temp_dir().join(format!("rustcreate-anim-{}.rcreate", std::process::id()));
    doc.save(&path).unwrap();
    let opened = Document::open(&path).unwrap();
    std::fs::remove_file(&path).ok();
    assert_eq!(opened.animation, doc.animation);
    assert_eq!(opened.layers()[1].hold, 2);
  }
}
