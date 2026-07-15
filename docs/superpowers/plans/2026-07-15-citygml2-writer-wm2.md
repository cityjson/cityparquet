# CityGML 2.0 Writer — W-M2 (Attributes + CompositeSolid) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Round-trip building attributes and CompositeSolid geometry from a CityParquet package back to CityGML 2.0; skip MultiSolid with a counter (no CityGML 2.0 Building slot).

**Architecture:** A new `writer/attributes.rs` routes each attribute by its stored column type to a typed `bldg:` element or a type-matched `gen:` generic attribute; `writer/geometry.rs` gains `write_composite_solid`; the driver (`writer/mod.rs`) stops skipping CompositeSolid, skips MultiSolid, and threads attributes into `write_building`. The round-trip invariant is package-level equality, verified on real fixtures.

**Tech Stack:** Rust, `quick_xml::Writer` (0.37), `serde_json`, existing `wkb_read`/`export`/`decode` primitives.

## Global Constraints

- Round-trip invariant is PACKAGE-LEVEL, not XML-identical: real CityGML fixture → package → `.gml` (writer) → package (re-convert) must produce equal stored geometry + attributes, keyed by `(building_id, major_lod)`.
- Real fixtures only for round-trip oracles; in-code `DecodedGeometry`/attribute values are fine for unit tests (matches existing writer unit-test style).
- Strict red-green TDD; one behaviour per test; frequent commits.
- Never error on unrepresentable data — skip-with-counter. A conversion always yields a valid document.
- Float formatting MUST be shortest-round-trip (Rust `Display`/`{}`). Fixed-precision (`{:.N}`) is FORBIDDEN.
- Attribute text and `name=` attributes go through `BytesText::new` / `push_attribute`, which **auto-escape** — never `from_escaped`.
- Green gate: `just check` (clippy `-D warnings` + fmt + tests + schema isolation).
- `cjseq::CityObject.attributes` is `Option<Value>` (a `Some(Value::Object(map))` when present).
- Reader-forced types for typed `bldg:` names: `function/usage/class/roofType/yearOfConstruction/yearOfDemolition` → String; `measuredHeight` → Float; `storeysAboveGround/storeysBelowGround` → Int.

---

### Task 1: Attribute routing module (`writer/attributes.rs`)

Pure attribute serialiser with all routing + skip logic, unit-tested with in-code maps. Adds the two attribute counters to `WriteReport`.

**Files:**
- Create: `crates/cityparquet/src/citygml/writer/attributes.rs`
- Modify: `crates/cityparquet/src/citygml/writer/mod.rs` (register module; add `attributes_written`/`attributes_skipped` to `WriteReport`)

**Interfaces:**
- Produces:
  - `pub fn write_attributes<W: std::io::Write>(w: &mut quick_xml::Writer<W>, attrs: &serde_json::Map<String, serde_json::Value>, report: &mut super::WriteReport) -> crate::Result<usize>` — emits attribute elements in `attrs` iteration order, increments `report.attributes_written`/`report.attributes_skipped`, and returns the number of attribute **values** written (an array counts each item). Emits nothing and returns 0 for an empty map.
  - `WriteReport` gains `pub attributes_written: usize` and `pub attributes_skipped: usize`.

- [ ] **Step 1: Add the two counters to `WriteReport`**

In `crates/cityparquet/src/citygml/writer/mod.rs`, inside `pub struct WriteReport { … }`, add:

```rust
    /// Attribute values emitted as `bldg:`/`gen:` elements.
    pub attributes_written: usize,
    /// Attribute values skipped as unrepresentable in CityGML 2.0 (Boolean,
    /// nested/heterogeneous `Json`, single/empty string lists, empty/whitespace
    /// strings, XML-illegal control chars).
    pub attributes_skipped: usize,
```

And register the module near the other `pub mod` lines:

```rust
pub mod attributes;
```

- [ ] **Step 2: Write the failing unit tests**

Create `crates/cityparquet/src/citygml/writer/attributes.rs` with ONLY the test module first (the file must still compile — add `use` for the function it calls, which will fail to resolve → red):

