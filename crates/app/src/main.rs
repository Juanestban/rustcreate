#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod canvas;
mod guides;
mod i18n;
mod library;
mod log;
mod palettes;
mod pen;
mod settings;
mod tools;
mod ui;
mod update;
mod updates;

use canvas::Canvas;
use eframe::egui::RichText;
use eframe::{egui, egui_wgpu};
use i18n::I18n;
use std::path::PathBuf;

use rustcreate_core::Document;
use settings::{Settings, Theme};
use ui::chrome::Popover;
use updates::{Status, Updates};

fn main() -> eframe::Result {
  log::start();
  update::clean_up();
  let icon = eframe::icon_data::from_png_bytes(include_bytes!("../assets/icon-256.png")).expect("bundled icon is a PNG");
  let options = eframe::NativeOptions {
    viewport: egui::ViewportBuilder::default().with_title("rustcreate").with_icon(icon).with_inner_size([1280.0, 800.0]),
    ..Default::default()
  };
  let result = eframe::run_native("rustcreate", options, Box::new(|cc| Ok(Box::new(App::new(cc)))));
  if let Err(error) = &result {
    log::line(&format!("failed: {error}"));
    startup_failed(error);
  }
  result
}

/// The window could not start, almost always for want of a DX12/Vulkan GPU (ADR 0001). A release
/// build has no console, so say it in a native dialog, in the user's language.
fn startup_failed(error: &eframe::Error) {
  let i18n = I18n::new(Settings::load().language());
  let reason = if matches!(error, eframe::Error::Wgpu(_)) { i18n.tr("ui-error-gpu") } else { i18n.tr("ui-error-start") };
  rfd::MessageDialog::new().set_level(rfd::MessageLevel::Error).set_title("rustcreate").set_description(format!("{reason}\n\n{error}")).show();
}

/// Something that would drop unsaved changes, held until the user answers "Save changes?".
/// How many files File > Open recent lists.
const RECENT_FILES: usize = 10;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Export {
  Png,
  Jpeg,
  Webp,
  Gif,
  AnimatedPng,
  PngSequence,
}

/// An animation export running on its own thread: frames written so far, out of `total`, and
/// its outcome once it ends.
struct Exporting {
  done: std::sync::Arc<std::sync::atomic::AtomicUsize>,
  total: usize,
  outcome: std::sync::Arc<std::sync::Mutex<Option<Result<(), String>>>>,
}

pub(crate) enum Pending {
  New(u32, u32),
  /// A file, or ask which one.
  Open(Option<PathBuf>),
  /// Back to the file as last saved.
  Revert,
  /// The app's window.
  Close,
  Install(update::Update),
}

pub(crate) struct App {
  ctx: egui::Context,
  i18n: I18n,
  updates: Updates,
  render_state: egui_wgpu::RenderState,
  doc: Document,
  canvas: Canvas,
  /// Where the document was last saved or opened from.
  path: Option<PathBuf>,
  /// `doc.revision()` at the last save: anything else is unsaved.
  saved: u64,
  popover: Option<Popover>,
  /// The layer being renamed and the text typed so far.
  renaming: Option<(u64, String)>,
  /// A layer opacity drag in progress (one undo step per drag).
  editing_slider: bool,
  /// Animation playback: when it started (egui time) and the timeline's frame thumbnails.
  pub(crate) playing: Option<f64>,
  /// An animation being written on another thread.
  exporting: Option<Exporting>,
  pub(crate) thumbnails_of_frames: ui::timeline::Thumbnails,
  /// Tab hides the interface.
  hidden: bool,
  confirm: Option<Pending>,
  /// Brush Studio is open, on this section, with this preview.
  studio: bool,
  color_tab: ui::color::Tab,
  harmony: ui::color::Harmony,
  /// The panel's color as HSV, so the hue survives gray (RGB loses it).
  color_hsv: Option<egui::ecolor::Hsva>,
  color_hex: String,
  palettes: palettes::Palettes,
  /// The palette shown in the color panel.
  palette: usize,
  /// The category the brushes panel shows; `None` follows the current brush.
  brush_category: Option<usize>,
  /// Brush previews in the library, by brush id, with the brush and theme they show.
  thumbnails: std::collections::HashMap<String, (rustcreate_core::Brush, bool, egui::TextureHandle)>,
  studio_section: ui::studio::Section,
  preview: Option<ui::studio::Preview>,
  settings: Settings,
  /// The "New canvas" dialog and the size typed in it.
  new_canvas: Option<(u32, u32)>,
  error: Option<String>,
  closing: bool,
  title: String,
  /// An adjustment dialog, previewing on the canvas.
  adjusting: Option<ui::adjust::Adjusting>,
  /// The Canvas Size or Image Size dialog.
  resize: Option<ui::resize::ResizeDialog>,
}

