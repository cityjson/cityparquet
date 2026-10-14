"""Independent semantic comparator for CityJSON-family datasets.

    python cjcompare.py SOURCE OUTPUT [--tol 0.001] [--uv-tol 1e-9] [--json report.json] [--pair-unmatched]

Compares SOURCE (the original) with OUTPUT (what a tool chain gave back) and
reports, per category, how many city objects agree. Both sides are read by
this file alone: no cjseq, no cityparquet-rs, no DuckDB. That independence is
the point. The Rust comparator (`cityparquet compare`) shares its parser with
the writer it checks; this one shares nothing, so the two oracles can only
agree on a wrong answer by making the same mistake twice.

Inputs: CityJSON (`.city.json`, `.json`), CityJSONSeq (`.city.jsonl`,
`.jsonl`) and FlatCityBuf (`.fcb`, through flatcitybuf's pure-Python reader).
CityGML is compared by converting both sides with citygml-tools first (see
run.py); this file never parses GML.

Everything is compared as a model, never as a document:

- vertices are dereferenced to real coordinates and compared within a
  per-axis tolerance, so the two sides' transforms and vertex pools are
  irrelevant;
- a ring's closing repeat is stripped and its start is rotated to the
  other side's start, because neither is part of the model; a reversed
  ring is still a difference;
- semantic surfaces, materials and textures are resolved per face (per ring
  for textures) to the objects they point at, so palette order and ids are
  irrelevant, and CityJSON's shorthands (a `null` for a whole shell, a
  material `value` for a whole geometry) are expanded first;
- a GeometryInstance is compared as its template's geometry plus the
  transformation matrix and the reference point.

Each difference is classified. `fail` is a loss or a change. `admitted` is a
loss the specification allows: a row of the round-trip table
(01-dataset-package.mdx, "Round-trip losslessness"), or a ring of fewer than
three distinct positions, which 03-geometry-semantics ("Ring closure") lets a
writer drop. `noted` changes no model content but is not a stated rule (a
ring that starts at another vertex, objects paired by position because the
source has no ids); the report shows it so that a reader can decide.

Exit status: 0 when nothing failed, 1 when something did, 2 on a usage or
read error.
"""

from __future__ import annotations

import argparse
import datetime as dt
import json
import math
import os
import sys
from collections import Counter, defaultdict
from dataclasses import dataclass, field
from typing import Any, Iterator

# --------------------------------------------------------------------------
# Reading
# --------------------------------------------------------------------------

# Address members the package keeps: one per field of the `address` struct
# (02-object-table-schema.mdx, "Addresses"), under the CityJSON member names
# of the CityJSON specification's own address example. The specification
# names the fields but not the CityJSON members they come from; these are the
# names both writers use. Anything else in a source address is an admitted
# loss (the "Addresses" row of the round-trip table).
KEPT_ADDRESS_MEMBERS = {
    "thoroughfareName",
    "thoroughfareNumber",
    "postBox",
    "postcode",
    "locality",
    "administrativeArea",
    "country",
    "freeText",
    "location",
}


# CityObject members CityJSON 2.0 defines. `geographicalExtent` is among them
# and is not compared: a package's `bbox` is the union over the object's
# subtree and the source's declared extent, so an export may write a wider
# box than the source held.
CORE_MEMBERS = {
    "type",
    "geometry",
    "attributes",
    "children",
    "parents",
    "children_roles",
    "address",
    "geographicalExtent",
}


@dataclass
class Chunk:
    """One self-contained unit of a dataset: a whole CityJSON document, or one
    CityJSONSeq/FlatCityBuf feature together with the header it needs."""

    objects: dict[str, Any]
    vertices: list[list[float]]
    appearance: dict[str, Any]
    templates: list[Any]
    template_vertices: list[list[float]]


@dataclass
class Dataset:
    path: str
    kind: str
    header: dict[str, Any]
    scale: list[float] | None
    chunks: Iterator[Chunk] = field(repr=False, default=iter(()))


def _deref_vertices(raw: list, transform: dict | None) -> list[list[float]]:
    if not transform:
        return [[float(c) for c in v] for v in raw]
    s, t = transform["scale"], transform["translate"]
    return [[v[0] * s[0] + t[0], v[1] * s[1] + t[1], v[2] * s[2] + t[2]] for v in raw]


def _templates(header: dict) -> tuple[list, list]:
    gt = header.get("geometry-templates") or {}
    return gt.get("templates", []), [
        [float(c) for c in v] for v in gt.get("vertices-templates", [])
    ]


def _kind(path: str) -> str:
    p = path.lower()
    if p.endswith(".fcb"):
        return "fcb"
    if p.endswith(".jsonl"):
        return "seq"
    if p.endswith(".json"):
        return "doc"
    raise ValueError(f"cannot tell the format of {path} from its name")


def open_dataset(path: str) -> Dataset:
    kind = _kind(path)
    if kind == "doc":
        with open(path, encoding="utf-8") as f:
            d = json.load(f)
        tpl, tv = _templates(d)
        chunk = Chunk(
            d.get("CityObjects", {}),
            _deref_vertices(d.get("vertices", []), d.get("transform")),
            d.get("appearance") or {},
            tpl,
            tv,
        )
        header = {k: v for k, v in d.items() if k not in ("CityObjects", "vertices")}
        return Dataset(path, kind, header, _scale(d), iter([chunk]))
    if kind == "seq":
        f = open(path, encoding="utf-8")
        first = f.readline()
        if not first.strip():
            raise ValueError(f"{path}: empty CityJSONSeq")
        header = json.loads(first)
        return Dataset(path, kind, header, _scale(header), _seq_chunks(f, header))
    return _open_fcb(path)


