"""Get an item's source bytes onto disk in a form the converter accepts.

Everything here is defensive because the catalogue's hosts are not: media
types are wrong (one collection advertises `application/gml+xml` and serves a
468 MB ZIP), archives nest (one is a zip inside a zip), one origin 403s
without a browser User-Agent, one serves query-string URLs with no filename,
and PLATEAU responses omit Content-Length entirely. Format is therefore
decided by magic bytes and nothing else.

Nothing here interprets CityJSON, CityGML or Parquet — the Rust `cityparquet`
binary owns every format decision. This module only decides which bytes to
hand it.

Failures are raised, never absorbed: a hostile or unreadable payload must
reach the caller so the run's ledger records it. The one silent outcome is
`normalise` returning an empty list, which means "nothing convertible in
here" — a fact about the payload rather than a failure to read it.
"""

from __future__ import annotations

import gzip
import zipfile
from pathlib import Path, PurePosixPath
from urllib.parse import parse_qs, urlsplit

from .discover import Item

CONVERTIBLE_SUFFIXES = frozenset({".json", ".jsonl", ".gml", ".xml"})

#: Some origins refuse a default client (montreal-3d returns 403).
USER_AGENT = (
    "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) "
    "Chrome/124.0 Safari/537.36"
)

#: Read granularity for both downloading and unpacking. Some payloads are over
#: a gigabyte, so nothing is ever held whole in memory.
_CHUNK = 1 << 20

#: Query parameters that carry a filename when the URL path does not.
_FILENAME_PARAMS = ("f", "filename", "file", "name")

#: Wrappers a payload may legitimately arrive in.
_ARCHIVE_SUFFIXES = frozenset({".zip", ".gz"})

#: Suffixes that make a saved name self-describing enough for `normalise`.
_USEFUL_SUFFIXES = CONVERTIBLE_SUFFIXES | _ARCHIVE_SUFFIXES

#: Document formats that are ZIP containers underneath. PLATEAU and several
#: German packages ship these beside the GML, and unpacking one would hand the
#: converter its OOXML parts and fail the whole item.
#:
#: This is the one place where the *name* is consulted rather than the content,
#: and deliberately so: the question here is not "what format is this?" (that
#: is always answered by magic bytes) but "should this be opened at all?".
#: Restoring pure content sniffing here would reintroduce the bug.
CONTAINER_SUFFIXES = frozenset(
    {".xlsx", ".xlsm", ".docx", ".pptx", ".odt", ".ods", ".odp", ".jar", ".kmz", ".epub"}
)


def sniff(head: bytes) -> str:
    """Classify by magic bytes. Never trust a declared media type."""
    if head.startswith(b"PK\x03\x04"):
        return "zip"
    if head.startswith(b"\x1f\x8b"):
        return "gzip"
    return "plain"


def local_name(url: str, fallback: str = "download") -> str:
    """A safe local filename for `url`'s payload.

    estonia-3d publishes hrefs whose real name sits in a query parameter. The
    saved name matters beyond tidiness: `normalise` decides convertibility
    from the suffix, so a payload stored as `dl.ashx` — or with no suffix at
    all — would be discarded as unconvertible.

    The query is therefore consulted whenever the path does not already end in
    a suffix that says what the payload is, which covers the commonest shape
    of all: a path ending in a handler (`download.php`, `dl.ashx`) rather than
    a filename.
    """
    parts = urlsplit(url)
    name = PurePosixPath(parts.path).name
    if PurePosixPath(name).suffix.lower() not in _USEFUL_SUFFIXES:
        params = parse_qs(parts.query)
        for key in _FILENAME_PARAMS:
            candidates = [v for v in params.get(key, []) if v]
            if candidates:
                # A name from a query string is attacker-controlled: keep the
                # last component only, so it cannot climb out of a directory.
                candidate = PurePosixPath(candidates[0].replace("\\", "/")).name
                if candidate not in ("", ".", ".."):
                    name = candidate
                    break
    if name in ("", ".", ".."):
        return fallback
    return name


def download(url: str, dest: Path, client, timeout: float = 900.0) -> int:
    """Stream `url` to `dest`, returning the byte count.

    No pre-flight size check is possible — PLATEAU replies are chunked and
    carry no Content-Length — and no retry is attempted here: timeouts and
    retries belong to the orchestrator, so every failure propagates.
    """
    dest.parent.mkdir(parents=True, exist_ok=True)
    written = 0
    with client.stream(
        "GET",
        url,
        timeout=timeout,
        follow_redirects=True,
        headers={"User-Agent": USER_AGENT},
    ) as response:
        response.raise_for_status()
        with dest.open("wb") as fh:
            for chunk in response.iter_bytes(_CHUNK):
                fh.write(chunk)
                written += len(chunk)
    return written


def is_duplicate_bundle(item: Item) -> bool:
    """True for Japan's whole-city ZIPs, which repackage tiles we convert.

    The 381 `*_citygml_*` items contain the same data as the 60,090 per-module
    tile items; converting both would encode Japan twice. This is a fact about
    one publisher's packaging, not a general rule — hence the hard-coded
    collection id and marker, which must not be generalised to other
    collections.
    """
    return item.collection == "japan-plateau-3d" and "_citygml_" in item.item_id


class _Budget:
    """Bytes still allowed to be written while unpacking one payload.

    The budget spans the whole payload rather than each archive, because a
    per-archive cap multiplies with nesting: three inner archives each just
    under the cap would together sail past it.
    """

    __slots__ = ("limit", "remaining")

    def __init__(self, limit: int) -> None:
        self.limit = limit
        self.remaining = limit

    def spend(self, count: int, label: str) -> None:
        self.remaining -= count
        if self.remaining < 0:
            raise ValueError(f"{label}: uncompressed size exceeds the {self.limit} byte limit")


