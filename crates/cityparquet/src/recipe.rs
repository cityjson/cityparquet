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

/// A named preset of Parquet `WriterProperties`, parameterised over the
/// handful of knobs the paper's benchmark actually varies.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WriterRecipe {
    pub row_group_size: usize,
    pub zstd_level: i32,
    /// Whether JSON-typed columns (geometry properties, material, texture,
    /// `other`, `Json`-typed attributes) still get column statistics.
    /// Off by default: statistics on serialised JSON blobs are not useful for
    /// predicate pushdown and only cost write time.
    pub statistics_for_json: bool,
}

impl Default for WriterRecipe {
    fn default() -> Self {
        Self {
            row_group_size: 65536,
            zstd_level: 3,
            statistics_for_json: false,
        }
    }
}

/// A `geometry*` WKB column, excluding its `geometry_properties*` JSON
/// sibling (which is handled by the arrow.json rule instead).
fn is_geometry_column(name: &str) -> bool {
    if name.starts_with("geometry_properties") {
        return false;
    }
    name == "geometry" || name.starts_with("geometry_")
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
    /// embedding `metadata` (plus the derived GeoParquet `geo` key) as
    /// Parquet file-level key-value metadata.
    pub fn writer_properties(
        &self,
        schema: &CityParquetSchema,
        metadata: &CityParquetMetadata,
    ) -> Result<WriterProperties> {
        let arrow_schema = schema.to_arrow_schema()?;

        let mut kvs: Vec<KeyValue> = metadata
            .to_key_values()?
            .into_iter()
            .map(|(key, value)| KeyValue::new(key, value))
            .collect();
        kvs.push(KeyValue::new(
            "geo".to_string(),
            metadata.geoparquet_geo_value()?.to_string(),
        ));

        let zstd_level = ZstdLevel::try_new(self.zstd_level).map_err(|e| {
            CityParquetError::Schema(format!("invalid zstd level {}: {e}", self.zstd_level))
        })?;

        let mut builder = WriterProperties::builder()
            .set_compression(Compression::ZSTD(zstd_level))
            // `set_max_row_group_size` is deprecated in parquet 58 in favour of
            // this row-count-only setter (semantically identical here: we
            // never set a row-group byte cap).
            .set_max_row_group_row_count(Some(self.row_group_size))
            .set_key_value_metadata(Some(kvs));

        builder = builder
            .set_column_dictionary_enabled(ColumnPath::from("object_type"), true)
            .set_column_statistics_enabled(
                ColumnPath::from("object_type"),
                EnabledStatistics::Chunk,
            );

        for name in ["id", "feature_id"] {
            builder = builder
                .set_column_encoding(ColumnPath::from(name), Encoding::DELTA_BYTE_ARRAY)
                .set_column_dictionary_enabled(ColumnPath::from(name), false);
        }

        for leaf in BBOX_LEAVES {
            let path = ColumnPath::from(format!("bbox.{leaf}"));
            builder = builder
                .set_column_encoding(path.clone(), Encoding::BYTE_STREAM_SPLIT)
                .set_column_statistics_enabled(path.clone(), EnabledStatistics::Chunk)
                .set_column_dictionary_enabled(path, false);
        }

        for field in arrow_schema.fields() {
            let name = field.name().as_str();
            if is_geometry_column(name) {
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
        }
    }

    #[test]
    fn recipe_renders_the_binding_per_column_rules() {
        let schema = sample_schema();
        let metadata = sample_metadata();
        let props = WriterRecipe::default()
            .writer_properties(&schema, &metadata)
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
        assert_eq!(
            props.encoding(&ColumnPath::from("bbox.xmin")),
            Some(Encoding::BYTE_STREAM_SPLIT)
        );
        assert_eq!(
            props.statistics_enabled(&ColumnPath::from("bbox.xmin")),
            EnabledStatistics::Chunk
        );
        assert!(!props.dictionary_enabled(&ColumnPath::from("bbox.xmin")));
        assert_eq!(
            props.encoding(&ColumnPath::from("bbox.zmax")),
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
    fn statistics_for_json_opts_json_columns_back_in() {
        let schema = sample_schema();
        let metadata = sample_metadata();
        let recipe = WriterRecipe {
            statistics_for_json: true,
            ..WriterRecipe::default()
        };
        let props = recipe.writer_properties(&schema, &metadata).unwrap();
        assert_ne!(
            props.statistics_enabled(&ColumnPath::from("other")),
            EnabledStatistics::None
        );
    }
}
