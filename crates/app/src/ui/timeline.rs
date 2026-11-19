//! Animation Assist's timeline, under the canvas: the frames (top-level layers and groups) as
//! thumbnails, play/pause, speed and playback mode.

use std::collections::HashMap;

use eframe::egui::{self, Align, Frame, Layout, Margin, RichText, Sense, Stroke, Vec2, vec2};
use rustcreate_core::Playback;

use super::icons::{self, Icon};
use super::theme;
use crate::App;

pub const TIMELINE_HEIGHT: f32 = 118.0;
const THUMB: Vec2 = Vec2::new(80.0, 60.0);

/// Thumbnail textures by frame layer id: the document revision and time they were drawn at.
pub type Thumbnails = HashMap<u64, (u64, f64, egui::TextureHandle)>;

enum FrameAction {
  Duplicate(usize),
  Delete(usize),
}

impl App {
  pub(crate) fn stop_playback(&mut self) {
    self.playing = None;
    self.doc.set_playing(None);
  }

  /// Moves playback on to the frame due now, and asks for the next repaint in time.
  fn advance_playback(&mut self, ctx: &egui::Context) {
    let Some(start) = self.playing else { return };
    let frames = self.doc.frames();
    let animation = self.doc.animation;
    let step = ((ctx.input(|i| i.time) - start) * animation.fps as f64) as u64;
    let (at, done) = animation.frame_at(step, frames.len());
    self.doc.set_playing(Some(frames[at]));
    if done {
      // One shot ends on the last frame, which stays selected.
      self.stop_playback();
      self.doc.select(frames[at]);
    } else {
      ctx.request_repaint_after(std::time::Duration::from_secs_f64(1.0 / animation.fps as f64));
    }
  }

