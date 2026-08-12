//! A small, fast document for the crawlers that build link previews.
//!
//! ## The metadata was never the problem
//!
//! A BitGoose story pasted into WeChat still rendered as a grey chain link and
//! the bare domain, after the Open Graph tags were correct, the images were on
//! our own domain and the card was the right shape. Measuring what a crawler
//! actually experiences explained why:
//!
//! ```text
//! 2s budget -> timeout
//! 3s budget -> timeout
//! 5s budget -> 200   (ttfb 2.8s, total 4.5s, 30 KB)
//! ```
//!
//! Link unfurlers are impatient — a couple of seconds is typical, and WeChat's
//! crawler reaches us from mainland China, which is slower still. The tags were
//! immaculate and nobody was ever reading them.
//!
//! Two costs, both avoidable. The full page runs seven queries and a complete
//! server render to produce 30 KB of article, sidebar, claim ledger and
//! hydration bundle — none of which a crawler wants. And it all has to arrive
//! over a link currently losing a large share of its packets.
//!
//! So a request from a known unfurler gets a document with the head and a short
//! body: one or two queries, no render, no hydration, and about a twentieth of
//! the bytes.
//!
//! ## This is not cloaking
//!
//! Same headline, same description, same picture, same canonical URL, pointing
//! at the same story. What is removed is the article body, the navigation and
//! the JavaScript — none of which is content a preview can show. The test is
//! whether a reader following the link finds what the card promised, and they
//! do; the body even carries the headline and standfirst as text, so a crawler
//! that ignores meta tags entirely still reads the same thing.
//!
//! A browser is never served this. If the user-agent is not a recognised
//! unfurler the request goes to the real page untouched.

