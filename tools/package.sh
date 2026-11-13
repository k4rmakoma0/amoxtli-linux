#!/bin/sh
# Build a prebuilt release tarball: dist/amoxtli-<version>-<arch>-linux.tar.gz
set -eu
ROOT="$(dirname "$(readlink -f "$0")")/.."
cd "$ROOT"
APP_ID=dev.alananaya.Amoxtli
VERSION=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
ARCH=$(uname -m)
NAME="amoxtli-$VERSION-$ARCH-linux"
STAGE="dist/$NAME"

cargo build --release --quiet
rm -rf "$STAGE" && mkdir -p "$STAGE/bin" "$STAGE/share/applications" "$STAGE/share/metainfo" \
  "$STAGE/share/icons/hicolor/scalable/apps" "$STAGE/share/icons/hicolor/symbolic/apps"
install -m 755 target/release/amoxtli "$STAGE/bin/"
python3 tools/i18n.py compile "$STAGE/share/locale" >/dev/null
cp "data/$APP_ID.desktop.in" "$STAGE/share/applications/"
cp "data/$APP_ID.metainfo.xml" "$STAGE/share/metainfo/"
cp "data/icons/hicolor/scalable/apps/$APP_ID.svg" "$STAGE/share/icons/hicolor/scalable/apps/"
cp "data/icons/hicolor/symbolic/apps/$APP_ID-symbolic.svg" "$STAGE/share/icons/hicolor/symbolic/apps/"
mkdir -p "$STAGE/share/icons/hicolor/scalable/actions"
cp data/icons/hicolor/scalable/actions/amoxtli-*-symbolic.svg "$STAGE/share/icons/hicolor/scalable/actions/"
cp LICENSE README.md "$STAGE/"

cat > "$STAGE/install.sh" <<'SH'
#!/bin/sh
# Install the prebuilt Amoxtli: ./install.sh [--default]   (PREFIX defaults to ~/.local)
set -eu
APP_ID=dev.alananaya.Amoxtli
HERE="$(dirname "$(readlink -f "$0")")"
PREFIX="${PREFIX:-$HOME/.local}"
install -d "$PREFIX/bin" "$PREFIX/share/applications"
install -m 755 "$HERE/bin/amoxtli" "$PREFIX/bin/amoxtli"
for dir in locale icons metainfo; do
  mkdir -p "$PREFIX/share/$dir" && cp -r "$HERE/share/$dir/." "$PREFIX/share/$dir/"
done
sed "s|@BINDIR@|$PREFIX/bin|g" "$HERE/share/applications/$APP_ID.desktop.in" > "$PREFIX/share/applications/$APP_ID.desktop"
command -v update-desktop-database >/dev/null && update-desktop-database -q "$PREFIX/share/applications" || true
command -v gtk-update-icon-cache >/dev/null && gtk-update-icon-cache -q -t -f "$PREFIX/share/icons/hicolor" 2>/dev/null || true
[ "${1:-}" = "--default" ] && xdg-mime default "$APP_ID.desktop" text/markdown text/x-markdown
echo "Amoxtli installed to $PREFIX. Launch it from your app grid or run: $PREFIX/bin/amoxtli"
SH
chmod 755 "$STAGE/install.sh"

tar -C dist -czf "dist/$NAME.tar.gz" "$NAME"
(cd dist && sha256sum "$NAME.tar.gz" > "$NAME.tar.gz.sha256")
echo "dist/$NAME.tar.gz"
