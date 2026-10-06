"""The hosted-corpus layer against a fake bucket: a local HTTP server that
honours Range requests, and a fake `rclone` on PATH. No real network."""
from __future__ import annotations

import functools
import http.server
import json
import os
import sys
import tempfile
import threading
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import corpus_bucket as cb  # noqa: E402

FAKE_RCLONE = r'''#!PYTHON
import json, os, shutil, sys
args = [a for a in sys.argv[1:] if a != "--s3-no-check-bucket"]
root = os.environ["FAKE_BUCKET"]
def local(spec):
    remote, path = spec.split(":", 1)
    return os.path.join(root, path)
with open(os.path.join(root, "calls.log"), "a") as log:
    log.write(" ".join(sys.argv[1:]) + "\n")
if args[0] == "lsjson":
    base = local(args[-1])
    if not os.path.isdir(base):
        sys.exit(3)
    items = []
    for d, _, files in os.walk(base):
        for f in files:
            p = os.path.join(d, f)
            items.append({"Path": os.path.relpath(p, base), "Size": os.path.getsize(p)})
    print(json.dumps(items))
elif args[0] == "cat":
    with open(local(args[1]), "rb") as s:
        sys.stdout.buffer.write(s.read())
elif args[0] == "copyto":
    target = local(args[2])
    os.makedirs(os.path.dirname(target), exist_ok=True)
    shutil.copyfile(args[1], target)
else:
    sys.exit(2)
'''


class RangeHandler(http.server.SimpleHTTPRequestHandler):
    ranges: list[str] = []

    def log_message(self, *args):
        pass

    agents: list[str] = []

    def do_GET(self):
        RangeHandler.agents.append(self.headers.get("User-Agent", ""))
        header = self.headers.get("Range")
        path = Path(self.translate_path(self.path))
        if not header or not path.is_file():
            return super().do_GET()
        RangeHandler.ranges.append(header)
        start = int(header.split("=")[1].split("-")[0])
        body = path.read_bytes()[start:]
        self.send_response(206)
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)


