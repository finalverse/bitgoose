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
    pub topics_searched: usize,
    pub directions_searched: usize,
}

fn discovery_locale(
    language: bg_core::domain::EditorialLanguage,
) -> (&'static str, &'static str, &'static str, &'static str) {
    use bg_core::domain::EditorialLanguage;
    match language {
        EditorialLanguage::En => ("gnews-en-topics", "en-US", "US", "US:en"),
        EditorialLanguage::Zh => ("gnews-zh-topics", "zh-CN", "CN", "CN:zh-Hans"),
        EditorialLanguage::ZhHant => ("gnews-zh-hant-topics", "zh-TW", "TW", "TW:zh-Hant"),
        EditorialLanguage::Fr => ("gnews-fr-topics", "fr", "FR", "FR:fr"),
        EditorialLanguage::Es => ("gnews-es-topics", "es", "ES", "ES:es"),
        EditorialLanguage::Ja => ("gnews-ja-topics", "ja", "JP", "JP:ja"),
        EditorialLanguage::Ko => ("gnews-ko-topics", "ko", "KR", "KR:ko"),
    }
}

fn topic_search_query(anchor_terms: &[String], keywords: &[String]) -> Option<String> {
    let normalize = |terms: &[String], limit: usize| {
        let mut terms: Vec<String> = terms
            .iter()
            .take(limit)
            .map(|term| term.trim().to_string())
            .filter(|term| !term.is_empty())
            .collect();
        terms.sort();
        terms.dedup();
        terms
    };
    let anchors = normalize(anchor_terms, 4);
    let signals = normalize(keywords, 5);
    match (anchors.is_empty(), signals.is_empty()) {
        (true, true) => None,
        (false, true) => Some(format!("({}) when:2d", anchors.join(" OR "))),
        (true, false) => Some(format!("({}) when:2d", signals.join(" OR "))),
        (false, false) => Some(format!(
            "({}) ({}) when:2d",
            anchors.join(" OR "),
            signals.join(" OR ")
        )),
    }
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
                // The page's own share image, where the feed carried none. Free
                // — the page is already open for the text — and it is the
                // difference between a story that looks like a news article and
                // one that looks like a link.
                if let Some(img) = ex.image.as_deref().filter(|u| !u.is_empty()) {
                    let _ = bg_db::items::record_page_image(&ctx.db, *id, img).await;
                }
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

        // A permanent topic is a reporting assignment. Search its own local
        // news ecosystem on every cadence instead of hoping a general feed
        // happens to repeat the exact dossier title.
        let due = bg_db::gaggles::searches_due(&ctx.db, 20, 8)
            .await
            .unwrap_or_default();
        for topic in due {
            let (source_slug, hl, gl, ceid) = discovery_locale(topic.editorial_language);
            if let Ok(base) = bg_db::sources::by_slug(&ctx.db, source_slug).await {
                let Some(query) = topic_search_query(&topic.anchor_terms, &topic.keywords) else {
                    let _ = bg_db::gaggles::mark_searched(&ctx.db, topic.id).await;
                    continue;
                };
                let mut search = base.clone();
                search.beat = Some(if topic.slug.contains("bitcoin")
                    || topic.slug.contains("crypto")
                    || topic.slug.contains("stablecoin")
                {
                    bg_core::domain::Beat::Crypto
                } else if topic.slug.contains("frontier-ai") {
                    bg_core::domain::Beat::Ai
                } else {
                    bg_core::domain::Beat::Tech
                });
                if let Ok(mut url) = reqwest::Url::parse("https://news.google.com/rss/search") {
                    url.query_pairs_mut()
                        .append_pair("q", &query)
                        .append_pair("hl", hl)
                        .append_pair("gl", gl)
                        .append_pair("ceid", ceid);
                    search.url = url.to_string();
                    search.etag = None;
                    search.last_modified = None;
                    let report = bg_ingest::feeds::poll_source(&ctx.db, &ctx.http, &search).await;
                    r.items_new += report.inserted;
                    if report.error.is_some() {
                        r.sources_failed += 1;
                    } else {
                        r.sources_polled += 1;
                    }
                    r.topics_searched += 1;
                    info!(topic = %topic.title, language = %topic.editorial_language, inserted = report.inserted, "topic discovery swept");
                }
            }
            let _ = bg_db::gaggles::mark_searched(&ctx.db, topic.id).await;
        }

        // Human editors may increase discovery attention, but cannot insert a
        // story or bypass the independent triage, verification and publishing
        // agents. Directions therefore enter at Scout, the safest boundary.
        let directions = bg_db::directions::searches_due(&ctx.db, 20, 4)
            .await
            .unwrap_or_default();
        for direction in directions {
            let (source_slug, hl, gl, ceid) = discovery_locale(direction.editorial_language);
            if let Ok(base) = bg_db::sources::by_slug(&ctx.db, source_slug).await {
                let Some(query) =
                    topic_search_query(&direction.anchor_terms, &direction.keywords)
                else {
                    let _ = bg_db::directions::mark_searched(&ctx.db, direction.id).await;
                    continue;
                };
                let mut search = base.clone();
                search.beat = Some(direction.beat);
                if let Ok(mut url) = reqwest::Url::parse("https://news.google.com/rss/search") {
                    url.query_pairs_mut()
                        .append_pair("q", &query)
                        .append_pair("hl", hl)
                        .append_pair("gl", gl)
                        .append_pair("ceid", ceid);
                    search.url = url.to_string();
                    search.etag = None;
                    search.last_modified = None;
                    let report = bg_ingest::feeds::poll_source(&ctx.db, &ctx.http, &search).await;
                    r.items_new += report.inserted;
                    if report.error.is_some() {
                        r.sources_failed += 1;
                    } else {
                        r.sources_polled += 1;
                    }
                    r.directions_searched += 1;
                    info!(direction = %direction.title, language = %direction.editorial_language, inserted = report.inserted, "human editorial direction swept");
                }
            }
            let _ = bg_db::directions::mark_searched(&ctx.db, direction.id).await;
        }

        info!(
            polled = r.sources_polled,
            failed = r.sources_failed,
            new = r.items_new,
            not_modified = r.not_modified,
            topics = r.topics_searched,
            directions = r.directions_searched,
            "scout swept"
        );
        let note = format!(
            "{} new items from {} sources; {} topic and {} editor-directed searches ({} unchanged, {} failed)",
            r.items_new,
            r.sources_polled,
            r.topics_searched,
            r.directions_searched,
            r.not_modified,
            r.sources_failed
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
        let crypto = bg_ingest::market::refresh(&ctx.db, &ctx.http).await;
        // Equities in the same stage, from a different vendor. A crypto-only
        // ticker tells a reader the newsroom covers crypto; BitGoose also runs
        // Markets, Business and AI desks, and on most days Nasdaq and Nvidia
        // are as much the story as Bitcoin is.
        let equities = bg_ingest::market::refresh_equities(&ctx.db, &ctx.http).await;
        let n = crypto + equities;
        Ok(StageOutput::plain(
            n,
            format!("{crypto} crypto, {equities} equities"),
        ))
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::topic_search_query;

    #[test]
    fn topic_search_requires_an_anchor_and_a_signal() {
        let query = topic_search_query(
            &["Bitcoin".into(), "比特币".into()],
            &["regulation".into(), "监管".into()],
        )
        .unwrap();
        assert_eq!(query, "(Bitcoin OR 比特币) (regulation OR 监管) when:2d");
    }

    #[test]
    fn topic_search_uses_available_group_when_one_is_missing() {
        let query = topic_search_query(&["  ".into()], &["headline".into()]).unwrap();
        assert_eq!(query, "(headline) when:2d");
    }
}
