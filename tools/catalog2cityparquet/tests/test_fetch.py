import contextlib
import gzip
import warnings
import zipfile
from pathlib import Path

import pytest

from catalog2cityparquet import fetch
from catalog2cityparquet.discover import Item

FIXTURES = Path(__file__).resolve().parents[3] / "tests" / "fixtures"


def test_sniff_recognises_zip_gzip_and_plain():
    assert fetch.sniff(b"PK\x03\x04rest") == "zip"
    assert fetch.sniff(b"\x1f\x8b\x08rest") == "gzip"
    assert fetch.sniff(b'{"type":"CityJSON"}') == "plain"


def test_a_lying_media_type_does_not_fool_normalise(tmp_path):
    # hamburg-3d advertises application/gml+xml with a .GML extension and
    # serves a 468 MB ZIP. Detection must be by content, never by the declared
    # type or the extension.
    src = FIXTURES / "b1_lod2_s.gml"
    archive = tmp_path / "lying.GML"  # extension says GML
    with zipfile.ZipFile(archive, "w") as zf:
        zf.write(src, "inner.gml")  # content is a ZIP

    found = fetch.normalise(archive, tmp_path / "work")
    assert [p.name for p in found] == ["inner.gml"]


def test_gzip_is_decompressed(tmp_path):
    src = FIXTURES / "delft.city.jsonl"
    packed = tmp_path / "t.city.json.gz"
    packed.write_bytes(gzip.compress(src.read_bytes()))

    found = fetch.normalise(packed, tmp_path / "work")
    assert len(found) == 1
    assert found[0].read_bytes() == src.read_bytes()


def test_nested_archives_are_extracted(tmp_path):
    # geobremen-bremen is a zip inside a zip; single-level extraction finds
    # nothing convertible.
    src = FIXTURES / "b1_lod2_s.gml"
    inner = tmp_path / "inner.zip"
    with zipfile.ZipFile(inner, "w") as zf:
        zf.write(src, "model.gml")
    outer = tmp_path / "outer.zip"
    with zipfile.ZipFile(outer, "w") as zf:
        zf.write(inner, "inner.zip")

    found = fetch.normalise(outer, tmp_path / "work")
    assert [p.name for p in found] == ["model.gml"]


def test_many_members_are_all_returned(tmp_path):
    # Japan's whole-city packages hold 136 GMLs that must be passed to one
    # convert invocation.
    src = FIXTURES / "b1_lod2_s.gml"
    archive = tmp_path / "bundle.zip"
    with zipfile.ZipFile(archive, "w") as zf:
        for i in range(5):
            zf.write(src, f"udx/bldg/{i}.gml")
        zf.writestr("codelists/ignore.xsd", "not convertible")
        zf.writestr("metadata/readme.pdf", "not convertible")

    found = fetch.normalise(archive, tmp_path / "work")
    assert len(found) == 5
    assert all(p.suffix == ".gml" for p in found)


def test_path_traversal_members_are_rejected(tmp_path):
    archive = tmp_path / "evil.zip"
    with zipfile.ZipFile(archive, "w") as zf:
        zf.writestr("../escape.gml", "<x/>")

    with pytest.raises(ValueError, match="unsafe member"):
        fetch.normalise(archive, tmp_path / "work")


def test_an_oversized_archive_is_refused(tmp_path):
    archive = tmp_path / "bomb.zip"
    with zipfile.ZipFile(archive, "w", zipfile.ZIP_DEFLATED) as zf:
        zf.writestr("big.gml", "0" * (1 << 20))

    with pytest.raises(ValueError, match="uncompressed size"):
        fetch.normalise(archive, tmp_path / "work", max_bytes=1024)


def test_an_archive_with_nothing_convertible_returns_empty(tmp_path):
    archive = tmp_path / "none.zip"
    with zipfile.ZipFile(archive, "w") as zf:
        zf.writestr("readme.txt", "hello")

    assert fetch.normalise(archive, tmp_path / "work") == []


