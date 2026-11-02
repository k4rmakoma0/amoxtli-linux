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

## Submitting to Flathub

1. Tag a release (e.g. `v1.1.0`) and push it.
2. Fork <https://github.com/flathub/flathub> and create a branch from its `new-pr` branch.
3. Add `dev.alananaya.Amoxtli.yml` (this manifest, with the `dir` source replaced by the
   tagged git source below) and `cargo-sources.json`:

   ```yaml
       sources:
         - type: git
           url: https://github.com/k4rmakoma0/amoxtli-linux.git
           tag: v1.1.0
           commit: <full commit hash of the tag>
         - cargo-sources.json
   ```

4. Open a pull request against `new-pr` titled **Add dev.alananaya.Amoxtli**.
5. Once published, verify the app in the Flathub developer portal: it gives you a token
   to serve at `https://alananaya.dev/.well-known/org.flathub.VerifiedApps.txt`.
