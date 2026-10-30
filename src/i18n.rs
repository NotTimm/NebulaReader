//! Interface translations, in [Fluent](https://projectfluent.org/) like
//! libcosmic and the COSMIC apps.
//!
//! Every language is a folder in `i18n/` holding `nebula_reader.ftl`,
//! embedded into the binary at build time. English (`i18n/en`) is the
//! reference and the fallback: a message missing from another language
//! shows in English. `fl!("message-id")` (or `fl!("id", name = value)` for
//! messages with variables) looks a message up in the chosen language; a
//! message id that isn't in the English file doesn't compile.
//!
//! Adding a language needs no code: a new folder shows up in Settings by
//! itself, named by its `language-name` message. See `i18n/README.md`.

use i18n_embed::fluent::{fluent_language_loader, FluentLanguageLoader};
use i18n_embed::unic_langid::LanguageIdentifier;
use i18n_embed::{DesktopLanguageRequester, LanguageLoader};
use rust_embed::RustEmbed;
use std::sync::LazyLock;

#[derive(RustEmbed)]
#[folder = "i18n/"]
struct Localizations;

pub static LANGUAGE_LOADER: LazyLock<FluentLanguageLoader> = LazyLock::new(|| {
    let loader: FluentLanguageLoader = fluent_language_loader!();
    loader
        .load_fallback_language(&Localizations)
        .expect("the English translation is embedded");
    // Fluent wraps every variable in invisible direction marks for
    // right-to-left text; iced draws them as boxes.
    loader.set_use_isolating(false);
    loader
});

/// Looks a message up in the chosen language: `fl!("id")` or
/// `fl!("id", name = value, ...)`.
#[macro_export]
macro_rules! fl {
    ($message_id:literal) => {{
        i18n_embed_fl::fl!($crate::i18n::LANGUAGE_LOADER, $message_id)
    }};
    // Each value goes in brackets: the inner macro writes `value.into()`,
    // which would turn `n + 1` into `n + 1.into()`.
    ($message_id:literal, $($key:ident = $value:expr),+ $(,)?) => {{
        i18n_embed_fl::fl!($crate::i18n::LANGUAGE_LOADER, $message_id, $($key = ($value)),+)
    }};
}

/// A language there's a translation for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Language {
    /// Its folder name, e.g. `es` or `pt-BR`; what the settings store.
    pub id: String,
    /// Its own name for itself, e.g. "Español", from `language-name`.
    pub name: String,
}

/// Every embedded translation, by name, English first.
pub fn available() -> Vec<Language> {
    let mut langs: Vec<Language> = Localizations::iter()
        .filter_map(|path| {
            let (id, file) = path.split_once('/')?;
            (file == "nebula_reader.ftl").then(|| id.to_string())
        })
        .map(|id| {
            let name = Localizations::get(&format!("{id}/nebula_reader.ftl"))
                .and_then(|f| language_name(&String::from_utf8_lossy(&f.data)))
                .unwrap_or_else(|| id.clone());
            Language { id, name }
        })
        .collect();
    langs.sort_by_key(|l| (l.id != "en", l.name.to_lowercase()));
    langs
}

/// The `language-name = ...` line of a translation file.
fn language_name(ftl: &str) -> Option<String> {
    ftl.lines()
        .find_map(|l| l.strip_prefix("language-name"))
        .and_then(|rest| rest.trim_start().strip_prefix('='))
        .map(|name| name.trim().to_string())
        .filter(|name| !name.is_empty())
}

