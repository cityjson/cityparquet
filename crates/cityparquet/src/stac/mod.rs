//! Derive a STAC Item from a written CityParquet package.
//!
//! Every field is read back from the files on disk — the Parquet footer, the
//! Arrow schema, and the dictionary-encoded `object_type` column. Nothing is
//! accumulated during conversion. This makes spec §13.2's rule ("where the
//! STAC Item and the Parquet footer disagree, the footer is authoritative")
//! true by construction rather than by discipline, and means a package written
//! by any conformant writer can be described, not just this crate's own.

pub mod assets;
pub mod attribute_type;
pub mod properties;

use std::fs;
use std::path::Path;

use chrono::{SecondsFormat, Utc};
use city3d_stac_types::metadata::{BBox3D, CRS};
use city3d_stac_types::stac::StacItemBuilder;
use city3d_stac_types::stac::types::{Asset, Item};
use cityparquet_schema::{CityParquetError, Result};
use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;
use parquet::file::metadata::RowGroupMetaData;
use parquet::file::statistics::Statistics;
use parquet::schema::types::ColumnPath;
use serde_json::Value;

use crate::reader::CityParquetReaderBuilder;
use crate::stac::properties::PackageTables;

/// How to fill the parts of an Item that a package cannot supply itself.
#[derive(Debug, Clone, Default)]
pub struct ItemOptions {
    /// Item id. Defaults to the package directory's name.
    pub id: Option<String>,
    /// RFC 3339 timestamp for `properties.datetime`, overriding whatever the
    /// package itself carries.
    ///
    /// Resolution order (see [`resolve_datetime`]): this explicit value wins;
    /// otherwise the source CityJSON header's `referenceDate` is used if the
    /// footer preserved one (`source_metadata`) — deriving from the package
    /// rather than inventing; otherwise the current UTC time at build time.
    ///
    /// **Design decision of 2026-07-21, superseding 2026-07-20's "the CLI
    /// stamps, the library does not":** almost no real CityJSON carries
    /// `referenceDate` — none of this repo's fixtures does — and STAC
    /// requires every Item to carry a `datetime`, so leaving it unset would
    /// make most derived Items schema-invalid on their own. The timestamp
    /// fallback therefore now applies uniformly, library included.
    pub datetime: Option<String>,
}

/// Build a STAC Item describing a written CityParquet package.
///
/// Reads the package back rather than relying on anything remembered from
/// conversion, so the Item cannot drift from the files it describes. Thin
/// wrapper around [`build_item`] for the common case of a package already on
/// disk; [`crate::package`]'s writer calls `build_item` directly with a
/// [`PackageTables`] built from the file list it is ABOUT to write (see
/// [`PackageTables::from_lists`]), before `metadata.json` itself exists for
/// `open` to read back.
///
/// Never guesses a `bbox`/`geometry`: when the package extent cannot be
/// expressed in WGS84 (no CRS, or one this crate cannot reproject), the Item
/// simply carries neither — both are optional STAC fields, and an "unlocated"
/// Item is honest where a wrong extent would not be. This is deliberate, not
/// merely tolerated: `build_item` also runs on every [`crate::package::convert`]
/// call, and a CRS is optional in CityJSON, so failing conversion itself over
/// a derived, discovery-only field would be a worse outcome.
pub fn item_for_package(dir: &Path, opts: &ItemOptions) -> Result<Item> {
    build_item(&PackageTables::open(dir)?, opts)
}

