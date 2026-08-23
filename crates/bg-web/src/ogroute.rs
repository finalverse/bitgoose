//! The pictures that represent a story off-site: `/og/:slug.png` and `/img/:slug`.
//!
//! ## Why both, and why they are both on our own domain
//!
//! A BitGoose link pasted into WeChat rendered as a grey chain-link icon, the
//! bare domain, and no description — while a Reuters link in the same chat
//! showed the roundel, the headline and a standfirst. The metadata was not the
//! problem; every tag was present and correct. Three other things were:
//!
//! * **`og:image` pointed at somebody else's CDN.** A story sourced from
//!   YouTube advertised `i.ytimg.com`, which is not reachable from mainland
//!   China, where WeChat's crawler runs. The card cannot render a picture it
//!   cannot fetch. Hotlinking was also spending a publisher's bandwidth on our
//!   share cards, and inheriting their outages and their hotlink rules.
//! * **Cards were rendered on demand.** Cold, `/og/:slug` measured 8.9 seconds.
//!   Crawlers do not wait that long; several give up in two.
//! * **The card was the wrong shape for the client.** See [`ogcard::Shape`].
//!
//! So: `/img/:slug` serves *our copy* of the publisher's picture, and
//! `/og/:slug.png` serves a card we drew. Either way the crawler fetches
//! `bitgoose.com`, and either way the answer comes off disk.
//!
//! ## The mirror is not an open proxy
//!
//! `/img/:slug` takes a **slug**, never a URL. The only thing it will ever
//! fetch is the image URL already recorded against a *published* story, so
//! there is no parameter for a caller to point at an internal address. A URL
//! handed to us in a request is never fetched.
//!
//! Held stories are unreachable through both routes, for the same reason they
//! are unreachable on the site: `published_by_slug` is the only lookup used. A
//! card rendering the headline of a story we decided not to publish would leak
//! it just as effectively as the page would.

use crate::ogcard::{self, Card, Shape};
// The cache, the slug guard and the byte sniffer live in `bg-ingest` so the
// newsroom can fill the cache at publish time. The web crate only serves it.
use axum::{
    extract::{Path, Query, State},
    http::{header, HeaderMap, StatusCode},
    response::{IntoResponse, Redirect, Response},
};
pub use bg_ingest::mirror::mirrored;
use bg_ingest::mirror::{cache_dir, safe_slug, sniff, store, store_lead_image};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

#[derive(Clone, Default)]
pub struct CardCache(Arc<Mutex<HashMap<String, Arc<Vec<u8>>>>>);

/// Beyond this many cards in memory, drop the lot and start again.
///
/// A plain clear rather than an LRU: disk is the real cache now, so a memory
/// miss costs a file read rather than a re-render, and an eviction policy here
/// would be more machinery than the problem deserves.
const MAX_CACHED: usize = 256;

/// Fetch a story's picture in the background, for the paths that discover a
/// gap at request time.
///
/// The main filling happens at publish, in the pipeline — this is the backstop
/// for stories published before that existed.
pub fn warm(db: bg_db::Db, slug: String) {
    if !safe_slug(&slug) || mirrored(&slug).is_some() {
        return;
    }
    tokio::spawn(async move {
        let Ok(story) = bg_db::stories::published_by_slug(&db, &slug).await else {
            return;
        };
        let Some(url) = story
            .image_url
            .as_deref()
            .and_then(bg_core::media::as_image)
        else {
            return;
        };
        let ua = std::env::var("BG_USER_AGENT")
            .unwrap_or_else(|_| bg_ingest::http::DEFAULT_UA.to_string());
        let Ok(client) = bg_ingest::http::client(&ua) else {
            return;
        };
        store_lead_image(&client, &slug, &url).await;
    });
}

pub fn router(db: bg_db::Db) -> axum::Router {
    axum::Router::new()
        .route("/og/{slug}", axum::routing::get(card))
        .route("/img/{slug}", axum::routing::get(mirror))
        .with_state((db, CardCache::default()))
}

async fn mirror(
    State((db, _)): State<(bg_db::Db, CardCache)>,
    Path(slug): Path<String>,
) -> Response {
    if !safe_slug(&slug) {
        return (StatusCode::NOT_FOUND, "no such image").into_response();
    }
    if let Some(path) = mirrored(&slug) {
        if let Ok(bytes) = std::fs::read(&path) {
            if let Some(ct) = sniff(&bytes) {
                return image_response(ct, bytes);
            }
        }
    }
    // Not held. Send the caller to the card we can always produce, and fetch
    // the real one behind their back for next time.
    warm(db, slug.clone());
    Redirect::temporary(&format!("/og/{slug}.png")).into_response()
}

