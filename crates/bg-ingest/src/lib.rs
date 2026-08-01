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
pub mod feeds;
pub mod http;
pub mod market;
pub mod robots;
pub mod seed;

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
        let ok = robots::allows(client, agent, &s.url).await;
        if ok != s.robots_ok {
            tracing::info!(source = %s.slug, allowed = ok, "robots.txt verdict changed");
            let _ = bg_db::sources::set_robots_ok(db, s.id, ok).await;
        }
        out.push((s.slug, ok));
    }
    out
}
