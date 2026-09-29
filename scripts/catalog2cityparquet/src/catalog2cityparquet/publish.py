"""Lay converted packages out as the tree a public bucket serves.

A conversion run writes `<out>/<collection>/items/<item-id>/`, named after the
source catalogue. What gets published is shorter and stable:
`<collection>/<slug>/` — or `<collection>/` itself for a collection of one
package — with a `collection.json` per collection and a `catalog.json` over
them all. Nothing here reads a city model: the payload is hard-linked, and
only each package's STAC Item is rewritten, for the three things the layout
changes — its id, a human-readable title, and its structural links.

Which packages go where is data, in a spec file, so no dataset has code of its
own. A slug comes from a regular expression's `slug` group over the package's
directory name.
"""

from __future__ import annotations

import glob
import json
import os
import re
import shutil
from dataclasses import dataclass, field
from pathlib import Path

import yaml

from . import aggregate, discover

#: Links that describe where an Item sits in the tree; rewritten on publish.
#: Everything else — provenance above all — is carried over.
_STRUCTURAL_RELS = frozenset({"self", "collection", "parent", "root"})

#: The rewritten Item. Written fresh: a hard link would be edited in place,
#: under the source package.
_ITEM = "metadata.json"


@dataclass(frozen=True)
class CollectionSpec:
    """One published collection."""

    #: Directory and collection id in the published tree.
    name: str
    #: The source catalogue's collection id, whose title, description, licence
    #: and providers the published collection carries.
    source: str
    #: Glob naming the package directories; one match makes a flat collection.
    packages: str
    #: Regex whose `slug` group names each package; required for more than one.
    slug: str | None = None
    #: Title of a flat collection's single Item.
    title: str | None = None
    #: Overrides for the collection document itself.
    overrides: dict = field(default_factory=dict)


@dataclass(frozen=True)
class Spec:
    catalog: dict
    collections: list[CollectionSpec]
    #: Where the tree is served, for absolute `self` links; none without it.
    public_url: str | None = None


def load_spec(path: Path) -> Spec:
    """Read a publish spec (YAML)."""
    raw = yaml.safe_load(path.read_text(encoding="utf-8"))
    return Spec(
        catalog=raw.get("catalog") or {},
        public_url=raw.get("public_url"),
        collections=[CollectionSpec(**c) for c in raw.get("collections") or []],
    )


def title_from_slug(slug: str) -> str:
    """`chiyoda-ku` → `Chiyoda-ku`; `02_Valence_LOD3_1` → `02 Valence LOD3 1`."""
    text = slug.replace("_", " ")
    return text[:1].upper() + text[1:]


def _packages(spec: CollectionSpec, data_root: Path | None) -> list[Path]:
    pattern = spec.packages
    if data_root is not None and not os.path.isabs(pattern):
        pattern = str(data_root / pattern)
    return sorted(Path(p) for p in glob.glob(pattern) if (Path(p) / _ITEM).is_file())


def _relink(doc: dict, *, collection: str, flat: bool) -> None:
    up, root = (".", "..") if flat else ("..", "../..")
    links = [link for link in doc.get("links", []) if link.get("rel") not in _STRUCTURAL_RELS]
    kind = "application/json"
    links += [
        {"rel": "collection", "href": f"{up}/collection.json", "type": kind},
        {"rel": "parent", "href": f"{up}/collection.json", "type": kind},
        {"rel": "root", "href": f"{root}/catalog.json", "type": kind},
    ]
    doc["links"] = links
    doc["collection"] = collection


def _place(src: Path, dest: Path, *, item_id: str, title: str, collection: str, flat: bool):
    dest.mkdir(parents=True, exist_ok=True)
    # The whole payload, subdirectories included: a package's texture images
    # sit beside it at the relative paths its `image_uri`s name.
    for entry in src.rglob("*"):
        if entry == src / _ITEM or not entry.is_file():
            continue
        target = dest / entry.relative_to(src)
        target.parent.mkdir(parents=True, exist_ok=True)
        os.link(entry, target)
    doc = json.loads((src / _ITEM).read_text(encoding="utf-8"))
    doc["id"] = item_id
    doc.setdefault("properties", {})["title"] = title
    _relink(doc, collection=collection, flat=flat)
    (dest / _ITEM).write_text(json.dumps(doc, indent=2, ensure_ascii=False), encoding="utf-8")


