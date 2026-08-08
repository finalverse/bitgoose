//! `/og/:slug.png` — the generated share card for one story.
//!
//! Rendered on demand and cached in memory. Rendering costs a few milliseconds
//! and a crawler fetches each card once or twice, so a small cache is enough to
//! keep a burst of shares from re-rasterising the same image; nothing here is
//! worth putting on disk, and a restart simply re-renders.
//!
//! Held stories are unreachable here for the same reason they are unreachable
//! on the site: `published_by_slug` is the only lookup used. A share card is a
//! public surface, and one that rendered the headline of a story we decided not
//! to publish would leak it just as effectively as the page would.

use crate::ogcard::{self, Card};
use axum::{
    extract::{Path, State},
    http::{header, StatusCode},
    response::{IntoResponse, Response},
};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

#[derive(Clone, Default)]
pub struct CardCache(Arc<Mutex<HashMap<String, Arc<Vec<u8>>>>>);

/// Beyond this many cards, drop the lot and start again.
///
/// A plain clear rather than an LRU: cards are cheap to regenerate, the working
/// set is the current front page, and an eviction policy here would be more
/// machinery than the problem deserves.
const MAX_CACHED: usize = 256;

pub fn router(db: bg_db::Db) -> axum::Router {
    axum::Router::new()
        .route("/og/{slug}", axum::routing::get(card))
        .with_state((db, CardCache::default()))
}

async fn card(
    State((db, cache)): State<(bg_db::Db, CardCache)>,
    Path(slug): Path<String>,
) -> Response {
    let slug = slug.strip_suffix(".png").unwrap_or(&slug).to_string();

    if let Some(bytes) = cache.0.lock().ok().and_then(|c| c.get(&slug).cloned()) {
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

    let rendered = ogcard::png(&Card {
        headline: &headline,
        beat: story.beat.as_str(),
        section: story.category.label(),
        sources: story.source_count,
        has_analysis,
    });

    let Some(bytes) = rendered else {
        // No usable font on this host. Point at the static card rather than
        // serving a blank one — a redirect keeps the URL in the meta tags valid
        // whatever the host can do.
        return axum::response::Redirect::temporary("/og-default.png").into_response();
    };

    let bytes = Arc::new(bytes);
    if let Ok(mut c) = cache.0.lock() {
        if c.len() >= MAX_CACHED {
            c.clear();
        }
        c.insert(slug, bytes.clone());
    }
    png_response(bytes)
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
