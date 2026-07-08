//! Writer-property presets: the paper's benchmark variable, rendered as
//! per-column `parquet::file::properties::WriterProperties`.
//!
//! [`WriterRecipe`] never hardcodes column names beyond the six fixed `bbox`
//! leaf paths — every other per-column decision (geometry columns, JSON-typed
//! columns) is derived from [`CityParquetSchema::to_arrow_schema`]'s field
//! names and extension metadata, so it can never drift from the schema it is
//! given.

use arrow_schema::Field;
use arrow_schema::extension::EXTENSION_TYPE_NAME_KEY;
use parquet::basic::{Compression, Encoding, ZstdLevel};
use parquet::file::metadata::KeyValue;
use parquet::file::properties::{EnabledStatistics, WriterProperties};
use parquet::schema::types::ColumnPath;

use cityparquet_schema::{CityParquetError, CityParquetMetadata, CityParquetSchema, Result};

/// The six fixed `bbox` struct leaves, in the order [`crate::scan`]'s bbox
/// union rule and [`cityparquet_schema::model::bbox_data_type`] both use.
const BBOX_LEAVES: [&str; 6] = ["xmin", "ymin", "zmin", "xmax", "ymax", "zmax"];

/// Extension type name tagging every column whose values are JSON text
/// (`geometry_properties*`, `material`, `texture`, `other`, `Json`-typed
/// attributes) — see [`cityparquet_schema::model`]'s `json_field` helper.
const ARROW_JSON_EXTENSION: &str = "arrow.json";

/// A named writer-property preset — the paper's benchmark variable.
///
/// `CityParquet` is the tuned default this crate has always written; the
/// rest are ablations against it (or, for `ParquetDefaults`, the complete
/// absence of tuning) so the paper can quantify what each rule buys.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecipePreset {
    /// The tuned default: delta ids, dictionary `object_type`, BYTE_STREAM_SPLIT
    /// bbox, no stats/dictionary on WKB+JSON, zstd 3.
    CityParquet,
    /// parquet-rs defaults + the recipe's global compression and row-group
    /// size ONLY — no per-column tuning at all. The "untuned writer"
    /// comparator; KV metadata is still embedded (never a preset variable).
    ParquetDefaults,
    /// CityParquet minus dictionary encoding everywhere (ablation).
    NoDictionary,
    /// CityParquet minus BYTE_STREAM_SPLIT on the bbox leaves (ablation);
    /// the leaves keep their chunk statistics and disabled dictionary.
    NoByteStreamSplit,
    /// CityParquet minus DELTA_BYTE_ARRAY on `id`/`feature_id` (ablation).
    /// Without delta encoding there is no reason to also force dictionary
    /// off for these columns, so they fall back to pure parquet defaults
    /// (dictionary enabled) rather than an encoding-less, dictionary-less
    /// no-man's-land.
    NoDelta,
    /// CityParquet with Snappy instead of zstd (DuckDB COPY's default codec).
    Snappy,
}

impl RecipePreset {
    /// Every preset, in the order they are named — used to drive the
    /// per-preset round-trip gate and to enumerate valid `--recipe` values.
    pub const ALL: [RecipePreset; 6] = [
        RecipePreset::CityParquet,
        RecipePreset::ParquetDefaults,
        RecipePreset::NoDictionary,
        RecipePreset::NoByteStreamSplit,
        RecipePreset::NoDelta,
        RecipePreset::Snappy,
    ];

    /// The stable, kebab-case name used on the CLI and in benchmark output.
    pub fn name(&self) -> &'static str {
        match self {
            RecipePreset::CityParquet => "cityparquet",
            RecipePreset::ParquetDefaults => "parquet-defaults",
            RecipePreset::NoDictionary => "no-dictionary",
            RecipePreset::NoByteStreamSplit => "no-bss",
            RecipePreset::NoDelta => "no-delta",
            RecipePreset::Snappy => "snappy",
        }
    }

    /// Parses one of [`RecipePreset::name`]'s exact strings; `None` for
    /// anything else (the caller decides how to report the invalid name).
    pub fn parse(s: &str) -> Option<RecipePreset> {
        RecipePreset::ALL
            .into_iter()
            .find(|preset| preset.name() == s)
    }

    /// The default [`WriterRecipe`] for this preset (row-group size 65536,
    /// zstd level 3 — ignored by `Snappy` — `statistics_for_json` off).
    pub fn recipe(&self) -> WriterRecipe {
        WriterRecipe {
            preset: *self,
            ..WriterRecipe::default()
        }
    }
}

