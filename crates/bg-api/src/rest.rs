//! REST surface at `/v1`.

use crate::ApiState;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::str::FromStr;

pub fn routes() -> Router<ApiState> {
    Router::new()
        .route("/v1", get(index))
        .route("/v1/health", get(health))
        .route("/v1/stories", get(list_stories))
        .route("/v1/stories/{slug}", get(get_story))
        .route("/v1/wire", get(wire))
        .route("/v1/claims/{id}", get(get_claim))
        .route("/v1/prices", get(prices))
        .route("/v1/assets/{ticker}", get(asset_stories))
        .route("/v1/flock", get(flock))
        .route("/v1/standards", get(standards))
        .route("/openapi.json", get(openapi))
}

/// API error that renders as JSON rather than a bare status code.
pub struct ApiError(StatusCode, String);

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.0, Json(json!({ "error": self.1 }))).into_response()
    }
}

impl From<bg_db::DbError> for ApiError {
    fn from(e: bg_db::DbError) -> Self {
        match e {
            bg_db::DbError::NotFound(what) => {
                ApiError(StatusCode::NOT_FOUND, format!("{what} not found"))
            }
            other => {
                tracing::error!(error = %other, "api database error");
                ApiError(StatusCode::INTERNAL_SERVER_ERROR, "internal error".into())
            }
        }
    }
}

type ApiResult<T> = Result<T, ApiError>;

#[derive(Debug, Deserialize)]
pub struct Page {
    #[serde(default = "default_limit")]
    limit: i64,
    #[serde(default)]
    offset: i64,
    kind: Option<String>,
    category: Option<String>,
}

fn default_limit() -> i64 {
    30
}

impl Page {
    /// Clamped so a client cannot ask for the whole archive in one request.
    fn limit(&self) -> i64 {
        self.limit.clamp(1, 100)
    }
    fn offset(&self) -> i64 {
        self.offset.max(0)
    }
}

async fn index() -> Json<serde_json::Value> {
    Json(json!({
        "name": bg_core::brand::NAME,
        "tagline": bg_core::brand::TAGLINE,
        "version": bg_core::API_VERSION,
        "description": "The claim graph behind BitGoose, machine-readable. Every story \
                        decomposes into claims; every claim carries its sources and a \
                        confidence score.",
        "endpoints": {
            "GET /v1/stories": "published stories (?kind=desk|wire&category=&limit=&offset=)",
            "GET /v1/stories/{slug}": "one story with its full claim ledger",
            "GET /v1/wire": "the fast aggregated feed",
            "GET /v1/claims/{id}": "one claim with every source backing it",
            "GET /v1/prices": "latest market data",
            "GET /v1/assets/{ticker}": "coverage for one asset",
            "GET /v1/flock": "live agent activity, cost and error rate",
            "GET /v1/standards": "editorial policy and the enforcement record",
            "POST /mcp": "MCP server (JSON-RPC 2.0) for AI agents"
        },
        "license": "Claims and metadata are freely reusable with attribution. \
                    Source text is never redistributed."
    }))
}

async fn health(State(s): State<ApiState>) -> ApiResult<Json<serde_json::Value>> {
    s.db.ping().await?;
    Ok(Json(json!({ "status": "ok" })))
}

#[derive(Serialize)]
struct StorySummary {
    slug: String,
    kind: String,
    title: String,
    summary: Option<String>,
    category: String,
    newsworthiness: i16,
    source_count: i32,
    assets: Vec<String>,
    published_at: Option<chrono::DateTime<chrono::Utc>>,
}

fn summarize(s: &bg_core::domain::Story) -> StorySummary {
    StorySummary {
        slug: s.slug.clone(),
        kind: s.kind.as_str().into(),
        title: s.title.clone(),
        summary: s.summary.clone(),
        category: s.category.as_str().into(),
        newsworthiness: s.newsworthiness,
        source_count: s.source_count,
        assets: s.assets.clone(),
        published_at: s.published_at,
    }
}

async fn list_stories(
    State(s): State<ApiState>,
    Query(p): Query<Page>,
) -> ApiResult<Json<serde_json::Value>> {
    let kind = p
        .kind
        .as_deref()
        .and_then(|k| bg_core::domain::StoryKind::from_str(k).ok());
    let stories = match p
        .category
        .as_deref()
        .and_then(|c| bg_core::domain::Category::from_str(c).ok())
    {
        Some(cat) => bg_db::stories::by_category(&s.db, cat, p.limit()).await?,
        None => bg_db::stories::published(&s.db, kind, p.limit(), p.offset()).await?,
    };
    Ok(Json(json!({
        "count": stories.len(),
        "stories": stories.iter().map(summarize).collect::<Vec<_>>()
    })))
}

async fn get_story(
    State(s): State<ApiState>,
    Path(slug): Path<String>,
) -> ApiResult<Json<serde_json::Value>> {
    let story = bg_db::stories::published_by_slug(&s.db, &slug).await?;
    let article = bg_db::articles::latest_for_story(&s.db, story.id).await?;
    let claims = bg_db::claims::with_sources(&s.db, story.id).await?;
    let sources = bg_db::stories::source_refs(&s.db, story.id).await?;
    let corrections = bg_db::articles::corrections_for_story(&s.db, story.id).await?;
    let runs = bg_db::agents::runs_for_story(&s.db, story.id).await?;

    Ok(Json(json!({
        "story": summarize(&story),
        "article": article,
        "claims": claims,
        "sources": sources,
        "corrections": corrections,
        // Provenance is part of the payload, not a separate endpoint: an agent
        // consuming a story should be able to see how it was produced without
        // a second request.
        "produced_by": runs,
    })))
}

