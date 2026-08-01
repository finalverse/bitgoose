//! MCP server — BitGoose as a tool for other AI agents.
//!
//! The infrastructure play. A crypto agent that needs to know what happened
//! today should not be scraping HTML and guessing at what is corroborated; it
//! should call `verify_claim` and get back a confidence score with its sources.
//!
//! Implements the JSON-RPC 2.0 subset MCP clients actually use — `initialize`,
//! `tools/list`, `tools/call` — over a single POST endpoint. A full transport
//! implementation is a dependency we do not need for a read-only tool server.

use crate::ApiState;
use axum::extract::State;
use axum::routing::post;
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::{json, Value};
use std::str::FromStr;

/// MCP protocol revision this server implements.
const PROTOCOL_VERSION: &str = "2025-06-18";

pub fn routes() -> Router<ApiState> {
    Router::new().route("/mcp", post(handle))
}

#[derive(Debug, Deserialize)]
pub struct RpcRequest {
    #[serde(default)]
    jsonrpc: String,
    #[serde(default)]
    id: Value,
    method: String,
    #[serde(default)]
    params: Value,
}

fn ok(id: Value, result: Value) -> Json<Value> {
    Json(json!({ "jsonrpc": "2.0", "id": id, "result": result }))
}

fn err(id: Value, code: i32, message: &str) -> Json<Value> {
    Json(json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } }))
}

/// Wrap a payload as MCP tool output.
fn tool_text(value: Value) -> Value {
    json!({
        "content": [{
            "type": "text",
            "text": serde_json::to_string_pretty(&value).unwrap_or_else(|_| "{}".into())
        }]
    })
}

fn tools() -> Value {
    json!([
        {
            "name": "search_stories",
            "description": "Search published BitGoose stories. Returns headline, summary, \
                            category, source count and slug. Use get_story for the full \
                            claim ledger.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "category": { "type": "string", "description": "markets, policy, tech, defi, business, security, ai, nft, gaming, culture" },
                    "asset": { "type": "string", "description": "ticker, e.g. BTC" },
                    "limit": { "type": "integer", "description": "1-50, default 10" }
                }
            }
        },
        {
            "name": "get_story",
            "description": "One story by slug, with every claim, each claim's verification \
                            state and confidence, and the sources backing it.",
            "inputSchema": {
                "type": "object",
                "properties": { "slug": { "type": "string" } },
                "required": ["slug"]
            }
        },
        {
            "name": "verify_claim",
            "description": "Look up how well-supported an assertion is. Returns matching \
                            claims with their verification state, confidence score and the \
                            independent sources behind each. Use this instead of trusting a \
                            headline.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "query": { "type": "string", "description": "the assertion to check" },
                    "limit": { "type": "integer", "description": "1-20, default 5" }
                },
                "required": ["query"]
            }
        },
        {
            "name": "get_prices",
            "description": "Latest market data for tracked assets: price, 24h change, volume, market cap.",
            "inputSchema": { "type": "object", "properties": {} }
        },
        {
            "name": "newsroom_status",
            "description": "Live status of the BitGoose AI newsroom: per-agent run counts, \
                            error rate, token spend and what each agent last did.",
            "inputSchema": { "type": "object", "properties": {} }
        }
    ])
}

async fn handle(State(s): State<ApiState>, Json(req): Json<RpcRequest>) -> Json<Value> {
    let id = req.id.clone();
    if !req.jsonrpc.is_empty() && req.jsonrpc != "2.0" {
        return err(id, -32600, "jsonrpc must be \"2.0\"");
    }

    match req.method.as_str() {
        "initialize" => ok(
            id,
            json!({
                "protocolVersion": PROTOCOL_VERSION,
                "capabilities": { "tools": {} },
                "serverInfo": { "name": "bitgoose", "version": env!("CARGO_PKG_VERSION") },
                "instructions": "BitGoose is an AI-run crypto newsroom. Its distinguishing \
                                 feature is the claim graph: every published assertion carries \
                                 its sources and a confidence score. Prefer verify_claim over \
                                 taking a headline at face value."
            }),
        ),
        // Notifications carry no id and expect no result.
        m if m.starts_with("notifications/") => ok(id, json!({})),
        "ping" => ok(id, json!({})),
        "tools/list" => ok(id, json!({ "tools": tools() })),
        "tools/call" => {
            let name = req.params.get("name").and_then(|v| v.as_str()).unwrap_or("");
            let args = req.params.get("arguments").cloned().unwrap_or_else(|| json!({}));
            match call_tool(&s, name, &args).await {
                Ok(v) => ok(id, tool_text(v)),
                Err(e) => ok(
                    id,
                    // MCP convention: tool failures are results with isError,
                    // not protocol errors — the model should see the message.
                    json!({
                        "content": [{ "type": "text", "text": e }],
                        "isError": true
                    }),
                ),
            }
        }
        other => err(id, -32601, &format!("unknown method: {other}")),
    }
}

