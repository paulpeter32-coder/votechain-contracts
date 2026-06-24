use crate::{
    ApiDoc, ApiError, Event, Indexer, ProposalDetail, ProposalListParams, ProposalSummary,
    VoteRecord,
};
use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use std::sync::{Arc, RwLock};
use utoipa::OpenApi;

#[derive(Clone)]
pub struct AppState {
    pub indexer: Arc<RwLock<Indexer>>,
}

#[utoipa::path(
    get,
    path = "/proposals",
    params(ProposalListParams),
    responses(
        (status = 200, description = "List proposals successfully", body = [ProposalSummary])
    )
)]
pub async fn list_proposals(
    State(state): State<AppState>,
    Query(params): Query<ProposalListParams>,
) -> Json<Vec<ProposalSummary>> {
    let offset = params.offset.unwrap_or(0);
    let limit = params.limit.unwrap_or(50).min(50);
    let state_filter = params.state.clone();
    let indexer = state.indexer.read().unwrap();
    Json(indexer.list_proposals(state_filter, offset, limit))
}

#[utoipa::path(
    get,
    path = "/proposals/{id}",
    responses(
        (status = 200, description = "Get proposal detail", body = ProposalDetail),
        (status = 404, description = "Proposal not found", body = ApiError)
    )
)]
pub async fn get_proposal(
    State(state): State<AppState>,
    Path(id): Path<u64>,
) -> Result<Json<ProposalDetail>, (StatusCode, Json<ApiError>)> {
    let indexer = state.indexer.read().unwrap();
    match indexer.get_proposal(id) {
        Some(proposal) => Ok(Json(proposal)),
        None => Err((
            StatusCode::NOT_FOUND,
            Json(ApiError {
                code: "ProposalNotFound".to_string(),
                message: format!("Proposal {} not found", id),
            }),
        )),
    }
}

#[utoipa::path(
    get,
    path = "/proposals/{id}/votes",
    responses(
        (status = 200, description = "Get proposal votes", body = [VoteRecord]),
        (status = 404, description = "Proposal not found", body = ApiError)
    )
)]
pub async fn get_proposal_votes(
    State(state): State<AppState>,
    Path(id): Path<u64>,
) -> Result<Json<Vec<VoteRecord>>, (StatusCode, Json<ApiError>)> {
    let indexer = state.indexer.read().unwrap();
    if indexer.get_proposal(id).is_none() {
        return Err((
            StatusCode::NOT_FOUND,
            Json(ApiError {
                code: "ProposalNotFound".to_string(),
                message: format!("Proposal {} not found", id),
            }),
        ));
    }
    Ok(Json(indexer.get_proposal_votes(id)))
}

#[utoipa::path(
    get,
    path = "/voters/{address}/votes",
    responses(
        (status = 200, description = "Get voter's vote history", body = [VoteRecord])
    )
)]
pub async fn get_voter_votes(
    State(state): State<AppState>,
    Path(address): Path<String>,
) -> Json<Vec<VoteRecord>> {
    let indexer = state.indexer.read().unwrap();
    Json(indexer.get_voter_votes(&address))
}

pub async fn ingest_event(
    State(state): State<AppState>,
    Json(event): Json<Event>,
) -> impl IntoResponse {
    let mut indexer = state.indexer.write().unwrap();
    indexer.ingest(event);
    StatusCode::NO_CONTENT
}

#[utoipa::path(
    get,
    path = "/openapi.json",
    responses(
        (status = 200, description = "OpenAPI specification", body = String)
    )
)]
pub async fn openapi_json() -> Json<utoipa::openapi::OpenApi> {
    Json(ApiDoc::openapi())
}
