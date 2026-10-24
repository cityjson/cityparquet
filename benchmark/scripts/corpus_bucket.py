#!/usr/bin/env python3
"""The hosted benchmark corpus: key layout, manifest, download and upload.

The prepared corpus lives in an object store (Cloudflare R2 by default), one
folder per preparation-chain version (`CHAIN_VERSION` in
`readbench_prepare.sh`):

    <prefix>/v<chain>/manifest.json
    <prefix>/v<chain>/citygml/<id>.gml
    <prefix>/v<chain>/cityjson/<id>.city.json          the normalised, compact source
    <prefix>/v<chain>/cityjsonseq/<id>.city.jsonl
    <prefix>/v<chain>/flatcitybuf/<id>.fcb
    <prefix>/v<chain>/cityparquet/<id>.parquet/...     every file of the package
    <prefix>/v<chain>/cityparquet-nobloom/<id>.parquet/...  bloom-axis datasets only

The folders exist only in keys: locally every artefact keeps the flat name
the prepare script gives it (`<id>.gml`, `<id>.parquet/`, the no-bloom
variant `<id>.cityparquet+nobloom.parquet/`), which is what `bench-run` and
the database family read.

Reading is anonymous, over the public base URL, with HTTP range requests so
an interrupted download resumes; every file is checked against the manifest
(bytes and sha256) and a file that does not match is fetched again once,
then refused. Writing goes through `rclone` and needs its credentials; every
call passes `--s3-no-check-bucket` because the token cannot list the bucket
root. An upload never replaces a key whose content differs unless `force`
is set, and identical content is skipped. The object store keeps no sha256,
so an existing key is compared with the manifest's recorded hash, or, when
the manifest does not record it, by downloading it through `rclone cat` and
hashing the bytes. After an upload the keys are verified by size with
`rclone lsjson`; the content itself is checked by rclone's own transfer
checksum (MD5 for a single-part S3 upload). `manifest.json` is written last,
so a partial upload is visible as an artefact the manifest does not list.

When the chain version changes, a dataset whose artefacts the new chain
writes byte for byte is carried forward rather than rebuilt and uploaded:
`carry-forward` copies its objects from `v<old>/` to `v<new>/` inside the
bucket (a server-side copy; nothing passes through the machine) and merges
its manifest entry, marked `carried_from`, into `v<new>/manifest.json`,
written last. With `--artefacts` only those artefacts are carried, so a
dataset whose other artefacts are rebuilt keeps an unchanged one, and the
rebuild's upload then finds it recorded and skips it without hashing it
remotely.

    python3 corpus_bucket.py carry-forward --from 8 --to 9 DATASET... [--artefacts citygml]
"""
from __future__ import annotations

import argparse
import datetime as _dt
import hashlib
import json
import os
import subprocess
import tempfile
import urllib.error
import urllib.request
from pathlib import Path

SCHEMA = 1
MANIFEST_NAME = "manifest.json"
DEFAULTS = {
    "remote": "r2",
    "bucket": "other-data",
    "prefix": "cityparquet-paper/benchmark",
    "base_url": "https://other-data.open3d.city",
}
ENV = {key: f"CITYPARQUET_CORPUS_{key.upper()}" for key in DEFAULTS}
# artefact name -> (bucket folder, key suffix, local suffix)
ARTEFACTS = {
    "citygml": ("citygml", ".gml", ".gml"),
    "cityjson": ("cityjson", ".city.json", ".city.json"),
    "cityjsonseq": ("cityjsonseq", ".city.jsonl", ".city.jsonl"),
    "flatcitybuf": ("flatcitybuf", ".fcb", ".fcb"),
    "cityparquet": ("cityparquet", ".parquet", ".parquet"),
    "cityparquet-nobloom": ("cityparquet-nobloom", ".parquet", ".cityparquet+nobloom.parquet"),
}
# Cloudflare answers 403 to urllib's default User-Agent, so every request names itself.
USER_AGENT = "cityparquet-bench-prep/1"
DIRECTORY_ARTEFACTS = frozenset({"cityparquet", "cityparquet-nobloom"})
REBUILD_HINT = "rebuild it with `just bench-prep --no-cache` or `--rebuild-sources` (needs the upload credentials), or prepare locally with `--local`"


