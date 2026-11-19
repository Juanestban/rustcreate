//! `.rcreate` documents and PNG export (ADR 0003).
//!
//! A `.rcreate` file is a zip with `document.json`, one full-canvas RGBA PNG per layer
//! (`layers/<id>.png`, unpremultiplied) and `thumbnail.png`. Images are written and read in
//! bands of one tile row, so a large canvas never needs a full-size buffer in memory.

use std::borrow::Cow;
use std::fs::File;
use std::io::{self, BufWriter, Cursor, Read, Seek, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use thiserror::Error;
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipArchive, ZipWriter};

use crate::{BlendMode, Document, History, Layer, MAX_SIDE, TILE, TILE_BYTES, blend};

/// The `format` this version writes and the newest it reads.
pub const FORMAT: u32 = 1;
const THUMBNAIL_SIDE: u32 = 256;

#[derive(Debug, Error)]
pub enum FileError {
  #[error(transparent)]
  Io(#[from] io::Error),
  #[error("damaged archive: {0}")]
  Zip(#[from] zip::result::ZipError),
  #[error("damaged document.json: {0}")]
  Json(#[from] serde_json::Error),
  #[error("could not write the image: {0}")]
  PngEncode(#[from] png::EncodingError),
  #[error("damaged image: {0}")]
  PngDecode(#[from] png::DecodingError),
  #[error("made with a newer rustcreate (format {0}); update the app to open it")]
  NewerFormat(u32),
  #[error("not a valid rustcreate document: {0}")]
  Invalid(String),
}

fn invalid(why: impl Into<String>) -> FileError {
  FileError::Invalid(why.into())
}

#[derive(Serialize, Deserialize)]
struct Manifest {
  format: u32,
  width: u32,
  height: u32,
  active: usize,
  /// Bottom to top.
  layers: Vec<LayerInfo>,
  /// The drawing guide (`Document::guide`); older files have none.
  #[serde(default, skip_serializing_if = "Option::is_none")]
  guide: Option<serde_json::Value>,
}

#[derive(Serialize, Deserialize)]
struct LayerInfo {
  /// Added after format 1 shipped: older files lack them.
  #[serde(default)]
  alpha_lock: bool,
  #[serde(default)]
  clip: bool,
  /// The layer has `masks/{id}.png`.
  #[serde(default)]
  mask: bool,
  /// A group: no `layers/{id}.png`.
  #[serde(default)]
  group: bool,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  parent: Option<u64>,
  id: u64,
  name: String,
  opacity: f32,
  hidden: bool,
  locked: bool,
  blend: BlendMode,
}

impl Document {
  /// Saves as `.rcreate`. Writes a temporary file next to `path` and renames it over the
  /// target, so a crash or a full disk never leaves a half-written document.
  pub fn save(&self, path: &Path) -> Result<(), FileError> {
    let mut temp = path.as_os_str().to_owned();
    temp.push(".tmp");
    let temp = PathBuf::from(temp);
    let result = self.write_rcreate(File::create(&temp)?).and_then(|file| Ok(file.sync_all()?)).and_then(|()| Ok(std::fs::rename(&temp, path)?));
    if result.is_err() {
      let _ = std::fs::remove_file(&temp);
    }
    result
  }

  fn write_rcreate(&self, file: File) -> Result<File, FileError> {
    let mut zip = ZipWriter::new(BufWriter::new(file));
    let stored = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);

    let manifest = Manifest {
      format: FORMAT,
      width: self.width,
      height: self.height,
      active: self.active,
      layers: self
        .layers
        .iter()
        .map(|l| LayerInfo {
          alpha_lock: l.alpha_lock,
          clip: l.clip,
          mask: l.mask.is_some(),
          group: l.group,
          parent: l.parent,
          id: l.id,
          name: l.name.clone(),
          opacity: l.opacity,
          hidden: l.hidden,
          locked: l.locked,
          blend: l.blend,
        })
        .collect(),
      guide: self.guide.clone(),
    };
    zip.start_file("document.json", stored)?;
    serde_json::to_writer_pretty(&mut zip, &manifest)?;

    for layer in &self.layers {
      if !layer.group {
        zip.start_file(format!("layers/{}.png", layer.id), stored)?;
        write_png(&mut zip, self.width, self.height, png::Compression::Fast, |tx, ty| layer.tile(tx, ty).map(Cow::Borrowed))?;
      }
      if let Some(mask) = &layer.mask {
        zip.start_file(format!("masks/{}.png", layer.id), stored)?;
        write_png(&mut zip, self.width, self.height, png::Compression::Fast, |tx, ty| mask.tile(tx, ty).map(Cow::Borrowed))?;
      }
    }

    zip.start_file("thumbnail.png", stored)?;
    let (thumb, tw, th) = self.thumbnail();
    write_png(&mut zip, tw, th, png::Compression::Balanced, |tx, ty| Some(Cow::Owned(crop_tile(&thumb, tw, th, tx, ty))))?;

    let writer = zip.finish()?;
    writer.into_inner().map_err(|e| e.into_error().into())
  }

  /// Opens a `.rcreate` file. Everything in it is checked: a damaged or hostile file is an
  /// error, never a panic or an oversized allocation.
  pub fn open(path: &Path) -> Result<Self, FileError> {
    Self::read_rcreate(File::open(path)?)
  }

  fn read_rcreate(reader: impl Read + Seek) -> Result<Self, FileError> {
    let mut zip = ZipArchive::new(reader)?;
    let manifest: Manifest = serde_json::from_reader(zip.by_name("document.json")?)?;
    if manifest.format > FORMAT {
      return Err(FileError::NewerFormat(manifest.format));
    }
    if !(1..=MAX_SIDE).contains(&manifest.width) || !(1..=MAX_SIDE).contains(&manifest.height) {
      return Err(invalid(format!("canvas size {}×{}", manifest.width, manifest.height)));
    }
    if manifest.layers.is_empty() {
      return Err(invalid("no layers"));
    }

    let mut doc = Document::with_history(manifest.width, manifest.height, History::default());
    doc.guide = manifest.guide;
    doc.layers.clear();
    for info in manifest.layers {
      if doc.layers.iter().any(|l| l.id == info.id) {
        return Err(invalid(format!("layer id {} repeated", info.id)));
      }
      let mut layer = Layer::new(info.id, info.name);
      (layer.group, layer.parent) = (info.group, info.parent);
      layer.opacity = if info.opacity.is_finite() { info.opacity.clamp(0.0, 1.0) } else { 1.0 };
      layer.hidden = info.hidden;
      layer.locked = info.locked;
      (layer.alpha_lock, layer.clip) = (info.alpha_lock, info.clip);
      layer.blend = info.blend;
      if !layer.group {
        let mut png = Vec::new();
        zip.by_name(&format!("layers/{}.png", info.id))?.read_to_end(&mut png)?;
        read_layer_png(&png, doc.width, doc.height, &mut layer)?;
      }
      if info.mask {
        let mut png = Vec::new();
        zip.by_name(&format!("masks/{}.png", info.id))?.read_to_end(&mut png)?;
        // Its id is given below, once every layer id is known.
        let mut mask = Layer::new(0, "Mask".into());
        read_layer_png(&png, doc.width, doc.height, &mut mask)?;
        layer.mask = Some(Box::new(mask));
      }
      doc.layers.push(layer);
    }
    if !crate::group::groups_valid(&doc.layers) || doc.layers.iter().all(|l| l.group) {
      return Err(invalid("layer groups do not nest properly"));
    }
    doc.next_id = doc.layers.iter().map(|l| l.id).max().unwrap_or(0) + 1;
    for i in 0..doc.layers.len() {
      if doc.layers[i].mask.is_some() {
        let id = doc.take_id();
        doc.layers[i].mask.as_mut().expect("checked").id = id;
      }
    }
    doc.active = manifest.active.min(doc.layers.len() - 1);
    Ok(doc)
  }

  /// Exports the visible layers, composited, as a PNG with transparency.
  pub fn export_png(&self, path: &Path) -> Result<(), FileError> {
    let mut out = BufWriter::new(File::create(path)?);
    write_png(&mut out, self.width, self.height, png::Compression::Balanced, |tx, ty| self.composite_tile(tx, ty).map(Cow::Owned))?;
    out.flush()?;
    Ok(())
  }

  /// The visible layers of one tile blended together (premultiplied), or `None` if no visible
  /// layer painted it.
  pub fn composite_tile(&self, tx: u32, ty: u32) -> Option<Vec<u8>> {
    self.composite_level(None, 0..self.layers.len(), tx, ty)
  }

  /// The layers in `range` whose group is `parent`, blended together; a group in there is
  /// composited on its own first (the GPU does the same in `render`).
  fn composite_level(&self, parent: Option<u64>, range: std::ops::Range<usize>, tx: u32, ty: u32) -> Option<Vec<u8>> {
    let mut out: Option<Vec<u8>> = None;
    // The base of a clipping group: its pixels, and whether it shows at all.
    // shortcut: clipping follows the base's paint, not its mask (the GPU does the same); pass
    // the base's mask along if people mask clipping bases.
    let mut base: (Option<Cow<[u8]>>, bool) = (None, false);
    let mut first = true;
    for i in range {
      let layer = &self.layers[i];
      if layer.parent != parent {
        continue;
      }
      let clipped = layer.clip && !first;
      first = false;
      let pixels: Option<Cow<[u8]>> = if layer.group {
        if layer.hidden { None } else { self.composite_level(Some(layer.id), crate::group_span(&self.layers, i), tx, ty).map(Cow::Owned) }
      } else {
        layer.tile(tx, ty).map(Cow::Borrowed)
      };
      if !clipped {
        base = (pixels.clone(), !layer.hidden);
      }
      if layer.hidden || (clipped && !base.1) {
        continue;
      }
      let Some(pixels) = pixels else { continue };
      let pixels = &*blend::masked(&pixels, layer.mask.as_deref(), tx, ty);
      let out = out.get_or_insert_with(|| vec![0; TILE_BYTES]);
      if clipped {
        // Only where the base has paint: scale the clipped layer by the base's alpha.
        let Some(under) = &base.0 else { continue };
        let masked = blend::clip(pixels, under);
        blend::composite(out, &masked, layer.opacity, layer.blend);
      } else {
        blend::composite(out, pixels, layer.opacity, layer.blend);
      }
    }
    out
  }

  /// The visible color at a canvas pixel, unpremultiplied RGBA (the eyedropper). Outside the
  /// canvas or where nothing is painted it is transparent.
  pub fn pixel(&self, x: u32, y: u32) -> [u8; 4] {
    if x >= self.width || y >= self.height {
      return [0; 4];
    }
    let Some(tile) = self.composite_tile(x / TILE, y / TILE) else { return [0; 4] };
    let p = (((y % TILE) * TILE + x % TILE) * 4) as usize;
    let mut rgba: [u8; 4] = tile[p..p + 4].try_into().expect("4 bytes");
    unpremultiply(&mut rgba);
    rgba
  }

  /// The composited image scaled down (box filter) to fit `THUMBNAIL_SIDE`: premultiplied
  /// pixels, width, height.
  fn thumbnail(&self) -> (Vec<u8>, u32, u32) {
    let scale = self.width.max(self.height).div_ceil(THUMBNAIL_SIDE).max(1);
    let (tw, th) = (self.width.div_ceil(scale), self.height.div_ceil(scale));
    let mut sums = vec![0u32; (tw * th * 4) as usize];
    let mut counts = vec![0u32; (tw * th) as usize];
    let (tiles_x, tiles_y) = self.tiles();
    for ty in 0..tiles_y {
      for tx in 0..tiles_x {
        let tile = self.composite_tile(tx, ty);
        for y in 0..TILE.min(self.height - ty * TILE) {
          for x in 0..TILE.min(self.width - tx * TILE) {
            let (cx, cy) = (tx * TILE + x, ty * TILE + y);
            let t = ((cy / scale) * tw + cx / scale) as usize;
            counts[t] += 1;
            if let Some(tile) = &tile {
              let p = ((y * TILE + x) * 4) as usize;
              (0..4).for_each(|c| sums[t * 4 + c] += tile[p + c] as u32);
            }
          }
        }
      }
    }
    let pixels = sums.iter().enumerate().map(|(i, &sum)| (sum / counts[i / 4].max(1)) as u8).collect();
    (pixels, tw, th)
  }
}

/// A `TILE`×`TILE` tile cut from a small `width`×`height` image, transparent outside it.
fn crop_tile(image: &[u8], width: u32, height: u32, tx: u32, ty: u32) -> Vec<u8> {
  let mut tile = vec![0; TILE_BYTES];
  for y in 0..TILE.min(height.saturating_sub(ty * TILE)) {
    let row = (tx * TILE..width.min((tx + 1) * TILE)).len() * 4;
    let src = (((ty * TILE + y) * width + tx * TILE) * 4) as usize;
    tile[(y * TILE * 4) as usize..][..row].copy_from_slice(&image[src..src + row]);
  }
  tile
}

/// Writes a `width`×`height` RGBA8 PNG from premultiplied tiles (`None` = transparent),
/// one tile row at a time.
fn write_png<'a>(
  out: impl Write,
  width: u32,
  height: u32,
  compression: png::Compression,
  tile: impl Fn(u32, u32) -> Option<Cow<'a, [u8]>>,
) -> Result<(), FileError> {
  let mut encoder = png::Encoder::new(out, width, height);
  encoder.set_color(png::ColorType::Rgba);
  encoder.set_depth(png::BitDepth::Eight);
  encoder.set_compression(compression);
  let mut writer = encoder.write_header()?;
  let mut stream = writer.stream_writer()?;
  let row_bytes = (width * 4) as usize;
  for ty in 0..height.div_ceil(TILE) {
    let rows = TILE.min(height - ty * TILE);
    let mut band = vec![0u8; row_bytes * rows as usize];
    for tx in 0..width.div_ceil(TILE) {
      let Some(tile) = tile(tx, ty) else { continue };
      let span = (TILE.min(width - tx * TILE) * 4) as usize;
      for y in 0..rows as usize {
        band[y * row_bytes + (tx * TILE * 4) as usize..][..span].copy_from_slice(&tile[y * TILE as usize * 4..][..span]);
      }
    }
    band.as_chunks_mut::<4>().0.iter_mut().for_each(unpremultiply);
    stream.write_all(&band)?;
  }
  stream.finish()?;
  Ok(())
}

/// Fills `layer` from a layer PNG, which must be exactly the canvas size. Fully transparent
/// tiles stay unallocated.
fn read_layer_png(png_bytes: &[u8], width: u32, height: u32, layer: &mut Layer) -> Result<(), FileError> {
  let mut decoder = png::Decoder::new(Cursor::new(png_bytes));
  // Palettes, gray and 16-bit become 8-bit; then only RGBA is accepted.
  decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
  let mut reader = decoder.read_info()?;
  let info = reader.info();
  if (info.width, info.height) != (width, height) {
    return Err(invalid(format!("layer {} is {}×{}, the canvas {width}×{height}", layer.id, info.width, info.height)));
  }
  if reader.output_color_type() != (png::ColorType::Rgba, png::BitDepth::Eight) {
    return Err(invalid(format!("layer {} is not RGBA", layer.id)));
  }
  let row_bytes = (width * 4) as usize;
  let mut band = vec![0u8; row_bytes * TILE as usize];
  for ty in 0..height.div_ceil(TILE) {
    let rows = TILE.min(height - ty * TILE) as usize;
    for y in 0..rows {
      let row = reader.next_row()?.ok_or_else(|| invalid(format!("layer {} image ends early", layer.id)))?;
      band[y * row_bytes..][..row_bytes].copy_from_slice(row.data());
    }
    for tx in 0..width.div_ceil(TILE) {
      let span = (TILE.min(width - tx * TILE) * 4) as usize;
      let offset = (tx * TILE * 4) as usize;
      if (0..rows).all(|y| band[y * row_bytes + offset..][..span].iter().skip(3).step_by(4).all(|&a| a == 0)) {
        continue;
      }
      let mut tile = vec![0u8; TILE_BYTES];
      for y in 0..rows {
        tile[y * TILE as usize * 4..][..span].copy_from_slice(&band[y * row_bytes + offset..][..span]);
      }
      tile.as_chunks_mut::<4>().0.iter_mut().for_each(premultiply);
      layer.tiles.insert((tx, ty), Arc::from(tile));
    }
  }
  Ok(())
}

fn premultiply(p: &mut [u8; 4]) {
  let a = p[3] as u32;
  for c in &mut p[..3] {
    *c = ((*c as u32 * a + 127) / 255) as u8;
  }
}

fn unpremultiply(p: &mut [u8; 4]) {
  let a = p[3] as u32;
  if a == 0 {
    *p = [0; 4];
    return;
  }
  for c in &mut p[..3] {
    *c = ((*c as u32 * 255 + a / 2) / a).min(255) as u8;
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rustcreate-test-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
  }

  fn sample() -> Document {
    // Not a multiple of the tile size, to cover the partial edge tiles.
    let mut doc = Document::new(300, 260);
    doc.layer_mut(0).unwrap().tile_mut(1, 1)[..4].copy_from_slice(&[200, 0, 0, 255]);
    doc.add_layer();
    let top = doc.layer_mut(1).unwrap();
    top.name = "Ink".into();
    top.opacity = 0.5;
    top.blend = BlendMode::Multiply;
    top.locked = true;
    top.tile_mut(0, 0)[..4].copy_from_slice(&[64, 64, 64, 128]);
    doc
  }

  #[test]
  fn save_and_open_round_trip() {
    let dir = temp_dir("roundtrip");
    let path = dir.join("art.rcreate");
    let mut doc = sample();
    doc.guide = Some(serde_json::json!({ "kind": "grid", "spacing": 50.0 }));
    doc.save(&path).unwrap();
    assert!(!dir.join("art.rcreate.tmp").exists(), "temporary file renamed away");

    let opened = Document::open(&path).unwrap();
    assert_eq!(opened.guide, doc.guide, "the guide comes back as saved");
    assert_eq!((opened.width, opened.height, opened.active()), (300, 260, 1));
    let top = &opened.layers()[1];
    assert_eq!((top.name.as_str(), top.opacity, top.blend, top.locked), ("Ink", 0.5, BlendMode::Multiply, true));
    assert_eq!(&opened.layers()[0].tile(1, 1).unwrap()[..4], &[200, 0, 0, 255]);
    assert_eq!(&top.tile(0, 0).unwrap()[..4], &[64, 64, 64, 128], "premultiplied pixels survive the round trip");
    assert_eq!(opened.layers()[0].painted_tiles(), 1, "empty tiles stay unallocated");
    assert!(!opened.history().can_undo());

    let mut opened = opened;
    opened.add_layer();
    assert_eq!(opened.layers()[2].id, 3, "new ids continue after the loaded ones");

    // A mask comes back with the layer and keeps hiding what it hid.
    assert!(opened.add_mask(0));
    assert!(opened.fill(5, 5, [0, 0, 0], 0.1, false));
    opened.save(&path).unwrap();
    let again = Document::open(&path).unwrap();
    assert!(again.layers()[0].mask.is_some());
    assert_eq!(again.composite_tile(1, 1).map(|t| t[3]), opened.composite_tile(1, 1).map(|t| t[3]));
    assert_ne!(again.layers()[0].mask.as_ref().unwrap().id, again.layers()[2].id, "mask ids do not clash");
    std::fs::remove_dir_all(dir).unwrap();
  }

  #[test]
  fn exports_a_composited_png() {
    let dir = temp_dir("export");
    let path = dir.join("art.png");
    let mut doc = Document::new(2, 1);
    doc.layer_mut(0).unwrap().tile_mut(0, 0)[..8].copy_from_slice(&[255, 255, 255, 255, 0, 0, 0, 0]);
    doc.add_layer();
    doc.layer_mut(1).unwrap().tile_mut(0, 0)[..4].copy_from_slice(&[128, 0, 0, 128]);
    doc.export_png(&path).unwrap();

    let decoder = png::Decoder::new(std::io::BufReader::new(File::open(&path).unwrap()));
    let mut reader = decoder.read_info().unwrap();
    let mut pixels = vec![0; reader.output_buffer_size().unwrap()];
    reader.next_frame(&mut pixels).unwrap();
    // Half-transparent red over white, then a transparent pixel (unpremultiplied).
    assert_eq!(pixels, [255, 127, 127, 255, 0, 0, 0, 0]);
    std::fs::remove_dir_all(dir).unwrap();
  }

  #[test]
  fn refuses_newer_or_broken_documents() {
    let write = |manifest: &str, layers: &[(&str, &[u8])]| {
      let mut zip = ZipWriter::new(Cursor::new(Vec::new()));
      zip.start_file("document.json", SimpleFileOptions::default()).unwrap();
      zip.write_all(manifest.as_bytes()).unwrap();
      for (name, bytes) in layers {
        zip.start_file(*name, SimpleFileOptions::default()).unwrap();
        zip.write_all(bytes).unwrap();
      }
      zip.finish().unwrap()
    };
    let layer = r#"{"id":1,"name":"a","opacity":1.0,"hidden":false,"locked":false,"blend":"normal"}"#;
    let manifest =
      |format: u32, side: u32, layers: &str| format!(r#"{{"format":{format},"width":{side},"height":{side},"active":0,"layers":[{layers}]}}"#);

    let newer = write(&manifest(2, 4, layer), &[]);
    assert!(matches!(Document::read_rcreate(newer), Err(FileError::NewerFormat(2))));
    let huge = write(&manifest(1, MAX_SIDE + 1, layer), &[]);
    assert!(matches!(Document::read_rcreate(huge), Err(FileError::Invalid(_))));
    let empty = write(&manifest(1, 4, ""), &[]);
    assert!(matches!(Document::read_rcreate(empty), Err(FileError::Invalid(_))));
    let missing_png = write(&manifest(1, 4, layer), &[]);
    assert!(matches!(Document::read_rcreate(missing_png), Err(FileError::Zip(_))));

    // A layer image that doesn't match the canvas size.
    let mut small = Vec::new();
    write_png(&mut small, 2, 2, png::Compression::Fast, |_, _| None).unwrap();
    let mismatched = write(&manifest(1, 4, layer), &[("layers/1.png", &small)]);
    assert!(matches!(Document::read_rcreate(mismatched), Err(FileError::Invalid(_))));
    assert!(matches!(Document::read_rcreate(Cursor::new(b"not a zip".to_vec())), Err(FileError::Zip(_))));
  }

  #[test]
  fn eyedropper_reads_the_visible_color() {
    let mut doc = Document::new(300, 300);
    doc.layer_mut(0).unwrap().tile_mut(1, 1)[..4].copy_from_slice(&[100, 50, 0, 200]);
    assert_eq!(doc.pixel(256, 256), [128, 64, 0, 200]);
    assert_eq!(doc.pixel(0, 0), [0; 4]);
    assert_eq!(doc.pixel(999, 0), [0; 4]);
  }

  #[test]
  fn thumbnail_fits_and_keeps_the_aspect() {
    let doc = Document::new(1024, 512);
    let (pixels, w, h) = doc.thumbnail();
    assert_eq!((w, h), (256, 128));
    assert_eq!(pixels.len(), (256 * 128 * 4) as usize);
    let (_, w, h) = Document::new(100, 50).thumbnail();
    assert_eq!((w, h), (100, 50), "small canvases are not scaled up");
  }

  #[test]
  fn premultiply_round_trips_opaque_and_clears_transparent() {
    let mut p = [10, 20, 30, 255];
    premultiply(&mut p);
    unpremultiply(&mut p);
    assert_eq!(p, [10, 20, 30, 255]);
    let mut clear = [9, 9, 9, 0];
    unpremultiply(&mut clear);
    assert_eq!(clear, [0; 4]);
  }
}
