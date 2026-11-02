#!/bin/sh
set -eu
APP_ID=dev.alananaya.Amoxtli
PREFIX="${PREFIX:-$HOME/.local}"
rm -rf "$PREFIX/share/amoxtli"
rm -f "$PREFIX/bin/amoxtli" \
  "$PREFIX/share/applications/$APP_ID.desktop" \
  "$PREFIX/share/metainfo/$APP_ID.metainfo.xml" \
  "$PREFIX/share/icons/hicolor/scalable/apps/$APP_ID.svg" \
  "$PREFIX/share/icons/hicolor/symbolic/apps/$APP_ID-symbolic.svg"
for mo in "$PREFIX"/share/locale/*/LC_MESSAGES/amoxtli.mo; do
  [ -e "$mo" ] && rm -f "$mo"
done
# files from the previous application ID (io.github.amoxtli.Amoxtli, before 1.1.0)
OLD_ID=io.github.amoxtli.Amoxtli
rm -f "$PREFIX/share/applications/$OLD_ID.desktop" "$PREFIX/share/metainfo/$OLD_ID.metainfo.xml" \
  "$PREFIX/share/icons/hicolor/scalable/apps/$OLD_ID.svg" "$PREFIX/share/icons/hicolor/symbolic/apps/$OLD_ID-symbolic.svg"

command -v update-desktop-database >/dev/null && update-desktop-database -q "$PREFIX/share/applications" || true
echo "Amoxtli removed from $PREFIX"
