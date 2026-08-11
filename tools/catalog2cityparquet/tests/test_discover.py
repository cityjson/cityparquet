import duckdb
import httpx
import pytest

# pytest's default prepend import mode puts `tests/` on sys.path (it has no
# __init__.py), so the helpers are imported as top-level `conftest`, NOT as
# `tests.conftest` — the latter raises ModuleNotFoundError.
from conftest import stac_item, write_json

from catalog2cityparquet import discover


@pytest.fixture
def client():
    with httpx.Client(timeout=10) as c:
        yield c


def write_index_parquet(path, rows) -> str:
    """Write a stac-geoparquet-shaped index with DuckDB (never pyarrow)."""
    values = ", ".join(
        "('{id}', {{'data': {{'href': '{href}', 'type': '{type}'}}}}, '{collection}')".format(**row)
        for row in rows
    )
    duckdb.sql(
        f"COPY (SELECT * FROM (VALUES {values}) AS t(id, assets, collection)) "
        f"TO '{path}' (FORMAT PARQUET)"
    )
    return str(path)


def test_collection_ids_follow_child_links(served_dir, client):
    root, base = served_dir
    write_json(
        root / "catalog.json",
        {
            "type": "Catalog",
            "id": "c",
            "links": [
                {"rel": "child", "href": "./alpha/collection.json"},
                {"rel": "child", "href": "./beta/collection.json"},
                {"rel": "self", "href": "./catalog.json"},
            ],
        },
    )
    assert discover.collection_ids(base, client) == ["alpha", "beta"]


def test_listing_is_fully_paginated(served_dir, client, monkeypatch):
    # The GCS API caps a page at 1000 objects. Japan has 60,471 items, so a
    # driver that ignores nextPageToken converts 1.6% of it and reports
    # success. Two pages here prove the token is followed.
    root, base = served_dir
    write_json(
        root / "page1.json",
        {
            "items": [{"name": f"jp/items/{i}.json"} for i in range(3)],
            "nextPageToken": "TOK",
        },
    )
    write_json(root / "page2.json", {"items": [{"name": "jp/items/3.json"}]})

    calls = []

    def fake_get(url, **kwargs):
        calls.append(url)
        target = "page2.json" if "pageToken=TOK" in url else "page1.json"
        return client.get(f"{base}/{target}")

    names = discover.list_item_objects(
        bucket_api=f"{base}/o", cid="jp", client=type("C", (), {"get": staticmethod(fake_get)})()
    )
    assert names == [f"jp/items/{i}.json" for i in range(4)]
    assert len(calls) == 2, "the second page must be requested"


def test_stale_parquet_index_loses_to_the_listing(served_dir, client, monkeypatch):
    # japan-plateau-3d publishes an items.parquet listing 306 of its 60,471
    # items. Preferring the fast path there would silently convert 0.5%.
    root, base = served_dir
    listing_items = [stac_item(f"i{i}", f"{base}/data/i{i}.json") for i in range(4)]
    for item in listing_items:
        write_json(root / "jp" / "items" / f"{item['id']}.json", item)

    monkeypatch.setattr(
        discover,
        "items_from_parquet",
        lambda url: [discover.Item("jp", "i0", f"{base}/data/i0.json", None, None)],
    )
    monkeypatch.setattr(
        discover,
        "list_item_objects",
        lambda bucket_api, cid, client: [f"jp/items/i{i}.json" for i in range(4)],
    )

    items, note = discover.enumerate_items(
        base_url=base, bucket_api=f"{base}/o", cid="jp", collection={}, client=client
    )
    assert len(items) == 4, "the listing must win when the index disagrees"
    assert note is not None and "306" not in note
    assert "1" in note and "4" in note, f"the note must record both counts: {note}"


def test_matching_counts_use_the_fast_path(served_dir, client, monkeypatch):
    _root, base = served_dir
    fast = [discover.Item("x", f"i{i}", f"{base}/d/i{i}.json", None, None) for i in range(2)]
    monkeypatch.setattr(discover, "items_from_parquet", lambda url: fast)
    monkeypatch.setattr(
        discover,
        "list_item_objects",
        lambda bucket_api, cid, client: ["x/items/i0.json", "x/items/i1.json"],
    )
    items, note = discover.enumerate_items(
        base_url=base, bucket_api=f"{base}/o", cid="x", collection={}, client=client
    )
    assert items == fast
    assert note is None


def test_an_empty_collection_yields_no_items(served_dir, client, monkeypatch):
    _root, base = served_dir
    monkeypatch.setattr(discover, "items_from_parquet", lambda url: None)
    monkeypatch.setattr(discover, "list_item_objects", lambda bucket_api, cid, client: [])
    items, _note = discover.enumerate_items(
        base_url=base,
        bucket_api=f"{base}/o",
        cid="empty",
        collection={"links": []},
        client=client,
    )
    assert items == []


def test_the_listing_request_carries_the_collection_prefix(served_dir, client):
    # Without the prefix the API returns the whole bucket — every collection's
    # items would be attributed to whichever one asked first.
    root, base = served_dir
    write_json(root / "page1.json", {"items": []})
    calls = []

    def fake_get(url, **kwargs):
        calls.append(url)
        return client.get(f"{base}/page1.json")

    discover.list_item_objects(
        bucket_api=f"{base}/o", cid="jp", client=type("C", (), {"get": staticmethod(fake_get)})()
    )
    assert "prefix=jp%2Fitems%2F" in calls[0], calls[0]