def test_japan_whole_city_bundles_are_recognised():
    # The 381 *_citygml_* items repackage tiles converted separately.
    bundle = Item(
        "japan-plateau-3d", "11348_hatoyama-machi_pref_2025_citygml_1_op", "u", None, None
    )
    tile = Item("japan-plateau-3d", "48395630_bldg_6697_op", "u", None, None)
    other = Item("rotterdam-3d", "x_citygml_y", "u", None, None)

    assert fetch.is_duplicate_bundle(bundle) is True
    assert fetch.is_duplicate_bundle(tile) is False
    assert fetch.is_duplicate_bundle(other) is False


def test_a_plain_convertible_file_passes_straight_through(tmp_path):
    src = tmp_path / "plain.city.jsonl"
    src.write_bytes((FIXTURES / "empty.city.jsonl").read_bytes())

    assert fetch.normalise(src, tmp_path / "work") == [src]


def test_a_gzip_bomb_is_refused(tmp_path):
    # A gzip member carries no honest size anywhere, so the only defence is
    # counting the bytes actually written.
    packed = tmp_path / "bomb.city.json.gz"
    packed.write_bytes(gzip.compress(b"0" * (1 << 20)))

    with pytest.raises(ValueError, match="uncompressed size"):
        fetch.normalise(packed, tmp_path / "work", max_bytes=1024)


def test_the_size_budget_spans_every_archive_in_one_payload(tmp_path):
    # Each inner archive is individually within the cap; together they are not.
    # A per-archive budget would let a payload multiply its way past the limit.
    blob = b"0" * 4096
    inners = []
    for name in ("a", "b", "c"):
        inner = tmp_path / f"{name}.zip"
        with zipfile.ZipFile(inner, "w", zipfile.ZIP_DEFLATED) as zf:
            zf.writestr(f"{name}.gml", blob)
        inners.append(inner)
    outer = tmp_path / "outer.zip"
    with zipfile.ZipFile(outer, "w", zipfile.ZIP_DEFLATED) as zf:
        for inner in inners:
            zf.write(inner, inner.name)

    with pytest.raises(ValueError, match="uncompressed size"):
        fetch.normalise(outer, tmp_path / "work", max_bytes=8192)


def test_archives_nested_beyond_the_depth_limit_are_refused(tmp_path):
    # Refusing is deliberate: quietly skipping the unopened archive would
    # report a partial conversion as a complete one.
    src = FIXTURES / "b1_lod2_s.gml"
    inner = tmp_path / "inner.zip"
    with zipfile.ZipFile(inner, "w") as zf:
        zf.write(src, "model.gml")
    outer = tmp_path / "outer.zip"
    with zipfile.ZipFile(outer, "w") as zf:
        zf.write(inner, "inner.zip")

    with pytest.raises(ValueError, match="nesting depth"):
        fetch.normalise(outer, tmp_path / "work", max_depth=1)


def test_like_named_members_of_different_archives_do_not_overwrite(tmp_path):
    # Japan's packages repeat filenames across subdirectories; unpacking two
    # like-named archives into one directory would lose data without a word.
    src = FIXTURES / "b1_lod2_s.gml"
    inner = tmp_path / "model.zip"
    with zipfile.ZipFile(inner, "w") as zf:
        zf.write(src, "data.gml")
    outer = tmp_path / "outer.zip"
    with zipfile.ZipFile(outer, "w") as zf:
        zf.write(inner, "a/model.zip")
        zf.write(inner, "b/model.zip")

    found = fetch.normalise(outer, tmp_path / "work")
    assert len(found) == 2
    assert len({p.parent for p in found}) == 2


def _office_document(path: Path) -> Path:
    """A minimal OOXML file: a ZIP by content, a document by every other measure."""
    with zipfile.ZipFile(path, "w") as zf:
        zf.writestr("[Content_Types].xml", "<Types/>")
        zf.writestr("xl/workbook.xml", "<workbook/>")
    return path