/// Build a STAC Item describing `tables`' object tables and sidecars.
///
/// See [`item_for_package`] for the package-on-disk entry point this backs.
pub fn build_item(tables: &PackageTables, opts: &ItemOptions) -> Result<Item> {
    let props = properties::derive_from_footer(tables)?;
    let crs = package_crs(tables)?;

    let id = opts.id.clone().unwrap_or_else(|| {
        tables
            .dir
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "cityparquet".to_string())
    });

    let mut builder = StacItemBuilder::new(id)
        .city3d(props)
        .map_err(|e| CityParquetError::Metadata(format!("cannot build STAC item: {e}")))?;

    let datetime = resolve_datetime(opts.datetime.as_deref(), source_metadata(tables)?.as_ref());
    builder = builder.datetime(Some(datetime));

    // `cityparquet:version` is footer-derived like everything else in this
    // module (any conformant package carries it). `cityparquet:profile` is
    // gone (spec-alignment gap 19): the Profile concept it described no
    // longer exists — a writer emits a sidecar whenever the source has
    // content for it, never gated by a declared profile.
    builder = builder.property(
        "cityparquet:version".to_string(),
        Value::String(package_cityparquet_version(tables)?),
    );

    if let Some(crs) = &crs {
        builder = builder.crs(crs);
    }

    // STAC requires WGS84; the package extent is in the source CRS. When it
    // cannot be expressed in WGS84 (no CRS at all, or one this crate cannot
    // reproject), the Item simply carries no `bbox`/`geometry` rather than a
    // wrong one — both are optional STAC fields, and this is an honest "no
    // WGS84 extent available", not a guess (the same "None over guessing"
    // discipline `package_crs`/`package_bbox` already apply themselves).
    // This function is now on `write_package`'s mandatory path (Task 4), and
    // a CRS is optional in CityJSON (`helsinki_address.city.jsonl` has none)
    // — failing the WHOLE conversion over a derived, discovery-only field
    // would be a worse outcome than an unlocated Item.
    if let Some(bbox) = package_bbox(tables)?
        && let Ok(wgs84) = bbox.to_wgs84(crs.as_ref().unwrap_or(&CRS::unknown()))
    {
        builder = builder.bbox(wgs84).geometry_from_bbox();
    }

    // The first table also goes through `data_asset`, purely because that is
    // the only `StacItemBuilder` method that flips on the STAC File
    // extension (`uses_file_extension` is a private field it alone sets) —
    // but the asset it inserts is keyed `"data"` with roles `["data"]`, and
    // it is *not* replaced by the properly keyed and roled one below: the
    // two live under different keys (`"data"` vs. the filename-derived key)
    // in the builder's asset map, so both coexist in the built Item.
    //
    // Without the filename-keyed asset the primary object table would be the
    // one asset a reader cannot map back to a file by key, and the only
    // table missing the `cityparquet-objects` role — which is exactly the
    // role Plan 2b binds `export` to when it drops the manifest's
    // `sidecar_files` list. Iterating that role would have silently skipped
    // the first table.
    //
    // Known interop wart: a generic STAC consumer enumerating `assets` sees
    // the first object table listed twice (once as `"data"`, once by
    // filename) with the same `href`. This crate's own `open()` is
    // unaffected — it filters by role and skips the `"data"`-only asset — so
    // the wart is cosmetic for this codebase, but it is real for other STAC
    // clients. It is not fixed here: `city3d_stac_types::StacItemBuilder`
    // exposes no way to declare the File extension without inserting a
    // `"data"`-keyed asset (no asset-removal method, no public
    // `uses_file_extension` setter), and that builder lives in the separate
    // `city3d-stac-tool` repo, not this one.
    let mut declared_file_extension = false;
    for path in &tables.tables {
        let (size, checksum) = assets::file_facts(path);
        let key = assets::asset_key(path);
        let href = format!("./{key}");
        if !declared_file_extension {
            builder = builder.data_asset(
                href.clone(),
                assets::PARQUET_MEDIA_TYPE,
                size,
                checksum.clone(),
            );
            declared_file_extension = true;
        }
        builder = builder.asset(
            key,
            package_asset(&href, size, checksum, assets::AssetKind::ObjectTable),
        );
    }
    for name in &tables.sidecar_files {
        let path = tables.dir.join(name);
        // A promise the package cannot keep is corruption, not something to
        // describe: an Item claiming an asset that is not on disk is worse
        // than a failed derivation. `crate::export` treats a listed-but-
        // missing sidecar the same way.
        if !path.exists() {
            return Err(CityParquetError::Io(format!(
                "package lists sidecar '{name}' but {} is not on disk",
                path.display()
            )));
        }
        let (size, checksum) = assets::file_facts(&path);
        builder = builder.asset(
            name.clone(),
            package_asset(
                &format!("./{name}"),
                size,
                checksum,
                assets::AssetKind::Sidecar,
            ),
        );
    }

    builder
        .build()
        .map_err(|e| CityParquetError::Metadata(format!("cannot build STAC item: {e}")))
}

