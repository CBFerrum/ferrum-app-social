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
        # (round five) `meta.label` names it apart in plain words (two Instagram sign-ins, two TikToks); the network's own
        # name stays `meta.social.name`.
        label = d.get("meta", {}).get("label") or d.get("meta", {}).get("social", {}).get("name") or name.replace("_", " ").title()
        entries.append({"name": name, "label": label, "description": d})

block = "connections:\n" + "".join("  - " + json.dumps(e, ensure_ascii=False) + "\n" for e in entries)
text = package.read_text()
start = text.index("\nconnections:\n") + 1
end = re.search(r"\n(?=\S)", text[start + len("connections:\n"):])
end = start + len("connections:\n") + (end.start() + 1 if end else len(text))
text = text[:start] + block + text[end:]

# The network profiles 0.6 seeded, kept for the accounts linked to them by name: each is its network's description's own
# rules (`social`), its flat fields read from them, so a profile never says other than the description.
PROFILES = {"X": "x", "Bluesky": "bluesky", "Mastodon": "mastodon", "Threads": "threads", "LinkedIn": "linkedin",
            "Facebook": "facebook", "Instagram": "instagram", "YouTube": "youtube"}
rows = []
for name, file in PROFILES.items():
    soc = json.loads((docs / f"{file}.json").read_text())["meta"]["social"]
    soc = dict(soc, name=name)
    media = soc.get("media", {})
    kinds = media.get("kinds", [])
    row = {"profile_name": name, "max_text": soc.get("text", {}).get("max") or 100000, "images": "image" in kinds, "videos": "video" in kinds,
           "media_required": bool(media.get("required")), "alt_text": media.get("alt", True), "first_comment": bool(soc.get("first_comment")),
           "threads": soc.get("follow_ups", {}).get("as") == "thread", "link_preview": bool(soc.get("link_preview")), "scheduling": bool(soc.get("scheduling"))}
    for field, value in (("link_length", soc.get("text", {}).get("link_counts")), ("max_media", media.get("max")),
                         ("max_image_bytes", (media.get("image") or {}).get("max_bytes")), ("max_video_bytes", (media.get("video") or {}).get("max_bytes"))):
        if value is not None:
            row[field] = value
    row["social"] = soc
    rows.append(row)
seed = "  - entity: SocialProfile\n    key: [profile_name]\n    records:\n" + "".join("      - " + json.dumps(r, ensure_ascii=False) + "\n" for r in rows)
a = text.index("  - entity: SocialProfile\n")
b = text.index("\n\n", a) + 1
text = text[:a] + seed + text[b:]
package.write_text(text)
print(f"{len(entries)} descriptions shipped, {len(rows)} profiles from them")
