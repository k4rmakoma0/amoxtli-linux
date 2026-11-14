# Snap Store package

The project builds a strictly confined `amoxtli` snap for amd64 using `core26`
and Snapcraft's GNOME extension. GTK and libadwaita come from the GNOME content
snap; no host GTK installation is needed to run the package.

## Build

Install Snapcraft 9.2 or newer and configure LXD as described in the
[Snapcraft documentation](https://ubuntu.com/docs/snapcraft/latest/how-to/select-a-build-provider/).
From the project root:

```sh
tools/build-snap.sh
```

The helper builds a snapshot of tracked and non-ignored project files, leaving
out Git internals and existing build artifacts. The Rust plugin downloads its
toolchain and builds from source with the committed
`Cargo.lock`. `build-aux/snap/install.sh` installs desktop resources, the license
and compiled translations into the package.

The build runs Snapcraft's linters inside the correct core26 environment.
Snapcraft 9.2.1's standalone `snapcraft lint` creates a core22 environment and can
print misleading GLIBC errors for a core26 snap, even when it exits successfully.
Run that additional check inside core26 instead; don't disable library checks
or bundle another copy of libc to silence it.

## Install and test locally

```sh
sudo snap install --dangerous dist/amoxtli_2.1.0_amd64.snap
snap connections amoxtli
amoxtli examples/README.md
```

Use scratch documents for editing tests. Verify:

- Open a file from the launcher, Files, drag and drop, and the system file chooser.
- Open a folder and follow relative Markdown links and images.
- Edit and save in Source, Format and Split; close and reopen to verify persistence.
- Change a document externally and check conflict handling.
- Cancel a system Save dialog and check that edits are retained.
- Copy a code block and check that the document and selection are unchanged.
- Create a Markdown file from the welcome screen and with Ctrl+Shift+N; cancel
  the chooser and choose an existing document to check that its content is kept.
- Check all 20 languages in the code block selector and highlighting in both editors.
- Launch a second document while the app is running (D-Bus registration).
- Check light/dark appearance, custom toolbar icons and a non-English locale.
- Open links and use "Edit in Text Editor" and "Show in Files" through portals.

For repeatable GUI smoke tests, start a separate Xvfb display and run
`python3 tools/test-snap.py --display :88` from your regular desktop terminal.
The script refuses to use that terminal's current display or an already running
Amoxtli instance. It uses scratch files under `target/snap-validation` and checks
the installed release binary, not a replacement debug executable.

The standard `home` interface grants access to ordinary, non-hidden files in
the home directory. Files outside that scope use the desktop portals when
supported. For a drive mounted under `/media`, `/run/media` or `/mnt`, users can
optionally connect the removable-media interface:

```sh
sudo snap connect amoxtli:removable-media
```

The app doesn't offer to change host default associations from inside Snap.
Users can select it in the desktop's Open With/default-app settings.

## Store listing and releases

The package is published as [Amoxtli on the Snap Store](https://snapcraft.io/amoxtli).
Users can install it with `sudo snap install amoxtli` and receive automatic updates.

For future releases, review the icon and the three screenshots from
`data/screenshots` and update the listing when the interface changes. The
`snap/snapcraft.yaml` file contains the title, description, license and links.

Start with a candidate release and test the store-installed build before
promoting that revision to stable. Follow the
[publishing guide](https://ubuntu.com/docs/snapcraft/latest/how-to/publishing/publish-a-snap/).
Store upload is separate from building and local testing.