def test_a_collection_id_that_could_escape_the_reports_dir_is_rejected(served_dir, client):
    # `Record.collection` is interpolated into `<collection>.jsonl`, so a child
    # link bearing a path separator must not survive discovery.
    root, base = served_dir
    write_json(
        root / "catalog.json",
        {
            "type": "Catalog",
            "id": "c",
            "links": [{"rel": "child", "href": "./../../etc/passwd/collection.json"}],
        },
    )
    with pytest.raises(ValueError, match="collection id"):
        discover.collection_ids(base, client)


def test_items_from_parquet_reads_the_generic_asset_struct(tmp_path):
    # `assets` is a struct whose keys vary per collection, so it is read as
    # JSON rather than by a fixed schema.
    path = write_index_parquet(
        tmp_path / "items.parquet",
        [
            {
                "id": "3-20-DELFSHAVEN.city",
                "href": "http://x/a.city.json",
                "type": "application/city+json",
                "collection": "rotterdam-3d",
            },
            {
                "id": "10-148-336.city.json",
                "href": "http://x/b.city.json",
                "type": "application/city+json",
                "collection": "rotterdam-3d",
            },
        ],
    )
    items = discover.items_from_parquet(path)
    assert [i.item_id for i in items] == ["3-20-DELFSHAVEN.city", "10-148-336.city.json"]
    assert items[0].href == "http://x/a.city.json"
    assert items[0].media_type == "application/city+json"
    assert items[0].collection == "rotterdam-3d"


def test_items_from_parquet_is_none_when_the_index_is_absent(tmp_path):
    # 20 of the 53 collections publish no items.parquet at all.
    assert discover.items_from_parquet(str(tmp_path / "missing.parquet")) is None


def test_items_from_parquet_is_none_for_a_malformed_file(tmp_path):
    path = tmp_path / "items.parquet"
    path.write_text("not a parquet file at all", encoding="utf-8")
    assert discover.items_from_parquet(str(path)) is None


def test_items_from_parquet_is_none_when_the_columns_are_unexpected(tmp_path):
    path = tmp_path / "items.parquet"
    duckdb.sql(f"COPY (SELECT 'i0' AS id) TO '{path}' (FORMAT PARQUET)")
    assert discover.items_from_parquet(str(path)) is None


def test_items_from_listing_ignores_objects_that_are_not_items(served_dir, client, monkeypatch):
    root, base = served_dir
    write_json(root / "jp" / "items" / "i0.json", stac_item("i0", f"{base}/data/i0.json"))
    (root / "jp" / "items" / "notes.txt").write_text("ignore me", encoding="utf-8")
    monkeypatch.setattr(
        discover,
        "list_item_objects",
        lambda bucket_api, cid, client: ["jp/items/i0.json", "jp/items/notes.txt"],
    )
    items = discover.items_from_listing(base, f"{base}/o", "jp", client)
    assert [i.item_id for i in items] == ["i0"]
    assert items[0].source_item_url == f"{base}/jp/items/i0.json"


def test_non_item_objects_do_not_count_towards_the_comparison(served_dir, client, monkeypatch):
    # The `items/` prefix may hold other files; counting them would fake a
    # discrepancy and force the slow path for every collection.
    _root, base = served_dir
    fast = [discover.Item("x", "i0", f"{base}/d/i0.json", None, None)]
    monkeypatch.setattr(discover, "items_from_parquet", lambda url: fast)
    monkeypatch.setattr(
        discover,
        "list_item_objects",
        lambda bucket_api, cid, client: ["x/items/i0.json", "x/items/README.md"],
    )
    items, note = discover.enumerate_items(
        base_url=base, bucket_api=f"{base}/o", cid="x", collection={}, client=client
    )
    assert items == fast
    assert note is None


def test_an_unlistable_collection_falls_back_to_its_parquet_index(served_dir, client, monkeypatch):
    _root, base = served_dir
    fast = [discover.Item("x", "i0", f"{base}/d/i0.json", None, None)]
    monkeypatch.setattr(discover, "items_from_parquet", lambda url: fast)
    monkeypatch.setattr(discover, "list_item_objects", lambda bucket_api, cid, client: [])
    items, note = discover.enumerate_items(
        base_url=base, bucket_api=f"{base}/o", cid="x", collection={}, client=client
    )
    assert items == fast
    assert note is None


def test_without_an_index_or_a_listing_the_collection_links_are_used(
    served_dir, client, monkeypatch
):
    root, base = served_dir
    write_json(root / "fr" / "items" / "i0.json", stac_item("i0", f"{base}/data/i0.json"))
    monkeypatch.setattr(discover, "items_from_parquet", lambda url: None)
    monkeypatch.setattr(discover, "list_item_objects", lambda bucket_api, cid, client: [])
    items, note = discover.enumerate_items(
        base_url=base,
        bucket_api=f"{base}/o",
        cid="fr",
        collection={"links": [{"rel": "item", "href": "./items/i0.json"}]},
        client=client,
    )
    assert [i.item_id for i in items] == ["i0"]
    assert note is None


def test_fetch_collection_reads_the_collection_document(served_dir, client):
    root, base = served_dir
    write_json(root / "fr" / "collection.json", {"type": "Collection", "id": "fr", "links": []})
    assert discover.fetch_collection(base, "fr", client)["id"] == "fr"
