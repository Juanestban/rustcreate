//! Several documents open at once. The shown one lives in `App`'s own fields (`doc`, `path`,
//! `saved`); the others wait in `App::tabs`, whose slot for the shown one is empty.

use std::path::PathBuf;

use rustcreate_core::Document;

use crate::App;
use crate::canvas::View;

/// A document in a tab that is not shown.
pub struct Tab {
  pub doc: Document,
  pub path: Option<PathBuf>,
  pub saved: u64,
  pub view: Option<View>,
}

impl Tab {
  fn dirty(&self) -> bool {
    self.doc.revision() != self.saved
  }
}

impl App {
  /// The shown document, packed into a tab (`self.doc` is left a placeholder).
  fn take_current(&mut self) -> Tab {
    self.canvas.end_text(&mut self.doc);
    self.canvas.end_transform(&mut self.doc, true);
    self.stop_playback();
    self.doc.guide = Some(self.canvas.tools.guides.to_json());
    let doc = std::mem::replace(&mut self.doc, Document::new(1, 1));
    Tab { doc, path: self.path.take(), saved: self.saved, view: self.canvas.view() }
  }

  fn show_tab(&mut self, tab: Tab) {
    self.replace(tab.doc, tab.path);
    self.saved = tab.saved;
    self.canvas.set_view(tab.view);
    // Frame thumbnails are keyed by layer id, which other documents reuse.
    self.thumbnails_of_frames.clear();
    self.adjusting = None;
    self.resize = None;
  }

  pub(crate) fn switch_tab(&mut self, index: usize) {
    if index == self.tab || index >= self.tabs.len() {
      return;
    }
    let current = self.take_current();
    self.tabs[self.tab] = Some(current);
    let tab = self.tabs[index].take().expect("a hidden tab");
    self.tab = index;
    self.show_tab(tab);
  }

  /// Shows `doc` in a new tab after the current one; an untouched untitled canvas is replaced.
  pub(crate) fn open_tab(&mut self, doc: Document, path: Option<PathBuf>) {
    let blank = self.path.is_none() && !self.dirty() && !self.doc.history().can_undo();
    if !blank {
      let current = self.take_current();
      self.tabs[self.tab] = Some(current);
      self.tabs.insert(self.tab + 1, None);
      self.tab += 1;
    }
    self.replace(doc, path);
    self.thumbnails_of_frames.clear();
  }

  /// Closes the shown tab (after asking to save) for its neighbor; the last one goes home.
  pub(crate) fn close_tab(&mut self) {
    if self.tabs.len() < 2 {
      self.replace(Document::new(self.settings.canvas.0, self.settings.canvas.1), None);
      self.open_home(false);
      return;
    }
    let next = if self.tab + 1 < self.tabs.len() { self.tab + 1 } else { self.tab - 1 };
    let tab = self.tabs[next].take().expect("a hidden tab");
    self.tabs.remove(self.tab);
    self.tab = if next > self.tab { next - 1 } else { next };
    self.show_tab(tab);
  }

  /// A hidden tab with unsaved changes.
  pub(crate) fn dirty_tab(&self) -> Option<usize> {
    self.tabs.iter().position(|t| t.as_ref().is_some_and(Tab::dirty))
  }

  /// The tab already showing `path`.
  pub(crate) fn tab_of(&self, path: &std::path::Path) -> Option<usize> {
    if self.path.as_deref() == Some(path) {
      return Some(self.tab);
    }
    self.tabs.iter().position(|t| t.as_ref().is_some_and(|t| t.path.as_deref() == Some(path)))
  }

  /// File > Save all: every document with unsaved changes (an untitled one asks where), then
  /// back to the tab that was shown. Stops at the first one not saved.
  pub(crate) fn save_all(&mut self) {
    let start = self.tab;
    for index in 0..self.tabs.len() {
      self.switch_tab(index);
      if self.dirty() && !self.save(false) {
        return;
      }
    }
    self.switch_tab(start);
  }

  /// Each tab's name and whether it has unsaved changes, in order.
  pub(crate) fn tab_names(&self) -> Vec<(String, bool)> {
    let untitled = self.i18n.tr("ui-untitled");
    let name =
      |path: &Option<PathBuf>| path.as_ref().and_then(|p| p.file_stem()).map_or_else(|| untitled.clone(), |n| n.to_string_lossy().into_owned());
    self.tabs.iter().map(|t| t.as_ref().map_or_else(|| (name(&self.path), self.dirty()), |t| (name(&t.path), t.dirty()))).collect()
  }
}
