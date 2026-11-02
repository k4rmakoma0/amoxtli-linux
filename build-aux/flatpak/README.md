# Flatpak

## Build and run locally

```sh
sudo apt install flatpak flatpak-builder
flatpak remote-add --user --if-not-exists flathub https://dl.flathub.org/repo/flathub.flatpakrepo
flatpak-builder --user --install-deps-from=flathub --install --force-clean \
  build-dir build-aux/flatpak/dev.alananaya.Amoxtli.yml
flatpak run dev.alananaya.Amoxtli
```

When `Cargo.lock` changes, regenerate the offline crate sources:

```sh
python3 tools/flatpak-cargo-sources.py
```

## Lint (what Flathub checks)

Flathub builds with the `flatpak-builder` bundled in `org.flatpak.Builder`, mirrors the
screenshots and writes full media URLs. To reproduce that locally:

```sh
flatpak install --user flathub org.flatpak.Builder
flatpak run --command=flatpak-builder-lint org.flatpak.Builder manifest build-aux/flatpak/dev.alananaya.Amoxtli.yml
flatpak run --command=flatpak-builder-lint org.flatpak.Builder appstream data/dev.alananaya.Amoxtli.metainfo.xml

# full build + repo lint (screenshots must already be reachable at the tagged URLs)
flatpak run org.flatpak.Builder --user --disable-rofiles-fuse --force-clean \
  --install-deps-from=flathub --mirror-screenshots-url=https://dl.flathub.org/media \
  --compose-url-policy=full --repo=repo builddir build-aux/flatpak/dev.alananaya.Amoxtli.yml
flatpak run --command=flatpak-builder-lint org.flatpak.Builder repo repo
```

`--disable-rofiles-fuse` is only needed where FUSE can't be used from inside the
Builder sandbox. If you change the compose options, delete `.flatpak-builder/` first:
its cache doesn't track them.

## Single-file bundle (GitHub releases)

Bundles can't carry the separate `.Locale` extension, so build them with
`separate-locales: false` added to the manifest, then:

```sh
flatpak-builder --user --force-clean --repo=repo --default-branch=stable builddir <manifest>
flatpak build-bundle repo amoxtli-<version>-x86_64.flatpak dev.alananaya.Amoxtli stable \
  --runtime-repo=https://dl.flathub.org/repo/flathub.flatpakrepo
```

## Flathub

Amoxtli is **not** on Flathub. Flathub's
[Generative AI policy](https://docs.flathub.org/docs/for-app-authors/requirements)
requires disclosing AI-generated material, does not allow AI-generated or AI-assisted
content in Flathub manifests, and does not allow AI tools to open or write submission
pull requests. Large parts of this project, including this manifest and the
`tools/flatpak-cargo-sources.py` output, were written with an AI assistant, so a Flathub
submission would need a manifest written by hand, crate sources generated with the
official `flatpak-cargo-generator`, a pull request opened and written by the maintainer,
and a disclosure of the AI-generated parts of the application.
