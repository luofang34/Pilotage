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

# The panel is shown two ways and both have to survive. Beside the map it grows out of
# the control that opened it; too narrow for that, it comes up from the bottom edge at the
# full width. Losing either leaves a panel that is correct at one window size and covers
# the map it describes at the other, which no build failure reports.
app_root="$root/clients/apple-situation/App"
if ! grep -q 'horizontalSizeClass == .regular' "$app_root/PilotageSituationApp.swift"; then
    echo "FORBIDDEN: the panel must ask the platform whether there is room beside the map" >&2
    status=1
fi
for shape in 'modesGrowFromControls' 'presentationSizing(.fitted)'; do
    if ! grep -qF "$shape" "$app_root/PilotageSituationApp.swift"; then
        echo "FORBIDDEN: the panel lost one of its two presentations ($shape)" >&2
        status=1
    fi
done
if ! grep -q 'fixedWidth' "$panel"; then
    echo "FORBIDDEN: a panel brought up from the bottom edge must take the width it is given" >&2
    status=1
fi

# The close button belongs to the platform, and two modifiers are what let it draw one.
# The glass style supplies the disc, and the icon label style keeps the cross instead of
# the word the role carries. Removing either leaves something that looks like the other
# one is at fault: a bare tinted cross, or a glass button that says "Close".
if ! grep -q 'Button(role: .close, action: close)' "$panel"; then
    echo "FORBIDDEN: the panel must be closed by the platform's own close button" >&2
    status=1
fi
if ! grep -A 6 'Button(role: .close, action: close)' "$panel" | grep -qF 'buttonStyle(.glass)'; then
    echo "FORBIDDEN: the close button needs the glass style, or the role draws a bare cross on nothing" >&2
    status=1
fi
# The role carries a text label of its own, and that label is the word "Close". Either
# naming the symbol or asking for icons only keeps it off the panel; neither leaves it
# free to appear.
if ! grep -A 6 'Button(role: .close, action: close)' "$panel" \
    | grep -Eq 'labelStyle\(.iconOnly\)|systemName: "xmark"'; then
    echo "FORBIDDEN: the close button must name its symbol or ask for icons only, or it shows the word Close" >&2
    status=1
fi
if grep -A 8 'Button(role: .close' "$panel" | grep -Eq 'secondarySystemFill|background\(.*in: .circle\)'; then
    echo "FORBIDDEN: the close button must not be redrawn by hand over the platform's own" >&2
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
