//! Color palettes from and to other apps: Procreate `.swatches` (a zip with `Swatches.json`, HSB),
//! Photoshop `.aco` (v1 colors, v2 names) and Adobe Swatch Exchange `.ase` (one palette per
//! group). Colors come in as sRGB bytes; CMYK and Lab are converted without a color profile.
//!
//! Like the brush import, a damaged or hostile file is an error, never a panic.

use std::io::{Cursor, Read, Write};

use serde_json::{Value, json};
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipArchive, ZipWriter};

use crate::FileError;
use crate::import::Reader;

/// Most colors kept from one palette file.
const MAX_COLORS: usize = 4096;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SwatchFormat {
  Procreate,
  Aco,
  Ase,
}

impl SwatchFormat {
  pub fn from_extension(extension: &str) -> Option<Self> {
    match extension.to_ascii_lowercase().as_str() {
      "swatches" => Some(Self::Procreate),
      "aco" => Some(Self::Aco),
      "ase" => Some(Self::Ase),
      _ => None,
    }
  }
}

/// A named list of sRGB colors.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Swatches {
  pub name: String,
  pub colors: Vec<[u8; 3]>,
}

fn invalid(why: impl Into<String>) -> FileError {
  FileError::Invalid(why.into())
}

/// The palettes in a file; `name` names those the file leaves unnamed. Empty palettes are dropped.
pub fn import_swatches(bytes: &[u8], format: SwatchFormat, name: &str) -> Result<Vec<Swatches>, FileError> {
  let mut palettes = match format {
    SwatchFormat::Procreate => procreate(bytes, name)?,
    SwatchFormat::Aco => vec![aco(bytes, name)?],
    SwatchFormat::Ase => ase(bytes, name)?,
  };
  palettes.retain(|p| !p.colors.is_empty());
  for p in &mut palettes {
    p.colors.truncate(MAX_COLORS);
    if p.name.trim().is_empty() {
      p.name = name.to_owned();
    }
  }
  if palettes.is_empty() {
    return Err(invalid("no colors in this file"));
  }
  Ok(palettes)
}

/// One palette written in `format`.
pub fn export_swatches(palette: &Swatches, format: SwatchFormat) -> Result<Vec<u8>, FileError> {
  match format {
    SwatchFormat::Procreate => {
      let swatches: Vec<Value> = palette
        .colors
        .iter()
        .map(|&c| {
          let (h, s, v) = to_hsv(c);
          json!({ "hue": h, "saturation": s, "brightness": v, "alpha": 1, "colorSpace": 0 })
        })
        .collect();
      let mut zip = ZipWriter::new(Cursor::new(Vec::new()));
      zip.start_file("Swatches.json", SimpleFileOptions::default().compression_method(CompressionMethod::Stored))?;
      zip.write_all(&serde_json::to_vec(&json!([{ "name": palette.name, "swatches": swatches }]))?)?;
      Ok(zip.finish()?.into_inner())
    }
    SwatchFormat::Aco => {
      let count = palette.colors.len().min(u16::MAX as usize);
      let mut out = Vec::new();
      for version in [1u16, 2] {
        out.extend(version.to_be_bytes());
        out.extend((count as u16).to_be_bytes());
        for (i, c) in palette.colors.iter().take(count).enumerate() {
          out.extend(0u16.to_be_bytes());
          for ch in [c[0], c[1], c[2], 0] {
            out.extend((ch as u16 * 257).to_be_bytes());
          }
          if version == 2 {
            let name = utf16(&format!("{} {}", palette.name, i + 1));
            out.extend(0u16.to_be_bytes());
            out.extend((name.len() as u32 / 2).to_be_bytes());
            out.extend(name);
          }
        }
      }
      Ok(out)
    }
    SwatchFormat::Ase => {
      let block = |kind: u16, body: Vec<u8>| [kind.to_be_bytes().to_vec(), (body.len() as u32).to_be_bytes().to_vec(), body].concat();
      let named = |name: &str| {
        let name = utf16(name);
        [(name.len() as u16 / 2).to_be_bytes().to_vec(), name].concat()
      };
      let mut blocks = vec![block(0xC001, named(&palette.name))];
      for (i, c) in palette.colors.iter().enumerate() {
        let mut body = named(&format!("{} {}", palette.name, i + 1));
        body.extend(b"RGB ");
        for ch in c {
          body.extend((*ch as f32 / 255.0).to_be_bytes());
        }
        body.extend(2u16.to_be_bytes());
        blocks.push(block(0x0001, body));
      }
      blocks.push(block(0xC002, Vec::new()));
      let mut out = b"ASEF".to_vec();
      out.extend([0, 1, 0, 0]);
      out.extend((blocks.len() as u32).to_be_bytes());
      out.extend(blocks.concat());
      Ok(out)
    }
  }
}

/// UTF-16BE with the closing zero, as `.aco` and `.ase` store names.
fn utf16(text: &str) -> Vec<u8> {
  text.encode_utf16().chain([0]).flat_map(u16::to_be_bytes).collect()
}

