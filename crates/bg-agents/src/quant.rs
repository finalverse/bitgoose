//! **Quant** — puts the numbers in context.
//!
//! Attaches live market data to a story and identifies which assets it is
//! actually about. Deliberately narrow: it does not interpret, forecast, or
//! explain price action, because "BTC fell 3% on the news" is a causal claim
//! that almost never survives scrutiny. It supplies the numbers and leaves the
//! reader to draw the line.

use crate::{stage, Ctx, Result, StageOutput};
use bg_core::domain::{AgentRole, ModelTier};
use bg_core::ids::StoryId;
use bg_llm::{schema as sch, Request};
use serde::Deserialize;
use tracing::info;

pub const SYSTEM: &str = "Quant identifies which assets a story is genuinely about and \
supplies market context.

Return only tickers the story is materially about. A story about an exchange \
outage that mentions BTC's price in passing is not about BTC. Most stories are \
about zero or one asset. An empty list is a good answer.

The primary asset is the one a reader would file the story under; null if none.

Never assert that a price movement was caused by the story. Correlation over \
hours is not causation, and claiming it is the fastest way to publish something \
false.";

#[derive(Debug, Deserialize)]
struct AssetPick {
    assets: Vec<String>,
    primary_asset: String,
}

fn schema() -> serde_json::Value {
    sch::object(
        vec![
            (
                "assets",
                sch::array(
                    sch::string_hinted("ticker", "asset"),
                    "tickers the story is about",
                ),
            ),
            (
                "primary_asset",
                sch::string_hinted("main ticker or empty", "asset"),
            ),
        ],
        &["assets", "primary_asset"],
    )
}

/// Market context attached to a story.
#[derive(Debug, Clone, Default)]
pub struct MarketContext {
    pub primary_asset: Option<String>,
    pub assets: Vec<String>,
    /// `(symbol, price, 24h change %)` at publication time.
    pub snapshot: Vec<(String, rust_decimal::Decimal, Option<f64>)>,
}

pub async fn run(ctx: &Ctx, story: StoryId) -> Result<MarketContext> {
    let s = bg_db::stories::by_id(&ctx.db, story).await?;
    let claims = bg_db::claims::by_story(&ctx.db, story).await?;
    let system = crate::system_prompt(ctx, AgentRole::Quant).await;

    stage(
        ctx,
        AgentRole::Quant,
        Some(story),
        "market",
        |_run| async move {
            let mut prompt = format!("Story: {}\n\nClaims:\n", s.title);
            for c in &claims {
                prompt.push_str(&format!("- {}\n", c.text));
            }
            prompt.push_str("\nWhich assets is this story materially about?");

            let req = Request::new("quant.assets", ModelTier::Mid, system, prompt)
                .with_schema(schema())
                .with_max_tokens(1_000);
            let (pick, completion) = ctx.llm.complete_json::<AssetPick>(&req).await?;

            let assets: Vec<String> = pick
                .assets
                .iter()
                .map(|a| a.trim().trim_start_matches('$').to_uppercase())
                .filter(|a| !a.is_empty() && a.len() <= 10)
                .collect();
            let primary = Some(
                pick.primary_asset
                    .trim()
                    .trim_start_matches('$')
                    .to_uppercase(),
            )
            .filter(|p| !p.is_empty())
            .or_else(|| assets.first().cloned());

            let mut snapshot = Vec::new();
            for sym in assets.iter().take(4) {
                if let Ok(Some(tick)) = bg_db::prices::latest(&ctx.db, sym).await {
                    snapshot.push((tick.symbol, tick.price_usd, tick.change_24h_pct));
                }
            }

            bg_db::stories::set_meta(&ctx.db, story, None, primary.as_deref(), &assets, None)
                .await?;

            let note = if assets.is_empty() {
                "no assets — story is not market-specific".to_string()
            } else {
                format!("assets: {}", assets.join(", "))
            };
            info!(story = %story, ?assets, "quant pass");
            Ok(StageOutput::with(
                MarketContext {
                    primary_asset: primary,
                    assets,
                    snapshot,
                },
                completion,
                note,
            ))
        },
    )
    .await
}