class CorpusError(RuntimeError):
    """A refusal: the hosted corpus cannot provide what was asked for."""


def config(env: dict[str, str] | None = None) -> dict[str, str]:
    env = os.environ if env is None else env
    return {key: env.get(ENV[key], default).rstrip("/") for key, default in DEFAULTS.items()}


def version_prefix(cfg: dict[str, str], chain: int) -> str:
    return f"{cfg['prefix'].strip('/')}/v{chain}"


def artefact_key(artefact: str, dataset_id: str) -> str:
    folder, suffix, _ = ARTEFACTS[artefact]
    return f"{folder}/{dataset_id}{suffix}"


def local_name(artefact: str, dataset_id: str) -> str:
    return f"{dataset_id}{ARTEFACTS[artefact][2]}"


def public_url(cfg: dict[str, str], chain: int, key: str) -> str:
    return f"{cfg['base_url']}/{version_prefix(cfg, chain)}/{key}"


def remote_path(cfg: dict[str, str], chain: int, key: str) -> str:
    return f"{cfg['remote']}:{cfg['bucket']}/{version_prefix(cfg, chain)}/{key}"


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1 << 20), b""):
            digest.update(chunk)
    return digest.hexdigest()


def now() -> str:
    return _dt.datetime.now(_dt.timezone.utc).isoformat(timespec="seconds")


# --- describing and verifying local artefacts ------------------------------

def describe(path: Path) -> dict:
    """bytes + sha256 of a file; of a package directory, of every file in it."""
    if path.is_dir():
        files = {p.relative_to(path).as_posix(): {"bytes": p.stat().st_size, "sha256": sha256(p)} for p in sorted(path.rglob("*")) if p.is_file()}
        return {"bytes": sum(f["bytes"] for f in files.values()), "files": files}
    return {"bytes": path.stat().st_size, "sha256": sha256(path)}


def file_records(artefact: str, dataset_id: str, record: dict) -> dict[str, dict]:
    """Every object of one artefact record: key relative to v<chain>/ -> {bytes, sha256}."""
    key = record["key"]
    if "files" in record:
        return {f"{key}/{rel}": info for rel, info in record["files"].items()}
    return {key: {"bytes": record["bytes"], "sha256": record["sha256"]}}


def local_files(prepared: Path, artefact: str, dataset_id: str, record: dict) -> dict[str, Path]:
    base = prepared / local_name(artefact, dataset_id)
    if "files" in record:
        return {f"{record['key']}/{rel}": base / rel for rel in record["files"]}
    return {record["key"]: base}


def mismatches(prepared: Path, dataset_id: str, entry: dict) -> list[str]:
    """Keys whose local file is missing or differs from the manifest."""
    bad = []
    for artefact, record in entry["artefacts"].items():
        paths = local_files(prepared, artefact, dataset_id, record)
        for key, info in file_records(artefact, dataset_id, record).items():
            path = paths[key]
            if not path.is_file() or path.stat().st_size != info["bytes"] or sha256(path) != info["sha256"]:
                bad.append(key)
    return bad


# --- the manifest ---------------------------------------------------------

def dataset_entry(prepared: Path, dataset_id: str, artefacts: list[str], source: dict, built: dict) -> dict:
    records = {}
    for artefact in artefacts:
        path = prepared / local_name(artefact, dataset_id)
        if not path.exists():
            raise CorpusError(f"{dataset_id}: artefact {artefact} missing locally: {path}")
        records[artefact] = {"key": artefact_key(artefact, dataset_id), **describe(path)}
    return {"source": source, "built": built, "artefacts": records}


def merge(existing: dict | None, chain: int, datasets: dict[str, dict]) -> dict:
    """Merge per-dataset entries into a manifest; other datasets are kept."""
    if existing is not None and existing.get("chain_version") != chain:
        raise CorpusError(f"manifest is for chain version {existing.get('chain_version')}, not {chain}")
    result = dict(existing) if existing else {"schema": SCHEMA, "chain_version": chain, "created_at": now(), "datasets": {}}
    result["datasets"] = {**result.get("datasets", {}), **datasets}
    result["updated_at"] = now()
    return result


