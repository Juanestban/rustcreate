//! Crop to selection, and the Canvas Size and Image Size dialogs.

use eframe::egui;

use crate::App;

pub(crate) struct ResizeDialog {
  /// Image Size (resample) rather than Canvas Size (add or cut space).
  image: bool,
  width: u32,
  height: u32,
  /// Canvas Size: which of the 3 × 3 points of the old canvas stays put.
  anchor: (usize, usize),
  /// Image Size: keep the proportions.
  keep: bool,
}

impl App {
  /// Opens Canvas Size (`image: false`) or Image Size.
  pub(crate) fn open_resize(&mut self, image: bool) {
    self.settle();
    self.resize = Some(ResizeDialog { image, width: self.doc.width, height: self.doc.height, anchor: (1, 1), keep: true });
  }

  pub(crate) fn crop(&mut self) {
    self.settle();
    self.doc.crop_to_selection();
  }

  /// A transform or adjustment in progress is kept before the canvas changes size.
  fn settle(&mut self) {
    self.canvas.end_transform(&mut self.doc, true);
    self.adjusting = None;
    self.popover = None;
  }

  pub(crate) fn resize_dialog(&mut self, ctx: &egui::Context) {
    let max = self.max_side();
    let (old_w, old_h) = (self.doc.width, self.doc.height);
    let Some(dialog) = &mut self.resize else { return };
    let i18n = &self.i18n;
    let mut done = None;
    egui::Modal::new(egui::Id::unique("resize")).show(ctx, |ui| {
      ui.set_width(300.0);
      ui.heading(i18n.tr(if dialog.image { "ui-image-size" } else { "ui-canvas-size" }));
      ui.label(egui::RichText::new(format!("{} × {} px", old_w, old_h)).weak());
      ui.add_space(6.0);
      let (w0, h0) = (dialog.width, dialog.height);
      ui.horizontal(|ui| {
        ui.label(i18n.tr("ui-canvas-width"));
        ui.add(egui::DragValue::new(&mut dialog.width).range(1..=max).suffix(" px"));
        ui.label(i18n.tr("ui-canvas-height"));
        ui.add(egui::DragValue::new(&mut dialog.height).range(1..=max).suffix(" px"));
      });
      if dialog.image {
        ui.checkbox(&mut dialog.keep, i18n.tr("ui-resize-keep"));
        // Proportional: the side that was not edited follows the one that was.
        let scale = |v: u32, from: u32, to: u32| ((v as f64 * to as f64 / from as f64).round() as u32).clamp(1, max);
        if dialog.keep && dialog.width != w0 {
          dialog.height = scale(dialog.width, old_w, old_h);
        } else if dialog.keep && dialog.height != h0 {
          dialog.width = scale(dialog.height, old_h, old_w);
        }
      } else {
        ui.label(i18n.tr("ui-resize-anchor"));
        egui::Grid::new("anchor").spacing([2.0, 2.0]).show(ui, |ui| {
          for row in 0..3 {
            for col in 0..3 {
              let on = dialog.anchor == (col, row);
              if ui.add_sized([28.0, 28.0], egui::Button::selectable(on, if on { "●" } else { "·" })).clicked() {
                dialog.anchor = (col, row);
              }
            }
            ui.end_row();
          }
        });
      }
      ui.add_space(8.0);
      ui.horizontal(|ui| {
        if ui.button(i18n.tr("ui-resize-apply")).clicked() || ui.input(|i| i.key_pressed(egui::Key::Enter)) {
          done = Some(true);
        }
        if ui.button(i18n.tr("ui-cancel")).clicked() || ui.input(|i| i.key_pressed(egui::Key::Escape)) {
          done = Some(false);
        }
      });
    });
    if done == Some(true) {
      let (w, h) = (dialog.width, dialog.height);
      if dialog.image {
        self.doc.resize_image(w, h);
      } else {
        let anchor = (dialog.anchor.0 as f32 / 2.0, dialog.anchor.1 as f32 / 2.0);
        self.doc.resize_canvas(w, h, anchor);
      }
    }
    if done.is_some() {
      self.resize = None;
    }
  }
}
