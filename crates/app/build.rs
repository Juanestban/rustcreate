//! Embeds the app icon (rendered from `assets/icon.svg`) and version info in the Windows
//! executable. Named, versioned executables with an icon look less suspicious to antivirus.

fn main() {
  println!("cargo::rerun-if-changed=assets/icon.svg");
  // Native Windows builds only (the build-dependencies are Windows-only too).
  #[cfg(windows)]
  windows::embed_icon();
}

#[cfg(windows)]
mod windows {
  use resvg::{tiny_skia, usvg};

  pub fn embed_icon() {
    let out = std::path::PathBuf::from(std::env::var("OUT_DIR").expect("cargo sets OUT_DIR"));
    let ico = out.join("rustcreate.ico");
    let svg = usvg::Tree::from_data(include_bytes!("assets/icon.svg"), &usvg::Options::default()).expect("assets/icon.svg parses");
    std::fs::write(&ico, ico_bytes(&svg, &[16, 24, 32, 48, 64, 256])).expect("write the icon");

    let mut resource = winresource::WindowsResource::new();
    resource.set_icon(ico.to_str().expect("UTF-8 path"));
    resource.set("ProductName", "rustcreate");
    resource.set("FileDescription", "rustcreate");
    resource.set("CompanyName", "rustcreate");
    resource.set("LegalCopyright", "rustcreate contributors");
    // Without the Windows SDK's resource compiler the exe just has no icon; still builds.
    if let Err(error) = resource.compile() {
      println!("cargo::warning=executable icon not embedded: {error}");
    }
  }

  /// An .ico holding one PNG image per size (supported since Windows Vista).
  fn ico_bytes(svg: &usvg::Tree, sizes: &[u32]) -> Vec<u8> {
    let images: Vec<Vec<u8>> = sizes.iter().map(|&size| png(svg, size)).collect();
    let mut out = vec![0, 0, 1, 0];
    out.extend((sizes.len() as u16).to_le_bytes());
    let mut offset = 6 + 16 * sizes.len() as u32;
    for (&size, image) in sizes.iter().zip(&images) {
      let side = if size >= 256 { 0 } else { size as u8 };
      out.extend([side, side, 0, 0]);
      out.extend(1u16.to_le_bytes()); // planes
      out.extend(32u16.to_le_bytes()); // bits per pixel
      out.extend((image.len() as u32).to_le_bytes());
      out.extend(offset.to_le_bytes());
      offset += image.len() as u32;
    }
    images.into_iter().for_each(|image| out.extend(image));
    out
  }

  fn png(svg: &usvg::Tree, size: u32) -> Vec<u8> {
    let mut pixmap = tiny_skia::Pixmap::new(size, size).expect("non-zero size");
    let scale = size as f32 / svg.size().width();
    resvg::render(svg, tiny_skia::Transform::from_scale(scale, scale), &mut pixmap.as_mut());
    pixmap.encode_png().expect("encode PNG")
  }
}
