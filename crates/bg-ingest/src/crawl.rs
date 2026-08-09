//! Crawling a publisher's index page directly, with no feed involved.
//!
//! Until now every source had to hand us an RSS feed. That is a dependency on
//! other people's infrastructure choices: plenty of publications worth reading
//! have no feed, offer a truncated one, or quietly stop maintaining theirs. A
//! newsroom that can only read what is pushed to it is not independent.
//!
//! So this reads the front page and follows the links, the way a person would.
//! Same downstream pipeline — the items it produces are indistinguishable from
//! feed items once ingested, which is the point: independence is an ingestion
//! concern, not something the rest of the system should have to know about.
//!
//! **robots.txt still governs, and that is not a limitation to route around.**
//! A crawler that ignores it is the reason publishers stop allowing crawlers at
//! all. Sites that actively block automated access — Cloudflare bot walls,
//! `Disallow: /` — are not sources, and defeating that is not a feature.
//!
//! What this *does* remove is the dependency on a publisher choosing to emit
//! XML. Any site whose robots.txt permits reading its index can now be a
//! source.

use crate::{canonical, http, robots, IngestError};
use scraper::{Html, Selector};
use std::collections::HashSet;

/// A headline found on an index page.
pub struct Found {
    pub url: String,
    pub title: String,
}

/// Shortest plausible headline. Below this it is navigation — "More", "Latest",
/// a category name — which links to a listing rather than an article.
const MIN_TITLE: usize = 25;

/// Anchors whose href is clearly not an article.
fn is_navigation(href: &str) -> bool {
    const SKIP: &[&str] = &[
        "/tag/",
        "/tags/",
        "/category/",
        "/categories/",
        "/author/",
        "/authors/",
        "/page/",
        "/about",
        "/contact",
        "/privacy",
        "/terms",
        "/subscribe",
        "/newsletter",
        "/login",
        "/signin",
        "/register",
        "/search",
        "/rss",
        "/feed",
        "/sitemap",
        "javascript:",
        "mailto:",
        "#",
    ];
    let lower = href.to_lowercase();
    SKIP.iter().any(|s| lower.contains(s)) || lower.trim_end_matches('/').is_empty()
}

/// An article URL usually has a slug — several words joined by hyphens, or a
/// numeric id. A bare `/markets` is a section.
fn looks_like_article(url: &str) -> bool {
    let path = url
        .split_once("://")
        .map(|(_, rest)| rest.split_once('/').map(|(_, p)| p).unwrap_or(""))
        .unwrap_or(url);
    let last = path.trim_end_matches('/').rsplit('/').next().unwrap_or("");
    // Two hyphens means at least three words, which no section name has.
    last.matches('-').count() >= 2 || last.chars().filter(|c| c.is_ascii_digit()).count() >= 6
}

/// Read an index page and return the article links on it.
///
/// `selector` narrows the search when a site's markup needs it; empty means
/// every anchor on the page, filtered by the heuristics above. Starting broad
/// and filtering works on more sites than a per-publisher selector does, and
/// degrades to "nothing found" rather than to wrong content.
pub async fn index(
    client: &reqwest::Client,
    agent: &str,
    index_url: &str,
    selector: Option<&str>,
    respect_robots: bool,
    max: usize,
) -> Result<Vec<Found>, IngestError> {
    if respect_robots && !robots::allows(client, agent, index_url).await {
        return Ok(Vec::new());
    }

    let resp = client.get(index_url).send().await?;
    if !resp.status().is_success() {
        return Ok(Vec::new());
    }
    let html = resp.text().await?;
    Ok(links(&html, index_url, selector, max))
}

/// Extract article links from index markup. Split out so it is testable
/// against fixtures without a network.
pub fn links(html: &str, base: &str, selector: Option<&str>, max: usize) -> Vec<Found> {
    let doc = Html::parse_document(html);
    let scope = selector
        .and_then(|s| Selector::parse(s).ok())
        .unwrap_or_else(|| Selector::parse("body").unwrap());
    let Ok(anchor) = Selector::parse("a[href]") else {
        return Vec::new();
    };

    let mut seen: HashSet<String> = HashSet::new();
    let mut out = Vec::new();

    for region in doc.select(&scope) {
        for a in region.select(&anchor) {
            let Some(href) = a.value().attr("href") else {
                continue;
            };
            if is_navigation(href) {
                continue;
            }
            let title = a.text().collect::<String>();
            let title = title.split_whitespace().collect::<Vec<_>>().join(" ");
            if title.chars().count() < MIN_TITLE {
                continue;
            }
            let Some(abs) = absolutise(href, base) else {
                continue;
            };
            if !looks_like_article(&abs) {
                continue;
            }
            // Canonicalise before de-duplicating, or the same article arrives
            // twice under two tracking parameters.
            let url = canonical::canonicalize(&abs);
            if !seen.insert(url.clone()) {
                continue;
            }
            out.push(Found { url, title });
            if out.len() >= max {
                return out;
            }
        }
    }
    out
}