def _copy_bounded(src, dst, budget: _Budget, label: str) -> None:
    """Copy a stream, charging every byte to `budget` as it is written.

    Declared sizes are checked too, but they cannot be the only defence: a zip
    header may lie, and a gzip member declares no size at all.
    """
    while True:
        chunk = src.read(_CHUNK)
        if not chunk:
            return
        budget.spend(len(chunk), label)
        dst.write(chunk)


def _safe_extract(archive: Path, into: Path, budget: _Budget) -> list[Path]:
    into.mkdir(parents=True, exist_ok=True)
    root = into.resolve()
    extracted: list[Path] = []
    with zipfile.ZipFile(archive) as zf:
        # Cheap refusal first, from the archive's own headers. Those headers
        # are the archive's word, so `_copy_bounded` still counts what is
        # really written.
        declared = sum(info.file_size for info in zf.infolist())
        if declared > budget.remaining:
            raise ValueError(
                f"{archive.name}: declared uncompressed size {declared} exceeds "
                f"the {budget.limit} byte limit"
            )
        seen: set[Path] = set()
        for info in zf.infolist():
            if info.is_dir():
                continue
            target = (into / info.filename).resolve()
            if not target.is_relative_to(root):
                raise ValueError(f"{archive.name}: unsafe member {info.filename!r}")
            if target in seen:
                # ZIP permits a repeated entry name. Writing both to one path
                # would drop the first payload while the returned list still
                # looked the right length.
                raise ValueError(f"{archive.name}: duplicate member {info.filename!r}")
            seen.add(target)
            target.parent.mkdir(parents=True, exist_ok=True)
            # Members are always written as plain files: a member marked as a
            # symlink becomes a file holding its target path, never a link out
            # of the working directory.
            with zf.open(info) as src, target.open("wb") as dst:
                _copy_bounded(src, dst, budget, f"{archive.name}:{info.filename}")
            extracted.append(target)
    return extracted


def _is_container_document(path: Path) -> bool:
    """Whether a ZIP-by-content file is really a document that must stay shut."""
    return path.suffix.lower() in CONTAINER_SUFFIXES


def _could_contribute(archive: Path) -> bool:
    """Whether a ZIP we may not open could still have held convertible data.

    Only the central directory is read — no member is extracted — so this is
    cheap and cannot itself be a decompression bomb. It exists so that an
    ordinary archive sitting past the depth limit with nothing convertible in
    it does not abort a payload whose real data was already found.
    """
    with zipfile.ZipFile(archive) as zf:
        names = zf.namelist()
    for name in names:
        suffix = PurePosixPath(name).suffix.lower()
        if suffix in CONVERTIBLE_SUFFIXES:
            return True
        if suffix in _ARCHIVE_SUFFIXES:
            return True
    return False


def normalise(
    path: Path, workdir: Path, max_depth: int = 3, max_bytes: int = 20 * 2**30
) -> list[Path]:
    """Decompress/extract `path` and return the convertible files inside.

    Recurses into nested archives. The result may hold many files:
    `cityparquet convert` accepts several inputs and merges them, which is what
    a multi-tile archive needs — Japan's whole-city ZIPs hold 136 GMLs under
    `udx/`, beside codelists and a spec PDF that are dropped here. Documents
    that merely happen to be ZIPs (`.xlsx`, `.docx`, …) are left shut.

    `max_depth` counts unpacking rounds, not directory levels: the downloaded
    payload is depth 0, whatever comes out of it is depth 1, and an archive at
    depth `max_depth` is not opened. An unopened ZIP is skipped when its
    listing shows it could not have contributed anything convertible, and
    raises otherwise; an unopened gzip always raises, since its content cannot
    be inspected without decompressing it.

    An empty list means the payload holds nothing convertible, which the
    caller ledgers. Hostile or lossy input — a member escaping the working
    directory, a duplicate member name, a payload larger than `max_bytes`,
    convertible data stranded past `max_depth` — raises `ValueError` instead,
    because quietly skipping it would report a partial conversion as a
    complete one.
    """
    workdir.mkdir(parents=True, exist_ok=True)
    budget = _Budget(max_bytes)
    pending: list[tuple[Path, int]] = [(path, 0)]
    found: list[Path] = []
    # Every unpacked payload gets its own directory. Names repeat across an
    # archive's subdirectories, and unpacking two like-named members into one
    # place would silently drop one of them.
    unpacked = 0

    while pending:
        current, depth = pending.pop()
        with current.open("rb") as fh:
            kind = sniff(fh.read(4))

        if kind == "zip" and _is_container_document(current):
            # A document, not a package: judged shut and treated as ordinary
            # content, which the suffix check below then discards.
            kind = "plain"

        if kind in ("zip", "gzip") and depth >= max_depth:
            if kind == "zip" and not _could_contribute(current):
                continue
            raise ValueError(
                f"{current.name}: archive exceeds the maximum nesting depth of {max_depth} "
                f"and may hold convertible data"
            )

        if kind == "zip":
            unpacked += 1
            into = workdir / f"x{unpacked}_{current.stem}"
            pending.extend((member, depth + 1) for member in _safe_extract(current, into, budget))
            continue

        if kind == "gzip":
            unpacked += 1
            name = current.name.removesuffix(".gz") or "decompressed"
            target = workdir / f"g{unpacked}_{name}"
            with gzip.open(current, "rb") as src, target.open("wb") as dst:
                _copy_bounded(src, dst, budget, current.name)
            pending.append((target, depth + 1))
            continue

        if current.suffix.lower() in CONVERTIBLE_SUFFIXES:
            found.append(current)

    return sorted(found)
