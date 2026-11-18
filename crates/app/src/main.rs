#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod i18n;
mod update;
mod updates;

use eframe::egui;
use i18n::{I18n, Language};
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
}

impl App {
  fn new(cc: &eframe::CreationContext) -> Self {
    Self { i18n: I18n::new(Language::system()), updates: Updates::start(cc.egui_ctx.clone()) }
  }
}

impl eframe::App for App {
  fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
    self.update_banner(ui);
    egui::CentralPanel::default().show(ui, |ui| {
      ui.centered_and_justified(|ui| ui.weak(self.i18n.tr("ui-canvas-empty")));
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
