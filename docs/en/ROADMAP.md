# Roadmap

[Português](../pt-BR/ROADMAP.md) · **English**

What is planned, with no dates and in no particular order. An item here is not a promise: if
you want to help with one, open an issue first to agree on the approach
([CONTRIBUTING](CONTRIBUTING.md)).

## More languages and scripts

The library's share of the
[Tirech Omni](https://github.com/TirechCorp/tirech-omni/blob/main/docs/en/ROADMAP.md) language
plan:

- **Fonts for other scripts.** Selawik covers only the Latin alphabet; Chinese, Japanese and
  Korean already exist through the `cjk` feature (about 4.5 MB). Arabic, Hebrew, Devanagari,
  Thai and others need bundled Noto fonts (behind a feature, like `cjk`) or the system fonts.
- **Right-to-left scripts.** Mirror the widget layout (`Row`, `NavigationView`, directional
  icons, Tab focus order) for Arabic and Hebrew; parley already handles the text order (bidi).
- **Search and line breaking** in languages without spaces between words.
- **Built-in strings (`Strings`) per language.** Today the default is Portuguese only and the
  app replaces each field; the idea is to ship ready-made sets (`Strings::en()`, for example).
- **Lint:** check clipped text and missing glyphs in each of the app's languages.
