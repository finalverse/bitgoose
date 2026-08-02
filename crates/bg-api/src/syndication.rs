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
        .route("/bot", get(bot))
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

/// The page our crawler's User-Agent points at.
///
/// Every request BitGooseBot makes carries `+https://bitgoose.com/bot`, and the
/// first thing a publisher does with an unfamiliar agent in their logs is open
/// that URL. Answering it with a 404 is how a crawler gets blocked. Everything
/// stated here is enforced in code, not promised: the limits come from
/// [`bg_core::policy`] and the fetch behaviour from `bg-ingest`.
async fn bot(State(s): State<ApiState>) -> Response {
    // A raw literal rather than `\`-continuations: this is a plain-text page a
    // human reads in a terminal, and `\` swallows the leading whitespace of the
    // next line, so hanging indents under the bullets silently vanish.
    const PAGE: &str = r#"{NAME}Bot — the {NAME} newsroom crawler

User-Agent: {UA}
Operator:   {NAME} ({BASE})
Contact:    https://github.com/finalverse/bitgoose/issues

WHAT IT FETCHES
Publisher RSS/Atom feeds only, and no more than one request per feed per poll.
It does not crawl article pages, follow links out of a feed, or fetch images,
scripts or stylesheets.

HOW IT BEHAVES
  * robots.txt is fetched and honoured before any feed request; a Disallow on
    the feed path drops the source from the roster entirely. Allow/Disallow
    prefixes and per-agent groups are supported; `$` anchoring is not, and
    anything it cannot parse is treated as a Disallow.
  * Conditional GET on every poll (If-None-Match / If-Modified-Since), so an
    unchanged feed costs you a 304 and no body.
  * Per-source poll interval, {INTERVAL}s at the fastest, with a small number
    of feeds in flight at once. It does not burst.

WHAT IT DOES WITH WHAT IT READS
Feed text is used to identify and cross-check events. It is held as a private
working copy and is never served. Published articles are original synthesis;
any direct quotation is capped at {QUOTE_CAP} words, attributed by name, and
carries a link back to you. These are hard publish gates — output that breaks
them is blocked, not corrected.

IMAGES
Where your feed declares one — <media:content>, <media:thumbnail> or an
<enclosure> — we may show it beside the headline, credited to you by name and
linked back to the article. It is referenced from your own CDN, never copied
onto ours, so the request is the reader's browser and the analytics and the
control stay yours. Only media you put in the feed is ever used; we do not open
the article page to look for more. Send us a Disallow, or an issue at the link
above, and we will stop.

HOW TO LIMIT OR BLOCK IT
  User-agent: BitGooseBot
  Disallow: /

Crawl-delay is NOT parsed. The fixed {INTERVAL}s floor above is more
conservative than the values publishers normally set, but if you need us
slower or gone, open an issue at the link above and we will change it — no
argument, no appeal process.

Our own feed, on the same terms we ask of you: {BASE}/feed.xml
"#;
    let body = PAGE
        .replace("{NAME}", bg_core::brand::NAME)
        .replace("{DOMAIN}", bg_core::brand::DOMAIN)
        .replace("{UA}", &crawler_ua())
        .replace("{BASE}", &base_url())
        .replace("{INTERVAL}", &fastest_poll_interval(&s).await.to_string())
        .replace("{QUOTE_CAP}", &bg_core::policy::MAX_QUOTE_WORDS.to_string());
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "text/plain; charset=utf-8")],
        body,
    )
        .into_response()
}

/// The User-Agent the ingester actually sends.
///
/// Resolved the same way the worker resolves it, so the page cannot advertise
/// one agent while the crawler sends another — which is worse than having no
/// page at all.
fn crawler_ua() -> String {
    std::env::var("BG_USER_AGENT").unwrap_or_else(|_| bg_core::brand::DEFAULT_UA.to_string())
}

/// The shortest poll interval across the live source roster.
///
/// Read from the roster rather than hardcoded: this is a promise about our
/// request rate made to the people we poll, and adding one fast source must not
/// be able to turn it into a false one. Falls back to the slowest plausible
/// claim if the roster cannot be read — understating our politeness is the safe
/// direction to be wrong in.
async fn fastest_poll_interval(s: &ApiState) -> i32 {
    bg_db::sources::all(&s.db)
        .await
        .ok()
        .and_then(|rows| rows.iter().map(|r| r.poll_interval_s).min())
        .unwrap_or(300)
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