def _scale(header: dict) -> list[float] | None:
    t = header.get("transform")
    return list(t["scale"]) if t else None


def _seq_chunks(f, header: dict) -> Iterator[Chunk]:
    tpl, tv = _templates(header)
    header_app = header.get("appearance") or {}
    with f:
        for line in f:
            if not line.strip():
                continue
            feat = json.loads(line)
            yield Chunk(
                feat.get("CityObjects", {}),
                _deref_vertices(feat.get("vertices", []), header.get("transform")),
                feat.get("appearance") or header_app,
                tpl,
                tv,
            )


def _open_fcb(path: str) -> Dataset:
    import flatcitybuf as fcb  # imported here: only FCB comparisons need it

    reader = fcb.FcbReader.open_file(path)
    header = fcb.to_cityjson_metadata(reader.header)
    tpl, tv = _templates(header)
    header_app = header.get("appearance") or {}

    def chunks() -> Iterator[Chunk]:
        for feature in reader.select_all():
            feat = fcb.to_cityjson_feature(feature, reader.header)
            yield Chunk(
                feat.get("CityObjects", {}),
                _deref_vertices(feat.get("vertices", []), header.get("transform")),
                feat.get("appearance") or header_app,
                tpl,
                tv,
            )

    return Dataset(path, "fcb", header, _scale(header), chunks())


# --------------------------------------------------------------------------
# Canonical model
# --------------------------------------------------------------------------
#
# A ring is a list of vertex records (xyz, {theme: uv or None}); a face is a
# list of rings; a geometry's faces are listed in boundary order with a "path"
# (solid, shell) so that shell structure is compared too.


@dataclass
class Face:
    path: tuple[int, ...]  # (solid, shell) for solids, () otherwise
    rings: list[list[tuple[tuple[float, float, float], dict[str, Any]]]]
    semantic: Any  # resolved surface dict or None
    material: dict[str, Any]  # theme -> material dict or None
    texture: dict[str, list[Any]]  # theme -> per ring: canonical texture or None


@dataclass
class Geom:
    gtype: str
    lod: str | None
    faces: list[Face] | None = None  # surface geometries
    points: list[tuple[float, float, float]] | None = None  # MultiPoint
    lines: list[list[tuple[float, float, float]]] | None = None  # MultiLineString
    instance: dict[str, Any] | None = None  # GeometryInstance
    surfaces: list[Any] | None = None  # the semantics `surfaces` array, in order


@dataclass
class Obj:
    otype: str
    parents: tuple[str, ...]
    children: tuple[str, ...]
    roles: tuple[tuple[str, Any], ...]
    attributes: dict[str, Any]
    address: list[dict[str, Any]]
    geoms: list[Geom]
    unmapped: tuple[str, ...]  # CityObject members outside the CityJSON core set


def _canon_lod(lod: Any) -> str | None:
    if lod is None:
        return None
    s = str(lod)
    try:
        f = float(s)
    except ValueError:
        return s
    return f"{f:.1f}" if "." not in s else s


def _strip_nulls(v: Any) -> Any:
    if isinstance(v, dict):
        return {k: _strip_nulls(x) for k, x in v.items() if x is not None}
    if isinstance(v, list):
        return [_strip_nulls(x) for x in v]
    return v


def _canon_surface(surfaces: list, idx: Any) -> Any:
    if idx is None:
        return None
    s = surfaces[idx]
    out = {k: v for k, v in s.items() if k not in ("parent", "children")}
    if "parent" in s and s["parent"] is not None:
        out["@parent"] = surfaces[s["parent"]].get("type")
    if s.get("children"):
        out["@children"] = len(s["children"])
    return _strip_nulls(out)


def _canon_texture_def(t: dict) -> dict:
    out = dict(t)
    if "image" in out:
        # Only the file name is the model: a writer may legitimately re-root
        # the image path, and the image bytes are not part of the encoding.
        out["image"] = os.path.basename(str(out["image"]).replace("\\", "/"))
    return _strip_nulls(out)


def _face_paths(gtype: str, boundaries: list) -> list[tuple[tuple[int, ...], list]]:
    """Every surface of a geometry, in boundary order, with its (solid, shell)
    path; each surface is its list of rings (lists of vertex indices)."""
    if gtype in ("MultiSurface", "CompositeSurface"):
        return [((), s) for s in boundaries]
    if gtype == "Solid":
        return [((sh,), s) for sh, shell in enumerate(boundaries) for s in shell]
    if gtype in ("MultiSolid", "CompositeSolid"):
        return [
            ((so, sh), s)
            for so, solid in enumerate(boundaries)
            for sh, shell in enumerate(solid)
            for s in shell
        ]
    raise ValueError(f"not a surface geometry: {gtype}")


def _per_face(gtype: str, boundaries: list, values: Any) -> list:
    """`values` (semantics or material) to one entry per surface, with the
    CityJSON null shorthand expanded."""
    if gtype in ("MultiSurface", "CompositeSurface"):
        if values is None:
            return [None] * len(boundaries)
        return [v for v in values]
    if gtype == "Solid":
        out = []
        for sh, shell in enumerate(boundaries):
            v = None if values is None else values[sh]
            out.extend([None] * len(shell) if v is None else v)
        return out
    out = []
    for so, solid in enumerate(boundaries):
        vs = None if values is None else values[so]
        for sh, shell in enumerate(solid):
            v = None if vs is None else vs[sh]
            out.extend([None] * len(shell) if v is None else v)
    return out


