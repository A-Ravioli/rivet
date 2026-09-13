#!/usr/bin/env python3
"""Check the book's internal links and its table of contents.

mdBook itself tolerates a link to a file that does not exist, which is how
a reorganised chapter quietly leaves a dead link behind.
"""
import re
import sys
from pathlib import Path

SRC = Path(__file__).resolve().parent.parent / "docs" / "book" / "src"
LINK = re.compile(r"\[[^\]]*\]\(([^)]+)\)")

problems = []
summary = (SRC / "SUMMARY.md").read_text()
listed = set()
for target in LINK.findall(summary):
    listed.add(target)
    if not (SRC / target).exists():
        problems.append(f"SUMMARY.md lists {target}, which does not exist")

for page in sorted(SRC.glob("*.md")):
    if page.name != "SUMMARY.md" and page.name not in listed:
        problems.append(f"{page.name} is not in SUMMARY.md")
    for target in LINK.findall(page.read_text()):
        if target.startswith(("http://", "https://", "#", "mailto:")):
            continue
        path, _, _anchor = target.partition("#")
        if not path:
            continue
        resolved = (page.parent / path).resolve()
        if not resolved.exists():
            problems.append(f"{page.name}: {target} does not resolve")

if problems:
    print("\n".join(problems))
    sys.exit(1)
print(f"{len(list(SRC.glob('*.md'))) - 1} chapters, every link resolves")