/// Common canvas sizes for the "New canvas" dialog, with their name tokens.
const PRESETS: [(&str, (u32, u32)); 5] = [
  ("ui-canvas-screen", (2048, 1536)),
  ("ui-canvas-hd", (1920, 1080)),
  ("ui-canvas-4k", (3840, 2160)),
  ("ui-canvas-square", (2048, 2048)),
  ("ui-canvas-a4", (2480, 3508)),
];

impl App {
  fn new(cc: &eframe::CreationContext) -> Self {
    let settings = Settings::load();
    let doc = Document::new(settings.canvas.0, settings.canvas.1);
    let render_state = cc.wgpu_render_state.clone().expect("eframe runs on wgpu (ADR 0001)");
    let gpu = render_state.adapter.get_info();
    log::line(&format!("gpu: {} ({:?}, {:?}, driver {} {})", gpu.name, gpu.backend, gpu.device_type, gpu.driver, gpu.driver_info));
    let mut canvas = Canvas::new(&render_state, &doc);
    log::line("canvas ready");
    canvas.tools.library = library::Library::load();
    canvas.tools.quickshape = settings.quickshape;
    pen::install(cc);
    ui::theme::install(&cc.egui_ctx);
    cc.egui_ctx.set_theme(theme_preference(settings.theme));
    Self {
      ctx: cc.egui_ctx.clone(),
      i18n: I18n::new(settings.language()),
      updates: Updates::start(cc.egui_ctx.clone(), settings.check_updates),
      render_state,
      saved: doc.revision(),
      doc,
      canvas,
      path: None,
      popover: None,
      renaming: None,
      editing_slider: false,
      hidden: false,
      confirm: None,
      studio: false,
      color_tab: Default::default(),
      harmony: Default::default(),
      color_hsv: None,
      color_hex: String::new(),
      palettes: palettes::Palettes::load(&I18n::new(settings.language()).tr("ui-palette-starter")),
      palette: 0,
      brush_category: None,
      thumbnails: Default::default(),
      studio_section: Default::default(),
      preview: None,
      playing: None,
      exporting: None,
      thumbnails_of_frames: Default::default(),
      settings,
      new_canvas: None,
      adjusting: None,
      resize: None,
      error: None,
      closing: false,
      title: String::new(),
    }
  }

  fn dirty(&self) -> bool {
    self.doc.revision() != self.saved
  }

  fn name(&self) -> String {
    self.path.as_ref().and_then(|p| p.file_stem()).map_or_else(|| self.i18n.tr("ui-untitled"), |n| n.to_string_lossy().into_owned())
  }

  /// Runs `action` now, or first asks to save unsaved changes.
  pub(crate) fn request(&mut self, action: Pending) {
    // An open adjustment keeps what it shows.
    self.adjusting = None;
    // A transform in progress is applied first, so it is saved or asked about like any edit.
    self.canvas.end_transform(&mut self.doc, true);
    if self.dirty() { self.confirm = Some(action) } else { self.perform(action) }
  }

  fn perform(&mut self, action: Pending) {
    match action {
      Pending::New(width, height) => self.replace(Document::new(width, height), None),
      Pending::Open(path) => {
        let Some(path) = path.or_else(|| rfd::FileDialog::new().add_filter("rustcreate", &["rcreate"]).pick_file()) else { return };
        self.open(path);
      }
      Pending::Revert => {
        if let Some(path) = self.path.clone() {
          self.open(path);
        }
      }
      Pending::Close => {
        self.closing = true;
        self.ctx.send_viewport_cmd(egui::ViewportCommand::Close);
      }
      Pending::Install(update) => self.updates.install(update),
    }
  }