fn read_utf16(r: &mut Reader, units: usize) -> Result<String, FileError> {
  let bytes = r.take(units.checked_mul(2).ok_or_else(|| invalid("damaged name"))?)?;
  let units: Vec<u16> = bytes.as_chunks::<2>().0.iter().map(|&b| u16::from_be_bytes(b)).take_while(|&u| u != 0).collect();
  Ok(String::from_utf16_lossy(&units))
}

fn procreate(bytes: &[u8], name: &str) -> Result<Vec<Swatches>, FileError> {
  let mut archive = ZipArchive::new(Cursor::new(bytes))?;
  let entry = archive.by_name("Swatches.json")?;
  if entry.size() > 16 << 20 {
    return Err(invalid("Swatches.json is too big"));
  }
  let mut text = Vec::new();
  entry.take(16 << 20).read_to_end(&mut text)?;
  let json: Value = serde_json::from_slice(&text)?;
  // One palette object, or a list of them.
  let list = match json {
    Value::Array(list) => list,
    one => vec![one],
  };
  Ok(
    list
      .iter()
      .map(|p| Swatches {
        name: p.get("name").and_then(Value::as_str).unwrap_or(name).to_owned(),
        colors: p
          .get("swatches")
          .and_then(Value::as_array)
          .into_iter()
          .flatten()
          // Empty slots are `null`.
          .filter_map(|s| {
            let get = |k| s.get(k).and_then(Value::as_f64).map(|v| v as f32);
            Some(from_hsv(get("hue")?, get("saturation")?, get("brightness")?))
          })
          .collect(),
      })
      .collect(),
  )
}

fn aco(bytes: &[u8], name: &str) -> Result<Swatches, FileError> {
  let mut r = Reader { bytes, at: 0 };
  let mut colors = Vec::new();
  // Version 1, then usually the same colors again as version 2 with names: keep the last read.
  while r.at < bytes.len() {
    let version = r.u16()?;
    if version != 1 && version != 2 {
      return Err(invalid(format!("unknown .aco version {version}")));
    }
    let count = r.u16()? as usize;
    colors.clear();
    for _ in 0..count {
      let space = r.u16()?;
      let v = [r.u16()?, r.u16()?, r.u16()?, r.u16()?];
      if version == 2 {
        r.u16()?;
        let units = r.u32()? as usize;
        read_utf16(&mut r, units)?;
      }
      let unit = |i: usize| v[i] as f32 / 65535.0;
      let color = match space {
        0 => Some([unit(0), unit(1), unit(2)].map(byte)),
        1 => Some(from_hsv(unit(0), unit(1), unit(2))),
        // Photoshop stores CMYK inverted: 0 is full ink.
        2 => Some(from_cmyk(1.0 - unit(0), 1.0 - unit(1), 1.0 - unit(2), 1.0 - unit(3))),
        7 => Some(from_lab(v[0] as f32 / 100.0, v[1] as i16 as f32 / 100.0, v[2] as i16 as f32 / 100.0)),
        8 => Some([byte(v[0] as f32 / 10000.0); 3]),
        _ => None,
      };
      colors.extend(color);
    }
  }
  Ok(Swatches { name: name.to_owned(), colors })
}

fn ase(bytes: &[u8], name: &str) -> Result<Vec<Swatches>, FileError> {
  let mut r = Reader { bytes, at: 0 };
  if r.take(4)? != b"ASEF" {
    return Err(invalid("not an Adobe Swatch Exchange file"));
  }
  r.take(4)?;
  let blocks = r.u32()?;
  // Colors outside any group go to a palette named after the file.
  let mut palettes = vec![Swatches { name: name.to_owned(), colors: Vec::new() }];
  let mut group = None;
  for _ in 0..blocks {
    let kind = r.u16()?;
    let length = r.u32()? as usize;
    let mut body = Reader { bytes: r.take(length)?, at: 0 };
    match kind {
      0xC001 => {
        let units = body.u16()? as usize;
        palettes.push(Swatches { name: read_utf16(&mut body, units)?, colors: Vec::new() });
        group = Some(palettes.len() - 1);
      }
      0xC002 => group = None,
      0x0001 => {
        let units = body.u16()? as usize;
        read_utf16(&mut body, units)?;
        let model = body.take(4)?;
        let mut f = || body.u32().map(f32::from_bits);
        let color = match model {
          b"RGB " => [f()?, f()?, f()?].map(byte),
          b"CMYK" => from_cmyk(f()?, f()?, f()?, f()?),
          b"Gray" => [byte(f()?); 3],
          // L is stored as 0..=1 by some apps and 0..=100 by others.
          b"LAB " => {
            let l = f()?;
            from_lab(if l <= 1.0 { l * 100.0 } else { l }, f()?, f()?)
          }
          _ => continue,
        };
        palettes[group.unwrap_or(0)].colors.push(color);
      }
      _ => {}
    }
  }
  Ok(palettes)
}

