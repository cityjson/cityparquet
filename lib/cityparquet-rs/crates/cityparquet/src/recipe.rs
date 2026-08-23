//! Writer-property presets: the paper's benchmark variable, rendered as
//! per-column `parquet::file::properties::WriterProperties`.
//!
//! [`WriterRecipe`] never hardcodes column names beyond the six fixed `bbox`
//! leaf paths — every other per-column decision (geometry columns, JSON-typed
//! columns) is derived from the field names, extension metadata and declared
//! types of the Arrow schema [`CityParquetSchema`] renders for the
//! [`cityparquet_schema::GeometryEncoding`] the file will actually be written
//! under, so it can never drift from the schema it is given.

use arrow_schema::extension::EXTENSION_TYPE_NAME_KEY;
use arrow_schema::{Field, Schema};
use parquet::arrow::ArrowSchemaConverter;
use parquet::basic::{BrotliLevel, Compression, Encoding, GzipLevel, ZstdLevel};
use parquet::file::properties::{EnabledStatistics, WriterProperties};
use parquet::schema::types::ColumnPath;

use cityparquet_schema::model::{arrow_native_geometry_data_type, arrow_native_vertices_data_type};
use cityparquet_schema::{CityParquetError, CityParquetSchema, GeometryEncoding, Result};

/// A compression codec, overriding whichever codec [`RecipePreset`] would
/// otherwise pick — the benchmark's compression-codec axis, orthogonal to
/// the preset's per-column tuning rules and to [`WriterRecipe::row_group_size`].
///
/// `Lz4` renders to `Compression::LZ4_RAW`, the variant parquet-rs actually
/// writes; plain `Compression::LZ4` is the deprecated Hadoop-era codec and is
/// effectively read-only in this ecosystem (see `parquet::basic::Compression`'s
/// own doc comment).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Codec {
    Uncompressed,
    Snappy,
    Gzip,
    Lz4,
    Brotli,
    Zstd,
}

impl Codec {
    /// Every codec, in the order they are named — used to enumerate valid
    /// `--compression` / `+<codec>` values.
    pub const ALL: [Codec; 6] = [
        Codec::Uncompressed,
        Codec::Snappy,
        Codec::Gzip,
        Codec::Lz4,
        Codec::Brotli,
        Codec::Zstd,
    ];

    /// The stable, kebab-case name used on the CLI and in benchmark variant
    /// identifiers.
    pub fn name(&self) -> &'static str {
        match self {
            Codec::Uncompressed => "uncompressed",
            Codec::Snappy => "snappy",
            Codec::Gzip => "gzip",
            Codec::Lz4 => "lz4",
            Codec::Brotli => "brotli",
            Codec::Zstd => "zstd",
        }
    }

    /// Parses one of [`Codec::name`]'s exact strings; `None` for anything
    /// else (the caller decides how to report the invalid name).
    pub fn parse(s: &str) -> Option<Codec> {
        Codec::ALL.into_iter().find(|codec| codec.name() == s)
    }
}

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
    /// Overrides the compression codec [`preset`](Self::preset) would
    /// otherwise pick (SNAPPY for [`RecipePreset::Snappy`], ZSTD at
    /// [`zstd_level`](Self::zstd_level) for every other preset). `None`
    /// (the default) keeps that exact preset behaviour untouched — this
    /// field is the benchmark's compression-codec axis, independent of the
    /// per-column tuning rules a preset selects.
    pub compression: Option<Codec>,
}