  fn open(&mut self, path: PathBuf) {
    match Document::open(&path) {
      Ok(doc) if doc.width.max(doc.height) > self.max_side() => {
        let error = format!("{} × {} > {}", doc.width, doc.height, self.max_side());
        self.error = Some(self.i18n.tr_args("ui-error-open", &[("error", error.into())]));
      }
      Ok(doc) => {
        self.remember(&path);
        self.replace(doc, Some(path));
      }
      Err(error) => {
        // A recent file that is gone leaves the list.
        if !path.exists() {
          self.forget(&path);
        }
        self.error = Some(self.i18n.tr_args("ui-error-open", &[("error", error.to_string().into())]));
      }
    }
  }

  /// Puts `path` first in File > Open recent.
  fn remember(&mut self, path: &std::path::Path) {
    let mut settings = self.settings.clone();
    settings.recent.retain(|p| p != path);
    settings.recent.insert(0, path.to_path_buf());
    settings.recent.truncate(RECENT_FILES);
    self.apply(settings);
  }

  pub(crate) fn forget(&mut self, path: &std::path::Path) {
    let mut settings = self.settings.clone();
    settings.recent.retain(|p| p != path);
    self.apply(settings);
  }

  /// The largest canvas side this GPU can hold as one texture (ADR 0001).
  fn max_side(&self) -> u32 {
    self.render_state.device.limits().max_texture_dimension_2d.min(rustcreate_core::MAX_SIDE)
  }

  /// Applies changed settings right away and saves them.
  pub(crate) fn apply(&mut self, settings: Settings) {
    if settings.theme != self.settings.theme {
      self.ctx.set_theme(theme_preference(settings.theme));
    }
    if settings.language != self.settings.language {
      self.i18n = I18n::new(settings.language());
      self.title.clear();
    }
    if settings.check_updates != self.settings.check_updates {
      self.updates.set_checks(settings.check_updates);
    }
    self.canvas.tools.quickshape = settings.quickshape;
    settings.save();
    self.settings = settings;
  }

  fn replace(&mut self, doc: Document, path: Option<PathBuf>) {
    self.doc = doc;
    self.saved = self.doc.revision();
    self.path = path;
    self.renaming = None;
    self.canvas.reset(&self.render_state, &self.doc);
    // The guide belongs to the canvas: the file's, or none for a new one.
    self.canvas.tools.guides = guides::Guides::from_json(self.doc.guide.as_ref());
  }

  /// Saves to the current file, or asks where. Returns whether it saved.
  pub(crate) fn save(&mut self, choose: bool) -> bool {
    self.adjusting = None;
    self.canvas.end_transform(&mut self.doc, true);
    let path = match &self.path {
      Some(path) if !choose => path.clone(),
      _ => {
        let dialog = rfd::FileDialog::new().add_filter("rustcreate", &["rcreate"]).set_file_name(format!("{}.rcreate", self.name()));
        let Some(path) = dialog.save_file() else { return false };
        path
      }
    };
    self.doc.guide = Some(self.canvas.tools.guides.to_json());
    match self.doc.save(&path) {
      Ok(()) => {
        self.saved = self.doc.revision();
        self.remember(&path);
        self.path = Some(path);
        true
      }
      Err(error) => {
        self.error = Some(self.i18n.tr_args("ui-error-save", &[("error", error.to_string().into())]));
        false
      }
    }
  }

