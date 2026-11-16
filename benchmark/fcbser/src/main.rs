//! `fcb-ser-stream INPUT.city.jsonl OUTPUT.fcb` — what `fcb ser -A INPUT OUTPUT`
//! writes, without holding the input in memory.
//!
//! `fcb ser` (fcb_cli 0.7.8) parses every feature of its input into a
//! `Vec<CityJSONFeature>` before it writes one. That is several times the
//! size of the CityJSONSeq text, and the national 3DBAG stream (66 GB,
//! 10.8 million features) does not fit in 503 GB that way. This tool drives the
//! same `fcb_core::FcbWriter` with the same settings and reads the input
//! twice instead:
//!
//! 1. count the features and derive the two schemas exactly as the CLI does —
//!    the attribute schema from the first 1000 features, the semantic-surface
//!    attribute schema from all of them;
//! 2. hand the features to the writer one at a time.
//!
//! The output is byte-identical to `fcb ser -A` on any input the CLI can
//! still read (`benchmark/scripts/tests/fcb_ser_stream_test.sh` proves it), so
//! the benchmark's one FlatCityBuf configuration is unchanged. Only the
//! parsed input is streamed: `FcbWriter::write` still assembles the feature
//! section and the attribute indices in memory, so peak memory is roughly the
//! size of the `.fcb` plus its index entries.

use std::fs::File;
use std::io::{BufRead, BufReader, BufWriter, Write};

use anyhow::{Context, Result, bail};
use cjseq::{CityJSON, CityJSONFeature};
use fcb_core::FcbWriter;
use fcb_core::attribute::{AttributeSchema, AttributeSchemaMethods};
use fcb_core::header_writer::HeaderWriterOptions;

/// `fcb ser`'s schema sample: the attribute schema is built from this many
/// leading features, no more.
const SCHEMA_SAMPLE: usize = 1000;
/// `fcb ser -A`'s attribute B+-tree branching factor.
const BRANCHING_FACTOR: u16 = 256;
/// `fcb ser`'s default R-tree node size.
const INDEX_NODE_SIZE: u16 = 16;

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [input, output] = args.as_slice() else {
        bail!("usage: fcb-ser-stream INPUT.city.jsonl OUTPUT.fcb");
    };

    let (metadata, attr_schema, semantic_attr_schema, feature_count) = scan(input)?;
    eprintln!("fcb-ser-stream: {feature_count} features");

    let attribute_indices = attr_schema.as_ref().map(|schema| {
        schema
            .keys()
            .map(|name| (name.clone(), Some(BRANCHING_FACTOR)))
            .collect::<Vec<_>>()
    });
    let header_options = HeaderWriterOptions {
        write_index: true,
        feature_count,
        index_node_size: INDEX_NODE_SIZE,
        attribute_indices,
        geographical_extent: None,
    };

    let mut fcb = FcbWriter::new(
        metadata,
        Some(header_options),
        attr_schema,
        semantic_attr_schema,
    )?;
    let mut previous: Option<Box<CityJSONFeature>> = None;
    let mut written = 0u64;
    for feature in features(input)?.1 {
        let feature = Box::new(feature?);
        add_streamed(&mut fcb, &feature)?;
        // The writer now points at `feature`; the one before it is no longer
        // referenced and can go.
        previous = Some(feature);
        written += 1;
        if written.is_multiple_of(1_000_000) {
            eprintln!("fcb-ser-stream: {written}/{feature_count} features encoded");
        }
    }
    if written != feature_count {
        bail!("{input} changed between the two passes: {feature_count} features, then {written}");
    }

    let mut out = BufWriter::new(File::create(output).with_context(|| format!("create {output}"))?);
    fcb.write(&mut out)?;
    out.flush()?;
    // Only now: `write` consumed the writer, so nothing refers to the last
    // feature any more.
    drop(previous);
    Ok(())
}

/// Adds one feature to a writer that outlives it.
///
/// `FcbWriter::add_feature` takes `&'a CityJSONFeature` for the writer's whole
/// lifetime, which is what forces `fcb ser` to keep every feature alive. In
/// `fcb_core` 0.7.8 the writer keeps exactly one such reference — its
/// `FeatureWriter::city_feature` — and `add_feature` overwrites it before
/// serialising, then never reads it again until the next call (`write` does
/// not read it at all). So a feature only has to live until the next
/// `add_feature`, or until `write` has returned for the last one. The caller
/// upholds that; the exact `=0.7.8` pin is what keeps the invariant true.
fn add_streamed<'a>(fcb: &mut FcbWriter<'a>, feature: &CityJSONFeature) -> Result<()> {
    // SAFETY: see above — the reference is replaced before `feature` is
    // dropped, and is never dereferenced after that.
    let feature: &'a CityJSONFeature = unsafe { &*std::ptr::from_ref(feature) };
    fcb.add_feature(feature)?;
    Ok(())
}

type Features = Box<dyn Iterator<Item = Result<CityJSONFeature>>>;

/// The stream's metadata line and an iterator over its features. Blank lines
/// are skipped, as `fcb ser` skips them.
fn features(input: &str) -> Result<(CityJSON, Features)> {
    let file = File::open(input).with_context(|| format!("open {input}"))?;
    let mut lines = BufReader::with_capacity(1 << 20, file).lines();
    let first = lines
        .next()
        .with_context(|| format!("{input} is empty"))??;
    let metadata: CityJSON = serde_json::from_str(&first).context("parse the metadata line")?;
    let rest = lines.enumerate().filter_map(|(index, line)| match line {
        Err(error) => Some(Err(error.into())),
        Ok(line) if line.trim().is_empty() => None,
        Ok(line) => Some(
            serde_json::from_str::<CityJSONFeature>(&line)
                .with_context(|| format!("parse the feature on line {}", index + 2)),
        ),
    });
    Ok((metadata, Box::new(rest)))
}

/// Pass one: the metadata, both schemas and the feature count, each derived
/// the way `fcb_cli`'s `serialize` derives it.
fn scan(
    input: &str,
) -> Result<(
    CityJSON,
    Option<AttributeSchema>,
    Option<AttributeSchema>,
    u64,
)> {
    let (metadata, features) = features(input)?;
    let mut attr_schema = AttributeSchema::new();
    let mut semantic_attr_schema = AttributeSchema::new();
    let mut count = 0u64;
    for feature in features {
        let feature = feature?;
        // Sorted ids: `add_attributes` hands each new attribute the next free
        // column index, so the visiting order is part of the output.
        let mut ids: Vec<&String> = feature.city_objects.keys().collect();
        ids.sort_unstable();
        for object in ids
            .into_iter()
            .filter_map(|id| feature.city_objects.get(id))
        {
            if (count as usize) < SCHEMA_SAMPLE
                && let Some(attributes) = &object.attributes
            {
                attr_schema.add_attributes(attributes);
            }
            for geometry in object.geometry.iter().flatten() {
                let Some(semantics) = geometry.common().and_then(|c| c.semantics.as_ref()) else {
                    continue;
                };
                for surface in &semantics.surfaces {
                    if !surface.other.is_empty() {
                        let other =
                            serde_json::Value::Object(surface.other.clone().into_iter().collect());
                        semantic_attr_schema.add_attributes(&other);
                    }
                }
            }
        }
        count += 1;
        if count.is_multiple_of(1_000_000) {
            eprintln!("fcb-ser-stream: {count} features scanned");
        }
    }
    let non_empty = |schema: AttributeSchema| (!schema.is_empty()).then_some(schema);
    Ok((
        metadata,
        non_empty(attr_schema),
        non_empty(semantic_attr_schema),
        count,
    ))
}