impl Default for WriterRecipe {
    fn default() -> Self {
        Self {
            row_group_size: 65536,
            zstd_level: 3,
            statistics_for_json: false,
            preset: RecipePreset::CityParquet,
            compression: None,
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

/// One half of the arrow-native geometry payload: a `geometry_lod*` column or
/// its `geometry_vertices_lod*` vertex-pool sibling, detected by its exact
/// declared `DataType` rather than by name — the same principle as
/// [`is_geometry_column`]'s `geoarrow.wkb` tag (an attribute
/// merely called `geometry_extra` must keep attribute defaults), and one no
/// attribute column can ever satisfy: `AttributeType::to_arrow` renders
/// nothing remotely like these five-deep `List` / `List<Struct<x,y,z>>`
/// shapes. Under the arrow-native encoding the geometry payload has NO
/// scalar leaf of its own, so without this the tuning below would key itself
/// to a WKB column path that does not exist and every real leaf would
/// silently take parquet's global defaults instead.
fn is_arrow_native_payload_column(field: &Field) -> bool {
    field.data_type() == &arrow_native_geometry_data_type()
        || field.data_type() == &arrow_native_vertices_data_type()
}

/// A column tagged with the canonical `arrow.json` Arrow extension type.
fn is_json_column(field: &Field) -> bool {
    field
        .metadata()
        .get(EXTENSION_TYPE_NAME_KEY)
        .map(String::as_str)
        == Some(ARROW_JSON_EXTENSION)
}

/// The `geometry_properties[_lod*]` `STRUCT` column (spec "Geometry
/// properties and semantics") — named, not extension-tagged, unlike the
/// other JSON-ish reserved columns: it is a genuine nested `Struct`/`List`
/// type now, so it carries no single `arrow.json` tag of its own (only its
/// `surfaces` child does). Detected by name, mirroring
/// `crate::stac::properties::is_reserved_column_for`'s identical idiom for
/// the same column.
fn is_geometry_properties_column(field: &Field) -> bool {
    field.name() == "geometry_properties" || field.name().starts_with("geometry_properties_lod")
}

/// Every LEAF `ColumnPath` a nested `field` resolves to in the physical
/// Parquet schema — e.g. `geometry_properties_lod2_2.face_semantics.list.item`
/// for its `List<Int32>` child. Computed by actually running the same
/// Arrow-to-Parquet schema conversion `ArrowWriter` itself uses (over a
/// throwaway one-field schema), rather than hardcoding Arrow's `list`/`item`
/// intermediate group names, so a converter version bump can never silently
/// desync these paths from what gets written.
fn leaf_column_paths(field: &Field) -> Result<Vec<ColumnPath>> {
    let schema = Schema::new(vec![field.clone()]);
    let descr = ArrowSchemaConverter::new()
        .convert(&schema)
        .map_err(|e| CityParquetError::Schema(format!("cannot resolve leaf columns: {e}")))?;
    Ok(descr.columns().iter().map(|c| c.path().clone()).collect())
}

impl WriterRecipe {
    /// Render this recipe into concrete `WriterProperties` for `schema`: the
    /// per-column compression/encoding/statistics rules only. The `city`/`geo`
    /// footer key-value metadata is deliberately NOT embedded here any more
    /// (spec-alignment M3, gap 16/per-module footer emission) — each
    /// by-module table's `city`/`geo` genuinely differs (its own realised
    /// column set), so it can only be known post-encode, once that table's
    /// rows are actually written; `crate::package::TableWriters::finish`
    /// appends it via `append_key_value_metadata`, mirroring how
    /// `sidecar_files` used to be appended post-encode.
    ///
    /// `encoding` is the [`GeometryEncoding`] the geometry columns will
    /// actually be written under, threaded in rather than assumed: the
    /// per-column rules below are keyed to real physical column paths, and
    /// the WKB and arrow-native renderings have entirely different ones (a
    /// single `Binary` leaf versus a nested `List` tree plus a
    /// `geometry_vertices_lod*` sibling). Rendering a hardcoded WKB schema
    /// here would key the geometry tuning to a leaf that does not exist under
    /// arrow-native and let every real geometry/vertex-pool leaf fall through
    /// to parquet's global defaults instead — silently making recipe
    /// semantics encoding-dependent, which would bias any WKB-vs-arrow-native
    /// benchmark built on these presets.
    pub fn writer_properties(
        &self,
        schema: &CityParquetSchema,
        encoding: GeometryEncoding,
    ) -> Result<WriterProperties> {
        // TAGGED (`geoarrow = true`): the WKB geometry-column detection below
        // keys off the `geoarrow.wkb` extension metadata that flag adds.
        // Encoding-aware: see this method's doc comment.
        let arrow_schema = schema.to_arrow_schema_tagged(true, encoding)?;

        // `compression` OVERRIDES the preset's default codec when set;
        // `None` keeps the exact pre-existing behaviour: `Snappy` compresses
        // with Snappy and ignores `zstd_level`, every other preset
        // compresses with zstd at `zstd_level`.
        let compression = match self.compression {
            Some(Codec::Uncompressed) => Compression::UNCOMPRESSED,
            Some(Codec::Snappy) => Compression::SNAPPY,
            Some(Codec::Gzip) => Compression::GZIP(GzipLevel::default()),
            Some(Codec::Lz4) => Compression::LZ4_RAW,
            Some(Codec::Brotli) => Compression::BROTLI(BrotliLevel::default()),
            Some(Codec::Zstd) => {
                let zstd_level = ZstdLevel::try_new(self.zstd_level).map_err(|e| {
                    CityParquetError::Schema(format!("invalid zstd level {}: {e}", self.zstd_level))
                })?;
                Compression::ZSTD(zstd_level)
            }
            None if self.preset == RecipePreset::Snappy => Compression::SNAPPY,
            None => {
                let zstd_level = ZstdLevel::try_new(self.zstd_level).map_err(|e| {
                    CityParquetError::Schema(format!("invalid zstd level {}: {e}", self.zstd_level))
                })?;
                Compression::ZSTD(zstd_level)
            }
        };

        let mut builder = WriterProperties::builder()
            .set_compression(compression)
            // `set_max_row_group_size` is deprecated in parquet 58 in favour of
            // this row-count-only setter (semantically identical here: we
            // never set a row-group byte cap).
            .set_max_row_group_row_count(Some(self.row_group_size));

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
            // The arrow-native geometry/vertex-pool pair gets the SAME
            // treatment the WKB geometry column above gets — dictionary off,
            // no statistics — applied to every physical leaf the nested
            // column resolves to, since neither has a leaf at its own top
            // level. Keeping the two encodings' geometry payload rules
            // identical is the point: a benchmark comparing them must differ
            // in the encoding alone, never in how the recipe happened to tune
            // one of them.
            if is_arrow_native_payload_column(field) {
                for path in leaf_column_paths(field)? {
                    builder = builder
                        .set_column_dictionary_enabled(path.clone(), false)
                        .set_column_statistics_enabled(path, EnabledStatistics::None);
                }
                continue;
            }
            if is_json_column(field) && !self.statistics_for_json {
                builder = builder
                    .set_column_statistics_enabled(ColumnPath::from(name), EnabledStatistics::None);
                continue;
            }
            // `geometry_properties*` is a nested Struct/List now (spec
            // "Geometry properties and semantics"), not a single JSON-tagged
            // leaf — every leaf underneath it (`type`, `surfaces`,
            // `face_semantics.list.item`, `shells.list.item.list.item`) gets
            // the same no-statistics-by-default treatment the whole column
            // used to get as one JSON blob, gated by the same toggle.
            if is_geometry_properties_column(field) && !self.statistics_for_json {
                for path in leaf_column_paths(field)? {
                    builder = builder.set_column_statistics_enabled(path, EnabledStatistics::None);
                }
            }
        }

        Ok(builder.build())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cityparquet_schema::{AttributeType, GeometryEncoding, Lod};
    use parquet::basic::Compression;

    fn sample_schema() -> CityParquetSchema {
        CityParquetSchema {
            lods: vec![Lod::parse("2.2").unwrap()],
            attributes: vec![("yoc".to_string(), AttributeType::Int64)],
            crs: None,
        }
    }

    #[test]
    fn recipe_renders_the_binding_per_column_rules() {
        let schema = sample_schema();
        let props = WriterRecipe::default()
            .writer_properties(&schema, GeometryEncoding::Wkb)
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
            props.statistics_enabled(&ColumnPath::from("other")),
            EnabledStatistics::None
        );

        // geometry_properties* is a nested Struct now — every leaf gets the
        // same no-statistics-by-default treatment the whole column used to
        // get as one JSON blob.
        for leaf in [
            vec!["geometry_properties_lod2_2".to_string(), "type".to_string()],
            vec![
                "geometry_properties_lod2_2".to_string(),
                "surfaces".to_string(),
            ],
            vec![
                "geometry_properties_lod2_2".to_string(),
                "face_semantics".to_string(),
                "list".to_string(),
                "item".to_string(),
            ],
            vec![
                "geometry_properties_lod2_2".to_string(),
                "shells".to_string(),
                "list".to_string(),
                "item".to_string(),
                "list".to_string(),
                "item".to_string(),
            ],
        ] {
            let path = ColumnPath::new(leaf.clone());
            assert_eq!(
                props.statistics_enabled(&path),
                EnabledStatistics::None,
                "leaf {leaf:?} must have statistics disabled by default"
            );
        }

        // attribute columns: parquet defaults (dictionary stays on).
        assert!(props.dictionary_enabled(&ColumnPath::from("yoc")));

        // global: ZSTD compression at the recipe's level.
        assert_eq!(
            props.compression(&ColumnPath::from("yoc")),
            Compression::ZSTD(ZstdLevel::try_new(3).unwrap())
        );
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
            .writer_properties(&schema, GeometryEncoding::Wkb)
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
        let recipe = WriterRecipe {
            statistics_for_json: true,
            ..WriterRecipe::default()
        };
        let props = recipe
            .writer_properties(&schema, GeometryEncoding::Wkb)
            .unwrap();
        assert_ne!(
            props.statistics_enabled(&ColumnPath::from("other")),
            EnabledStatistics::None
        );
    }

    // `geo`/`city` footer key-value metadata is no longer rendered by
    // `WriterRecipe` at all (spec-alignment M3: per-module footer emission
    // happens post-encode in `crate::package::TableWriters::finish`) — see
    // `crate::package`'s own tests for the `geo`-present-iff-a-legal-column
    // coverage this recipe-level test used to carry.

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
        let props = RecipePreset::ParquetDefaults
            .recipe()
            .writer_properties(&schema, GeometryEncoding::Wkb)
            .unwrap();

        let bbox_xmin = ColumnPath::new(vec!["bbox".to_string(), "xmin".to_string()]);
        assert_eq!(props.encoding(&bbox_xmin), None);
        assert!(props.dictionary_enabled(&ColumnPath::from("id")));
    }

    #[test]
    fn no_dictionary_disables_dictionary_everywhere() {
        let schema = sample_schema();
        let props = RecipePreset::NoDictionary
            .recipe()
            .writer_properties(&schema, GeometryEncoding::Wkb)
            .unwrap();

        assert!(!props.dictionary_enabled(&ColumnPath::from("object_type")));
        assert!(!props.dictionary_enabled(&ColumnPath::from("yoc")));
    }

    #[test]
    fn no_byte_stream_split_keeps_delta_ids_but_drops_bbox_bss() {
        let schema = sample_schema();
        let props = RecipePreset::NoByteStreamSplit
            .recipe()
            .writer_properties(&schema, GeometryEncoding::Wkb)
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
    }

    #[test]
    fn no_delta_keeps_bbox_bss_but_drops_id_delta() {
        let schema = sample_schema();
        let props = RecipePreset::NoDelta
            .recipe()
            .writer_properties(&schema, GeometryEncoding::Wkb)
            .unwrap();

        let bbox_xmin = ColumnPath::new(vec!["bbox".to_string(), "xmin".to_string()]);
        assert_eq!(
            props.encoding(&bbox_xmin),
            Some(Encoding::BYTE_STREAM_SPLIT)
        );
        assert_eq!(props.encoding(&ColumnPath::from("id")), None);
    }

    #[test]
    fn snappy_compresses_with_snappy_globally() {
        let schema = sample_schema();
        let props = RecipePreset::Snappy
            .recipe()
            .writer_properties(&schema, GeometryEncoding::Wkb)
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
    }

    #[test]
    fn codec_all_lists_exactly_six_codecs() {
        assert_eq!(Codec::ALL.len(), 6);
    }

    #[test]
    fn codec_parse_round_trips_every_name() {
        for codec in Codec::ALL {
            assert_eq!(
                Codec::parse(codec.name()),
                Some(codec),
                "parse(name()) must round-trip for {codec:?}"
            );
        }
        assert_eq!(Codec::parse("not-a-real-codec"), None);
    }

    #[test]
    fn compression_override_none_keeps_preset_default_codec() {
        let schema = sample_schema();

        // cityparquet preset defaults to ZSTD at zstd_level.
        let props = WriterRecipe::default()
            .writer_properties(&schema, GeometryEncoding::Wkb)
            .unwrap();
        assert_eq!(
            props.compression(&ColumnPath::from("yoc")),
            Compression::ZSTD(ZstdLevel::try_new(3).unwrap())
        );

        // snappy preset defaults to SNAPPY.
        let props = RecipePreset::Snappy
            .recipe()
            .writer_properties(&schema, GeometryEncoding::Wkb)
            .unwrap();
        assert_eq!(
            props.compression(&ColumnPath::from("yoc")),
            Compression::SNAPPY
        );
    }

    #[test]
    fn compression_override_wins_over_the_preset_default() {
        let schema = sample_schema();

        let expected = [
            (Codec::Uncompressed, Compression::UNCOMPRESSED),
            (Codec::Snappy, Compression::SNAPPY),
            (Codec::Gzip, Compression::GZIP(GzipLevel::default())),
            (Codec::Lz4, Compression::LZ4_RAW),
            (Codec::Brotli, Compression::BROTLI(BrotliLevel::default())),
            (
                Codec::Zstd,
                Compression::ZSTD(ZstdLevel::try_new(3).unwrap()),
            ),
        ];

        for (codec, want) in expected {
            let recipe = WriterRecipe {
                compression: Some(codec),
                ..WriterRecipe::default()
            };
            let props = recipe
                .writer_properties(&schema, GeometryEncoding::Wkb)
                .unwrap();
            assert_eq!(
                props.compression(&ColumnPath::from("yoc")),
                want,
                "codec {codec:?} should render to {want:?}"
            );

            // The override also applies on top of a non-default preset (e.g.
            // Snappy, which would otherwise force SNAPPY regardless).
            let recipe = WriterRecipe {
                preset: RecipePreset::Snappy,
                compression: Some(codec),
                ..WriterRecipe::default()
            };
            let props = recipe
                .writer_properties(&schema, GeometryEncoding::Wkb)
                .unwrap();
            assert_eq!(
                props.compression(&ColumnPath::from("yoc")),
                want,
                "codec {codec:?} should override the snappy preset's default too"
            );
        }
    }

    /// Whole-branch review finding 3: under the arrow-native encoding the
    /// recipe used to derive its per-column rules from a hardcoded WKB
    /// rendering, so the geometry-specific settings targeted a scalar WKB leaf
    /// that does not exist, while the REAL nested geometry/vertex-pool leaves
    /// silently fell through to parquet's global defaults. Not a correctness
    /// bug, but it made recipe semantics encoding-dependent — precisely the
    /// kind of hidden asymmetry that would bias a WKB-vs-arrow-native
    /// benchmark. Every leaf of both arrow-native columns must now carry the
    /// same explicit treatment the WKB geometry column gets.
    #[test]
    fn arrow_native_geometry_and_vertex_pool_leaves_get_explicit_settings() {
        let schema = sample_schema();
        let props = WriterRecipe::default()
            .writer_properties(&schema, GeometryEncoding::ArrowNative)
            .unwrap();
        // Leaf paths are resolved through the same `leaf_column_paths` the
        // recipe itself uses, so this test can never hardcode a stale
        // `list`/`item` intermediate name.
        let arrow = schema
            .to_arrow_schema_tagged(true, GeometryEncoding::ArrowNative)
            .unwrap();
        for name in ["geometry_lod2_2", "geometry_vertices_lod2_2"] {
            let field = arrow
                .field_with_name(name)
                .unwrap_or_else(|e| panic!("arrow-native schema must carry {name}: {e}"));
            let leaves = leaf_column_paths(field).unwrap();
            assert!(
                !leaves.is_empty(),
                "{name} must resolve to at least one physical leaf"
            );
            for path in leaves {
                assert!(
                    !props.dictionary_enabled(&path),
                    "{name} leaf {path:?} must have dictionary encoding explicitly off, \
                     not parquet's default"
                );
                assert_eq!(
                    props.statistics_enabled(&path),
                    EnabledStatistics::None,
                    "{name} leaf {path:?} must have statistics explicitly off, not parquet's \
                     default"
                );
            }
        }
    }

    /// The other half of the same finding: the WKB rendering must be
    /// completely unaffected — its single `geometry_lod2_2` leaf keeps the
    /// exact treatment it always had, and no `geometry_vertices_lod2_2`
    /// column exists to tune at all.
    #[test]
    fn the_wkb_rendering_keeps_its_geometry_column_treatment_unchanged() {
        let schema = sample_schema();
        let props = WriterRecipe::default()
            .writer_properties(&schema, GeometryEncoding::Wkb)
            .unwrap();
        let path = ColumnPath::from("geometry_lod2_2");
        assert!(!props.dictionary_enabled(&path));
        assert_eq!(props.statistics_enabled(&path), EnabledStatistics::None);
        assert!(
            schema
                .to_arrow_schema_tagged(true, GeometryEncoding::Wkb)
                .unwrap()
                .field_with_name("geometry_vertices_lod2_2")
                .is_err(),
            "the WKB rendering has no vertex-pool column at all"
        );
    }
}