fn byte(v: f32) -> u8 {
  (v.clamp(0.0, 1.0) * 255.0).round() as u8
}

fn from_hsv(h: f32, s: f32, v: f32) -> [u8; 3] {
  let h = h.rem_euclid(1.0) * 6.0;
  let (s, v) = (s.clamp(0.0, 1.0), v.clamp(0.0, 1.0));
  let c = v * s;
  let x = c * (1.0 - (h % 2.0 - 1.0).abs());
  let (r, g, b) = match h as u32 {
    0 => (c, x, 0.0),
    1 => (x, c, 0.0),
    2 => (0.0, c, x),
    3 => (0.0, x, c),
    4 => (x, 0.0, c),
    _ => (c, 0.0, x),
  };
  [r, g, b].map(|ch| byte(ch + v - c))
}

fn to_hsv(c: [u8; 3]) -> (f32, f32, f32) {
  let [r, g, b] = c.map(|v| v as f32 / 255.0);
  let max = r.max(g).max(b);
  let delta = max - r.min(g).min(b);
  let h = if delta == 0.0 {
    0.0
  } else if max == r {
    ((g - b) / delta).rem_euclid(6.0)
  } else if max == g {
    (b - r) / delta + 2.0
  } else {
    (r - g) / delta + 4.0
  };
  (h / 6.0, if max == 0.0 { 0.0 } else { delta / max }, max)
}

/// shortcut: naive CMYK without a profile; colors from press profiles come out brighter, so
/// convert through an ICC profile once color management lands (Phase 3).
fn from_cmyk(c: f32, m: f32, y: f32, k: f32) -> [u8; 3] {
  [c, m, y].map(|ch| byte((1.0 - ch) * (1.0 - k)))
}

/// CIE Lab (D50, as Adobe stores it) to sRGB.
fn from_lab(l: f32, a: f32, b: f32) -> [u8; 3] {
  let fy = (l + 16.0) / 116.0;
  let (fx, fz) = (fy + a / 500.0, fy - b / 200.0);
  let f = |t: f32| if t > 6.0 / 29.0 { t * t * t } else { 3.0 * (6.0f32 / 29.0).powi(2) * (t - 4.0 / 29.0) };
  let (x, y, z) = (0.9642 * f(fx), f(fy), 0.8251 * f(fz));
  // Bradford-adapted D50 XYZ to linear sRGB.
  let linear = [3.1339 * x - 1.6169 * y - 0.4906 * z, -0.9788 * x + 1.9161 * y + 0.0335 * z, 0.0719 * x - 0.2290 * y + 1.4052 * z];
  linear.map(|v| byte(if v <= 0.003_130_8 { 12.92 * v } else { 1.055 * v.max(0.0).powf(1.0 / 2.4) - 0.055 }))
}

#[cfg(test)]
mod tests {
  use super::*;

  fn sample() -> Swatches {
    Swatches { name: "Sunset".into(), colors: vec![[214, 58, 52], [246, 206, 72], [56, 102, 206], [0, 0, 0], [255, 255, 255]] }
  }

  #[test]
  fn every_format_round_trips() {
    for format in [SwatchFormat::Procreate, SwatchFormat::Aco, SwatchFormat::Ase] {
      let bytes = export_swatches(&sample(), format).unwrap();
      let back = import_swatches(&bytes, format, "Imported").unwrap();
      assert_eq!(back.len(), 1, "{format:?}");
      let close = back[0].colors.iter().zip(&sample().colors).all(|(a, b)| a.iter().zip(b).all(|(x, y)| x.abs_diff(*y) <= 1));
      assert!(close && back[0].colors.len() == 5, "{format:?}: {:?}", back[0].colors);
      // `.aco` has no palette name; the others keep it.
      assert_eq!(back[0].name, if format == SwatchFormat::Aco { "Imported" } else { "Sunset" });
    }
  }

  #[test]
  fn reads_other_color_models() {
    assert_eq!(from_cmyk(0.0, 1.0, 1.0, 0.0), [255, 0, 0]);
    let white = from_lab(100.0, 0.0, 0.0);
    assert!(white.iter().all(|&v| v >= 254), "{white:?}");
    let red = from_lab(54.29, 80.81, 69.89);
    assert!(red[0] > 240 && red[1] < 30 && red[2] < 40, "{red:?}");
    assert_eq!(from_hsv(1.0 / 3.0, 1.0, 1.0), [0, 255, 0]);
  }

  #[test]
  fn damaged_files_are_errors() {
    for format in [SwatchFormat::Procreate, SwatchFormat::Aco, SwatchFormat::Ase] {
      let bytes = export_swatches(&sample(), format).unwrap();
      for cut in [0, 3, bytes.len() / 2, bytes.len() - 1] {
        assert!(import_swatches(&bytes[..cut], format, "x").is_err(), "{format:?} cut at {cut}");
      }
    }
    assert!(import_swatches(b"ASEF\0\x01\0\0\xff\xff\xff\xff", SwatchFormat::Ase, "x").is_err());
  }
}
