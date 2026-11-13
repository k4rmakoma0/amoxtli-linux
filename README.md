# Amoxtli: Markdown Editor

*Amoxtli* (“book” in Nahuatl) is a native Markdown editor for Linux, written in
**Rust** with **GTK 4 + libadwaita** so it looks and behaves like a first-class GNOME app.
It ships as a single ~4 MB binary.

![The formatted editor, with its formatting tools and the table of contents](data/screenshots/light.png)

<table>
  <tr>
    <td><img src="data/screenshots/dark.png" alt="The split view in dark mode: Markdown source and formatted document side by side"></td>
    <td><img src="data/screenshots/folder.png" alt="Editing the Markdown source in a folder of notes"></td>
  </tr>
  <tr>
    <td align="center">Source and formatted view side by side</td>
    <td align="center">The source, in a folder of notes</td>
  </tr>
</table>

There is no embedded browser. Markdown is edited straight in native widgets:
Pango-shaped text in a `GtkTextView`, tables as grids of editable cells, images as GPU textures.

## Features

- Open files (`Ctrl+O`), folders (`Ctrl+Shift+O`), drag & drop, or `amoxtli file.md|folder`
- Three views (`Ctrl+Alt+3/4/5`): **Source**, the Markdown with live highlighting; **Format**, a
  word-processor view where Markdown never shows; and **Split**, both side by side, both editable,
  scrolling together block by block
- Formatting tools in both editors (`Ctrl+B`, `Ctrl+I`, `Ctrl+1…6` for headings, `Ctrl+K` for links…);
  lists continue on Return and end on an empty item, `Tab` nests them, `# ` typed starts a heading,
  tables are edited cell by cell. Links open with `Ctrl+click` in the formatted view
- Code blocks colored by language in both editors (Swift, Python, JavaScript, TypeScript, JSON,
  HTML/XML, CSS, SQL and Shell, with a small hand-written lexer); in the formatted view, a chip on each
  block shows its language and changes it
- Opening a file never rewrites it. Edits are saved a second after you stop typing (or `Ctrl+S`), and
  never over a file another app changed: Amoxtli asks which version to keep. A renamed file is followed;
  a deleted one is never brought back unasked
- Documents over 1 MB are edited in the source, which stays fast at that size
- **Contents** sidebar: the document's table of contents, follows your scroll position
- **Files** sidebar: lazy folder tree that shows only Markdown files and directories
- Opening a folder shows its `README.md` / `index.md` automatically
- Live reload when the file changes on disk (keeps your caret and scroll position)
- Relative links between `.md` files, `#heading` anchors, back/forward history (`Alt+←/→`)
- CommonMark + tables, task lists, strikethrough, GitHub alerts (`> [!NOTE]`), `{#custom-id}` heading
  anchors, `<u>` underline, local and remote images (PNG/JPEG/SVG/GIF/WebP). HTML blocks, footnotes and
  YAML/TOML front matter are kept exactly as written
- Find in document (`Ctrl+F`, or just start typing)
- Choose the font (System, Serif, Monospace, or Rounded when a rounded font is installed) and the text
  size (`Ctrl +/-/0`) from the **Aa** menu, for the source and the formatted view alike
- Light/dark mode and the system accent colour, adaptive layout for narrow windows
- Can register itself as the default app for `text/markdown`
- Word count and reading time in the status bar (or for the selected text)
- Translated into Spanish, Portuguese (Brazil), French, German, Italian, Russian,
  Chinese (Simplified and Traditional), Japanese and Korean — follows the system language

## Download

**Flatpak repository (recommended, with automatic updates):** open
<https://k4rmakoma0.github.io/amoxtli-linux/> and click **Install**, or run:

```sh
flatpak install --user https://k4rmakoma0.github.io/amoxtli-linux/amoxtli.flatpakref
```

Updates then arrive through GNOME Software or `flatpak update`. The GNOME runtime is
downloaded from Flathub.

From the [latest release](https://github.com/k4rmakoma0/amoxtli-linux/releases/latest)
you can also get a single-file Flatpak bundle (no automatic updates):
`flatpak install --user amoxtli-<version>-x86_64.flatpak`.

**Prebuilt binary** (needs GTK ≥ 4.20 and libadwaita ≥ 1.8, i.e. GNOME 49+):

```sh
tar xzf amoxtli-2.0.0-x86_64-linux.tar.gz
cd amoxtli-2.0.0-x86_64-linux
./install.sh            # or ./install.sh --default to make it your Markdown app
```

Or build it from source as described below. The Flatpak manifest lives in
[`build-aux/flatpak`](build-aux/flatpak/README.md).

## Requirements

Ubuntu/Debian:

```sh
sudo apt install rustc cargo build-essential pkg-config libgtk-4-dev libadwaita-1-dev
```

Fedora: `sudo dnf install rust cargo gtk4-devel libadwaita-devel`

GTK ≥ 4.20 and libadwaita ≥ 1.8 are required (GNOME 49+).

## Run from source

```sh
python3 tools/i18n.py compile   # once, for translations in development builds
cargo run -- examples/README.md
cargo test
```

## Install

```sh
./install.sh             # builds (release) and installs to ~/.local (app grid entry, icon, “Open With”)
./install.sh --default   # …and makes it the default Markdown app
cargo build --release && sudo PREFIX=/usr/local ./install.sh   # system-wide
./uninstall.sh
```

You can also make it the default later from the app menu (**Set as Default Markdown App**),
from GNOME Settings → Apps → Default Apps, or with:

```sh
xdg-mime default dev.alananaya.Amoxtli.desktop text/markdown text/x-markdown
```

## Translations

Translations live in `po/<lang>.po` (standard gettext format, editable with Poedit,
GNOME Translation Editor, or any text editor). In code, mark strings with `tr("…")`
or `trn("…", "…", n)`. No gettext tools are required:

```sh
python3 tools/i18n.py update    # extract strings from src/ and merge them into every po/*.po
python3 tools/i18n.py check     # list missing translations per language
python3 tools/i18n.py compile   # build target/locale/*/LC_MESSAGES/amoxtli.mo
```

To add a language, copy `po/amoxtli.pot` to `po/<lang>.po`, set the `Language` and
`Plural-Forms` headers, translate, then run `compile`. Try it with e.g.
`LANGUAGE=es cargo run`. The launcher's name/description translations are in
`data/dev.alananaya.Amoxtli.desktop.in`.

## Layout

```
src/main.rs          entry point
src/application.rs   Adw.Application: actions, shortcuts, CSS, file opening
src/window.rs        main window: header bar, split view, search, history, reload, status bar
src/document.rs      headings, word counts and other helpers both editors share
src/markdown/        GTK-free logic (tested): document model, parser and canonical serializer,
                     syntax scanner, code lexers, formatting tools, Return in lists, split scroll map
src/editor/          source editor, formatting toolbar and menus, link/table dialogs
src/editor/rich/     the formatted editor: conversion, keys, tools, drawing, tables, undo
src/widgets.rs       FitImage / FitBox helper widgets
src/sidebar.rs       outline (index) and folder tree panels
src/i18n.rs          gettext setup
src/config.rs        constants and saved window state
src/typography.rs    font and text size choices (tested)
po/                  translations (.po) and template (.pot)
tools/i18n.py        extract / merge / compile / check translations
tools/package.sh     build the prebuilt release tarball into dist/
tools/flatpak-cargo-sources.py  offline crate sources for the Flatpak build
build-aux/flatpak/   Flatpak manifest and Flathub submission notes
data/                .desktop entry, AppStream metainfo, app and toolbar icons
```

## License

Amoxtli is open source under the [MIT License](LICENSE).
