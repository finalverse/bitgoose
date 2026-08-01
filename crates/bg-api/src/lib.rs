//! # bg-api — the machine-readable layer
//!
//! The half of the thesis that isn't a website. Decrypt and CoinDesk publish
//! HTML; an agent that wants their reporting has to scrape it. BitGoose
//! publishes the claim graph directly — every story, claim, source and
//! confidence score over REST, plus an MCP server so an AI agent can query the
//! newsroom as a tool rather than parsing pages.
//!
//! Everything here is public and unauthenticated by design. The product is the
//! provenance; making it hard to read would defeat the point.

pub mod mcp;
pub mod rest;

use axum::Router;
use bg_db::Db;
use tower_http::cors::{Any, CorsLayer};

#[derive(Clone)]
pub struct ApiState {
    pub db: Db,
}

/// The full public API, ready to merge into a server.
pub fn router(db: Db) -> Router {
    let state = ApiState { db };
    Router::new()
        .merge(rest::routes())
        .merge(mcp::routes())
        // Open CORS: this is a public read-only API meant to be called from
        // anywhere, including other people's browsers and agents.
        .layer(CorsLayer::new().allow_origin(Any).allow_methods(Any).allow_headers(Any))
        .with_state(state)
}
