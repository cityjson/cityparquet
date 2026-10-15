# citygml-rs

Streaming CityGML readers for Rust.

- [`citygml-core`](crates/core): the version-neutral model (`GmlObject`), GML
  geometry, and the conversion to [cjseq](https://crates.io/crates/cjseq)'s
  CityJSON types.
- [`citygml2`](crates/citygml2): the CityGML 2.0 reader.
- [`citygml3`](crates/citygml3): the CityGML 3.0 reader, with the same API.

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
