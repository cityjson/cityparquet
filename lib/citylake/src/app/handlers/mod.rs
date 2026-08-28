//! One handler module per resource group: datasets, their objects, the
//! package operations the extension makes cheap (export, package write,
//! merge), and housekeeping (validate, reconcile, vacuum, compact).
//!
//! # What this API trusts
//!
//! Every caller is trusted, and the API has no authentication. Three surfaces
//! act directly on that trust:
//!
//! - `source_path` on dataset creation and ingest names a path the SERVER
//!   reads. The extension resolves `http(s)://` and `s3://` URLs as readily as
//!   local paths, so a caller chooses both which of the server's files are
//!   read and which hosts it contacts.
//! - `output_path` and `output_dir` on export and package write name a
//!   destination the SERVER writes, and an existing file at that path is
//!   replaced.
//! - `filter` on query and predicate-delete is a SQL predicate interpolated
//!   as written, because `cityparquet_delete` takes a predicate string by
//!   design.
//!
//! Together these mean the API belongs on a trusted network, operated by
//! people who already have the rights it exercises on their behalf. Exposing
//! it more widely needs authentication, a path policy confining reads and
//! writes to a configured root, and a restricted predicate grammar — none of
//! which exist.

pub mod dataset;
pub mod maintenance;
pub mod objects;
pub mod package;

use axum::extract::Multipart;

use crate::core::interface::types::CityLakeError;

/// Read the first (and only expected) part of a multipart upload into a
/// temporary file, and return it.
///
/// The temp file's suffix mirrors [`sql::reader_for`](crate::core::db::sql::reader_for)'s
/// own dispatch — `.jsonl` for CityJSONSeq, `.fcb` for FlatCityBuf, `.json`
/// otherwise — so an uploaded file is routed to the same reader a JSON-body
/// `source_path` ending the same way would get.
///
/// The caller must keep the returned `NamedTempFile` alive across the
/// repository call: it deletes its file on drop, and the extension needs the
/// path to still exist when it opens it.
///
/// A malformed multipart body (no file part, a stream error) is a client
/// mistake in principle, but `CityLakeError` has no dedicated bucket for it
/// beyond `Sql` — which is reserved for the dataset/module newtypes — so it
/// falls through the default arm to 500.
pub(crate) async fn receive_upload(
    mut multipart: Multipart,
) -> Result<tempfile::NamedTempFile, CityLakeError> {
    let field = multipart
        .next_field()
        .await
        .map_err(|e| CityLakeError::Internal(format!("reading multipart field: {e}")))?
        .ok_or_else(|| CityLakeError::Internal("no file part in multipart upload".to_string()))?;

    let file_name = field.file_name().unwrap_or("upload").to_ascii_lowercase();
    let suffix = if file_name.ends_with(".jsonl") {
        ".jsonl"
    } else if file_name.ends_with(".fcb") {
        ".fcb"
    } else {
        ".json"
    };

    let bytes = field
        .bytes()
        .await
        .map_err(|e| CityLakeError::Internal(format!("reading multipart body: {e}")))?;

    let temp = tempfile::Builder::new().suffix(suffix).tempfile()?;
    // Async: the upload can be up to `UPLOAD_BODY_LIMIT` (256 MiB), and a
    // synchronous write of that size on the executor would stall every other
    // request the way a blocking DuckDB call does — see
    // `repository_impl.rs`'s doc comment for the same discipline there.
    tokio::fs::write(temp.path(), &bytes).await?;
    Ok(temp)
}
