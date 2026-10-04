"""Extract the match-489 `/setting` JSON from the Issue #243 body.

Usage: python3 target/abl/extract_setting.py <issue-body.md> <out.json>

Policy:
1. The paste is used EXACTLY as-is when it parses. No reformatting, no
   padding, no guessing — a wrong board would invalidate the ablation.
2. Only when the paste does not parse do we look for a purely *syntactic*
   artefact (a stray trailing brace left by the markdown paste) and report
   what we did. Row widths are always printed so a truncated board is
   caught here rather than inside the importer.
3. Anything else (wrong row widths, bad values) fails loudly.
"""

import json
import re
import sys


def row_report(data: dict) -> str:
    rows = data.get("map", {}).get("cells", [])
    widths = [len(r) for r in rows]
    want = data.get("map", {}).get("width")
    ok = bool(widths) and all(w == want for w in widths)
    return (
        f"rows={len(widths)} widths={sorted(set(widths))} "
        f"declared width={want} all_rows_match={ok}"
    )


def main(path: str, out: str) -> int:
    text = open(path).read()
    m = re.search(r"```json\n(.*?)\n```", text, re.S)
    if not m:
        print("ERROR: no ```json fence in the issue body", file=sys.stderr)
        return 1
    raw = m.group(1)
    print(f"paste bytes={len(raw)} open={raw.count('{')} close={raw.count('}')}")

    try:
        data = json.loads(raw)
        print("parse: exact paste parsed as-is (no modification)")
    except json.JSONDecodeError as first_err:
        # Only tolerate a *purely syntactic* paste artefact: a trailing '}'
        # that makes the text over-closed. Anything else aborts.
        try:
            data, end = json.JSONDecoder().raw_decode(raw)
        except json.JSONDecodeError:
            print(f"parse: FAILED ({first_err})", file=sys.stderr)
            return 1
        tail = raw[end:]
        if tail.strip(" \t\r\n}"):
            print(f"parse: FAILED ({first_err}); unparsable tail {tail!r}", file=sys.stderr)
            return 1
        # The tail is whitespace + braces only (checked above), so count the
        # braces themselves: `len(tail.strip())` also counted the whitespace
        # between them and over-reported the size of the repair.
        print(
            f"parse: OK after dropping a stray trailing brace "
            f"({tail.count('}')} extra '}}' in the tail) — "
            f"first error was: {first_err}"
        )

    print("board:", row_report(data))
    widths = [len(r) for r in data["map"]["cells"]]
    if any(w != data["map"]["width"] for w in widths):
        print("ERROR: ragged rows — board is truncated/corrupted, NOT writing", file=sys.stderr)
        return 2

    json.dump(data, open(out, "w"))
    print(
        f"wrote {out}: days={len(data['daySteps'])} fuel={data['fuelLimits']} "
        f"size={data['map']['width']}x{data['map']['height']} "
        f"spots={len(data['spots'])} agents={len(data['agents'])} "
        f"brands={len({s['brand'] for s in data['spots']})} players={data['players']}"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1], sys.argv[2]))
