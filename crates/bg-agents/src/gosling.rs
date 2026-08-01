//! **Gosling** — first read on everything that lands.
//!
//! Runs on every raw item, so it must be cheap: the fast tier, one batched call
//! per group of items rather than one per item. Its job is to answer three
//! questions — is this news, what is it about, and roughly how much does it
//! matter — well enough that the expensive agents downstream only see material
//! worth their time.

use crate::{stage, Ctx, Result, StageOutput};
use bg_core::domain::{AgentRole, Category, ModelTier};
use bg_llm::{schema as sch, Request};
use serde::Deserialize;
use tracing::info;

pub const SYSTEM: &str = "Gosling is the first read on everything that lands.

For each item you are given, decide:
- is_news: true if this reports something that happened. False for opinion \
columns, price-prediction filler, sponsored posts, listicles, 'top 5 coins to \
watch' content, and anything that is purely promotional.
- category: which desk it belongs to.
- assets: ticker symbols the item is genuinely about (uppercase, no $ prefix). \
An article about an exchange hack that mentions BTC's price in passing is not \
about BTC. Empty list is correct and common.
- score: 0-100, how much a well-informed reader would care. Anchors: 90+ is a \
major hack, an enforcement action against a top-10 entity, or a chain halt. \
70-89 is a significant funding round, a large protocol upgrade, or a notable \
regulatory filing. 40-69 is routine industry news. Below 40 is noise.

Be strict. Most of what crosses the wire is not news.";

/// How many items go into one triage call.
///
/// Batching is what makes triaging every item affordable — 25 items in one
/// call instead of 25 calls. Larger batches save more but degrade judgement as
/// the model's attention spreads across the list.
const BATCH: usize = 25;

#[derive(Debug, Deserialize)]
struct TriageBatch {
    items: Vec<TriageItem>,
}

#[derive(Debug, Deserialize)]
struct TriageItem {
    index: i64,
    is_news: bool,
    category: String,
    assets: Vec<String>,
    score: f64,
}

fn schema(n: usize) -> serde_json::Value {
    let cats: Vec<&str> = Category::ALL.iter().map(|c| c.as_str()).collect();
    sch::object(
        vec![(
            "items",
            sch::array_n(
                sch::object(
                    vec![
                        ("index", sch::integer_index("index of the item, as given")),
                        ("is_news", sch::boolean("does this report something that happened")),
                        ("category", sch::enumeration(&cats, "desk")),
                        ("assets", sch::array(sch::string_hinted("ticker", "asset"), "tickers")),
                        ("score", sch::number_range("newsworthiness", 0.0, 100.0)),
                    ],
                    &["index", "is_news", "category", "assets", "score"],
                ),
                "one entry per item",
                n,
            ),
        )],
        &["items"],
    )
}

/// Triage every untriaged item.
pub async fn run(ctx: &Ctx, limit: i64) -> Result<usize> {
    let items = bg_db::items::untriaged(&ctx.db, limit).await?;
    if items.is_empty() {
        return Ok(0);
    }

    let system = crate::system_prompt(ctx, AgentRole::Gosling).await;
    let mut triaged = 0usize;

    for chunk in items.chunks(BATCH) {
        let chunk = chunk.to_vec();
        let system = system.clone();
        let n = stage(ctx, AgentRole::Gosling, None, "triage", |_run| async move {
            let mut prompt = String::from("Triage these items.\n\n");
            for (i, it) in chunk.iter().enumerate() {
                prompt.push_str(&format!(
                    "[{i}] {}\n    {}\n",
                    it.title,
                    it.summary_raw
                        .as_deref()
                        .map(|s| bg_core::text::truncate_words(s, 40))
                        .unwrap_or_default()
                ));
            }

            let req = Request::new("gosling.triage", ModelTier::Fast, system, prompt)
                .with_schema(schema(chunk.len()))
                .with_max_tokens(4_000);
            let (parsed, completion) = ctx.llm.complete_json::<TriageBatch>(&req).await?;

            let mut n = 0usize;
            for r in parsed.items {
                let Some(item) = chunk.get(r.index as usize) else { continue };
                // A non-news item is still marked triaged — otherwise it is
                // re-read on every run forever.
                let score = if r.is_news { r.score.clamp(0.0, 100.0) as i16 } else { 0 };
                let assets: Vec<String> = r
                    .assets
                    .iter()
                    .map(|a| a.trim().trim_start_matches('$').to_uppercase())
                    .filter(|a| !a.is_empty() && a.len() <= 10)
                    .collect();
                bg_db::items::mark_triaged(&ctx.db, item.id, Some(&r.category), &assets, score)
                    .await?;
                n += 1;
            }
            let note = format!("triaged {n} items");
            Ok(StageOutput::with(n, completion, note))
        })
        .await?;
        triaged += n;
    }

    info!(triaged, "gosling pass complete");
    Ok(triaged)
}