def lay_out(spec: CollectionSpec, out: Path, data_root: Path | None = None) -> list[Path]:
    """Write one collection's packages under `out/<name>/`. Returns their directories.

    The collection's directory is replaced wholesale, so a republish never
    leaves a package behind that the spec no longer names. A directory without
    an Item is not a package — a half-written conversion leaves exactly that —
    and is not published.
    """
    packages = _packages(spec, data_root)
    if not packages:
        raise ValueError(f"{spec.name}: {spec.packages!r} names no package with an Item")
    root = out / spec.name
    flat = spec.slug is None
    if flat and len(packages) != 1:
        raise ValueError(
            f"{spec.name}: {len(packages)} packages need a slug pattern to tell them apart"
        )
    placements: list[tuple[Path, Path, str, str]] = []
    if flat:
        placements.append((packages[0], root, spec.name, spec.title or spec.name))
    else:
        pattern = re.compile(spec.slug)
        seen: dict[str, Path] = {}
        for pkg in packages:
            match = pattern.search(pkg.name)
            if not match or not match.group("slug"):
                raise ValueError(f"{spec.name}: the slug pattern does not match {pkg.name!r}")
            slug = match.group("slug")
            if slug in seen:
                raise ValueError(
                    f"{spec.name}: {seen[slug].name!r} and {pkg.name!r} share the slug {slug!r}"
                )
            seen[slug] = pkg
            placements.append((pkg, root / slug, slug, title_from_slug(slug)))

    _refuse_overlap([spec.name], [src for src, *_ in placements], out)
    if root.exists():
        shutil.rmtree(root)
    for src, dest, item_id, title in placements:
        _place(src, dest, item_id=item_id, title=title, collection=spec.name, flat=flat)
    return [dest for _, dest, _, _ in placements]


def _refuse_overlap(names: list[str], sources: list[Path], out: Path) -> None:
    for name in names:
        target = (out / name).resolve()
        for src in sources:
            source = src.resolve()
            if source.is_relative_to(target) or target.is_relative_to(source):
                raise ValueError(
                    f"{src} and the published {out / name} overlap; publishing would "
                    "delete a package it publishes"
                )


def check_no_overlap(spec: Spec, out: Path, data_root: Path | None = None) -> None:
    """Refuse a publish whose output overlaps any collection's sources.

    Checked across every collection before any is laid out: collections are
    replaced one at a time, and one's output may hold another's packages.
    """
    sources = [pkg for c in spec.collections for pkg in _packages(c, data_root)]
    _refuse_overlap([c.name for c in spec.collections], sources, out)


def absolutise_self_links(out: Path, public_url: str | None) -> None:
    """Make every catalogue and collection `self` link absolute, or drop it.

    STAC defines `self` as the document's absolute online location, and the
    aggregation writes `./collection.json`. Where the tree will be served is
    known only to the spec; without it, no `self` is better than a wrong one.
    """
    for path in [out / "catalog.json", *sorted(out.glob("*/collection.json"))]:
        if not path.is_file():
            continue
        doc = json.loads(path.read_text(encoding="utf-8"))
        links = []
        for link in doc.get("links", []):
            if link.get("rel") == "self":
                if public_url is None:
                    continue
                href = public_url.rstrip("/") + "/" + path.relative_to(out).as_posix()
                link = {**link, "href": href}
            links.append(link)
        doc["links"] = links
        path.write_text(json.dumps(doc, indent=2, ensure_ascii=False), encoding="utf-8")


def aggregate_tree(spec: Spec, out: Path, *, tool: Path, base_url: str, client) -> None:
    """Write each collection's `collection.json`, then the root `catalog.json`.

    A published collection carries the source collection's identity (title,
    description, licence, providers) under its own id; its extent and
    summaries are recomputed from the Items actually published.
    """
    configs = out.parent / f"{out.name}-configs"
    written: list[Path] = []
    for c in spec.collections:
        source = discover.fetch_collection(base_url, c.source, client)
        config = {**aggregate.collection_config(source), **c.overrides, "id": c.name}
        path = aggregate.write_config(config, configs / f"{c.name}.yaml")
        target = out / c.name / "collection.json"
        aggregate.update_collection(tool, out / c.name, path, target, geoparquet=False)
        written.append(target)
    catalog = aggregate.write_config(spec.catalog, configs / "catalog.yaml")
    aggregate.update_catalog(tool, written, out, catalog)
    absolutise_self_links(out, spec.public_url)
