#!/usr/bin/env python3
"""Merge CityJSON 2.0 documents into one, with exact integer vertex arithmetic.

The benchmark corpus's two derived sources are built with this script
(`benchmark/formats/corpus_urls.txt` records the exact invocations): Tokyo from
the 21 PLATEAU tiles of Chiyoda-ku, Montréal from three boroughs. It exists
because neither common tool can do it losslessly: cjio serialises every float
with six decimals, so a transform scale of 1e-10 is written as 0 and texture
coordinates are truncated, and cjseq has no merge.

Method:

* every input must share `version`, `metadata.referenceSystem` and
  `transform.scale`, and carry no `extensions`;
* the merged `translate` is the per-axis minimum of the inputs' translates;
* each input's integer vertices are shifted by
  ``round((translate_i - translate_merged) / scale)``, in exact decimal
  arithmetic; the residual of that rounding is reported and must stay below
  ``--allow-residual`` integer units;
* geometry boundaries are offset by the running vertex count, texture
  references by the running texture and texture-vertex counts, material
  references by the running material count, and `GeometryInstance` template
  references by the running template count;
* CityObject ids must be unique across the inputs;
* `metadata` keeps the shared `referenceSystem`, and a `geographicalExtent`
  that is the union of the inputs' extents when every input has one.

Explicit options, each reported on stderr:

``--rescale sx,sy,sz``
    refine the shared scale before merging; each axis's input scale must be
    an integer power-of-ten multiple of the new one, so the step is exact.
``--z-scale S``
    after merging, re-quantise `z` to scale ``S`` (an integer multiple of the
    current `z` scale), rounding half up. FlatCityBuf stores vertices as
    32-bit integers, and a fine uniform scale overflows on height.
``--texture-type-from-extension``
    set a texture's missing (or ``unknown``) `type` from its image file's
    extension: ``.jpg``/``.jpeg`` -> ``JPG``, ``.png`` -> ``PNG``.

The output is compact JSON (no whitespace, UTF-8 kept as is).
"""

from __future__ import annotations

import argparse
import json
import sys
from decimal import ROUND_HALF_UP, Decimal


def exact(value) -> Decimal:
    """`value` as an exact decimal: a float through its shortest repr."""
    return Decimal(repr(value)) if isinstance(value, float) else Decimal(value)


def offset_boundaries(boundaries: list, offset: int) -> None:
    for i, item in enumerate(boundaries):
        if isinstance(item, list):
            offset_boundaries(item, offset)
        else:
            boundaries[i] = item + offset


def offset_texture_values(values: list, texture_offset: int, uv_offset: int) -> None:
    """Offsets a texture theme's `values`; each innermost list is a ring,
    ``[texture_index, uv_index, ...]`` or ``[null]``."""
    if values and all(not isinstance(item, list) for item in values):
        if values[0] is None:
            return
        values[0] += texture_offset
        for k in range(1, len(values)):
            values[k] += uv_offset
        return
    for item in values:
        if isinstance(item, list):
            offset_texture_values(item, texture_offset, uv_offset)


def offset_material_values(values: list, offset: int) -> None:
    for i, item in enumerate(values):
        if isinstance(item, list):
            offset_material_values(item, offset)
        elif item is not None:
            values[i] = item + offset


def log(message: str) -> None:
    print(message, file=sys.stderr)


