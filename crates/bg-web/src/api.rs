//! Server functions.
//!
//! Each is a normal Rust `async fn` on the server and an RPC call from the
//! hydrated client. The database work sits behind `#[cfg(feature = "ssr")]`, so
//! `bg-db` and its native dependencies are never compiled into the WASM bundle.

use crate::model::*;
use leptos::prelude::*;

#[cfg(feature = "ssr")]
pub use server_impl::db;

#[cfg(feature = "ssr")]
mod server_impl {
    use bg_db::Db;
    use std::sync::OnceLock;

    static DB: OnceLock<Db> = OnceLock::new();

    /// Install the pool once at startup.
    pub fn set_db(db: Db) {
        let _ = DB.set(db);
    }

    /// The pool, for server functions.
    ///
    /// A process-wide handle rather than Leptos context: the pool is genuinely
    /// global, cheap to clone, and threading it through every render would add
    /// plumbing without adding safety.
    pub fn db() -> &'static Db {
        DB.get().expect("database pool not initialised — call set_db() at startup")
    }
}

#[cfg(feature = "ssr")]
pub use server_impl::set_db;

#[cfg(feature = "ssr")]
fn e(err: impl std::fmt::Display) -> ServerFnError {
    tracing::error!(error = %err, "server function failed");
    ServerFnError::new(err.to_string())
}

#[cfg(feature = "ssr")]
fn card(s: &bg_core::domain::Story, lead: Option<(&str, &str)>) -> StoryCard {
    StoryCard {
        slug: s.slug.clone(),
        kind: s.kind.as_str().into(),
        title: s.title.clone(),
        dek: s.summary.clone().unwrap_or_default(),
        category: s.category.as_str().into(),
        category_label: s.category.label().into(),
        source_count: s.source_count,
        newsworthiness: s.newsworthiness,
        published_at: s.published_at.map(|d| d.to_rfc3339()).unwrap_or_default(),
        ago: s.published_at.map(ago).unwrap_or_default(),
        assets: s.assets.clone(),
        lead_source: lead.map(|(n, _)| n.to_string()).unwrap_or_default(),
        lead_url: lead.map(|(_, u)| u.to_string()).unwrap_or_default(),
    }
}

/// Render the article body, resolving `[^cN]` markers into ledger links.
///
/// Done server-side so the citation links exist in the initial HTML — they are
/// the primary navigation of a story page, and a reader with JavaScript off
/// should still be able to walk from a sentence to its evidence.
#[cfg(feature = "ssr")]
fn render_body(md: &str) -> String {
    use pulldown_cmark::{html, Options, Parser};
    let mut opts = Options::empty();
    opts.insert(Options::ENABLE_STRIKETHROUGH);
    opts.insert(Options::ENABLE_TABLES);

    let mut out = String::new();
    html::push_html(&mut out, Parser::new_ext(md, opts));

    // Markdown leaves `[^c1]` untouched; turn each into an anchor.
    let mut result = String::with_capacity(out.len());
    let chars: Vec<char> = out.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if i + 3 < chars.len() && chars[i] == '[' && chars[i + 1] == '^' {
            let mut j = i + 2;
            let mut tok = String::new();
            while j < chars.len() && chars[j] != ']' && tok.len() < 8 {
                tok.push(chars[j]);
                j += 1;
            }
            if j < chars.len() && chars[j] == ']' && !tok.is_empty()
                && tok.chars().all(|c| c.is_ascii_alphanumeric())
            {
                result.push_str(&format!(
                    "<a class=\"cite\" href=\"#claim-{tok}\" title=\"Jump to the evidence for this statement\">{tok}</a>"
                ));
                i = j + 1;
                continue;
            }
        }
        result.push(chars[i]);
        i += 1;
    }
    result
}

// ---------------------------------------------------------------------------
// front page
// ---------------------------------------------------------------------------

