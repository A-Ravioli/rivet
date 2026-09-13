#!/usr/bin/env python3
"""Fail if a publishable crate is missing metadata crates.io requires.

`cargo publish --dry-run` cannot run before the first release, because the
path dependencies do not exist in the registry yet. This checks the same
fields publishing would reject.
"""
import json
import subprocess
import sys

REQUIRED = ["description", "license", "repository", "readme", "rust_version"]

md = json.loads(subprocess.run(
    ["cargo", "metadata", "--no-deps", "--format-version", "1"],
    capture_output=True, text=True, check=True).stdout)

bad = []
publishable = [p for p in md["packages"] if p.get("publish") != []]
for pkg in publishable:
    for field in REQUIRED:
        if not pkg.get(field):
            bad.append(f"{pkg['name']}: missing {field}")
    if not pkg.get("keywords"):
        bad.append(f"{pkg['name']}: missing keywords")
    if len(pkg.get("keywords", [])) > 5:
        bad.append(f"{pkg['name']}: crates.io allows at most 5 keywords")
    for kw in pkg.get("keywords", []):
        if len(kw) > 20:
            bad.append(f"{pkg['name']}: keyword {kw!r} is longer than 20 characters")

if bad:
    print("\n".join(bad))
    sys.exit(1)
print(f"{len(publishable)} publishable crates carry complete metadata")
