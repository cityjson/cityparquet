from pathlib import Path

import pytest

FIXTURES = Path(__file__).parent / "fixtures"


@pytest.fixture
def fixture_path() -> Path:
    return FIXTURES / "tile_slice.parquet"