#[derive(serde::Deserialize)]
struct CardQuery {
    /// `?sq=1` forces the square card. Used by the meta tags rather than left
    /// to user-agent sniffing at image-fetch time, because the crawler that
    /// fetches the picture is often not the one that read the page.
    #[serde(default)]
    sq: Option<String>,
}

async fn card(
    State((db, cache)): State<(bg_db::Db, CardCache)>,
    Path(slug): Path<String>,
    Query(q): Query<CardQuery>,
    headers: HeaderMap,
) -> Response {
    let slug = slug.strip_suffix(".png").unwrap_or(&slug).to_string();
    if !safe_slug(&slug) {
        return (StatusCode::NOT_FOUND, "no such story").into_response();
    }

    let shape = if q.sq.is_some() || is_wechat(&headers) {
        Shape::Square
    } else {
        Shape::Wide
    };
    let key = format!("{slug}-{}", if shape == Shape::Square { "s" } else { "w" });

    if let Some(bytes) = cache.0.lock().ok().and_then(|c| c.get(&key).cloned()) {
        return png_response(bytes);
    }
    let disk = cache_dir().join(format!("og-{key}.png"));
    if let Ok(bytes) = std::fs::read(&disk) {
        let bytes = Arc::new(bytes);
        remember(&cache, &key, &bytes);
        return png_response(bytes);
    }

    let Ok(story) = bg_db::stories::published_by_slug(&db, &slug).await else {
        return (StatusCode::NOT_FOUND, "no such story").into_response();
    };

    // The headline as published, not the raw feed title: the card should match
    // what the page says.
    let headline = bg_db::articles::latest_for_story(&db, story.id)
        .await
        .ok()
        .flatten()
        .map(|a| a.headline)
        .unwrap_or_else(|| story.title.clone());

    let has_analysis = bg_db::analyses::for_story(&db, story.id)
        .await
        .ok()
        .flatten()
        .is_some();

    let rendered = ogcard::png_shaped(
        &Card {
            headline: &headline,
            beat: story.beat.as_str(),
            section: story.category.label(),
            sources: story.source_count,
            has_analysis,
        },
        shape,
    );

    let Some(bytes) = rendered else {
        // No usable font on this host. Point at the static card rather than
        // serving a blank one — a redirect keeps the URL in the meta tags valid
        // whatever the host can do.
        return Redirect::temporary("/og-default.png").into_response();
    };

    store(&disk, &bytes);
    let bytes = Arc::new(bytes);
    remember(&cache, &key, &bytes);
    png_response(bytes)
}

fn remember(cache: &CardCache, key: &str, bytes: &Arc<Vec<u8>>) {
    if let Ok(mut c) = cache.0.lock() {
        if c.len() >= MAX_CACHED {
            c.clear();
        }
        c.insert(key.to_string(), bytes.clone());
    }
}

/// WeChat's crawler, which wants the square card.
///
/// Belt and braces: the meta tags already ask for `?sq=1`, but WeChat fetches
/// images with several different agents and not all of them carry the query
/// through a redirect.
fn is_wechat(h: &HeaderMap) -> bool {
    h.get(header::USER_AGENT)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|ua| ua.contains("MicroMessenger") || ua.contains("wechat"))
}

fn image_response(content_type: &'static str, bytes: Vec<u8>) -> Response {
    (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, content_type),
            (header::CACHE_CONTROL, "public, max-age=604800, immutable"),
        ],
        bytes,
    )
        .into_response()
}

fn png_response(bytes: Arc<Vec<u8>>) -> Response {
    (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, "image/png"),
            // A day. Crawlers re-fetch on their own schedule and the card only
            // changes if the headline does, which a correction would do — long
            // enough to matter, short enough that a fix is seen.
            (header::CACHE_CONTROL, "public, max-age=86400"),
        ],
        (*bytes).clone(),
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugs_cannot_escape_the_cache_directory() {
        assert!(safe_slug("how-china-will-win-the-ai-race"));
        assert!(!safe_slug("../../etc/passwd"));
        assert!(!safe_slug("a/b"));
        assert!(!safe_slug("Story"));
        assert!(!safe_slug(""));
    }

    #[test]
    fn only_real_image_bytes_are_served() {
        assert_eq!(sniff(&[0xFF, 0xD8, 0xFF, 0xE0]), Some("image/jpeg"));
        assert_eq!(sniff(b"\x89PNG\r\n\x1a\n"), Some("image/png"));
        // An HTML error page served with an image content type.
        assert_eq!(sniff(b"<!DOCTYPE html><html>"), None);
        // SVG can carry script, and we would be serving it from our own origin.
        assert_eq!(sniff(b"<svg xmlns=\"http://www.w3.org/2000/svg\">"), None);
    }
}
