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

impl Document {
  /// An H.264 MP4 on white (video has no transparency), each frame encoded once and held for
  /// its duration. The picture grows with white to an even size of at least 16 px, as H.264
  /// needs.
  pub fn export_mp4(&self, out: impl Write + std::io::Seek, done: &dyn Fn(usize)) -> Result<(), FileError> {
    use openh264::encoder::{BitRate, Encoder, EncoderConfig, FrameRate, FrameType};
    use openh264::formats::{RgbSliceU8, YUVBuffer};
    let video = |e: openh264::Error| FileError::Video(e.to_string());
    let (width, height) = (self.width as usize, self.height as usize);
    let (w, h) = (((width + 1) & !1).max(16), ((height + 1) & !1).max(16));
    let fps = self.animation.fps.clamp(1, 60);
    let config = EncoderConfig::new()
      .max_frame_rate(FrameRate::from_hz(fps as f32))
      // About 0.3 bits per pixel per frame: crisp lines without huge files.
      .bitrate(BitRate::from_bps(((w * h) as f64 * fps as f64 * 0.3).clamp(500_000.0, 50_000_000.0) as u32))
      .skip_frames(false);
    let mut encoder = Encoder::with_api_config(openh264::OpenH264API::from_source(), config).map_err(video)?;
    let mut mp4: Option<mp4::Mp4Writer<_>> = None;
    let mut out = Some(out);
    let mut time = 0u64;
    let mut rgb = vec![255u8; w * h * 3];
    for (n, (index, steps)) in self.export_frames().into_iter().enumerate() {
      let rgba = self.frame_rgba(index);
      for y in 0..height {
        for x in 0..width {
          let (s, d) = ((y * width + x) * 4, (y * w + x) * 3);
          let a = rgba[s + 3] as u32;
          for c in 0..3 {
            rgb[d + c] = ((rgba[s + c] as u32 * a + 255 * (255 - a) + 127) / 255) as u8;
          }
        }
      }
      let stream = encoder.encode(&YUVBuffer::from_rgb_source(RgbSliceU8::new(&rgb, (w, h)))).map_err(video)?;
      // Annex B NAL units to MP4's length-prefixed ones; parameter sets go in the track header.
      let (mut sample, mut sps, mut pps) = (Vec::new(), None, None);
      for layer in (0..stream.num_layers()).filter_map(|i| stream.layer(i)) {
        for nal in (0..layer.nal_count()).filter_map(|i| layer.nal_unit(i)) {
          let start = nal.iter().position(|&b| b != 0).map_or(nal.len(), |i| i + 1);
          let nal = &nal[start.min(nal.len())..];
          match nal.first().map(|b| b & 0x1f) {
            None => {}
            Some(7) => sps = Some(nal.to_vec()),
            Some(8) => pps = Some(nal.to_vec()),
            Some(_) => {
              sample.extend_from_slice(&(nal.len() as u32).to_be_bytes());
              sample.extend_from_slice(nal);
            }
          }
        }
      }
      let is_sync = matches!(stream.frame_type(), FrameType::IDR | FrameType::I);
      if mp4.is_none() {
        let (Some(sps), Some(pps)) = (sps, pps) else { return Err(FileError::Video("no parameter sets".into())) };
        let config = mp4::Mp4Config {
          major_brand: str::parse("isom").expect("brand"),
          minor_version: 512,
          compatible_brands: ["isom", "iso2", "avc1", "mp41"].iter().map(|b| str::parse(b).expect("brand")).collect(),
          timescale: fps,
        };
        let mut writer = mp4::Mp4Writer::write_start(out.take().expect("once"), &config).map_err(|e| FileError::Video(e.to_string()))?;
        let avc = mp4::AvcConfig { width: w as u16, height: h as u16, seq_param_set: sps, pic_param_set: pps };
        let track = mp4::TrackConfig {
          track_type: mp4::TrackType::Video,
          timescale: fps,
          language: "und".into(),
          media_conf: mp4::MediaConfig::AvcConfig(avc),
        };
        writer.add_track(&track).map_err(|e| FileError::Video(e.to_string()))?;
        mp4 = Some(writer);
      }
      let sample = mp4::Mp4Sample { start_time: time, duration: steps, rendering_offset: 0, is_sync, bytes: bytes::Bytes::from(sample) };
      mp4.as_mut().expect("started").write_sample(1, &sample).map_err(|e| FileError::Video(e.to_string()))?;
      time += steps as u64;
      done(n + 1);
    }
    if let Some(mut writer) = mp4 {
      writer.write_end().map_err(|e| FileError::Video(e.to_string()))?;
    }
    Ok(())
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

  #[test]
  fn mp4_holds_every_frame_for_its_duration() {
    let mut doc = animation();
    // Odd sizes grow to even ones, tiny ones to 16 px.
    doc.resize_canvas(33, 7, (0.0, 0.0));
    let mut out = std::io::Cursor::new(Vec::new());
    doc.export_mp4(&mut out, &|_| {}).unwrap();
    let bytes = out.into_inner();
    let size = bytes.len() as u64;
    let mut reader = mp4::Mp4Reader::read_header(std::io::Cursor::new(&bytes), size).unwrap();
    let track = reader.tracks().values().next().unwrap();
    assert_eq!((track.width(), track.height(), track.sample_count()), (34, 16, 3));
    let durations: Vec<u32> = (1..=3).map(|i| reader.read_sample(1, i).unwrap().unwrap().duration).collect();
    assert_eq!(durations, vec![1, 2, 1]);
  }
}
