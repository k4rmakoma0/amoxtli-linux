#!/usr/bin/env python3
"""Generate flatpak-builder sources for the crates in Cargo.lock (offline builds).

    tools/flatpak-cargo-sources.py [Cargo.lock] [-o build-aux/flatpak/cargo-sources.json]

Produces the same layout as flatpak-builder-tools' flatpak-cargo-generator:
every crates.io package is downloaded into cargo/vendor/<name>-<version>
with its .cargo-checksum.json, and cargo/config.toml points cargo at it.
Run it again whenever Cargo.lock changes.
"""

import json
import sys
import tomllib
from pathlib import Path

CRATES_IO = "registry+https://github.com/rust-lang/crates.io-index"
VENDOR = "cargo/vendor"


def main() -> int:
    args = sys.argv[1:]
    out = Path("build-aux/flatpak/cargo-sources.json")
    if "-o" in args:
        i = args.index("-o")
        out = Path(args[i + 1])
        del args[i:i + 2]
    lock = Path(args[0] if args else "Cargo.lock")
    packages = tomllib.loads(lock.read_text())["package"]

    sources = []
    for pkg in packages:
        source = pkg.get("source")
        if source is None:
            continue  # the workspace crate itself
        if source != CRATES_IO:
            sys.exit(f"unsupported source for {pkg['name']}: {source}")
        name, version, checksum = pkg["name"], pkg["version"], pkg["checksum"]
        dest = f"{VENDOR}/{name}-{version}"
        sources.append({
            "type": "archive",
            "archive-type": "tar-gzip",
            "url": f"https://static.crates.io/crates/{name}/{name}-{version}.crate",
            "sha256": checksum,
            "dest": dest,
        })
        sources.append({
            "type": "inline",
            "contents": json.dumps({"package": checksum, "files": {}}),
            "dest": dest,
            "dest-filename": ".cargo-checksum.json",
        })
    sources.append({
        "type": "inline",
        "contents": f'[source.vendored-sources]\ndirectory = "{VENDOR}"\n\n'
                    '[source.crates-io]\nreplace-with = "vendored-sources"\n',
        "dest": "cargo",
        "dest-filename": "config.toml",
    })
    out.parent.mkdir(parents=True, exist_ok=True)
    out.write_text(json.dumps(sources, indent=4) + "\n")
    print(f"wrote {out} ({len(sources) // 2} crates)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
