//! **Scout** — polls every source, normalizes, dedupes.
//!
//! The only agent with no model behind it. Ingestion is a solved deterministic
//! problem; putting a model on it would add cost, latency and nondeterminism to
//! something that is already correct. It is still a first-class agent with a
//! ledger row, because "how much did the newsroom fetch today" is part of the
//! public record.

use crate::{stage, Ctx, Result, StageOutput};
use bg_core::domain::AgentRole;
use tracing::info;

pub const SYSTEM: &str = "Scout watches every source on the roster, around the clock. \
It is deterministic and calls no model: it fetches feeds politely (conditional GET, \
robots.txt, rate limits), canonicalizes URLs, fingerprints content, and drops \
duplicates before anything else in the newsroom sees them.";

#[derive(Debug, Default, Clone)]
pub struct ScoutReport {
    pub sources_polled: usize,
    pub sources_failed: usize,
    pub sources_stale: usize,
    pub items_new: usize,
    pub not_modified: usize,
}

/// Poll every source that is due.
/// Fetch article text for items that do not have it yet.
///
/// Runs in the pipeline rather than only from the CLI, because otherwise the
/// Skein has nothing to read for anything published after the last manual
/// `bg enrich` — the analysis would quietly only ever cover the back catalogue.
///
/// Bounded per pass and serialised with a delay: this host has a ~15 KB/s
/// downlink shared with the live site, and an unbounded fetch loop measurably
/// slowed the site for readers while it ran.
pub async fn enrich(ctx: &Ctx, limit: i64) -> Result<(usize, usize)> {
    let targets = bg_db::items::needing_extraction(&ctx.db, limit).await?;
    if targets.is_empty() {
        return Ok((0, 0));
    }
    let respect_robots = std::env::var("BG_RESPECT_ROBOTS")
        .map(|v| v != "false")
        .unwrap_or(true);

    let (mut got, mut missed) = (0usize, 0usize);
    for (id, url) in &targets {
        match bg_ingest::readable::fetch(&ctx.http, &ctx.cfg.user_agent, url, respect_robots).await
        {
            Ok(Some(ex)) => {
                bg_db::items::record_extraction(&ctx.db, *id, Some(&ex.text), ex.via).await?;
                got += 1;
            }
            Ok(None) => {
                bg_db::items::record_extraction(&ctx.db, *id, None, "none").await?;
                missed += 1;
            }
            Err(_) => {
                // Counted, not marked done: a blip retries, a refusal gives up
                // after MAX_EXTRACT_ATTEMPTS.
                bg_db::items::record_extract_failure(&ctx.db, *id).await?;
                missed += 1;
            }
        }
        tokio::time::sleep(std::time::Duration::from_millis(750)).await;
    }
    info!(got, missed, "scout enriched");
    Ok((got, missed))
}

pub async fn run(ctx: &Ctx) -> Result<ScoutReport> {
    stage(ctx, AgentRole::Scout, None, "poll", |_run| async move {
        // Re-read robots.txt before polling, not just at seed time. This was
        // written for that and then never called, which meant `robots_ok` held
        // whatever the seed assumed — and we polled a site whose robots.txt had
        // said `Disallow: /` the entire time. A permission checked once is a
        // permission assumed.
        let verdicts = bg_ingest::refresh_robots(&ctx.db, &ctx.http, &ctx.cfg.user_agent).await;
        let blocked = verdicts.iter().filter(|(_, ok)| !ok).count();
        if blocked > 0 {
            info!(blocked, "sources currently disallowed by robots.txt");
        }

        let reports =
            bg_ingest::feeds::poll_due(&ctx.db, &ctx.http, ctx.cfg.ingest_concurrency).await;

        let mut r = ScoutReport::default();
        for rep in &reports {
            if rep.error.is_some() {
                r.sources_failed += 1;
                if rep.is_stale_feed() {
                    r.sources_stale += 1;
                }
            } else {
                r.sources_polled += 1;
            }
            r.items_new += rep.inserted;
            if rep.not_modified {
                r.not_modified += 1;
            }
        }

        info!(
            polled = r.sources_polled,
            failed = r.sources_failed,
            new = r.items_new,
            not_modified = r.not_modified,
            "scout swept"
        );
        let note = format!(
            "{} new items from {} sources ({} unchanged, {} failed)",
            r.items_new, r.sources_polled, r.not_modified, r.sources_failed
        );
        Ok(StageOutput::plain(r, note))
    })
    .await
}

/// Refresh market data. Folded into Scout because it is the same job — reach
/// out, fetch, normalize, store — and because a separate agent for it would
/// clutter `/flock` without telling a reader anything new.
pub async fn refresh_prices(ctx: &Ctx) -> Result<usize> {
    stage(ctx, AgentRole::Scout, None, "prices", |_run| async move {
        let n = bg_ingest::market::refresh(&ctx.db, &ctx.http).await;
        Ok(StageOutput::plain(n, format!("{n} price ticks")))
    })
    .await
}
