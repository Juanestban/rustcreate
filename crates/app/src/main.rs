#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod canvas;
mod i18n;
mod tools;
mod update;
mod updates;

use canvas::Canvas;
use eframe::egui;
use i18n::{I18n, Language};
use rustcreate_core::Document;
use updates::{Status, Updates};

fn main() -> eframe::Result {
  update::clean_up();
  let icon = eframe::icon_data::from_png_bytes(include_bytes!("../assets/icon-256.png")).expect("bundled icon is a PNG");
  let options = eframe::NativeOptions {
    viewport: egui::ViewportBuilder::default().with_title("rustcreate").with_icon(icon).with_inner_size([1280.0, 800.0]),
    ..Default::default()
  };
  eframe::run_native("rustcreate", options, Box::new(|cc| Ok(Box::new(App::new(cc)))))
}

struct App {
  i18n: I18n,
  updates: Updates,
  doc: Document,
  canvas: Canvas,
}

impl App {
  fn new(cc: &eframe::CreationContext) -> Self {
    // shortcut: a fixed blank canvas until the gallery and "New canvas" exist.
    let doc = Document::new(2048, 1536);
    let render_state = cc.wgpu_render_state.as_ref().expect("eframe runs on wgpu (ADR 0001)");
    let canvas = Canvas::new(render_state, &doc);
    Self { i18n: I18n::new(Language::system()), updates: Updates::start(cc.egui_ctx.clone()), doc, canvas }
  }
}

impl eframe::App for App {
  fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
    self.update_banner(ui);
    let render_state = frame.wgpu_render_state().expect("eframe runs on wgpu (ADR 0001)");
    egui::CentralPanel::default().frame(egui::Frame::NONE.fill(ui.visuals().extreme_bg_color)).show(ui, |ui| {
      self.canvas.show(ui, render_state, &mut self.doc, &self.i18n);
    });
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
          // shortcut: no documents yet, so nothing to save first; ask once documents exist (ADR 0002).
          if ui.button(self.i18n.tr("ui-update-install")).clicked() {
            self.updates.install(update);
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
