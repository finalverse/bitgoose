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
/// Feeds that syndicate other outlets' headlines rather than their own.
///
/// Matched by slug rather than by sniffing the title, because the "Headline -
/// Outlet" split is only safe where the format is guaranteed. Guessing it on a
/// publisher's own feed would amputate any headline containing a dash.
fn is_aggregator(slug: &str) -> bool {
    slug.starts_with("gnews")
}

pub async fn poll_source(db: &Db, client: &reqwest::Client, src: &Source) -> PollReport {
    let mut rep = PollReport {
        source_slug: src.slug.clone(),
        ..Default::default()
    };

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
    // A crawled source has no feed to parse; everything below this point is
    // about XML. Dispatching here rather than at the caller keeps the two
    // mechanisms behind one "poll this source" entry point, so the scheduler,
    // the robots gate and the reporting do not have to know the difference.
    if src.kind == bg_core::domain::SourceKind::Html {
        return crawl_inner(db, client, src, rep).await;
    }

    let fetched = conditional_get(
        client,
        &src.url,
        src.etag.as_deref(),
        src.last_modified.as_deref(),
    )
    .await?;

    let (bytes, etag, last_modified) = match fetched {
        Fetched::NotModified => {
            debug!(source = %src.slug, "304 not modified");
            rep.not_modified = true;
            sources::record_success(db, src.id, None, None).await?;
            return Ok(());
        }
        Fetched::Body {
            bytes,
            etag,
            last_modified,
            ..
        } => (bytes, etag, last_modified),
    };

    let feed = feed_rs::parser::parse(&bytes[..]).map_err(|e| IngestError::Parse {
        source_slug: src.slug.clone(),
        detail: e.to_string(),
    })?;

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

        // Which desk does this belong to, and does it belong here at all?
        //
        // A source that pins a beat (arXiv cs.AI, CoinDesk) is taken wholesale:
        // everything it publishes is on topic by definition. A general-interest
        // source (Bloomberg, Ars Technica, The Verge) is routed one item at a
        // time and dropped if it matches neither desk — their feeds are mostly
        // equities and consumer gadgets, and taking them whole would bury the
        // coverage we actually want.
        //
        // Deliberately a string scan rather than a triage model call: these
        // outlets publish hundreds of items a day, and this answers a question
        // a word list answers well.
        let beat = match src.beat {
            Some(b) => b,
            None => {
                let blurb = entry
                    .summary
                    .as_ref()
                    .map(|s| strip_html(&s.content))
                    .unwrap_or_default();
                match crate::relevance::classify(&format!("{title} {blurb}")) {
                    Some(b) => b,
                    None => {
                        rep.skipped += 1;
                        continue;
                    }
                }
            }
        };

        let published: DateTime<Utc> = entry.published.or(entry.updated).unwrap_or(now);
        if published > too_new || published < too_old {
            debug!(source = %src.slug, %title, %published, "dropping implausible timestamp");
            rep.skipped += 1;
            rep.stale += 1;
            continue;
        }

        // YouTube puts the description in `media:group/media:description`, which
        // feed-rs surfaces on the media object rather than as `entry.summary` —
        // so without this every video item arrived with no text at all. That is
        // not a cosmetic gap: Herald was handed a bare title and asked for two
        // or three sentences, and a small model given nothing to summarise
        // invents something. See the guard in `bg_agents::herald`.
        let summary = entry
            .summary
            .as_ref()
            .map(|s| strip_html(&s.content))
            .or_else(|| {
                entry
                    .media
                    .iter()
                    .find_map(|m| m.description.as_ref().map(|d| strip_html(&d.content)))
            })
            .filter(|s| !s.trim().is_empty());
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

        let mut authors: Vec<String> = entry
            .authors
            .iter()
            .map(|a| a.name.clone())
            .filter(|n| !n.is_empty())
            .collect();

        // An aggregator's title carries the outlet on the end of it. Lift it
        // off, and credit the outlet rather than the aggregator: a story
        // reported by Bloomberg Law News and found through Google News was not
        // written by Google News, and on a site built to show who stands behind
        // a fact that distinction is the product.
        let title = if is_aggregator(&src.slug) {
            let (head, outlet) = bg_core::text::split_aggregator_title(&title);
            if let Some(o) = outlet {
                if !authors.iter().any(|a| a == o) {
                    authors.push(o.to_string());
                }
            }
            head.to_string()
        } else {
            title
        };

        let image = entry
            .media
            .iter()
            .flat_map(|m| m.content.iter())
            .find_map(|c| c.url.as_ref().map(|u| u.to_string()))
            .or_else(|| {
                entry
                    .media
                    .iter()
                    .flat_map(|m| m.thumbnails.iter())
                    .next()
                    .map(|t| t.image.uri.clone())
            });

        // Only consulted for video sources: a text publisher linking to a
        // YouTube page is citing it, not syndicating it, and should not turn
        // into a player on our page.
        let video_id = if src.kind == bg_core::domain::SourceKind::Video {
            crate::video::youtube_id(&link, &entry.id)
        } else {
            None
        };

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
            dek: summary
                .as_ref()
                .map(|s| bg_core::text::truncate_words(s, 40)),
            authors,
            published_at: published,
            summary_raw: summary,
            body_hash: body.as_deref().map(sha256_hex),
            body_raw: body,
            simhash: simhash64(&fingerprint_input),
            // Normalised at the boundary. The corpus already holds `en-us`,
            // `en` and `en-US` for the same language, which makes any
            // per-language surface built on this quietly wrong.
            lang: bg_core::text::normalize_lang(feed.language.as_deref().unwrap_or("en")),
            // YouTube's media:content is the player URL, not a picture.
            // Converting here means the column holds something an <img> can
            // actually load, rather than every reader having to know.
            image_url: image.as_deref().and_then(bg_core::media::as_image),
            video_id,
            beat: Some(beat),
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
/// Poll a source that publishes no feed, by reading its index page.
///
/// Produces exactly the items a feed would: a title, a canonical URL, a
/// timestamp. The body is left to `readable::fetch` later in the pipeline, the
/// same as for a feed item whose summary was truncated — so a crawled source
/// and a fed one are indistinguishable downstream, which is the point.
async fn crawl_inner(
    db: &Db,
    client: &reqwest::Client,
    src: &Source,
    rep: &mut PollReport,
) -> Result<()> {
    let agent =
        std::env::var("BG_USER_AGENT").unwrap_or_else(|_| bg_core::brand::DEFAULT_UA.into());
    let respect = std::env::var("BG_RESPECT_ROBOTS")
        .map(|v| v != "false")
        .unwrap_or(true);

    let found = crate::crawl::index(client, &agent, &src.url, None, respect, 40).await?;
    rep.fetched = found.len();

    for f in &found {
        // An index page carries no publication time. Using "now" is honest
        // about what we know — we saw it now — and the news horizon then treats
        // a crawled item the same as a freshly published one, which is right:
        // it appeared on the front page just now.
        let item = NewItem {
            source_id: src.id,
            external_id: None,
            canonical_url: f.url.clone(),
            url_hash: url_hash(&f.url),
            title: f.title.clone(),
            dek: None,
            authors: Vec::new(),
            published_at: chrono::Utc::now(),
            summary_raw: None,
            body_raw: None,
            body_hash: None,
            simhash: simhash64(&f.title),
            lang: bg_core::text::normalize_lang("en"),
            image_url: None,
            video_id: None,
            beat: crate::relevance::classify(&f.title),
        };
        if bg_db::items::insert_new(db, &item).await?.is_some() {
            rep.inserted += 1;
        }
    }

    sources::record_success(db, src.id, None, None).await?;
    Ok(())
}

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
