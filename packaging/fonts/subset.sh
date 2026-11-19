#!/bin/sh
# Supply the three upstream static TTFs from Inter v4.1 and JetBrains Mono v2.304.
# Requires FontTools (pyftsubset) and Python 3 with the fontTools module.
set -eu
src=${1:?usage: subset.sh path/to/upstream-ttfs}
out=$(CDPATH='' cd -- "$(dirname -- "$0")/../../assets/fonts" && pwd)
ranges='U+0020-007E,U+00A0-00FF,U+2000-206F,U+2190-2193,U+2318,U+2325,U+23CE,U+25B6,U+25BE,U+2713,U+2717,U+21E7,U+2212'
for face in Inter-Regular Inter-SemiBold JetBrainsMono-Regular; do
    pyftsubset "$src/$face.ttf" --output-file="$out/$face.ttf" --unicodes="$ranges" \
        --name-IDs='*' --name-languages='*' --no-recalc-timestamp
done

# Inter is a Reserved Font Name. Give its modified subsets a distinct family name.
python3 - "$out" <<'PY'
import sys
from pathlib import Path

from fontTools.ttLib import TTFont

for name in ("Inter-Regular.ttf", "Inter-SemiBold.ttf"):
    path = Path(sys.argv[1]) / name
    with TTFont(path, recalcTimestamp=False) as font:
        for record in font["name"].names:
            if record.nameID in {1, 3, 4, 6, 16, 18, 21, 25}:
                record.string = record.toUnicode().replace("Inter", "EmuGridSans")
        font.save(path)
PY
