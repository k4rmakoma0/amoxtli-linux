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

The home mount is deliberately read-only. Files selected through the document
portal can be writable; files reached through the built-in folder browser may
not be. On the first permission-denied or read-only save, Amoxtli opens the
system Save dialog so the user can authorize that file or choose a copy. It
retains the returned writable URI for subsequent saves. Cancellation keeps the
edits in memory; automatic saves do not reopen the dialog until an explicit Save
action retries it. Closing after cancellation still offers to discard changes.

When testing, cover opening a file from Files, the file chooser, a selected
folder, the built-in folder browser, and a relative Markdown link. Check saving,
canceling the permission dialog, retrying with Ctrl+S, closing with unsaved edits,
and external changes while the permission dialog is open. The debug self-test
also checks the save-dialog completion paths without driving the native chooser.

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

## Flatpak repository (GitHub Pages)

`.github/workflows/flatpak-repo.yml` builds the app on every `v*` tag (or by hand from
the Actions tab), signs it, and publishes a Flatpak repository to
<https://k4rmakoma0.github.io/amoxtli-linux/> using `publish-repo.sh`. Users install
it with the `.flatpakref` there and receive updates automatically.

One-time setup:

1. **Pages source**: Settings → Pages → Source: *GitHub Actions*.
2. **Signing key** (keep a backup: if it's lost, users have to re-add the remote):

   ```sh
   gpg --batch --passphrase '' --quick-gen-key \
     "Amoxtli Flatpak repository <hello@alananaya.dev>" rsa4096 sign never
   KEY=$(gpg --list-secret-keys --with-colons "Amoxtli Flatpak repository" | awk -F: '/^fpr/{print $10; exit}')
   gpg --armor --export-secret-keys "$KEY" | gh secret set FLATPAK_GPG_PRIVATE_KEY
   ```

To test the same build locally (with any key from your keyring):

```sh
build-aux/flatpak/publish-repo.sh <key-id> public http://127.0.0.1:8000
(cd public && python3 -m http.server 8000) &
flatpak install --user http://127.0.0.1:8000/amoxtli.flatpakref
```

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
