//! Translations: one Fluent bundle per language. A message missing in the chosen language falls
//! back to English, then to its id.

use fluent_bundle::{FluentArgs, FluentBundle, FluentResource, FluentValue};
use unic_langid::LanguageIdentifier;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Language {
  Es,
  En,
}

impl Language {
  pub fn system() -> Self {
    match sys_locale::get_locale() {
      Some(locale) if locale.to_ascii_lowercase().starts_with("es") => Self::Es,
      _ => Self::En,
    }
  }

  fn code(self) -> &'static str {
    match self {
      Self::Es => "es",
      Self::En => "en",
    }
  }

  fn source(self) -> &'static str {
    match self {
      Self::Es => include_str!("../locales/es.ftl"),
      Self::En => include_str!("../locales/en.ftl"),
    }
  }
}

pub struct I18n {
  current: FluentBundle<FluentResource>,
  english: FluentBundle<FluentResource>,
}

impl I18n {
  pub fn new(language: Language) -> Self {
    Self { current: bundle(language), english: bundle(Language::En) }
  }

  pub fn tr(&self, id: &str) -> String {
    self.format(id, None)
  }

  /// `args` are `(name, value)` pairs for the message's `{ $name }`s.
  pub fn tr_args<'a>(&self, id: &str, args: &[(&'a str, FluentValue<'a>)]) -> String {
    let mut fluent_args = FluentArgs::new();
    for (name, value) in args {
      fluent_args.set(*name, value.clone());
    }
    self.format(id, Some(&fluent_args))
  }

  fn format(&self, id: &str, args: Option<&FluentArgs>) -> String {
    for bundle in [&self.current, &self.english] {
      if let Some(pattern) = bundle.get_message(id).and_then(|message| message.value()) {
        let mut errors = Vec::new();
        return bundle.format_pattern(pattern, args, &mut errors).into_owned();
      }
    }
    id.to_owned()
  }
}

fn bundle(language: Language) -> FluentBundle<FluentResource> {
  let locale: LanguageIdentifier = language.code().parse().expect("valid language code");
  let mut bundle = FluentBundle::new(vec![locale]);
  // Unicode isolation marks around arguments render as boxes in egui.
  bundle.set_use_isolating(false);
  // The files are bundled and tested, so a parse error is a bug.
  let resource = FluentResource::try_new(language.source().to_owned()).expect("bundled Fluent file parses");
  bundle.add_resource(resource).expect("no duplicate Fluent ids");
  bundle
}

#[cfg(test)]
mod tests {
  use std::collections::BTreeSet;
  use std::fs;
  use std::path::Path;

  use super::*;

  fn ids(source: &str) -> BTreeSet<String> {
    source
      .lines()
      .filter(|line| line.starts_with(|c: char| c.is_ascii_alphabetic()))
      .filter_map(|line| line.split_once('=').map(|(id, _)| id.trim().to_owned()))
      .collect()
  }

  /// Every `"ui-…"` literal in the app's source.
  fn used_ids() -> BTreeSet<String> {
    let mut used = BTreeSet::new();
    let mut dirs = vec![Path::new(env!("CARGO_MANIFEST_DIR")).join("src")];
    while let Some(dir) = dirs.pop() {
      for entry in fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
          dirs.push(path);
        } else if path.extension().is_some_and(|ext| ext == "rs") {
          let source = fs::read_to_string(&path).unwrap();
          for (start, _) in source.match_indices("\"ui-") {
            let id: String = source[start + 1..].chars().take_while(|&c| c.is_ascii_alphanumeric() || c == '-' || c == '_').collect();
            if !id.ends_with('-') {
              used.insert(id);
            }
          }
        }
      }
    }
    used
  }

  #[test]
  fn ui_strings_exist_in_both_languages() {
    let es = ids(include_str!("../locales/es.ftl"));
    let en = ids(include_str!("../locales/en.ftl"));
    assert_eq!(es, en, "es.ftl and en.ftl must define the same ids");
    let missing: Vec<_> = used_ids().into_iter().filter(|id| !es.contains(id)).collect();
    assert!(missing.is_empty(), "used but not translated: {missing:?}");
  }

  #[test]
  fn formats_with_arguments_and_fallback() {
    let es = I18n::new(Language::Es);
    assert_eq!(es.tr_args("ui-update-available", &[("version", "1.2.3".into())]), "Versión 1.2.3 disponible");
    assert_eq!(es.tr("no-such-id"), "no-such-id");
    assert_eq!(I18n::new(Language::En).tr("ui-update-later"), "Later");
  }
}