/// A named preset of Parquet `WriterProperties`, parameterised over the
/// handful of knobs the paper's benchmark actually varies.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WriterRecipe {
    pub row_group_size: usize,
    /// Ignored when `preset` is [`RecipePreset::Snappy`], which always
    /// compresses with Snappy regardless of this value.
    pub zstd_level: i32,
    /// Whether JSON-typed columns (geometry properties, material, texture,
    /// `other`, `Json`-typed attributes) still get column statistics.
    /// Off by default: statistics on serialised JSON blobs are not useful for
    /// predicate pushdown and only cost write time.
    pub statistics_for_json: bool,
    /// Which named preset's per-column rules to render. Defaults to
    /// [`RecipePreset::CityParquet`], preserving this struct's pre-M5
    /// behaviour when constructed with `..WriterRecipe::default()`.
    pub preset: RecipePreset,
}

impl Default for WriterRecipe {
    fn default() -> Self {
        Self {
            row_group_size: 65536,
            zstd_level: 3,
            statistics_for_json: false,
            preset: RecipePreset::CityParquet,
        }
    }
}

/// Extension type name tagging every WKB geometry column — see
/// [`cityparquet_schema::model`]'s `geometry_field` (via
/// `geoarrow_schema::WkbType`).
const GEOARROW_WKB_EXTENSION: &str = "geoarrow.wkb";

/// A WKB geometry column, detected by its `geoarrow.wkb` extension metadata —
/// never by name, so an attribute that merely happens to be called
/// `geometry_extra` keeps attribute defaults. `geometry_properties*` JSON
/// siblings carry `arrow.json` instead and are handled by the JSON rule.
fn is_geometry_column(field: &Field) -> bool {
    field
        .metadata()
        .get(EXTENSION_TYPE_NAME_KEY)
        .map(String::as_str)
        == Some(GEOARROW_WKB_EXTENSION)
}

/// A column tagged with the canonical `arrow.json` Arrow extension type.
fn is_json_column(field: &Field) -> bool {
    field
        .metadata()
        .get(EXTENSION_TYPE_NAME_KEY)
        .map(String::as_str)
        == Some(ARROW_JSON_EXTENSION)
}