def _per_ring_textures(gtype: str, boundaries: list, values: Any) -> list:
    """A texture theme's `values` to one list per surface of one entry per
    ring: [tex, uv0, uv1, ...] or None."""
    faces = [s for _, s in _face_paths(gtype, boundaries)]
    if gtype in ("MultiSurface", "CompositeSurface"):
        flat = values
    elif gtype == "Solid":
        flat = [s for shell in values for s in shell]
    else:
        flat = [s for solid in values for shell in solid for s in shell]
    out = []
    for face, fv in zip(faces, flat):
        rings = []
        for r, _ in enumerate(face):
            rv = fv[r] if fv is not None and r < len(fv) else None
            rings.append(None if rv is None or rv == [None] or rv[0] is None else rv)
        out.append(rings)
    return out


def _definition(palette: list, idx: Any, what: str, canon) -> Any:
    """The appearance definition a face's index names. An index the document's
    palette does not have is kept as a marker that compares unequal to every
    definition, so a writer that drops the palette fails the textures or
    materials category instead of stopping the comparison."""
    if isinstance(idx, int) and 0 <= idx < len(palette):
        return canon(palette[idx])
    return {"__undefined__": f"{what} {idx} has no definition in the document"}


def _canon_geometry(g: dict, chunk: Chunk, template_mode: bool = False) -> Geom:
    gtype = g["type"]
    lod = _canon_lod(g.get("lod"))
    verts = chunk.template_vertices if template_mode else chunk.vertices
    app = chunk.appearance

    def xyz(i: int) -> tuple[float, float, float]:
        v = verts[i]
        return (v[0], v[1], v[2])

    if gtype == "GeometryInstance":
        tpl = chunk.templates[g["template"]]
        return Geom(
            gtype,
            lod,
            instance={
                "template": _canon_geometry(tpl, chunk, template_mode=True),
                "matrix": [float(x) for x in g.get("transformationMatrix", [])],
                "point": xyz(g["boundaries"][0]),
            },
        )
    if gtype == "MultiPoint":
        return Geom(gtype, lod, points=[xyz(i) for i in g["boundaries"]])
    if gtype == "MultiLineString":
        return Geom(gtype, lod, lines=[[xyz(i) for i in ls] for ls in g["boundaries"]])

    b = g["boundaries"]
    paths = _face_paths(gtype, b)
    sem = g.get("semantics")
    sem_vals = _per_face(gtype, b, sem.get("values") if sem else None)
    surfaces = sem.get("surfaces", []) if sem else []

    materials = app.get("materials", [])
    mat: dict[str, list] = {}
    for theme, m in (g.get("material") or {}).items():
        if "value" in m and m["value"] is not None and "values" not in m:
            mat[theme] = [m["value"]] * len(paths)  # whole-geometry shorthand
        else:
            mat[theme] = _per_face(gtype, b, m.get("values"))

    textures = app.get("textures", [])
    uvs = app.get("vertices-texture", [])
    tex: dict[str, list] = {}
    for theme, t in (g.get("texture") or {}).items():
        tex[theme] = _per_ring_textures(gtype, b, t.get("values"))

    faces: list[Face] = []
    for fi, (path, rings) in enumerate(paths):
        crings = []
        face_tex: dict[str, list] = {}
        for theme in tex:
            face_tex[theme] = []
        for ri, ring in enumerate(rings):
            recs = []
            for vi, idx in enumerate(ring):
                uvmap: dict[str, Any] = {}
                for theme, per_face in tex.items():
                    rv = per_face[fi][ri]
                    if rv is not None and vi + 1 < len(rv) and rv[vi + 1] is not None:
                        # A UV index past the document's vertices-texture is
                        # NaN, which is close to nothing.
                        k = rv[vi + 1]
                        u = uvs[k] if 0 <= k < len(uvs) else (math.nan, math.nan)
                        uvmap[theme] = (float(u[0]), float(u[1]))
                    else:
                        uvmap[theme] = None
                recs.append((xyz(idx), uvmap))
            crings.append(recs)
            for theme, per_face in tex.items():
                rv = per_face[fi][ri]
                face_tex[theme].append(None if rv is None else _definition(textures, rv[0], "texture", _canon_texture_def))
        sv = sem_vals[fi] if fi < len(sem_vals) else None
        faces.append(
            Face(
                path,
                crings,
                _canon_surface(surfaces, sv),
                {
                    theme: (None if vals[fi] is None else _definition(materials, vals[fi], "material", _strip_nulls))
                    for theme, vals in mat.items()
                },
                face_tex,
            )
        )
    canon_surfaces = [
        _strip_nulls(
            {
                **{k: v for k, v in s.items() if k not in ("parent", "children")},
                **({"@parent": s["parent"]} if s.get("parent") is not None else {}),
                **({"@children": list(s["children"])} if s.get("children") else {}),
            }
        )
        for s in surfaces
    ]
    return Geom(gtype, lod, faces=faces, surfaces=canon_surfaces if sem else None)


def _canon_address(a: dict, chunk: Chunk) -> tuple[dict, list[str]]:
    out: dict[str, Any] = {}
    dropped = []
    for k, v in a.items():
        if k == "location":
            loc = v
            out["location"] = [tuple(chunk.vertices[i]) for i in loc.get("boundaries", [])]
        elif k in KEPT_ADDRESS_MEMBERS:
            if v is not None:
                out[k] = v
        else:
            dropped.append(k)
    return out, dropped