use axum::{
    body::Body,
    extract::{Request, State},
    http::{header, HeaderMap, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// The agents that fetch a URL only to draw a card for it.
///
/// Matched as substrings, lowercased. Deliberately a list rather than a guess:
/// treating an unknown agent as a crawler would eventually serve a stub to a
/// reader, and the cost of missing one is only that its preview stays slow.
const UNFURLERS: &[&str] = &[
    "micromessenger", // WeChat
    "wxwork",         // WeCom
    "twitterbot",     // X
    "facebookexternalhit",
    "facebot",
    "linkedinbot",
    "slackbot",
    "slack-imgproxy",
    "discordbot",
    "telegrambot",
    "whatsapp",
    "skypeuripreview",
    "redditbot",
    "pinterest",
    "embedly",
    "quora link preview",
    "showyoubot",
    "outbrain",
    "vkshare",
    "w3c_validator",
    "applebot", // also used for Messages previews
    "bingpreview",
    "iframely",
    "opengraph",
    "qq",         // QQ's preview fetcher, and QQ browser's
    "bytespider", // Douyin / Toutiao
    "toutiaospider",
    "weibo",
];

pub fn is_unfurler(headers: &HeaderMap) -> bool {
    headers
        .get(header::USER_AGENT)
        .and_then(|v| v.to_str().ok())
        .map(|ua| ua.to_lowercase())
        .is_some_and(|ua| UNFURLERS.iter().any(|u| ua.contains(u)))
}

/// WeChat crops a preview to a small square; everyone else renders it wide.
fn wants_square(headers: &HeaderMap) -> bool {
    headers
        .get(header::USER_AGENT)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|ua| {
            let ua = ua.to_lowercase();
            ua.contains("micromessenger") || ua.contains("wxwork") || ua.contains("qq")
        })
}

#[derive(Clone, Default)]
pub struct UnfurlCache(Arc<Mutex<HashMap<String, Arc<String>>>>);

/// Past this many documents, drop the lot. They cost two queries to rebuild and
/// the working set is whatever is being shared right now.
const MAX_CACHED: usize = 512;

fn esc(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// Trim to a length a preview will actually show, on a word boundary.
///
/// WeChat shows roughly two lines and every platform truncates somewhere. Doing
/// it here means the cut lands between words rather than mid-syllable, and
/// keeps the document small.
fn clip(s: &str, max: usize) -> String {
    let s = s.trim();
    if s.chars().count() <= max {
        return s.to_string();
    }
    let cut: String = s.chars().take(max).collect();
    match cut.rsplit_once(' ') {
        Some((head, _)) if head.chars().count() > max / 2 => format!("{head}…"),
        _ => format!("{cut}…"),
    }
}

pub struct Card {
    pub title: String,
    pub description: String,
    pub url: String,
    pub image: String,
    pub square: bool,
    pub published: String,
    pub section: String,
}

/// Build the document. Kept separate from the handler so the shape of it can be
/// tested without a database.
pub fn document(c: &Card) -> String {
    let (w, h) = if c.square {
        ("800", "800")
    } else {
        ("1200", "630")
    };
    let twitter_card = if c.square {
        "summary"
    } else {
        "summary_large_image"
    };
    let (title, desc) = (esc(&c.title), esc(&c.description));
    let (url, image) = (esc(&c.url), esc(&c.image));

    // The meta tags come first, before anything else in the head. Several
    // crawlers read only the opening few kilobytes; on the full page `og:title`
    // sat at byte 1,976 behind the stylesheet and hydration preloads.
    let mut s = String::with_capacity(2048);
    s.push_str("<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\">");
    s.push_str(&format!("<title>{title} — BitGoose</title>"));
    s.push_str(&format!("<meta name=\"description\" content=\"{desc}\">"));
    s.push_str("<meta property=\"og:type\" content=\"article\">");
    s.push_str("<meta property=\"og:site_name\" content=\"BitGoose\">");
    s.push_str("<meta property=\"og:locale\" content=\"en\">");
    s.push_str(&format!("<meta property=\"og:title\" content=\"{title}\">"));
    s.push_str(&format!(
        "<meta property=\"og:description\" content=\"{desc}\">"
    ));
    s.push_str(&format!("<meta property=\"og:url\" content=\"{url}\">"));
    s.push_str(&format!("<meta property=\"og:image\" content=\"{image}\">"));
    s.push_str(&format!(
        "<meta property=\"og:image:secure_url\" content=\"{image}\">"
    ));
    s.push_str(&format!(
        "<meta property=\"og:image:width\" content=\"{w}\">"
    ));
    s.push_str(&format!(
        "<meta property=\"og:image:height\" content=\"{h}\">"
    ));
    s.push_str(&format!(
        "<meta property=\"og:image:alt\" content=\"{title}\">"
    ));
    s.push_str(&format!(
        "<meta name=\"twitter:card\" content=\"{twitter_card}\">"
    ));
    s.push_str(&format!(
        "<meta name=\"twitter:title\" content=\"{title}\">"
    ));
    s.push_str(&format!(
        "<meta name=\"twitter:description\" content=\"{desc}\">"
    ));
    s.push_str(&format!(
        "<meta name=\"twitter:image\" content=\"{image}\">"
    ));
    if !c.published.is_empty() {
        s.push_str(&format!(
            "<meta property=\"article:published_time\" content=\"{}\">",
            esc(&c.published)
        ));
    }
    if !c.section.is_empty() {
        s.push_str(&format!(
            "<meta property=\"article:section\" content=\"{}\">",
            esc(&c.section)
        ));
    }
    s.push_str("<meta property=\"article:publisher\" content=\"BitGoose\">");
    s.push_str(&format!("<link rel=\"canonical\" href=\"{url}\">"));
    s.push_str("<link rel=\"icon\" href=\"/favicon.ico\">");
    s.push_str("</head><body>");
    // The same words again as text, for the crawlers that read the body rather
    // than the head — WeChat among them, historically.
    s.push_str(&format!("<h1>{title}</h1><p>{desc}</p>"));
    s.push_str(&format!(
        "<p><img src=\"{image}\" alt=\"{title}\" width=\"{w}\" height=\"{h}\"></p>"
    ));
    s.push_str(&format!(
        "<p><a href=\"{url}\">Read this story on BitGoose</a></p>"
    ));
    s.push_str("</body></html>");
    s
}

/// Serve unfurlers the small document; pass everyone else through untouched.
pub async fn layer(
    State((db, cache)): State<(bg_db::Db, UnfurlCache)>,
    req: Request,
    next: Next,
) -> Response {
    if !is_unfurler(req.headers()) {
        return next.run(req).await;
    }
    let path = req.uri().path().to_string();
    let square = wants_square(req.headers());
    let key = format!("{path}|{}", if square { "s" } else { "w" });

    if let Some(doc) = cache.0.lock().ok().and_then(|c| c.get(&key).cloned()) {
        return html(doc);
    }
    let Some(card) = build(&db, &path, square).await else {
        // Not a page we can describe — a section front, an asset page, the
        // wire. Those still render fine; they are just not worth a special
        // case, so the real page answers.
        return next.run(req).await;
    };
    let doc = Arc::new(document(&card));
    if let Ok(mut c) = cache.0.lock() {
        if c.len() >= MAX_CACHED {
            c.clear();
        }
        c.insert(key, doc.clone());
    }
    html(doc)
}

fn html(doc: Arc<String>) -> Response {
    (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, "text/html; charset=utf-8"),
            // Short: a correction should reach a re-fetch the same day, and
            // crawlers re-check on their own schedule anyway.
            (header::CACHE_CONTROL, "public, max-age=900"),
            // Tells a shared cache that the body depends on who asked, so a
            // reader is never handed the crawler's copy.
            (header::VARY, "User-Agent"),
        ],
        (*doc).clone(),
    )
        .into_response()
}

