"""A throwaway local HTTP server standing in for the GCS-hosted catalogue.

Tests never touch the network: every fixture below is served from a temp dir,
so the suite is deterministic and runnable offline.
"""

import json
import threading
from functools import partial
from http.server import SimpleHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

import pytest


@pytest.fixture
def served_dir(tmp_path):
    """Serve `tmp_path` over HTTP; yields (root_path, base_url)."""
    handler = partial(SimpleHTTPRequestHandler, directory=str(tmp_path))
    server = ThreadingHTTPServer(("127.0.0.1", 0), handler)
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    host, port = server.server_address
    try:
        yield tmp_path, f"http://{host}:{port}"
    finally:
        server.shutdown()
        server.server_close()


def write_json(path: Path, payload) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(payload), encoding="utf-8")


def stac_item(item_id: str, href: str, media_type: str = "application/city+json") -> dict:
    return {
        "type": "Feature",
        "stac_version": "1.1.0",
        "id": item_id,
        "geometry": None,
        "bbox": None,
        "properties": {"datetime": None},
        "links": [],
        "assets": {"data": {"href": href, "type": media_type, "roles": ["data"]}},
    }
