//! Syndication and discovery: RSS, sitemap, robots.
//!
//! A news property that cannot be syndicated or indexed does not get read.
//! These endpoints live next to the JSON API rather than in the Leptos app
//! because they are documents, not pages — they need exact content types and
//! byte-level control over their XML.
//!
//! Note the symmetry with the rest of the system: BitGoose consumes nine RSS
//! feeds and publishes one. Our feed carries summaries and links, never full
//! source text — the same rule we hold other people's content to.

use crate::ApiState;
use axum::extract::State;
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::Router;

pub fn routes() -> Router<ApiState> {
    Router::new()
        .route("/robots.txt", get(robots))
        .route("/feed.xml", get(rss))
        .route("/rss", get(rss))
        .route("/sitemap.xml", get(sitemap))
}

/// Public base URL, for absolute links in feeds and sitemaps.
fn base_url() -> String {
    std::env::var("BG_PUBLIC_BASE_URL")
        .unwrap_or_else(|_| format!("https://{}", bg_core::brand::DOMAIN))
        .trim_end_matches('/')
        .to_string()
}

/// Escape text for XML content.
///
/// Headlines routinely contain `&` and quotes; an unescaped ampersand makes the
/// whole feed unparseable, which is a silent, total failure — every aggregator
/// drops it at once.
fn xml_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 16);
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            // Control characters are illegal in XML 1.0 even when escaped.
            c if (c as u32) < 0x20 && c != '\t' && c != '\n' && c != '\r' => out.push(' '),
            c => out.push(c),
        }
    }
    out
}

fn xml(body: String) -> Response {
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "application/xml; charset=utf-8")],
        body,
    )
        .into_response()
}

// ---------------------------------------------------------------------------

async fn robots() -> Response {
    let base = base_url();
    // Deliberately permissive to well-behaved crawlers, including AI ones.
    // Publishing a machine-readable claim graph and then blocking the agents
    // that would use it would be incoherent.
    let body = format!(
        "# BitGoose — the AI newsroom for crypto\n\
         # Machine-readable API: {base}/v1   MCP: {base}/mcp\n\
         \n\
         User-agent: *\n\
         Allow: /\n\
         Disallow: /rpc/\n\
         \n\
         Sitemap: {base}/sitemap.xml\n"
    );
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "text/plain; charset=utf-8")],
        body,
    )
        .into_response()
}

async fn rss(State(s): State<ApiState>) -> Response {
    let base = base_url();
    let stories = bg_db::stories::published(&s.db, None, 60, 0)
        .await
        .unwrap_or_default();

    let now = chrono::Utc::now().to_rfc2822();
    let mut items = String::new();
    for st in &stories {
        let link = format!("{base}/story/{}", st.slug);
        let pub_date = st
            .published_at
            .map(|d| d.to_rfc2822())
            .unwrap_or_else(|| now.clone());
        let description = st.summary.clone().unwrap_or_else(|| st.title.clone());
        items.push_str(&format!(
            "    <item>\n\
             \x20     <title>{}</title>\n\
             \x20     <link>{}</link>\n\
             \x20     <guid isPermaLink=\"true\">{}</guid>\n\
             \x20     <pubDate>{}</pubDate>\n\
             \x20     <category>{}</category>\n\
             \x20     <description>{}</description>\n\
             \x20   </item>\n",
            xml_escape(&st.title),
            xml_escape(&link),
            xml_escape(&link),
            pub_date,
            xml_escape(st.category.label()),
            xml_escape(&description),
        ));
    }

    let last_build = stories
        .first()
        .and_then(|s| s.published_at)
        .map(|d| d.to_rfc2822())
        .unwrap_or(now);

    xml(format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
         <rss version=\"2.0\" xmlns:atom=\"http://www.w3.org/2005/Atom\">\n\
         \x20 <channel>\n\
         \x20   <title>BitGoose</title>\n\
         \x20   <link>{base}</link>\n\
         \x20   <atom:link href=\"{base}/feed.xml\" rel=\"self\" type=\"application/rss+xml\" />\n\
         \x20   <description>{}</description>\n\
         \x20   <language>en</language>\n\
         \x20   <lastBuildDate>{last_build}</lastBuildDate>\n\
         \x20   <generator>BitGoose</generator>\n\
         {items}\
         \x20 </channel>\n\
         </rss>\n",
        xml_escape(bg_core::brand::TAGLINE),
    ))
}

async fn sitemap(State(s): State<ApiState>) -> Response {
    let base = base_url();
    let stories = bg_db::stories::published(&s.db, None, 5_000, 0)
        .await
        .unwrap_or_default();
    let assets = bg_db::prices::assets(&s.db).await.unwrap_or_default();

    let mut urls = String::new();
    let mut add = |loc: String, changefreq: &str, priority: &str, lastmod: Option<String>| {
        urls.push_str(&format!(
            "  <url>\n    <loc>{}</loc>\n{}    <changefreq>{}</changefreq>\n    <priority>{}</priority>\n  </url>\n",
            xml_escape(&loc),
            lastmod
                .map(|m| format!("    <lastmod>{m}</lastmod>\n"))
                .unwrap_or_default(),
            changefreq,
            priority
        ));
    };

    add(base.clone(), "hourly", "1.0", None);
    for (path, freq, pri) in [
        ("/desk", "hourly", "0.9"),
        ("/wire", "hourly", "0.9"),
        ("/prices", "hourly", "0.7"),
        ("/flyway", "daily", "0.6"),
        ("/flock", "hourly", "0.6"),
        ("/standards", "monthly", "0.5"),
        ("/developers", "monthly", "0.5"),
    ] {
        add(format!("{base}{path}"), freq, pri, None);
    }

    for st in &stories {
        // Recent stories change (corrections, new corroboration); older ones
        // settle. Telling crawlers that is the difference between a useful
        // recrawl budget and a wasted one.
        let age_h = st
            .published_at
            .map(|p| (chrono::Utc::now() - p).num_hours())
            .unwrap_or(999);
        let (freq, pri) = match age_h {
            0..=24 => ("hourly", "0.9"),
            25..=168 => ("daily", "0.7"),
            _ => ("monthly", "0.4"),
        };
        add(
            format!("{base}/story/{}", st.slug),
            freq,
            pri,
            Some(st.updated_at.format("%Y-%m-%d").to_string()),
        );
    }

    for a in &assets {
        add(format!("{base}/asset/{}", a.symbol), "daily", "0.5", None);
    }

    xml(format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
         <urlset xmlns=\"http://www.sitemaps.org/schemas/sitemap/0.9\">\n{urls}</urlset>\n"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn xml_escaping_covers_the_characters_that_break_feeds() {
        assert_eq!(
            xml_escape(r#"Coinbase & Circle's "deal" <live>"#),
            "Coinbase &amp; Circle&apos;s &quot;deal&quot; &lt;live&gt;"
        );
    }

    #[test]
    fn control_characters_are_stripped_not_escaped() {
        // Illegal in XML 1.0 even as entities — escaping them still breaks parsers.
        let out = xml_escape("head\u{0}line\u{7}");
        assert!(!out.contains('\u{0}') && !out.contains('\u{7}'), "{out:?}");
        assert!(out.starts_with("head"));
    }

    #[test]
    fn newlines_and_tabs_survive() {
        assert_eq!(xml_escape("a\tb\nc"), "a\tb\nc");
    }
}
