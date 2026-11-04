"""Uploads the e2e recordings to asciinema.org and writes a demos page.

Usage: upload-demos RECORDINGS [PAGE]

RECORDINGS is the push-recordings output; PAGE (docs/demos.md by default) is
the markdown file to write, with each scenario's preview image linking to its
player. Run `asciinema auth` once first: asciinema.org deletes recordings that
no account has claimed after a few days, which would break the page's links.
"""

import json
import re
import subprocess
import sys
import tempfile
from pathlib import Path

URL = re.compile(r"https://asciinema\.org/a/\w+")
HEADER = """# Demos

Recorded by the NixOS VM tests in `nix/e2e`: real pushes and rebuilds
between VMs. Click a preview to play it.
"""


def titled(cast: Path, title: str, into: Path) -> Path:
    header, rest = cast.read_text(encoding="utf-8").split("\n", 1)
    copy = into / cast.name
    copy.write_text(json.dumps({**json.loads(header), "title": title})
                    + "\n" + rest, encoding="utf-8")
    return copy


def upload(cast: Path) -> str:
    done = subprocess.run(["asciinema", "upload", str(cast)],
                          capture_output=True, text=True)
    output = done.stdout + done.stderr
    found = URL.search(output)
    if done.returncode != 0 or not found:
        sys.exit(f"upload-demos: uploading {cast.name} failed:\n{output}")
    return found.group()


def section(scenario: dict, url: str) -> str:
    return (f"## {scenario['title']}\n\n`{scenario['command']}`\n\n"
            f"[![{scenario['name']}]({url}.svg)]({url})\n")


def main() -> None:
    recordings = Path(sys.argv[1])
    page = Path(sys.argv[2] if len(sys.argv) > 2 else "docs/demos.md")
    scenarios = json.loads((recordings / "scenarios.json").read_text())
    sections = []
    with tempfile.TemporaryDirectory() as tmp:
        for scenario in scenarios:
            cast = recordings / f"{scenario['name']}.cast"
            url = upload(titled(cast, scenario["title"], Path(tmp)))
            print(f"{scenario['name']}: {url}")
            sections.append(section(scenario, url))
    page.write_text("\n".join([HEADER, *sections]))


if __name__ == "__main__":
    main()
