//! RSS / Atom ingestion.

use crate::canonical::{sha256_hex, url_hash};
use crate::http::{conditional_get, Fetched};
use crate::{IngestError, Result};
use bg_core::domain::Source;
use bg_core::text::{simhash64, strip_html};
use bg_db::{items::NewItem, sources, Db};
use chrono::{DateTime, Duration as ChronoDuration, Utc};
use tracing::{debug, info, warn};

/// How far back an item may be dated and still be ingested.
///
/// Two failure modes this guards against, both seen in the live roster: a feed
/// that has simply stopped being updated (blockworks and dlnews were both months
/// stale when this was written), and a feed regenerating with epoch-zero or
/// otherwise broken timestamps. Either would put months-old items at the top of
/// a front page ranked by recency. Items dated in the *future* are a publisher
/// clock error, not tomorrow's news, and are rejected too.
const FRESHNESS_DAYS: i64 = 30;

/// What one poll of one source produced.
#[derive(Debug, Default, Clone)]
pub struct PollReport {
    pub source_slug: String,
    pub fetched: usize,
    pub inserted: usize,
    pub duplicates: usize,
    /// Dropped before insertion: no link, no title, or an implausible date.
    pub skipped: usize,
    /// Dropped specifically for being outside the freshness window.
    pub stale: usize,
    pub not_modified: bool,
    pub error: Option<String>,
}

impl PollReport {
    /// True when the feed parsed fine but everything in it was too old.
    ///
    /// Worth distinguishing from a clean empty poll: it means the publisher has
    /// stopped updating (or moved the feed) and the source is quietly
    /// contributing nothing. Silent zero-yield is how a dead source stays in the
    /// roster for months.
    pub fn is_stale_feed(&self) -> bool {
        self.error.is_none() && self.fetched > 0 && self.inserted == 0 && self.stale == self.fetched
    }
}

/// Poll one source and persist anything new.
///
/// Errors are captured into the report rather than propagated: one dead feed
/// must not abort a sweep across the other eight.
pub async fn poll_source(db: &Db, client: &reqwest::Client, src: &Source) -> PollReport {
    let mut rep = PollReport { source_slug: src.slug.clone(), ..Default::default() };

    match poll_inner(db, client, src, &mut rep).await {
        Ok(()) if rep.is_stale_feed() => {
            // Not an error — the fetch and the parse both worked — but the
            // source is contributing nothing and an operator needs to see that.
            let note = format!(
                "feed parsed but all {} entries are outside the {}-day freshness window; \
                 publisher may have stopped updating or moved the feed",
                rep.fetched, FRESHNESS_DAYS
            );
            warn!(source = %src.slug, "{note}");
            let _ = sources::record_failure(db, src.id, &note).await;
        }
        Ok(()) => {
            if let Err(e) = sources::record_success(db, src.id, None, None).await {
                warn!(source = %src.slug, error = %e, "could not record poll success");
            }
        }
        Err(e) => {
            let msg = e.to_string();
            warn!(source = %src.slug, error = %msg, "poll failed");
            rep.error = Some(msg.clone());
            let _ = sources::record_failure(db, src.id, &msg).await;
        }
    }
    rep
}