/// A STAC asset for one package file.
fn package_asset(
    href: &str,
    size: Option<u64>,
    checksum: Option<String>,
    kind: assets::AssetKind,
) -> Asset {
    let mut asset = Asset::new(href);
    asset.media_type = Some(assets::PARQUET_MEDIA_TYPE.to_string());
    asset.roles = kind.roles();
    if let Some(size) = size {
        asset
            .additional_fields
            .insert("file:size".to_string(), Value::Number(size.into()));
    }
    if let Some(checksum) = checksum {
        asset
            .additional_fields
            .insert("file:checksum".to_string(), Value::String(checksum));
    }
    asset
}

/// The source CityJSON header's `metadata` object, as the footer preserved it.
///
/// Carries `referenceDate` when the source had one, which is the only honest
/// source of a `datetime` for a package.
fn source_metadata(tables: &PackageTables) -> Result<Option<Value>> {
    let Some(path) = tables.tables.first() else {
        return Ok(None);
    };
    let file = fs::File::open(path)
        .map_err(|e| CityParquetError::Io(format!("cannot open {}: {e}", path.display())))?;
    let builder = ParquetRecordBatchReaderBuilder::try_new(file)
        .map_err(|e| CityParquetError::Parquet(format!("cannot open {}: {e}", path.display())))?;
    // `source_metadata` is no longer its own footer key (spec-alignment M3,
    // gap 16) — the source CityJSON header's `metadata` object, when a
    // writer chose to keep it, now lives at `city.other.source_metadata`
    // (informational only; see `crate::export::source_metadata_from_other`).
    Ok(builder
        .cityparquet_metadata()?
        .other
        .as_ref()
        .and_then(|o| o.get("source_metadata"))
        .cloned())
}

/// The CityParquet spec/encoding version the package's footer declares
/// (`cityparquet_version`, §13.1) — e.g. `"0.1.0"`. Distinct from
/// `city3d:version`, which is the *source* CityJSON version.
fn package_cityparquet_version(tables: &PackageTables) -> Result<String> {
    let path = tables.tables.first().ok_or_else(|| {
        CityParquetError::Metadata("package has no object tables to read a version from".into())
    })?;
    let file = fs::File::open(path)
        .map_err(|e| CityParquetError::Io(format!("cannot open {}: {e}", path.display())))?;
    let builder = ParquetRecordBatchReaderBuilder::try_new(file)
        .map_err(|e| CityParquetError::Parquet(format!("cannot open {}: {e}", path.display())))?;
    Ok(builder.cityparquet_metadata()?.version)
}

