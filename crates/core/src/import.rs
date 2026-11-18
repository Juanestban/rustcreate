//! Imports brushes the user owns: Procreate `.brushset`/`.brush` (zips with `Shape.png` and
//! `Grain.png` per brush) and Photoshop `.abr` v6+ (sampled tips). Only the shape and grain
//! images come across; the rest of each brush starts from sensible defaults.
//!
//! Like `Document::open`, everything is checked: a damaged or hostile file is an error, never a
//! panic or an oversized allocation.

use std::collections::BTreeMap;
use std::io::{Cursor, Read};

use crate::tip::{self, GRAIN_SIZE, MASK_SIZE};
use crate::{Brush, FileError, Grain, Tip};

/// An imported tip or grain, normalized (`MASK_SIZE`² or `GRAIN_SIZE`² gray PNG), for the app to
/// keep and register again at the next start with `load_brush_image`.
#[derive(Debug, Clone)]
pub struct BrushImage {
  pub grain: bool,
  pub id: u32,
  pub png: Vec<u8>,
}

#[derive(Debug, Default)]
pub struct Imported {
  /// Name and brush, in the file's order.
  pub brushes: Vec<(String, Brush)>,
  pub images: Vec<BrushImage>,
}

/// A `.brushset` folder's `Shape.png` and `Grain.png`.
type ShapeAndGrain = (Option<Vec<u8>>, Option<Vec<u8>>);

/// Biggest image accepted, per side.
const MAX_SIDE: usize = 8192;
/// Biggest file entry read into memory.
const MAX_ENTRY: u64 = 64 << 20;

fn invalid(why: impl Into<String>) -> FileError {
  FileError::Invalid(why.into())
}

/// Registers a tip or grain kept from an earlier import; returns its id.
pub fn load_brush_image(png: &[u8], grain: bool) -> Result<u32, FileError> {
  let (pixels, w, h) = decode_gray(png)?;
  Ok(normalize(&pixels, w, h, grain).id)
}

/// The defaults imported brushes start from.
fn imported(tip: Tip, grain: Option<Grain>) -> Brush {
  Brush {
    size: 40.0,
    tip,
    spacing: 0.1,
    flow: 1.0,
    pressure_size: 0.5,
    pressure_flow: 0.3,
    taper_start: 0.0,
    taper_end: 0.0,
    streamline: 0.3,
    grain: grain.unwrap_or(Grain::None),
    grain_depth: if grain.is_some() { 0.7 } else { 0.0 },
    ..Brush::ink()
  }
}

/// "Name", or "Name 1", "Name 2"… when the file holds several brushes.
fn names(set: &str, count: usize) -> impl Iterator<Item = String> + '_ {
  (1..=count).map(move |i| if count == 1 { set.to_owned() } else { format!("{set} {i}") })
}

/// A Procreate `.brushset` (one folder per brush) or `.brush` (files at the root).
pub fn import_brushset(bytes: &[u8], set: &str) -> Result<Imported, FileError> {
  let mut zip = zip::ZipArchive::new(Cursor::new(bytes))?;
  // Folder → (shape, grain).
  let mut brushes: BTreeMap<String, ShapeAndGrain> = BTreeMap::new();
  for i in 0..zip.len() {
    let mut entry = zip.by_index(i)?;
    let path = entry.name()?.into_owned();
    let (folder, file) = path.rsplit_once('/').unwrap_or(("", &path));
    let is_shape = file.eq_ignore_ascii_case("Shape.png");
    if !is_shape && !file.eq_ignore_ascii_case("Grain.png") {
      continue;
    }
    if entry.size() > MAX_ENTRY {
      return Err(invalid(format!("{path} is too big")));
    }
    let mut data = Vec::with_capacity(entry.size() as usize);
    entry.by_ref().take(MAX_ENTRY).read_to_end(&mut data)?;
    let slot = brushes.entry(folder.to_owned()).or_default();
    if is_shape { slot.0 = Some(data) } else { slot.1 = Some(data) }
  }
  if brushes.is_empty() {
    return Err(invalid("no brushes in this file"));
  }
  let mut out = Imported::default();
  let count = brushes.len();
  for ((shape, grain), name) in brushes.into_values().zip(names(set, count)) {
    // A brush without its own shape uses one of Procreate's, which is not ours to copy: round.
    let tip = match shape {
      Some(png) => Tip::Image(out.add(&png, false)?),
      None => Tip::Round,
    };
    let grain = grain.map(|png| out.add(&png, true)).transpose()?.map(Grain::Image);
    out.brushes.push((name, imported(tip, grain)));
  }
  Ok(out)
}

impl Imported {
  fn add(&mut self, png: &[u8], grain: bool) -> Result<u32, FileError> {
    let (pixels, w, h) = decode_gray(png)?;
    Ok(self.keep(normalize(&pixels, w, h, grain)))
  }

