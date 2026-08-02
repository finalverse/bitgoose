//! The shared HTTP client and conditional-GET plumbing.

use crate::IngestError;
use std::time::Duration;

/// Some publishers (Bitcoin Magazine among the sources we poll) return 403 to
/// anything that does not look like a browser. We still identify ourselves —
/// the bot token and contact URL stay in the string — but pair it with a
/// browser product token so the WAF lets us through. Announcing who we are and
/// getting blocked helps nobody; this is the honest middle.
///
/// Defined in [`bg_core::brand`] so the `/bot` page the URL points at is
/// generated from the same string we send.
pub use bg_core::brand::DEFAULT_UA;

/// Whole-request timeout, seconds. Override with `BG_HTTP_TIMEOUT_S`.
///
/// 20s is right for a well-connected host and wrong for a constrained one, and
/// the failure is quiet: the largest feeds simply time out and the roster
/// silently shrinks. Measured, the sources we poll run from 29KB to 262KB, so
/// the biggest needs 26s on a 10KB/s uplink before any concurrency — and
/// `BG_INGEST_CONCURRENCY` divides that bandwidth, making a higher setting
/// actively harmful on a slow link. Both are configurable for that reason.
const DEFAULT_TIMEOUT_S: u64 = 20;
const DEFAULT_CONNECT_TIMEOUT_S: u64 = 8;

fn secs_from_env(key: &str, default: u64) -> Duration {
    let v = std::env::var(key)
        .ok()
        .and_then(|s| s.trim().parse::<u64>().ok())
        .filter(|n| *n > 0)
        .unwrap_or(default);
    Duration::from_secs(v)
}

pub fn client(user_agent: &str) -> Result<reqwest::Client, IngestError> {
    Ok(reqwest::Client::builder()
        .user_agent(user_agent)
        .timeout(secs_from_env("BG_HTTP_TIMEOUT_S", DEFAULT_TIMEOUT_S))
        .connect_timeout(secs_from_env(
            "BG_HTTP_CONNECT_TIMEOUT_S",
            DEFAULT_CONNECT_TIMEOUT_S,
        ))
        // Feeds redirect constantly (blockworks.co -> blockworks.com,
        // coindesk's trailing slash), so following them is required, but a long
        // chain means someone is bouncing us and we should stop.
        .redirect(reqwest::redirect::Policy::limited(5))
        .gzip(true)
        .build()?)
}

/// Outcome of a conditional GET.
pub enum Fetched {
    /// Server said nothing changed. Costs us one round trip and no parsing.
    NotModified,
    Body {
        bytes: Vec<u8>,
        etag: Option<String>,
        last_modified: Option<String>,
        final_url: String,
    },
}

/// GET with `If-None-Match` / `If-Modified-Since` when we hold validators.
///
/// Sending these is the difference between a good citizen and a scraper: on a
/// five-minute poll across nine feeds it turns ~2,600 full downloads a day into
/// a handful of real ones.
pub async fn conditional_get(
    client: &reqwest::Client,
    url: &str,
    etag: Option<&str>,
    last_modified: Option<&str>,
) -> Result<Fetched, IngestError> {
    let mut req = client.get(url);
    if let Some(e) = etag {
        req = req.header(reqwest::header::IF_NONE_MATCH, e);
    }
    if let Some(lm) = last_modified {
        req = req.header(reqwest::header::IF_MODIFIED_SINCE, lm);
    }

    let resp = req.send().await?;
    let status = resp.status();

    if status == reqwest::StatusCode::NOT_MODIFIED {
        return Ok(Fetched::NotModified);
    }
    if !status.is_success() {
        return Err(IngestError::Http {
            status: status.as_u16(),
            url: url.to_string(),
        });
    }

    let final_url = resp.url().to_string();
    let hdr = |name: reqwest::header::HeaderName| {
        resp.headers()
            .get(name)
            .and_then(|v| v.to_str().ok())
            .map(str::to_string)
    };
    let new_etag = hdr(reqwest::header::ETAG);
    let new_lm = hdr(reqwest::header::LAST_MODIFIED);
    let bytes = resp.bytes().await?.to_vec();

    Ok(Fetched::Body {
        bytes,
        etag: new_etag,
        last_modified: new_lm,
        final_url,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timeout_overrides_are_read_but_never_accept_zero() {
        // A zero or unparseable override would mean "no timeout" in reqwest,
        // which turns one wedged publisher into a stuck poll cycle.
        unsafe { std::env::set_var("BG_TEST_TMO", "0") };
        assert_eq!(secs_from_env("BG_TEST_TMO", 20), Duration::from_secs(20));
        unsafe { std::env::set_var("BG_TEST_TMO", "not-a-number") };
        assert_eq!(secs_from_env("BG_TEST_TMO", 20), Duration::from_secs(20));
        unsafe { std::env::set_var("BG_TEST_TMO", " 120 ") };
        assert_eq!(secs_from_env("BG_TEST_TMO", 20), Duration::from_secs(120));
        unsafe { std::env::remove_var("BG_TEST_TMO") };
        assert_eq!(secs_from_env("BG_TEST_TMO", 20), Duration::from_secs(20));
    }
}