/// Resolve `properties.datetime` for a built Item.
///
/// Order: `explicit` wins when given *and it parses as RFC 3339*; otherwise
/// `source_metadata`'s `referenceDate` (a CityJSON header field, `YYYY-MM-DD`
/// or already an RFC 3339 datetime) when the footer preserved one *and the
/// normalised result parses*; otherwise the current UTC time at build time,
/// as RFC 3339.
///
/// STAC requires every Item to carry a non-null `datetime` (or an explicit
/// start/end pair, which this crate never writes) to be schema-valid. A
/// malformed candidate — a caller-supplied `explicit` that isn't RFC 3339, or
/// a source `referenceDate` too broken to normalise (e.g. `"2026-99-99"`) —
/// must not be returned as-is: [`StacItemBuilder::datetime`] silently turns
/// an unparsable string into a `null` `properties.datetime`, which is exactly
/// the schema-invalid Item this function exists to rule out. Each candidate
/// is therefore validated with [`chrono::DateTime::parse_from_rfc3339`]
/// before being returned; a candidate that fails validation is skipped in
/// favour of the next one, never returned.
///
/// Design decision of 2026-07-21, superseding 2026-07-20's "the CLI stamps,
/// the library does not": almost no real CityJSON carries `referenceDate`,
/// so leaving `datetime` unset would make most derived Items schema-invalid
/// on their own — the timestamp fallback applies uniformly here, in the one
/// place the whole policy lives, rather than per-caller.
fn resolve_datetime(explicit: Option<&str>, source_metadata: Option<&Value>) -> String {
    if let Some(dt) = explicit
        && chrono::DateTime::parse_from_rfc3339(dt).is_ok()
    {
        return dt.to_string();
    }
    if let Some(reference_date) = source_metadata
        .and_then(|m| m.get("referenceDate"))
        .and_then(|v| v.as_str())
    {
        let normalised = if reference_date.contains('T') {
            reference_date.to_string()
        } else {
            format!("{reference_date}T00:00:00Z")
        };
        if chrono::DateTime::parse_from_rfc3339(&normalised).is_ok() {
            return normalised;
        }
    }
    Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true)
}

/// The package's CRS, as an EPSG code lifted out of the stored PROJJSON.
///
/// The footer stores the dataset CRS as PROJJSON (§13.3). `CRS` here is
/// EPSG-based, so the authority code is read from the PROJJSON `id`. A package
/// with no CRS — or one this crate cannot resolve to an EPSG code (a non-EPSG
/// authority, or a code that doesn't fit) — yields `None`, never a guess: the
/// PROJJSON itself is untouched in the footer regardless (§13.2 authority),
/// this is only what the discovery-only Item can additionally express. `None`
/// here is also on [`crate::package::convert`]'s mandatory path (Task 4), so
/// it must never be an `Err` — a CRS this crate cannot resolve to EPSG is not
/// a corrupt package, just one `build_item` can describe less precisely.
fn package_crs(tables: &PackageTables) -> Result<Option<CRS>> {
    let Some(path) = tables.tables.first() else {
        return Ok(None);
    };
    let file = fs::File::open(path)
        .map_err(|e| CityParquetError::Io(format!("cannot open {}: {e}", path.display())))?;
    let builder = ParquetRecordBatchReaderBuilder::try_new(file)
        .map_err(|e| CityParquetError::Parquet(format!("cannot open {}: {e}", path.display())))?;
    let meta = builder.cityparquet_metadata()?;

    let Some(id) = meta.crs.as_ref().and_then(|c| c.get("id")) else {
        return Ok(None);
    };

    // The authority must actually be EPSG — labelling an OGC or IAU code as
    // EPSG would produce a confidently WRONG reprojection, which is worse
    // than reporting no CRS at all.
    if id.get("authority").and_then(|a| a.as_str()) != Some("EPSG") {
        return Ok(None);
    }

    // PROJJSON permits the code as a number or a digit string.
    let code = id.get("code").and_then(|code| {
        code.as_u64()
            .or_else(|| code.as_str().and_then(|s| s.parse::<u64>().ok()))
    });
    let Some(code) = code.and_then(|c| u32::try_from(c).ok()) else {
        return Ok(None);
    };
    Ok(Some(CRS::from_epsg(code)))
}

/// The `bbox` struct's six leaf columns, in the order [`BBox3D`] uses.
/// Mirrors `crate::reader`'s `BBOX_LEAVES` and
/// `cityparquet_schema::model::bbox_data_type`.
const BBOX_LEAVES: [&str; 6] = ["xmin", "ymin", "zmin", "xmax", "ymax", "zmax"];

