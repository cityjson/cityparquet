"""Walk the published catalogue: collections, then the items inside them.

Item enumeration has two independent sources — the collection's
`items.parquet` (fast: DuckDB range-reads it over HTTPS, no download) and the
object-store listing (slow but complete). Both are consulted and their counts
compared, because a published index can be badly out of date: at the time of
writing, `japan-plateau-3d` publishes an `items.parquet` describing 306 of its
60,471 items. Preferring the fast path unconditionally would convert one item
in two hundred and report success.
"""

from __future__ import annotations

import contextlib
from dataclasses import dataclass
from urllib.parse import quote

from .ledger import validate_collection_id


@dataclass(frozen=True)
class Item:
    collection: str
    item_id: str
    href: str
    media_type: str | None
    source_item_url: str | None


def collection_ids(base_url: str, client) -> list[str]:
    """The ids of every child collection of the root catalogue.

    Each id is validated as a slug: it later names a ledger file, and the
    catalogue is not ours to trust.
    """
    catalog = client.get(f"{base_url}/catalog.json").json()
    ids = []
    for link in catalog.get("links", []):
        if link.get("rel") != "child":
            continue
        href = link.get("href", "")
        ids.append(validate_collection_id(href.removeprefix("./").removesuffix("/collection.json")))
    return ids


def fetch_collection(base_url: str, cid: str, client) -> dict:
    return client.get(f"{base_url}/{cid}/collection.json").json()


def list_item_objects(bucket_api: str, cid: str, client) -> list[str]:
    """Every object name under `<cid>/items/`, following every page.

    Pagination is mandatory, not an optimisation: the API caps a page at 1000
    objects and the largest collection has 60,471 items.
    """
    names: list[str] = []
    token: str | None = None
    while True:
        url = f"{bucket_api}?prefix={quote(cid + '/items/', safe='')}&maxResults=1000"
        if token:
            url += f"&pageToken={quote(token, safe='')}"
        payload = client.get(url).json()
        names.extend(obj["name"] for obj in payload.get("items", []))
        token = payload.get("nextPageToken")
        if not token:
            return names


def items_from_parquet(url: str) -> list[Item] | None:
    """Read a collection's stac-geoparquet index remotely, or `None`.

    `None` means "no usable index" for every reason alike — the file is absent
    (20 of the 53 collections publish none), unreadable, or shaped differently
    from what is read here. The caller falls back to the object listing, so a
    missing index is never fatal.

    `assets` is a struct whose keys differ per collection, so it is read
    generically as JSON rather than by a fixed schema.
    """
    import duckdb

    con = None
    try:
        con = duckdb.connect()
        # httpfs is what lets DuckDB range-read an https:// index. INSTALL is
        # attempted for a clean machine; both statements are allowed to fail,
        # since a local file needs neither and a remote read without them
        # simply raises below and yields `None`.
        for statement in ("INSTALL httpfs", "LOAD httpfs"):
            with contextlib.suppress(Exception):
                con.execute(statement)
        rows = con.execute(
            """
            SELECT id,
                   json_extract_string(to_json(assets), '$.data.href') AS href,
                   json_extract_string(to_json(assets), '$.data.type') AS media_type,
                   collection
            FROM read_parquet(?)
            """,
            [url],
        ).fetchall()
    except Exception:
        return None
    finally:
        if con is not None:
            con.close()
    return [
        Item(collection=r[3] or "", item_id=r[0], href=r[1], media_type=r[2], source_item_url=None)
        for r in rows
        if r[1]
    ]


def items_from_listing(
    base_url: str, bucket_api: str, cid: str, client, names: list[str] | None = None
) -> list[Item]:
    """Fetch every listed item document and read its single `data` asset.

    `names` lets a caller that has already paginated the listing hand it over
    rather than pay for 61 more requests on the largest collection.
    """
    items: list[Item] = []
    for name in list_item_objects(bucket_api, cid, client) if names is None else names:
        if not name.endswith(".json"):
            continue
        url = f"{base_url}/{name}"
        try:
            doc = client.get(url).json()
        except Exception:
            continue
        asset = (doc.get("assets") or {}).get("data") or {}
        href = asset.get("href")
        if not href:
            continue
        items.append(
            Item(
                collection=cid,
                item_id=doc.get("id") or name.rsplit("/", 1)[-1].removesuffix(".json"),
                href=href,
                media_type=asset.get("type"),
                source_item_url=url,
            )
        )
    return items


def items_from_collection_links(base_url: str, cid: str, collection: dict, client) -> list[Item]:
    """Last resort: the `rel=item` links the collection document carries."""
    items: list[Item] = []
    for link in collection.get("links", []):
        if link.get("rel") != "item":
            continue
        url = f"{base_url}/{cid}/{link.get('href', '').removeprefix('./')}"
        try:
            doc = client.get(url).json()
        except Exception:
            continue
        asset = (doc.get("assets") or {}).get("data") or {}
        if asset.get("href"):
            items.append(Item(cid, doc.get("id", ""), asset["href"], asset.get("type"), url))
    return items


def enumerate_items(
    base_url: str, bucket_api: str, cid: str, collection: dict, client
) -> tuple[list[Item], str | None]:
    """Return this collection's items, plus a note when the index was stale.

    Policy: use `items.parquet` only when its row count agrees with the object
    listing. On any disagreement the listing wins and the discrepancy is
    reported, so a stale index can never silently truncate a run. The note is
    returned rather than logged; the caller decides what to do with it.

    A collection that publishes nothing is not an error — 20 of the 53 hold
    only a `collection.json` — so an empty list comes back instead.
    """
    fast = items_from_parquet(f"{base_url}/{cid}/items.parquet")
    listed_names = list_item_objects(bucket_api, cid, client)
    # Only item documents count: the `items/` prefix may hold other files, and
    # counting them would fake a discrepancy.
    listed_count = sum(1 for n in listed_names if n.endswith(".json"))

    if fast is not None and listed_count and len(fast) == listed_count:
        return fast, None

    if listed_count:
        note = None
        if fast is not None and len(fast) != listed_count:
            note = (
                f"stale item index: items.parquet lists {len(fast)} item(s) "
                f"but the object listing has {listed_count}; using the listing"
            )
        return items_from_listing(base_url, bucket_api, cid, client, names=listed_names), note

    # Nothing listable: an index we cannot cross-check still beats nothing.
    if fast:
        return fast, None
    return items_from_collection_links(base_url, cid, collection, client), None
