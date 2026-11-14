# Amoxtli v2.1.0

Create Markdown documents from the welcome screen and highlight more languages.

- Create an empty Markdown file from the welcome screen, the main menu or
  **Ctrl+Shift+N**, choose its name and location, and start editing immediately.
- Highlight and select **Ruby, Rust, Go, C, C++, Java, Kotlin, PHP, Lua, YAML and
  TOML** in both the source and formatted editors: 20 supported languages in total.
- Recognize common language aliases, raw and multiline strings, nested comments,
  and YAML/TOML configuration keys.
- Translate the new file creation flow into all ten supported languages.

## Downloads

- `amoxtli-2.1.0-x86_64-linux.tar.gz`: native Linux build; requires GTK 4.20+
  and libadwaita 1.8+.
- `amoxtli-2.1.0-x86_64.flatpak`: standalone Flatpak bundle with translations.
- `amoxtli_2.1.0_amd64.snap`: strictly confined Snap package.
- Each download includes a `.sha256` checksum file.

## Maintainer preparation

```sh
cargo test --offline --locked
python3 tools/i18n.py check
appstreamcli validate --no-net data/dev.alananaya.Amoxtli.metainfo.xml
tools/package.sh
tools/build-flatpak.sh
tools/build-snap.sh
```

Verify installation, the welcome-screen creation flow and the language selector
in each packaged build before publishing. Use scratch documents to check saving,
canceling creation, and preserving an existing document when its name is chosen.
Snap GUI smoke tests are documented in `build-aux/snap/README.md`.

## Publishing

After committing the reviewed release changes, tag the release `v2.1.0` and push
the commit and tag. The tag triggers the existing GitHub Actions workflow that
builds and deploys the signed Flatpak repository on GitHub Pages.

Create the GitHub release using the title and introductory notes above, and
attach the six versioned downloads/checksums from `dist/`.

Upload `dist/amoxtli_2.1.0_amd64.snap` to the Snap Store candidate channel, verify
the store-installed build, then promote that revision to stable. See
`build-aux/snap/README.md` for the store release procedure.
