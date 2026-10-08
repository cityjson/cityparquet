//! `cityparquet collection`: a dataset-level STAC Collection (`collection.json`)
//! over several CityParquet packages, each described by its `metadata.json`
//! STAC Item (spec 05 "Dataset-level `collection.json`").
//!
//! The aggregation — the `city3d:*` summaries, the spatial extent as the union
//! of the Items' bboxes — is `city3d_stac::stac::StacCollectionBuilder`'s, from
//! the City3D STAC tool; this module only reads the Items, adds the temporal
//! extent and one `item` link per package, and writes the result.

use std::fs;
use std::path::{Component, Path, PathBuf};

use chrono::{DateTime, Utc};
use city3d_stac::stac::{StacCollectionBuilder, StacItem};
use cityparquet_schema::{CityParquetError, Result};

/// What to aggregate and how to name the result.
#[derive(Debug, Clone)]
pub struct CollectionOptions {
    /// CityParquet package directories, each holding a `metadata.json` Item.
    pub packages: Vec<PathBuf>,
    /// The `collection.json` to write.
    pub output: PathBuf,
    pub id: String,
    pub title: Option<String>,
    pub description: Option<String>,
    pub license: Option<String>,
}

fn err(msg: String) -> CityParquetError {
    CityParquetError::Metadata(msg)
}

/// Write the STAC Collection over `opts.packages`' Items to `opts.output`,
/// returning how many Items it aggregates.
pub fn write_collection(opts: &CollectionOptions) -> Result<usize> {
    if opts.packages.is_empty() {
        return Err(err("a collection needs at least one package".to_string()));
    }
    let base = absolute(
        opts.output
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new(".")),
    )?;

    let mut items = Vec::with_capacity(opts.packages.len());
    let mut hrefs = Vec::with_capacity(opts.packages.len());
    for package in &opts.packages {
        let path = package.join("metadata.json");
        let text = fs::read_to_string(&path).map_err(|e| {
            CityParquetError::io_source(format!("cannot read {}", path.display()), e)
        })?;
        let item: StacItem = serde_json::from_str(&text)
            .map_err(|e| err(format!("{} is not a STAC Item: {e}", path.display())))?;
        hrefs.push(relative_href(&base, &absolute(&path)?));
        items.push(item);
    }

    let datetimes: Vec<DateTime<Utc>> = items
        .iter()
        .flat_map(|i| {
            [
                i.properties.datetime,
                i.properties.start_datetime,
                i.properties.end_datetime,
            ]
        })
        .flatten()
        .collect();

    let mut builder = StacCollectionBuilder::new(&opts.id)
        .temporal_extent(
            datetimes.iter().min().copied(),
            datetimes.iter().max().copied(),
        )
        .aggregate_from_items(&items)
        .map_err(|e| err(format!("cannot aggregate the Items: {e}")))?;
    if let Some(title) = &opts.title {
        builder = builder.title(title);
    }
    if let Some(description) = &opts.description {
        builder = builder.description(description);
    }
    if let Some(license) = &opts.license {
        builder = builder.license(license);
    }
    for (item, href) in items.iter().zip(&hrefs) {
        builder = builder.item_link(href, Some(item.id.clone()));
    }
    let collection = builder.build().map_err(|e| {
        err(format!(
            "cannot build the Collection: {e} (an Item carries a WGS84 bbox only when its \
             package's CRS is known)"
        ))
    })?;

    let json = serde_json::to_string_pretty(&collection)?;
    fs::write(&opts.output, json).map_err(|e| {
        CityParquetError::io_source(format!("cannot write {}", opts.output.display()), e)
    })?;
    Ok(items.len())
}

fn absolute(path: &Path) -> Result<PathBuf> {
    path.canonicalize()
        .map_err(|e| CityParquetError::io_source(format!("cannot resolve {}", path.display()), e))
}

/// `target` relative to the directory `base`, `/`-separated as a STAC href,
/// with a leading `./` when it is inside `base`.
fn relative_href(base: &Path, target: &Path) -> String {
    let base: Vec<Component> = base.components().collect();
    let target: Vec<Component> = target.components().collect();
    let common = base.iter().zip(&target).take_while(|(a, b)| a == b).count();
    let mut parts: Vec<String> = vec!["..".to_string(); base.len() - common];
    if parts.is_empty() {
        parts.push(".".to_string());
    }
    parts.extend(
        target[common..]
            .iter()
            .map(|c| c.as_os_str().to_string_lossy().into_owned()),
    );
    parts.join("/")
}
