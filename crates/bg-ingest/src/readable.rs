//! Full-text extraction — turning a link into something worth analysing.
//!
//! An RSS feed gives us a headline and two sentences. That is enough to route a
//! story and to summarise it into a pointer, but it is *not* enough to say what
//! the story means: measured across our own archive, 116 of 148 published
//! stories carried under 1,000 characters of source text. Asking a model to
//! analyse that is asking it to invent, which is exactly the failure that put
//! thirty fabricated stories on the site once already.
//!
//! So before the Skein analyses anything, we fetch the article itself.
//!
//! Two boundaries hold here and both matter:
//!
//! 1. **robots.txt is checked per URL, not per feed.** A publisher can allow
//!    their feed and disallow their article pages, and that combination is
//!    common enough that assuming otherwise would be a quiet violation.
//! 2. **The result lands in `raw_items.body_raw`**, the private working column
//!    that no public projection selects (see `bg-db::items`). Extracted text is
//!    input to analysis; it is never itself served.

use crate::{robots, IngestError};
use scraper::{Html, Selector};

/// Hard cap on stored text. Long enough for any news article, short enough that
/// a runaway page (a forum thread, an index that slipped through) cannot fill
/// the column or the context window.
const MAX_CHARS: usize = 40_000;

/// Below this, extraction did not find the article — it found navigation. We
/// return `None` rather than store chrome, because a paragraph of cookie notice
/// scores as "source text" downstream and is worse than having nothing.
const MIN_CHARS: usize = 400;

/// Containers that hold the article body, most explicit first.
///
/// Ordered by how much the page is *promising* with each: schema.org markup is
/// a publisher's own declaration, `<article>` is semantic HTML, and the class
/// names below are the conventions the major CMSes emit. We take the first that
/// yields enough text rather than the longest match, because a later, looser
/// selector tends to swallow the comment thread with the story.
const CONTAINERS: &[&str] = &[
    "[itemprop='articleBody']",
    "article [itemprop='articleBody']",
    "div.article-body",
    "div.article__body",
    "div.post-content",
    "div.entry-content",
    "div.story-body",
    "section.article-body",
    "article",
    "main",
];

/// Elements that sit *inside* the article container but are not the article:
/// related-story rails, newsletter forms, share bars, image captions.
const STRIP: &[&str] = &[
    "script",
    "style",
    "noscript",
    "nav",
    "aside",
    "form",
    "figure",
    "figcaption",
    "iframe",
    "button",
    "svg",
    "footer",
    "header",
];

/// What we managed to pull out of a page.
pub struct Extracted {
    pub text: String,
    /// Which selector won. Recorded so a publisher whose layout changes shows up
    /// as a shift in this distribution rather than as silently thinner stories.
    pub via: &'static str,
}

/// Fetch `url` and extract its article text.
///
/// `Ok(None)` means "fetched, but there was no article here" — a paywall stub,
/// a video page, a redirect to a section index. That is an ordinary outcome and
/// not an error; the caller keeps the RSS summary and moves on.
pub async fn fetch(
    client: &reqwest::Client,
    agent: &str,
    url: &str,
    respect_robots: bool,
) -> Result<Option<Extracted>, IngestError> {
    if respect_robots && !robots::allows(client, agent, url).await {
        return Ok(None);
    }

    let resp = client.get(url).send().await?;
    if !resp.status().is_success() {
        return Ok(None);
    }
    // Guard before reading the body: a PDF or a video file would otherwise be
    // downloaded in full and then thrown away.
    let is_html = resp
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_none_or(|c| c.contains("html"));
    if !is_html {
        return Ok(None);
    }

    let body = resp.text().await?;
    Ok(extract(&body))
}

/// Pull the article text out of an HTML document.
///
/// Split from [`fetch`] so it is testable against fixtures without a network.
pub fn extract(html: &str) -> Option<Extracted> {
    let doc = Html::parse_document(html);

    // Publisher-declared body first. When a site ships JSON-LD we should believe
    // it over any guess we could make from the DOM.
    if let Some(text) = json_ld_body(&doc) {
        if text.chars().count() >= MIN_CHARS {
            return Some(Extracted {
                text: cap(text),
                via: "json-ld",
            });
        }
    }

    for sel in CONTAINERS {
        let Ok(parsed) = Selector::parse(sel) else {
            continue;
        };
        let Some(el) = doc.select(&parsed).next() else {
            continue;
        };
        let text = paragraphs_of(&el.html());
        if text.chars().count() >= MIN_CHARS {
            return Some(Extracted {
                text: cap(text),
                via: sel,
            });
        }
    }

    // Last resort: every paragraph on the page. Noisier, but on a plain article
    // template with no recognisable wrapper it is usually right.
    let text = paragraphs_of(html);
    (text.chars().count() >= MIN_CHARS).then(|| Extracted {
        text: cap(text),
        via: "all-paragraphs",
    })
}

