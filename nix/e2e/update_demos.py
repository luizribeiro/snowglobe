"""Copies the e2e recordings into the docs and writes a demos page.

Usage: update-demos RECORDINGS [PAGE]

RECORDINGS is the deploy-recordings output; PAGE (docs/demos.md by default) is
the markdown file to write. Each demo's cast, which the documentation site
plays, and its animated SVG, which GitHub shows, go into a `demos` directory
next to PAGE, replacing what was there.
"""

import json
import shutil
import sys
from pathlib import Path

HEADER = """# Demos

Every clip below is one of the NixOS VM tests in `nix/e2e`. Real deployments
and rebuilds between VMs, and the test checks they worked, so what you see
here is tested.
"""
KINDS = ("cast", "svg")


def section(demo: dict) -> str:
    return (f"## {demo['title']}\n\n`{demo['command']}`\n\n"
            f"![{demo['name']}](demos/{demo['name']}.svg)\n")


def main() -> None:
    recordings = Path(sys.argv[1])
    page = Path(sys.argv[2] if len(sys.argv) > 2 else "docs/demos.md")
    demos = json.loads((recordings / "demos.json").read_text())
    files = [f"{demo['name']}.{kind}" for demo in demos for kind in KINDS]
    missing = [name for name in files if not (recordings / name).exists()]
    if missing:
        sys.exit(f"update-demos: the recordings lack {', '.join(missing)}")
    into = page.parent / "demos"
    into.mkdir(exist_ok=True)
    for kind in KINDS:
        for old in into.glob(f"*.{kind}"):
            old.unlink()
    for name in files:
        shutil.copyfile(recordings / name, into / name)
    page.write_text("\n".join([HEADER, *map(section, demos)]))


if __name__ == "__main__":
    main()