def dumps(manifest: dict) -> str:
    return json.dumps(manifest, indent=2, sort_keys=True) + "\n"


def require_entry(manifest: dict, dataset_id: str, artefacts: list[str]) -> dict:
    entry = manifest.get("datasets", {}).get(dataset_id)
    missing = [a for a in artefacts if entry is None or a not in entry.get("artefacts", {})]
    if missing:
        raise CorpusError(f"the hosted manifest has no {', '.join(missing)} for {dataset_id}; {REBUILD_HINT}")
    return entry


# --- download (anonymous HTTP) --------------------------------------------

def fetch_manifest(cfg: dict[str, str], chain: int) -> tuple[dict, str, str]:
    """(manifest, its URL, its sha256); refuses when v<chain>/ has none."""
    url = public_url(cfg, chain, MANIFEST_NAME)
    try:
        with urllib.request.urlopen(urllib.request.Request(url, headers={"User-Agent": USER_AGENT}), timeout=60) as response:
            body = response.read()
    except urllib.error.HTTPError as error:
        raise CorpusError(f"no hosted corpus for chain version {chain} ({url}: HTTP {error.code}); {REBUILD_HINT}") from error
    manifest = json.loads(body)
    if manifest.get("schema") != SCHEMA or manifest.get("chain_version") != chain:
        raise CorpusError(f"{url} is not a schema-{SCHEMA} manifest for chain version {chain}; {REBUILD_HINT}")
    return manifest, url, hashlib.sha256(body).hexdigest()


def download(url: str, target: Path, size: int, digest: str) -> bool:
    """Fetch url into target, resuming a partial file; True when bytes moved.

    A complete matching file is skipped. A file that fails the check after
    download is discarded and fetched once more from scratch, then refused."""
    if target.is_file() and target.stat().st_size == size and sha256(target) == digest:
        return False
    target.parent.mkdir(parents=True, exist_ok=True)
    partial = target.with_name(target.name + ".part")
    if target.is_file() and not partial.exists():
        target.replace(partial)  # a stale or truncated file: resume from it
    for attempt in range(2):
        offset = partial.stat().st_size if partial.is_file() else 0
        if offset > size:
            partial.unlink()
            offset = 0
        if offset < size:
            request = urllib.request.Request(url, headers={"User-Agent": USER_AGENT, **({"Range": f"bytes={offset}-"} if offset else {})})
            with urllib.request.urlopen(request, timeout=120) as response:
                if offset and response.status != 206:
                    offset = 0  # the server ignored the range: start again
                with partial.open("ab" if offset else "wb") as stream:
                    for chunk in iter(lambda: response.read(1 << 20), b""):
                        stream.write(chunk)
        if partial.stat().st_size == size and sha256(partial) == digest:
            partial.replace(target)
            return True
        partial.unlink()
    raise CorpusError(f"{url} does not match the manifest (expected {size} bytes, sha256 {digest}); {REBUILD_HINT}")


def download_dataset(cfg: dict[str, str], chain: int, manifest: dict, dataset_id: str, artefacts: list[str], prepared: Path) -> list[str]:
    """Download and verify the artefacts of one dataset; returns the keys fetched."""
    entry = require_entry(manifest, dataset_id, artefacts)
    fetched = []
    for artefact in artefacts:
        record = entry["artefacts"][artefact]
        paths = local_files(prepared, artefact, dataset_id, record)
        for key, info in file_records(artefact, dataset_id, record).items():
            if download(public_url(cfg, chain, key), paths[key], info["bytes"], info["sha256"]):
                fetched.append(key)
    return fetched


# --- upload (rclone, credentialed) ----------------------------------------

def rclone(*args: str, capture: bool = True) -> str:
    result = subprocess.run(["rclone", *args, "--s3-no-check-bucket"], check=True, text=True, capture_output=capture)
    return result.stdout if capture else ""


def remote_sizes(cfg: dict[str, str], chain: int) -> dict[str, int]:
    """key (relative to v<chain>/) -> bytes, for every object under v<chain>/."""
    try:
        listing = rclone("lsjson", "-R", "--files-only", remote_path(cfg, chain, ""))
    except subprocess.CalledProcessError:
        return {}  # nothing under v<chain>/ yet
    return {item["Path"]: item["Size"] for item in json.loads(listing or "[]")}


