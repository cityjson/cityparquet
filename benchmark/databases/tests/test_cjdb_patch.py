"""Proves the cjdb ground-surfaces patch itself works — independent of any
CjdbSystem plumbing (see test_cjdb_adapter.py for that, and
test_cjdb_integration.py for a real end-to-end import) and of a live
PostgreSQL container.

Calls `cjdb.modules.geometric.get_ground_surfaces` directly, in a
subprocess built from the SAME patched source `CjdbSystem.ingest()` drives
(`patched_cjdb_source()`), against two synthetic, non-overlapping
horizontal polygons that share a mean Z — exactly the tie condition
`vendor/cjdb/README.md` and the Task 12 fix report describe (verified
against delft: 9 of 1116 BuildingParts hit this for real). An unpatched
cjdb silently drops one of the two (the second overwrites the first in the
buggy dict); the patched cjdb must keep both.

Marked `integration`: not because it needs Docker, but because it needs
`scripts/patch_cjdb.sh` (`just patch-cjdb`) to have already built the
patched source — a one-time, network-dependent step outside a plain unit
test's scope, in the same spirit as this project's other `integration`
tests needing `just up` first.
"""

from __future__ import annotations

import json
import subprocess

import pytest

from citybench.systems.cjdb import patched_cjdb_source

pytestmark = pytest.mark.integration

# Two horizontal (non-vertical), non-overlapping unit squares, BOTH at
# z=5 — the exact tie condition the patch exists for. A third, higher
# polygon (z=15) establishes a genuine z-mean split (mean of the two
# DISTINCT z values, 5 and 15, is 10), so both z=5 squares land below the
# mean and are exactly what get_ground_surfaces is expected to return —
# mirroring the algorithm's own "ground vs roof" split, not a contrived
# shape that only exercises the tie in isolation.
_SCRIPT = """
import json
from shapely.geometry import Polygon
from cjdb.modules.geometric import get_ground_surfaces

a = Polygon([(0, 0, 5), (1, 0, 5), (1, 1, 5), (0, 1, 5)])
b = Polygon([(10, 10, 5), (11, 10, 5), (11, 11, 5), (10, 11, 5)])
c = Polygon([(0, 0, 15), (1, 0, 15), (1, 1, 15), (0, 1, 15)])

result = get_ground_surfaces([a, b, c])
print(json.dumps({"n_returned": len(result)}))
"""


def test_patched_get_ground_surfaces_retains_both_tied_z_faces():
    source = patched_cjdb_source()
    proc = subprocess.run(
        ["uv", "run", "--with", str(source), "python", "-c", _SCRIPT],
        check=True, capture_output=True, text=True, timeout=120,
    )
    payload = json.loads(proc.stdout.strip().splitlines()[-1])

    # Confirmed empirically (see this test's own module docstring and the
    # Task 12 fix report): stock cjdb==2.2.0 returns 1 here — the second
    # tied-Z polygon silently overwrites the first in the buggy
    # `ground_surfaces[z] = ...` dict. If this ever drops back to 1, the
    # patch has been lost, reverted, or `patched_cjdb_source()` is
    # resolving to an unpatched build — exactly the "unpatched
    # environment" this test exists to catch, loudly, rather than letting
    # a benchmark run silently produce stock cjdb's numbers again.
    assert payload["n_returned"] == 2, (
        f"expected both tied-Z faces retained, got {payload['n_returned']}; "
        "the cjdb patch (vendor/cjdb/ground-surfaces-tie.patch) may be "
        "missing or stale — re-run `just patch-cjdb`"
    )


def test_patched_source_directory_name_embeds_the_current_patch_hash():
    # Cheap, no-subprocess companion check: the content-addressing scheme
    # patch_cjdb.sh relies on (see cjdb.py's patched_cjdb_source()) is
    # itself part of what makes the test above trustworthy — if the
    # resolved path were stale relative to the committed patch file,
    # patched_cjdb_source() would have already raised before this line.
    source = patched_cjdb_source()
    assert source.name.startswith("cjdb-2.2.0+")