```rust
//! Building attribute serialisation: route each attribute by its stored column
//! type to a typed `bldg:` element or a type-matched `gen:` generic attribute.
//! The round-trip invariant is package-level: the re-read `name -> type` map and
//! values must match, so an attribute is written with the `bldg:` element only
//! when its stored type equals the type the reader forces back for that name;
//! otherwise it falls back to the `gen:` element of its stored type. Values
//! CityGML 2.0 cannot represent are skipped-with-counter, never errored.

use std::io::Write;

use quick_xml::Writer;
use quick_xml::events::{BytesEnd, BytesStart, BytesText, Event};
use serde_json::{Map, Value};

use super::WriteReport;
use crate::Result;
use cityparquet_schema::CityParquetError;

#[cfg(test)]
mod tests {
    use super::*;

    fn emit(attrs: &Map<String, Value>) -> (String, WriteReport) {
        let mut w = Writer::new(Vec::new());
        let mut report = WriteReport::default();
        let n = write_attributes(&mut w, attrs, &mut report).unwrap();
        report.attributes_written = report.attributes_written; // silence unused if 0
        let xml = String::from_utf8(w.into_inner()).unwrap();
        assert_eq!(n, report.attributes_written, "return value counts written values");
        (xml, report)
    }

    fn map(pairs: Vec<(&str, Value)>) -> Map<String, Value> {
        pairs.into_iter().map(|(k, v)| (k.to_string(), v)).collect()
    }

    #[test]
    fn known_bldg_name_with_matching_type_uses_bldg_element() {
        // measuredHeight stored as float -> matches reader-forced Float -> bldg:, uom="m".
        let (xml, r) = emit(&map(vec![("measuredHeight", Value::from(8.0))]));
        assert!(xml.contains("<bldg:measuredHeight uom=\"m\">8</bldg:measuredHeight>"), "{xml}");
        assert_eq!(r.attributes_written, 1);
        assert_eq!(r.attributes_skipped, 0);
    }

    #[test]
    fn string_forced_bldg_name_uses_bldg_element() {
        let (xml, _) = emit(&map(vec![("roofType", Value::from("1000"))]));
        assert!(xml.contains("<bldg:roofType>1000</bldg:roofType>"), "{xml}");
    }

    #[test]
    fn integer_storeys_uses_bldg_element() {
        let (xml, _) = emit(&map(vec![("storeysAboveGround", Value::from(3i64))]));
        assert!(xml.contains("<bldg:storeysAboveGround>3</bldg:storeysAboveGround>"), "{xml}");
    }

    #[test]
    fn known_name_with_mismatched_type_falls_back_to_gen() {
        // yearOfConstruction is String-forced, but stored as an integer here
        // (CityJSON-origin): must go gen:intAttribute so it re-infers as Int64.
        let (xml, _) = emit(&map(vec![("yearOfConstruction", Value::from(1985i64))]));
        assert!(
            xml.contains("<gen:intAttribute name=\"yearOfConstruction\"><gen:value>1985</gen:value></gen:intAttribute>"),
            "{xml}"
        );
        assert!(!xml.contains("<bldg:yearOfConstruction>"));
    }

    #[test]
    fn unknown_string_uses_gen_string_attribute() {
        let (xml, _) = emit(&map(vec![("owner", Value::from("Acme & Co <x>"))]));
        // value is auto-escaped.
        assert!(
            xml.contains("<gen:stringAttribute name=\"owner\"><gen:value>Acme &amp; Co &lt;x&gt;</gen:value></gen:stringAttribute>"),
            "{xml}"
        );
    }

    #[test]
    fn unknown_float_uses_gen_double_attribute() {
        let (xml, _) = emit(&map(vec![("area", Value::from(12.5))]));
        assert!(xml.contains("<gen:doubleAttribute name=\"area\"><gen:value>12.5</gen:value></gen:doubleAttribute>"), "{xml}");
    }

    #[test]
    fn date_shaped_string_uses_gen_date_attribute() {
        let (xml, _) = emit(&map(vec![("built", Value::from("1985-06-17"))]));
        assert!(xml.contains("<gen:dateAttribute name=\"built\"><gen:value>1985-06-17</gen:value></gen:dateAttribute>"), "{xml}");
    }

    #[test]
    fn multi_element_string_list_emits_one_per_item_in_order() {
        let (xml, r) = emit(&map(vec![(
            "function",
            Value::Array(vec![Value::from("1000"), Value::from("1610")]),
        )]));
        // function is a bldg: name (String-forced); each item -> its own bldg: element.
        let a = xml.find("1000").unwrap();
        let b = xml.find("1610").unwrap();
        assert!(a < b, "items preserve order: {xml}");
        assert_eq!(xml.matches("<bldg:function>").count(), 2, "{xml}");
        assert_eq!(r.attributes_written, 2);
    }

    #[test]
    fn boolean_is_skipped() {
        let (xml, r) = emit(&map(vec![("flag", Value::from(true))]));
        assert!(xml.is_empty(), "{xml}");
        assert_eq!(r.attributes_written, 0);
        assert_eq!(r.attributes_skipped, 1);
    }

    #[test]
    fn nested_object_is_skipped() {
        let (_, r) = emit(&map(vec![("meta", serde_json::json!({"a": 1}))]));
        assert_eq!(r.attributes_skipped, 1);
        assert_eq!(r.attributes_written, 0);
    }

    #[test]
    fn single_element_string_list_is_skipped() {
        // ["a"] would re-infer as scalar String, flipping the column type.
        let (_, r) = emit(&map(vec![("tags", Value::Array(vec![Value::from("a")]))]));
        assert_eq!(r.attributes_skipped, 1);
        assert_eq!(r.attributes_written, 0);
    }

    #[test]
    fn empty_and_whitespace_strings_are_skipped() {
        let (_, r) = emit(&map(vec![("a", Value::from("")), ("b", Value::from("   "))]));
        assert_eq!(r.attributes_skipped, 2);
        assert_eq!(r.attributes_written, 0);
    }

    #[test]
    fn control_char_string_is_skipped() {
        let (_, r) = emit(&map(vec![("bad", Value::from("x\u{0007}y"))]));
        assert_eq!(r.attributes_skipped, 1);
    }

    #[test]
    fn float_formatting_is_shortest_round_trip() {
        // 8.0 -> "8"; -0.0 -> "-0"; long decimal preserved exactly and re-parses.
        for v in [8.0_f64, -0.0, 1.0 / 3.0, 1e21] {
            let (xml, _) = emit(&map(vec![("x", Value::from(v))]));
            // extract the text between the value tags and re-parse.
            let start = xml.find("<gen:value>").unwrap() + "<gen:value>".len();
            let end = xml.find("</gen:value>").unwrap();
            let parsed: f64 = xml[start..end].parse().unwrap();
            assert!(parsed.to_bits() == v.to_bits() || (parsed == v), "{v} -> {}", &xml[start..end]);
        }
    }
}
```

- [ ] **Step 3: Run the tests to verify they fail**

Run: `cargo test -p cityparquet --lib citygml::writer::attributes 2>&1 | tail -20`
Expected: FAIL — `write_attributes` is not defined.

- [ ] **Step 4: Implement `write_attributes`**

Insert above the `#[cfg(test)]` module in `attributes.rs`:

