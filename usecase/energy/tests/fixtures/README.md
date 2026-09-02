`tile_slice.parquet` is a deterministic 150-building slice (plus their
BuildingPart children) of 3DBAG tile 10-756-44 as CityParquet, regenerated with
`make_fixture.py SOURCE_PARQUET` (the source tile's `building.parquet` path is
required — there is no default). Committed so the test suite runs without the
18 GB tile set. 3DBAG data: CC-BY 4.0, © 3DBAG / TU Delft.