def test_duplicate_member_names_within_one_archive_are_refused(tmp_path):
    # ZIP permits a repeated entry name. Extracting both into one directory
    # loses the first payload while the returned list still looks right.
    archive = tmp_path / "dupes.zip"
    with warnings.catch_warnings():
        # zipfile warns when *writing* a repeated name; reading one is silent,
        # which is the whole problem.
        warnings.simplefilter("ignore", UserWarning)
        with zipfile.ZipFile(archive, "w") as zf:
            zf.writestr("udx/1.gml", "<first/>")
            zf.writestr("udx/1.gml", "<second/>")

    with pytest.raises(ValueError, match="duplicate member"):
        fetch.normalise(archive, tmp_path / "work")


def test_office_documents_beside_the_data_are_not_unpacked(tmp_path):
    # PLATEAU and several German packages ship .xlsx/.docx metadata beside the
    # GML. Those begin with PK\x03\x04 too, so a pure content sniff would feed
    # the converter their OOXML parts and fail the whole item.
    src = FIXTURES / "b1_lod2_s.gml"
    archive = tmp_path / "pkg.zip"
    doc = _office_document(tmp_path / "book.xlsx")
    with zipfile.ZipFile(archive, "w") as zf:
        zf.write(src, "udx/bldg/real.gml")
        zf.write(doc, "metadata/book.xlsx")

    found = fetch.normalise(archive, tmp_path / "work")
    assert [p.name for p in found] == ["real.gml"]


def test_a_document_deeper_than_the_limit_does_not_abort_the_payload(tmp_path):
    # An innocuous spreadsheet three levels down must not discard the GML that
    # was already found.
    src = FIXTURES / "b1_lod2_s.gml"
    doc = _office_document(tmp_path / "book.xlsx")
    level1 = tmp_path / "l1.zip"
    with zipfile.ZipFile(level1, "w") as zf:
        zf.write(doc, "book.xlsx")
    level2 = tmp_path / "l2.zip"
    with zipfile.ZipFile(level2, "w") as zf:
        zf.write(level1, "l1.zip")
    outer = tmp_path / "l3.zip"
    with zipfile.ZipFile(outer, "w") as zf:
        zf.write(src, "good.gml")
        zf.write(level2, "l2.zip")

    found = fetch.normalise(outer, tmp_path / "work")
    assert [p.name for p in found] == ["good.gml"]


def test_an_over_deep_archive_holding_nothing_convertible_is_skipped(tmp_path):
    src = FIXTURES / "b1_lod2_s.gml"
    inner = tmp_path / "inner.zip"
    with zipfile.ZipFile(inner, "w") as zf:
        zf.writestr("readme.txt", "nothing to convert")
    outer = tmp_path / "outer.zip"
    with zipfile.ZipFile(outer, "w") as zf:
        zf.write(src, "good.gml")
        zf.write(inner, "inner.zip")

    found = fetch.normalise(outer, tmp_path / "work", max_depth=1)
    assert [p.name for p in found] == ["good.gml"]


def test_an_over_deep_archive_that_could_contribute_still_raises(tmp_path):
    # Silently dropping this one would report a partial conversion as complete.
    src = FIXTURES / "b1_lod2_s.gml"
    inner = tmp_path / "inner.zip"
    with zipfile.ZipFile(inner, "w") as zf:
        zf.write(src, "model.gml")
    outer = tmp_path / "outer.zip"
    with zipfile.ZipFile(outer, "w") as zf:
        zf.write(src, "good.gml")
        zf.write(inner, "inner.zip")

    with pytest.raises(ValueError, match="nesting depth"):
        fetch.normalise(outer, tmp_path / "work", max_depth=1)


