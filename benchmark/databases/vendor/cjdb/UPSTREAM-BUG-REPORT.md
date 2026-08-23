# Draft upstream bug report — cjdb `get_ground_surfaces()`

cjdb is a TU Delft 3D geoinformation project, the same group as this
paper's work — written up here in case the user wants to file it upstream
(`https://github.com/tudelft3d/cjdb`).

---

**Title**: `get_ground_surfaces()` silently drops footprint faces that
share a mean Z height

**Summary**: `cjdb/modules/geometric.py`'s `get_ground_surfaces()`
collects an object's candidate ground/roof faces into a `dict` keyed by
each face's own mean Z coordinate:

```python
ground_surfaces = {}
for polygon in polygons:
    ...
    z = mean([point[2] for point in polygon.exterior.coords])
    ground_surfaces[z] = force_2d(polygon)
```

Any two non-vertical faces that happen to share a mean Z overwrite one
another in this dict — only the last one processed survives. This is not
a contrived edge case: it is the ordinary shape of a building whose lowest
LoD has more than one ground- or roof-level face at the same height (a
flat, multi-part footprint; a flat roof split into several polygons). The
resulting `ground_geometry` can therefore be missing part, or in some
cases most, of the object's true footprint, silently — no error, no
warning, just a smaller-than-correct polygon.

**Minimal reproduction**:

```python
from shapely.geometry import Polygon
from cjdb.modules.geometric import get_ground_surfaces

# Two non-overlapping unit squares, both at z=5 (the tie), plus a third,
# higher polygon establishing a genuine ground/roof split so the split
# threshold (the mean of the two DISTINCT z values, 5 and 15, is 10)
# keeps both z=5 squares and drops the z=15 one, as intended.
a = Polygon([(0, 0, 5), (1, 0, 5), (1, 1, 5), (0, 1, 5)])
b = Polygon([(10, 10, 5), (11, 10, 5), (11, 11, 5), (10, 11, 5)])
c = Polygon([(0, 0, 15), (1, 0, 15), (1, 1, 15), (0, 1, 15)])

result = get_ground_surfaces([a, b, c])
print(len(result))  # prints 1 -- should be 2; `b` silently overwrote `a`
```

Reproduced against real CityJSON data too: on the delft LoD1/LoD2
BuildingPart fixture published at
`https://storage.googleapis.com/cityjson/delft.city.jsonl` (2231
CityObjects), 9 of the 1116 BuildingParts whose minimum-LoD geometry
enters this function have two or more candidate faces sharing a mean Z;
one loses up to 8 of them.

**Suggested fix**: accumulate candidates into a `list` of `(z, polygon)`
pairs instead of a `dict` keyed by `z`, so a tie is appended rather than
overwriting. Keep the split threshold (`z_mean`) as the mean of *distinct*
z values — matching the current `mean(ground_surfaces.keys())` exactly,
since a dict's keys are unique by construction — rather than switching to
a mean over every retained pair, which would be a separate, unrelated
semantic change (a count-weighted threshold) bundled into the same fix:

```python
def get_ground_surfaces(polygons: List[Polygon]) -> List[Polygon]:
    ground_surfaces = []
    for polygon in polygons:
        if not is_valid(polygon):
            logger.debug("Invalid polygon found while extracting ground surfaces. Skipping")
            logger.debug(is_valid_reason(polygon))
            continue
        xyz = np.asarray(polygon.exterior.coords)[0:-1]
        normal, is_coplanar = get_normal_newell(xyz)
        if is_surface_vertical(normal):
            continue
        else:
            z = mean([point[2] for point in polygon.exterior.coords])
            ground_surfaces.append((z, force_2d(polygon)))
    if len(ground_surfaces) == 0:
        return []
    z_mean = mean({z for z, _ in ground_surfaces})
    return [polygon for z, polygon in ground_surfaces if z < z_mean]
```

A working patch against `cjdb==2.2.0` implementing exactly this is
available at `benchmark/databases/vendor/cjdb/ground-surfaces-tie.patch` in the
cityparquet-paper repository, along with a minimal regression test
(`benchmark/databases/tests/test_cjdb_patch.py`).

---

## Fix report (round 4): correcting an overclaim about 3DCityDB's LoD scoping

Round 3's `semantic-surface` any-LoD justification claimed 3DCityDB v5's
schema offers **no** queryable way to scope a RoofSurface link to one
