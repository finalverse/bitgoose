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
//! So a request from a known unfurler gets a lean document: two or three
//! queries, no server render, no hydration bundle, roughly a sixth of the
//! bytes. The head is under 4 KB, which is all a crawler ever reads.
//!
//! ## It carries the whole story, and that is not a nicety
//!
//! The first version served a headline, two lines and a link out. Then a reader
//! tapped a shared link inside WeChat and got exactly that — because **WeChat's
//! in-app browser sends `MicroMessenger` just as its crawler does**, so a person
//! was matched as a bot and handed a stub where the article should have been.
//!
//! Two independent fixes, either of which alone prevents it. [`is_navigation`]
//! separates a person opening a page from something fetching it for a card, on
//! fetch metadata rather than on the user-agent. And this document now contains
//! the article, its sources and enough inline style to read on a phone — so
//! even a misjudged request lands on something worth reading.
//!
//! ## This is not cloaking
//!
//! Same headline, same standfirst, same picture, same canonical URL, and now
//! the same reporting. What is dropped is the navigation, the claim ledger and
//! the JavaScript. A reader who follows the link finds what the card promised,
//! which is the only test that matters.

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
    let named = headers
        .get(header::USER_AGENT)
        .and_then(|v| v.to_str().ok())
        .map(|ua| ua.to_lowercase())
        .is_some_and(|ua| UNFURLERS.iter().any(|u| ua.contains(u)));
    named && !is_navigation(headers)
}