```rust
fn io_err(e: std::io::Error) -> CityParquetError {
    CityParquetError::Io(e.to_string())
}

/// The stored column type a value round-trips through, decided by the JSON shape
/// `decode` produced (serde's own `is_i64`/`is_f64`, not numeric range).
enum Kind {
    Str,
    Int,
    Float,
    /// A date-shaped string (`YYYY-MM-DD`) — re-infers as a Date column.
    Date,
    /// A string list with >= 2 string items — re-infers as StringList.
    StrList,
    /// Boolean, nested/heterogeneous Json, single/empty string list — no
    /// round-trip-stable CityGML 2.0 form.
    Unwritable,
}

fn is_date_shaped(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() == 10
        && b[4] == b'-'
        && b[7] == b'-'
        && b[..4].iter().all(u8::is_ascii_digit)
        && b[5..7].iter().all(u8::is_ascii_digit)
        && b[8..].iter().all(u8::is_ascii_digit)
}

/// A string CityGML/XML cannot carry losslessly: empty/whitespace-only (the
/// reader drops it) or containing an XML-1.0-illegal control char.
fn is_unwritable_string(s: &str) -> bool {
    if s.trim().is_empty() {
        return true;
    }
    s.chars().any(|c| {
        let u = c as u32;
        // Legal XML 1.0 chars: 0x9, 0xA, 0xD, and >= 0x20 (plus higher ranges,
        // always legal here). Everything else in 0x00..0x1F is illegal.
        (u < 0x20) && !matches!(u, 0x9 | 0xA | 0xD)
    })
}

fn value_kind(v: &Value) -> Kind {
    match v {
        Value::String(s) if is_date_shaped(s) => Kind::Date,
        Value::String(_) => Kind::Str,
        Value::Number(n) if n.is_i64() || n.is_u64() => Kind::Int,
        Value::Number(_) => Kind::Float,
        Value::Array(items) if items.len() >= 2 && items.iter().all(Value::is_string) => {
            Kind::StrList
        }
        _ => Kind::Unwritable,
    }
}

/// The reader-forced type for a typed `bldg:` name, or `None` if not a known
/// typed attribute. `Str`/`Int`/`Float` here mirror `citygml::attributes`.
fn bldg_forced_kind(name: &str) -> Option<Kind> {
    Some(match name {
        "function" | "usage" | "class" | "roofType" | "yearOfConstruction"
        | "yearOfDemolition" => Kind::Str,
        "measuredHeight" => Kind::Float,
        "storeysAboveGround" | "storeysBelowGround" => Kind::Int,
        _ => return None,
    })
}

/// `X Y Z`-free scalar text for a JSON scalar: shortest-round-trip for numbers,
/// verbatim for strings.
fn scalar_text(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Number(n) => n.to_string(), // serde uses shortest round-trip
        other => other.to_string(),
    }
}

fn write_bldg_element<W: Write>(
    w: &mut Writer<W>,
    name: &str,
    text: &str,
    uom: Option<&str>,
) -> Result<()> {
    let mut start = BytesStart::new(format!("bldg:{name}"));
    if let Some(u) = uom {
        start.push_attribute(("uom", u));
    }
    w.write_event(Event::Start(start)).map_err(io_err)?;
    w.write_event(Event::Text(BytesText::new(text))).map_err(io_err)?;
    w.write_event(Event::End(BytesEnd::new(format!("bldg:{name}"))))
        .map_err(io_err)?;
    Ok(())
}

fn write_gen_element<W: Write>(
    w: &mut Writer<W>,
    element: &str, // "stringAttribute" | "intAttribute" | "doubleAttribute" | "dateAttribute"
    name: &str,
    text: &str,
) -> Result<()> {
    let mut start = BytesStart::new(format!("gen:{element}"));
    start.push_attribute(("name", name));
    w.write_event(Event::Start(start)).map_err(io_err)?;
    w.write_event(Event::Start(BytesStart::new("gen:value"))).map_err(io_err)?;
    w.write_event(Event::Text(BytesText::new(text))).map_err(io_err)?;
    w.write_event(Event::End(BytesEnd::new("gen:value"))).map_err(io_err)?;
    w.write_event(Event::End(BytesEnd::new(format!("gen:{element}"))))
        .map_err(io_err)?;
    Ok(())
}

/// Write ONE scalar value under `name`. Returns true if it was written, false
/// if skipped (caller counts). Never errors on unrepresentable data.
fn write_one<W: Write>(w: &mut Writer<W>, name: &str, v: &Value) -> Result<bool> {
    let kind = value_kind(v);
    // String-shaped values need an XML-writability check.
    if let Value::String(s) = v {
        if is_unwritable_string(s) {
            return Ok(false);
        }
    }
    let text = scalar_text(v);
    match kind {
        Kind::Unwritable => Ok(false),
        Kind::StrList => unreachable!("lists are expanded by the caller"),
        Kind::Str => {
            match bldg_forced_kind(name) {
                Some(Kind::Str) => write_bldg_element(w, name, &text, None)?,
                _ => write_gen_element(w, "stringAttribute", name, &text)?,
            }
            Ok(true)
        }
        Kind::Date => {
            // Date-shaped strings always route to gen:dateAttribute (no typed
            // bldg: date attribute exists on Building).
            write_gen_element(w, "dateAttribute", name, &text)?;
            Ok(true)
        }
        Kind::Int => {
            match bldg_forced_kind(name) {
                // Only non-negative integers are schema-clean as storeys; a
                // negative one falls back to gen:intAttribute.
                Some(Kind::Int) if v.as_i64().is_some_and(|i| i >= 0) => {
                    write_bldg_element(w, name, &text, None)?
                }
                _ => write_gen_element(w, "intAttribute", name, &text)?,
            }
            Ok(true)
        }
        Kind::Float => {
            match bldg_forced_kind(name) {
                Some(Kind::Float) => write_bldg_element(w, name, &text, Some("m"))?,
                _ => write_gen_element(w, "doubleAttribute", name, &text)?,
            }
            Ok(true)
        }
    }
}

/// Serialise a building's attributes. See module docs for the routing rule.
pub fn write_attributes<W: Write>(
    w: &mut Writer<W>,
    attrs: &Map<String, Value>,
    report: &mut WriteReport,
) -> Result<usize> {
    let mut written = 0usize;
    for (name, value) in attrs {
        match value {
            // Writable string list: one element per item, same route each.
            Value::Array(items) if items.len() >= 2 && items.iter().all(Value::is_string) => {
                for item in items {
                    if write_one(w, name, item)? {
                        written += 1;
                        report.attributes_written += 1;
                    } else {
                        report.attributes_skipped += 1;
                    }
                }
            }
            // A bldg:function/usage array of non-strings, or any array reaching
            // here, is not a StringList; single-scalar path handles scalars,
            // everything else is unwritable.
            other => {
                if write_one(w, name, other)? {
                    written += 1;
                    report.attributes_written += 1;
                } else {
                    report.attributes_skipped += 1;
                }
            }
        }
    }
    Ok(written)
}
```