async fn wire(
    State(s): State<ApiState>,
    Query(p): Query<Page>,
) -> ApiResult<Json<serde_json::Value>> {
    let entries = bg_db::stories::wire(&s.db, None, p.limit(), p.offset()).await?;
    Ok(Json(json!({ "count": entries.len(), "wire": entries })))
}

async fn get_claim(
    State(s): State<ApiState>,
    Path(id): Path<String>,
) -> ApiResult<Json<serde_json::Value>> {
    let claim_id = bg_core::ids::ClaimId::from_str(&id)
        .map_err(|_| ApiError(StatusCode::BAD_REQUEST, "claim id must be a uuid".into()))?;
    let claim = bg_db::claims::by_id(&s.db, claim_id).await?;
    let all = bg_db::claims::with_sources(&s.db, claim.story_id).await?;
    let with_sources = all
        .into_iter()
        .find(|c| c.claim.id == claim_id)
        .ok_or(ApiError(StatusCode::NOT_FOUND, "claim not found".into()))?;
    let story = bg_db::stories::by_id(&s.db, claim.story_id).await?;
    Ok(Json(
        json!({ "claim": with_sources, "story_slug": story.slug }),
    ))
}

async fn prices(State(s): State<ApiState>) -> ApiResult<Json<serde_json::Value>> {
    Ok(Json(
        json!({ "prices": bg_db::prices::latest_all(&s.db).await? }),
    ))
}

async fn asset_stories(
    State(s): State<ApiState>,
    Path(ticker): Path<String>,
) -> ApiResult<Json<serde_json::Value>> {
    let stories = bg_db::stories::by_asset(&s.db, &ticker, 40).await?;
    let price = bg_db::prices::latest(&s.db, &ticker).await?;
    Ok(Json(json!({
        "ticker": ticker.to_uppercase(),
        "price": price,
        "count": stories.len(),
        "stories": stories.iter().map(summarize).collect::<Vec<_>>()
    })))
}

async fn flock(State(s): State<ApiState>) -> ApiResult<Json<serde_json::Value>> {
    let stats = bg_db::agents::flock_stats(&s.db).await?;
    let recent = bg_db::agents::recent_runs(&s.db, 40).await?;
    let totals = bg_db::agents::newsroom_totals(&s.db).await?;
    Ok(Json(json!({
        "totals": {
            "runs_24h": totals.runs_24h,
            "failures_24h": totals.failures_24h,
            "tokens_24h": totals.tokens_24h,
            "cost_24h_usd": totals.cost_24h,
            "stories_published_24h": totals.stories_published_24h,
            "claims_24h": totals.claims_24h,
        },
        "agents": stats.iter().map(|a| json!({
            "role": a.role.as_str(),
            "name": a.name,
            "beat": a.role.beat(),
            "tier": a.role.tier().as_str(),
            "runs_24h": a.runs_24h,
            "ok_24h": a.ok_24h,
            "failed_24h": a.failed_24h,
            "cost_24h_usd": a.cost_24h_usd,
            "tokens_24h": a.tokens_24h,
            "avg_latency_ms": a.avg_latency_ms,
            "last_run_at": a.last_run_at,
            "last_note": a.last_note,
            "enabled": a.enabled,
        })).collect::<Vec<_>>(),
        "recent": recent,
    })))
}

async fn standards(State(s): State<ApiState>) -> ApiResult<Json<serde_json::Value>> {
    let tally = bg_db::violations::tally(&s.db, 30).await?;
    let blocks = bg_db::violations::count_blocks_24h(&s.db).await?;
    let sources = bg_db::sources::all(&s.db).await?;
    Ok(Json(json!({
        "disclosure": bg_core::brand::AI_DISCLOSURE,
        "policy": {
            "max_quote_words": bg_core::policy::MAX_QUOTE_WORDS,
            "max_verbatim_run_words": bg_core::policy::MAX_VERBATIM_RUN,
            "min_desk_sources": bg_core::policy::MIN_DESK_SOURCES,
            "source_text_republished": false,
            "attribution_and_linkout": "required on every source, enforced at publish time",
        },
        "enforcement_30d": tally.iter().map(|(c, n)| json!({ "code": c, "count": n })).collect::<Vec<_>>(),
        "blocks_24h": blocks,
        "sources": sources.iter().map(|s| json!({
            "slug": s.slug, "name": s.name, "homepage": s.homepage,
            "trust": s.trust, "enabled": s.enabled, "robots_ok": s.robots_ok,
        })).collect::<Vec<_>>(),
    })))
}

async fn openapi() -> Json<serde_json::Value> {
    Json(json!({
        "openapi": "3.1.0",
        "info": {
            "title": "BitGoose API",
            "version": bg_core::API_VERSION,
            "description": bg_core::brand::TAGLINE,
        },
        "paths": {
            "/v1/stories": { "get": { "summary": "List published stories" } },
            "/v1/stories/{slug}": { "get": { "summary": "One story with its claim ledger" } },
            "/v1/wire": { "get": { "summary": "The aggregated Wire feed" } },
            "/v1/claims/{id}": { "get": { "summary": "One claim with all backing sources" } },
            "/v1/prices": { "get": { "summary": "Latest market data" } },
            "/v1/assets/{ticker}": { "get": { "summary": "Coverage for one asset" } },
            "/v1/flock": { "get": { "summary": "Live AI newsroom activity and cost" } },
            "/v1/standards": { "get": { "summary": "Editorial policy and enforcement record" } }
        }
    }))
}
