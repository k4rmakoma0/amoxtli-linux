#!/bin/sh
# Build and install Amoxtli for the current user (default) or system-wide:
#   ./install.sh                     -> ~/.local
#   ./install.sh --default           -> also make it the default Markdown viewer
#   sudo PREFIX=/usr/local ./install.sh   (build first as your user: cargo build --release)
set -eu

APP_ID=io.github.amoxtli.Amoxtli
SRC="$(dirname "$(readlink -f "$0")")"
PREFIX="${PREFIX:-$HOME/.local}"
BINDIR="$PREFIX/bin"
DATADIR="$PREFIX/share"
LOCALEDIR="$DATADIR/locale"
MAKE_DEFAULT=0
for arg in "$@"; do
  case "$arg" in
    --default) MAKE_DEFAULT=1 ;;
    -h|--help) sed -n '2,5p' "$0"; exit 0 ;;
    *) echo "unknown option: $arg" >&2; exit 1 ;;
  esac
done

command -v cargo >/dev/null || {
  echo "Rust is required. On Ubuntu/Debian: sudo apt install rustc cargo build-essential pkg-config libgtk-4-dev libadwaita-1-dev" >&2
  exit 1
}

echo "Building (release)…"
cd "$SRC"
AMOXTLI_LOCALEDIR="$LOCALEDIR" cargo build --release --quiet

echo "Installing to $PREFIX"
install -d "$BINDIR" "$DATADIR/applications" "$DATADIR/metainfo" \
  "$DATADIR/icons/hicolor/scalable/apps" "$DATADIR/icons/hicolor/symbolic/apps"
install -m 755 target/release/amoxtli "$BINDIR/amoxtli"

# translations: msgfmt when available, otherwise the bundled pure-Python compiler
for po in po/*.po; do
  lang=$(basename "$po" .po)
  install -d "$LOCALEDIR/$lang/LC_MESSAGES"
  if command -v msgfmt >/dev/null; then
    msgfmt -o "$LOCALEDIR/$lang/LC_MESSAGES/amoxtli.mo" "$po"
  fi
done
command -v msgfmt >/dev/null || python3 tools/i18n.py compile "$LOCALEDIR" >/dev/null

sed "s|@BINDIR@|$BINDIR|g" "data/$APP_ID.desktop.in" > "$DATADIR/applications/$APP_ID.desktop"
install -m 644 "data/$APP_ID.metainfo.xml" "$DATADIR/metainfo/"
install -m 644 "data/icons/hicolor/scalable/apps/$APP_ID.svg" "$DATADIR/icons/hicolor/scalable/apps/"
install -m 644 "data/icons/hicolor/symbolic/apps/$APP_ID-symbolic.svg" "$DATADIR/icons/hicolor/symbolic/apps/"

# remove the old Python version if present
rm -rf "$DATADIR/amoxtli"

command -v update-desktop-database >/dev/null && update-desktop-database -q "$DATADIR/applications" || true
command -v gtk4-update-icon-cache >/dev/null && gtk4-update-icon-cache -q -t -f "$DATADIR/icons/hicolor" 2>/dev/null || true
command -v gtk-update-icon-cache >/dev/null && gtk-update-icon-cache -q -t -f "$DATADIR/icons/hicolor" 2>/dev/null || true

if [ "$MAKE_DEFAULT" = 1 ]; then
  xdg-mime default "$APP_ID.desktop" text/markdown text/x-markdown
  echo "Amoxtli is now the default application for Markdown files."
fi

echo "Done. Launch “Amoxtli” from your app grid, or run: $BINDIR/amoxtli [file.md|folder]"
case ":$PATH:" in *":$BINDIR:"*) ;; *) echo "Note: $BINDIR is not in your PATH." ;; esac