  pub(crate) fn timeline(&mut self, ui: &mut egui::Ui) {
    if !self.doc.animation.enabled {
      return;
    }
    let ctx = ui.ctx().clone();
    // A press on the canvas paints the active frame: stop playing first, so it shows.
    let screen = ctx.content_rect();
    let canvas = egui::Rect::from_min_max(screen.min + vec2(theme::SIDEBAR_WIDTH, theme::BAR_HEIGHT), screen.max - vec2(0.0, TIMELINE_HEIGHT));
    if self.playing.is_some() && ctx.input(|i| i.pointer.primary_pressed() && i.pointer.interact_pos().is_some_and(|p| canvas.contains(p))) {
      self.stop_playback();
    }
    self.advance_playback(&ctx);
    let palette = theme::palette(ui);
    let mut action = None;
    egui::Panel::bottom("timeline").exact_size(TIMELINE_HEIGHT).frame(Frame::NONE.fill(palette.chrome).inner_margin(Margin::symmetric(12, 8))).show(
      ui,
      |ui| {
        ui.horizontal(|ui| {
          // Controls: play/pause, speed and how it plays.
          ui.vertical(|ui| {
            ui.set_width(120.0);
            let playing = self.playing.is_some();
            let tip = self.i18n.tr(if playing { "ui-animation-pause" } else { "ui-animation-play" });
            if icons::button(ui, if playing { Icon::Pause } else { Icon::Play }, playing, true, &tip).clicked() {
              if playing {
                self.stop_playback();
              } else {
                self.canvas.end_text(&mut self.doc);
                self.canvas.end_transform(&mut self.doc, true);
                self.playing = Some(ctx.input(|i| i.time));
              }
            }
            let animation = &mut self.doc.animation;
            ui.add(egui::DragValue::new(&mut animation.fps).range(1..=60).suffix(" fps"));
            let modes =
              [(Playback::Loop, "ui-animation-loop"), (Playback::PingPong, "ui-animation-ping-pong"), (Playback::OneShot, "ui-animation-one-shot")];
            let current = modes.iter().find(|(m, _)| *m == animation.playback).map_or("ui-animation-loop", |(_, id)| id);
            egui::ComboBox::from_id_salt("playback").selected_text(self.i18n.tr(current)).width(110.0).show_ui(ui, |ui| {
              for (mode, id) in modes {
                ui.selectable_value(&mut animation.playback, mode, self.i18n.tr(id));
              }
            });
          });
          ui.separator();
          let shown = self.doc.shown_frame();
          egui::ScrollArea::horizontal().auto_shrink([false, true]).show(ui, |ui| {
            ui.horizontal(|ui| {
              ui.spacing_mut().item_spacing.x = 8.0;
              let frames = self.doc.frames();
              let mut refreshed = false;
              for (n, &index) in frames.iter().enumerate() {
                let texture = self.frame_thumbnail(&ctx, index, &mut refreshed);
                let (rect, response) = ui.allocate_exact_size(THUMB + vec2(0.0, 20.0), Sense::click());
                let picture = egui::Rect::from_min_size(rect.min, THUMB);
                let active = index == shown;
                ui.painter().rect_filled(picture, 6, egui::Color32::from_gray(235));
                let size = texture.size_vec2();
                let fit = (THUMB.x / size.x).min(THUMB.y / size.y);
                let image = egui::Rect::from_center_size(picture.center(), size * fit);
                ui.painter().image(texture.id(), image, egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), egui::Color32::WHITE);
                let edge = if active {
                  Stroke::new(3.0, theme::ACCENT)
                } else if response.hovered() {
                  Stroke::new(1.5, palette.ink)
                } else {
                  Stroke::new(1.0, palette.line)
                };
                ui.painter().rect_stroke(picture, 6, edge, egui::StrokeKind::Outside);
                let color = if active { theme::ACCENT } else { palette.muted };
                ui.painter().text(
                  egui::pos2(rect.center().x, rect.bottom() - 9.0),
                  egui::Align2::CENTER_CENTER,
                  format!("{}", n + 1),
                  egui::FontId::proportional(13.0),
                  color,
                );
                let response = response.on_hover_text(self.doc.layers()[index].name.clone());
                if response.clicked() {
                  self.stop_playback();
                  self.doc.select(index);
                }
                response.context_menu(|ui| {
                  if ui.button(self.i18n.tr("ui-layer-duplicate")).clicked() {
                    action = Some(FrameAction::Duplicate(index));
                    ui.close();
                  }
                  if ui.add_enabled(frames.len() > 1, egui::Button::new(self.i18n.tr("ui-layer-delete"))).clicked() {
                    action = Some(FrameAction::Delete(index));
                    ui.close();
                  }
                });
              }
              // A new frame after the selected one.
              ui.with_layout(Layout::top_down(Align::Center), |ui| {
                ui.add_space(14.0);
                if icons::button(ui, Icon::Plus, false, true, &self.i18n.tr("ui-animation-add-frame")).clicked() {
                  self.stop_playback();
                  self.doc.add_frame();
                }
                ui.label(RichText::new(self.i18n.tr("ui-animation-add-frame")).small().color(palette.muted));
              });
            });
          });
        });
      },
    );
    match action {
      Some(FrameAction::Duplicate(i)) => {
        self.doc.duplicate_layer(i);
      }
      Some(FrameAction::Delete(i)) => {
        self.doc.delete_layer(i);
      }
      None => {}
    }
  }

  /// Frame `index`'s thumbnail, drawn again when the document changed. Only one stale frame is
  /// redrawn per UI pass besides the shown one, so long animations do not stall a frame.
  fn frame_thumbnail(&mut self, ctx: &egui::Context, index: usize, refreshed: &mut bool) -> egui::TextureHandle {
    let id = self.doc.layers()[index].id;
    let (revision, now) = (self.doc.revision(), ctx.input(|i| i.time));
    let shown = index == self.doc.shown_frame() && self.playing.is_none();
    let stale = match self.thumbnails_of_frames.get(&id) {
      None => true,
      // The shown frame is painted on without new revisions mid-stroke: refresh it now and then.
      Some((rev, at, _)) => (*rev != revision && (shown || !*refreshed)) || (shown && now - at > 0.5),
    };
    if stale {
      if !shown {
        *refreshed = true;
      }
      let (w, h, pixels) = self.doc.frame_thumbnail(index, 160);
      let image = egui::ColorImage::from_rgba_unmultiplied([w as usize, h as usize], &pixels);
      let texture = ctx.load_texture(format!("frame-{id}"), image, egui::TextureOptions::LINEAR);
      self.thumbnails_of_frames.insert(id, (revision, now, texture));
      if shown {
        ctx.request_repaint_after(std::time::Duration::from_millis(500));
      }
    }
    let live: Vec<u64> = self.doc.frames().iter().map(|&i| self.doc.layers()[i].id).collect();
    self.thumbnails_of_frames.retain(|id, _| live.contains(id));
    self.thumbnails_of_frames[&id].2.clone()
  }
}