  /// File > Save a copy: writes the document somewhere else and keeps working on this file.
  pub(crate) fn save_copy(&mut self) {
    self.canvas.end_text(&mut self.doc);
    self.canvas.end_transform(&mut self.doc, true);
    let dialog = rfd::FileDialog::new().add_filter("rustcreate", &["rcreate"]).set_file_name(format!("{} copy.rcreate", self.name()));
    let Some(path) = dialog.save_file() else { return };
    self.doc.guide = Some(self.canvas.tools.guides.to_json());
    if let Err(error) = self.doc.save(&path) {
      self.error = Some(self.i18n.tr_args("ui-error-save", &[("error", error.to_string().into())]));
    }
  }
  /// An animation as GIF, animated PNG or a folder of PNGs, written on another thread (big
  /// canvases take a while) with its progress shown.
  fn export_animation(&mut self, format: Export) {
    self.stop_playback();
    let total = self.doc.export_frames().len();
    if total == 0 || self.exporting.is_some() {
      return;
    }
    let name = self.name();
    let target = match format {
      Export::Gif => rfd::FileDialog::new().add_filter("GIF", &["gif"]).set_file_name(format!("{name}.gif")).save_file(),
      Export::AnimatedPng => rfd::FileDialog::new().add_filter("APNG", &["png", "apng"]).set_file_name(format!("{name}.png")).save_file(),
      _ => rfd::FileDialog::new().pick_folder(),
    };
    let Some(target) = target else { return };
    let (doc, ctx) = (self.doc.clone(), self.ctx.clone());
    let done = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let outcome = std::sync::Arc::new(std::sync::Mutex::new(None));
    let (thread_done, thread_outcome) = (done.clone(), outcome.clone());
    std::thread::spawn(move || {
      let progress = |n: usize| {
        thread_done.store(n, std::sync::atomic::Ordering::Relaxed);
        ctx.request_repaint();
      };
      let result = match format {
        Export::Gif => std::fs::File::create(&target).map_err(Into::into).and_then(|file| doc.export_gif(file, &progress)),
        Export::AnimatedPng => std::fs::File::create(&target).map_err(Into::into).and_then(|file| doc.export_apng(file, &progress)),
        _ => doc.export_png_sequence(&target, &name, &progress).map(|_| ()),
      };
      if let Ok(mut slot) = thread_outcome.lock() {
        *slot = Some(result.map_err(|e| e.to_string()));
      }
      ctx.request_repaint();
    });
    self.exporting = Some(Exporting { done, total, outcome });
  }

  /// The running animation export's progress, at the bottom of the window; its error if it failed.
  fn export_progress(&mut self, ctx: &egui::Context) {
    let Some(exporting) = &self.exporting else { return };
    if let Some(result) = exporting.outcome.lock().ok().and_then(|mut slot| slot.take()) {
      self.exporting = None;
      if let Err(error) = result {
        self.error = Some(self.i18n.tr_args("ui-error-save", &[("error", error.into())]));
      }
      return;
    }
    let done = exporting.done.load(std::sync::atomic::Ordering::Relaxed);
    let text = self.i18n.tr_args("ui-exporting", &[("done", (done as i64).into()), ("total", (exporting.total as i64).into())]);
    let fraction = done as f32 / exporting.total.max(1) as f32;
    egui::Area::new(egui::Id::unique("export-progress"))
      .order(egui::Order::Tooltip)
      .anchor(egui::Align2::CENTER_BOTTOM, egui::vec2(0.0, -24.0))
      .show(ctx, |ui| {
        egui::Frame::popup(ui.style()).show(ui, |ui| {
          ui.set_width(260.0);
          ui.label(text);
          ui.add(egui::ProgressBar::new(fraction));
        });
      });
  }

