//! The settings window (Edit > Preferences, Ctrl+K, or the button on the home screen): the app's
//! preferences, and About with the version, the authors, the repository and the update check.

use eframe::egui::{self, RichText};

use super::theme;
use crate::App;
use crate::i18n::Language;
use crate::settings::Theme;
use crate::updates::Status;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Page {
  General,
  About,
}

impl App {
  pub(crate) fn preferences_window(&mut self, ctx: &egui::Context) {
    let Some(mut page) = self.preferences else { return };
    let mut open = true;
    egui::Window::new(self.i18n.tr("ui-preferences"))
      .id(egui::Id::unique("preferences"))
      .open(&mut open)
      .collapsible(false)
      .resizable(false)
      .pivot(egui::Align2::CENTER_CENTER)
      .default_pos(ctx.content_rect().center())
      .show(ctx, |ui| {
        ui.set_width(400.0);
        ui.horizontal(|ui| {
          ui.selectable_value(&mut page, Page::General, self.i18n.tr("ui-preferences-general"));
          ui.selectable_value(&mut page, Page::About, self.i18n.tr("ui-about"));
        });
        ui.separator();
        match page {
          Page::General => self.general_settings(ui),
          Page::About => self.about(ui),
        }
      });
    self.preferences = open.then_some(page);
  }

  /// Applied and saved as soon as they change.
  fn general_settings(&mut self, ui: &mut egui::Ui) {
    let mut settings = self.settings.clone();
    let muted = theme::palette(ui).muted;
    ui.label(RichText::new(self.i18n.tr("ui-theme")).color(muted));
    ui.horizontal(|ui| {
      ui.selectable_value(&mut settings.theme, Theme::System, self.i18n.tr("ui-theme-system"));
      ui.selectable_value(&mut settings.theme, Theme::Dark, self.i18n.tr("ui-theme-dark"));
      ui.selectable_value(&mut settings.theme, Theme::Light, self.i18n.tr("ui-theme-light"));
    });
    ui.add_space(6.0);
    ui.label(RichText::new(self.i18n.tr("ui-language")).color(muted));
    ui.horizontal(|ui| {
      ui.selectable_value(&mut settings.language, None, self.i18n.tr("ui-theme-system"));
      // Language names are written in their own language, not translated.
      ui.selectable_value(&mut settings.language, Some(Language::En), "English");
      ui.selectable_value(&mut settings.language, Some(Language::Es), "Español");
    });
    ui.add_space(6.0);
    ui.checkbox(&mut settings.check_updates, self.i18n.tr("ui-check-updates"));
    ui.checkbox(&mut settings.quickshape, self.i18n.tr("ui-quickshape")).on_hover_text(self.i18n.tr("ui-quickshape-help"));
    if cfg!(windows) {
      ui.checkbox(&mut settings.wintab, self.i18n.tr("ui-wintab")).on_hover_text(self.i18n.tr("ui-wintab-help"));
    }
    if settings != self.settings {
      self.apply(settings);
    }
  }

  fn about(&mut self, ui: &mut egui::Ui) {
    let muted = theme::palette(ui).muted;
    ui.heading(RichText::new("rustcreate").strong());
    ui.label(self.i18n.tr_args("ui-about-version", &[("version", env!("CARGO_PKG_VERSION").into())]));
    ui.label(RichText::new(self.i18n.tr("ui-about-description")).color(muted));
    ui.add_space(6.0);
    ui.label(self.i18n.tr_args("ui-about-authors", &[("authors", env!("CARGO_PKG_AUTHORS").replace(':', ", ").into())]));
    ui.label(self.i18n.tr("ui-about-license"));
    ui.hyperlink_to(self.i18n.tr("ui-about-repository"), env!("CARGO_PKG_REPOSITORY"));
    ui.separator();
    let status = self.updates.status();
    match &status {
      Status::Checking => {
        ui.horizontal(|ui| {
          ui.spinner();
          ui.label(self.i18n.tr("ui-update-checking"));
        });
      }
      Status::UpToDate => {
        ui.label(self.i18n.tr("ui-update-up-to-date"));
      }
      Status::Available(update) => {
        ui.horizontal(|ui| {
          ui.label(self.i18n.tr_args("ui-update-available", &[("version", update.version.to_string().into())]));
          if ui.button(self.i18n.tr("ui-update-install")).clicked() {
            self.request(crate::Pending::Install(update.clone()));
          }
        });
      }
      Status::Downloading(progress) => {
        ui.label(self.i18n.tr_args("ui-update-downloading", &[("percent", ((progress * 100.0).round() as i64).into())]));
      }
      Status::Failed(error) => {
        ui.label(self.i18n.tr_args("ui-update-failed", &[("error", error.clone().into())]));
      }
      Status::Idle | Status::Quit => {}
    }
    let busy = matches!(status, Status::Checking | Status::Downloading(_));
    if ui.add_enabled(!busy, egui::Button::new(self.i18n.tr("ui-update-check-now"))).clicked() {
      self.updates.check_now();
    }
  }
}