class CorpusBucketTest(unittest.TestCase):
    def setUp(self):
        self.tmp = Path(tempfile.mkdtemp())
        self.bucket = self.tmp / "bucket"
        (self.bucket / "other-data").mkdir(parents=True)
        tools = self.tmp / "tools"
        tools.mkdir()
        rclone = tools / "rclone"
        rclone.write_text(FAKE_RCLONE.replace("PYTHON", sys.executable))
        rclone.chmod(0o755)
        self.env = {"PATH": os.environ["PATH"], "FAKE_BUCKET": str(self.bucket)}
        self.old_env = dict(os.environ)
        os.environ["PATH"] = f"{tools}:{os.environ['PATH']}"
        os.environ["FAKE_BUCKET"] = str(self.bucket)
        handler = functools.partial(RangeHandler, directory=str(self.bucket / "other-data"))
        self.server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), handler)
        threading.Thread(target=self.server.serve_forever, daemon=True).start()
        self.cfg = cb.config({cb.ENV["remote"]: "fake", cb.ENV["base_url"]: f"http://127.0.0.1:{self.server.server_port}"})
        self.prepared = self.tmp / "prepared"
        self.prepared.mkdir()
        (self.prepared / "rdam.gml").write_text("<gml/>")
        (self.prepared / "rdam.city.jsonl").write_text('{"type":"CityJSON"}\n')
        package = self.prepared / "rdam.parquet"
        (package / "building").mkdir(parents=True)
        (package / "metadata.json").write_text("{}")
        (package / "building" / "part.parquet").write_bytes(b"PAR1" * 100)

    def tearDown(self):
        self.server.shutdown()
        self.server.server_close()
        os.environ.clear()
        os.environ.update(self.old_env)

    def entry(self, artefacts=("citygml", "cityjsonseq", "cityparquet")):
        return cb.dataset_entry(self.prepared, "rdam", list(artefacts), {"url": "https://example.org/rdam.json", "sha256": "0" * 64}, {"monorepo_commit": "abc"})

    def publish(self, force=False):
        entry = self.entry()
        existing = cb.remote_manifest(self.cfg, 7)
        done = cb.upload_dataset(self.cfg, 7, self.prepared, entry, "rdam", existing, force=force)
        cb.publish_manifest(self.cfg, 7, {"rdam": entry})
        return done

    def test_keys_map_formats_to_folders_and_keep_local_names_flat(self):
        self.assertEqual(cb.artefact_key("citygml", "x"), "citygml/x.gml")
        self.assertEqual(cb.artefact_key("cityparquet-nobloom", "x"), "cityparquet-nobloom/x.parquet")
        self.assertEqual(cb.local_name("cityparquet-nobloom", "x"), "x.cityparquet+nobloom.parquet")
        self.assertEqual(cb.public_url(self.cfg, 7, "fcb/x.fcb").split("/", 3)[3], "cityparquet-paper/benchmark/v7/fcb/x.fcb")
        self.assertEqual(cb.remote_path(self.cfg, 7, "a"), "fake:other-data/cityparquet-paper/benchmark/v7/a")

    def test_package_directory_is_described_file_by_file(self):
        record = self.entry()["artefacts"]["cityparquet"]
        self.assertEqual(sorted(record["files"]), ["building/part.parquet", "metadata.json"])
        self.assertEqual(record["key"], "cityparquet/rdam.parquet")

    def test_an_entry_whose_content_matches_the_hosted_one_is_unchanged(self):
        hosted = {"chain_version": 8, "datasets": {"rotterdam": {
            "source": {"url": "u", "sha256": "s"},
            "built": {"created_at": "2026-10-06T16:30:16+00:00"},
            "artefacts": {"cityjson": {"key": "cityjson/r.city.json", "bytes": 1, "sha256": "a"}}}}}
        rebuilt = json.loads(json.dumps(hosted["datasets"]["rotterdam"]))
        rebuilt["built"]["created_at"] = "2026-10-06T17:41:48+00:00"
        self.assertTrue(cb.entry_unchanged(hosted, "rotterdam", rebuilt))
        rebuilt["artefacts"]["cityjson"]["sha256"] = "b"
        self.assertFalse(cb.entry_unchanged(hosted, "rotterdam", rebuilt))
        self.assertFalse(cb.entry_unchanged(hosted, "vienna", rebuilt))
        self.assertFalse(cb.entry_unchanged(None, "rotterdam", rebuilt))

    def test_merge_keeps_other_datasets_and_refuses_another_chain(self):
        first = cb.merge(None, 7, {"a": {"x": 1}})
        second = cb.merge(first, 7, {"b": {"x": 2}})
        self.assertEqual(sorted(second["datasets"]), ["a", "b"])
        self.assertEqual(second["created_at"], first["created_at"])
        with self.assertRaises(cb.CorpusError):
            cb.merge(first, 8, {})

    def test_upload_then_download_round_trip_and_manifest_written_last(self):
        done = self.publish()
        self.assertEqual(len(done["uploaded"]), 4)
        calls = (self.bucket / "calls.log").read_text().splitlines()
        self.assertTrue(all("--s3-no-check-bucket" in c for c in calls))
        copies = [c for c in calls if c.startswith("copyto")]
        self.assertTrue(copies[-1].replace(" --s3-no-check-bucket", "").endswith("v7/manifest.json"))
        manifest, url, digest = cb.fetch_manifest(self.cfg, 7)
        self.assertTrue(url.endswith("/v7/manifest.json"))
        self.assertEqual(len(digest), 64)
        fresh = self.tmp / "fresh"
        fetched = cb.download_dataset(self.cfg, 7, manifest, "rdam", ["citygml", "cityjsonseq", "cityparquet"], fresh)
        self.assertEqual(len(fetched), 4)
        self.assertEqual((fresh / "rdam.parquet" / "building" / "part.parquet").read_bytes(), b"PAR1" * 100)
        self.assertEqual(cb.mismatches(fresh, "rdam", manifest["datasets"]["rdam"]), [])
        # a second pass moves nothing
        self.assertEqual(cb.download_dataset(self.cfg, 7, manifest, "rdam", ["citygml"], fresh), [])

    def test_identical_reupload_is_skipped_and_different_content_needs_force(self):
        self.publish()
        self.assertEqual(self.publish()["uploaded"], [])
        (self.prepared / "rdam.parquet" / "metadata.json").write_text('{"datetime": "later"}')
        with self.assertRaises(cb.CorpusError) as caught:
            self.publish()
        self.assertIn("cityparquet/rdam.parquet/metadata.json", str(caught.exception))
        self.assertNotIn("part.parquet", str(caught.exception))
        self.assertEqual(self.publish(force=True)["uploaded"], ["cityparquet/rdam.parquet/metadata.json"])

    def test_unrecorded_existing_key_is_hashed_through_rclone_cat(self):
        target = self.bucket / "other-data/cityparquet-paper/benchmark/v7/citygml/rdam.gml"
        target.parent.mkdir(parents=True)
        target.write_text("<other/>")  # same size, different bytes, no manifest
        entry = self.entry(["citygml"])
        with self.assertRaises(cb.CorpusError):
            cb.upload_dataset(self.cfg, 7, self.prepared, entry, "rdam", None)
        target.write_text("<gml/>")
        self.assertEqual(cb.upload_dataset(self.cfg, 7, self.prepared, entry, "rdam", None)["skipped"], ["citygml/rdam.gml"])

    def test_partial_download_resumes_with_a_range_request(self):
        self.publish()
        manifest, _, _ = cb.fetch_manifest(self.cfg, 7)
        fresh = self.tmp / "fresh"
        fresh.mkdir()
        (fresh / "rdam.city.jsonl.part").write_text('{"type":')
        RangeHandler.ranges.clear()
        cb.download_dataset(self.cfg, 7, manifest, "rdam", ["cityjsonseq"], fresh)
        self.assertEqual(RangeHandler.ranges, ["bytes=8-"])
        self.assertTrue(RangeHandler.agents and all(agent == cb.USER_AGENT for agent in RangeHandler.agents))
        self.assertEqual((fresh / "rdam.city.jsonl").read_text(), '{"type":"CityJSON"}\n')

    def test_altered_local_file_is_detected_and_fetched_again(self):
        self.publish()
        manifest, _, _ = cb.fetch_manifest(self.cfg, 7)
        fresh = self.tmp / "fresh"
        cb.download_dataset(self.cfg, 7, manifest, "rdam", ["citygml"], fresh)
        (fresh / "rdam.gml").write_text("<bad/>")
        self.assertEqual(cb.mismatches(fresh, "rdam", manifest["datasets"]["rdam"])[0], "citygml/rdam.gml")
        self.assertEqual(cb.download_dataset(self.cfg, 7, manifest, "rdam", ["citygml"], fresh), ["citygml/rdam.gml"])
        self.assertEqual((fresh / "rdam.gml").read_text(), "<gml/>")

    def test_hash_mismatch_against_the_manifest_refuses(self):
        self.publish()
        manifest, _, _ = cb.fetch_manifest(self.cfg, 7)
        manifest["datasets"]["rdam"]["artefacts"]["citygml"]["sha256"] = "f" * 64
        with self.assertRaises(cb.CorpusError) as caught:
            cb.download_dataset(self.cfg, 7, manifest, "rdam", ["citygml"], self.tmp / "fresh")
        self.assertIn("--no-cache", str(caught.exception))

    def test_missing_chain_or_dataset_refuses_naming_the_rebuild_modes(self):
        with self.assertRaises(cb.CorpusError) as caught:
            cb.fetch_manifest(self.cfg, 7)
        self.assertIn("--rebuild-sources", str(caught.exception))
        self.publish()
        manifest, _, _ = cb.fetch_manifest(self.cfg, 7)
        with self.assertRaises(cb.CorpusError):
            cb.download_dataset(self.cfg, 7, manifest, "vienna", ["citygml"], self.tmp / "fresh")
        with self.assertRaises(cb.CorpusError):
            cb.download_dataset(self.cfg, 7, manifest, "rdam", ["flatcitybuf"], self.tmp / "fresh")


if __name__ == "__main__":
    unittest.main()
