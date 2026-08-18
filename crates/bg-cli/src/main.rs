//! `bg` — the BitGoose operations CLI.

use anyhow::{Context, Result};
use bg_agents::{runner, Ctx, FlockConfig};
use bg_db::Db;
use bg_llm::Llm;
use clap::{Parser, Subcommand};
use std::time::Duration;
use tracing_subscriber::EnvFilter;

#[derive(Parser)]
#[command(name = "bg", version, about = "BitGoose newsroom operations")]
struct Cli {
    /// Postgres URL. Defaults to DATABASE_URL.
    #[arg(long, env = "DATABASE_URL", global = true)]
    database_url: Option<String>,

    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Apply database migrations.
    Migrate,
    /// Seed sources, assets, entities and the agent roster.
    Seed,
    /// Check database, pgvector, sources and LLM providers.
    Doctor,
    /// Poll every due source once.
    Ingest,
    /// Refresh market prices.
    Prices,
    /// Run the newsroom pipeline once.
    Run {
        /// Skip feed polling (use what is already ingested).
        #[arg(long)]
        no_ingest: bool,
        /// Skip the post-publish correction pass.
        #[arg(long)]
        no_ombuds: bool,
        /// Override the provider for this run (anthropic | openai | stub).
        #[arg(long)]
        provider: Option<String>,
    },
    /// Run the pipeline on a loop.
    Worker {
        /// Seconds between full passes.
        #[arg(long, default_value_t = 300)]
        interval: u64,
        /// Seconds between fast passes — feeds, crawls and trend scoring.
        ///
        /// Separate because that work consults no model and so is not paced by
        /// the token budget. A full pass can take most of an hour on a free
        /// tier; a trending topic that updates hourly is not trending.
        #[arg(long, default_value_t = 90)]
        fast_interval: u64,
    },
    /// Print newsroom statistics.
    Stats,
    /// Show recent policy violations.
    Violations {
        #[arg(long, default_value_t = 20)]
        limit: i64,
    },
    /// Re-check published stories and issue corrections.
    Ombuds {
        #[arg(long, default_value_t = 10)]
        limit: i64,
    },
    /// Re-judge published stories with the currently configured model.
    ///
    /// Story ranking is driven by triage scores, which were produced by
    /// whichever model was configured when the item first landed. Swapping in a
    /// stronger one does nothing to the archive by itself, so the front page
    /// keeps whatever the old one thought — which is how a stock-promo item
    /// ended up leading a desk. This re-triages the highest-ranked stories and
    /// recomputes their scores.
    Rescore {
        #[arg(long, default_value_t = 25)]
        limit: i64,
    },
    /// Write summaries for Wire stories published without one.
    ///
    /// Everything published while the offline stub was the only provider has a
    /// story page consisting of a headline and a source list, because the
    /// stub's summaries only restated the headline and were dropped. This
    /// re-runs Herald over them with whatever provider is now configured.
    RefreshWire {
        #[arg(long, default_value_t = 25)]
        limit: i64,
    },
    /// Fetch the article page for ingested items and extract its text.
    ///
    /// RSS gives a headline and two sentences. That is enough to route and to
    /// summarise, but not enough to analyse: measured over the archive, most
    /// published stories carry under 1,000 characters of source text, and
    /// analysis drawn from that is analysis of a headline. This fetches the
    /// real page, honouring robots.txt per URL rather than per feed.
    Enrich {
        #[arg(long, default_value_t = 40)]
        limit: i64,
        /// Seconds to wait between fetches, per host politeness.
        #[arg(long, default_value_t = 2)]
        delay: u64,
    },
    /// Withdraw material we should not be serving.
    ///
    /// Two things, both consequences of bugs already fixed:
    ///
    /// Seven sources were polled for weeks against a `Disallow: /`, because
    /// the function that re-reads robots.txt existed and was never called.
    /// Stopping was half the fix; the other half is not continuing to serve
    /// what was taken. Stories with at least one permitted source stand — the
    /// event is real and someone we could read reported it. Stories whose every
    /// source disallows us do not.
    ///
    /// And eight stories merge up to twenty unrelated events, artifacts of one
    /// early run whose clustering was adjudicated by the deterministic stub.
    /// A site claiming every assertion shows its sources cannot serve a page
    /// that is not about one thing.
    ///
    /// Killed, not deleted, in every case: reversible, auditable, and the
    /// record of what was published and withdrawn stays intact.
    Retract {
        /// Report what would change without changing it.
        #[arg(long)]
        dry_run: bool,
    },
    /// Put a source back in the rotation, or take it out.
    ///
    /// The Steward rests a source that is failing and producing nothing. That
    /// has to be reversible or it is not a safe action, and `bg seed` does not
    /// do it — the upsert deliberately leaves `enabled` alone so an operator's
    /// decision survives a redeploy.
    Source {
        /// Source slug, as shown by `bg doctor`.
        slug: String,
        /// Put it back in the rotation.
        #[arg(long, conflicts_with = "rest")]
        wake: bool,
        /// Take it out.
        #[arg(long)]
        rest: bool,
    },
    /// Look after the newsroom: check its health, fix what is safe, report
    /// the rest.
    ///
    /// Uses no inference at all. The condition most worth catching is the
    /// newsroom being broken, and one of the ways it breaks is the inference
    /// provider refusing us — a check that needed a model would be offline
    /// exactly when it mattered.
    Steward {
        /// Actually apply the safe fixes. Without this, only reports.
        #[arg(long)]
        apply: bool,
    },
    /// Re-examine published single-source stories and fold together the ones
    /// that were always one event.
    ///
    /// The Curator only ever sees an item once. Everything it failed to merge
    /// while the matcher was too strict — or while the model it depended on was
    /// rate limited — is still sitting on the site as its own story, and no
    /// amount of fixing the live path repairs that. This is the repair.
    ///
    /// Deterministic: the same rare-vocabulary test the Curator now uses, at
    /// its confident threshold, with no model in the loop. Dry by default.
    Recluster {
        /// How far back to look.
        #[arg(long, default_value_t = 336)]
        hours: i64,
        #[arg(long, default_value_t = 1500)]
        limit: i64,
        /// How far apart two stories may have been first seen and still be one
        /// event. Beyond this it is a running story, which is what a gaggle is
        /// for — the CLARITY Act ran for a fortnight and is a dozen events, not
        /// one.
        #[arg(long, default_value_t = 48)]
        apart_hours: i64,
        /// Actually fold them. Without this, only reports.
        #[arg(long)]
        apply: bool,
    },
    /// Run the Skein over published stories: what it means, where it goes.
    ///
    /// Skips any story without enough real source text behind it. Run `enrich`
    /// first — the two together are what turns a link aggregator into analysis.
    Analyze {
        #[arg(long, default_value_t = 10)]
        limit: i64,
        /// Re-analyse stories that already have one.
        #[arg(long)]
        redo: bool,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    // `.env` before anything reads the environment; a missing file is fine.
    let _ = dotenvy::dotenv();
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .with_target(false)
        .init();

    let cli = Cli::parse();
    let url = cli
        .database_url
        .clone()
        .context("DATABASE_URL is not set (copy .env.example to .env)")?;

    match cli.cmd {
        Cmd::Migrate => {
            let db = Db::connect(&url).await?;
            db.migrate().await?;
            println!("migrations applied");
        }

        Cmd::Seed => {
            let db = Db::connect(&url).await?;
            let s = bg_ingest::seed::seed_sources(&db).await?;
            let a = bg_ingest::seed::seed_assets(&db).await?;
            let e = bg_ingest::seed::seed_entities(&db).await?;
            let r = bg_agents::seed_roster(&db).await?;
            println!("seeded {s} sources, {a} assets, {e} entities, {r} agents");
        }

        Cmd::Doctor => doctor(&url).await?,

        Cmd::Ingest => {
            let ctx = context(&url, None).await?;
            let r = bg_agents::scout::run(&ctx).await?;
            println!(
                "polled {} sources ({} unchanged, {} failed) — {} new items",
                r.sources_polled, r.not_modified, r.sources_failed, r.items_new
            );
        }

        Cmd::Prices => {
            let ctx = context(&url, None).await?;
            let n = bg_agents::scout::refresh_prices(&ctx).await?;
            println!("{n} price ticks written");
        }

        Cmd::Run {
            no_ingest,
            no_ombuds,
            provider,
        } => {
            let ctx = context(&url, provider).await?;
            let opts = runner::RunOpts {
                ingest: !no_ingest,
                prices: !no_ingest,
                ombuds: !no_ombuds,
                ..Default::default()
            };
            let rep = runner::run_once(&ctx, &opts).await?;
            println!("\n{}", rep.summary());
            if !rep.errors.is_empty() {
                println!("\nerrors:");
                for e in &rep.errors {
                    println!("  - {e}");
                }
            }
        }

        Cmd::Worker {
            interval,
            fast_interval,
        } => {
            let ctx = context(&url, None).await?;
            println!(
                "worker started — full pass every {interval}s, fast pass every {fast_interval}s"
            );

            // The gap is measured from the *end* of a pass, not its start.
            // Token pacing means a pass now takes as long as the budget
            // requires rather than a predictable few seconds, so a fixed
            // wall-clock schedule would either overlap passes or drift.
            let base = Duration::from_secs(interval);
            // Backing off on repeated failure keeps a broken provider or a
            // dead database from being hammered every interval, and keeps the
            // journal readable enough to see what actually went wrong.
            let mut consecutive_failures = 0u32;
            // The Steward runs on its own clock, not every pass: its checks are
            // about days (a silent desk, a barren source) and running them each
            // time would be noise in the log without being any earlier to
            // notice anything.
            let mut next_steward = std::time::Instant::now();

            loop {
                let started = std::time::Instant::now();

                let backlog = match runner::run_once(&ctx, &runner::RunOpts::default()).await {
                    Ok(r) => {
                        println!(
                            "[{}] {} ({}s)",
                            chrono::Utc::now().to_rfc3339(),
                            r.summary(),
                            started.elapsed().as_secs()
                        );
                        consecutive_failures = 0;
                        // Did this pass leave work on the table?
                        r.items_triaged > 0 || r.enriched > 0 || r.analysed > 0
                    }
                    Err(e) => {
                        eprintln!("[{}] pass failed: {e}", chrono::Utc::now().to_rfc3339());
                        consecutive_failures = consecutive_failures.saturating_add(1);
                        false
                    }
                };

                // *After* the pass, detached, and on a clock of its own.
                //
                // It used to run inline before the pass, on the reasoning that
                // clearing a fault first is tidier. Then it grew two checks
                // that touch the network — the image backfill and the delivery
                // probe — and on a host doing about 7 KB/s those take longer
                // than the pass interval. Production went **seventeen minutes
                // publishing nothing**, worker healthy, log silent, stuck in
                // its first round.
                //
                // Nothing the Steward does is urgent to the minute. The
                // newsroom's own work is, so the Steward is spawned and the
                // loop never waits on it. The timeout is the backstop: a round
                // that cannot finish in five minutes has met something worse
                // than slow, and a second copy piling up behind it would be
                // the same bug again.
                if std::time::Instant::now() >= next_steward {
                    next_steward = std::time::Instant::now() + STEWARD_EVERY;
                    let sctx = ctx.clone();
                    tokio::spawn(async move {
                        let round = bg_agents::steward::run(&sctx, true);
                        match tokio::time::timeout(STEWARD_MAX_ROUND, round).await {
                            Ok(Ok(f)) if f.is_empty() => {}
                            Ok(Ok(f)) => {
                                let (fixed, human): (Vec<_>, Vec<_>) =
                                    f.iter().partition(|x| !x.needs_a_human());
                                println!(
                                    "[{}] steward: {} fixed, {} need a human",
                                    chrono::Utc::now().to_rfc3339(),
                                    fixed.len(),
                                    human.len()
                                );
                                for x in human {
                                    println!("    ? [{}] {}", x.kind, x.detail);
                                }
                            }
                            Ok(Err(e)) => eprintln!("steward round failed: {e}"),
                            Err(_) => eprintln!(
                                "steward round exceeded {}s and was abandoned; \
                                 the next one starts clean",
                                STEWARD_MAX_ROUND.as_secs()
                            ),
                        }
                    });
                }

                let wait = if consecutive_failures > 0 {
                    // 2x, 4x, 8x … capped at an hour.
                    let factor = 1u64 << consecutive_failures.min(5);
                    (base * factor as u32).min(Duration::from_secs(3600))
                } else if backlog {
                    // There was work and there is likely more — a feed burst,
                    // or a queue we are chewing through. Come back promptly
                    // rather than idling for the full interval with items
                    // waiting. Floor of 30s so a busy period cannot become a
                    // hot loop against the publishers or the token budget.
                    (base / 4).max(Duration::from_secs(30))
                } else {
                    // Nothing moved. Feeds are polled on their own per-source
                    // intervals anyway, so there is nothing to gain from
                    // checking more often than this.
                    base
                };

                println!("  next full pass in {}s", wait.as_secs());

                // Fast passes fill the gap. Feeds, index crawls and trend
                // scoring cost no tokens, so the front page stays current even
                // while the budgeted half of the pipeline is waiting its turn.
                //
                // Capped to the gap: when the adaptive interval shortens to a
                // quarter of base because work is flowing, a 90s fast pass does
                // not fit inside a 75s wait and none ran at all — the fast lane
                // was idling exactly when the news was moving.
                let fast = Duration::from_secs(fast_interval.max(30)).min(wait / 2);
                let deadline = std::time::Instant::now() + wait;
                while std::time::Instant::now() + fast < deadline {
                    tokio::time::sleep(fast).await;
                    match runner::run_fast(&ctx).await {
                        Ok(r) if r.items_ingested > 0 || r.gaggles > 0 => println!(
                            "  [{}] fast: {} new, {} topics refreshed",
                            chrono::Utc::now().to_rfc3339(),
                            r.items_ingested,
                            r.gaggles
                        ),
                        Ok(_) => {}
                        Err(e) => eprintln!("  fast pass failed: {e}"),
                    }
                }
                let remaining = deadline.saturating_duration_since(std::time::Instant::now());
                tokio::time::sleep(remaining).await;
            }
        }

        Cmd::Stats => stats(&url).await?,

        Cmd::Violations { limit } => {
            let db = Db::connect(&url).await?;
            let rows = bg_db::violations::recent(&db, limit).await?;
            if rows.is_empty() {
                println!("no policy violations recorded");
            }
            for v in rows {
                println!(
                    "{}  {:<24} {:<6} {}",
                    v.created_at.format("%m-%d %H:%M"),
                    v.code,
                    v.severity,
                    v.detail
                );
            }
        }

        Cmd::Ombuds { limit } => {
            let ctx = context(&url, None).await?;
            let n = bg_agents::ombuds::run(&ctx, limit).await?;
            println!("{n} correction(s) issued");
        }
        Cmd::Rescore { limit } => {
            let ctx = context(&url, None).await?;
            let stories = bg_db::stories::top_published(&ctx.db, limit).await?;
            println!("re-judging {} stor(ies)", stories.len());
            let mut items = 0u64;
            for s in &stories {
                items += bg_db::items::reset_triage_for_story(&ctx.db, s.id).await?;
            }
            println!("  {items} item(s) queued for re-triage");
            let n = bg_agents::gosling::run(&ctx, (items as i64).max(1)).await?;
            println!("  {n} re-triaged");
            for s in &stories {
                let before = s.newsworthiness;
                let after = bg_agents::curator::rescore(&ctx, s.id).await?;
                if (after - before).abs() >= 8 {
                    println!("  {before:>3} -> {after:>3}  {}", s.slug);
                }
            }
            println!("done");
        }
        Cmd::RefreshWire { limit } => {
            let ctx = context(&url, None).await?;
            let stories = bg_db::stories::needing_summary(&ctx.db, limit).await?;
            println!("{} wire stor(ies) without a summary", stories.len());
            let (mut done, mut failed) = (0usize, 0usize);
            for s in &stories {
                // One story's model failing must not abandon the rest of the
                // batch; local inference is slow enough that a rerun is costly.
                match bg_agents::herald::run(&ctx, s.id).await {
                    Ok(_) => {
                        done += 1;
                        println!("  ok   {}", s.slug);
                    }
                    Err(e) => {
                        failed += 1;
                        println!("  FAIL {} — {e}", s.slug);
                    }
                }
            }
            println!("{done} refreshed, {failed} failed");
        }

        Cmd::Recluster {
            hours,
            limit,
            apart_hours,
            apply,
        } => {
            let db = Db::connect(&url).await?;
            // Before anything else: make the counter agree with the evidence.
            // A story that has three outlets attached but says one is already
            // wrong, and would also be picked up here as a singleton needing a
            // merge it has already had.
            match bg_db::stories::reconcile_source_counts(&db).await {
                Ok(0) => {}
                Ok(n) => println!("corrected source_count on {n} stories"),
                Err(e) => eprintln!("could not reconcile source counts: {e}"),
            }

            // Folds made before `merged_into` existed left their URLs serving
            // an empty 200. The destination is gone from the join table by
            // then, so it is reconstructed the only way left: re-run the same
            // matcher over the husk's title against what is still published.
            match bg_db::stories::folds_missing_destination(&db, hours).await {
                Ok((orphans, live)) if !orphans.is_empty() => {
                    let corpus = bg_core::samestory::Corpus::of(
                        &live
                            .iter()
                            .chain(orphans.iter())
                            .map(|(_, t, _)| t.clone())
                            .collect::<Vec<_>>(),
                    );
                    let mut fixed = 0usize;
                    for (id, title, _) in &orphans {
                        let best = live
                            .iter()
                            .map(|(lid, lt, _)| {
                                (lid, bg_core::samestory::overlap(title, lt, &corpus))
                            })
                            .filter(|(_, o)| o.confident())
                            .max_by(|a, b| a.1.score.total_cmp(&b.1.score));
                        if let Some((target, _)) = best {
                            bg_db::stories::set_merged_into(&db, *id, *target).await?;
                            fixed += 1;
                        }
                    }
                    if fixed > 0 {
                        println!("restored {fixed} of {} fold redirects", orphans.len());
                    }
                }
                Ok(_) => {}
                Err(e) => eprintln!("could not repair fold redirects: {e}"),
            }

            let stories = bg_db::stories::singletons(&db, hours, limit).await?;
            println!("examining {} single-source stories", stories.len());
            let corpus = bg_core::samestory::Corpus::of(
                &stories
                    .iter()
                    .map(|(_, t, _)| t.clone())
                    .collect::<Vec<_>>(),
            );

            // Union-find, because merges are transitive: where A matches B and
            // B matches C, the answer is one story of three sources, not two
            // pairs. Folding pairwise in sequence would leave the second merge
            // pointing at a story that has already been withdrawn.
            let mut parent: Vec<usize> = (0..stories.len()).collect();
            fn find(p: &mut [usize], mut i: usize) -> usize {
                while p[i] != i {
                    p[i] = p[p[i]];
                    i = p[i];
                }
                i
            }
            let apart = apart_hours * 3600;
            let mut pairs = 0usize;
            for i in 0..stories.len() {
                for j in (i + 1)..stories.len() {
                    // Same event means same few days. Two reports on one bill a
                    // fortnight apart are two events.
                    if (stories[i].2 - stories[j].2).abs() > apart {
                        continue;
                    }
                    let o = bg_core::samestory::overlap(&stories[i].1, &stories[j].1, &corpus);
                    if !o.confident() {
                        continue;
                    }
                    let (a, b) = (find(&mut parent, i), find(&mut parent, j));
                    if a == b {
                        continue;
                    }
                    // Against the cluster's representative, not merely against
                    // some member of it. Chained agreement is not agreement:
                    // the first run of this reached three unrelated finance
                    // stories from a crypto bill, one "Wall Street" at a time.
                    let (keep, fold) = if stories[a].2 <= stories[b].2 {
                        (a, b)
                    } else {
                        (b, a)
                    };
                    if keep != i
                        && keep != j
                        && !bg_core::samestory::overlap(&stories[keep].1, &stories[fold].1, &corpus)
                            .confident()
                    {
                        continue;
                    }
                    pairs += 1;
                    parent[fold] = keep;
                }
            }

            let mut groups: std::collections::HashMap<usize, Vec<usize>> = Default::default();
            for i in 0..stories.len() {
                let r = find(&mut parent, i);
                groups.entry(r).or_default().push(i);
            }
            let groups: Vec<_> = groups.into_iter().filter(|(_, v)| v.len() > 1).collect();
            let folded: usize = groups.iter().map(|(_, v)| v.len() - 1).sum();

            println!(
                "{pairs} confident pairs -> {} clusters covering {} stories ({folded} would be folded away)",
                groups.len(),
                folded + groups.len()
            );
            for (root, members) in groups.iter().take(if apply { 0 } else { 12 }) {
                println!("\n  KEEP  {}", stories[*root].1);
                for m in members.iter().filter(|m| *m != root) {
                    println!("  fold  {}", stories[*m].1);
                }
            }

            if !apply {
                println!("\ndry run — nothing changed. Re-run with --apply to fold them.");
                return Ok(());
            }
            let mut moved = 0u64;
            for (root, members) in &groups {
                for m in members.iter().filter(|m| *m != root) {
                    moved +=
                        bg_db::stories::merge_into(&db, stories[*m].0, stories[*root].0).await?;
                }
            }
            println!(
                "folded {folded} stories into {}, moving {moved} items",
                groups.len()
            );
            println!("now run `bg run --once` so the Sentinel re-verifies the widened claims");
        }
        Cmd::Source { slug, wake, rest } => {
            let db = Db::connect(&url).await?;
            if !wake && !rest {
                println!("say which: --wake or --rest");
                return Ok(());
            }
            bg_db::sources::set_enabled(&db, &slug, wake).await?;
            println!("{slug} is now {}", if wake { "polling" } else { "rested" });
        }
        Cmd::Steward { apply } => {
            let ctx = context(&url, None).await?;
            let findings = bg_agents::steward::run(&ctx, apply).await?;
            if findings.is_empty() {
                println!("nothing to report — the newsroom is healthy");
                return Ok(());
            }
            for f in findings.iter().filter(|f| !f.needs_a_human()) {
                println!(
                    "  fixed   [{}] {} — {}",
                    f.kind,
                    f.detail,
                    f.action.as_deref().unwrap_or("")
                );
            }
            for f in findings.iter().filter(|f| f.needs_a_human()) {
                println!("  needs a human  [{}] {}", f.kind, f.detail);
            }
            if !apply && findings.iter().any(|f| !f.needs_a_human()) {
                println!("\n(read-only; re-run with --apply to make the safe fixes)");
            }
        }
        Cmd::Retract { dry_run } => {
            let db = Db::connect(&url).await?;
            // Same principle as retracting content from a source that
            // disallowed us: a posture we failed to notice earlier applies to
            // what we already hold, not only to what we fetch next.
            match bg_db::items::declined_text_held(&db).await {
                Ok(0) => {}
                Ok(n) if dry_run => {
                    println!("would erase stored text from {n} items whose publisher declines model input")
                }
                Ok(n) => {
                    let done = bg_db::items::purge_declined_text(&db).await?;
                    println!("erased stored text from {done} of {n} items (publisher declines model input)");
                }
                Err(e) => eprintln!("could not check declined text: {e}"),
            }
            if dry_run {
                let (waiting, _) = bg_db::items::queue_health(&db).await.unwrap_or((0, 0));
                println!("dry run — nothing will be changed (queue: {waiting} waiting)");
                for (slug, n) in bg_db::analyses::incoherent_stories(&db).await? {
                    println!("  would retract (incoherent, {n} items): /story/{slug}");
                }
                println!("  run without --dry-run to apply");
                return Ok(());
            }
            let text = bg_db::items::purge_disallowed_text(&db).await?;
            let robots = bg_db::stories::retract_disallowed(&db).await?;
            let blobs = bg_db::stories::retract_incoherent(&db, 10).await?;
            println!("purged working text from {text} item(s) of disallowed sources");
            println!("retracted {robots} story(ies) sourced only from disallowed feeds");
            println!("retracted {blobs} story(ies) that merged unrelated events");
        }

        Cmd::Enrich { limit, delay } => {
            let ctx = context(&url, None).await?;
            let targets = bg_db::items::needing_extraction(&ctx.db, limit).await?;
            println!("{} item(s) to fetch", targets.len());
            let (mut got, mut empty, mut failed) = (0usize, 0usize, 0usize);
            for (id, url_str) in &targets {
                match bg_ingest::readable::fetch(
                    &ctx.http,
                    &ctx.cfg.user_agent,
                    url_str,
                    // Same switch the feed poller reads, defaulting on: a
                    // fetch that skips robots because a config field was
                    // missing is the kind of violation nobody notices.
                    std::env::var("BG_RESPECT_ROBOTS")
                        .map(|v| v != "false")
                        .unwrap_or(true),
                )
                .await
                {
                    Ok(Some(ex)) => {
                        let n = ex.text.chars().count();
                        bg_db::items::record_extraction(&ctx.db, *id, Some(&ex.text), ex.via)
                            .await?;
                        got += 1;
                        println!("  {n:>6} chars  via {:<28} {url_str}", ex.via);
                    }
                    Ok(None) => {
                        // A paywall or a video page is a permanent answer.
                        // Recording it stops us asking again every run.
                        bg_db::items::record_extraction(&ctx.db, *id, None, "none").await?;
                        empty += 1;
                        println!("       -  no article       {url_str}");
                    }
                    Err(e) => {
                        // Leave `extracted_at` NULL so a transient network
                        // failure is retried, unlike a page that had nothing —
                        // but count the attempt, so a host that refuses us
                        // every time stops heading the queue forever.
                        bg_db::items::record_extract_failure(&ctx.db, *id).await?;
                        failed += 1;
                        println!("       !  {e}");
                    }
                }
                if delay > 0 {
                    tokio::time::sleep(std::time::Duration::from_secs(delay)).await;
                }
            }
            println!("{got} extracted, {empty} with no article, {failed} failed");
        }

        Cmd::Analyze { limit, redo } => {
            let ctx = context(&url, None).await?;
            if redo {
                let n = bg_db::analyses::clear(&ctx.db).await?;
                println!("cleared {n} existing analyses");
            }
            let stories = bg_db::analyses::needing_analysis(
                &ctx.db,
                bg_agents::skein::MIN_GROUNDING_CHARS as i64,
                limit,
            )
            .await?;
            println!(
                "{} story(ies) with enough source text to analyse",
                stories.len()
            );
            let (mut done, mut held, mut failed) = (0usize, 0usize, 0usize);
            for id in &stories {
                match bg_agents::skein::run(&ctx, *id).await {
                    Ok(true) => {
                        done += 1;
                        println!("  ok   {id}");
                    }
                    Ok(false) => {
                        held += 1;
                        println!("  thin {id}");
                    }
                    Err(e) => {
                        failed += 1;
                        println!("  FAIL {id} — {e}");
                    }
                }
            }
            println!("{done} analysed, {held} too thin, {failed} failed");
        }
    }

    Ok(())
}

/// How often the newsroom looks after itself.
///
/// Six hours. Its checks are measured in days — a desk silent for two, a source
/// barren for three — so a tighter loop would fill the log without noticing
/// anything sooner.
const STEWARD_EVERY: std::time::Duration = std::time::Duration::from_secs(6 * 3600);

/// Longest a round may take before it is abandoned.
///
/// Its network-touching checks are bounded per item, not in total, and on a bad
/// link the total is what matters. Five minutes is longer than a healthy round
/// needs and short enough that a stuck one cannot overlap the next.
const STEWARD_MAX_ROUND: std::time::Duration = std::time::Duration::from_secs(300);

async fn context(url: &str, provider_override: Option<String>) -> Result<Ctx> {
    if let Some(p) = provider_override {
        // SAFETY: single-threaded startup, before any task is spawned.
        unsafe { std::env::set_var("BG_LLM_PROVIDER", p) };
    }
    let db = Db::connect(url).await?;
    let llm = Llm::from_env();
    // Resumed, not new: the pacer's daily ledger is in memory, and a restart
    // that forgets the day's spend walks straight back into a quota that is
    // already gone.
    Ok(Ctx::resumed(db, llm, FlockConfig::from_env()).await?)
}

async fn doctor(url: &str) -> Result<()> {
    println!("BitGoose doctor\n");

    // -- database -----------------------------------------------------------
    let db = match Db::connect(url).await {
        Ok(db) => {
            println!("  [ok]   database connected");
            db
        }
        Err(e) => {
            println!("  [FAIL] database: {e}");
            println!("\n  Start it with: docker compose up -d");
            return Ok(());
        }
    };
    match db.server_version().await {
        Ok(v) => println!("  [ok]   postgres {v}"),
        Err(e) => println!("  [warn] server version: {e}"),
    }
    match db.pgvector_version().await {
        Ok(Some(v)) => println!("  [ok]   pgvector {v}"),
        Ok(None) => println!("  [FAIL] pgvector extension is not installed"),
        Err(e) => println!("  [warn] pgvector: {e}"),
    }

    let applied: Result<i64, _> = sqlx::query_scalar("SELECT count(*) FROM _sqlx_migrations")
        .fetch_one(&db.pool)
        .await;
    match applied {
        Ok(n) => println!("  [ok]   {n} migration(s) applied"),
        Err(_) => println!("  [FAIL] no migrations applied — run: bg migrate"),
    }

    // -- row counts ---------------------------------------------------------
    println!("\n  tables:");
    for (t, n) in db.counts().await.unwrap_or_default() {
        println!("    {t:<20} {n:>8}");
    }

    // -- sources ------------------------------------------------------------
    println!("\n  sources:");
    let health = bg_db::sources::health(&db).await.unwrap_or_default();
    if health.is_empty() {
        println!("    none — run: bg seed");
    }
    for s in &health {
        let mark = match (&s.last_error, s.enabled, s.robots_ok) {
            (_, false, _) => "[off ]",
            (_, _, false) => "[robo]",
            (Some(_), _, _) => "[FAIL]",
            (None, _, _) => "[ok  ]",
        };
        println!(
            "    {mark} {:<18} {:>5} items   {}",
            s.slug,
            s.items,
            s.last_error
                .as_deref()
                .map(|e| bg_core::text::truncate_words(e, 12))
                .unwrap_or_default()
        );
    }

    // -- agents -------------------------------------------------------------
    let agents = bg_db::agents::all(&db).await.unwrap_or_default();
    // Against the enum. This is the third place a literal ten had to be
    // corrected after the Flock gained a member; the count belongs in one place
    // and `AgentRole::ALL` is it.
    let expected = bg_core::domain::AgentRole::ALL.len();
    if agents.len() == expected {
        println!("\n  [ok]   flock roster complete ({expected} agents)");
    } else {
        println!(
            "\n  [FAIL] roster has {} of {expected} agents — run: bg seed",
            agents.len()
        );
    }

    // -- LLM ----------------------------------------------------------------
    println!("\n  llm:");
    let llm = Llm::from_env();
    println!("    chain: {}", llm.provider_names().join(" -> "));
    match llm.primary().health().await {
        Ok(()) => println!("    [ok]   {} reachable", llm.primary().name()),
        Err(e) => println!("    [warn] {}: {e}", llm.primary().name()),
    }
    for tier in [
        bg_core::domain::ModelTier::Fast,
        bg_core::domain::ModelTier::Mid,
        bg_core::domain::ModelTier::Top,
    ] {
        let s = llm.primary().spec(tier);
        println!(
            "    {:<5} {:<22} ${:.2}/${:.2} per Mtok",
            tier.as_str(),
            s.id,
            s.input_per_mtok,
            s.output_per_mtok
        );
    }

    // -- spend --------------------------------------------------------------
    if let Ok(t) = bg_db::agents::newsroom_totals(&db).await {
        println!(
            "\n  last 24h: {} runs, {} failures, ${:.4}, {} stories published, {} claims",
            t.runs_24h, t.failures_24h, t.cost_24h, t.stories_published_24h, t.claims_24h
        );
    }
    if let Ok(n) = bg_db::violations::count_blocks_24h(&db).await {
        println!("  policy blocks in last 24h: {n}");
    }

    // The queue is the health signal that matters on a constrained tier: if
    // `waiting` climbs pass after pass, intake is outrunning the budget and
    // either the news horizon or the source list needs attention.
    if let Ok((waiting, lapsed)) = bg_db::items::queue_health(&db).await {
        println!("  triage queue: {waiting} waiting, {lapsed} lapsed past the news horizon");
    }

    // Text held against a publisher's stated wishes. Should always be zero;
    // if it is not, either a source changed its posture or a gate has a hole.
    if let Ok(n) = bg_db::items::declined_text_held(&db).await {
        if n > 0 {
            println!(
                "!! holding extracted text from {n} items whose publisher declines model input"
            );
            println!("     run `bg retract` to erase it");
        }
    }

    // Corroboration is the product. A ratio this visible is the difference
    // between noticing that clustering has stopped working and finding out
    // weeks later from the front page.
    if let Ok((alone, total)) = bg_db::stories::corroboration_health(&db, 14).await {
        if total > 0 {
            let pct = (alone as f64 / total as f64) * 100.0;
            let mark = if pct > 90.0 {
                "!!"
            } else if pct > 70.0 {
                " ?"
            } else {
                " ok"
            };
            println!(
                "{mark} corroboration: {alone} of {total} published stories in 14d have a single source ({pct:.0}%)"
            );
            if pct > 90.0 {
                println!("     clustering is not merging; try `bg recluster --hours 336`");
            }
        }
    }

    // Loud, because nothing else surfaces it: these stories render as one
    // event on the site and are not one. They are excluded from analysis but
    // still readable, so silence here would mean nobody ever finds them.
    match bg_db::analyses::incoherent_stories(&db).await {
        Ok(bad) if !bad.is_empty() => {
            println!(
                "\n  ! {} story(ies) merge too many items to be one event:",
                bad.len()
            );
            for (slug, n) in bad.iter().take(10) {
                println!("    {n:>3} items  /story/{slug}");
            }
            println!("    (stub-era clustering; re-cluster or kill them)");
        }
        _ => {}
    }

    Ok(())
}

async fn stats(url: &str) -> Result<()> {
    let db = Db::connect(url).await?;
    let t = bg_db::agents::newsroom_totals(&db).await?;

    println!("BitGoose — last 24 hours\n");
    println!("  agent runs        {:>8}", t.runs_24h);
    println!("  failures          {:>8}", t.failures_24h);
    println!("  tokens            {:>8}", t.tokens_24h);
    println!("  cost              {:>8}", format!("${:.4}", t.cost_24h));
    println!("  stories published {:>8}", t.stories_published_24h);
    println!("  claims extracted  {:>8}", t.claims_24h);

    println!("\n  the flock:");
    println!(
        "    {:<10} {:>5} {:>5} {:>6} {:>10} {:>9}",
        "agent", "runs", "fail", "tokens", "cost", "latency"
    );
    for s in bg_db::agents::flock_stats(&db).await? {
        println!(
            "    {:<10} {:>5} {:>5} {:>6} {:>10} {:>8}ms",
            s.role.display_name(),
            s.runs_24h,
            s.failed_24h,
            s.tokens_24h,
            format!("${:.4}", s.cost_24h_usd),
            s.avg_latency_ms
        );
    }

    // -- content health -------------------------------------------------------
    let extraction = bg_db::items::extraction_stats(&db)
        .await
        .unwrap_or_default();
    if !extraction.is_empty() {
        println!("\n  article extraction:");
        for (via, n) in extraction.iter().take(8) {
            println!("    {n:>6}  {via}");
        }
    }

    println!(
        "\n  analyses: {}",
        bg_db::analyses::count(&db).await.unwrap_or(0)
    );

    // The queue is the newsroom's real health signal on a constrained tier: if
    // `waiting` keeps climbing pass after pass, intake is outrunning what the
    // budget can process and the horizon or the source list needs attention.
    if let Ok((waiting, lapsed)) = bg_db::items::queue_health(&db).await {
        println!("  triage queue: {waiting} waiting, {lapsed} lapsed past the news horizon");
    }

    println!("\n  recent stories:");
    for st in bg_db::stories::published(&db, None, 10, 0).await? {
        println!(
            "    [{:>3}] {:<6} {:<10} {}",
            st.newsworthiness,
            st.kind.as_str(),
            st.category.as_str(),
            bg_core::text::truncate_words(&st.title, 10)
        );
    }
    Ok(())
}