#[server(name = GetFrontPage, prefix = "/rpc")]
pub async fn get_front_page() -> Result<FrontPage, ServerFnError> {
    let db = db();
    let ranked = bg_db::stories::front_page(db, 40).await.map_err(e)?;

    let mut lead = None;
    let mut desk = Vec::new();
    for s in ranked.iter().filter(|s| s.kind == bg_core::domain::StoryKind::Desk) {
        let c = card(s, None);
        if lead.is_none() {
            lead = Some(c);
        } else if desk.len() < 8 {
            desk.push(c);
        }
    }

    let wire_entries = bg_db::stories::wire(db, 14, 0).await.map_err(e)?;
    let wire: Vec<StoryCard> = wire_entries
        .iter()
        .filter(|w| Some(&w.slug) != lead.as_ref().map(|l| &l.slug))
        .map(|w| StoryCard {
            slug: w.slug.clone(),
            kind: "wire".into(),
            title: w.title.clone(),
            dek: w.summary.clone(),
            category: w.category.as_str().into(),
            category_label: w.category.label().into(),
            source_count: w.source_count,
            newsworthiness: w.newsworthiness,
            published_at: w.published_at.to_rfc3339(),
            ago: ago(w.published_at),
            assets: w.assets.clone(),
            lead_source: w.source_name.clone(),
            lead_url: w.source_url.clone(),
        })
        .collect();

    let prices = bg_db::prices::latest_all(db)
        .await
        .map_err(e)?
        .iter()
        .take(14)
        .map(|t| Tick {
            symbol: t.symbol.clone(),
            price: fmt_price(t.price_usd.to_string().parse().unwrap_or(0.0)),
            change: t.change_24h_pct,
        })
        .collect();

    // The Honk bar is for genuinely breaking news, so it is gated on both
    // score and recency. A stale banner that never changes is worse than none.
    let honk = ranked
        .iter()
        .find(|s| {
            s.newsworthiness >= 80
                && s.published_at
                    .is_some_and(|p| (chrono::Utc::now() - p).num_hours() < 6)
        })
        .map(|s| card(s, None));

    Ok(FrontPage { lead, desk, wire, prices, honk })
}

// ---------------------------------------------------------------------------
// listings
// ---------------------------------------------------------------------------

#[server(name = GetStories, prefix = "/rpc")]
pub async fn get_stories(kind: String, limit: i64) -> Result<Vec<StoryCard>, ServerFnError> {
    use std::str::FromStr;
    let db = db();
    let k = bg_core::domain::StoryKind::from_str(&kind).ok();
    if k == Some(bg_core::domain::StoryKind::Wire) {
        let entries = bg_db::stories::wire(db, limit, 0).await.map_err(e)?;
        return Ok(entries
            .iter()
            .map(|w| StoryCard {
                slug: w.slug.clone(),
                kind: "wire".into(),
                title: w.title.clone(),
                dek: w.summary.clone(),
                category: w.category.as_str().into(),
                category_label: w.category.label().into(),
                source_count: w.source_count,
                newsworthiness: w.newsworthiness,
                published_at: w.published_at.to_rfc3339(),
                ago: ago(w.published_at),
                assets: w.assets.clone(),
                lead_source: w.source_name.clone(),
                lead_url: w.source_url.clone(),
            })
            .collect());
    }
    let stories = bg_db::stories::published(db, k, limit, 0).await.map_err(e)?;
    Ok(stories.iter().map(|s| card(s, None)).collect())
}

// ---------------------------------------------------------------------------
// story page
// ---------------------------------------------------------------------------

#[server(name = GetStory, prefix = "/rpc")]
pub async fn get_story(slug: String) -> Result<Option<StoryPage>, ServerFnError> {
    let db = db();
    let story = match bg_db::stories::by_slug(db, &slug).await {
        Ok(s) => s,
        Err(bg_db::DbError::NotFound(_)) => return Ok(None),
        Err(err) => return Err(e(err)),
    };

    let article = bg_db::articles::latest_for_story(db, story.id).await.map_err(e)?;
    let claims = bg_db::claims::with_sources(db, story.id).await.map_err(e)?;
    let refs = bg_db::stories::source_refs(db, story.id).await.map_err(e)?;
    let corrections = bg_db::articles::corrections_for_story(db, story.id).await.map_err(e)?;
    let runs = bg_db::agents::runs_for_story(db, story.id).await.map_err(e)?;

    let claim_cards = claims
        .iter()
        .enumerate()
        .map(|(i, c)| {
            let (supporting, disputing): (Vec<_>, Vec<_>) = c
                .sources
                .iter()
                .partition(|s| s.stance != bg_core::domain::Stance::Contradicts);
            ClaimCard {
                marker: format!("c{}", i + 1),
                text: c.claim.text.clone(),
                kind: c.claim.kind.as_str().into(),
                verification: c.claim.verification.as_str().into(),
                verification_label: c.claim.verification.label().into(),
                confidence: c.claim.confidence,
                excerpt: c.sources.iter().find_map(|s| s.excerpt.clone()),
                sources: supporting
                    .iter()
                    .map(|s| SourceCard {
                        name: s.source_name.clone(),
                        slug: s.source_slug.clone(),
                        url: s.url.clone(),
                        title: s.title.clone(),
                        trust: s.source_trust,
                    })
                    .collect(),
                disputed_by: disputing
                    .iter()
                    .map(|s| SourceCard {
                        name: s.source_name.clone(),
                        slug: s.source_slug.clone(),
                        url: s.url.clone(),
                        title: s.title.clone(),
                        trust: s.source_trust,
                    })
                    .collect(),
            }
        })
        .collect();

    let published = story.published_at.unwrap_or(story.first_seen_at);
    Ok(Some(StoryPage {
        slug: story.slug.clone(),
        headline: article.as_ref().map(|a| a.headline.clone()).unwrap_or(story.title.clone()),
        dek: article
            .as_ref()
            .map(|a| a.dek.clone())
            .filter(|d| !d.is_empty())
            .or(story.summary.clone())
            .unwrap_or_default(),
        body_html: article.as_ref().map(|a| render_body(&a.body_md)).unwrap_or_default(),
        category_label: story.category.label().into(),
        published_at: published.format("%B %-d, %Y at %H:%M UTC").to_string(),
        ago: ago(published),
        reading_time_min: article.as_ref().map(|a| (a.reading_time_s / 60).max(1)).unwrap_or(1),
        kind: story.kind.as_str().into(),
        claims: claim_cards,
        sources: refs
            .iter()
            .map(|r| SourceCard {
                name: r.name.clone(),
                slug: r.slug.clone(),
                url: r.url.clone(),
                title: r.title.clone(),
                trust: r.trust,
            })
            .collect(),
        corrections: corrections
            .iter()
            .map(|c| CorrectionCard {
                reason: c.reason.clone(),
                issued_at: c.issued_at.format("%b %-d, %Y").to_string(),
                from_version: c.from_version,
                to_version: c.to_version,
            })
            .collect(),
        runs: runs.iter().map(run_line).collect(),
        assets: story.assets.clone(),
    }))
}