impl WriterRecipe {
    /// Render this recipe into concrete `WriterProperties` for `schema`,
    /// embedding `metadata` (plus the derived GeoParquet `geo` key, iff
    /// `geoarrow`) as Parquet file-level key-value metadata.
    pub fn writer_properties(
        &self,
        schema: &CityParquetSchema,
        metadata: &CityParquetMetadata,
        geoarrow: bool,
    ) -> Result<WriterProperties> {
        // Stays TAGGED (zero-arg `to_arrow_schema`): used only to detect
        // geometry columns for the per-column properties below, independent
        // of whether the `geo` key itself is emitted.
        let arrow_schema = schema.to_arrow_schema()?;

        let mut kvs: Vec<KeyValue> = metadata
            .to_key_values()?
            .into_iter()
            .map(|(key, value)| KeyValue::new(key, value))
            .collect();
        if geoarrow {
            kvs.push(KeyValue::new(
                "geo".to_string(),
                metadata.geoparquet_geo_value()?.to_string(),
            ));
        }

        // `Snappy` compresses with Snappy and ignores `zstd_level`; every
        // other preset compresses with zstd at `zstd_level`.
        let compression = if self.preset == RecipePreset::Snappy {
            Compression::SNAPPY
        } else {
            let zstd_level = ZstdLevel::try_new(self.zstd_level).map_err(|e| {
                CityParquetError::Schema(format!("invalid zstd level {}: {e}", self.zstd_level))
            })?;
            Compression::ZSTD(zstd_level)
        };

        let mut builder = WriterProperties::builder()
            .set_compression(compression)
            // `set_max_row_group_size` is deprecated in parquet 58 in favour of
            // this row-count-only setter (semantically identical here: we
            // never set a row-group byte cap).
            .set_max_row_group_row_count(Some(self.row_group_size))
            .set_key_value_metadata(Some(kvs));

        // `NoDictionary` disables dictionary encoding globally: this is the
        // fallback every column without an explicit per-column dictionary
        // setting (e.g. plain attributes) resolves to. Per-column settings
        // below still take precedence over this default, so the object_type
        // rule is threaded through `preset` explicitly rather than relying
        // on the global default to win.
        if self.preset == RecipePreset::NoDictionary {
            builder = builder.set_dictionary_enabled(false);
        }

        // `ParquetDefaults` is the "untuned writer" comparator: no per-column
        // tuning at all, only global compression + row-group size + KV
        // metadata (already set above). Return before any of the CityParquet
        // per-column rules below are applied.
        if self.preset == RecipePreset::ParquetDefaults {
            return Ok(builder.build());
        }

        builder = builder
            .set_column_dictionary_enabled(
                ColumnPath::from("object_type"),
                self.preset != RecipePreset::NoDictionary,
            )
            .set_column_statistics_enabled(
                ColumnPath::from("object_type"),
                EnabledStatistics::Chunk,
            );

        // `NoDelta`: drop both the DELTA_BYTE_ARRAY encoding and the
        // dictionary-off override for id/feature_id, so they fall back to
        // pure parquet defaults (dictionary on, no explicit encoding) —
        // see the variant's doc comment for why dictionary-off alone would
        // be the wrong ablation.
        if self.preset != RecipePreset::NoDelta {
            for name in ["id", "feature_id"] {
                builder = builder
                    .set_column_encoding(ColumnPath::from(name), Encoding::DELTA_BYTE_ARRAY)
                    .set_column_dictionary_enabled(ColumnPath::from(name), false);
            }
        }

        for leaf in BBOX_LEAVES {
            // `ColumnPath::from(String)` does NOT split on `.` — it produces
            // a single-part path. The physical column's path is the nested
            // struct path `["bbox", leaf]`, so the path must be built with
            // `ColumnPath::new` over the two parts, or these properties are
            // silently never applied (dead code, keyed to a path that never
            // matches any real column).
            let path = ColumnPath::new(vec!["bbox".to_string(), leaf.to_string()]);
            builder = builder
                .set_column_statistics_enabled(path.clone(), EnabledStatistics::Chunk)
                .set_column_dictionary_enabled(path.clone(), false);
            // `NoByteStreamSplit`: keep the stats + dictionary-off above,
            // drop only the BSS encoding itself.
            if self.preset != RecipePreset::NoByteStreamSplit {
                builder = builder.set_column_encoding(path, Encoding::BYTE_STREAM_SPLIT);
            }
        }

        for field in arrow_schema.fields() {
            let name = field.name().as_str();
            if is_geometry_column(field) {
                let path = ColumnPath::from(name);
                builder = builder
                    .set_column_dictionary_enabled(path.clone(), false)
                    .set_column_statistics_enabled(path, EnabledStatistics::None);
                continue;
            }
            if is_json_column(field) && !self.statistics_for_json {
                builder = builder
                    .set_column_statistics_enabled(ColumnPath::from(name), EnabledStatistics::None);
            }
        }

        Ok(builder.build())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cityparquet_schema::{AttributeType, CITYPARQUET_VERSION, Lod, SourceFormat};
    use parquet::basic::Compression;

    fn sample_schema() -> CityParquetSchema {
        CityParquetSchema {
            lods: vec![Lod::parse("2.2").unwrap()],
            attributes: vec![("yoc".to_string(), AttributeType::Int64)],
            crs: None,
        }
    }

    fn sample_metadata() -> CityParquetMetadata {
        CityParquetMetadata {
            cityparquet_version: CITYPARQUET_VERSION.to_string(),
            source_format: SourceFormat::CityJsonSeq,
            source_version: None,
            crs: None,
            transform: None,
            extensions: None,
            attribute_columns: vec!["yoc".to_string()],
            reserved_columns: vec!["id".to_string(), "object_type".to_string()],
            default_geometry: "geometry_lod2_2".to_string(),
            bbox_column: "bbox".to_string(),
            sidecar_files: vec![],
            source_metadata: None,
            appearance_defaults: None,
        }
    }

    #[test]
    fn recipe_renders_the_binding_per_column_rules() {
        let schema = sample_schema();
        let metadata = sample_metadata();
        let props = WriterRecipe::default()
            .writer_properties(&schema, &metadata, true)
            .unwrap();

        // id / feature_id: DELTA_BYTE_ARRAY, dictionary off.
        assert!(!props.dictionary_enabled(&ColumnPath::from("id")));
        assert_eq!(
            props.encoding(&ColumnPath::from("id")),
            Some(Encoding::DELTA_BYTE_ARRAY)
        );
        assert!(!props.dictionary_enabled(&ColumnPath::from("feature_id")));
        assert_eq!(
            props.encoding(&ColumnPath::from("feature_id")),
            Some(Encoding::DELTA_BYTE_ARRAY)
        );

        // object_type: dictionary explicitly on, chunk statistics.
        assert!(props.dictionary_enabled(&ColumnPath::from("object_type")));
        assert_eq!(
            props.statistics_enabled(&ColumnPath::from("object_type")),
            EnabledStatistics::Chunk
        );

        // bbox leaves: BYTE_STREAM_SPLIT, chunk statistics, dictionary off.
        // Queried with the same nested-struct `ColumnPath::new(["bbox", leaf])`
        // the recipe now writes properties under — `ColumnPath::from` a
        // dotted string does NOT split on `.` and would silently miss these.
        let bbox_xmin = ColumnPath::new(vec!["bbox".to_string(), "xmin".to_string()]);
        let bbox_zmax = ColumnPath::new(vec!["bbox".to_string(), "zmax".to_string()]);
        assert_eq!(
            props.encoding(&bbox_xmin),
            Some(Encoding::BYTE_STREAM_SPLIT)
        );
        assert_eq!(
            props.statistics_enabled(&bbox_xmin),
            EnabledStatistics::Chunk
        );
        assert!(!props.dictionary_enabled(&bbox_xmin));
        assert_eq!(
            props.encoding(&bbox_zmax),
            Some(Encoding::BYTE_STREAM_SPLIT)
        );

        // geometry columns: dictionary off, no statistics.
        assert!(!props.dictionary_enabled(&ColumnPath::from("geometry_lod2_2")));
        assert_eq!(
            props.statistics_enabled(&ColumnPath::from("geometry_lod2_2")),
            EnabledStatistics::None
        );

        // arrow.json-tagged columns: no statistics by default.
        assert_eq!(
            props.statistics_enabled(&ColumnPath::from("geometry_properties_lod2_2")),
            EnabledStatistics::None
        );
        assert_eq!(
            props.statistics_enabled(&ColumnPath::from("other")),
            EnabledStatistics::None
        );

        // attribute columns: parquet defaults (dictionary stays on).
        assert!(props.dictionary_enabled(&ColumnPath::from("yoc")));

        // global: ZSTD compression at the recipe's level.
        assert_eq!(
            props.compression(&ColumnPath::from("yoc")),
            Compression::ZSTD(ZstdLevel::try_new(3).unwrap())
        );

        // key-value metadata: dataset metadata plus the derived `geo` key.
        let kvs = props.key_value_metadata().expect("key-value metadata set");
        assert!(kvs.iter().any(|kv| kv.key == "cityparquet_version"));
        assert!(kvs.iter().any(|kv| kv.key == "geo"));
    }

    #[test]
    fn attribute_named_like_geometry_keeps_attribute_defaults() {
        // `geometry_extra` is a legitimate attribute name: schema validation
        // only rejects exact reserved/geometry column names. It must get
        // parquet attribute defaults, not the geometry (WKB) treatment —
        // geometry columns are identified by their geoarrow.wkb extension
        // metadata, never by name alone.
        let schema = CityParquetSchema {
            lods: vec![Lod::parse("2.2").unwrap()],
            attributes: vec![
                ("yoc".to_string(), AttributeType::Int64),
                ("geometry_extra".to_string(), AttributeType::String),
            ],
            crs: None,
        };
        let props = WriterRecipe::default()
            .writer_properties(&schema, &sample_metadata(), true)
            .unwrap();

        // Attribute defaults: dictionary on, statistics not disabled.
        assert!(props.dictionary_enabled(&ColumnPath::from("geometry_extra")));
        assert_ne!(
            props.statistics_enabled(&ColumnPath::from("geometry_extra")),
            EnabledStatistics::None
        );

        // The real geometry column still gets the WKB treatment.
        assert!(!props.dictionary_enabled(&ColumnPath::from("geometry_lod2_2")));
        assert_eq!(
            props.statistics_enabled(&ColumnPath::from("geometry_lod2_2")),
            EnabledStatistics::None
        );
    }

    #[test]
    fn statistics_for_json_opts_json_columns_back_in() {
        let schema = sample_schema();
        let metadata = sample_metadata();
        let recipe = WriterRecipe {
            statistics_for_json: true,
            ..WriterRecipe::default()
        };
        let props = recipe.writer_properties(&schema, &metadata, true).unwrap();
        assert_ne!(
            props.statistics_enabled(&ColumnPath::from("other")),
            EnabledStatistics::None
        );
    }

    #[test]
    fn geo_key_present_only_when_geoarrow_enabled() {
        let schema = sample_schema();
        let metadata = sample_metadata();
        let recipe = WriterRecipe::default();

        let has_geo = |geoarrow: bool| {
            recipe
                .writer_properties(&schema, &metadata, geoarrow)
                .unwrap()
                .key_value_metadata()
                .map(|kvs| kvs.iter().any(|kv| kv.key == "geo"))
                .unwrap_or(false)
        };

        assert!(
            has_geo(true),
            "geoarrow=true must emit the GeoParquet `geo` key"
        );
        assert!(
            !has_geo(false),
            "geoarrow=false must omit the `geo` key entirely"
        );
    }

    #[test]
    fn geometry_columns_keep_wkb_properties_even_with_geoarrow_off() {
        // Regression pin for the doc comment above `writer_properties`'s
        // `to_arrow_schema()` call (line ~168): that call is deliberately
        // the TAGGED zero-arg form, independent of the `geoarrow` flag, so
        // `is_geometry_column` still finds geometry columns via their
        // `geoarrow.wkb` extension metadata even when the file itself is
        // written untagged (plain-BLOB WKB, `geoarrow=false`). If a future
        // refactor swapped that call for `to_arrow_schema_tagged(geoarrow)`,
        // then with `geoarrow=false` no field would carry the extension tag,
        // `is_geometry_column` would match nothing, and geometry columns
        // would silently regain dictionary encoding + full statistics on
        // opaque WKB blobs.
        let schema = sample_schema();
        let metadata = sample_metadata();
        let props = WriterRecipe::default()
            .writer_properties(&schema, &metadata, false)
            .unwrap();

        assert!(!props.dictionary_enabled(&ColumnPath::from("geometry_lod2_2")));
        assert_eq!(
            props.statistics_enabled(&ColumnPath::from("geometry_lod2_2")),
            EnabledStatistics::None
        );
    }

    #[test]
    fn all_lists_exactly_six_presets() {
        assert_eq!(RecipePreset::ALL.len(), 6);
    }

    #[test]
    fn parse_round_trips_every_name() {
        for preset in RecipePreset::ALL {
            assert_eq!(
                RecipePreset::parse(preset.name()),
                Some(preset),
                "parse(name()) must round-trip for {preset:?}"
            );
        }
        assert_eq!(RecipePreset::parse("not-a-real-preset"), None);
    }

    #[test]
    fn parquet_defaults_leaves_columns_untuned() {
        let schema = sample_schema();
        let metadata = sample_metadata();
        let props = RecipePreset::ParquetDefaults
            .recipe()
            .writer_properties(&schema, &metadata, true)
            .unwrap();

        let bbox_xmin = ColumnPath::new(vec!["bbox".to_string(), "xmin".to_string()]);
        assert_eq!(props.encoding(&bbox_xmin), None);
        assert!(props.dictionary_enabled(&ColumnPath::from("id")));

        // KV metadata is never a preset variable.
        let kvs = props.key_value_metadata().expect("key-value metadata set");
        assert!(kvs.iter().any(|kv| kv.key == "cityparquet_version"));
        assert!(kvs.iter().any(|kv| kv.key == "geo"));
    }

    #[test]
    fn no_dictionary_disables_dictionary_everywhere() {
        let schema = sample_schema();
        let metadata = sample_metadata();
        let props = RecipePreset::NoDictionary
            .recipe()
            .writer_properties(&schema, &metadata, true)
            .unwrap();

        assert!(!props.dictionary_enabled(&ColumnPath::from("object_type")));
        assert!(!props.dictionary_enabled(&ColumnPath::from("yoc")));

        let kvs = props.key_value_metadata().expect("key-value metadata set");
        assert!(kvs.iter().any(|kv| kv.key == "cityparquet_version"));
        assert!(kvs.iter().any(|kv| kv.key == "geo"));
    }

    #[test]
    fn no_byte_stream_split_keeps_delta_ids_but_drops_bbox_bss() {
        let schema = sample_schema();
        let metadata = sample_metadata();
        let props = RecipePreset::NoByteStreamSplit
            .recipe()
            .writer_properties(&schema, &metadata, true)
            .unwrap();

        assert_eq!(
            props.encoding(&ColumnPath::from("id")),
            Some(Encoding::DELTA_BYTE_ARRAY)
        );
        let bbox_xmin = ColumnPath::new(vec!["bbox".to_string(), "xmin".to_string()]);
        assert_ne!(
            props.encoding(&bbox_xmin),
            Some(Encoding::BYTE_STREAM_SPLIT)
        );
        // Stats + dictionary-off are kept even without BSS.
        assert_eq!(
            props.statistics_enabled(&bbox_xmin),
            EnabledStatistics::Chunk
        );
        assert!(!props.dictionary_enabled(&bbox_xmin));

        let kvs = props.key_value_metadata().expect("key-value metadata set");
        assert!(kvs.iter().any(|kv| kv.key == "cityparquet_version"));
        assert!(kvs.iter().any(|kv| kv.key == "geo"));
    }

    #[test]
    fn no_delta_keeps_bbox_bss_but_drops_id_delta() {
        let schema = sample_schema();
        let metadata = sample_metadata();
        let props = RecipePreset::NoDelta
            .recipe()
            .writer_properties(&schema, &metadata, true)
            .unwrap();

        let bbox_xmin = ColumnPath::new(vec!["bbox".to_string(), "xmin".to_string()]);
        assert_eq!(
            props.encoding(&bbox_xmin),
            Some(Encoding::BYTE_STREAM_SPLIT)
        );
        assert_eq!(props.encoding(&ColumnPath::from("id")), None);

        let kvs = props.key_value_metadata().expect("key-value metadata set");
        assert!(kvs.iter().any(|kv| kv.key == "cityparquet_version"));
        assert!(kvs.iter().any(|kv| kv.key == "geo"));
    }

    #[test]
    fn snappy_compresses_with_snappy_globally() {
        let schema = sample_schema();
        let metadata = sample_metadata();
        let props = RecipePreset::Snappy
            .recipe()
            .writer_properties(&schema, &metadata, true)
            .unwrap();

        assert_eq!(
            props.compression(&ColumnPath::from("yoc")),
            Compression::SNAPPY
        );
        // Snappy keeps the rest of the CityParquet rules (e.g. bbox BSS).
        let bbox_xmin = ColumnPath::new(vec!["bbox".to_string(), "xmin".to_string()]);
        assert_eq!(
            props.encoding(&bbox_xmin),
            Some(Encoding::BYTE_STREAM_SPLIT)
        );

        let kvs = props.key_value_metadata().expect("key-value metadata set");
        assert!(kvs.iter().any(|kv| kv.key == "cityparquet_version"));
        assert!(kvs.iter().any(|kv| kv.key == "geo"));
    }
}
