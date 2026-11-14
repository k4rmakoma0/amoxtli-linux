#!/bin/sh
# Build a standalone Flatpak bundle with translations included.
set -eu
ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$ROOT"
VERSION=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
ARCH=$(flatpak --default-arch)
APP_ID=dev.alananaya.Amoxtli
RELEASE="$ROOT/target/flatpak-release"
mkdir -p "$RELEASE" "$ROOT/dist"
MANIFEST="$RELEASE/$APP_ID.yml"

# Resolve sources before moving the manifest; keep the development manifest intact.
python3 - "$ROOT" "$MANIFEST" <<'PY'
import sys
from pathlib import Path

root, dest = map(Path, sys.argv[1:])
source = root / "build-aux/flatpak/dev.alananaya.Amoxtli.yml"
text = source.read_text()
text = "separate-locales: false\n" + text
text = text.replace("path: ../..", "path: " + str(root))
text = text.replace("- cargo-sources.json", "- " + str(source.parent / "cargo-sources.json"))
dest.write_text(text)
PY

if command -v flatpak-builder >/dev/null; then
    set -- flatpak-builder
else
    set -- flatpak run org.flatpak.Builder
fi
"$@" --user --disable-rofiles-fuse --force-clean --install-deps-from=flathub \
    --state-dir="$ROOT/.flatpak-builder" --default-branch=stable \
    --repo="$RELEASE/repo" "$RELEASE/build" "$MANIFEST"

BUNDLE="amoxtli-$VERSION-$ARCH.flatpak"
flatpak build-bundle "$RELEASE/repo" "$ROOT/dist/$BUNDLE" "$APP_ID" stable \
    --runtime-repo=https://dl.flathub.org/repo/flathub.flatpakrepo
(cd "$ROOT/dist" && sha256sum "$BUNDLE" > "$BUNDLE.sha256")
echo "dist/$BUNDLE"