  /// The flattened picture as PNG, JPEG (on white: JPEG has no transparency) or WebP (lossless).
  pub(crate) fn export(&mut self, format: Export) {
    self.canvas.end_text(&mut self.doc);
    self.canvas.end_transform(&mut self.doc, true);
    let (label, extension) = match format {
      Export::Png => ("PNG", "png"),
      Export::Jpeg => ("JPEG", "jpg"),
      Export::Webp => ("WebP", "webp"),
      animation => return self.export_animation(animation),
    };
    let dialog = rfd::FileDialog::new().add_filter(label, &[extension]).set_file_name(format!("{}.{extension}", self.name()));
    let Some(path) = dialog.save_file() else { return };
    let (width, height) = (self.doc.width, self.doc.height);
    let result = match format {
      Export::Png => self.doc.export_png(&path).map_err(|e| e.to_string()),
      Export::Jpeg => {
        let rgb: Vec<u8> = self
          .doc
          .flatten()
          .as_chunks::<4>()
          .0
          .iter()
          .flat_map(|p| {
            let a = p[3] as f32 / 255.0;
            [0, 1, 2].map(|c| (p[c] as f32 * a + 255.0 * (1.0 - a)).round() as u8)
          })
          .collect();
        std::fs::File::create(&path).map_err(|e| e.to_string()).and_then(|file| {
          let encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(std::io::BufWriter::new(file), 92);
          image::ImageEncoder::write_image(encoder, &rgb, width, height, image::ExtendedColorType::Rgb8).map_err(|e| e.to_string())
        })
      }
      Export::Gif | Export::AnimatedPng | Export::PngSequence => unreachable!("exported above"),
      Export::Webp => std::fs::File::create(&path).map_err(|e| e.to_string()).and_then(|file| {
        let encoder = image::codecs::webp::WebPEncoder::new_lossless(std::io::BufWriter::new(file));
        image::ImageEncoder::write_image(encoder, &self.doc.flatten(), width, height, image::ExtendedColorType::Rgba8).map_err(|e| e.to_string())
      }),
    };
    if let Err(error) = result {
      self.error = Some(self.i18n.tr_args("ui-error-save", &[("error", error.into())]));
    }
  }

  /// File > Place: an image (PNG, JPEG, WebP) as a new layer in the middle of the canvas,
  /// shrunk to fit if it is bigger, then a Free Transform to put it in place (Enter applies).
  pub(crate) fn place(&mut self) {
    self.canvas.end_text(&mut self.doc);
    self.canvas.end_transform(&mut self.doc, true);
    let Some(path) = rfd::FileDialog::new().add_filter(self.i18n.tr("ui-file-images"), &["png", "jpg", "jpeg", "webp"]).pick_file() else { return };
    let mut picture = match image::open(&path) {
      Ok(picture) => picture.to_rgba8(),
      Err(error) => {
        self.error = Some(self.i18n.tr_args("ui-error-place", &[("error", error.to_string().into())]));
        return;
      }
    };
    let (width, height) = (self.doc.width, self.doc.height);
    let fit = (width as f32 / picture.width() as f32).min(height as f32 / picture.height() as f32);
    if fit < 1.0 {
      let size = ((picture.width() as f32 * fit).round().max(1.0) as u32, (picture.height() as f32 * fit).round().max(1.0) as u32);
      picture = image::imageops::resize(&picture, size.0, size.1, image::imageops::FilterType::Triangle);
    }
    let at = ((width as i64 - picture.width() as i64) / 2, (height as i64 - picture.height() as i64) / 2);
    let name = path.file_stem().map_or_else(|| self.i18n.tr("ui-file-place"), |n| n.to_string_lossy().into_owned());
    self.doc.add_image_layer(&name, picture.as_raw(), picture.dimensions(), at);
    // A selection would limit the transform to part of the image.
    if self.doc.selection().is_none() {
      self.canvas.start_transform(&mut self.doc);
    }
  }

  /// File and interface shortcuts (docs/SHORTCUTS.md). The longer combinations go first.
  fn shortcuts(&mut self, ctx: &egui::Context) {
    if ctx.egui_wants_keyboard_input() || self.confirm.is_some() || self.new_canvas.is_some() || self.adjusting.is_some() || self.resize.is_some() {
      return;
    }
    let (m, shift, alt) = (egui::Modifiers::COMMAND, egui::Modifiers::SHIFT, egui::Modifiers::ALT);
    let key = |modifiers, key| ctx.input_mut(|i| i.consume_key(modifiers, key));
    if key(m | alt | shift, egui::Key::W) {
      self.export(Export::Png);
    }
    if key(m, egui::Key::W) {
      self.request(Pending::New(self.settings.canvas.0, self.settings.canvas.1));
    }
    if key(m, egui::Key::Q) {
      self.request(Pending::Close);
    }
    if key(egui::Modifiers::NONE, egui::Key::F12) && self.path.is_some() {
      self.request(Pending::Revert);
    }
    if key(m | alt, egui::Key::S) {
      self.save_copy();
    }
    // Canvas Size and Image Size (Photoshop), before Ctrl+I takes the I.
    if key(m | alt, egui::Key::C) {
      self.open_resize(false);
    }
    if key(m | alt, egui::Key::I) {
      self.open_resize(true);
    }
    // Adjustments. Ctrl+Shift+I (invert selection) is the canvas's: Ctrl+I must not take it.
    if key(m | shift, egui::Key::U) {
      self.start_adjust(rustcreate_core::Adjustment::Desaturate);
    }
    if !ctx.input(|i| i.modifiers.shift) && key(m, egui::Key::I) {
      self.start_adjust(rustcreate_core::Adjustment::Invert);
    }
    for (k, index) in [(egui::Key::U, 0), (egui::Key::L, 2), (egui::Key::M, 3)] {
      if key(m, k) {
        self.start_adjust(ui::adjust::menu()[index].1.clone());
      }
    }
    if key(m | shift, egui::Key::S) {
      self.save(true);
    }
    if key(m, egui::Key::S) {
      self.save(false);
    }
    if key(m, egui::Key::O) {
      self.request(Pending::Open(None));
    }
    if key(m, egui::Key::N) {
      self.new_canvas = Some(self.settings.canvas);
    }
    if key(egui::Modifiers::NONE, egui::Key::Tab) {
      self.hidden = !self.hidden;
      self.popover = None;
    }
  }