  fn keep(&mut self, image: BrushImage) -> u32 {
    let id = image.id;
    if !self.images.iter().any(|i| i.id == id && i.grain == image.grain) {
      self.images.push(image);
    }
    id
  }
}

/// A Photoshop `.abr`, version 6 or later: the sampled tips in its `samp` section.
pub fn import_abr(bytes: &[u8], set: &str) -> Result<Imported, FileError> {
  let mut r = Reader { bytes, at: 0 };
  let version = r.u16()?;
  if !(6..=10).contains(&version) {
    return Err(invalid(format!("Photoshop brush format {version} is not supported (only 6 and later)")));
  }
  let subversion = r.u16()?;
  let mut tips = Vec::new();
  while r.at + 12 <= bytes.len() {
    if r.take(4)? != b"8BIM" {
      return Err(invalid("damaged section"));
    }
    let key = r.take(4)?.to_vec();
    let size = r.u32()? as usize;
    let end = r.at.checked_add(size).filter(|&e| e <= bytes.len()).ok_or_else(|| invalid("section past the end"))?;
    if key == b"samp" {
      while r.at + 4 <= end {
        let length = r.u32()? as usize;
        let next = r.at.checked_add(length.next_multiple_of(4)).ok_or_else(|| invalid("sample past the end"))?;
        if let Some(tip) = sampled(&bytes[r.at..next.min(end)], subversion)? {
          tips.push(tip);
        }
        r.at = next;
      }
    }
    r.at = end;
  }
  if tips.is_empty() {
    return Err(invalid("no sampled brushes in this file"));
  }
  let mut out = Imported::default();
  let count = tips.len();
  for ((pixels, w, h), name) in tips.into_iter().zip(names(set, count)) {
    let id = out.keep(normalize(&pixels, w, h, false));
    out.brushes.push((name, imported(Tip::Image(id), None)));
  }
  Ok(out)
}

/// One sampled tip: header, bounds, depth, then raw or PackBits rows. `None` for tips this
/// importer skips (empty).
fn sampled(bytes: &[u8], subversion: u16) -> Result<Option<(Vec<f32>, usize, usize)>, FileError> {
  let mut r = Reader { bytes, at: if subversion == 1 { 47 } else { 301 } };
  let (top, left, bottom, right) = (r.i32()?, r.i32()?, r.i32()?, r.i32()?);
  let (depth, compressed) = (r.u16()?, r.u8()? != 0);
  let (w, h) = ((right - left).max(0) as usize, (bottom - top).max(0) as usize);
  if w == 0 || h == 0 {
    return Ok(None);
  }
  if w > MAX_SIDE || h > MAX_SIDE || !matches!(depth, 8 | 16) {
    return Err(invalid(format!("unsupported tip: {w}×{h}, {depth}-bit")));
  }
  let row = w * depth as usize / 8;
  let data = if compressed {
    let counts: Vec<usize> = (0..h).map(|_| r.u16().map(|c| c as usize)).collect::<Result<_, _>>()?;
    let mut data = Vec::with_capacity(row * h);
    for count in counts {
      unpack(r.take(count)?, row, &mut data)?;
    }
    data
  } else {
    r.take(row * h)?.to_vec()
  };
  // 16-bit samples: keep the high byte.
  let step = depth as usize / 8;
  Ok(Some((data.iter().step_by(step).map(|&v| v as f32 / 255.0).collect(), w, h)))
}

/// PackBits: one row of `row` bytes.
fn unpack(mut src: &[u8], row: usize, out: &mut Vec<u8>) -> Result<(), FileError> {
  let start = out.len();
  while let [n, rest @ ..] = src {
    let n = *n as i8;
    src = rest;
    if n >= 0 {
      let count = n as usize + 1;
      let literal = src.get(..count).ok_or_else(|| invalid("damaged tip data"))?;
      out.extend_from_slice(literal);
      src = &src[count..];
    } else if n != -128 {
      let (&value, rest) = src.split_first().ok_or_else(|| invalid("damaged tip data"))?;
      out.extend(std::iter::repeat_n(value, 1 + (-(n as i32)) as usize));
      src = rest;
    }
    if out.len() - start > row {
      return Err(invalid("damaged tip data"));
    }
  }
  out.resize(start + row, 0);
  Ok(())
}

/// Big-endian reads that fail instead of panicking.
pub(crate) struct Reader<'a> {
  pub(crate) bytes: &'a [u8],
  pub(crate) at: usize,
}

