from pathlib import Path

import pytest

from energy.db import extensions_available

FIXTURES = Path(__file__).parent / "fixtures"


@pytest.fixture
def fixture_path() -> Path:
    return FIXTURES / "tile_slice.parquet"


requires_extensions = pytest.mark.skipif(
    not extensions_available(),
    reason="cannot install the cityjson / three_d community builds (offline?)",
)
