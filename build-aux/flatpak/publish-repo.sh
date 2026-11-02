#!/bin/sh
# Build a signed Flatpak repository ready to be served as static files.
#
#   build-aux/flatpak/publish-repo.sh <gpg-key-id> <output-dir> <base-url>
#
# Produces:
#   <output-dir>/repo/                 OSTree repo (branch "stable"), GPG-signed
#   <output-dir>/amoxtli.flatpakref    one-click install + updates
#   <output-dir>/amoxtli.flatpakrepo   adds the remote only
#   <output-dir>/index.html            install instructions
#
# The GNOME runtime is not hosted here: the .flatpakref points to Flathub for it.
set -eu

KEY_ID="$1"
OUT="$(realpath -m "$2")"
BASE_URL="${3%/}"
APP_ID=dev.alananaya.Amoxtli
ROOT="$(dirname "$(readlink -f "$0")")/../.."
MANIFEST="$ROOT/build-aux/flatpak/$APP_ID.yml"
STATE="${FLATPAK_STATE_DIR:-$ROOT/.flatpak-builder}"
# the build dir must live on the same filesystem as the state dir
mkdir -p "$STATE"
BUILD="$STATE/publish-build"
trap 'rm -rf "$BUILD"' EXIT

GPG_HOME_ARG=""
[ -n "${GNUPGHOME:-}" ] && GPG_HOME_ARG="--gpg-homedir=$GNUPGHOME"

mkdir -p "$OUT"
flatpak-builder $GPG_HOME_ARG --user --disable-rofiles-fuse --force-clean --install-deps-from=flathub \
  --state-dir="$STATE" --default-branch=stable \
  --gpg-sign="$KEY_ID" --repo="$OUT/repo" \
  "$BUILD" "$MANIFEST"
flatpak build-update-repo $GPG_HOME_ARG --gpg-sign="$KEY_ID" --generate-static-deltas --prune \
  --title="Amoxtli" "$OUT/repo"

GPG_KEY=$(gpg --export "$KEY_ID" | base64 -w0)
VERSION=$(sed -n 's/^version = "\(.*\)"/\1/p' "$ROOT/Cargo.toml" | head -1)

cat > "$OUT/amoxtli.flatpakref" <<EOF
[Flatpak Ref]
Name=$APP_ID
Branch=stable
Title=Amoxtli
Comment=A native Markdown viewer
Url=$BASE_URL/repo/
SuggestRemoteName=amoxtli
Homepage=https://github.com/k4rmakoma0/amoxtli-linux
RuntimeRepo=https://dl.flathub.org/repo/flathub.flatpakrepo
IsRuntime=false
GPGKey=$GPG_KEY
EOF

cat > "$OUT/amoxtli.flatpakrepo" <<EOF
[Flatpak Repo]
Title=Amoxtli
Comment=Flatpak repository for Amoxtli
Url=$BASE_URL/repo/
Homepage=https://github.com/k4rmakoma0/amoxtli-linux
GPGKey=$GPG_KEY
EOF

cp "$ROOT/data/icons/hicolor/scalable/apps/$APP_ID.svg" "$OUT/icon.svg"
touch "$OUT/.nojekyll"
cat > "$OUT/index.html" <<EOF
<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>Amoxtli — Flatpak repository</title>
<link rel="icon" href="icon.svg">
<style>
  :root { color-scheme: light dark; --accent: #c0512b; }
  body { font: 16px/1.6 system-ui, sans-serif; max-width: 42rem; margin: 3rem auto; padding: 0 1.25rem; }
  header { display: flex; gap: 1rem; align-items: center; }
  header img { width: 72px; height: 72px; }
  h1 { margin: 0; }
  .btn { display: inline-block; background: var(--accent); color: #fff; padding: .6rem 1.2rem;
         border-radius: 999px; text-decoration: none; font-weight: 600; }
  pre { background: rgba(127,127,127,.12); padding: .8rem 1rem; border-radius: 8px; overflow-x: auto; }
  small { opacity: .7; }
</style>
</head>
<body>
<header>
  <img src="icon.svg" alt="">
  <div><h1>Amoxtli</h1><div>A native Markdown viewer for Linux · version $VERSION</div></div>
</header>
<p>Install it from this Flatpak repository to get automatic updates through GNOME Software
or <code>flatpak update</code>.</p>
<p><a class="btn" href="amoxtli.flatpakref">Install Amoxtli</a></p>
<p>Or from a terminal:</p>
<pre>flatpak install --user $BASE_URL/amoxtli.flatpakref</pre>
<p><small>The GNOME runtime is downloaded from Flathub. Packages are signed with the key
embedded in <a href="amoxtli.flatpakref">amoxtli.flatpakref</a>.
Source code: <a href="https://github.com/k4rmakoma0/amoxtli-linux">github.com/k4rmakoma0/amoxtli-linux</a>.</small></p>
</body>
</html>
EOF
echo "Repository ready in $OUT (served at $BASE_URL)"
