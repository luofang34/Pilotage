#!/usr/bin/env bash
# Keep every map source's notice on screen.
#
# A source draws ground a reader looks at, and the people who surveyed that ground ask to
# be named for it. The failure is silent in both directions: a source added without a
# notice draws anyway, and a notice reworded out of the panel's provider table falls back
# to a generic line that credits nobody. Neither shows up as a crash or a failed build.
set -euo pipefail

root="${1:-$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)}"
style="$root/clients/apple-situation/Resources/SituationStyle.json"
panel="$root/clients/apple-situation/App/MapModesView.swift"
resource="$root/clients/apple-situation/App/SituationStyleResource.swift"
status=0

if ! grep -q 'static func attributions' "$resource"; then
    echo "FORBIDDEN: the notices must be read from the style document, not from a loaded map" >&2
    status=1
fi

python3 - "$style" "$panel" <<'PY' || status=1
import json
import sys

style_path, panel_path = sys.argv[1], sys.argv[2]
with open(style_path) as handle:
    sources = json.load(handle).get("sources", {})
panel = open(panel_path).read()

failed = False
for name, source in sources.items():
    notice = (source.get("attribution") or "").strip()
    if not notice:
        print(f"FORBIDDEN: style source {name!r} draws without a notice", file=sys.stderr)
        failed = True
        continue
    # The panel names one provider and says there are others. The name it looks for has
    # to still be in the notice, or the panel silently credits nobody.
    quoted = [
        part.split('"')[0]
        for part in panel.split('contains("')[1:]
    ]
    if not any(fragment and fragment in notice for fragment in quoted):
        print(
            f"FORBIDDEN: no provider name in the map modes panel matches the notice for "
            f"{name!r}, so the panel falls back to a line that credits nobody",
            file=sys.stderr,
        )
        failed = True

sys.exit(1 if failed else 0)
PY

exit "$status"