#[cfg(feature = "ssr")]
fn run_line(r: &bg_core::domain::AgentRunSummary) -> RunLine {
    RunLine {
        role: r.role.as_str().into(),
        role_name: r.role.display_name().into(),
        status: r.status.as_str().into(),
        model: r.model.clone(),
        cost: format!("${:.4}", r.cost_usd),
        latency_ms: r.latency_ms,
        at: ago(r.started_at),
        note: r.note.clone(),
    }
}

// ---------------------------------------------------------------------------
// the flock
// ---------------------------------------------------------------------------

#[server(name = GetFlock, prefix = "/rpc")]
pub async fn get_flock() -> Result<FlockPage, ServerFnError> {
    let db = db();
    let stats = bg_db::agents::flock_stats(db).await.map_err(e)?;
    let totals = bg_db::agents::newsroom_totals(db).await.map_err(e)?;
    let recent = bg_db::agents::recent_runs(db, 30).await.map_err(e)?;
    let blocks = bg_db::violations::count_blocks_24h(db).await.unwrap_or(0);

    Ok(FlockPage {
        agents: stats
            .iter()
            .map(|a| AgentCard {
                role: a.role.as_str().into(),
                name: a.name.clone(),
                beat: a.role.beat().into(),
                tier: a.role.tier().as_str().into(),
                runs_24h: a.runs_24h,
                failed_24h: a.failed_24h,
                tokens_24h: a.tokens_24h,
                cost_24h: format!("${:.4}", a.cost_24h_usd),
                avg_latency_ms: a.avg_latency_ms,
                last_note: a.last_note.clone(),
                enabled: a.enabled,
            })
            .collect(),
        recent: recent.iter().map(run_line).collect(),
        runs_24h: totals.runs_24h,
        failures_24h: totals.failures_24h,
        tokens_24h: totals.tokens_24h,
        cost_24h: format!("${:.4}", totals.cost_24h),
        published_24h: totals.stories_published_24h,
        claims_24h: totals.claims_24h,
        blocks_24h: blocks,
    })
}

// ---------------------------------------------------------------------------
// markets
// ---------------------------------------------------------------------------

#[server(name = GetPrices, prefix = "/rpc")]
pub async fn get_prices() -> Result<PricesPage, ServerFnError> {
    let db = db();
    let ticks = bg_db::prices::latest_all(db).await.map_err(e)?;
    let assets = bg_db::prices::assets(db).await.unwrap_or_default();

    let mut rows = Vec::new();
    for t in &ticks {
        let name = assets
            .iter()
            .find(|a| a.symbol == t.symbol)
            .map(|a| a.name.clone())
            .unwrap_or_else(|| t.symbol.clone());
        let story_count = bg_db::stories::by_asset(db, &t.symbol, 100).await.map(|v| v.len() as i64).unwrap_or(0);
        rows.push(PriceRow {
            symbol: t.symbol.clone(),
            name,
            price: fmt_price(t.price_usd.to_string().parse().unwrap_or(0.0)),
            change: t.change_24h_pct,
            market_cap: t.market_cap.map(|m| compact_usd(m.to_string().parse().unwrap_or(0.0))),
            volume: t.volume_24h.map(|v| compact_usd(v.to_string().parse().unwrap_or(0.0))),
            story_count,
        });
    }
    Ok(PricesPage { ticks: rows })
}

