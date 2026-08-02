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

pub fn client(user_agent: &str) -> Result<reqwest::Client, IngestError> {
    Ok(reqwest::Client::builder()
        .user_agent(user_agent)
        .timeout(Duration::from_secs(20))
        .connect_timeout(Duration::from_secs(8))
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