def merge(docs: list[dict], rescale: str | None = None, allow_residual: float = 1e-6) -> dict:
    """The merged document. `docs` are modified in place."""
    versions = {d["version"] for d in docs}
    systems = {d.get("metadata", {}).get("referenceSystem") for d in docs}
    scales = {tuple(exact(s) for s in d["transform"]["scale"]) for d in docs}
    if len(versions) != 1:
        raise SystemExit(f"error: versions differ: {versions}")
    if len(systems) != 1:
        raise SystemExit(f"error: referenceSystem differs: {systems}")
    if len(scales) != 1:
        raise SystemExit(f"error: transform scales differ: {scales}")
    for d in docs:
        if d.get("extensions"):
            raise SystemExit("error: an input declares extensions, which this merge does not handle")
    scale = list(scales.pop())

    if rescale:
        new = [Decimal(x) for x in rescale.split(",")]
        factors = []
        for axis in range(3):
            factor = scale[axis] / new[axis]
            if factor != factor.to_integral_value() or factor < 1:
                raise SystemExit(f"error: rescale axis {axis}: {scale[axis]} / {new[axis]} is not integral")
            factors.append(int(factor))
        for d in docs:
            for v in d["vertices"]:
                v[0] *= factors[0]
                v[1] *= factors[1]
                v[2] *= factors[2]
        log(f"rescaled input integers by {factors} (scale {[str(x) for x in scale]} -> {[str(x) for x in new]})")
        scale = new
    translate = [min(exact(d["transform"]["translate"][axis]) for d in docs) for axis in range(3)]

    objects: dict = {}
    vertices: list = []
    textures: list = []
    uvs: list = []
    materials: list = []
    templates: list = []
    template_vertices: list = []
    worst_residual = Decimal(0)
    for d in docs:
        shift = []
        for axis in range(3):
            q = (exact(d["transform"]["translate"][axis]) - translate[axis]) / scale[axis]
            r = q.to_integral_value(rounding=ROUND_HALF_UP)
            worst_residual = max(worst_residual, abs(q - r))
            shift.append(int(r))
        v_off, t_off, uv_off, m_off, g_off = (
            len(vertices),
            len(textures),
            len(uvs),
            len(materials),
            len(templates),
        )
        for v in d["vertices"]:
            vertices.append([v[0] + shift[0], v[1] + shift[1], v[2] + shift[2]])
        appearance = d.get("appearance", {})
        textures += appearance.get("textures", [])
        uvs += appearance.get("vertices-texture", [])
        materials += appearance.get("materials", [])
        own_templates = d.get("geometry-templates")
        if own_templates:
            templates += own_templates["templates"]
            tv_off = len(template_vertices)
            template_vertices += own_templates["vertices-templates"]
            for template in templates[g_off:]:
                offset_boundaries(template["boundaries"], tv_off)
        for key, obj in d["CityObjects"].items():
            if key in objects:
                raise SystemExit(f"error: duplicate CityObject id across inputs: {key}")
            for geometry in obj.get("geometry", []):
                if geometry["type"] == "GeometryInstance":
                    geometry["template"] += g_off
                    offset_boundaries(geometry["boundaries"], v_off)
                    continue
                offset_boundaries(geometry["boundaries"], v_off)
                for theme in geometry.get("texture", {}).values():
                    offset_texture_values(theme["values"], t_off, uv_off)
                for theme in geometry.get("material", {}).values():
                    if "values" in theme:
                        offset_material_values(theme["values"], m_off)
                    elif theme.get("value") is not None:
                        theme["value"] += m_off
            objects[key] = obj
    if worst_residual > Decimal(str(allow_residual)):
        raise SystemExit(f"error: translate offset residual {worst_residual} units exceeds {allow_residual}")
    log(
        f"merged: {len(objects)} CityObjects, {len(vertices)} vertices, {len(textures)} textures, "
        f"{len(uvs)} UVs, {len(materials)} materials, {len(templates)} templates; "
        f"merged translate {[str(t) for t in translate]}; max translate residual {worst_residual} units"
    )

    out = {
        "type": "CityJSON",
        "version": versions.pop(),
        "transform": {"scale": [float(s) for s in scale], "translate": [float(t) for t in translate]},
    }
    metadata = {"referenceSystem": systems.pop()}
    extents = [d.get("metadata", {}).get("geographicalExtent") for d in docs]
    if all(extents):
        metadata["geographicalExtent"] = [min(e[i] for e in extents) for i in range(3)] + [
            max(e[i] for e in extents) for i in range(3, 6)
        ]
    out["metadata"] = metadata
    out["CityObjects"] = objects
    out["vertices"] = vertices
    merged_appearance = {}
    if materials:
        merged_appearance["materials"] = materials
    if textures:
        merged_appearance["textures"] = textures
        merged_appearance["vertices-texture"] = uvs
    if merged_appearance:
        out["appearance"] = merged_appearance
    if templates:
        out["geometry-templates"] = {"templates": templates, "vertices-templates": template_vertices}
    return out


def requantise_z(doc: dict, z_scale: str) -> None:
    """Re-quantises `z` to `z_scale`, rounding half up. The merged translate
    is each axis's minimum, so every integer `z` is non-negative."""
    target = Decimal(z_scale)
    current = exact(doc["transform"]["scale"][2])
    factor = target / current
    if factor != factor.to_integral_value() or factor < 1:
        raise SystemExit("error: --z-scale must be an integer multiple of the input z scale")
    factor = int(factor)
    worst = 0
    for v in doc["vertices"]:
        z = v[2]
        quantised = (2 * z + factor) // (2 * factor)
        worst = max(worst, abs(quantised * factor - z))
        v[2] = quantised
    doc["transform"]["scale"][2] = float(target)
    log(f"z re-quantised from {current} to {target}: max change {worst} input units = {Decimal(worst) * current} CRS units")


def set_texture_types(doc: dict) -> None:
    textures = doc.get("appearance", {}).get("textures", [])
    changed = 0
    for texture in textures:
        if texture.get("type") in (None, "unknown"):
            extension = texture["image"].rsplit(".", 1)[-1].lower()
            texture["type"] = {"jpg": "JPG", "jpeg": "JPG", "png": "PNG"}[extension]
            changed += 1
    log(f"texture type set from file extension on {changed} of {len(textures)} textures")


def main(argv: list[str] | None = None) -> None:
    parser = argparse.ArgumentParser(description="Merge CityJSON 2.0 documents exactly.")
    parser.add_argument("inputs", nargs="+")
    parser.add_argument("-o", "--output", required=True)
    parser.add_argument("--z-scale", default=None)
    parser.add_argument("--texture-type-from-extension", action="store_true")
    parser.add_argument("--rescale", default=None, help="sx,sy,sz: refine the shared scale before merging")
    parser.add_argument(
        "--allow-residual",
        type=float,
        default=1e-6,
        help="largest |residual|, in integer units, of a translate offset",
    )
    args = parser.parse_args(argv)

    docs = []
    for path in args.inputs:
        with open(path, encoding="utf-8") as handle:
            docs.append(json.load(handle))
        log(f"read {path}: {len(docs[-1]['CityObjects'])} CityObjects, {len(docs[-1]['vertices'])} vertices")
    out = merge(docs, args.rescale, args.allow_residual)
    if args.z_scale:
        requantise_z(out, args.z_scale)
    if args.texture_type_from_extension:
        set_texture_types(out)
    for axis in range(3):
        values = [v[axis] for v in out["vertices"]]
        log(f"axis {axis}: integer range [{min(values)}, {max(values)}] (i32 max 2147483647)")
    with open(args.output, "w", encoding="utf-8") as handle:
        json.dump(out, handle, separators=(",", ":"), ensure_ascii=False)
    log(f"wrote {args.output}")


if __name__ == "__main__":
    main()