#[server(name = GetAsset, prefix = "/rpc")]
pub async fn get_asset(ticker: String) -> Result<(Option<PriceRow>, Vec<StoryCard>), ServerFnError> {
    let db = db();
    let stories = bg_db::stories::by_asset(db, &ticker, 40).await.map_err(e)?;
    let tick = bg_db::prices::latest(db, &ticker).await.map_err(e)?;
    let assets = bg_db::prices::assets(db).await.unwrap_or_default();

    let row = tick.map(|t| PriceRow {
        symbol: t.symbol.clone(),
        name: assets
            .iter()
            .find(|a| a.symbol == t.symbol)
            .map(|a| a.name.clone())
            .unwrap_or_else(|| t.symbol.clone()),
        price: fmt_price(t.price_usd.to_string().parse().unwrap_or(0.0)),
        change: t.change_24h_pct,
        market_cap: t.market_cap.map(|m| compact_usd(m.to_string().parse().unwrap_or(0.0))),
        volume: t.volume_24h.map(|v| compact_usd(v.to_string().parse().unwrap_or(0.0))),
        story_count: stories.len() as i64,
    });

    Ok((row, stories.iter().map(|s| card(s, None)).collect()))
}

// ---------------------------------------------------------------------------
// standards & trends
// ---------------------------------------------------------------------------

#[server(name = GetStandards, prefix = "/rpc")]
pub async fn get_standards() -> Result<StandardsPage, ServerFnError> {
    let db = db();
    let health = bg_db::sources::health(db).await.map_err(e)?;
    let all = bg_db::sources::all(db).await.map_err(e)?;

    Ok(StandardsPage {
        sources: health
            .iter()
            .map(|h| {
                let meta = all.iter().find(|s| s.slug == h.slug);
                SourceHealthRow {
                    slug: h.slug.clone(),
                    name: h.name.clone(),
                    homepage: meta.map(|m| m.homepage.clone()).unwrap_or_default(),
                    trust: meta.map(|m| m.trust).unwrap_or(50),
                    items: h.items,
                    enabled: h.enabled,
                    robots_ok: h.robots_ok,
                    healthy: h.last_error.is_none(),
                }
            })
            .collect(),
        enforcement: bg_db::violations::tally(db, 30).await.unwrap_or_default(),
        blocks_24h: bg_db::violations::count_blocks_24h(db).await.unwrap_or(0),
        max_quote_words: bg_core::policy::MAX_QUOTE_WORDS,
        max_verbatim_run: bg_core::policy::MAX_VERBATIM_RUN,
        min_desk_sources: bg_core::policy::MIN_DESK_SOURCES,
    })
}

#[server(name = GetFlyway, prefix = "/rpc")]
pub async fn get_flyway() -> Result<FlywayPage, ServerFnError> {
    use std::collections::BTreeMap;
    use std::str::FromStr;
    let db = db();
    const DAYS: i32 = 14;

    let rows = bg_db::stories::flyway(db, DAYS).await.map_err(e)?;
    let mut by_cat: BTreeMap<String, BTreeMap<chrono::NaiveDate, i64>> = BTreeMap::new();
    for (cat, day, n) in rows {
        *by_cat.entry(cat).or_default().entry(day).or_insert(0) += n;
    }

    // A dense series: days with no coverage must render as gaps, not be
    // dropped, or the chart implies continuous activity that did not happen.
    let today = chrono::Utc::now().date_naive();
    let mut categories: Vec<CategoryTrend> = by_cat
        .into_iter()
        .map(|(cat, days)| {
            let series: Vec<i64> = (0..DAYS)
                .rev()
                .map(|d| {
                    let day = today - chrono::Duration::days(d as i64);
                    days.get(&day).copied().unwrap_or(0)
                })
                .collect();
            let label = bg_core::domain::Category::from_str(&cat)
                .map(|c| c.label().to_string())
                .unwrap_or_else(|_| cat.clone());
            CategoryTrend { category: cat, label, total: series.iter().sum(), series }
        })
        .collect();
    categories.sort_by(|a, b| b.total.cmp(&a.total));

    let entities = bg_db::entities::trending(db, DAYS, 14)
        .await
        .unwrap_or_default()
        .into_iter()
        .map(|(ent, n)| (ent.name, ent.slug, n))
        .collect();

    Ok(FlywayPage { categories, entities, days: DAYS })
}
