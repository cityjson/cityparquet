//! A dataset's objects: ingest further sources, query a module's page,
//! update or delete one object, delete by predicate.

use std::sync::Arc;

use axum::extract::{Multipart, Path, Query, State};
use axum::http::StatusCode;
use axum::Json;
use serde::Deserialize;

use crate::core::interface::repository::CityLakeRepository;
use crate::core::interface::types::{CityLakeError, DatasetName, ModuleName, QueryParams};

use super::receive_upload;

#[derive(Debug, Deserialize)]
pub struct IngestBody {
    source_path: String,
}

/// `POST /datasets/{ds}/objects` — ingest a further source into an existing
/// dataset.
pub async fn ingest(
    State(repo): State<Arc<dyn CityLakeRepository>>,
    Path(dataset): Path<String>,
    Json(body): Json<IngestBody>,
) -> Result<(StatusCode, Json<serde_json::Value>), CityLakeError> {
    let dataset = DatasetName::new(&dataset)?;
    let ingested = repo.ingest(&dataset, &body.source_path).await?;
    Ok((
        StatusCode::CREATED,
        Json(serde_json::json!({ "ingested": ingested })),
    ))
}

/// `POST /datasets/{ds}/objects/upload` — the multipart variant of [`ingest`].
pub async fn ingest_upload(
    State(repo): State<Arc<dyn CityLakeRepository>>,
    Path(dataset): Path<String>,
    multipart: Multipart,
) -> Result<(StatusCode, Json<serde_json::Value>), CityLakeError> {
    let dataset = DatasetName::new(&dataset)?;
    let temp = receive_upload(multipart).await?;
    let source_path = temp
        .path()
        .to_str()
        .ok_or_else(|| CityLakeError::Internal("temp file path is not valid UTF-8".to_string()))?;
    let ingested = repo.ingest(&dataset, source_path).await?;
    Ok((
        StatusCode::CREATED,
        Json(serde_json::json!({ "ingested": ingested })),
    ))
}

/// The query string a page read accepts. `QueryParams` itself has no
/// `Deserialize` — the same validate-at-the-boundary posture the `DatasetName`
/// and `ModuleName` newtypes take, so nothing can construct one straight from
/// untrusted request text. This DTO is that boundary: it deserialises what
/// arrived over HTTP, then `From` converts it into `QueryParams`, defaulting
/// the numeric fields.
#[derive(Debug, Deserialize)]
pub struct QueryParamsDto {
    filter: Option<String>,
    limit: Option<usize>,
    offset: Option<usize>,
}

impl From<QueryParamsDto> for QueryParams {
    fn from(dto: QueryParamsDto) -> Self {
        let defaults = QueryParams::default();
        QueryParams {
            filter: dto.filter,
            limit: dto.limit.unwrap_or(defaults.limit),
            offset: dto.offset.unwrap_or(defaults.offset),
        }
    }
}

/// `GET /datasets/{ds}/modules/{module}/objects?filter=&limit=&offset=`.
///
/// `filter`, when present, is a caller-supplied SQL predicate interpolated as
/// written — the same trust model `cityparquet_delete` assumes. The API has
/// no authentication; this endpoint does not attempt to sanitise or parse it.
pub async fn query(
    State(repo): State<Arc<dyn CityLakeRepository>>,
    Path((dataset, module)): Path<(String, String)>,
    Query(dto): Query<QueryParamsDto>,
) -> Result<Json<Vec<serde_json::Value>>, CityLakeError> {
    let dataset = DatasetName::new(&dataset)?;
    let module = ModuleName::new(&module)?;
    let params: QueryParams = dto.into();
    Ok(Json(repo.query_objects(&dataset, &module, &params).await?))
}

/// `PUT /datasets/{ds}/objects/{id}` — the body is a JSON object of attribute
/// names to new values.
pub async fn update(
    State(repo): State<Arc<dyn CityLakeRepository>>,
    Path((dataset, id)): Path<(String, String)>,
    Json(attributes): Json<serde_json::Map<String, serde_json::Value>>,
) -> Result<StatusCode, CityLakeError> {
    let dataset = DatasetName::new(&dataset)?;
    repo.update_object(&dataset, &id, &attributes).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// `DELETE /datasets/{ds}/objects/{id}`.
pub async fn delete(
    State(repo): State<Arc<dyn CityLakeRepository>>,
    Path((dataset, id)): Path<(String, String)>,
) -> Result<Json<serde_json::Value>, CityLakeError> {
    let dataset = DatasetName::new(&dataset)?;
    let deleted = repo.delete_object(&dataset, &id).await?;
    Ok(Json(serde_json::json!({ "deleted": deleted })))
}

#[derive(Debug, Deserialize)]
pub struct DeleteWhereQuery {
    filter: String,
}

/// `DELETE /datasets/{ds}/objects?filter=` — delete every object the
/// predicate matches, cascading through `children`.
///
/// As with [`query`], `filter` is a caller-supplied SQL predicate interpolated
/// as written, by design, with no authentication in front of it. It is
/// required here — an absent filter would otherwise read as "delete
/// everything", which axum's `Query` extractor already refuses with 400 rather
/// than this handler silently defaulting to it.
pub async fn delete_where(
    State(repo): State<Arc<dyn CityLakeRepository>>,
    Path(dataset): Path<String>,
    Query(query): Query<DeleteWhereQuery>,
) -> Result<Json<serde_json::Value>, CityLakeError> {
    let dataset = DatasetName::new(&dataset)?;
    let deleted = repo.delete_where(&dataset, &query.filter).await?;
    Ok(Json(serde_json::json!({ "deleted": deleted })))
}
