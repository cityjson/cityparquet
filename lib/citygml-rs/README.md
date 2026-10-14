# citygml-rs

Streaming CityGML readers for Rust.

- [`citygml-core`](crates/core): the version-neutral model (`GmlObject`), GML
  geometry, and the conversion to [cjseq](https://github.com/HideBa/cjseq)'s
  CityJSON types.
- [`citygml2`](crates/citygml2): the CityGML 2.0 reader.

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