impl<'a> Reader<'a> {
  pub(crate) fn take(&mut self, n: usize) -> Result<&'a [u8], FileError> {
    let slice = self.bytes.get(self.at..self.at.checked_add(n).ok_or_else(|| invalid("damaged file"))?).ok_or_else(|| invalid("file ends early"))?;
    self.at += n;
    Ok(slice)
  }
  pub(crate) fn u8(&mut self) -> Result<u8, FileError> {
    Ok(self.take(1)?[0])
  }
  pub(crate) fn u16(&mut self) -> Result<u16, FileError> {
    Ok(u16::from_be_bytes(self.take(2)?.try_into().expect("2 bytes")))
  }
  pub(crate) fn u32(&mut self) -> Result<u32, FileError> {
    Ok(u32::from_be_bytes(self.take(4)?.try_into().expect("4 bytes")))
  }
  pub(crate) fn i32(&mut self) -> Result<i32, FileError> {
    Ok(i32::from_be_bytes(self.take(4)?.try_into().expect("4 bytes")))
  }
}

/// A PNG as 0..=1 coverage: brightness times alpha (white paints, as in Procreate's shapes).
fn decode_gray(png_bytes: &[u8]) -> Result<(Vec<f32>, usize, usize), FileError> {
  let mut decoder = png::Decoder::new(Cursor::new(png_bytes));
  decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
  let mut reader = decoder.read_info()?;
  let (w, h) = (reader.info().width as usize, reader.info().height as usize);
  if w == 0 || h == 0 || w > MAX_SIDE || h > MAX_SIDE {
    return Err(invalid(format!("image of {w}×{h}")));
  }
  let channels = reader.output_color_type().0.samples();
  let mut buffer = vec![0; reader.output_buffer_size().ok_or_else(|| invalid("image too big"))?];
  reader.next_frame(&mut buffer)?;
  let pixels = buffer
    .chunks_exact(channels)
    .take(w * h)
    .map(|p| {
      let (luma, alpha) = match *p {
        [v] => (v as f32, 255.0),
        [v, a] => (v as f32, a as f32),
        [r, g, b] => (0.2126 * r as f32 + 0.7152 * g as f32 + 0.0722 * b as f32, 255.0),
        [r, g, b, a, ..] => (0.2126 * r as f32 + 0.7152 * g as f32 + 0.0722 * b as f32, a as f32),
        [] => (0.0, 0.0),
      };
      luma / 255.0 * alpha / 255.0
    })
    .collect();
  Ok((pixels, w, h))
}

/// Resamples to the registry's size and registers it. Tips keep their proportions, centered;
/// grains are stretched (they tile).
fn normalize(pixels: &[f32], w: usize, h: usize, grain: bool) -> BrushImage {
  let n = if grain { GRAIN_SIZE } else { MASK_SIZE };
  let (sx, sy) = if grain {
    (w as f32 / n as f32, h as f32 / n as f32)
  } else {
    let s = w.max(h) as f32 / n as f32;
    (s, s)
  };
  let (ox, oy) = if grain { (0.0, 0.0) } else { ((w as f32 - n as f32 * sx) / 2.0, (h as f32 - n as f32 * sy) / 2.0) };
  // Averages up to 4×4 points per output pixel, enough to shrink big images without aliasing.
  let k = (sx.max(sy).ceil() as usize).clamp(1, 4);
  let at = |x: f32, y: f32| -> f32 {
    let (x, y) = (x.floor(), y.floor());
    if x < 0.0 || y < 0.0 || x >= w as f32 || y >= h as f32 { 0.0 } else { pixels[y as usize * w + x as usize] }
  };
  let mut out = Vec::with_capacity(n * n);
  for j in 0..n {
    for i in 0..n {
      let mut sum = 0.0;
      for a in 0..k {
        for b in 0..k {
          let (u, v) = ((i as f32 + (b as f32 + 0.5) / k as f32) * sx + ox, (j as f32 + (a as f32 + 0.5) / k as f32) * sy + oy);
          sum += at(u, v);
        }
      }
      out.push((sum / (k * k) as f32 * 255.0).round().clamp(0.0, 255.0) as u8);
    }
  }
  let id = tip::register(grain, &out);
  let mut png = Vec::new();
  let mut encoder = png::Encoder::new(&mut png, n as u32, n as u32);
  encoder.set_color(png::ColorType::Grayscale);
  encoder.set_depth(png::BitDepth::Eight);
  encoder.write_header().and_then(|mut w| w.write_image_data(&out)).expect("encoding to memory");
  BrushImage { grain, id, png }
}

#[cfg(test)]
mod tests {
  use std::io::Write;

  use super::*;

  fn gray_png(w: u32, h: u32, f: impl Fn(u32, u32) -> u8) -> Vec<u8> {
    let mut out = Vec::new();
    let mut encoder = png::Encoder::new(&mut out, w, h);
    encoder.set_color(png::ColorType::Grayscale);
    encoder.set_depth(png::BitDepth::Eight);
    let data: Vec<u8> = (0..h).flat_map(|y| (0..w).map(move |x| (x, y))).map(|(x, y)| f(x, y)).collect();
    encoder.write_header().unwrap().write_image_data(&data).unwrap();
    out
  }