Remove the `report.attributes_written = report.attributes_written;` no-op line from the test helper if clippy objects; keep the `assert_eq!(n, report.attributes_written, …)` check.

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cargo test -p cityparquet --lib citygml::writer::attributes 2>&1 | tail -20`
Expected: PASS (all attribute tests green).

- [ ] **Step 6: Green gate + commit**

Run: `just check`
Expected: clean.

```bash
git add crates/cityparquet/src/citygml/writer/attributes.rs crates/cityparquet/src/citygml/writer/mod.rs
git commit -m "feat(citygml): W-M2 attribute routing (stored-type -> bldg:/gen:)"
```

---

### Task 2: Wire attributes into the Building + driver; extend the round-trip oracle

Attributes emit inside `bldg:Building` before geometry; an attributes-only Building now emits. The Ingolstadt round-trip oracle gains an attribute-map assertion.

**Files:**
- Modify: `crates/cityparquet/src/citygml/writer/building.rs` (add `attributes` to `BuildingSolids`; buffer+emit attributes; emptiness rule; update existing unit tests)
- Modify: `crates/cityparquet/src/citygml/writer/mod.rs` (driver: extract `obj.object.attributes` into `BuildingSolids`)
- Modify: `crates/cityparquet/tests/citygml_writer_real_data.rs` (compare attribute maps across the round trip)

**Interfaces:**
- Consumes: `write_attributes` (Task 1).
- Produces: `BuildingSolids` gains `pub attributes: serde_json::Map<String, serde_json::Value>`. `write_building` now returns `Ok(true)` when the building has at least one written attribute even with no solid.

- [ ] **Step 1: Write the failing test — attributes-only building emits**

In `building.rs` tests, add (the `BuildingSolids` literal will not yet have `attributes`, so this is red on the struct too):

```rust
    #[test]
    fn attributes_only_building_emits_with_no_solid() {
        let mut attributes = serde_json::Map::new();
        attributes.insert("roofType".into(), serde_json::json!("1000"));
        let b = BuildingSolids { id: "B5".into(), attributes, solids: vec![] };
        let mut report = WriteReport::default();
        let mut w = Writer::new(Vec::new());
        assert!(write_building(&mut w, &b, &mut Bounds::new(), &mut report).unwrap());
        let xml = String::from_utf8(w.into_inner()).unwrap();
        assert!(xml.contains("<bldg:Building gml:id=\"B5\">"));
        assert!(xml.contains("<bldg:roofType>1000</bldg:roofType>"));
        assert_eq!(report.attributes_written, 1);
    }

    #[test]
    fn empty_building_still_returns_false() {
        let b = BuildingSolids { id: "B6".into(), attributes: serde_json::Map::new(), solids: vec![] };
        let mut w = Writer::new(Vec::new());
        assert!(!write_building(&mut w, &b, &mut Bounds::new(), &mut WriteReport::default()).unwrap());
        assert!(w.into_inner().is_empty());
    }

    #[test]
    fn attributes_precede_geometry() {
        let mut attributes = serde_json::Map::new();
        attributes.insert("roofType".into(), serde_json::json!("1000"));
        let b = BuildingSolids {
            id: "B7".into(),
            attributes,
            solids: vec![(Lod::parse("2").unwrap(), tri_solid(), Some(solid_props()))],
        };
        let mut w = Writer::new(Vec::new());
        write_building(&mut w, &b, &mut Bounds::new(), &mut WriteReport::default()).unwrap();
        let xml = String::from_utf8(w.into_inner()).unwrap();
        assert!(xml.find("<bldg:roofType>").unwrap() < xml.find("<bldg:lod2Solid>").unwrap());
    }
```

- [ ] **Step 2: Update the struct and existing test literals**

In `building.rs`, add the field to `BuildingSolids`:

```rust
pub struct BuildingSolids {
    pub id: String,
    /// Building-level attributes (already decoded from the package's typed
    /// columns), emitted before geometry.
    pub attributes: serde_json::Map<String, Value>,
    pub solids: Vec<(Lod, DecodedGeometry, Option<Value>)>,
}
```

Add `attributes: serde_json::Map::new(),` to every existing `BuildingSolids { … }` literal in this file's tests (`B1`, `B2`, `B3`, `B4`, `3bad`).

- [ ] **Step 3: Run to verify failure**

Run: `cargo test -p cityparquet --lib citygml::writer::building 2>&1 | tail -20`
Expected: FAIL — the three new tests fail (attributes not yet emitted / emptiness rule not applied).

- [ ] **Step 4: Emit attributes in `write_building`**

Add the import at the top of `building.rs`:

```rust
use super::attributes::write_attributes;
```

In `write_building`, after the non-finite check and before `w.write_event(Event::Start(BytesStart::new("cityObjectMember")))`, buffer the attributes and apply the emptiness rule:

```rust
    // Buffer attributes first so the emptiness decision can see whether any
    // attribute is actually writable (an attributes-only Building is valid, but
    // a Building with neither geometry nor a writable attribute is not emitted).
    let mut attr_buf = Writer::new(Vec::new());
    let attrs_written = write_attributes(&mut attr_buf, &b.attributes, report)?;

    if by_major.is_empty() && attrs_written == 0 {
        return Ok(false);
    }
```

Then, immediately after writing the `<bldg:Building gml:id="…">` start event and before the geometry loop, flush the attribute buffer:

```rust
    w.get_mut()
        .write_all(&attr_buf.into_inner())
        .map_err(io_err)?;
```

Change the early `if by_major.is_empty() { return Ok(false); }` (the existing one before emission) to remove it — the new combined check above replaces it. Keep the NCName and non-finite checks where they are.

- [ ] **Step 5: Thread attributes through the driver**

In `writer/mod.rs`, where the driver builds `BuildingSolids { id: obj.id, solids }`, extract the attribute map from the decoded object first:

```rust
                let attributes = obj
                    .object
                    .attributes
                    .as_ref()
                    .and_then(Value::as_object)
                    .cloned()
                    .unwrap_or_default();
                let building = BuildingSolids { id: obj.id, attributes, solids };
```

Add `use serde_json::Value;` to `mod.rs` if not already imported (it is used only here — confirm and add if missing).

- [ ] **Step 6: Run unit tests to verify pass**

Run: `cargo test -p cityparquet --lib citygml::writer 2>&1 | tail -20`
Expected: PASS.

- [ ] **Step 7: Write the failing round-trip attribute assertion**

In `crates/cityparquet/tests/citygml_writer_real_data.rs`, add a helper that reads each building's attribute map from a package and a second assertion in the existing test. Add near `solid_coords`:

```rust
/// Per Building `id`, its decoded attribute map, for round-trip comparison.
fn building_attributes(pkg: &Path) -> BTreeMap<String, serde_json::Map<String, serde_json::Value>> {
    let manifest: PackageManifest =
        serde_json::from_str(&fs::read_to_string(pkg.join("metadata.json")).unwrap()).unwrap();
    let meta = ParquetRecordBatchReaderBuilder::try_new(
        fs::File::open(pkg.join(&manifest.tables[0])).unwrap(),
    )
    .unwrap()
    .cityparquet_metadata()
    .unwrap();

    let mut map = BTreeMap::new();
    for name in &manifest.tables {
        let reader = ParquetRecordBatchReaderBuilder::try_new(fs::File::open(pkg.join(name)).unwrap())
            .unwrap()
            .build()
            .unwrap();
        for batch in reader {
            let batch = batch.unwrap();
            for obj in decode_batch(&batch, &meta).unwrap() {
                if obj.object.thetype != "Building" {
                    continue;
                }
                let attrs = obj
                    .object
                    .attributes
                    .as_ref()
                    .and_then(serde_json::Value::as_object)
                    .cloned()
                    .unwrap_or_default();
                map.insert(obj.id.clone(), attrs);
            }
        }
    }
    map
}
```

In `ingolstadt_lod2_solids_round_trip_gml_to_parquet_to_gml`, after the existing geometry assertion, add:

```rust
    // Attributes must survive the round trip too (measuredHeight/roofType/
    // storeysAboveGround + gen: string attributes on this fixture).
    let attrs_before = building_attributes(&pkg);
    let attrs_after = building_attributes(&pkg2);
    assert!(
        attrs_before.values().any(|m| !m.is_empty()),
        "the original package must carry building attributes"
    );
    assert_eq!(attrs_before, attrs_after, "Building attributes must survive the round trip");
    assert_eq!(report.attributes_skipped, 0, "Ingolstadt attributes are all representable");
