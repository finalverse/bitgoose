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
        /// Seconds between passes.
        #[arg(long, default_value_t = 300)]
        interval: u64,
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

        Cmd::Worker { interval } => {
            let ctx = context(&url, None).await?;
            println!("worker started, {interval}s interval — ctrl-c to stop");
            loop {
                match runner::run_once(&ctx, &runner::RunOpts::default()).await {
                    Ok(r) => println!("[{}] {}", chrono::Utc::now().to_rfc3339(), r.summary()),
                    Err(e) => eprintln!("[{}] pass failed: {e}", chrono::Utc::now().to_rfc3339()),
                }
                tokio::time::sleep(Duration::from_secs(interval)).await;
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
    }

    Ok(())
}

async fn context(url: &str, provider_override: Option<String>) -> Result<Ctx> {
    if let Some(p) = provider_override {
        // SAFETY: single-threaded startup, before any task is spawned.
        unsafe { std::env::set_var("BG_LLM_PROVIDER", p) };
    }
    let db = Db::connect(url).await?;
    let llm = Llm::from_env();
    Ok(Ctx::new(db, llm, FlockConfig::from_env())?)
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
    if agents.len() == 10 {
        println!("\n  [ok]   flock roster complete (10 agents)");
    } else {
        println!(
            "\n  [FAIL] roster has {} of 10 agents — run: bg seed",
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