  /// A white disc on black, `w`×`h`.
  fn disc(w: u32, h: u32) -> Vec<u8> {
    gray_png(w, h, |x, y| if (x as f32 - w as f32 / 2.0).hypot(y as f32 - h as f32 / 2.0) < w.min(h) as f32 * 0.4 { 255 } else { 0 })
  }

  #[test]
  fn brushset_folders_become_brushes_with_their_images() {
    let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let options = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    for (path, data) in [
      ("A/Shape.png", disc(300, 200)),
      ("A/Grain.png", gray_png(64, 64, |x, y| ((x ^ y) * 4) as u8)),
      ("B/Shape.png", disc(64, 64)),
      ("B/Brush.archive", vec![1, 2, 3]),
    ] {
      zip.start_file(path, options).unwrap();
      zip.write_all(&data).unwrap();
    }
    let bytes = zip.finish().unwrap().into_inner();
    let imported = import_brushset(&bytes, "Set").unwrap();
    let names: Vec<&str> = imported.brushes.iter().map(|(n, _)| n.as_str()).collect();
    assert_eq!(names, ["Set 1", "Set 2"]);
    let a = imported.brushes[0].1;
    assert!(matches!((a.tip, a.grain), (Tip::Image(_), Grain::Image(_))));
    assert_eq!(imported.images.len(), 3);
    // The registered tip paints: solid in the middle, empty in the corner.
    assert!(a.tip.coverage(0.0, 0.0, 1.0, 10.0) > 0.9 && a.tip.coverage(0.95, 0.95, 1.0, 10.0) < 0.05);
    // Kept images register again with the same id.
    let kept = &imported.images[0];
    assert_eq!(load_brush_image(&kept.png, kept.grain).unwrap(), kept.id);
    assert!(import_brushset(b"not a zip", "x").is_err());
  }

  /// An `.abr` v6.2 with one tip, PackBits-compressed when `rle`.
  fn abr(w: u16, h: u16, rle: bool) -> Vec<u8> {
    let rows: Vec<Vec<u8>> = (0..h).map(|y| (0..w).map(|x| if (x + y) % 3 == 0 { 255 } else { 0 }).collect()).collect();
    let mut sample = vec![0u8; 301];
    for v in [0i32, 0, h as i32, w as i32] {
      sample.extend(v.to_be_bytes());
    }
    sample.extend(8u16.to_be_bytes());
    sample.push(rle as u8);
    if rle {
      // Literal runs of up to 128 bytes.
      let packed: Vec<Vec<u8>> =
        rows.iter().map(|row| row.chunks(128).flat_map(|c| std::iter::once(c.len() as u8 - 1).chain(c.iter().copied())).collect()).collect();
      packed.iter().for_each(|p| sample.extend((p.len() as u16).to_be_bytes()));
      packed.iter().for_each(|p| sample.extend(p));
    } else {
      rows.iter().for_each(|r| sample.extend(r));
    }
    let mut section = (sample.len() as u32).to_be_bytes().to_vec();
    section.extend(&sample);
    section.resize(4 + sample.len().next_multiple_of(4), 0);
    let mut out = vec![0, 6, 0, 2];
    out.extend(b"8BIMdesc");
    out.extend(0u32.to_be_bytes());
    out.extend(b"8BIMsamp");
    out.extend((section.len() as u32).to_be_bytes());
    out.extend(section);
    out
  }

  #[test]
  fn abr_sampled_tips_import_raw_and_packed() {
    for rle in [false, true] {
      let imported = import_abr(&abr(200, 150, rle), "Ps").unwrap();
      assert_eq!(imported.brushes.len(), 1, "rle {rle}");
      assert_eq!(imported.brushes[0].0, "Ps");
      assert!(matches!(imported.brushes[0].1.tip, Tip::Image(_)));
    }
    let raw = import_abr(&abr(60, 60, false), "a").unwrap();
    let packed = import_abr(&abr(60, 60, true), "b").unwrap();
    assert_eq!(raw.images[0].id, packed.images[0].id, "same pixels either way");
  }

  #[test]
  fn damaged_abr_files_are_errors() {
    let good = abr(40, 40, true);
    assert!(import_abr(&[0, 1, 0, 0], "x").is_err(), "old format");
    for cut in [3, 10, 30, good.len() - 5] {
      assert!(import_abr(&good[..cut], "x").is_err(), "cut at {cut}");
    }
    let mut huge = abr(40, 40, false);
    // Bottom = 100 000 rows.
    let bottom = 4 + 8 + 4 + 8 + 4 + 4 + 301 + 8;
    huge[bottom..bottom + 4].copy_from_slice(&100_000i32.to_be_bytes());
    assert!(import_abr(&huge, "x").is_err());
  }
}
