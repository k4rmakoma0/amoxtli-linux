#!/bin/sh
# Build a clean source snapshot, excluding ignored artifacts and Git internals.
set -eu
ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
mkdir -p "$ROOT/target" "$ROOT/dist"
AMOXTLI_SNAP_SOURCE_DIR=$(mktemp -d "$ROOT/target/snap-source.XXXXXX")
trap 'rm -rf "$AMOXTLI_SNAP_SOURCE_DIR"' EXIT HUP INT TERM
python3 - "$ROOT" "$AMOXTLI_SNAP_SOURCE_DIR" <<'PY'
import shutil
import subprocess
import sys
from pathlib import Path

root, dest = map(Path, sys.argv[1:])
files = subprocess.check_output(
    ["git", "ls-files", "--cached", "--others", "--exclude-standard", "-z"], cwd=root
)
for name in files.decode().split("\0"):
    if not name:
        continue
    source = root / name
    if not source.is_file():
        continue
    target = dest / name
    target.parent.mkdir(parents=True, exist_ok=True)
    shutil.copy2(source, target)
PY
cd "$AMOXTLI_SNAP_SOURCE_DIR"
mkdir -p dist
if command -v snapcraft >/dev/null; then
    snapcraft pack --use-lxd --output=dist
else
    /snap/bin/snapcraft pack --use-lxd --output=dist
fi
cp dist/*.snap "$ROOT/dist/"
for package in dist/*.snap; do
    name=$(basename "$package")
    (cd "$ROOT/dist" && sha256sum "$name" > "$name.sha256")
done
