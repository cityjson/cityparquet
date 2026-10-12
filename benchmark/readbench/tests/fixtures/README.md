# Read-benchmark test fixtures

Small real inputs the readbench integration tests run against. The larger
shared fixtures (`delft.city.jsonl`, `lod3_railway.city.json`) are fetched by
`just fixtures` in `lib/cityparquet-rs`; the ones here are committed because no
fetched fixture has the property they test.

## `tokyo_chiyoda_40.city.jsonl`

The header line and the first 40 features of the benchmark corpus's Tokyo
CityJSONSeq artefact (`tokyo.city.jsonl`, as `readbench_prepare.sh` builds it
from `tokyo.city.json`), cut verbatim. Its CRS, JGD2011 (EPSG:6697), declares
latitude before longitude, so its vertices are stored latitude first; a
CityParquet package stores them longitude first, as GeoParquet requires. The
fixture exists to test that every runner applies a query window in its own
artefact's axis order.

Source: 3D city model of Chiyoda-ku, Tokyo, PLATEAU, Ministry of Land,
Infrastructure, Transport and Tourism (MLIT), Japan — licensed under
[CC BY 4.0](https://creativecommons.org/licenses/by/4.0/). Converted from
CityGML to CityJSON and merged as `benchmark/formats/README.md` describes.