def remote_manifest(cfg: dict[str, str], chain: int) -> dict | None:
    if MANIFEST_NAME not in remote_sizes(cfg, chain):
        return None
    return json.loads(rclone("cat", remote_path(cfg, chain, MANIFEST_NAME)))


def remote_sha256(cfg: dict[str, str], chain: int, key: str) -> str:
    digest = hashlib.sha256()
    with subprocess.Popen(["rclone", "cat", remote_path(cfg, chain, key), "--s3-no-check-bucket"], stdout=subprocess.PIPE) as process:
        for chunk in iter(lambda: process.stdout.read(1 << 20), b""):
            digest.update(chunk)
    if process.returncode:
        raise CorpusError(f"rclone cat failed for {key}")
    return digest.hexdigest()


def upload_dataset(cfg: dict[str, str], chain: int, prepared: Path, entry: dict, dataset_id: str, existing: dict | None, force: bool = False) -> dict[str, list[str]]:
    """Upload one dataset's artefacts; returns {"uploaded": [...], "skipped": [...]}.

    Refuses, before anything is sent, when an existing key holds different
    content and `force` is not set, naming every differing key."""
    sizes = remote_sizes(cfg, chain)
    recorded = {}
    old = (existing or {}).get("datasets", {}).get(dataset_id, {})
    for artefact, record in old.get("artefacts", {}).items():
        recorded.update(file_records(artefact, dataset_id, record))
    plan, differing = [], []
    for artefact, record in entry["artefacts"].items():
        paths = local_files(prepared, artefact, dataset_id, record)
        for key, info in file_records(artefact, dataset_id, record).items():
            if key in sizes:
                same_size = sizes[key] == info["bytes"]
                remote_hash = recorded.get(key, {}).get("sha256") if key in recorded and recorded[key]["bytes"] == sizes[key] else None
                if same_size and (remote_hash or remote_sha256(cfg, chain, key)) == info["sha256"]:
                    plan.append((key, paths[key], False))
                    continue
                differing.append(key)
            plan.append((key, paths[key], True))
    if differing and not force:
        raise CorpusError(f"{dataset_id}: these keys already exist with different content: {', '.join(differing)}; pass --force-upload to replace them")
    done = {"uploaded": [], "skipped": []}
    for key, path, send in plan:
        if send:
            rclone("copyto", str(path), remote_path(cfg, chain, key), capture=False)
        done["uploaded" if send else "skipped"].append(key)
    after = remote_sizes(cfg, chain)
    wrong = [key for key, path, _ in plan if after.get(key) != path.stat().st_size]
    if wrong:
        raise CorpusError(f"{dataset_id}: upload not verified by size for {', '.join(wrong)}")
    return done


def entry_unchanged(existing: dict | None, dataset_id: str, entry: dict) -> bool:
    """Whether the hosted manifest already records this dataset with the same
    source and artefacts. Only `built` (when and by what it was built) may
    differ: a rebuild that reproduced every hosted byte leaves the manifest
    as it is, so the run uploads nothing at all."""
    hosted = ((existing or {}).get("datasets") or {}).get(dataset_id)
    return hosted is not None and all(hosted.get(k) == entry.get(k) for k in ("source", "artefacts"))


def publish_manifest(cfg: dict[str, str], chain: int, entries: dict[str, dict]) -> dict:
    """Merge entries into the hosted manifest and write it LAST."""
    manifest = merge(remote_manifest(cfg, chain), chain, entries)
    with tempfile.TemporaryDirectory() as scratch:
        local = Path(scratch) / MANIFEST_NAME
        local.write_text(dumps(manifest))
        rclone("copyto", str(local), remote_path(cfg, chain, MANIFEST_NAME), capture=False)
    return manifest


