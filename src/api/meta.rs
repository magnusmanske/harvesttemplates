//! Lookups that drive the form: wikis, templates, properties, permalinks.

use super::{ApiError, ApiResult, SharedState};
use crate::constraints;
use crate::harvest::JobSpec;
use crate::ids::{ItemId, PropertyId};
use crate::wiki::Site;
use crate::wiki::site::{NS_TEMPLATE, host_for};
use crate::wikidata::{ConstraintDef, ConstraintStatus};
use axum::extract::{Path, Query, State};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::{Value, json};

pub fn routes() -> Router<SharedState> {
    Router::new()
        .route("/spec/from-query", get(spec_from_query))
        .route("/spec/to-query", post(spec_to_query))
        .route("/site", get(site))
        .route("/template", get(template))
        .route("/property/{id}", get(property))
}

/// The original tool's permalink parameters → spec.
async fn spec_from_query(Query(pairs): Query<Vec<(String, String)>>) -> Json<JobSpec> {
    Json(JobSpec::from_legacy_query(&pairs))
}

async fn spec_to_query(Json(spec): Json<JobSpec>) -> Json<Value> {
    Json(json!({ "query": spec.to_legacy_query() }))
}

#[derive(Debug, Deserialize)]
struct WikiQuery {
    siteid: String,
    project: String,
    template: Option<String>,
}

async fn load_site(app: &SharedState, q: &WikiQuery) -> Result<Site, ApiError> {
    let host = host_for(&q.siteid, &q.project).map_err(|e| ApiError::bad_request(e.to_string()))?;
    Site::load(&app.clients.mw, &host).await.map_err(|_| ApiError::bad_request(format!("cannot reach {host}")))
}

async fn site(State(app): State<SharedState>, Query(q): Query<WikiQuery>) -> ApiResult<Site> {
    Ok(Json(load_site(&app, &q).await?))
}

/// Whether the template exists, and its redirects for the "include" checkboxes.
async fn template(State(app): State<SharedState>, Query(q): Query<WikiQuery>) -> ApiResult<Value> {
    let name = q.template.as_deref().unwrap_or_default();
    if name.trim().is_empty() {
        return Err(ApiError::bad_request("template missing"));
    }
    let site = load_site(&app, &q).await?;
    let key = site.db_key(NS_TEMPLATE, name);
    let redirects = site.template_redirects(&app.clients.mw, &key).await?;
    let url = format!("https://{}/wiki/{}", site.host, urlencoding::encode(&site.full_title(NS_TEMPLATE, &key)));
    Ok(Json(
        json!({ "exists": redirects.is_some(), "name": key.replace('_', " "), "redirects": redirects, "url": url }),
    ))
}

/// One entry per constraint type (the form selects by type), with its strictest status.
fn constraint_types(defs: &[ConstraintDef]) -> Vec<(ItemId, ConstraintStatus)> {
    let mut kinds: Vec<(ItemId, ConstraintStatus)> = vec![];
    for def in defs {
        match kinds.iter_mut().find(|(kind, _)| *kind == def.kind) {
            Some((_, status)) => *status = (*status).max(def.status),
            None => kinds.push((def.kind, def.status)),
        }
    }
    kinds
}

/// Datatype, constraints (with labels and whether we can check them) and allowed units.
async fn property(State(app): State<SharedState>, Path(id): Path<String>) -> ApiResult<Value> {
    let id: PropertyId = id.parse().map_err(ApiError::BadRequest)?;
    let info = app.clients.wikidata.property(id).await?.ok_or(ApiError::NotFound)?;
    let units = info.allowed_units();
    let mut label_ids: Vec<String> = info.constraints.iter().map(|c| c.kind.to_string()).collect();
    label_ids.extend(units.iter().flatten().flatten().map(ItemId::to_string));
    label_ids.dedup();
    let labels = app.clients.wikidata.labels(&label_ids).await?;
    let label = |q: &ItemId| labels.get(&q.to_string()).cloned().unwrap_or_else(|| q.to_string());
    let constraints: Vec<Value> = constraint_types(&info.constraints)
        .into_iter()
        .map(|(kind, status)| {
            json!({ "id": kind, "label": label(&kind), "status": status, "supported": constraints::find(kind).is_some() })
        })
        .collect();
    let units = units.map(|u| {
        u.iter()
            .map(|q| json!({ "id": q, "label": q.as_ref().map_or_else(|| "no unit".to_string(), label) }))
            .collect::<Vec<_>>()
    });
    Ok(Json(json!({
        "id": info.id,
        "label": info.label,
        "datatype": info.datatype_name,
        "supported": info.datatype.is_some(),
        "deprecated": info.deprecated,
        "formatter_url": info.formatter_url,
        "constraints": constraints,
        "units": units,
    })))
}