async fn poll_inner(
    db: &Db,
    client: &reqwest::Client,
    src: &Source,
    rep: &mut PollReport,
) -> Result<()> {
    let fetched =
        conditional_get(client, &src.url, src.etag.as_deref(), src.last_modified.as_deref()).await?;

    let (bytes, etag, last_modified) = match fetched {
        Fetched::NotModified => {
            debug!(source = %src.slug, "304 not modified");
            rep.not_modified = true;
            sources::record_success(db, src.id, None, None).await?;
            return Ok(());
        }
        Fetched::Body { bytes, etag, last_modified, .. } => (bytes, etag, last_modified),
    };

    let feed = feed_rs::parser::parse(&bytes[..])
        .map_err(|e| IngestError::Parse { source_slug: src.slug.clone(), detail: e.to_string() })?;

    rep.fetched = feed.entries.len();

    let now = Utc::now();
    let too_new = now + ChronoDuration::hours(6);
    let too_old = now - ChronoDuration::days(FRESHNESS_DAYS);

    for entry in &feed.entries {
        let Some(link) = entry.links.first().map(|l| l.href.clone()) else {
            rep.skipped += 1;
            continue;
        };
        let title = entry
            .title
            .as_ref()
            .map(|t| strip_html(&t.content))
            .filter(|t| !t.trim().is_empty());
        let Some(title) = title else {
            rep.skipped += 1;
            continue;
        };

        let published: DateTime<Utc> = entry.published.or(entry.updated).unwrap_or(now);
        if published > too_new || published < too_old {
            debug!(source = %src.slug, %title, %published, "dropping implausible timestamp");
            rep.skipped += 1;
            rep.stale += 1;
            continue;
        }

        let summary = entry.summary.as_ref().map(|s| strip_html(&s.content));
        // feed-rs gives us `content` when the publisher syndicates full text.
        // We store it privately for claim extraction and the overlap check; it
        // is never served. See bg-db::items for the accessor boundary.
        let body = entry
            .content
            .as_ref()
            .and_then(|c| c.body.as_ref())
            .map(|b| strip_html(b))
            .filter(|b| b.len() > 200)
            .or_else(|| summary.clone().filter(|s| s.len() > 200));

        let authors: Vec<String> =
            entry.authors.iter().map(|a| a.name.clone()).filter(|n| !n.is_empty()).collect();

        let image = entry
            .media
            .iter()
            .flat_map(|m| m.content.iter())
            .find_map(|c| c.url.as_ref().map(|u| u.to_string()))
            .or_else(|| {
                entry.media.iter().flat_map(|m| m.thumbnails.iter()).next().map(|t| t.image.uri.clone())
            });

        // Fingerprint over title + lede: enough signal to spot a rewrite of the
        // same event, short enough that a long article does not swamp it.
        let fingerprint_input = match &summary {
            Some(s) => format!("{title} {}", s.chars().take(400).collect::<String>()),
            None => title.clone(),
        };

        let item = NewItem {
            source_id: src.id,
            external_id: Some(entry.id.clone()).filter(|s| !s.is_empty()),
            canonical_url: crate::canonical::canonicalize(&link),
            url_hash: url_hash(&link),
            title,
            dek: summary.as_ref().map(|s| bg_core::text::truncate_words(s, 40)),
            authors,
            published_at: published,
            summary_raw: summary,
            body_hash: body.as_deref().map(sha256_hex),
            body_raw: body,
            simhash: simhash64(&fingerprint_input),
            lang: feed.language.clone().unwrap_or_else(|| "en".into()),
            image_url: image,
        };

        match bg_db::items::insert_new(db, &item).await {
            Ok(Some(_)) => rep.inserted += 1,
            Ok(None) => rep.duplicates += 1,
            Err(e) => {
                warn!(source = %src.slug, error = %e, "insert failed");
                rep.skipped += 1;
            }
        }
    }

    sources::record_success(db, src.id, etag.as_deref(), last_modified.as_deref()).await?;
    info!(
        source = %src.slug,
        fetched = rep.fetched, inserted = rep.inserted, dupes = rep.duplicates,
        "polled"
    );
    Ok(())
}

/// Poll every source that is due, bounded concurrency.
///
/// Concurrency is capped rather than unbounded so a sweep looks like a handful
/// of polite readers instead of a burst that trips rate limiting.
pub async fn poll_due(db: &Db, client: &reqwest::Client, concurrency: usize) -> Vec<PollReport> {
    let due = match sources::due_for_poll(db, 64).await {
        Ok(d) => d,
        Err(e) => {
            warn!(error = %e, "could not list due sources");
            return Vec::new();
        }
    };
    poll_all(db, client, &due, concurrency).await
}

pub async fn poll_all(
    db: &Db,
    client: &reqwest::Client,
    srcs: &[Source],
    concurrency: usize,
) -> Vec<PollReport> {
    use futures::stream::{self, StreamExt};
    stream::iter(srcs.iter())
        .map(|s| poll_source(db, client, s))
        .buffer_unordered(concurrency.max(1))
        .collect()
        .await
}
