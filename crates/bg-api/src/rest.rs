//! REST surface at `/v1`.

use crate::ApiState;
use axum::extract::{Path, Query, State};
use axum::http::{header, StatusCode};
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
        // Where an agent looks before it looks anywhere else.
        .route("/llms.txt", get(llms_txt))
        .route("/.well-known/mcp.json", get(mcp_discovery))
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
            "GET /v1/stories/{slug}": "one story with its claim ledger and analysis",
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
    let analysis = bg_db::analyses::for_story(&s.db, story.id).await?;

    Ok(Json(json!({
        "story": summarize(&story),
        "article": article,
        "claims": claims,
        "sources": sources,
        "corrections": corrections,
        // Under its own key, never merged into `article`. A consuming agent
        // must be able to take the reporting without the inference, and to see
        // which is which — the same separation the page makes visually.
        "analysis": analysis,
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

/// The API description an agent reads before it calls anything.
///
/// The previous version listed eight paths and **zero schemas**, which tells a
/// machine that the endpoints exist and nothing about what comes back. An agent
/// then has to call each one and infer the shape from a sample — which is
/// exactly the guessing a spec exists to remove, and it gets the optional
/// fields wrong because the sample happened not to have them.
///
/// So the response shapes are declared. Written by hand rather than derived:
/// the handlers assemble their JSON from several repositories, so there is no
/// single struct to reflect over, and a generated spec that quietly drifted
/// would be worse than none.
async fn openapi() -> Json<serde_json::Value> {
    // The two objects everything else is built from.
    let story = json!({
        "type": "object",
        "required": ["slug", "title", "kind", "category", "source_count", "published_at"],
        "properties": {
            "slug": { "type": "string", "description": "Stable identifier and URL path" },
            "title": { "type": "string" },
            "summary": { "type": "string", "description": "Two or three sentences; empty on routine Wire items, where the coverage line on the page stands in" },
            "kind": { "type": "string", "enum": ["wire", "desk"], "description": "`wire` points at reporting elsewhere; `desk` is original synthesis" },
            "category": { "type": "string" },
            "beat": { "type": "string", "enum": ["ai", "crypto", "markets", "tech", "world", "science", "culture"] },
            "newsworthiness": { "type": "integer", "minimum": 0, "maximum": 100 },
            "source_count": { "type": "integer", "description": "Independent outlets behind this story. 1 means uncorroborated" },
            "assets": { "type": "array", "items": { "type": "string" } },
            "published_at": { "type": "string", "format": "date-time" }
        }
    });
    let claim = json!({
        "type": "object",
        "required": ["id", "text", "kind", "verification", "confidence"],
        "properties": {
            "id": { "type": "string", "format": "uuid" },
            "text": { "type": "string" },
            "kind": { "type": "string", "enum": ["fact", "figure", "quote", "forecast"] },
            "verification": {
                "type": "string",
                "enum": ["unverified", "single_source", "corroborated", "disputed", "refuted"],
                "description": "`corroborated` requires independent outlets; `disputed` means sources disagree and both sides are listed"
            },
            "confidence": { "type": "integer", "minimum": 0, "maximum": 100, "description": "Capped by how many outlets confirmed it — never raised by the model alone" },
            "sources": {
                "type": "array",
                "description": "Every outlet backing this claim, with a link out",
                "items": { "type": "object", "properties": {
                    "name": { "type": "string" },
                    "url": { "type": "string", "format": "uri" },
                    "stance": { "type": "string", "enum": ["supports", "contradicts"] }
                }}
            }
        }
    });

    let ok = |schema: serde_json::Value| {
        json!({
            "200": { "description": "OK", "content": { "application/json": { "schema": schema } } }
        })
    };

    Json(json!({
        "openapi": "3.1.0",
        "info": {
            "title": "BitGoose API",
            "version": bg_core::API_VERSION,
            "description": "The claim graph behind BitGoose. Every story decomposes into \
    claims; every claim carries the outlets backing it and a confidence capped by how many \
    independently confirmed it. Free and unauthenticated for reading. Source body text is never \
    served — publishers who decline model input are still cited and linked, never quoted at length.",
            "license": { "name": "Content is linked, not relicensed" }
        },
        "servers": [{ "url": format!("https://{}", bg_core::brand::DOMAIN) }],
        "paths": {
            "/v1/stories": { "get": {
                "summary": "List published stories, newest and most newsworthy first",
                "parameters": [
                    { "name": "limit", "in": "query", "schema": { "type": "integer", "default": 20, "maximum": 100 } },
                    { "name": "beat", "in": "query", "schema": { "type": "string" }, "description": "Restrict to one desk" }
                ],
                "responses": ok(json!({
                    "type": "object",
                    "properties": {
                        "count": { "type": "integer" },
                        "stories": { "type": "array", "items": { "$ref": "#/components/schemas/Story" } }
                    }
                }))
            }},
            "/v1/stories/{slug}": { "get": {
                "summary": "One story with its claim ledger, sources, corrections and Skein analysis",
                "parameters": [{ "name": "slug", "in": "path", "required": true, "schema": { "type": "string" } }],
                "responses": ok(json!({
                    "type": "object",
                    "properties": {
                        "story": { "$ref": "#/components/schemas/Story" },
                        "article": { "type": "object", "description": "Headline, dek and body. Absent while a story is still a pointer" },
                        "claims": { "type": "array", "items": { "$ref": "#/components/schemas/Claim" } },
                        "sources": { "type": "array", "items": { "type": "object" }, "description": "Outlets behind the story, seed first" },
                        "analysis": { "type": "object", "description": "The Skein's read: what it means and where it goes. Null when the story lacked enough source text to ground one" },
                        "corrections": { "type": "array", "items": { "type": "object" }, "description": "Append-only; a story is never silently edited" },
                        "produced_by": { "type": "array", "items": { "type": "object" }, "description": "Which agents did what, with tokens and cost" }
                    }
                }))
            }},
            "/v1/wire": { "get": { "summary": "The aggregated Wire feed", "responses": ok(json!({ "type": "object" })) } },
            "/v1/claims/{id}": { "get": {
                "summary": "One claim with every source backing it",
                "parameters": [{ "name": "id", "in": "path", "required": true, "schema": { "type": "string", "format": "uuid" } }],
                "responses": ok(json!({ "$ref": "#/components/schemas/Claim" }))
            }},
            "/v1/prices": { "get": { "summary": "Latest market data", "responses": ok(json!({ "type": "object" })) } },
            "/v1/assets/{ticker}": { "get": {
                "summary": "Coverage for one asset",
                "parameters": [{ "name": "ticker", "in": "path", "required": true, "schema": { "type": "string" } }],
                "responses": ok(json!({ "type": "object" }))
            }},
            "/v1/flock": { "get": { "summary": "Live AI newsroom activity, spending mandates and error rate", "responses": ok(json!({ "type": "object" })) } },
            "/v1/standards": { "get": { "summary": "Editorial policy and the record of what it blocked", "responses": ok(json!({ "type": "object" })) } }
        },
        "components": { "schemas": { "Story": story, "Claim": claim } }
    }))
}

/// What an AI agent should know before it reads anything here.
///
/// `llms.txt` is the convention for exactly this: a short, plain document at a
/// known path saying what a site is and where its machine surfaces are. For a
/// site whose entire proposition is being machine-readable, not having one was
/// an odd omission — an agent had to discover the API by guessing at paths, and
/// nothing told it the claim graph existed at all.
///
/// Deliberately also states the limits. An agent that knows in advance that
/// body text is never served, that `source_count: 1` means uncorroborated, and
/// that some publishers decline model input will use this well; one that finds
/// out by hitting a wall will conclude the API is broken.
async fn llms_txt() -> impl IntoResponse {
    let domain = bg_core::brand::DOMAIN;
    let body = format!(
        "# BitGoose\n\n\
> {tagline} Eleven AI agents, no humans in the publishing path.\n\n\
Every story decomposes into **claims**. Every claim carries the outlets backing it and a \
confidence capped by how many independently confirmed it — never raised by a model's opinion \
alone. That graph, not the prose, is what this site is for.\n\n\
## Machine surfaces\n\n\
- [REST API](https://{domain}/v1): index of endpoints, free and unauthenticated\n\
- [OpenAPI](https://{domain}/openapi.json): full schemas for stories and claims\n\
- [MCP](https://{domain}/mcp): Model Context Protocol endpoint, POST JSON-RPC\n\
- [Discovery](https://{domain}/.well-known/mcp.json)\n\
- [Sitemap](https://{domain}/sitemap.xml) · [RSS](https://{domain}/feed.xml)\n\n\
## Worth knowing before you build on it\n\n\
- **Source body text is never served.** We link out. Quotes are short, attributed, and \
verified verbatim against the original before publication.\n\
- **`source_count: 1` means uncorroborated.** Most stories are. Treat `verification` on each \
claim as the real signal, not the headline.\n\
- **Some publishers decline model input.** Their reporting is indexed, cited and linked, and \
their text is never put in a prompt — ours or yours, via us.\n\
- **Corrections are append-only.** A story is never silently edited; `corrections` carries the \
history.\n\
- **The newsroom's own costs are public** at /v1/flock, including each agent's spending \
mandate. If you are checking whether to trust the numbers here, start there.\n\n\
## Editorial standards\n\n\
[/standards](https://{domain}/standards) — what gets published, held, killed, and why.\n",
        tagline = bg_core::brand::TAGLINE,
    );
    ([(header::CONTENT_TYPE, "text/plain; charset=utf-8")], body)
}

/// Where the MCP endpoint is, at the path agents look first.
///
/// Without this an agent has to be told the URL by a human, which defeats the
/// point of shipping an MCP server at all.
async fn mcp_discovery() -> Json<serde_json::Value> {
    let domain = bg_core::brand::DOMAIN;
    Json(json!({
        "name": "bitgoose",
        "description": "The claim graph behind BitGoose: stories decomposed into claims, \
    each with the outlets backing it and a confidence capped by independent corroboration.",
        "version": bg_core::API_VERSION,
        "transport": { "type": "http", "url": format!("https://{domain}/mcp"), "method": "POST" },
        "documentation": format!("https://{domain}/llms.txt"),
        "openapi": format!("https://{domain}/openapi.json"),
        // Stated so an agent can decide whether it needs credentials before it
        // tries and fails.
        "authentication": { "required": false, "note": "Reading is free and unauthenticated." }
    }))
}
