#!/usr/bin/env python3
"""Writes every description under docs/connectors (networks, then services) into the package's `connections:`, so a
workspace is offered each one (turned off, no secrets). tests/connectors_it.rs fails when the two differ."""
import json, os, pathlib, re

root = pathlib.Path(__file__).resolve().parent.parent
docs = root / "docs" / "connectors"
package = root / "packages" / "social" / "package.yaml"

entries = []
for sub in ("", "services"):
    for path in sorted((docs / sub).glob("*.json")):
        d = json.loads(path.read_text())
        name = path.stem
        label = d.get("meta", {}).get("social", {}).get("name") or d.get("label") or name.replace("_", " ").title()
        entries.append({"name": name, "label": label, "description": d})

block = "connections:\n" + "".join("  - " + json.dumps(e, ensure_ascii=False) + "\n" for e in entries)
text = package.read_text()
start = text.index("\nconnections:\n") + 1
end = re.search(r"\n(?=\S)", text[start + len("connections:\n"):])
end = start + len("connections:\n") + (end.start() + 1 if end else len(text))
package.write_text(text[:start] + block + text[end:])
print(f"{len(entries)} descriptions shipped")