```

- [ ] **Step 8: Run the round-trip test to verify it fails, then passes**

Run: `cargo test -p cityparquet --test citygml_writer_real_data 2>&1 | tail -30`
Expected: initially the assertion should already pass IF Steps 4–5 are complete; if this step is executed before them it FAILS on `attrs_before != attrs_after` / empty attrs. Confirm it PASSES now.

If it fails on a specific attribute (e.g. a value formatting mismatch), debug via `systematic-debugging`; the most likely culprits are float formatting or an escaping gap — both handled in Task 1.

- [ ] **Step 9: Green gate + commit**

Run: `just check`

```bash
git add crates/cityparquet/src/citygml/writer/building.rs crates/cityparquet/src/citygml/writer/mod.rs crates/cityparquet/tests/citygml_writer_real_data.rs
git commit -m "feat(citygml): W-M2 emit building attributes + round-trip oracle"
```

---

### Task 3: `write_composite_solid` + shared solid-body writer (`writer/geometry.rs`)

Factor the shell-emitting body of `write_solid` into a shared helper, then add `write_composite_solid`. Unit-tested with in-code faces. Makes `export::shell_faces_nested` `pub(crate)`.

**Files:**
- Modify: `crates/cityparquet/src/citygml/writer/geometry.rs` (factor `write_solid_shells`; add `write_composite_solid`; unit tests)
- Modify: `crates/cityparquet/src/export.rs` (make `shell_faces_nested` `pub(crate)`)

**Interfaces:**
- Consumes: `export::{shell_faces_flat, shell_faces_nested, partition_shells}` (all `pub(crate)`), `wkb_read::DecodedKind`.
- Produces: `pub fn write_composite_solid<W: std::io::Write>(w: &mut quick_xml::Writer<W>, coords: &[[f64; 3]], members: &[crate::wkb_read::DecodedKind], props: Option<&serde_json::Value>) -> crate::Result<()>` — emits `<gml:CompositeSolid>` of `<gml:solidMember><gml:Solid>…` per member. Requires `props.type == "CompositeSolid"`; errors on any other type, a zero-member slice, a non-`PolyhedralSurface` member, or a nested-count/member mismatch.

- [ ] **Step 1: Make `shell_faces_nested` visible**

In `crates/cityparquet/src/export.rs`, change `fn shell_faces_nested(` to `pub(crate) fn shell_faces_nested(`.

- [ ] **Step 2: Write the failing unit tests**

Append to the `#[cfg(test)] mod tests` in `geometry.rs`:

```rust
    use crate::wkb_read::DecodedKind;

    fn two_member_props() -> serde_json::Value {
        // Each solid: one shell of one face.
        serde_json::json!({ "type": "CompositeSolid", "solid_shell_faces": [[1], [1]] })
    }

    #[test]
    fn write_composite_solid_emits_a_member_per_solid() {
        let coords = vec![
            [0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [1.0, 1.0, 0.0], // member 0 face
            [2.0, 0.0, 0.0], [3.0, 0.0, 0.0], [3.0, 1.0, 0.0], // member 1 face
        ];
        let members = vec![
            DecodedKind::PolyhedralSurface(vec![vec![vec![0usize, 1, 2]]]),
            DecodedKind::PolyhedralSurface(vec![vec![vec![3usize, 4, 5]]]),
        ];
        let props = two_member_props();
        let xml = emit(|w| write_composite_solid(w, &coords, &members, Some(&props)));
        assert!(xml.starts_with("<gml:CompositeSolid>"), "{xml}");
        assert_eq!(xml.matches("<gml:solidMember>").count(), 2, "{xml}");
        assert_eq!(xml.matches("<gml:Solid>").count(), 2, "{xml}");
    }

    #[test]
    fn write_composite_solid_single_member_stays_composite() {
        let coords = vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [1.0, 1.0, 0.0]];
        let members = vec![DecodedKind::PolyhedralSurface(vec![vec![vec![0usize, 1, 2]]])];
        let props = serde_json::json!({ "type": "CompositeSolid", "solid_shell_faces": [[1]] });
        let xml = emit(|w| write_composite_solid(w, &coords, &members, Some(&props)));
        assert!(xml.starts_with("<gml:CompositeSolid>"));
        assert_eq!(xml.matches("<gml:solidMember>").count(), 1);
    }

    #[test]
    fn write_composite_solid_rejects_zero_members() {
        let members: Vec<DecodedKind> = vec![];
        let props = serde_json::json!({ "type": "CompositeSolid", "solid_shell_faces": [] });
        let mut w = Writer::new(Vec::new());
        assert!(write_composite_solid(&mut w, &[], &members, Some(&props)).is_err());
    }

    #[test]
    fn write_composite_solid_rejects_non_composite_type() {
        let coords = vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [1.0, 1.0, 0.0]];
        let members = vec![DecodedKind::PolyhedralSurface(vec![vec![vec![0usize, 1, 2]]])];
        let props = serde_json::json!({ "type": "MultiSolid", "solid_shell_faces": [[1]] });
        let mut w = Writer::new(Vec::new());
        assert!(write_composite_solid(&mut w, &coords, &members, Some(&props)).is_err());
    }

    #[test]
    fn write_composite_solid_rejects_member_count_mismatch() {
        let coords = vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [1.0, 1.0, 0.0]];
        let members = vec![DecodedKind::PolyhedralSurface(vec![vec![vec![0usize, 1, 2]]])];
        // counts claim 2 solids, only 1 member present.
        let props = serde_json::json!({ "type": "CompositeSolid", "solid_shell_faces": [[1], [1]] });
        let mut w = Writer::new(Vec::new());
        assert!(write_composite_solid(&mut w, &coords, &members, Some(&props)).is_err());
    }
```

- [ ] **Step 3: Run to verify failure**

Run: `cargo test -p cityparquet --lib citygml::writer::geometry 2>&1 | tail -20`
Expected: FAIL — `write_composite_solid` undefined.

- [ ] **Step 4: Factor the shared solid body and implement `write_composite_solid`**

In `geometry.rs`, refactor `write_solid` so the shell-emitting body is a reusable helper. Replace the current body of `write_solid` (from `let counts = …` through the final `Ok(())`) so it delegates:

```rust
pub fn write_solid<W: Write>(
    w: &mut Writer<W>,
    coords: &[[f64; 3]],
    faces: &[Vec<Vec<usize>>],
    props: Option<&serde_json::Value>,
) -> Result<()> {
    let is_solid = props
        .and_then(|p| p.get("type"))
        .and_then(|t| t.as_str())
        == Some("Solid");
    if !is_solid {
        return Err(CityParquetError::Schema(
            "geometry_properties.type is not \"Solid\"; refusing to emit a gml:Solid from a \
             PolyhedralSurface without Solid shell metadata"
                .to_string(),
        ));
    }
    let counts = crate::export::shell_faces_flat(props)?;
    write_gml_solid(w, coords, faces, counts.as_deref())
}

/// Emit one `<gml:Solid>` from a flat face list + its shell partition counts,
/// with shell 0 exterior and shells 1.. interior. Shared by the top-level
/// `write_solid` and each `write_composite_solid` member.
fn write_gml_solid<W: Write>(
    w: &mut Writer<W>,
    coords: &[[f64; 3]],
    faces: &[Vec<Vec<usize>>],
    counts: Option<&[usize]>,
) -> Result<()> {
    let shells = crate::export::partition_shells(faces.to_vec(), counts)?;

    w.write_event(Event::Start(BytesStart::new("gml:Solid")))
        .map_err(io_err)?;
    let (exterior, interiors) = shells
        .split_first()
        .ok_or_else(|| CityParquetError::Geometry("solid has no shells to write".to_string()))?;
    w.write_event(Event::Start(BytesStart::new("gml:exterior")))
        .map_err(io_err)?;
    write_composite_surface(w, coords, exterior)?;
    w.write_event(Event::End(BytesEnd::new("gml:exterior")))
        .map_err(io_err)?;
    for shell in interiors {
        w.write_event(Event::Start(BytesStart::new("gml:interior")))
            .map_err(io_err)?;
        write_composite_surface(w, coords, shell)?;
        w.write_event(Event::End(BytesEnd::new("gml:interior")))
            .map_err(io_err)?;
    }
    w.write_event(Event::End(BytesEnd::new("gml:Solid")))
        .map_err(io_err)?;
    Ok(())
}

/// A `GeometryCollection` of `PolyhedralSurface` members (a CityJSON
/// `CompositeSolid`) -> `<gml:CompositeSolid>` of `<gml:solidMember>`-wrapped
/// `<gml:Solid>`s. Each member's shells come from the nested
/// `geometry_properties.solid_shell_faces[m]`. `MultiSolid` is NOT routed here
/// (CityGML 2.0 Building has no slot; the driver skips it).
pub fn write_composite_solid<W: Write>(
    w: &mut Writer<W>,
    coords: &[[f64; 3]],
    members: &[crate::wkb_read::DecodedKind],
    props: Option<&serde_json::Value>,
) -> Result<()> {
    let is_composite = props
        .and_then(|p| p.get("type"))
        .and_then(|t| t.as_str())
        == Some("CompositeSolid");
    if !is_composite {
        return Err(CityParquetError::Schema(
            "geometry_properties.type is not \"CompositeSolid\"; refusing to emit a \
             gml:CompositeSolid"
                .to_string(),
        ));
    }
    if members.is_empty() {
        return Err(CityParquetError::Geometry(
            "CompositeSolid has no solid members (gml:solidMember is minOccurs=1)".to_string(),
        ));
    }
    let nested = crate::export::shell_faces_nested(props)?;
    if let Some(counts) = &nested {
        if counts.len() != members.len() {
            return Err(CityParquetError::Geometry(format!(
                "solid_shell_faces lists {} solids but the CompositeSolid has {} members",
                counts.len(),
                members.len()
            )));
        }
    }

    w.write_event(Event::Start(BytesStart::new("gml:CompositeSolid")))
        .map_err(io_err)?;
    for (m, member) in members.iter().enumerate() {
        let crate::wkb_read::DecodedKind::PolyhedralSurface(faces) = member else {
            return Err(CityParquetError::Geometry(
                "CompositeSolid member is not a PolyhedralSurface".to_string(),
            ));
        };
        let counts = nested.as_ref().map(|c| c[m].as_slice());
        w.write_event(Event::Start(BytesStart::new("gml:solidMember")))
            .map_err(io_err)?;
        write_gml_solid(w, coords, faces, counts)?;
        w.write_event(Event::End(BytesEnd::new("gml:solidMember")))
            .map_err(io_err)?;
    }
    w.write_event(Event::End(BytesEnd::new("gml:CompositeSolid")))
        .map_err(io_err)?;
    Ok(())
}
```

- [ ] **Step 5: Run unit tests to verify pass**

Run: `cargo test -p cityparquet --lib citygml::writer::geometry 2>&1 | tail -20`
Expected: PASS (existing `write_solid` tests + new composite tests).

- [ ] **Step 6: Green gate + commit**

Run: `just check`

```bash
git add crates/cityparquet/src/citygml/writer/geometry.rs crates/cityparquet/src/export.rs
git commit -m "feat(citygml): W-M2 write_composite_solid + shared gml:Solid body"
```

---

### Task 4: Driver routing for CompositeSolid/MultiSolid + `write_building` dispatch + CompositeSolid oracle

The driver stops skipping CompositeSolid (carries it into `write_building`), skips MultiSolid with a counter, and the report counters are updated. `write_building` dispatches by geometry kind. A real-fixture round-trip oracle covers CompositeSolid.

**Files:**
- Modify: `crates/cityparquet/src/citygml/writer/mod.rs` (`WriteReport`: swap `composite_solids_skipped` → `composite_solids_written` + `multi_solids_skipped`; driver routing)
- Modify: `crates/cityparquet/src/citygml/writer/building.rs` (`write_building`: accept + emit CompositeSolid; extend non-finite check to composite members; count `composite_solids_written`)
- Create: `crates/cityparquet/tests/data/b1_lod2_cs_w_sem.gml` (copy of `tests/fixtures/b1_lod2_cs_w_sem.gml`)
- Create: `crates/cityparquet/tests/citygml_writer_composite.rs` (CompositeSolid round-trip oracle)

**Interfaces:**
- Consumes: `write_composite_solid` (Task 3); `BuildingSolids.solids` now carries `GeometryCollection` (CompositeSolid) entries too.
- Produces: `WriteReport` has `pub composite_solids_written: usize` and `pub multi_solids_skipped: usize` (replacing `composite_solids_skipped`).

- [ ] **Step 1: Copy the fixture into the crate's data dir**

```bash
cp tests/fixtures/b1_lod2_cs_w_sem.gml crates/cityparquet/tests/data/b1_lod2_cs_w_sem.gml
```

- [ ] **Step 2: Swap the report counters**

In `writer/mod.rs` `WriteReport`, replace:

```rust
    /// LoD columns skipped because the WKB was MultiSolid/CompositeSolid
    /// (`GeometryCollection`) — deferred to W-M2.
    pub composite_solids_skipped: usize,
```

with:

```rust
    /// `gml:CompositeSolid` geometries emitted.
    pub composite_solids_written: usize,
    /// MultiSolid geometry columns skipped: CityGML 2.0 `Building` has no
    /// `lodNMultiSolid` slot and `gml:MultiSolid` is not a `gml:_Solid`.
    pub multi_solids_skipped: usize,
```

- [ ] **Step 3: Write the failing MultiSolid-skip unit test (driver behaviour via write_building)**

Because the driver's routing is hard to unit-test in isolation, test the two behaviours at the `write_building` boundary plus a small driver-routing helper. First, in `building.rs` tests, add a CompositeSolid emit test and a stray-MultiSolid guard:

```rust
    fn composite_props() -> Value {
        serde_json::json!({ "type": "CompositeSolid", "solid_shell_faces": [[1]] })
    }

    fn composite_geom() -> DecodedGeometry {
        DecodedGeometry {
            coords: vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [1.0, 1.0, 0.0]],
            kind: DecodedKind::GeometryCollection(vec![DecodedKind::PolyhedralSurface(vec![vec![
                vec![0usize, 1, 2],
            ]])]),
        }
    }

    #[test]
    fn composite_solid_is_emitted_and_counted() {
        let b = BuildingSolids {
            id: "C1".into(),
            attributes: serde_json::Map::new(),
            solids: vec![(Lod::parse("2").unwrap(), composite_geom(), Some(composite_props()))],
        };
        let mut report = WriteReport::default();
        let mut w = Writer::new(Vec::new());
        assert!(write_building(&mut w, &b, &mut Bounds::new(), &mut report).unwrap());
        let xml = String::from_utf8(w.into_inner()).unwrap();
        assert!(xml.contains("<bldg:lod2Solid><gml:CompositeSolid>"), "{xml}");
        assert_eq!(report.composite_solids_written, 1);
    }
```

- [ ] **Step 4: Run to verify failure**

Run: `cargo test -p cityparquet --lib citygml::writer::building 2>&1 | tail -20`
Expected: FAIL — `write_building` rejects `GeometryCollection` (only PolyhedralSurface reaches `by_major`) and `composite_solids_written` doesn't exist.

- [ ] **Step 5: Make `write_building` accept + emit CompositeSolid**

In `building.rs`:

Add to imports: `use super::geometry::{write_composite_solid, write_solid};`

In the `by_major` gathering loop, widen the representable check:

```rust
    for (lod, geom, props) in &b.solids {
        let representable = matches!(
            geom.kind,
            DecodedKind::PolyhedralSurface(_) | DecodedKind::GeometryCollection(_)
        );
        if !representable {
            report.lod_columns_skipped += 1;
            continue;
        }
        let major = lod.major();
        // … existing (1..=4) check and highest-minor dedup, unchanged …
    }
```

Extend the non-finite coordinate check to also scan composite members. Replace the current non-finite loop body so it checks all of `geom.coords` referenced by either a `PolyhedralSurface` or a `GeometryCollection`. The simplest correct form scans the full coord pool of each candidate geometry (WKB-decoded geometry has no orphan coords):

```rust
    for (_, (_, geom, _)) in &by_major {
        if geom.coords.iter().any(|c| c.iter().any(|v| !v.is_finite())) {
            return Err(CityParquetError::Geometry(format!(
                "building {:?} has a non-finite coordinate; cannot serialise as gml:posList",
                b.id
            )));
        }
    }
```

In the emit loop, dispatch by kind:

```rust
    for (major, (_, geom, props)) in &by_major {
        let elem = format!("bldg:lod{major}Solid");
        w.write_event(Event::Start(BytesStart::new(elem.as_str())))
            .map_err(io_err)?;
        match &geom.kind {
            DecodedKind::PolyhedralSurface(faces) => {
                write_solid(w, &geom.coords, faces, *props)?;
            }
            DecodedKind::GeometryCollection(members) => {
                write_composite_solid(w, &geom.coords, members, *props)?;
                report.composite_solids_written += 1;
            }
            _ => unreachable!("only PolyhedralSurface/GeometryCollection reach by_major"),
        }
        for c in &geom.coords {
            bounds.add(*c);
        }
        w.write_event(Event::End(BytesEnd::new(elem.as_str())))
            .map_err(io_err)?;
    }
```

(Bounds now accumulates over the geometry's coord pool directly, which equals the referenced set for WKB-decoded geometry and covers both solids and composites.)

- [ ] **Step 6: Route CompositeSolid/MultiSolid in the driver**

In `writer/mod.rs`, replace the geometry-classification `match` in the per-row loop:

```rust
                let mut solids = Vec::new();
                for (lod, decoded, props) in obj.geometries {
                    match (&decoded.kind, lod) {
                        (DecodedKind::GeometryCollection(_), Some(lod)) => {
                            let is_multi = props
                                .as_ref()
                                .and_then(|p| p.get("type"))
                                .and_then(|t| t.as_str())
                                == Some("MultiSolid");
                            if is_multi {
                                // No CityGML 2.0 Building slot for a MultiSolid.
                                report.multi_solids_skipped += 1;
                            } else {
                                solids.push((lod, decoded, props));
                            }
                        }
                        // A lodless geometry cannot be a lod<n>Solid.
                        (_, None) => report.lod_columns_skipped += 1,
                        (_, Some(lod)) => solids.push((lod, decoded, props)),
                    }
                }
```

- [ ] **Step 7: Run unit tests to verify pass**

Run: `cargo test -p cityparquet --lib citygml::writer 2>&1 | tail -20`
Expected: PASS.

- [ ] **Step 8: Write the failing CompositeSolid round-trip oracle**

Create `crates/cityparquet/tests/citygml_writer_composite.rs`:

```rust
//! W-M2 CompositeSolid round-trip oracle over a real fixture.
//!
//! `b1_lod2_cs_w_sem.gml` (one Building whose `bldg:lod2Solid` is a 2-member
//! `gml:CompositeSolid`) is converted to a package, written back to `.gml`, and
//! re-converted. Equal stored geometry across the round trip, keyed by
//! `(building_id, major_lod)`, proves the CompositeSolid survived.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

use cityparquet::citygml::writer::{WriteOptions, write_package};
use cityparquet::decode::decode_batch;
use cityparquet::package::{ConvertOptions, convert};
use cityparquet::reader::CityParquetReaderBuilder;
use cityparquet::schema::PackageManifest;
use cityparquet::wkb_read::DecodedKind;
use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;

fn fixture() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/b1_lod2_cs_w_sem.gml")
}

fn mm(v: f64) -> i64 {
    (v * 1000.0).round() as i64
}

/// Per Building `id`, per major LoD, the distinct world coords of its
/// CompositeSolid (`GeometryCollection` of `PolyhedralSurface`) on the 1 mm grid.
fn composite_coords(pkg: &Path) -> BTreeMap<(String, u8), BTreeSet<(i64, i64, i64)>> {
    let manifest: PackageManifest =
        serde_json::from_str(&fs::read_to_string(pkg.join("metadata.json")).unwrap()).unwrap();
    let meta = ParquetRecordBatchReaderBuilder::try_new(
        fs::File::open(pkg.join(&manifest.tables[0])).unwrap(),
    )
    .unwrap()
    .cityparquet_metadata()
    .unwrap();

    let mut map: BTreeMap<(String, u8), BTreeSet<(i64, i64, i64)>> = BTreeMap::new();
    for name in &manifest.tables {
        let reader = ParquetRecordBatchReaderBuilder::try_new(fs::File::open(pkg.join(name)).unwrap())
            .unwrap()
            .build()
            .unwrap();
        for batch in reader {
            let batch = batch.unwrap();
            for obj in decode_batch(&batch, &meta).unwrap() {
                if obj.object.thetype != "Building" {
                    continue;
                }
                for (lod, decoded, _props) in &obj.geometries {
                    if matches!(decoded.kind, DecodedKind::GeometryCollection(_)) {
                        let Some(major) =
                            lod.as_ref().map(|l| l.major()).filter(|m| (1..=4).contains(m))
                        else {
                            continue;
                        };
                        let entry = map.entry((obj.id.clone(), major)).or_default();
                        for c in &decoded.coords {
                            entry.insert((mm(c[0]), mm(c[1]), mm(c[2])));
                        }
                    }
                }
            }
        }
    }
    map
}

#[test]
fn b1_composite_solid_round_trips_gml_to_parquet_to_gml() {
    let tmp = tempfile::tempdir().unwrap();
    let pkg = tmp.path().join("pkg");
    let out_gml = tmp.path().join("out.gml");
    let pkg2 = tmp.path().join("pkg2");

    convert(&ConvertOptions::new(fixture(), pkg.clone())).unwrap();
    let report =
        write_package(&WriteOptions { package_dir: pkg.clone(), output: out_gml.clone() }).unwrap();
    assert_eq!(report.composite_solids_written, 1, "one CompositeSolid expected");
    assert_eq!(report.multi_solids_skipped, 0);

    convert(&ConvertOptions::new(out_gml.clone(), pkg2.clone())).unwrap();

    let before = composite_coords(&pkg);
    let after = composite_coords(&pkg2);
    assert!(!before.is_empty(), "the original package must have a CompositeSolid");
    assert_eq!(before, after, "CompositeSolid coordinates must survive the round trip");
}
```

- [ ] **Step 9: Run the round-trip oracle to verify it passes**

Run: `cargo test -p cityparquet --test citygml_writer_composite 2>&1 | tail -30`
Expected: PASS. If the reader collapses a single-member CompositeSolid or the member/shell partition differs, debug via `systematic-debugging` (compare `before`/`after` maps).

- [ ] **Step 10: Green gate + commit**

Run: `just check`

```bash
git add crates/cityparquet/src/citygml/writer/mod.rs crates/cityparquet/src/citygml/writer/building.rs crates/cityparquet/tests/data/b1_lod2_cs_w_sem.gml crates/cityparquet/tests/citygml_writer_composite.rs
git commit -m "feat(citygml): W-M2 CompositeSolid routing + real-fixture round-trip oracle"
```

---

## Final milestone steps (after Task 4)

- [ ] **Whole-branch review** via superpowers:requesting-code-review (final reviewer, most-capable model).
- [ ] **Codex external review** (standing project instruction): `codex exec --cd "$(pwd)" --sandbox read-only "Review the W-M2 CityGML writer changes on branch feat/citygml2-writer-wm2 for correctness, round-trip losslessness, and CityGML 2.0 conformance."` — triage and fix Criticals/Importants.
- [ ] **Finish the branch** via superpowers:finishing-a-development-branch (verify `just check` green, then merge to `main`).
- [ ] Update the milestone memory (`cityparquet-rs-milestones.md`) to record W-M2 done and W-M3 scope (semantic surfaces / BuildingParts / appearance).

## Self-Review notes

- **Spec coverage:** attribute routing (Task 1), attribute wiring + oracle (Task 2), CompositeSolid writer (Task 3), driver routing + MultiSolid skip + CompositeSolid oracle (Task 4). All spec sections A–E covered; MultiSolid skip counter + attributes-only emit + counter swap present.
- **Type consistency:** `BuildingSolids` gains `attributes` (Task 2) then carries composites in `solids` (Task 4); `WriteReport` gains attribute counters (Task 1) and swaps the composite counter (Task 4); `write_composite_solid` signature identical in Tasks 3 and 4; `write_gml_solid` shared helper introduced once (Task 3).
- **No placeholders:** every code step shows complete code; commands have expected outcomes.