@dataclass
class Side:
    dataset: Dataset
    objects: dict[str, Obj]
    dropped_address_members: Counter
    duplicate_ids: list[str]


def load(path: str) -> Side:
    ds = open_dataset(path)
    objects: dict[str, Obj] = {}
    dropped: Counter = Counter()
    dups: list[str] = []
    for chunk in ds.chunks:
        for oid, co in chunk.objects.items():
            if oid in objects:
                dups.append(oid)
                continue
            addr = []
            for a in co.get("address", []) or []:
                ca, d = _canon_address(a, chunk)
                addr.append(ca)
                dropped.update(d)
            roles = co.get("children_roles")
            # A member CityJSON does not define is kept by a package in `other`
            # and comes back among the attributes (01-dataset-package.mdx,
            # "Unmapped source members"), so the two are compared as one bag.
            unmapped = tuple(sorted(k for k in co if k not in CORE_MEMBERS))
            bag = dict(co.get("attributes") or {})
            for k in unmapped:
                bag.setdefault(k, co[k])
            objects[oid] = Obj(
                co["type"],
                tuple(sorted(co.get("parents", []) or [])),
                tuple(sorted(co.get("children", []) or [])),
                tuple(sorted(zip(co.get("children", []) or [], roles)))
                if roles
                else (),
                _strip_nulls(bag),
                addr,
                [_canon_geometry(g, chunk) for g in co.get("geometry", []) or []],
                unmapped,
            )
    return Side(ds, objects, dropped, dups)


# --------------------------------------------------------------------------
# Comparison
# --------------------------------------------------------------------------

CATEGORIES = [
    "objects",
    "hierarchy",
    "attributes",
    "address",
    "geometry",
    "coordinates",
    "semantics",
    "materials",
    "textures",
    "templates",
]


class Tally:
    def __init__(self) -> None:
        self.checked = Counter()
        self.fail = Counter()
        self.admitted = Counter()
        self.noted = Counter()
        self.examples: dict[str, list[str]] = defaultdict(list)
        self.max_dev = 0.0
        self.max_uv_dev = 0.0

    def record(self, status: str, cat: str, msg: str, limit: int = 8) -> None:
        getattr(self, status)[cat] += 1
        key = f"{status}:{cat}"
        if len(self.examples[key]) < limit:
            self.examples[key].append(msg)


def _close(a: tuple, b: tuple, tol: list[float]) -> bool:
    return all(abs(x - y) <= t for x, y, t in zip(a, b, tol))


def _dev(a: tuple, b: tuple) -> float:
    return max(abs(x - y) for x, y in zip(a, b))


def _strip_closure(ring: list, tol: list[float]) -> list:
    r = list(ring)
    while len(r) > 1 and _close(r[-1][0], r[0][0], tol):
        r.pop()
    return r


def _distinct(ring: list) -> int:
    # Exact, not within tolerance: two vertices a quantum apart still span area.
    return len({p for p, _ in ring})


def _align(a: list, b: list, tol: list[float]) -> tuple[list | None, int]:
    """b rotated so that it starts where a does (and by how much), or None.
    Each vertex keeps its UVs while it moves, so a writer that rotated a ring
    without its texture coordinates still fails the texture check."""
    if len(a) != len(b):
        return None, 0
    if not a:
        return b, 0
    for k in range(len(b)):
        if _close(b[k][0], a[0][0], tol):
            rot = b[k:] + b[:k]
            if all(_close(p[0], q[0], tol) for p, q in zip(a, rot)):
                return rot, k
    return None, 0


RANK = {"eq": 0, "widened": 1, "instant": 2, "ne": 3}


def _worst(results: list[str]) -> str:
    return max(results, key=RANK.__getitem__, default="eq")


def _instant(s: str) -> dt.datetime | None:
    """An ISO 8601 date-time with an explicit offset, else None."""
    if "T" not in s:
        return None
    try:
        v = dt.datetime.fromisoformat(s)
    except ValueError:
        return None
    return v if v.tzinfo is not None else None


def values_equal(a: Any, b: Any) -> str:
    """'eq'; 'widened' (equal value, different JSON type: an admitted type
    promotion); 'instant' (two date-time strings naming the same instant in
    different spellings, as a UTC-normalised TIMESTAMP column gives back); or
    'ne'."""
    if isinstance(a, bool) or isinstance(b, bool):
        return "eq" if a == b and type(a) is type(b) else _widened(a, b)
    if isinstance(a, (int, float)) and isinstance(b, (int, float)):
        if a == b:
            return "eq" if type(a) is type(b) else "widened"
        if math.isclose(float(a), float(b), rel_tol=1e-12, abs_tol=0.0):
            return "widened"
        return "ne"
    if isinstance(a, dict) and isinstance(b, dict):
        if set(a) != set(b):
            return "ne"
        return _worst([values_equal(a[k], b[k]) for k in a])
    if isinstance(a, list) and isinstance(b, list):
        if len(a) != len(b):
            return "ne"
        return _worst([values_equal(x, y) for x, y in zip(a, b)])
    if isinstance(a, str) and isinstance(b, str) and a != b:
        ia, ib = _instant(a), _instant(b)
        return "instant" if ia is not None and ib is not None and ia == ib else "ne"
    if type(a) is type(b):
        return "eq" if a == b else "ne"
    return _widened(a, b)


