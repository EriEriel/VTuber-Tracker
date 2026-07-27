#!/usr/bin/env python3
"""Re-render OVER_VIEW.html's diagrams from the .mmd sources in this directory.

Why this exists: OVER_VIEW.html carries its diagrams as *inlined SVG* so the
file works offline with no network and no JavaScript. That would normally cost
you the ability to edit them -- an SVG full of absolute coordinates is not
something anyone maintains by hand. So the Mermaid source stays here, and this
script is the one-way path from source to page.

Edit a .mmd, run this, review the diff. Do not hand-edit the <svg> blocks in
OVER_VIEW.html; the next run of this script will overwrite them.

Sources map to the page in sorted filename order: 01-*.mmd becomes the element
with id="ovd0", 02-*.mmd becomes "ovd1", and so on. Adding a diagram means
adding both a .mmd here and an <svg id="ovdN"> placeholder in the page.

Requires npx (mermaid-cli and svgo are fetched on demand, no install needed).
"""

import pathlib
import re
import shutil
import subprocess
import sys
import tempfile

HERE = pathlib.Path(__file__).resolve().parent
PAGE = HERE.parent.parent / "OVER_VIEW.html"

# mermaid-cli bakes these light-mode hexes into each SVG's <style> block.
# Because the SVG is inlined into the document, a var() here resolves against
# the page's :root tokens -- which is what makes the diagrams follow the
# light/dark toggle without a pile of !important overrides.
COLOR_MAP = {
    "#7b8b90": "var(--ink-faint)",   # lineColor, node borders
    "#0b0b0b": "var(--ink-faint)",   # arrowheads -- must match the lines
    "#4a5c62": "var(--ink-soft)",    # unclassed text: edge labels, cluster titles
    "#eef4f6": "var(--surface-alt)",  # default node / ER entity fill
    "#b9cdd4": "var(--rule)",        # cluster border
    "#000000": "var(--ink)",
}


def run(cmd, **kw):
    result = subprocess.run(cmd, capture_output=True, text=True, **kw)
    if result.returncode != 0:
        sys.exit(f"failed: {' '.join(cmd)}\n{result.stderr[-2000:]}")
    return result


def prepare(svg: str, index: int) -> str:
    """Scope one SVG to the page and make its colours theme-aware."""
    # Every mermaid-cli SVG ships id="my-svg" and scopes its entire <style>
    # block to #my-svg. Nine of those in one document is nine duplicate IDs,
    # and each block would cross-apply to all nine diagrams.
    svg = svg.replace("my-svg", f"ovd{index}")

    for hex_value, token in COLOR_MAP.items():
        svg = re.sub(hex_value, token, svg, flags=re.I)

    # Render at natural size and let the .diagram container scroll. Scaling a
    # 2000px-wide flowchart down to the column width would shrink its 13px
    # labels to about 5px, which defeats the point of having a diagram.
    box = re.search(r'viewBox="[\d.-]+ [\d.-]+ ([\d.]+) ([\d.]+)"', svg)
    if box:
        svg = svg.replace('width="100%"', f'width="{box.group(1)}" height="{box.group(2)}"')
    svg = re.sub(r"max-width:\s*[\d.]+px;\s*", "", svg)
    return svg.strip()


def replace_element(page: str, index: int, svg: str) -> str:
    """Swap the existing <svg id="ovdN">...</svg> for a freshly rendered one."""
    marker = f'<svg id="ovd{index}"'
    start = page.find(marker)
    if start == -1:
        sys.exit(f"OVER_VIEW.html has no element with id=ovd{index}")
    end = page.index("</svg>", start) + len("</svg>")
    return page[:start] + svg + page[end:]


def main() -> None:
    if not shutil.which("npx"):
        sys.exit("npx not found -- needed to fetch mermaid-cli and svgo")

    sources = sorted(HERE.glob("*.mmd"))
    if not sources:
        sys.exit(f"no .mmd sources in {HERE}")

    page = PAGE.read_text()

    with tempfile.TemporaryDirectory() as tmpdir:
        tmp = pathlib.Path(tmpdir)
        for index, source in enumerate(sources):
            raw = tmp / f"{index}.svg"
            print(f"  {source.name} -> ovd{index}", flush=True)

            run([
                "npx", "-y", "@mermaid-js/mermaid-cli@11",
                "-i", str(source), "-o", str(raw),
                "-b", "transparent",
                "-p", str(HERE / "puppeteer-config.json"),
                "-c", str(HERE / "mermaid-config.json"),
            ])
            # Conservative pass only -- anything that restructures nodes risks
            # mangling the foreignObject HTML carrying every multi-line label.
            run([
                "npx", "-y", "svgo@3", str(raw), "-o", str(raw),
                "--config", str(HERE / "svgo.config.mjs"),
            ])

            page = replace_element(page, index, prepare(raw.read_text(), index))

    PAGE.write_text(page)
    print(f"wrote {PAGE} ({len(page):,} bytes)")


if __name__ == "__main__":
    main()