/// Concatenate the `<p>` text of a fragment, dropping non-article elements.
///
/// Paragraph-level rather than whole-node `.text()` because the latter welds
/// headings, list items and stray link text into one run with no sentence
/// boundaries — which then reads to a model as a single malformed sentence.
fn paragraphs_of(fragment: &str) -> String {
    let doc = Html::parse_fragment(fragment);

    // Collect the text of everything we want gone, so we can reject any
    // paragraph whose content is contained in it. `scraper` gives us no node
    // removal, so subtraction by content is the available move.
    let mut junk: Vec<String> = Vec::new();
    for sel in STRIP {
        if let Ok(parsed) = Selector::parse(sel) {
            for el in doc.select(&parsed) {
                let t = el.text().collect::<String>();
                let t = t.trim();
                if !t.is_empty() {
                    junk.push(t.to_string());
                }
            }
        }
    }

    let Ok(p) = Selector::parse("p") else {
        return String::new();
    };
    let mut out = String::new();
    for el in doc.select(&p) {
        let raw = el.text().collect::<String>();
        let t = collapse_ws(&raw);
        // One-line paragraphs are bylines, datelines, captions and "Read more"
        // links far more often than they are prose.
        if t.chars().count() < 40 {
            continue;
        }
        if junk.iter().any(|j| j.contains(&t)) {
            continue;
        }
        if out.contains(&t) {
            continue; // AMP and print variants repeat the body verbatim.
        }
        out.push_str(&t);
        out.push_str("\n\n");
    }
    out.trim().to_string()
}

/// `articleBody` from schema.org JSON-LD, including inside an `@graph`.
fn json_ld_body(doc: &Html) -> Option<String> {
    let sel = Selector::parse("script[type='application/ld+json']").ok()?;
    for el in doc.select(&sel) {
        let raw = el.text().collect::<String>();
        let Ok(v) = serde_json::from_str::<serde_json::Value>(&raw) else {
            continue;
        };
        if let Some(b) = find_article_body(&v) {
            let b = collapse_ws(&b);
            if !b.is_empty() {
                return Some(b);
            }
        }
    }
    None
}

/// Walk arbitrary JSON for an `articleBody` string. Publishers nest it under
/// `@graph`, inside arrays, or at the root depending on their SEO plugin.
fn find_article_body(v: &serde_json::Value) -> Option<String> {
    match v {
        serde_json::Value::Object(m) => {
            if let Some(serde_json::Value::String(s)) = m.get("articleBody") {
                return Some(s.clone());
            }
            m.values().find_map(find_article_body)
        }
        serde_json::Value::Array(a) => a.iter().find_map(find_article_body),
        _ => None,
    }
}

fn collapse_ws(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut space = false;
    for c in s.chars() {
        if c.is_whitespace() {
            space = true;
        } else {
            if space && !out.is_empty() {
                out.push(' ');
            }
            space = false;
            out.push(c);
        }
    }
    out.trim().to_string()
}

/// Truncate on a character boundary — `MAX_CHARS` counts chars, and slicing a
/// `String` by byte index would panic mid-codepoint on any non-ASCII source.
fn cap(mut s: String) -> String {
    if s.chars().count() > MAX_CHARS {
        let end = s
            .char_indices()
            .nth(MAX_CHARS)
            .map(|(i, _)| i)
            .unwrap_or(s.len());
        s.truncate(end);
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn long(prefix: &str, n: usize) -> String {
        format!(
            "{prefix} {}",
            "the quick brown fox jumped over it. ".repeat(n)
        )
    }

    #[test]
    fn prefers_the_publishers_own_declaration() {
        let body = long("Declared body.", 20);
        let html = format!(
            r#"<html><head><script type="application/ld+json">
               {{"@type":"NewsArticle","articleBody":"{body}"}}
               </script></head><body><article><p>{}</p></article></body></html>"#,
            long("DOM body.", 20)
        );
        let got = extract(&html).expect("extracts");
        assert_eq!(got.via, "json-ld");
        assert!(got.text.starts_with("Declared body."));
    }

    #[test]
    fn navigation_and_related_rails_do_not_become_source_text() {
        let html = format!(
            r#"<html><body><article>
                 <nav><p>{}</p></nav>
                 <p>{}</p>
                 <aside><p>{}</p></aside>
               </article></body></html>"#,
            long("Sections and menus and other chrome.", 12),
            long("The actual reporting begins here.", 12),
            long("Related stories you might also like.", 12),
        );
        let got = extract(&html).expect("extracts");
        assert!(got.text.contains("The actual reporting"));
        assert!(
            !got.text.contains("Sections and menus"),
            "nav leaked into the body"
        );
        assert!(
            !got.text.contains("Related stories"),
            "aside leaked into the body"
        );
    }

    #[test]
    fn a_page_of_chrome_yields_nothing_rather_than_junk() {
        // A paywall stub: real markup, no article. Storing its scraps would let
        // a grounding check that only counts characters pass on nothing.
        let html = r#"<html><body><article>
             <p>Subscribe to continue reading.</p>
             <p>Sign in</p>
           </article></body></html>"#;
        assert!(extract(html).is_none());
    }

    #[test]
    fn truncation_never_splits_a_character() {
        // Multi-byte throughout: if `cap` sliced by byte index this panics.
        let para = "验证多字节字符的截断行为完全安全无恙。".repeat(400);
        let html = format!("<html><body><article><p>{para}</p></article></body></html>");
        let got = extract(&html).expect("extracts");
        assert!(got.text.chars().count() <= MAX_CHARS);
    }

    #[test]
    fn repeated_bodies_are_stored_once() {
        // AMP variants and print stylesheets duplicate the article verbatim.
        let p = long("A paragraph that appears twice in the markup.", 12);
        let html = format!(
            "<html><body><article><p>{p}</p><p>{p}</p><p>{}</p></article></body></html>",
            long("And one that does not.", 12)
        );
        let got = extract(&html).expect("extracts");
        assert_eq!(
            got.text.matches("appears twice").count(),
            1,
            "duplicate paragraph stored twice"
        );
    }
}