def _widened(a: Any, b: Any) -> str:
    """A value promoted to text or JSON keeps its value: compare the text."""
    for s, o in ((a, b), (b, a)):
        if isinstance(s, str) and not isinstance(o, str):
            try:
                if values_equal(json.loads(s), o) != "ne":
                    return "widened"
            except (ValueError, TypeError):
                pass
            if str(o) == s:
                return "widened"
    return "ne"


def compare_faces(
    oid: str, key: str, fa: list[Face], fb: list[Face], tol: list[float], uv_tol: float, t: Tally
) -> dict[str, bool]:
    """Returns which categories failed for this geometry."""
    bad = {c: False for c in ("geometry", "coordinates", "semantics", "materials", "textures")}
    # A ring of fewer than three distinct positions cannot form a WKB ring,
    # and the specification lets a writer drop it (03-geometry-semantics.mdx,
    # "Ring closure and geometric validity"). Such rings are set aside on
    # both sides, a face whose exterior ring goes takes the face with it, and
    # a difference in how many went is reported as admitted.
    def keep(faces: list[Face]) -> tuple[list[Face], int]:
        out, dropped = [], 0
        for f in faces:
            rings = [_strip_closure(r, tol) for r in f.rings]
            if not rings or _distinct(rings[0]) < 3:
                dropped += len(rings)
                continue
            idx = [0] + [r for r in range(1, len(rings)) if _distinct(rings[r]) >= 3]
            dropped += len(rings) - len(idx)
            tex = {th: [v[r] for r in idx] for th, v in f.texture.items()}
            out.append(Face(f.path, [rings[r] for r in idx], f.semantic, f.material, tex))
        return out, dropped

    fa, deg_a = keep(fa)
    fb, deg_b = keep(fb)
    if deg_a != deg_b:
        t.record(
            "admitted",
            "geometry",
            f"{oid} {key}: rings of fewer than 3 distinct positions: {deg_a} in the source, {deg_b} in the output (03-geometry-semantics, ring closure)",
        )
    if len(fa) != len(fb):
        t.record("fail", "geometry", f"{oid} {key}: {len(fa)} faces in the source, {len(fb)} in the output")
        bad["geometry"] = True
        return bad
    for i, (a, b) in enumerate(zip(fa, fb)):
        if a.path != b.path:
            t.record("fail", "geometry", f"{oid} {key} face {i}: shell path {a.path} vs {b.path}")
            bad["geometry"] = True
        if len(a.rings) != len(b.rings):
            t.record("fail", "geometry", f"{oid} {key} face {i}: {len(a.rings)} rings vs {len(b.rings)}")
            bad["geometry"] = True
            continue
        aligned = []
        for r, (ra, rb) in enumerate(zip(a.rings, b.rings)):
            al, k = _align(ra, rb, tol)
            if k:
                t.record("noted", "coordinates", f"{oid} {key} face {i} ring {r}: same ring, starting {k} vertices later")
            if al is None:
                bad["coordinates"] = True
                worst = max((_dev(p[0], q[0]) for p, q in zip(ra, rb)), default=float("nan"))
                t.record(
                    "fail",
                    "coordinates",
                    f"{oid} {key} face {i} ring {r}: {len(ra)} vs {len(rb)} vertices, no rotation matches within tolerance (pointwise max deviation {worst:.6g})",
                )
                aligned.append(None)
                continue
            for p, q in zip(ra, al):
                t.max_dev = max(t.max_dev, _dev(p[0], q[0]))
            aligned.append(al)
        if a.semantic is not None or b.semantic is not None:
            if values_equal(a.semantic, b.semantic) != "eq":
                bad["semantics"] = True
                t.record("fail", "semantics", f"{oid} {key} face {i}: {a.semantic} vs {b.semantic}")
        for theme in set(a.material) | set(b.material):
            ma, mb = a.material.get(theme), b.material.get(theme)
            if values_equal(ma, mb) == "ne":
                bad["materials"] = True
                t.record("fail", "materials", f"{oid} {key} face {i} theme {theme!r}: {ma} vs {mb}")
        for theme in set(a.texture) | set(b.texture):
            ta = a.texture.get(theme) or [None] * len(a.rings)
            tb = b.texture.get(theme) or [None] * len(b.rings)
            for r in range(len(a.rings)):
                if values_equal(ta[r], tb[r]) == "ne":
                    bad["textures"] = True
                    t.record("fail", "textures", f"{oid} {key} face {i} ring {r} theme {theme!r}: {ta[r]} vs {tb[r]}")
                    continue
                if ta[r] is None or aligned[r] is None:
                    continue
                for p, q in zip(a.rings[r], aligned[r]):
                    ua, ub = p[1].get(theme), q[1].get(theme)
                    if (ua is None) != (ub is None):
                        bad["textures"] = True
                        t.record("fail", "textures", f"{oid} {key} face {i} ring {r} theme {theme!r}: UV {ua} vs {ub}")
                        break
                    if ua is not None:
                        d = max(abs(ua[0] - ub[0]), abs(ua[1] - ub[1]))
                        t.max_uv_dev = max(t.max_uv_dev, d)
                        if d > uv_tol:
                            bad["textures"] = True
                            t.record("fail", "textures", f"{oid} {key} face {i} ring {r} theme {theme!r}: UV {ua} vs {ub}")
                            break
    return bad


# Template vertices and texture coordinates are stored unquantised on both
# sides (04-appearance-templates.mdx), so they are compared far tighter than
# the dataset's quantum.
TEMPLATE_TOL = [1e-9, 1e-9, 1e-9]