/// Resolve a possibly-relative href against the page it was found on.
fn absolutise(href: &str, base: &str) -> Option<String> {
    if href.starts_with("http://") || href.starts_with("https://") {
        return Some(href.to_string());
    }
    let (scheme, rest) = base.split_once("://")?;
    let host = rest.split('/').next()?;
    if let Some(path) = href.strip_prefix('/') {
        return Some(format!("{scheme}://{host}/{path}"));
    }
    // A relative path without a leading slash, resolved against the directory.
    let dir = base.rsplit_once('/').map(|(d, _)| d).unwrap_or(base);
    Some(format!("{dir}/{href}"))
}

/// A conditional GET for index pages.
///
/// Reuses the same validators as feed polling, so a publisher who supports
/// `If-None-Match` sees the same good-citizen behaviour from the crawler as
/// from the feed reader.
pub async fn index_conditional(
    client: &reqwest::Client,
    url: &str,
    etag: Option<&str>,
    last_modified: Option<&str>,
) -> Result<http::Fetched, IngestError> {
    http::conditional_get(client, url, etag, last_modified).await
}

#[cfg(test)]
mod tests {
    use super::*;

    const PAGE: &str = r#"
      <html><body>
        <nav><a href="/markets">Markets section navigation here</a></nav>
        <main>
          <a href="/2026/08/senate-clears-clarity-act-vote">Senate clears the Clarity Act for a floor vote</a>
          <a href="https://other.example/news/chip-export-rules-tighten">Chip export rules tighten again this quarter</a>
          <a href="/tag/bitcoin">Bitcoin coverage and analysis archive</a>
          <a href="/subscribe">Subscribe to our daily newsletter today</a>
          <a href="/2026/08/senate-clears-clarity-act-vote?utm_source=rss">Senate clears the Clarity Act for a floor vote</a>
          <a href="/x">Short</a>
        </main>
      </body></html>"#;

    #[test]
    fn articles_are_found_and_navigation_is_not() {
        let found = links(PAGE, "https://example.com/news", None, 20);
        let urls: Vec<&str> = found.iter().map(|f| f.url.as_str()).collect();
        assert!(
            urls.iter().any(|u| u.contains("clarity-act")),
            "article missed: {urls:?}"
        );
        assert!(
            urls.iter().any(|u| u.contains("chip-export-rules")),
            "absolute-href article missed: {urls:?}"
        );
        assert!(
            !urls.iter().any(|u| u.contains("/tag/")),
            "tag page treated as an article: {urls:?}"
        );
        assert!(
            !urls.iter().any(|u| u.contains("subscribe")),
            "subscribe link treated as an article: {urls:?}"
        );
    }

    #[test]
    fn the_same_article_under_a_tracking_parameter_is_one_item() {
        // Index pages routinely link the same piece twice, once with campaign
        // parameters. Two rows for one article is a duplicate on the site.
        let found = links(PAGE, "https://example.com/news", None, 20);
        let clarity = found
            .iter()
            .filter(|f| f.url.contains("clarity-act"))
            .count();
        assert_eq!(clarity, 1, "tracking parameter created a duplicate");
    }

    #[test]
    fn a_section_link_is_not_an_article() {
        // "/markets" has no slug shape; a section page ingested as a story
        // would be a headline with no article behind it.
        assert!(!looks_like_article("https://example.com/markets"));
        assert!(looks_like_article(
            "https://example.com/2026/08/a-real-headline-here"
        ));
        assert!(looks_like_article("https://example.com/news/1234567"));
    }

    #[test]
    fn relative_hrefs_resolve_against_the_page_they_were_found_on() {
        assert_eq!(
            absolutise("/a/b", "https://example.com/news/index.html").as_deref(),
            Some("https://example.com/a/b")
        );
        assert_eq!(
            absolutise("https://x.example/y", "https://example.com/").as_deref(),
            Some("https://x.example/y")
        );
    }

    #[test]
    fn a_selector_narrows_the_search() {
        // Without scoping, a sidebar of "most read" links pollutes every crawl.
        let found = links(PAGE, "https://example.com/news", Some("nav"), 20);
        assert!(
            found.is_empty(),
            "nav contains no articles, so a nav-scoped crawl should find none: {:?}",
            found.iter().map(|f| &f.url).collect::<Vec<_>>()
        );
    }
}
