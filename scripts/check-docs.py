#!/usr/bin/env python3
"""Check repository-owned Markdown links without network access or dependencies."""

from pathlib import Path
import re
import sys
from urllib.parse import unquote, urlsplit

ROOT = Path(__file__).resolve().parent.parent
LINK = re.compile(r"!?\[[^\]\n]*\]\(([^\s)]+)(?:\s+[^)]*)?\)")
HEADING = re.compile(r"^ {0,3}#{1,6}\s+(.+?)(?:\s+#+)?\s*$", re.MULTILINE)
FENCE = re.compile(r"^\s*(`{3,}|~{3,})")


def prose(text: str) -> str:
    """Ignore examples in fenced code blocks, preserving source line numbers."""
    marker = None
    lines = []
    for line in text.splitlines():
        match = FENCE.match(line)
        if match:
            fence = match.group(1)
            if marker is None:
                marker = fence
            elif fence[0] == marker[0] and len(fence) >= len(marker):
                marker = None
            lines.append("")
        else:
            lines.append(line if marker is None else "")
    return "\n".join(lines)


def anchors(path: Path) -> set[str]:
    counts: dict[str, int] = {}
    result = set()
    for heading in HEADING.findall(prose(path.read_text(encoding="utf-8"))):
        base = re.sub(r"[^\w\- ]", "", heading.lower()).replace(" ", "-")
        count = counts.get(base, 0)
        counts[base] = count + 1
        result.add(f"{base}-{count}" if count else base)
    return result


def main() -> int:
    sources = sorted(
        list(ROOT.glob("*.md"))
        + list((ROOT / "assets").rglob("*.md"))
        + list((ROOT / "docs").rglob("*.md"))
        + list((ROOT / ".github").rglob("*.md"))
    )
    errors = []
    checked = 0
    for source in sources:
        text = prose(source.read_text(encoding="utf-8"))
        for match in LINK.finditer(text):
            target = urlsplit(match.group(1).strip("<>"))
            if target.scheme or target.netloc:
                continue
            checked += 1
            path = (source.parent / unquote(target.path)).resolve() if target.path else source
            reason = None
            if not path.is_relative_to(ROOT):
                reason = "link escapes the repository"
            elif not path.exists():
                reason = "target does not exist"
            elif target.fragment and path.suffix == ".md":
                if unquote(target.fragment) not in anchors(path):
                    reason = "heading does not exist"
            if reason:
                line = text.count("\n", 0, match.start()) + 1
                errors.append(f"{source.relative_to(ROOT)}:{line}: {reason}: {match.group(1)}")
    if errors:
        print("\n".join(errors), file=sys.stderr)
        return 1
    print(f"Documentation: {len(sources)} files, {checked} local links checked")
    return 0


if __name__ == "__main__":
    sys.exit(main())
