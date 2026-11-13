#!/bin/sh
# Install a compiled binary and desktop resources into a Snap part's staging tree.
set -eu
APP_ID=dev.alananaya.Amoxtli
BINARY="$1"
DEST="$2/usr"

install -Dm755 "$BINARY" "$DEST/bin/amoxtli"
install -d "$DEST/share/applications"
sed 's|@BINDIR@|/usr/bin|g' "data/$APP_ID.desktop.in" \
  | sed '/^TryExec=/d' > "$DEST/share/applications/$APP_ID.desktop"
install -Dm644 "data/$APP_ID.metainfo.xml" "$DEST/share/metainfo/$APP_ID.metainfo.xml"
install -Dm644 LICENSE "$DEST/share/licenses/amoxtli/LICENSE"
for variant in scalable symbolic; do
  install -d "$DEST/share/icons/hicolor/$variant/apps"
  cp data/icons/hicolor/"$variant"/apps/*.svg "$DEST/share/icons/hicolor/$variant/apps/"
done
install -d "$DEST/share/icons/hicolor/scalable/actions"
cp data/icons/hicolor/scalable/actions/amoxtli-*-symbolic.svg "$DEST/share/icons/hicolor/scalable/actions/"
python3 tools/i18n.py compile "$DEST/share/locale"