/// Whether this is a person opening a page, rather than something fetching it
/// to draw a card.
///
/// **The user-agent alone cannot answer this**, and assuming it could shipped a
/// real regression: WeChat's in-app browser sends `MicroMessenger` in exactly
/// the same way its crawler does, so every reader who tapped a shared BitGoose
/// link inside WeChat was handed the crawler's stub — a headline, two lines and
/// a link out — instead of the article. The test that was supposed to catch
/// this checked Safari and Chrome, and missed the one case where a crawler and
/// a reader share a token.
///
/// Fetch metadata settles it. A browser navigating to a page sends
/// `Sec-Fetch-Dest: document` (or at minimum `Upgrade-Insecure-Requests`, which
/// predates it); a preview fetcher sends neither. Where both are absent, an
/// `Accept` header that asks for HTML *by preference* is the older signal —
/// crawlers overwhelmingly send `*/*`.
///
/// Erring toward "person" throughout: serving a reader the stub is a broken
/// page, while serving a crawler the full article only makes its preview slower.
fn is_navigation(h: &HeaderMap) -> bool {
    if h.get("sec-fetch-dest")
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.eq_ignore_ascii_case("document"))
    {
        return true;
    }
    if h.get("sec-fetch-mode")
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.eq_ignore_ascii_case("navigate"))
    {
        return true;
    }
    if h.contains_key("upgrade-insecure-requests") {
        return true;
    }
    // No fetch metadata at all. `Accept: text/html,...` with a q-list is what a
    // browser sends; `*/*` or an absent header is what a fetcher sends.
    h.get(header::ACCEPT)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|a| a.contains("text/html") && a.contains(','))
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
    /// The story itself, as HTML.
    ///
    /// Present because this document is not only read by crawlers. A person who
    /// taps a shared link inside an app's own browser can land here, and a
    /// headline over a "read this elsewhere" link is a broken page, not a fast
    /// one. What is dropped is the navigation, the claim ledger and the
    /// hydration bundle — not the reporting.
    pub body_html: String,
    /// Outlets behind the story, so the attribution survives too.
    pub sources: Vec<(String, String)>,
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
    // Enough style to be readable on a phone without fetching a stylesheet.
    // Inline and tiny: a second request would cost more than the rules are
    // worth, and this document exists because the network is slow.
    s.push_str(
        "<style>:root{color-scheme:light dark}\
body{margin:0 auto;padding:1.25rem;max-width:38rem;font:1.05rem/1.65 -apple-system,\
BlinkMacSystemFont,'Segoe UI',Roboto,sans-serif;color:#14181d;background:#fff;\
overflow-wrap:break-word}\
h1{font-size:1.6rem;line-height:1.25;margin:0 0 .5rem}\
.dek{font-size:1.1rem;color:#4a5560;margin:0 0 .75rem}\
.meta{font-size:.85rem;letter-spacing:.06em;text-transform:uppercase;color:#6b7480;margin:0 0 1rem}\
img{max-width:100%;height:auto;border-radius:6px}\
h2{font-size:1.15rem;margin:1.5rem 0 .4rem}\
a{color:#8a6200}\
.srcs{margin:1.25rem 0 0;padding:.9rem 0 0;border-top:1px solid #e4e0d8;font-size:.92rem}\
.srcs li{margin:.3rem 0}\
footer{margin-top:1.75rem;font-size:.85rem;color:#6b7480}\
@media(prefers-color-scheme:dark){body{color:#edeae3;background:#0b0d10}\
.dek{color:#9aa3ad}.meta,footer{color:#838c97}.srcs{border-color:#232a31}a{color:#f5b301}}\
</style>",
    );
    s.push_str("</head><body>");
    if !c.section.is_empty() {
        s.push_str(&format!("<p class=\"meta\">{}</p>", esc(&c.section)));
    }
    s.push_str(&format!("<h1>{title}</h1>"));
    if !desc.is_empty() {
        s.push_str(&format!("<p class=\"dek\">{desc}</p>"));
    }
    s.push_str(&format!(
        "<p><img src=\"{image}\" alt=\"{title}\" width=\"{w}\" height=\"{h}\"></p>"
    ));
    // The story itself. Its absence is what made this page useless to the
    // reader who tapped a shared link inside an app's own browser and got a
    // headline over a link out.
    if !c.body_html.is_empty() {
        s.push_str(&c.body_html);
    }
    if !c.sources.is_empty() {
        s.push_str("<h2>Sources</h2><ul class=\"srcs\">");
        for (name, href) in &c.sources {
            s.push_str(&format!(
                "<li><a href=\"{}\" rel=\"nofollow noopener\">{}</a></li>",
                esc(href),
                esc(name)
            ));
        }
        s.push_str("</ul>");
    }
    s.push_str(&format!(
        "<footer><a href=\"{url}\">Open on BitGoose</a> — every claim with the \
         outlets behind it.</footer>"
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
                body_html: String::new(),
                sources: Vec::new(),
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

    // Markdown to HTML, the same conversion the full page uses, so the two
    // cannot render the same story differently.
    let body_html = article
        .as_ref()
        .map(|a| crate::api::render_body(&a.body_md))
        .unwrap_or_default();
    let sources: Vec<(String, String)> = refs
        .iter()
        .map(|r| (r.name.clone(), r.url.clone()))
        .collect();

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
        body_html,
        sources,
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
            body_html: String::new(),
            sources: Vec::new(),
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
        // The head, which is all a crawler reads, must stay tiny — the real
        // page is 30 KB and takes 4.5 seconds over this link, and crawlers give
        // up in two.
        let d = document(&card());
        let head = d.find("</head>").expect("has a head");
        assert!(head < 4_000, "head grew to {head} bytes");
    }

    /// The regression that sent this back for a second pass: a reader who
    /// tapped a shared link inside WeChat's own browser was served the crawler
    /// document, and it had no article in it.
    #[test]
    fn the_page_carries_the_story_not_a_stub() {
        let mut c = card();
        c.body_html = "<p>The commission said it would proceed regardless.</p>".into();
        c.sources = vec![("Decrypt".into(), "https://decrypt.co/x".into())];
        let d = document(&c);
        assert!(d.contains("The commission said it would proceed regardless."));
        assert!(d.contains("Decrypt"));
        assert!(d.contains("https://decrypt.co/x"));
    }

    #[test]
    fn a_person_in_an_in_app_browser_is_not_a_crawler() {
        // WeChat's in-app browser and WeChat's crawler share a user-agent
        // token. Fetch metadata is what separates them, and getting this wrong
        // served every WeChat reader a stub.
        let wechat = "Mozilla/5.0 (iPhone; CPU iPhone OS 17_0 like Mac OS X) \
                      AppleWebKit/605.1.15 MicroMessenger/8.0.49 NetType/WIFI";
        let mut nav = ua(wechat);
        nav.insert("sec-fetch-dest", "document".parse().unwrap());
        assert!(!is_unfurler(&nav), "a WeChat reader was served the stub");

        let mut older = ua(wechat);
        older.insert("upgrade-insecure-requests", "1".parse().unwrap());
        assert!(!is_unfurler(&older));

        let mut accepts = ua(wechat);
        accepts.insert(
            header::ACCEPT,
            "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8"
                .parse()
                .unwrap(),
        );
        assert!(!is_unfurler(&accepts));

        // The crawler itself sends none of that.
        let mut bot = ua(wechat);
        bot.insert(header::ACCEPT, "*/*".parse().unwrap());
        assert!(
            is_unfurler(&bot),
            "the crawler must still get the fast path"
        );
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
