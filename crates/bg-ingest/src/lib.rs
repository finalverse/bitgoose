//! # bg-ingest
//!
//! Everything that reaches out to the network: feed polling, URL
//! canonicalization, robots.txt, and market data.
//!
//! The design constraint is politeness. BitGoose reads other people's servers
//! continuously and forever, so every request carries an identifying user
//! agent, honours robots.txt, sends conditional-GET validators, and runs under
//! a concurrency cap. A source that blocks us is a source we lose permanently.

pub mod canonical;
pub mod crawl;
pub mod feeds;
pub mod http;
pub mod market;
pub mod readable;
pub mod relevance;
pub mod robots;
pub mod seed;
pub mod video;

use thiserror::Error;

pub type Result<T, E = IngestError> = std::result::Result<T, E>;

#[derive(Debug, Error)]
pub enum IngestError {
    #[error("http {status} from {url}")]
    Http { status: u16, url: String },

    #[error("could not parse feed from {source_slug}: {detail}")]
    Parse { source_slug: String, detail: String },

    #[error("decode error: {0}")]
    Decode(String),

    #[error(transparent)]
    Request(#[from] reqwest::Error),

    #[error(transparent)]
    Db(#[from] bg_db::DbError),
}

/// Re-check robots.txt for every source and persist the verdict.
///
/// Run on a schedule, not just at seed time: a publisher can add a
/// `Disallow` at any point, and continuing to poll after that is exactly the
/// behaviour that gets a crawler banned.
pub async fn refresh_robots(
    db: &bg_db::Db,
    client: &reqwest::Client,
    agent: &str,
) -> Vec<(String, bool)> {
    let Ok(all) = bg_db::sources::all(db).await else {
        return Vec::new();
    };
    let mut out = Vec::with_capacity(all.len());
    for s in all {
        // One fetch, two questions: may we read it, and may a model read it.
        // They are increasingly different answers and were previously not even
        // being asked separately.
        let verdict = robots::verdict(client, agent, &s.url).await;
        // The AI posture belongs to the publisher, and the feed is often on a
        // different host that carries none of their rules. `feeds.bbci.co.uk`
        // says nothing while `www.bbc.co.uk` blocks five AI crawlers, and
        // `feeds.arstechnica.com` says nothing while `arstechnica.com` blocks
        // four. Extraction fetches the *article*, so the article's host is the
        // one whose wishes apply. Falls back to the feed's verdict when a
        // source has no homepage recorded.
        let ai = if s.homepage.is_empty() || s.homepage == s.url {
            verdict.clone()
        } else {
            robots::verdict(client, agent, &s.homepage).await
        };
        if verdict.allowed != s.robots_ok {
            tracing::info!(source = %s.slug, allowed = verdict.allowed, "robots.txt verdict changed");
            let _ = bg_db::sources::set_robots_ok(db, s.id, verdict.allowed).await;
        }
        if ai.ai_input != s.ai_input_ok {
            tracing::info!(
                source = %s.slug,
                ai_input = ai.ai_input,
                signal = ai.signal.as_deref().unwrap_or("(none stated)"),
                "publisher's AI posture changed"
            );
            let _ = bg_db::sources::set_ai_input(db, s.id, ai.ai_input, ai.signal.as_deref()).await;
        }
        out.push((s.slug, verdict.allowed));
    }
    out
}