def test_the_declared_size_refusal_writes_nothing_at_all(tmp_path):
    # The header check earns its place by refusing before any byte is written;
    # without it the budget would still refuse, but only after filling the disk
    # up to the cap.
    archive = tmp_path / "many.zip"
    with zipfile.ZipFile(archive, "w", zipfile.ZIP_DEFLATED) as zf:
        for i in range(50):
            zf.writestr(f"{i}.gml", "0" * 100_000)
    workdir = tmp_path / "work"

    with pytest.raises(ValueError, match="declared uncompressed size"):
        fetch.normalise(archive, workdir, max_bytes=1 << 20)

    assert [p for p in workdir.rglob("*") if p.is_file()] == []


class _FakeResponse:
    def __init__(self, chunks, error=None):
        self._chunks = chunks
        self._error = error
        self.chunk_sizes = []

    def raise_for_status(self):
        if self._error is not None:
            raise self._error

    def iter_bytes(self, size=None):
        self.chunk_sizes.append(size)
        yield from self._chunks


class _FakeClient:
    """Enough of `httpx.Client` for `download`; the suite never uses a socket."""

    def __init__(self, chunks, error=None):
        self.response = _FakeResponse(chunks, error)
        self.calls = []

    @contextlib.contextmanager
    def stream(self, method, url, **kwargs):
        self.calls.append((method, url, kwargs))
        yield self.response


def test_download_streams_to_disk_and_reports_the_byte_count(tmp_path):
    client = _FakeClient([b"abc", b"defg"])
    dest = tmp_path / "nested" / "out.bin"

    written = fetch.download("https://example.invalid/x", dest, client, timeout=12.0)

    assert written == 7
    assert dest.read_bytes() == b"abcdefg"
    # Streamed in bounded chunks: some of these payloads exceed a gigabyte.
    assert client.response.chunk_sizes == [1 << 20]


def test_download_sends_a_browser_user_agent_and_follows_redirects(tmp_path):
    # montreal-3d returns 403 to a default client.
    client = _FakeClient([b"x"])

    fetch.download("https://example.invalid/x", tmp_path / "out.bin", client, timeout=5.0)

    _method, _url, kwargs = client.calls[0]
    assert "Mozilla" in kwargs["headers"]["User-Agent"]
    assert kwargs["follow_redirects"] is True
    assert kwargs["timeout"] == 5.0


def test_download_lets_a_failed_response_propagate(tmp_path):
    # Retries belong to the orchestrator; this must not swallow the failure.
    client = _FakeClient([b"x"], error=RuntimeError("403 Forbidden"))

    with pytest.raises(RuntimeError, match="403"):
        fetch.download("https://example.invalid/x", tmp_path / "out.bin", client, timeout=5.0)


def test_local_name_prefers_the_query_filename_when_the_path_has_none():
    # estonia-3d publishes query-string hrefs whose real name is in `f=`; a
    # download saved without a suffix would look unconvertible afterwards.
    url = "https://example.invalid/api/download?ds=ehr&f=lod2_44_tartu.gml&fmt=citygml"
    assert fetch.local_name(url) == "lod2_44_tartu.gml"


def test_local_name_prefers_the_query_over_a_script_endpoint_name():
    # The commonest shape: the path ends in a handler, not a filename. Saving
    # a CityJSON as `dl.ashx` would have `normalise` discard it as unconvertible.
    assert fetch.local_name("https://example.invalid/api/dl.ashx?f=lod2_tartu.gml") == (
        "lod2_tartu.gml"
    )
    assert fetch.local_name("https://example.invalid/download.php?f=city.json") == "city.json"


def test_local_name_keeps_an_archive_name_from_the_path():
    assert fetch.local_name("https://example.invalid/d/city.zip?token=1") == "city.zip"
    assert fetch.local_name("https://example.invalid/d/tile.json.gz") == "tile.json.gz"


def test_local_name_uses_the_path_when_there_is_one():
    assert fetch.local_name("https://example.invalid/a/b/delft.city.jsonl?token=1") == (
        "delft.city.jsonl"
    )


def test_local_name_never_escapes_its_directory():
    assert fetch.local_name("https://example.invalid/x?f=../../etc/passwd") == "passwd"
    assert fetch.local_name("https://example.invalid/") == "download"