/// The package's spatial extent, in the **source** CRS.
///
/// Unioned from the `bbox` column's per-row-group leaf statistics across every
/// object table, so this is O(footer) — unlike the appearance and semantics
/// flags, the bbox leaves are plain `f64` columns and do carry statistics
/// (`crate::recipe` only disables them for JSON columns).
///
/// Returns `None` when no row group carries the leaf statistics — a package
/// with no geometry at all, or one written without them. Callers must not
/// substitute a guess: a STAC Item with a wrong bbox is worse than one with
/// none.
///
/// Reprojection to WGS84 is deliberately *not* done here; the Item assembly
/// step does it, because that is where the CRS is known.
pub fn package_bbox(tables: &PackageTables) -> Result<Option<BBox3D>> {
    // (min, max) accumulators per leaf, in BBOX_LEAVES order.
    let mut acc: [Option<(f64, f64)>; 6] = [None; 6];

    for path in &tables.tables {
        let file = fs::File::open(path)
            .map_err(|e| CityParquetError::Io(format!("cannot open {}: {e}", path.display())))?;
        let builder = ParquetRecordBatchReaderBuilder::try_new(file).map_err(|e| {
            CityParquetError::Parquet(format!("cannot open {}: {e}", path.display()))
        })?;

        for rg in builder.metadata().row_groups() {
            for (i, leaf) in BBOX_LEAVES.iter().enumerate() {
                let Some((lo, hi)) = bbox_leaf_bounds(rg, leaf) else {
                    // A row group that cannot report a leaf may hold geometry
                    // outside everything observed so far. Continuing would
                    // yield a bbox that is smaller than the data — the one
                    // failure mode a spatial extent must never have, because a
                    // too-small bbox causes false-negative pruning. Give up on
                    // the whole extent rather than emit an under-bound.
                    return Ok(None);
                };
                acc[i] = Some(match acc[i] {
                    Some((min, max)) => (min.min(lo), max.max(hi)),
                    None => (lo, hi),
                });
            }
        }
    }

    // Every leaf must have contributed; a partial extent is not an extent.
    let mut bounds = [(0.0f64, 0.0f64); 6];
    for (i, slot) in acc.iter().enumerate() {
        match slot {
            Some(v) => bounds[i] = *v,
            None => return Ok(None),
        }
    }

    Ok(Some(BBox3D {
        // A min leaf contributes its minimum, a max leaf its maximum — taking
        // the min of `xmin` and the max of `xmax` is what makes the union a
        // superset rather than an intersection.
        xmin: bounds[0].0,
        ymin: bounds[1].0,
        zmin: bounds[2].0,
        xmax: bounds[3].1,
        ymax: bounds[4].1,
        zmax: bounds[5].1,
    }))
}

