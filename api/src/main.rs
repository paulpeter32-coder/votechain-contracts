use std::{net::SocketAddr, sync::{Arc, RwLock}};

use axum::{routing::get, routing::post, Router};
use tokio::net::TcpListener;
use tower_governor::{governor::GovernorConfigBuilder, GovernorLayer};
use votechain_api::{api, Indexer};
use api::AppState;

#[tokio::main]
async fn main() {
    let indexer = Arc::new(RwLock::new(Indexer::new()));
    let state = AppState { indexer };

    // Read endpoints: 100 requests/minute per IP
    // 1 token replenished every 600 ms, burst of 100
    let read_conf = Arc::new(
        GovernorConfigBuilder::default()
            .per_millisecond(600)
            .burst_size(100)
            .use_headers()
            .finish()
            .unwrap(),
    );

    // Write endpoints: 10 requests/minute per IP
    // 1 token replenished every 6 s, burst of 10
    let write_conf = Arc::new(
        GovernorConfigBuilder::default()
            .per_second(6)
            .burst_size(10)
            .use_headers()
            .finish()
            .unwrap(),
    );

    let read_routes = Router::new()
        .route("/proposals", get(api::list_proposals))
        .route("/proposals/{id}", get(api::get_proposal))
        .route("/proposals/{id}/votes", get(api::get_proposal_votes))
        .route("/voters/{address}/votes", get(api::get_voter_votes))
        .route("/openapi.json", get(api::openapi_json))
        .layer(GovernorLayer::new(read_conf));

    let write_routes = Router::new()
        .route("/ingest", post(api::ingest_event))
        .layer(GovernorLayer::new(write_conf));

    let app = Router::new()
        .merge(read_routes)
        .merge(write_routes)
        .with_state(state);

    let addr = SocketAddr::from(([0, 0, 0, 0], 3000));
    let listener = TcpListener::bind(addr).await.unwrap();
    println!("VoteChain API listening on {addr}");
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .await
    .unwrap();
}