async fn build(db: &bg_db::Db, path: &str, square: bool) -> Option<Card> {
    let base = std::env::var("BG_PUBLIC_BASE_URL")
        .unwrap_or_else(|_| format!("https://{}", bg_core::brand::DOMAIN));
    let base = base.trim_end_matches('/').to_string();

    let Some(slug) = path.strip_prefix("/story/") else {
        // The front page is shared too, and it was rendering as the bare
        // domain for exactly the same reason.
        if path == "/" || path.is_empty() {
            return Some(Card {
                title: bg_core::brand::NAME.to_string(),
                description: bg_core::brand::TAGLINE.to_string(),
                url: format!("{base}/"),
                image: format!("{base}/og-default.png"),
                square: false,
                published: String::new(),
                section: String::new(),
            });
        }
        return None;
    };
    let slug = slug.trim_end_matches('/');
    let story = bg_db::stories::published_by_slug(db, slug).await.ok()?;
    let article = bg_db::articles::latest_for_story(db, story.id)
        .await
        .ok()
        .flatten();

    let title = article
        .as_ref()
        .map(|a| a.headline.clone())
        .unwrap_or_else(|| story.title.clone());
    // Never blank. Roughly a quarter of published stories have neither a dek
    // nor a summary — the allowance does not stretch to summarising everything
    // the Wire carries — and each of those was sharing as a headline over an
    // empty space. `bg_core::share` falls back to who reported it.
    let refs = bg_db::stories::source_refs(db, story.id)
        .await
        .unwrap_or_default();
    let outlets: Vec<String> = refs.iter().map(|r| r.name.clone()).collect();
    let has_analysis = bg_db::analyses::for_story(db, story.id)
        .await
        .ok()
        .flatten()
        .is_some();
    let description = bg_core::share::description(
        article.as_ref().map(|a| a.dek.as_str()).unwrap_or(""),
        story.summary.as_deref().unwrap_or(""),
        &outlets,
        has_analysis,
    );

    // Our copy of the publisher's picture if we hold one, our own card
    // otherwise — never a hotlink, for the same reasons as the full page.
    let image = if crate::ogroute::mirrored(slug).is_some() {
        format!("{base}/img/{slug}")
    } else {
        crate::ogroute::warm(db.clone(), slug.to_string());
        format!("{base}/og/{slug}.png{}", if square { "?sq=1" } else { "" })
    };

    Some(Card {
        title: clip(&title, 110),
        description: clip(&description, 200),
        url: format!("{base}/story/{slug}"),
        image,
        square,
        published: story
            .published_at
            .map(|t| t.to_rfc3339())
            .unwrap_or_default(),
        section: story.category.label().to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ua(s: &str) -> HeaderMap {
        let mut h = HeaderMap::new();
        h.insert(header::USER_AGENT, s.parse().unwrap());
        h
    }

    #[test]
    fn readers_are_never_served_the_stub() {
        let iphone = "Mozilla/5.0 (iPhone; CPU iPhone OS 17_0 like Mac OS X) \
                      AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.0 Mobile/15E148 Safari";
        assert!(!is_unfurler(&ua(iphone)));
        assert!(!is_unfurler(&ua(
            "Mozilla/5.0 (Macintosh) Chrome/140.0 Safari/537.36"
        )));
        assert!(!is_unfurler(&HeaderMap::new()));
    }

    #[test]
    fn the_agents_that_draw_cards_are_recognised() {
        for a in [
            "Mozilla/5.0 (iPhone) MicroMessenger/8.0.49 NetType/WIFI Language/zh_CN",
            "Twitterbot/1.0",
            "facebookexternalhit/1.1",
            "LinkedInBot/1.0 (compatible; Mozilla/5.0)",
            "Slackbot-LinkExpanding 1.0",
            "TelegramBot (like TwitterBot)",
            "WhatsApp/2.23",
        ] {
            assert!(is_unfurler(&ua(a)), "missed {a}");
        }
    }

    #[test]
    fn wechat_gets_the_square_card_and_x_does_not() {
        assert!(wants_square(&ua("iPhone MicroMessenger/8.0.49")));
        assert!(!wants_square(&ua("Twitterbot/1.0")));
    }

    fn card() -> Card {
        Card {
            title: "SEC to address crypto regulations in absence of Clarity passage".into(),
            description: "The regulator said it would act on its own if the bill stalls.".into(),
            url: "https://bitgoose.com/story/sec-to-address-crypto-regulations".into(),
            image: "https://bitgoose.com/og/sec-to-address-crypto-regulations.png?sq=1".into(),
            square: true,
            published: "2026-08-11T09:00:00Z".into(),
            section: "Policy".into(),
        }
    }

    #[test]
    fn the_document_carries_everything_a_card_needs() {
        let d = document(&card());
        for want in [
            "og:title",
            "og:description",
            "og:image",
            "og:url",
            "twitter:card",
            "canonical",
        ] {
            assert!(d.contains(want), "missing {want}");
        }
        // …and the same words in the body, for crawlers that skip the head.
        assert!(d.contains("<h1>SEC to address"));
    }

    #[test]
    fn it_is_small_enough_to_arrive() {
        // The whole point. The real page is 30 KB and takes 4.5 seconds over
        // this link; crawlers give up in two.
        let d = document(&card());
        assert!(d.len() < 2_500, "stub grew to {} bytes", d.len());
    }

    #[test]
    fn the_tags_come_before_anything_that_could_be_truncated() {
        let d = document(&card());
        let title_at = d.find("og:title").unwrap();
        // Only `<title>` and the description precede it, so the bound moves
        // with how long a headline is. Anything under a kilobyte is inside the
        // opening chunk of every crawler that truncates; on the full page this
        // sat at byte 1,976 behind a stylesheet and two hydration preloads.
        assert!(title_at < 700, "og:title sits at byte {title_at}");
    }

    #[test]
    fn a_headline_with_markup_characters_cannot_break_the_document() {
        let mut c = card();
        c.title = r#"Fed & "the market" <script>alert(1)</script>"#.into();
        let d = document(&c);
        assert!(!d.contains("<script>"));
        assert!(d.contains("&lt;script&gt;"));
        assert!(d.contains("&amp;"));
        assert!(d.contains("&quot;"));
    }

    #[test]
    fn long_text_is_cut_between_words() {
        let s = "The Securities and Exchange Commission said on Monday that it would move \
                 ahead with its own rulemaking regardless of what the Senate decides";
        let c = clip(s, 60);
        assert!(c.chars().count() <= 61, "{c}");
        assert!(c.ends_with('…'));
        assert!(!c.contains("  "));
        // Cut at a space, so no word is left as a fragment.
        let body = c.trim_end_matches('…');
        assert!(s.starts_with(body), "clip invented text: {c}");
        assert!(!body.ends_with(' '));
    }

    #[test]
    fn short_text_is_left_alone() {
        assert_eq!(clip("Bitcoin tops $65,000", 60), "Bitcoin tops $65,000");
    }
}