/// The `(min, max)` of the `bbox.<leaf>` column chunk in `rg`, if it exists
/// and carries f64 statistics.
///
/// The column path is built with `ColumnPath::new` over the two nested parts:
/// `ColumnPath::from("bbox.xmin")` does **not** split on `.` in parquet 58 and
/// would silently match nothing. `crate::reader`'s `bbox_leaf_statistics`
/// documents the same trap — it bit the writer side once already.
fn bbox_leaf_bounds(rg: &RowGroupMetaData, leaf: &str) -> Option<(f64, f64)> {
    let path = ColumnPath::new(vec!["bbox".to_string(), leaf.to_string()]);
    let stats = rg
        .columns()
        .iter()
        .find(|c| c.column_path() == &path)?
        .statistics()?;
    match stats {
        Statistics::Double(s) => Some((*s.min_opt()?, *s.max_opt()?)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::resolve_datetime;
    use chrono::{DateTime, Utc};
    use serde_json::json;

    /// An explicit value wins over everything else, including a source
    /// `referenceDate` that is also present.
    #[test]
    fn explicit_datetime_wins_over_source_reference_date() {
        let source = json!({"referenceDate": "2019-06-01"});
        let resolved = resolve_datetime(Some("2024-01-15T12:00:00Z"), Some(&source));
        assert_eq!(resolved, "2024-01-15T12:00:00Z");
    }

    /// A date-only `referenceDate` (CityJSON's usual `YYYY-MM-DD` form) is
    /// normalised to midnight UTC so it parses as RFC 3339.
    #[test]
    fn date_only_reference_date_is_normalised_to_midnight_utc() {
        let source = json!({"referenceDate": "2019-06-01"});
        let resolved = resolve_datetime(None, Some(&source));
        assert_eq!(resolved, "2019-06-01T00:00:00Z");
        assert!(
            resolved.parse::<DateTime<Utc>>().is_ok(),
            "must be valid RFC 3339: {resolved}"
        );
    }

    /// A `referenceDate` that is already a full datetime is passed through
    /// unchanged rather than double-stamped with a midnight time.
    #[test]
    fn full_datetime_reference_date_passes_through() {
        let source = json!({"referenceDate": "2019-06-01T08:30:00Z"});
        let resolved = resolve_datetime(None, Some(&source));
        assert_eq!(resolved, "2019-06-01T08:30:00Z");
    }

    /// With neither an explicit value nor a source `referenceDate`, the
    /// fallback is the current UTC time — design decision of 2026-07-21: a
    /// package must never end up with a null `datetime`.
    #[test]
    fn falls_back_to_a_recent_utc_timestamp_when_nothing_else_is_available() {
        let before = Utc::now();
        let resolved = resolve_datetime(None, None);
        let after = Utc::now();

        let parsed: DateTime<Utc> = resolved
            .parse()
            .unwrap_or_else(|e| panic!("fallback datetime {resolved} is not RFC 3339: {e}"));
        assert!(
            parsed >= before - chrono::Duration::seconds(1) && parsed <= after,
            "fallback {parsed} must be close to conversion time ({before} .. {after})"
        );
    }

    /// A source `metadata` object with no `referenceDate` member behaves the
    /// same as no source metadata at all: falls back to the timestamp.
    #[test]
    fn source_metadata_without_reference_date_still_falls_back() {
        let source = json!({"title": "no reference date here"});
        let resolved = resolve_datetime(None, Some(&source));
        assert!(
            resolved.parse::<DateTime<Utc>>().is_ok(),
            "must still fall back to a valid RFC 3339 timestamp: {resolved}"
        );
    }

    /// A malformed `referenceDate` (not a valid calendar date, so it cannot
    /// be normalised into anything RFC 3339 parses) must not be returned
    /// as-is — `StacItemBuilder::datetime` would silently turn it into a
    /// null `properties.datetime`. It falls through to the timestamp
    /// fallback instead.
    #[test]
    fn malformed_reference_date_falls_through_to_timestamp() {
        let source = json!({"referenceDate": "2026-99-99"});
        let resolved = resolve_datetime(None, Some(&source));
        assert!(
            resolved.parse::<DateTime<Utc>>().is_ok(),
            "malformed referenceDate must fall through to a valid RFC 3339 timestamp: {resolved}"
        );
    }

    /// A well-formed `referenceDate` still resolves exactly as before —
    /// the validation added for the malformed case must not change the
    /// happy path.
    #[test]
    fn well_formed_reference_date_still_resolves_normally() {
        let source = json!({"referenceDate": "2019-06-01"});
        let resolved = resolve_datetime(None, Some(&source));
        assert_eq!(resolved, "2019-06-01T00:00:00Z");
    }

    /// A malformed explicit value must not be returned as-is either; it
    /// falls through to `source_metadata`'s `referenceDate`.
    #[test]
    fn malformed_explicit_falls_through_to_reference_date() {
        let source = json!({"referenceDate": "2019-06-01"});
        let resolved = resolve_datetime(Some("not-a-datetime"), Some(&source));
        assert_eq!(resolved, "2019-06-01T00:00:00Z");
    }
}