async fn call_tool(s: &ApiState, name: &str, args: &Value) -> Result<Value, String> {
    let limit = |default: i64, max: i64| -> i64 {
        args.get("limit").and_then(|v| v.as_i64()).unwrap_or(default).clamp(1, max)
    };

    match name {
        "search_stories" => {
            let stories = if let Some(asset) = args.get("asset").and_then(|v| v.as_str()) {
                bg_db::stories::by_asset(&s.db, asset, limit(10, 50)).await
            } else if let Some(cat) = args
                .get("category")
                .and_then(|v| v.as_str())
                .and_then(|c| bg_core::domain::Category::from_str(c).ok())
            {
                bg_db::stories::by_category(&s.db, cat, limit(10, 50)).await
            } else {
                bg_db::stories::published(&s.db, None, limit(10, 50), 0).await
            }
            .map_err(|e| e.to_string())?;

            Ok(json!({
                "count": stories.len(),
                "stories": stories.iter().map(|st| json!({
                    "slug": st.slug,
                    "title": st.title,
                    "summary": st.summary,
                    "category": st.category.as_str(),
                    "kind": st.kind.as_str(),
                    "source_count": st.source_count,
                    "published_at": st.published_at,
                    "url": format!("https://{}/story/{}", bg_core::brand::DOMAIN, st.slug),
                })).collect::<Vec<_>>()
            }))
        }

        "get_story" => {
            let slug = args
                .get("slug")
                .and_then(|v| v.as_str())
                .ok_or("slug is required")?;
            let story = bg_db::stories::by_slug(&s.db, slug).await.map_err(|e| e.to_string())?;
            let claims = bg_db::claims::with_sources(&s.db, story.id).await.map_err(|e| e.to_string())?;
            let article =
                bg_db::articles::latest_for_story(&s.db, story.id).await.map_err(|e| e.to_string())?;

            Ok(json!({
                "slug": story.slug,
                "title": article.as_ref().map(|a| a.headline.clone()).unwrap_or(story.title.clone()),
                "dek": article.as_ref().map(|a| a.dek.clone()),
                "body_md": article.as_ref().map(|a| a.body_md.clone()),
                "category": story.category.as_str(),
                "published_at": story.published_at,
                "claims": claims.iter().map(|c| json!({
                    "id": c.claim.id,
                    "text": c.claim.text,
                    "kind": c.claim.kind.as_str(),
                    "verification": c.claim.verification.as_str(),
                    "confidence": c.claim.confidence,
                    "sources": c.sources.iter().map(|src| json!({
                        "outlet": src.source_name,
                        "url": src.url,
                        "stance": src.stance.as_str(),
                        "trust": src.source_trust,
                    })).collect::<Vec<_>>()
                })).collect::<Vec<_>>()
            }))
        }

        "verify_claim" => {
            let query = args.get("query").and_then(|v| v.as_str()).ok_or("query is required")?;
            // Trigram similarity over claim text. Postgres does the ranking,
            // so this stays one round trip regardless of archive size.
            let rows = sqlx::query_as::<_, (uuid::Uuid, String, String, f32, String, f32)>(
                "SELECT c.id, c.text, c.verification, c.confidence, s.slug, similarity(c.text, $1)
                 FROM claims c
                 JOIN stories s ON s.id = c.story_id
                 WHERE s.status = 'published' AND similarity(c.text, $1) > 0.1
                 ORDER BY similarity(c.text, $1) DESC
                 LIMIT $2",
            )
            .bind(query)
            .bind(limit(5, 20))
            .fetch_all(&s.db.pool)
            .await
            .map_err(|e| e.to_string())?;

            let mut out = Vec::new();
            for (id, text, verification, confidence, story_slug, score) in rows {
                let cid = bg_core::ids::ClaimId::from_uuid(id);
                let sources = sqlx::query_as::<_, (String, String, i16)>(
                    "SELECT src.name, r.canonical_url, src.trust
                     FROM claim_sources cs
                     JOIN raw_items r ON r.id = cs.raw_item_id
                     JOIN sources src ON src.id = r.source_id
                     WHERE cs.claim_id = $1 AND cs.stance = 'supports'",
                )
                .bind(cid.into_uuid())
                .fetch_all(&s.db.pool)
                .await
                .unwrap_or_default();

                out.push(json!({
                    "claim": text,
                    "verification": verification,
                    "confidence": confidence,
                    "match_score": score,
                    "story_url": format!("https://{}/story/{}", bg_core::brand::DOMAIN, story_slug),
                    "independent_sources": sources.len(),
                    "sources": sources.iter().map(|(n, u, t)| json!({
                        "outlet": n, "url": u, "trust": t
                    })).collect::<Vec<_>>(),
                }));
            }

            Ok(json!({
                "query": query,
                "matches": out.len(),
                "claims": out,
                "note": "confidence is capped by the number of independent sources; a claim \
                         with one source is never rated corroborated"
            }))
        }

        "get_prices" => {
            let prices = bg_db::prices::latest_all(&s.db).await.map_err(|e| e.to_string())?;
            Ok(json!({ "prices": prices }))
        }

        "newsroom_status" => {
            let stats = bg_db::agents::flock_stats(&s.db).await.map_err(|e| e.to_string())?;
            let totals = bg_db::agents::newsroom_totals(&s.db).await.map_err(|e| e.to_string())?;
            Ok(json!({
                "runs_24h": totals.runs_24h,
                "failures_24h": totals.failures_24h,
                "cost_24h_usd": totals.cost_24h,
                "stories_published_24h": totals.stories_published_24h,
                "agents": stats.iter().map(|a| json!({
                    "name": a.name,
                    "beat": a.role.beat(),
                    "runs_24h": a.runs_24h,
                    "failed_24h": a.failed_24h,
                    "last_note": a.last_note,
                })).collect::<Vec<_>>()
            }))
        }

        other => Err(format!("unknown tool: {other}")),
    }
}