  fn new_canvas_dialog(&mut self, ctx: &egui::Context) {
    let Some((mut width, mut height)) = self.new_canvas else { return };
    let max = self.max_side();
    let (mut create, mut cancel) = (false, false);
    egui::Modal::new(egui::Id::unique("new-canvas")).show(ctx, |ui| {
      ui.set_width(320.0);
      ui.heading(self.i18n.tr("ui-file-new"));
      ui.add_space(6.0);
      for (name, size) in PRESETS {
        let label = format!("{}  ·  {} × {}", self.i18n.tr(name), size.0, size.1);
        if ui.selectable_label((width, height) == size, label).clicked() {
          (width, height) = size;
        }
      }
      ui.add_space(6.0);
      ui.horizontal(|ui| {
        ui.label(self.i18n.tr("ui-canvas-width"));
        ui.add(egui::DragValue::new(&mut width).range(1..=max).suffix(" px"));
        if ui.button("⇄").on_hover_text(self.i18n.tr("ui-canvas-swap")).clicked() {
          std::mem::swap(&mut width, &mut height);
        }
        ui.label(self.i18n.tr("ui-canvas-height"));
        ui.add(egui::DragValue::new(&mut height).range(1..=max).suffix(" px"));
      });
      ui.label(RichText::new(self.i18n.tr_args("ui-canvas-max", &[("max", max.into())])).small().weak());
      ui.add_space(10.0);
      ui.horizontal(|ui| {
        create = ui.button(self.i18n.tr("ui-canvas-create")).clicked() || ui.input(|i| i.key_pressed(egui::Key::Enter));
        cancel = ui.button(self.i18n.tr("ui-cancel")).clicked() || ui.input(|i| i.key_pressed(egui::Key::Escape));
      });
    });
    self.new_canvas = Some((width, height));
    if cancel {
      self.new_canvas = None;
    } else if create {
      self.new_canvas = None;
      self.apply(Settings { canvas: (width, height), ..self.settings.clone() });
      self.request(Pending::New(width, height));
    }
  }

  fn dialogs(&mut self, ctx: &egui::Context) {
    self.new_canvas_dialog(ctx);
    if self.confirm.is_some() {
      let mut answer = None;
      egui::Modal::new(egui::Id::unique("unsaved")).show(ctx, |ui| {
        ui.set_width(340.0);
        ui.heading(self.i18n.tr_args("ui-unsaved-title", &[("name", self.name().into())]));
        ui.label(self.i18n.tr("ui-unsaved-body"));
        ui.add_space(10.0);
        ui.horizontal(|ui| {
          if ui.button(self.i18n.tr("ui-unsaved-save")).clicked() {
            answer = Some(1);
          }
          if ui.button(self.i18n.tr("ui-unsaved-discard")).clicked() {
            answer = Some(2);
          }
          if ui.button(self.i18n.tr("ui-cancel")).clicked() || ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            answer = Some(0);
          }
        });
      });
      match answer {
        Some(1) => {
          if self.save(false)
            && let Some(action) = self.confirm.take()
          {
            self.perform(action);
          }
        }
        Some(2) => {
          if let Some(action) = self.confirm.take() {
            self.perform(action);
          }
        }
        Some(_) => self.confirm = None,
        None => {}
      }
    }
    if let Some(error) = self.error.clone() {
      egui::Modal::new(egui::Id::unique("error")).show(ctx, |ui| {
        ui.set_width(340.0);
        ui.label(error);
        ui.add_space(10.0);
        if ui.button(self.i18n.tr("ui-ok")).clicked() {
          self.error = None;
        }
      });
    }
  }
}

