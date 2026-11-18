#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod canvas;
mod guides;
mod i18n;
mod library;
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
  update::clean_up();
  let icon = eframe::icon_data::from_png_bytes(include_bytes!("../assets/icon-256.png")).expect("bundled icon is a PNG");
  let options = eframe::NativeOptions {
    viewport: egui::ViewportBuilder::default().with_title("rustcreate").with_icon(icon).with_inner_size([1280.0, 800.0]),
    ..Default::default()
  };
  let result = eframe::run_native("rustcreate", options, Box::new(|cc| Ok(Box::new(App::new(cc)))));
  if let Err(error) = &result {
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
pub(crate) enum Pending {
  New(u32, u32),
  Open,
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
    let mut canvas = Canvas::new(&render_state, &doc);
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
      settings,
      new_canvas: None,
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
    if self.dirty() { self.confirm = Some(action) } else { self.perform(action) }
  }

  fn perform(&mut self, action: Pending) {
    match action {
      Pending::New(width, height) => self.replace(Document::new(width, height), None),
      Pending::Open => {
        let Some(path) = rfd::FileDialog::new().add_filter("rustcreate", &["rcreate"]).pick_file() else { return };
        match Document::open(&path) {
          Ok(doc) if doc.width.max(doc.height) > self.max_side() => {
            let error = format!("{} × {} > {}", doc.width, doc.height, self.max_side());
            self.error = Some(self.i18n.tr_args("ui-error-open", &[("error", error.into())]));
          }
          Ok(doc) => self.replace(doc, Some(path)),
          Err(error) => self.error = Some(self.i18n.tr_args("ui-error-open", &[("error", error.to_string().into())])),
        }
      }
      Pending::Close => {
        self.closing = true;
        self.ctx.send_viewport_cmd(egui::ViewportCommand::Close);
      }
      Pending::Install(update) => self.updates.install(update),
    }
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
  }

  /// Saves to the current file, or asks where. Returns whether it saved.
  pub(crate) fn save(&mut self, choose: bool) -> bool {
    let path = match &self.path {
      Some(path) if !choose => path.clone(),
      _ => {
        let dialog = rfd::FileDialog::new().add_filter("rustcreate", &["rcreate"]).set_file_name(format!("{}.rcreate", self.name()));
        let Some(path) = dialog.save_file() else { return false };
        path
      }
    };
    match self.doc.save(&path) {
      Ok(()) => {
        self.saved = self.doc.revision();
        self.path = Some(path);
        true
      }
      Err(error) => {
        self.error = Some(self.i18n.tr_args("ui-error-save", &[("error", error.to_string().into())]));
        false
      }
    }
  }

  pub(crate) fn export_png(&mut self) {
    let dialog = rfd::FileDialog::new().add_filter("PNG", &["png"]).set_file_name(format!("{}.png", self.name()));
    if let Some(path) = dialog.save_file()
      && let Err(error) = self.doc.export_png(&path)
    {
      self.error = Some(self.i18n.tr_args("ui-error-save", &[("error", error.to_string().into())]));
    }
  }

  /// File and interface shortcuts (docs/SHORTCUTS.md). The longer combinations go first.
  fn shortcuts(&mut self, ctx: &egui::Context) {
    if ctx.egui_wants_keyboard_input() || self.confirm.is_some() || self.new_canvas.is_some() {
      return;
    }
    let (m, shift, alt) = (egui::Modifiers::COMMAND, egui::Modifiers::SHIFT, egui::Modifiers::ALT);
    let key = |modifiers, key| ctx.input_mut(|i| i.consume_key(modifiers, key));
    if key(m | alt | shift, egui::Key::W) {
      self.export_png();
    }
    if key(m | shift, egui::Key::S) {
      self.save(true);
    }
    if key(m, egui::Key::S) {
      self.save(false);
    }
    if key(m, egui::Key::O) {
      self.request(Pending::Open);
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
    let keys = !ctx.egui_wants_keyboard_input() && self.confirm.is_none();
    let render_state = self.render_state.clone();
    egui::CentralPanel::default().frame(egui::Frame::NONE.fill(ui.visuals().extreme_bg_color)).show(ui, |ui| {
      self.canvas.show(ui, &render_state, &mut self.doc, keys);
    });
    if !self.hidden {
      self.popovers(&ctx);
      self.brush_studio(&ctx);
    }
    self.dialogs(&ctx);
    // Not mid-drag, so a size slider being dragged saves once at the end.
    if !ctx.input(|i| i.pointer.any_down()) {
      self.canvas.tools.library.save_if_changed();
      self.palettes.save_if_changed();
    }
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