def carry_forward(cfg: dict[str, str], from_chain: int, to_chain: int, dataset_ids: list[str], artefacts: list[str] | None = None) -> dict[str, list[str]]:
    """Copy datasets' hosted artefacts (all, or `artefacts`) from v<from>/ to
    v<to>/ inside the bucket and merge their manifest entries into
    v<to>/manifest.json, written last; returns {"copied": [...], "skipped": [...]}.

    Refuses, before anything is copied, when a dataset or artefact is not in
    v<from>/'s manifest or a key exists in v<to>/ with different content."""
    old = remote_manifest(cfg, from_chain)
    if old is None or old.get("chain_version") != from_chain:
        raise CorpusError(f"no manifest for chain version {from_chain} at {remote_path(cfg, from_chain, MANIFEST_NAME)}")
    new = remote_manifest(cfg, to_chain)
    sizes = remote_sizes(cfg, to_chain)
    plan, differing, entries = [], [], {}
    for dataset_id in dataset_ids:
        hosted = old.get("datasets", {}).get(dataset_id)
        if hosted is None:
            raise CorpusError(f"the v{from_chain} manifest has no dataset {dataset_id}")
        wanted = artefacts or sorted(hosted.get("artefacts", {}))
        entry = require_entry(old, dataset_id, wanted)
        current = ((new or {}).get("datasets") or {}).get(dataset_id)
        recorded = {}
        for artefact, record in (current or {}).get("artefacts", {}).items():
            recorded.update(file_records(artefact, dataset_id, record))
        carried = {artefact: entry["artefacts"][artefact] for artefact in wanted}
        for artefact, record in carried.items():
            for key, info in file_records(artefact, dataset_id, record).items():
                if key in sizes:
                    known = recorded.get(key, {}).get("sha256") if recorded.get(key, {}).get("bytes") == sizes[key] else None
                    if sizes[key] == info["bytes"] and (known or remote_sha256(cfg, to_chain, key)) == info["sha256"]:
                        plan.append((key, info["bytes"], False))
                        continue
                    differing.append(key)
                plan.append((key, info["bytes"], True))
        if current is not None:
            entries[dataset_id] = {**current, "artefacts": {**current.get("artefacts", {}), **carried}}
        else:
            entries[dataset_id] = {"source": entry["source"], "built": entry["built"], "carried_from": from_chain, "artefacts": carried}
    if differing:
        raise CorpusError(f"these keys already exist in v{to_chain}/ with different content: {', '.join(differing)}")
    done = {"copied": [], "skipped": []}
    for key, _, send in plan:
        if send:
            rclone("copyto", remote_path(cfg, from_chain, key), remote_path(cfg, to_chain, key), capture=False)
        done["copied" if send else "skipped"].append(key)
    after = remote_sizes(cfg, to_chain)
    wrong = [key for key, size, _ in plan if after.get(key) != size]
    if wrong:
        raise CorpusError(f"copy not verified by size for {', '.join(wrong)}")
    if done["copied"] or any(((new or {}).get("datasets") or {}).get(d) != e for d, e in entries.items()):
        publish_manifest(cfg, to_chain, entries)
    return done


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description="The hosted benchmark corpus (benchmark/README.md, \"The hosted corpus\").")
    commands = parser.add_subparsers(dest="command", required=True)
    carry = commands.add_parser("carry-forward", help="copy datasets' artefacts from v<from>/ to v<to>/ inside the bucket, with their manifest entries (needs the rclone credentials)")
    carry.add_argument("--from", dest="from_chain", type=int, required=True, help="the chain version the artefacts are hosted under")
    carry.add_argument("--to", dest="to_chain", type=int, required=True, help="the chain version they are carried to")
    carry.add_argument("--artefacts", help="comma-separated artefacts to carry (default: every artefact of the dataset)")
    carry.add_argument("datasets", nargs="+", help="dataset ids (file stems, e.g. rotterdam_delfshaven)")
    args = parser.parse_args(argv)
    cfg = config()
    try:
        artefacts = args.artefacts.split(",") if args.artefacts else None
        done = carry_forward(cfg, args.from_chain, args.to_chain, args.datasets, artefacts)
    except CorpusError as error:
        raise SystemExit(f"corpus_bucket: {error}") from error
    print(f"v{args.from_chain} -> v{args.to_chain}: {len(done['copied'])} copied, {len(done['skipped'])} identical and skipped")
    print(f"manifest: {public_url(cfg, args.to_chain, MANIFEST_NAME)}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