impl eframe::App for App {
  fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
    let ctx = ui.ctx().clone();
    let frame = ctx.cumulative_frame_nr();
    log::ui_started(frame);
    if frame == 0 {
      // A second frame tells the log that the first one was painted and the window shown.
      ctx.request_repaint();
    }
    // Closing the window with unsaved changes asks first.
    if ctx.input(|i| i.viewport().close_requested()) && !self.closing && self.dirty() {
      ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
      self.confirm = Some(Pending::Close);
    }
    let title = format!("{}{} — rustcreate", self.name(), if self.dirty() { " •" } else { "" });
    if title != self.title {
      ctx.send_viewport_cmd(egui::ViewportCommand::Title(title.clone()));
      self.title = title;
    }

    self.update_banner(ui);
    self.shortcuts(&ctx);
    if !self.hidden {
      self.top_bar(ui);
      self.sidebar(ui);
    }
    self.timeline(ui);
    self.export_progress(&ctx);
    // Esc closes an open panel before anything else (a transform, a selection) sees it.
    if self.popover.is_some() && !ctx.egui_wants_keyboard_input() && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
      self.popover = None;
    }
    let keys = !ctx.egui_wants_keyboard_input() && self.confirm.is_none() && self.adjusting.is_none() && self.resize.is_none();
    let render_state = self.render_state.clone();
    egui::CentralPanel::default().frame(egui::Frame::NONE.fill(ui.visuals().extreme_bg_color)).show(ui, |ui| {
      self.canvas.show(ui, &render_state, &mut self.doc, keys, &self.i18n);
    });
    if !self.hidden {
      self.popovers(&ctx);
      self.brush_studio(&ctx);
    }
    self.adjust_dialog(&ctx);
    self.resize_dialog(&ctx);
    self.dialogs(&ctx);
    // Not mid-drag, so a size slider being dragged saves once at the end.
    if !ctx.input(|i| i.pointer.any_down()) {
      self.canvas.tools.library.save_if_changed();
      self.palettes.save_if_changed();
    }
    log::ui_finished();
  }
}

impl App {
  /// The non-modal "new version" banner (ADR 0002).
  fn update_banner(&mut self, ui: &mut egui::Ui) {
    let status = self.updates.status();
    if matches!(status, Status::Idle) {
      return;
    }
    egui::Panel::top("update-banner").show(ui, |ui| {
      ui.horizontal(|ui| match status {
        Status::Idle => {}
        Status::Available(update) => {
          ui.label(self.i18n.tr_args("ui-update-available", &[("version", update.version.to_string().into())]));
          if ui.button(self.i18n.tr("ui-update-install")).clicked() {
            self.request(Pending::Install(update));
          }
          if ui.button(self.i18n.tr("ui-update-later")).clicked() {
            self.updates.dismiss();
          }
        }
        Status::Downloading(progress) => {
          ui.label(self.i18n.tr_args("ui-update-downloading", &[("percent", ((progress * 100.0).round() as i64).into())]));
        }
        Status::Failed(error) => {
          ui.label(self.i18n.tr_args("ui-update-failed", &[("error", error.into())]));
          if ui.button(self.i18n.tr("ui-update-later")).clicked() {
            self.updates.dismiss();
          }
        }
        Status::Quit => ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close),
      });
    });
  }
}

fn theme_preference(theme: Theme) -> egui::ThemePreference {
  match theme {
    Theme::System => egui::ThemePreference::System,
    Theme::Dark => egui::ThemePreference::Dark,
    Theme::Light => egui::ThemePreference::Light,
  }
}
