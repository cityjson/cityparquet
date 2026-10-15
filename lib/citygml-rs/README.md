# citygml-rs

Streaming CityGML readers and writers for Rust.

- [`citygml`](crates/citygml): CityGML 2.0 or 3.0 to and from CityJSON,
  whichever version a document is: its CityJSON header, its features, and a
  writer by version.
- [`citygml-core`](crates/core): the version-neutral model (`GmlObject`), GML
  geometry, the CRS an `srsName` names, and the conversions to and from
  [cjseq](https://crates.io/crates/cjseq)'s CityJSON types.
- [`citygml2`](crates/citygml2): the CityGML 2.0 reader and writer.
- [`citygml3`](crates/citygml3): the CityGML 3.0 reader and writer, with the
  same API.

```rust
use citygml::{Version, Writer, WriterOptions};

// The transform follows the units of the CRS's axes.
let (header, features) = citygml::open(path)?;
let mut writer = Writer::new(&Version::V3_0, out, &header, WriterOptions::default())?;
for feature in features {
    writer.write_feature(&feature?)?; // a cjseq::CityJSONFeature
}
let (_out, report) = writer.finish()?; // what CityGML 3.0 could not hold
```

One version's crate reads a `GmlObject` per top-level city object:

```rust
use citygml2::FeatureReader;

let reader = FeatureReader::open(path)?;
for object in reader {
    let object = object?; // a citygml_core::GmlObject
    println!("{} {:?}", object.class.local, object.id);
}
```

With the default `cjseq` feature, `FeatureReader::features(transform)` yields
`cjseq::CityJSONFeature`s quantised against `transform` instead.

Licensed under MIT or Apache-2.0, at your option.