def compare_geom(oid: str, ga: Geom, gb: Geom, tol: list[float], uv_tol: float, t: Tally) -> dict[str, bool]:
    key = f"{ga.gtype}@lod{ga.lod}"
    bad = {c: False for c in ("geometry", "coordinates", "semantics", "materials", "textures", "templates")}
    if ga.gtype != gb.gtype:
        t.record("fail", "geometry", f"{oid}: {ga.gtype} at lod {ga.lod} came back as {gb.gtype}")
        bad["geometry"] = True
        return bad
    if ga.instance is not None:
        ia, ib = ga.instance, gb.instance
        inner = compare_geom(oid + " template", ia["template"], ib["template"], TEMPLATE_TOL, uv_tol, t)
        ok = not any(inner.values())
        if not all(math.isclose(x, y, rel_tol=1e-9, abs_tol=1e-9) for x, y in zip(ia["matrix"], ib["matrix"])) or len(ia["matrix"]) != len(ib["matrix"]):
            t.record("fail", "templates", f"{oid}: transformation matrix {ia['matrix']} vs {ib['matrix']}")
            ok = False
        if not _close(ia["point"], ib["point"], tol):
            t.record("fail", "templates", f"{oid}: reference point {ia['point']} vs {ib['point']}")
            ok = False
        if not ok:
            if any(inner.values()):
                t.record("fail", "templates", f"{oid}: template geometry differs ({', '.join(k for k, v in inner.items() if v)})")
            bad["templates"] = True
        return bad
    if ga.points is not None:
        if len(ga.points) != len(gb.points) or not all(_close(p, q, tol) for p, q in zip(ga.points, gb.points)):
            t.record("fail", "coordinates", f"{oid} {key}: points differ")
            bad["coordinates"] = True
        return bad
    if ga.lines is not None:
        if len(ga.lines) != len(gb.lines) or not all(
            len(la) == len(lb) and all(_close(p, q, tol) for p, q in zip(la, lb)) for la, lb in zip(ga.lines, gb.lines)
        ):
            t.record("fail", "coordinates", f"{oid} {key}: line strings differ")
            bad["coordinates"] = True
        return bad
    if ga.surfaces is not None or gb.surfaces is not None:
        # The array itself, in order, parent/child links included: a writer
        # that reorders, merges or drops surfaces breaks it even when every
        # face still resolves to an equal surface (03-geometry-semantics.mdx).
        if values_equal(ga.surfaces, gb.surfaces) != "eq":
            t.record("fail", "semantics", f"{oid} {key}: semantic surfaces array {str(ga.surfaces)[:200]} vs {str(gb.surfaces)[:200]}")
            bad["semantics"] = True
    bad.update({k: v or bad.get(k, False) for k, v in compare_faces(oid, key, ga.faces or [], gb.faces or [], tol, uv_tol, t).items()})
    return bad


