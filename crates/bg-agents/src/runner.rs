//! The newsroom pipeline.
//!
//! One pass: ingest, triage, cluster, then route each open story to the Desk or
//! the Wire and hand it to Gander. Failures are per-story — one bad draft must
//! not stop the run — and the whole pass is bounded by the spend ceiling.

use crate::{curator, gander, gosling, herald, ombuds, quant, scout, scribe, sentinel};
use crate::{Ctx, FlockError, Result};
use bg_core::domain::StoryStatus;
use rust_decimal::Decimal;
use tracing::{info, warn};

#[derive(Debug, Default, Clone)]
pub struct PipelineReport {
    pub items_ingested: usize,
    pub items_triaged: usize,
    pub items_clustered: usize,
    pub desk_published: usize,
    pub desk_held: usize,
    pub desk_killed: usize,
    pub wire_published: usize,
    pub corrections: usize,
    pub errors: Vec<String>,
    pub cost_usd: Decimal,
}

impl PipelineReport {
    pub fn summary(&self) -> String {
        format!(
            "ingested {} · triaged {} · clustered {} · desk {}✓/{}⏸/{}✗ · wire {} · corrections {} · ${:.4}",
            self.items_ingested,
            self.items_triaged,
            self.items_clustered,
            self.desk_published,
            self.desk_held,
            self.desk_killed,
            self.wire_published,
            self.corrections,
            self.cost_usd
        )
    }
}

/// Options for one pass.
#[derive(Debug, Clone)]
pub struct RunOpts {
    pub ingest: bool,
    pub prices: bool,
    pub ombuds: bool,
    pub max_triage: i64,
    pub max_cluster: i64,
}

impl Default for RunOpts {
    fn default() -> Self {
        Self { ingest: true, prices: true, ombuds: true, max_triage: 100, max_cluster: 60 }
    }
}

/// Run the whole newsroom once.
pub async fn run_once(ctx: &Ctx, opts: &RunOpts) -> Result<PipelineReport> {
    let cost_before = ctx.spent_recently().await;
    let mut rep = PipelineReport::default();

    // -- Scout --------------------------------------------------------------
    if opts.ingest {
        match scout::run(ctx).await {
            Ok(r) => rep.items_ingested = r.items_new,
            Err(e) => rep.errors.push(format!("scout: {e}")),
        }
    }
    if opts.prices {
        if let Err(e) = scout::refresh_prices(ctx).await {
            rep.errors.push(format!("prices: {e}"));
        }
    }

    // -- Gosling ------------------------------------------------------------
    match gosling::run(ctx, opts.max_triage).await {
        Ok(n) => rep.items_triaged = n,
        Err(FlockError::BudgetExhausted { .. }) => {
            rep.errors.push("budget exhausted before triage".into());
            rep.cost_usd = ctx.spent_recently().await - cost_before;
            return Ok(rep);
        }
        Err(e) => rep.errors.push(format!("gosling: {e}")),
    }

    // -- Curator ------------------------------------------------------------
    match curator::run(ctx, opts.max_cluster).await {
        Ok(n) => rep.items_clustered = n,
        Err(e) => rep.errors.push(format!("curator: {e}")),
    }

    // -- Desk / Wire routing ------------------------------------------------
    let open = bg_db::stories::open(&ctx.db, 60).await?;
    let mut desk_budget = ctx.cfg.desk_max_per_run;

    for story in open {
        // Gosling scores non-news at zero; those never reach a surface.
        if story.newsworthiness == 0 {
            bg_db::stories::set_status(
                &ctx.db,
                story.id,
                StoryStatus::Killed,
                Some("not news (triage)"),
            )
            .await?;
            continue;
        }

        let go_desk = story.newsworthiness >= ctx.cfg.desk_threshold
            && story.source_count >= bg_core::policy::MIN_DESK_SOURCES as i32
            && desk_budget > 0;

        if go_desk {
            desk_budget -= 1;
            match desk_pipeline(ctx, story.id).await {
                Ok(gander::Outcome::Published { .. }) => rep.desk_published += 1,
                Ok(gander::Outcome::Held { .. }) => rep.desk_held += 1,
                Ok(gander::Outcome::Killed { .. }) => rep.desk_killed += 1,
                Err(FlockError::BudgetExhausted { .. }) => {
                    rep.errors.push("budget exhausted mid-desk".into());
                    break;
                }
                Err(e) => {
                    warn!(story = %story.id, error = %e, "desk pipeline failed");
                    rep.errors.push(format!("desk {}: {e}", story.slug));
                    // A failed draft is held, not left dangling in `drafting`
                    // where the next run would pick it up and fail again.
                    let _ = bg_db::stories::set_status(
                        &ctx.db,
                        story.id,
                        StoryStatus::Held,
                        Some(&format!("pipeline error: {e}")),
                    )
                    .await;
                }
            }
        } else {
            match herald::run(ctx, story.id).await {
                Ok(gander::Outcome::Published { .. }) => rep.wire_published += 1,
                Ok(_) => {}
                Err(FlockError::BudgetExhausted { .. }) => {
                    rep.errors.push("budget exhausted mid-wire".into());
                    break;
                }
                Err(e) => {
                    warn!(story = %story.id, error = %e, "wire failed");
                    rep.errors.push(format!("wire {}: {e}", story.slug));
                }
            }
        }
    }

    // -- Ombuds -------------------------------------------------------------
    if opts.ombuds {
        match ombuds::run(ctx, 10).await {
            Ok(n) => rep.corrections = n,
            Err(e) => rep.errors.push(format!("ombuds: {e}")),
        }
    }

    rep.cost_usd = ctx.spent_recently().await - cost_before;
    info!("{}", rep.summary());
    Ok(rep)
}

/// The Desk path for one story: draft → verify → context → copy → review.
pub async fn desk_pipeline(ctx: &Ctx, story: bg_core::ids::StoryId) -> Result<gander::Outcome> {
    bg_db::stories::set_status(&ctx.db, story, StoryStatus::Drafting, None).await?;

    let (claim_ids, body_md) = scribe::run(ctx, story).await?;
    sentinel::run(ctx, story).await?;
    // Market context is nice to have, not load-bearing — a failure here must
    // not sink an otherwise publishable story.
    if let Err(e) = quant::run(ctx, story).await {
        warn!(story = %story, error = %e, "quant failed; continuing without market context");
    }
    let copy = crate::copydesk::run(ctx, story, &body_md).await?;

    bg_db::stories::set_status(&ctx.db, story, StoryStatus::Review, None).await?;
    gander::review_desk(ctx, story, &claim_ids, &body_md, &copy).await
}