/// Switches the interface to `choice` (a folder name), or with `None` to the
/// system's preferred languages. Anything without a translation falls back
/// to English. Takes effect on the next redraw.
pub fn select(choice: Option<&str>) {
    let requested: Vec<LanguageIdentifier> = match choice {
        Some(id) => id.parse().ok().into_iter().collect(),
        None => DesktopLanguageRequester::requested_languages(),
    };
    if let Err(e) = i18n_embed::select(&*LANGUAGE_LOADER, &Localizations, &requested) {
        log::warn!("language {choice:?}: {e}");
    }
    // libcosmic's own words (Close, Cancel...) follow the same choice.
    cosmic::localize::select(&requested);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    /// Message and term ids defined in a Fluent file.
    fn ids(ftl: &str) -> BTreeSet<String> {
        ftl.lines()
            .filter(|l| l.starts_with(|c: char| c.is_ascii_alphabetic() || c == '-'))
            .filter_map(|l| l.split_once('=').map(|(id, _)| id.trim().to_string()))
            .collect()
    }

    fn file(id: &str) -> String {
        let f = Localizations::get(&format!("{id}/nebula_reader.ftl")).expect("embedded");
        String::from_utf8(f.data.into_owned()).expect("UTF-8")
    }

    #[test]
    fn english_comes_first_and_every_language_has_a_name() {
        let langs = available();
        assert_eq!(langs[0].id, "en");
        for l in &langs {
            assert_ne!(l.name, l.id, "{} has no language-name", l.id);
        }
    }

    /// Each message's text, joined over its lines, by id.
    fn messages(ftl: &str) -> std::collections::BTreeMap<String, String> {
        let mut out = std::collections::BTreeMap::new();
        let mut current: Option<String> = None;
        for line in ftl.lines() {
            if line.starts_with(|c: char| c.is_ascii_alphabetic()) {
                if let Some((id, text)) = line.split_once('=') {
                    current = Some(id.trim().to_string());
                    out.insert(id.trim().to_string(), text.to_string());
                    continue;
                }
            }
            if line.starts_with(' ') || line.starts_with('}') {
                if let Some(id) = &current {
                    out.get_mut(id).unwrap().push_str(line);
                }
            } else {
                current = None;
            }
        }
        out
    }

    /// The `$variables` a message uses.
    fn variables(text: &str) -> BTreeSet<String> {
        text.split('$')
            .skip(1)
            .map(|rest| rest.chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_').collect())
            .collect()
    }

    /// A translation can only use the variables English passes in: any
    /// other one (a typo) would show up as `{$name}` on screen.
    #[test]
    fn translations_use_only_englishs_variables() {
        let english = messages(&file("en"));
        for lang in available() {
            for (id, text) in messages(&file(&lang.id)) {
                let Some(en) = english.get(&id) else { continue };
                let extra: Vec<_> = variables(&text).difference(&variables(en)).cloned().collect();
                assert!(extra.is_empty(), "{}: {id} uses {extra:?}, which English doesn't pass in", lang.id);
            }
        }
    }

    /// Every translation parses, and has no message English doesn't (a
    /// typo'd or stale id). Missing ones just fall back to English, so
    /// they're listed, not failed.
    #[test]
    fn translations_parse_and_match_english() {
        let english = ids(&file("en"));
        for lang in available() {
            let text = file(&lang.id);
            if let Err((_, errors)) = fluent_syntax::parser::parse(text.as_str()) {
                panic!("{}: {errors:?}", lang.id);
            }
            let theirs = ids(&text);
            let unknown: Vec<_> = theirs.difference(&english).collect();
            assert!(unknown.is_empty(), "{} has messages English doesn't: {unknown:?}", lang.id);
            let missing: Vec<_> = english.difference(&theirs).collect();
            if !missing.is_empty() {
                println!("{} is missing (shown in English): {missing:?}", lang.id);
            }
        }
    }

    /// On a loader of its own: other tests read English from the shared one.
    #[test]
    fn a_language_without_a_translation_falls_back_to_english() {
        let loader: FluentLanguageLoader = fluent_language_loader!();
        loader.load_fallback_language(&Localizations).unwrap();
        let es: LanguageIdentifier = "es".parse().unwrap();
        i18n_embed::select(&loader, &Localizations, &[es]).unwrap();
        assert_eq!(loader.current_languages()[0].to_string(), "es");
        assert_eq!(loader.get("language-name"), "Español");
        let xx: LanguageIdentifier = "xx".parse().unwrap();
        i18n_embed::select(&loader, &Localizations, &[xx]).unwrap();
        assert_eq!(loader.get("language-name"), "English");
    }
}