def compare(
    source: str,
    output: str,
    tol: float | list[float] | None = None,
    uv_tol: float = 1e-9,
    pair_unmatched: bool = False,
) -> dict:
    a = load(source)
    b = load(output)
    t = Tally()
    if pair_unmatched:
        _pair_unmatched(a, b, t)
    if tol is None:
        scales = [s for s in (a.dataset.scale, b.dataset.scale) if s]
        tol_v = [max(s[i] for s in scales) for i in range(3)] if scales else [1e-3] * 3
    elif isinstance(tol, (int, float)):
        tol_v = [float(tol)] * 3
    else:
        tol_v = list(tol)
    tol_v = [x + 1e-9 for x in tol_v]

    types_a = Counter(o.otype for o in a.objects.values())
    types_b = Counter(o.otype for o in b.objects.values())
    missing = sorted(set(a.objects) - set(b.objects))
    extra = sorted(set(b.objects) - set(a.objects))
    for oid in missing:
        t.record("fail", "objects", f"{oid} ({a.objects[oid].otype}) missing from the output")
    for oid in extra:
        t.record("fail", "objects", f"{oid} ({b.objects[oid].otype}) only in the output")
    for oid in b.duplicate_ids:
        t.record("fail", "objects", f"{oid} appears more than once in the output")
    for oid in a.duplicate_ids:
        t.record("noted", "objects", f"{oid} appears more than once in the source")
    # Members outside the kept set are compared by count only: the output
    # may carry fewer of them (an admitted loss), never more.
    lost = a.dropped_address_members - b.dropped_address_members
    if lost:
        t.record("admitted", "address", f"address members outside the kept set dropped: {dict(lost)}")
    if b.dropped_address_members - a.dropped_address_members:
        t.record("fail", "address", f"address members only in the output: {dict(b.dropped_address_members - a.dropped_address_members)}")

    attr_types: dict[str, set[str]] = defaultdict(set)
    for o in a.objects.values():
        for k, v in o.attributes.items():
            attr_types[k].add(_json_type(v))

    per_object_fail = Counter()
    for oid in sorted(set(a.objects) & set(b.objects)):
        oa, ob = a.objects[oid], b.objects[oid]
        t.checked["objects"] += 1
        failed: set[str] = set()
        if oa.otype != ob.otype:
            t.record("fail", "objects", f"{oid}: type {oa.otype} vs {ob.otype}")
            failed.add("objects")
        t.checked["hierarchy"] += 1
        if oa.parents != ob.parents or oa.children != ob.children or oa.roles != ob.roles:
            t.record("fail", "hierarchy", f"{oid}: parents {oa.parents}/{ob.parents} children {oa.children}/{ob.children} roles {oa.roles}/{ob.roles}")
            failed.add("hierarchy")
        t.checked["attributes"] += 1
        ka, kb = set(oa.attributes), set(ob.attributes)
        diff, promoted, retyped, respelled = [], [], [], []
        for k in ka & kb:
            r = values_equal(oa.attributes[k], ob.attributes[k])
            if r == "ne":
                diff.append(k)
            elif r == "instant":
                respelled.append(k)
            elif r == "widened":
                # Only an attribute whose source values are of mixed types may
                # change type (01-dataset-package.mdx, "Attributes with mixed
                # types across objects"); a consistent one must come back as it was.
                (promoted if len(attr_types.get(k, ())) > 1 else retyped).append(k)
        if ka != kb or diff or retyped:
            t.record(
                "fail",
                "attributes",
                f"{oid}: only in source {sorted(ka - kb)[:5]}, only in output {sorted(kb - ka)[:5]}, "
                f"differing {[(k, oa.attributes[k], ob.attributes[k]) for k in sorted(diff)[:3]]}, "
                f"type changed {[(k, oa.attributes[k], ob.attributes[k]) for k in sorted(retyped)[:3]]}",
            )
            failed.add("attributes")
        else:
            if promoted:
                t.record("admitted", "attributes", f"{oid}: {sorted(promoted)[:5]} promoted to a common type, values kept")
            if respelled:
                # 02-object-table-schema.mdx, "Temporal columns": a TIMESTAMP
                # stores the instant, UTC-normalised, so the source's offset and
                # spelling are not kept. The round-trip table does not list it.
                k = sorted(respelled)[0]
                t.record(
                    "admitted",
                    "attributes",
                    f"{oid}: date-times {sorted(respelled)[:5]} come back as the same instant, respelled "
                    f"({oa.attributes[k]!r} as {ob.attributes[k]!r}; UTC-normalised TIMESTAMP, 02-object-table-schema)",
                )
        if oa.unmapped != ob.unmapped:
            t.record("noted", "attributes", f"{oid}: unmapped members {list(oa.unmapped)} in the source, {list(ob.unmapped)} in the output (compared together with the attributes)")
        if oa.address or ob.address:
            t.checked["address"] += 1
            if len(oa.address) != len(ob.address) or any(
                values_equal({k: v for k, v in x.items() if k != "location"}, {k: v for k, v in y.items() if k != "location"}) == "ne"
                or len(x.get("location", [])) != len(y.get("location", []))
                or not all(_close(p, q, tol_v) for p, q in zip(x.get("location", []), y.get("location", [])))
                for x, y in zip(oa.address, ob.address)
            ):
                t.record("fail", "address", f"{oid}: {oa.address} vs {ob.address}")
                failed.add("address")

        # One geometry per (object, LoD) and one implicit geometry per object:
        # the source's extras are admitted losses. The specification does not
        # say which one is kept, so the output's geometry is compared with the
        # source candidate of its own type and size, else with the first.
        def by_lod(geoms: list[Geom]) -> dict:
            out: dict = {}
            for g in geoms:
                out.setdefault("instance" if g.gtype == "GeometryInstance" else g.lod, []).append(g)
            return out

        def size(g: Geom) -> int:
            return len(g.faces or g.points or g.lines or [])

        ga_all, gb_all = by_lod(oa.geoms), by_lod(ob.geoms)
        ka: dict = {}
        xa: list[Geom] = []
        for k, cands in ga_all.items():
            pick = cands[0]
            if len(cands) > 1 and k in gb_all:
                out = gb_all[k][0]
                pick = next((c for c in cands if c.gtype == out.gtype and size(c) == size(out)), None) or next(
                    (c for c in cands if c.gtype == out.gtype), cands[0]
                )
            ka[k] = pick
            xa.extend(c for c in cands if c is not pick)
        kb = {k: v[0] for k, v in gb_all.items()}
        xb = [g for v in gb_all.values() for g in v[1:]]
        for g in xa:
            row = "Additional implicit geometries" if g.gtype == "GeometryInstance" else "Duplicate geometry at one (object, LoD)"
            t.record("admitted", "geometry", f"{oid}: a second {g.gtype} at lod {g.lod} dropped ({row})")
        for g in xb:
            t.record("fail", "geometry", f"{oid}: the output holds two geometries at lod {g.lod}")
            failed.add("geometry")
        for k in sorted(set(ka) | set(kb), key=str):
            if k not in kb:
                t.record("fail", "geometry", f"{oid}: {ka[k].gtype} at lod {ka[k].lod} missing from the output")
                failed.add("geometry")
                continue
            if k not in ka:
                t.record("fail", "geometry", f"{oid}: {kb[k].gtype} at lod {kb[k].lod} only in the output")
                failed.add("geometry")
                continue
            bad = compare_geom(oid, ka[k], kb[k], tol_v, uv_tol, t)
            failed.update(c for c, v in bad.items() if v)
        has_geom = bool(oa.geoms)
        for cat in ("geometry", "coordinates", "semantics", "materials", "textures", "templates"):
            if cat == "templates" and not any(g.gtype == "GeometryInstance" for g in oa.geoms):
                continue
            if cat in ("semantics",) and not any(f.semantic is not None for g in oa.geoms if g.faces for f in g.faces):
                if cat not in failed:
                    continue
            if cat == "materials" and not any(f.material for g in oa.geoms if g.faces for f in g.faces):
                if cat not in failed:
                    continue
            if cat == "textures" and not any(f.texture for g in oa.geoms if g.faces for f in g.faces):
                if cat not in failed:
                    continue
            if cat in ("geometry", "coordinates") and not has_geom:
                continue
            t.checked[cat] += 1
        for cat in failed:
            per_object_fail[cat] += 1

    crs = lambda d: (d.dataset.header.get("metadata") or {}).get("referenceSystem")
    meta = {
        "crs_source": crs(a),
        "crs_output": crs(b),
        "crs_equal": _epsg(crs(a)) == _epsg(crs(b)),
        "extensions_source": sorted((a.dataset.header.get("extensions") or {}).keys()),
        "extensions_output": sorted((b.dataset.header.get("extensions") or {}).keys()),
    }
    if not meta["crs_equal"]:
        t.record("fail", "objects", f"CRS {meta['crs_source']} vs {meta['crs_output']}")
    categories = {}
    for c in CATEGORIES:
        categories[c] = {
            "checked": t.checked[c],
            "objects_failed": per_object_fail[c],
            "fail": t.fail[c],
            "admitted": t.admitted[c],
            "noted": t.noted[c],
            "status": "fail" if t.fail[c] else ("admitted" if t.admitted[c] else ("noted" if t.noted[c] else ("pass" if t.checked[c] else "n/a"))),
        }
    return {
        "source": source,
        "output": output,
        "tolerance": tol_v,
        "uv_tolerance": uv_tol,
        "max_coordinate_deviation": t.max_dev,
        "max_uv_deviation": t.max_uv_dev,
        "objects_source": len(a.objects),
        "objects_output": len(b.objects),
        "types_source": dict(sorted(types_a.items())),
        "types_output": dict(sorted(types_b.items())),
        "metadata": meta,
        "categories": categories,
        "examples": {k: v for k, v in t.examples.items()},
        "ok": not any(t.fail.values()),
    }


