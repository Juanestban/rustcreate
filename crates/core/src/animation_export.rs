//! Animation export: GIF, animated PNG and PNG sequences, frame by frame from
//! `Document::frame_tile` (background and foreground frames included, onion skin never).

use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};

use crate::animation::Playback;
use crate::{Document, FileError};

impl Document {
  /// What an export holds, in order: (frame layer index, how many steps at the frame rate it
  /// lasts). Holds are durations; ping-pong comes back without repeating its ends.
  pub fn export_frames(&self) -> Vec<(usize, u32)> {
    let mut frames: Vec<(usize, u32)> = self.animated_frames().into_iter().map(|i| (i, 1 + self.layers[i].hold.min(crate::MAX_HOLD))).collect();
    if self.animation.playback == Playback::PingPong && frames.len() > 2 {
      let back: Vec<_> = frames[1..frames.len() - 1].iter().rev().copied().collect();
      frames.extend(back);
    }
    frames
  }

  /// Frame `index` as straight RGBA8 over the whole canvas.
  pub fn frame_rgba(&self, index: usize) -> Vec<u8> {
    self.straight(|tx, ty| self.frame_tile(index, tx, ty))
  }

  /// An animated GIF: 256 colors per frame, binary transparency. `done(n)` after each frame.
  pub fn export_gif(&self, out: impl Write, done: &dyn Fn(usize)) -> Result<(), FileError> {
    let (width, height) = (u16::try_from(self.width), u16::try_from(self.height));
    let (Ok(width), Ok(height)) = (width, height) else { return Err(FileError::Invalid("too large for a GIF (65535 px)".into())) };
    let mut encoder = gif::Encoder::new(BufWriter::new(out), width, height, &[])?;
    encoder.set_repeat(if self.animation.playback == Playback::OneShot { gif::Repeat::Finite(0) } else { gif::Repeat::Infinite })?;
    let fps = self.animation.fps.max(1) as f64;
    // Delays are in hundredths of a second: carry the rounding so the total time stays right.
    let (mut time, mut written) = (0.0f64, 0u64);
    for (n, (index, steps)) in self.export_frames().into_iter().enumerate() {
      let mut pixels = self.frame_rgba(index);
      let mut frame = gif::Frame::from_rgba_speed(width, height, &mut pixels, 10);
      time += steps as f64 / fps * 100.0;
      let delay = (time.round() as u64).saturating_sub(written).clamp(2, u16::MAX as u64);
      written += delay;
      frame.delay = delay as u16;
      // Each frame replaces the last; nothing of it stays under transparent pixels.
      frame.dispose = gif::DisposalMethod::Background;
      encoder.write_frame(&frame)?;
      done(n + 1);
    }
    Ok(())
  }

  /// An animated PNG: full color and transparency.
  pub fn export_apng(&self, out: impl Write, done: &dyn Fn(usize)) -> Result<(), FileError> {
    let frames = self.export_frames();
    let mut encoder = png::Encoder::new(BufWriter::new(out), self.width, self.height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let plays = if self.animation.playback == Playback::OneShot { 1 } else { 0 };
    encoder.set_animated(frames.len().max(1) as u32, plays)?;
    let mut writer = encoder.write_header()?;
    let fps = self.animation.fps.clamp(1, 60) as u16;
    for (n, (index, steps)) in frames.into_iter().enumerate() {
      writer.set_dispose_op(png::DisposeOp::Background)?;
      writer.set_blend_op(png::BlendOp::Source)?;
      writer.set_frame_delay(steps.min(u16::MAX as u32) as u16, fps)?;
      writer.write_image_data(&self.frame_rgba(index))?;
      done(n + 1);
    }
    writer.finish()?;
    Ok(())
  }

  /// One PNG per step at the frame rate (a held frame repeats), named `{stem}_0001.png`… in
  /// `dir`, so a video editor gets the timing by importing at the same frame rate. Returns the
  /// files written.
  pub fn export_png_sequence(&self, dir: &Path, stem: &str, done: &dyn Fn(usize)) -> Result<Vec<PathBuf>, FileError> {
    let mut paths = Vec::new();
    for (n, (index, steps)) in self.export_frames().into_iter().enumerate() {
      let pixels = self.frame_rgba(index);
      for _ in 0..steps {
        let path = dir.join(format!("{stem}_{:04}.png", paths.len() + 1));
        let mut encoder = png::Encoder::new(BufWriter::new(File::create(&path)?), self.width, self.height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder.write_header()?.write_image_data(&pixels)?;
        paths.push(path);
      }
      done(n + 1);
    }
    Ok(paths)
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  /// Three frames (red, green, blue dots), the middle one held a step.
  fn animation() -> Document {
    let mut doc = Document::new(16, 8);
    for (i, color) in [[255, 0, 0, 255], [0, 255, 0, 255], [0, 0, 255, 255]].into_iter().enumerate() {
      if i > 0 {
        doc.add_frame();
      }
      doc.layer_mut(i).unwrap().tile_mut(0, 0)[..4].copy_from_slice(&color);
    }
    doc.layer_mut(1).unwrap().hold = 1;
    doc.animation.enabled = true;
    doc.animation.fps = 10;
    doc
  }

  #[test]
  fn frames_play_in_order_with_holds_and_ping_pong() {
    let mut doc = animation();
    assert_eq!(doc.export_frames(), vec![(0, 1), (1, 2), (2, 1)]);
    doc.animation.playback = Playback::PingPong;
    assert_eq!(doc.export_frames(), vec![(0, 1), (1, 2), (2, 1), (1, 2)]);
    assert_eq!(&doc.frame_rgba(2)[..4], &[0, 0, 255, 255]);
  }

  #[test]
  fn gif_apng_and_png_sequence_hold_every_frame() {
    let doc = animation();
    let mut gif = Vec::new();
    doc.export_gif(&mut gif, &|_| {}).unwrap();
    let mut options = gif::DecodeOptions::new();
    options.set_color_output(gif::ColorOutput::RGBA);
    let mut decoder = options.read_info(&gif[..]).unwrap();
    let mut seen = Vec::new();
    while let Some(frame) = decoder.read_next_frame().unwrap() {
      seen.push((frame.delay, frame.buffer[..4].to_vec()));
    }
    assert_eq!(seen, vec![(10, vec![255, 0, 0, 255]), (20, vec![0, 255, 0, 255]), (10, vec![0, 0, 255, 255])]);

    let mut apng = Vec::new();
    doc.export_apng(&mut apng, &|_| {}).unwrap();
    let mut reader = png::Decoder::new(std::io::Cursor::new(&apng)).read_info().unwrap();
    assert_eq!(reader.info().animation_control().map(|a| a.num_frames), Some(3));
    let mut buffer = vec![0; reader.output_buffer_size().unwrap()];
    reader.next_frame(&mut buffer).unwrap();
    assert_eq!(&buffer[..4], &[255, 0, 0, 255]);

    let dir = std::env::temp_dir().join(format!("rustcreate-seq-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let paths = doc.export_png_sequence(&dir, "walk", &|_| {}).unwrap();
    assert_eq!(paths.len(), 4, "the held frame is written twice");
    assert!(paths[3].ends_with("walk_0004.png"));
    std::fs::remove_dir_all(&dir).ok();
  }
}
