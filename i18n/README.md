# Translating Nebula Reader

Nebula's interface text lives in this folder, one folder per language:

```
i18n/
  en/nebula_reader.ftl   English: the reference, and the fallback
  es/nebula_reader.ftl   Spanish
```

The files use [Fluent](https://projectfluent.org/), the format libcosmic
and the COSMIC apps use. A new language needs no code changes: add its
folder and it appears in **Settings → Appearance → Language**.

## Adding a language

1. Copy `en/` to a folder named after the language's
   [code](https://en.wikipedia.org/wiki/IETF_language_tag): `fr` for
   French, `de` for German, `pt-BR` for Brazilian Portuguese, `zh-Hans`
   for Simplified Chinese, and so on.
2. In the copy, change `language-name` first. It's the language's own name
   for itself (`Français`, `Deutsch`, `Português (Brasil)`), which is how
   it's listed in Settings.
3. Translate the text after each `=`. Leave the ids on the left alone.
4. Run `cargo test`. It checks that your file parses, has no ids English
   doesn't, and only uses the `{ $variables }` English uses.
5. Try it: `cargo run`, then pick your language in Settings. It switches
   straight away.
6. Open a pull request. A partial translation is welcome too: anything
   you leave out shows in English.

## The format, briefly

```ftl
# A comment for translators.
open-file = Open File

# { $name } is filled in by Nebula. Keep it, but move it wherever your
# language needs it.
jf-signed-in = Signed in as { $user }. Loading your books…

# Text that depends on a number. Use your language's plural categories
# (zero, one, two, few, many, other); the one marked * is the default.
page-count = { $count ->
    [one] { $count } page
   *[other] { $count } pages
}
```

- Keep every `{ $variable }` that the English message has, spelled the
  same. You can leave one out if your language doesn't need it, but
  never invent new ones.
- Plural categories differ by language: Polish uses `one`, `few`, `many`
  and `other`; Japanese only needs `other`. Fluent picks the right one
  from the number. See the
  [CLDR plural rules](https://www.unicode.org/cldr/charts/latest/supplemental/language_plural_rules.html).
- Keyboard keys in hints (`Esc`, `Ctrl`, `Space`, the letters) are the
  keys to press, so keep the letters as they are. Translate the names of
  keys when your keyboards label them differently (`Espacio`, `Mayús`).
- Leave product names untranslated: Nebula Reader, Jellyfin, COSMIC,
  ComicVine, AniList, Flathub, GitHub.
- Dates are built from `month-1` … `month-12` and `date-day` /
  `date-month`, so they can follow your language's order.
- Comment lines (`#`) and section headings (`##`) are for you; they don't
  show in the app.

## Keeping a translation up to date

When new text is added to English, it shows in English in your language
until someone translates it. `cargo test -- --nocapture i18n` lists the
messages each language is missing.

## For developers

Use `fl!("message-id")`, or `fl!("message-id", name = value)` for a
message with variables, instead of a string literal for anything a user
reads (src/i18n.rs). Add the message to `en/nebula_reader.ftl` in the
section it belongs to; an id that isn't there doesn't compile. Add it to
the other languages if you can, or leave it for their translators.
Log messages, file and folder names on disk, and data from the user's
files or servers aren't translated.