def _pair_unmatched(a: Side, b: Side, t: Tally) -> None:
    """A CityGML object without a gml:id gets an id from whoever reads it,
    so the two sides name it differently. Objects found on one side only are
    paired by type and order of appearance, when each type has as many on
    both sides; the output's are renamed to the source's ids."""
    only_a = [i for i in a.objects if i not in b.objects]
    only_b = [i for i in b.objects if i not in a.objects]
    by_type_a: dict[str, list[str]] = defaultdict(list)
    by_type_b: dict[str, list[str]] = defaultdict(list)
    for i in only_a:
        by_type_a[a.objects[i].otype].append(i)
    for i in only_b:
        by_type_b[b.objects[i].otype].append(i)
    rename: dict[str, str] = {}
    for otype, ids in by_type_a.items():
        if len(ids) == len(by_type_b.get(otype, [])):
            rename.update(zip(by_type_b[otype], ids))
    if not rename:
        return
    t.record("noted", "objects", f"{len(rename)} object(s) without a common id paired by type and position, e.g. {next(iter(rename.items()))}")

    def ren(x: str) -> str:
        return rename.get(x, x)

    objects = {}
    for i, o in b.objects.items():
        o.parents = tuple(sorted(ren(x) for x in o.parents))
        o.children = tuple(sorted(ren(x) for x in o.children))
        o.roles = tuple(sorted((ren(x), r) for x, r in o.roles))
        objects[ren(i)] = o
    b.objects = objects


def _json_type(v: Any) -> str:
    if isinstance(v, bool):
        return "bool"
    if isinstance(v, int):
        return "int"
    if isinstance(v, float):
        return "float"
    if isinstance(v, str):
        return "str"
    return "list" if isinstance(v, list) else "object"


def _epsg(rs: Any) -> str | None:
    if rs is None:
        return None
    s = str(rs).rstrip("/")
    return s.rsplit("/", 1)[-1].split(":")[-1]


def main(argv: list[str] | None = None) -> int:
    p = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    p.add_argument("source")
    p.add_argument("output")
    p.add_argument("--tol", help="coordinate tolerance in CRS units, one value or x,y,z (default: one quantum of the coarser transform)")
    p.add_argument("--uv-tol", type=float, default=1e-9)
    p.add_argument("--json", help="write the full report here")
    p.add_argument("--pair-unmatched", action="store_true", help="pair objects found on one side only by type and position (CityGML without gml:id)")
    args = p.parse_args(argv)
    tol = None
    if args.tol:
        parts = [float(x) for x in args.tol.split(",")]
        tol = parts[0] if len(parts) == 1 else parts
    try:
        rep = compare(args.source, args.output, tol, args.uv_tol, args.pair_unmatched)
    except (OSError, ValueError, KeyError, IndexError) as e:
        print(f"error: {e}", file=sys.stderr)
        return 2
    if args.json:
        with open(args.json, "w", encoding="utf-8") as f:
            json.dump(rep, f, indent=1, default=str)
    print(f"{'OK  ' if rep['ok'] else 'FAIL'} {rep['objects_source']} -> {rep['objects_output']} objects, max deviation {rep['max_coordinate_deviation']:.3g}")
    for c, v in rep["categories"].items():
        if v["status"] != "n/a":
            print(f"  {c:12} {v['status']:8} checked {v['checked']}, failed objects {v['objects_failed']}, admitted {v['admitted']}, noted {v['noted']}")
    for k, ex in rep["examples"].items():
        for e in ex[:3]:
            print(f"    {k}: {e[:300]}")
    return 0 if rep["ok"] else 1


if __name__ == "__main__":
    sys.exit(main())
