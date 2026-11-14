#!/usr/bin/env python3
"""Check relative markdown links/anchors and reachability from AGENTS.md.

Run from the repository root: python3 tools/doclinks.py
Exit 1 when a link is broken. GitHub line anchors (#L12-L20) count as broken: they rot.
"""
import collections
import os
import re
import subprocess
import sys

files = [f for f in subprocess.check_output(["git", "ls-files", "-co", "--exclude-standard", "*.md"]).decode().split() if os.path.exists(f)]


def slug(h):
    h = re.sub(r"[`*~]", "", h.strip().lower())
    h = re.sub(r"\[([^\]]*)\]\([^)]*\)", r"\1", h)
    h = re.sub(r"[^\w\- ]", "", h)
    return h.replace(" ", "-")


heads = {}
for f in files:
    text = open(f, encoding="utf-8", errors="replace").read()
    text = re.sub(r"```.*?```", "", text, flags=re.S)
    seen = collections.Counter()
    out = set()
    for h in re.findall(r"^#{1,6}\s+(.+?)\s*#*$", text, re.M):
        s = slug(h)
        out.add(s if seen[s] == 0 else f"{s}-{seen[s]}")
        seen[s] += 1
    heads[f] = out

broken = []
graph = collections.defaultdict(set)
for f in files:
    text = open(f, encoding="utf-8", errors="replace").read()
    text = re.sub(r"```.*?```", "", text, flags=re.S)
    for m in re.finditer(r"\]\(([^)\s]+)(?:\s+\"[^\"]*\")?\)", text):
        url = m.group(1)
        if re.match(r"[a-z][a-z0-9+.-]*:", url):
            continue
        path, _, anchor = url.partition("#")
        target = os.path.normpath(os.path.join(os.path.dirname(f), path)) if path else f
        if not os.path.exists(target):
            broken.append((f, url, "missing file"))
            continue
        if target.endswith(".md"):
            graph[f].add(target)
        if anchor and target in heads and anchor not in heads[target]:
            broken.append((f, url, "missing anchor"))

reach = set()
stack = ["AGENTS.md"]
while stack:
    x = stack.pop()
    if x in reach:
        continue
    reach.add(x)
    stack.extend(graph[x])

for b in broken:
    print("BROKEN", *b)
for f in files:
    if f not in reach and f != "CLAUDE.md":
        print("UNREACHABLE", f)
print(f"{len(broken)} broken, {len([f for f in files if f not in reach and f != 'CLAUDE.md'])} unreachable, {len(files)} files")
sys.exit(1 if broken else 0)
