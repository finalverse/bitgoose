//! Shared components.

use crate::model::*;
use leptos::prelude::*;
use leptos_meta::Html;
use leptos_router::components::A;
use leptos_router::hooks::use_location;

fn edition_code(path: &str) -> &'static str {
    if path == "/zh-hant" || path.starts_with("/zh-hant/") {
        "zh-hant"
    } else if path == "/zh" || path.starts_with("/zh/") {
        "zh"
    } else if path == "/fr" || path.starts_with("/fr/") {
        "fr"
    } else if path == "/es" || path.starts_with("/es/") {
        "es"
    } else if path == "/ja" || path.starts_with("/ja/") {
        "ja"
    } else if path == "/ko" || path.starts_with("/ko/") {
        "ko"
    } else {
        "en"
    }
}

/// Resolve an edition from either a route path or an absolute canonical URL.
/// English deliberately lives at the root, so an unprefixed path is English.
fn edition_from_url(url: &str) -> &'static str {
    let path = url
        .split_once("://")
        .and_then(|(_, rest)| rest.find('/').map(|at| &rest[at..]))
        .unwrap_or(url);
    edition_code(path)
}

fn edition_prefix(language: &str) -> &'static str {
    match language {
        "zh" => "/zh",
        "zh-hant" => "/zh-hant",
        "fr" => "/fr",
        "es" => "/es",
        "ja" => "/ja",
        "ko" => "/ko",
        _ => "",
    }
}

fn switch_edition(path: &str, target: &str) -> String {
    let current = edition_code(path);
    let rest = if current == "en" {
        path
    } else {
        path.strip_prefix(edition_prefix(current)).unwrap_or(path)
    };
    let rest = if rest.is_empty() { "/" } else { rest };
    if target == "en" {
        rest.to_string()
    } else if rest == "/" {
        edition_prefix(target).to_string()
    } else {
        format!("{}{}", edition_prefix(target), rest)
    }
}

/// BitGoose's gate-and-beacon mark.
#[component]
pub fn GooseMark(#[prop(default = 26)] size: u32) -> impl IntoView {
    view! {
        <img
            src="/favicon.svg"
            width=size
            height=size
            alt=""
            aria-hidden="true"
            class="brand-mark"
        />
    }
}

#[component]
pub fn Masthead() -> impl IntoView {
    let location = use_location();
    let pathname = location.pathname;
    let language = Memo::new(move |_| edition_code(&pathname.get()));
    let edition_href =
        move |path: &'static str| move || format!("{}{path}", edition_prefix(language.get()));
    let switch_href = move |target: &'static str| move || switch_edition(&pathname.get(), target);
    view! {
        <Html {..} lang=move || match language.get() {
            "zh" => "zh-CN",
            "zh-hant" => "zh-Hant",
            "fr" => "fr",
            "es" => "es",
            "ja" => "ja",
            "ko" => "ko",
            _ => "en",
        } />
        <header class="masthead">
            <div class="shell">
                <A href=move || edition_prefix(language.get()).to_string() attr:class="brand">
                    <GooseMark size=34 />
                    <span>
                        <span class="brand-bit">"Bit"</span>
                        <span class="brand-goose">"Goose"</span>
                    </span>
                </A>
                // The desk switcher sits ahead of the sections and is styled
                // apart from them, because choosing a beat is a different kind
                // of decision from choosing a section — it changes what the
                // whole site is about, not which slice of it you are reading.
                <nav class="desks" aria-label="News desks">
                    <A href=edition_href("/ai") attr:class="desk-link">{move || match language.get() { "zh" | "zh-hant" => "人工智能", "fr" => "IA", "es" => "IA", _ => "AI" }}</A>
                    <A href=edition_href("/crypto") attr:class="desk-link">{move || match language.get() { "zh" => "加密市场", "zh-hant" => "加密市場", "ja" => "暗号資産", "ko" => "가상자산", "es" => "Cripto", _ => "Crypto" }}</A>
                    <A href=edition_href("/markets") attr:class="desk-link">{move || match language.get() { "zh" => "市场", "zh-hant" => "市場", "fr" => "Marchés", "es" => "Mercados", "ja" => "市場", "ko" => "시장", _ => "Markets" }}</A>
                    <A href=edition_href("/tech") attr:class="desk-link">{move || match language.get() { "zh" | "zh-hant" => "科技", "fr" => "Technologie", "es" => "Tecnología", "ja" => "テクノロジー", "ko" => "기술", _ => "Tech" }}</A>
                    <A href=edition_href("/world") attr:class="desk-link">{move || match language.get() { "zh" => "世界", "zh-hant" => "世界", "fr" => "Monde", "es" => "Mundo", "ja" => "国際", "ko" => "국제", _ => "World" }}</A>
                    <A href=edition_href("/science") attr:class="desk-link">{move || match language.get() { "zh" => "科学", "zh-hant" => "科學", "fr" => "Sciences", "es" => "Ciencia", "ja" => "科学", "ko" => "과학", _ => "Science" }}</A>
                    <A href=edition_href("/culture") attr:class="desk-link">{move || match language.get() { "zh" | "zh-hant" => "文化", "fr" => "Culture", "es" => "Cultura", "ja" => "文化", "ko" => "문화", _ => "Culture" }}</A>
                    // Seven desks fit across a laptop; the newsroom files under
                    // twenty-three categories, and the other sixteen were
                    // reachable only from a chip row part-way down /desk. A
                    // reader who wants Energy or Space should not have to know
                    // that. `<details>` rather than a scripted menu: it opens
                    // without JavaScript, closes on Escape, and is reachable by
                    // keyboard for free — and this bar renders server-side
                    // before hydration, so anything needing JS would be dead
                    // for the first moments of every page load.
                    <details class="desk-more">
                        <summary class="desk-link" aria-label="All sections">
                            {move || match language.get() { "zh" => "更多", "zh-hant" => "更多", "fr" => "Plus", "es" => "Más", "ja" => "その他", "ko" => "더보기", _ => "More" }}
                        </summary>
                        <div class="desk-more-panel">
                            {bg_core::domain::Category::ALL
                                .iter()
                                .map(|c| {
                                    view! {
                                        <a
                                            class="desk-more-link"
                                            href=move || format!("{}/section/{}", edition_prefix(language.get()), c.as_str())
                                        >
                                            {move || match language.get() {
                                                "zh" => c.label_zh(),
                                                "zh-hant" => c.label_zh_hant(),
                                                "fr" => c.label_fr(),
                                                "es" => c.label_es(),
                                                "ja" => c.label_ja(),
                                                "ko" => c.label_ko(),
                                                _ => c.label(),
                                            }}
                                        </a>
                                    }
                                })
                                .collect_view()}
                        </div>
                    </details>
                </nav>
                <nav class="nav" aria-label="Sections">
                    <A href=edition_href("/desk")>{move || match language.get() { "zh" => "原创", "zh-hant" => "原創", "fr" => "Dossiers", "es" => "Análisis", "ja" => "独自報道", "ko" => "자체 보도", _ => "Desk" }}</A>
                    <A href=edition_href("/wire")>{move || match language.get() { "zh" => "快讯", "zh-hant" => "快訊", "fr" => "Fil", "es" => "Últimas", "ja" => "速報", "ko" => "속보", _ => "Wire" }}</A>
                    <A href=edition_href("/flyway")>{move || match language.get() { "zh" => "专题", "zh-hant" => "專題", "fr" => "Sujets", "es" => "Temas", "ja" => "特集", "ko" => "특집", _ => "Topics" }}</A>
                    <A href=edition_href("/flock")>{move || match language.get() { "zh" => "AI 编辑部", "zh-hant" => "AI 編輯部", "ja" => "AI編集部", "ko" => "AI 편집국", _ => "Agents" }}</A>
                    <A href=edition_href("/standards")>{move || match language.get() { "zh" => "编辑标准", "zh-hant" => "編輯標準", "ja" => "編集基準", "ko" => "편집 기준", _ => "Standards" }}</A>
                </nav>
                <div class="masthead-right">
                    <nav class="edition-switcher" aria-label="Language editions">
                        <A href=switch_href("en") attr:class=move || if language.get() == "en" { "language-switch active" } else { "language-switch" } >"EN"</A>
                        <A href=switch_href("zh") attr:class=move || if language.get() == "zh" { "language-switch active" } else { "language-switch" } >"简中"</A>
                        <A href=switch_href("zh-hant") attr:class=move || if language.get() == "zh-hant" { "language-switch active" } else { "language-switch" } >"繁中"</A>
                        <A href=switch_href("fr") attr:class=move || if language.get() == "fr" { "language-switch active" } else { "language-switch" } >"FR"</A>
                        <A href=switch_href("es") attr:class=move || if language.get() == "es" { "language-switch active" } else { "language-switch" } >"ES"</A>
                        <A href=switch_href("ja") attr:class=move || if language.get() == "ja" { "language-switch active" } else { "language-switch" } >"日本語"</A>
                        <A href=switch_href("ko") attr:class=move || if language.get() == "ko" { "language-switch active" } else { "language-switch" } >"한국어"</A>
                    </nav>
                    <ThemeToggle />
                </div>
            </div>
        </header>
    }
}

/// Theme switch.
///
/// Writes `data-theme` on `<html>`, which the stylesheet's
/// `:root[data-theme=...]` rules use to override the media query — so an
/// explicit choice always beats the OS preference, in both directions.
#[component]
pub fn ThemeToggle() -> impl IntoView {
    let toggle = move |_| {
        if let Some(root) = document().document_element() {
            // Before any explicit choice there is no attribute to read, so fall
            // back to what the OS is actually showing. Reading the attribute
            // alone would make the first click a no-op for a reader already in
            // light mode: it would "switch" them to the light they were in.
            let showing_light = match root.get_attribute("data-theme").as_deref() {
                Some("light") => true,
                Some("dark") => false,
                _ => window()
                    .match_media("(prefers-color-scheme: light)")
                    .ok()
                    .flatten()
                    .is_some_and(|m| m.matches()),
            };
            let next = if showing_light { "dark" } else { "light" };
            let _ = root.set_attribute("data-theme", next);
            // Persist so the choice survives a reload; the inline script in the
            // document head reapplies it before first paint.
            if let Ok(Some(store)) = window().local_storage() {
                let _ = store.set_item("bg-theme", next);
            }
        }
    };
    view! {
        <button
            class="theme-toggle"
            on:click=toggle
            aria-label="Toggle colour theme"
            title="Toggle theme"
        >
            "◐"
        </button>
    }
}

#[component]
pub fn Footer() -> impl IntoView {
    view! {
        <footer class="footer">
            <div class="shell">
                <div class="footer-grid">
                    <div>
                        <div class="brand" style="font-size:1.15rem;margin-bottom:.6rem">
                            <GooseMark size=20 />
                            <span>
                                <span class="brand-bit">"Bit"</span>
                                <span class="brand-goose">"Goose"</span>
                            </span>
                        </div>
                        <p style="margin:0;max-width:26rem;line-height:1.6">
                            "Seven independently edited AI newsrooms covering crypto markets,
                             artificial intelligence and frontier technology."
                        </p>
                    </div>
                    <div>
                        <h4>"Read"</h4>
                        <ul>
                            <li><A href="/ai">"AI"</A></li>
                            <li><A href="/crypto">"Crypto"</A></li>
                            <li><A href="/markets">"Markets"</A></li>
                            <li><A href="/tech">"Technology"</A></li>
                            <li><A href="/flyway">"Topics"</A></li>
                        </ul>
                    </div>
                    <div>
                        <h4>"Newsroom"</h4>
                        <ul>
                            <li><A href="/flock">"The Flock"</A></li>
                            <li><A href="/standards">"Standards"</A></li>
                            <li><A href="/standards">"Corrections"</A></li>
                        </ul>
                    </div>
                    <div>
                        <h4>"Build"</h4>
                        <ul>
                            <li><A href="/developers">"API"</A></li>
                            <li><a href="/v1" class="out">"REST"</a></li>
                            <li><a href="/openapi.json" class="out">"OpenAPI"</a></li>
                        </ul>
                    </div>
                </div>
                <div class="disclosure">
                    <span>{bg_core::brand::AI_DISCLOSURE}</span>
                    <span>"Every claim links to sources; copyrighted articles are never republished in full."</span>
                </div>
            </div>
        </footer>
    }
}

/// Everything a link preview needs, in one place.
///
/// X, WeChat, Slack, Telegram and iMessage all read some subset of Open Graph
/// and Twitter cards, and the subsets disagree — so the safe move is to emit
/// the union rather than guess. Having one component own it also fixes a real
/// bug: the app shell emitted a site-wide `description` and story pages emitted
/// their own, leaving two in the document. A crawler takes the first, so every
/// shared story was described with the generic site blurb.
///
/// `image` falls back to a branded 1200x630 card served from our own domain.
/// That size is what X and WeChat crop a *large* card from — smaller images get
/// a thumbnail instead — and serving it ourselves means a story with no
/// photograph still shares as something rather than a bare link.
/// The width a URL claims for itself, from a `width=` or `w=` query parameter.
///
/// Image CDNs (Reddit, WordPress, Cloudinary, most Next.js sites) encode the
/// rendered size in the URL, which is the only way to know how big a remote
/// image is without fetching it. `None` means the URL says nothing — we assume
/// it is fine rather than discarding every image that lacks the hint.
fn declared_width(url: &str) -> Option<u32> {
    // A query parameter, as most image CDNs use.
    if let Some((_, query)) = url.split_once('?') {
        if let Some(w) = query.split('&').find_map(|pair| {
            let (k, v) = pair.split_once('=')?;
            matches!(k, "width" | "w").then(|| v.parse().ok())?
        }) {
            return Some(w);
        }
    }
    // Or baked into the filename, which is what WordPress does for every
    // resized copy it generates: `IRS-e1746518301375-600x450.jpg`. Missing this
    // meant a 150x150 thumbnail read as "no size declared" and was waved
    // through as a full-bleed card.
    let stem = url.split('?').next()?.rsplit('/').next()?;
    let stem = stem.rsplit_once('.').map(|(s, _)| s).unwrap_or(stem);
    let (_, dims) = stem.rsplit_once('-')?;
    let (w, h) = dims.split_once('x')?;
    // Both halves must parse, or a filename like `photo-2x-retina` would read
    // as a 2-pixel-wide image and send every such story to a generated card.
    let w: u32 = w.parse().ok()?;
    h.parse::<u32>().ok()?;
    Some(w)
}

#[component]
pub fn ShareMeta(
    title: String,
    description: String,
    /// Absolute URL of this page.
    url: String,
    /// Story image if there is one; the branded card is used when empty.
    #[prop(optional, into)]
    image: String,
    /// Slug of a story that can have a card generated for it.
    ///
    /// When the publisher gave us no usable picture, `/og/<slug>.png` renders
    /// one from the story itself — headline, desk, source count. Far better
    /// than the single static card, which turns a timeline of shares into a row
    /// of identical logos telling the reader nothing.
    #[prop(optional, into)]
    card_slug: String,
    /// Advertise the square card rather than the wide one.
    ///
    /// Set for WeChat, whose preview is a small centre-cropped square. The
    /// story is the same; only the crop the client is going to perform differs.
    #[prop(optional)]
    square: bool,
    /// `article` for a story, `website` for everything else.
    #[prop(default = "website")]
    kind: &'static str,
    /// ISO-8601 publication time. LinkedIn and Facebook show a date on the card
    /// when this is present and nothing when it is not, which makes a fresh
    /// story look undated rather than new.
    #[prop(optional, into)]
    published_time: String,
    /// Section name, e.g. "AI". LinkedIn renders it above the headline.
    #[prop(optional, into)]
    section: String,
    /// ISO-8601 last-modified time, when a correction has changed the story.
    #[prop(optional, into)]
    modified_time: String,
) -> impl IntoView {
    use leptos_meta::Meta;
    let base = url
        .split_once("://")
        .and_then(|(scheme, rest)| {
            rest.split_once('/')
                .map(|(host, _)| format!("{scheme}://{host}"))
        })
        .unwrap_or_else(|| url.trim_end_matches('/').to_string());
    // A hotlinked publisher image is only usable as a share card if it is
    // actually big enough to be one. Several syndicate a thumbnail — one story
    // shipped `?width=140&height=105` — and a 140px file on a
    // `summary_large_image` card renders as a blurred smear or is dropped
    // outright. Where the URL states a width we can read, hold it to the size
    // the card needs; otherwise fall back to our own.
    // `image`, when set, is already a URL on our own domain — the server only
    // hands over a mirror it has on disk. It used to be the publisher's CDN
    // link, which is how a story sourced from YouTube came to advertise
    // `i.ytimg.com`: unreachable from mainland China, so WeChat rendered a grey
    // placeholder for the one audience that button exists to serve.
    let usable = !image.trim().is_empty() && declared_width(&image).is_none_or(|w| w >= 600);
    let (img, own_card) = if usable {
        (image, false)
    } else if !card_slug.trim().is_empty() {
        // Generated per story. Still our own domain and our own dimensions, so
        // the width/height declarations below stay truthful.
        (
            format!(
                "{base}/og/{card_slug}.png{}",
                if square { "?sq=1" } else { "" }
            ),
            true,
        )
    } else {
        (format!("{base}/og-default.png"), true)
    };
    let edition = edition_from_url(&url);
    let (site_name, locale, author) = match edition {
        "zh" => ("BitGoose 中文", "zh_CN", "BitGoose AI 编辑部"),
        "zh-hant" => ("BitGoose 繁中", "zh_TW", "BitGoose AI 編輯部"),
        "fr" => ("BitGoose France", "fr_FR", "Rédaction IA BitGoose"),
        "es" => ("BitGoose Español", "es_ES", "Redacción IA BitGoose"),
        "ja" => ("BitGoose 日本語", "ja_JP", "BitGoose AI編集部"),
        "ko" => ("BitGoose 한국어", "ko_KR", "BitGoose AI 편집국"),
        _ => ("BitGoose", "en_US", "BitGoose AI Desk"),
    };
    view! {
        <Meta name="description" content=description.clone() />
        <Meta property="og:type" content=kind />
        <Meta property="og:site_name" content=site_name />
        <Meta property="og:locale" content=locale />
        <Meta property="og:title" content=title.clone() />
        <Meta property="og:description" content=description.clone() />
        <Meta property="og:url" content=url.clone() />
        <Meta property="og:image" content=img.clone() />
        // WeChat and several chat clients size the preview from these rather
        // than fetching the image first; without them the card can collapse to
        // a thumbnail even when the image is large enough. Declared only for
        // our own card, whose dimensions we know — asserting 1200x630 over a
        // publisher image we have never measured is how the thumbnail above
        // came to be advertised as a full-bleed card.
        {own_card
            .then(|| {
                view! {
                    <>
                        <Meta
                            property="og:image:width"
                            content=if square { "800" } else { "1200" }
                        />
                        <Meta
                            property="og:image:height"
                            content=if square { "800" } else { "630" }
                        />
                    </>
                }
            })}
        <Meta property="og:image:alt" content=title.clone() />
        <Meta
            name="twitter:card"
            content=if square { "summary" } else { "summary_large_image" }
        />
        <Meta name="twitter:title" content=title />
        <Meta name="twitter:description" content=description />
        <Meta name="twitter:image" content=img.clone() />
        // LinkedIn reads og:image:secure_url in preference to og:image and
        // silently shows no picture when only the latter is present over some
        // of its crawler paths. Same URL — we are https-only — so this costs a
        // line and removes a whole class of blank card.
        <Meta property="og:image:secure_url" content=img />
        // Article facts. LinkedIn and Facebook use these to date and file the
        // card; X ignores them. Emitted only for stories, since a section front
        // has no publication time and claiming one would be a small lie in
        // structured data.
        {(!published_time.is_empty())
            .then(|| {
                view! { <Meta property="article:published_time" content=published_time.clone() /> }
            })}
        {(!section.is_empty())
            .then(|| view! { <Meta property="article:section" content=section.clone() /> })}
        {(!modified_time.is_empty())
            .then(|| {
                view! { <Meta property="article:modified_time" content=modified_time.clone() /> }
            })}
        {(kind == "article")
            .then(|| {
                view! { <Meta property="article:author" content=author /> }
            })}
    }
}

/// Share controls for a story.
///
/// Three, chosen for how the two platforms actually behave:
///
/// * **X** takes an intent URL, so one tap opens a composer pre-filled with the
///   headline and link.
/// * **WeChat** has no share URL at all — sharing happens inside the app. On a
///   phone the reader uses the browser's own share sheet, so the native
///   `navigator.share` is offered when the browser has it; on a desktop the
///   normal path is to scan the page into the phone, which is why the QR is
///   generated rather than linking somewhere that cannot work.
/// * **Copy link** is the universal fallback, and the only one that works in
///   every client including the ones that block both of the above.
/// A verbatim line from a source, rendered as a pull-quote.
///
/// The attribution links out to the outlet it came from. That is not decoration:
/// a quote whose source the reader cannot reach is indistinguishable from one we
/// made up, and this site's whole argument is that the difference is visible.
#[component]
pub fn PullQuote(quote: crate::model::QuoteCard) -> impl IntoView {
    let has_speaker = !quote.speaker.is_empty();
    view! {
        <figure class="pullquote">
            <blockquote>{quote.text.clone()}</blockquote>
            <figcaption>
                {has_speaker
                    .then(|| view! { <span class="pq-who">{quote.speaker.clone()}</span> })}
                <a href=quote.source_url.clone() target="_blank" rel="noopener nofollow">
                    {quote.source_name.clone()}
                </a>
            </figcaption>
        </figure>
    }
}

/// The Skein's analysis: what the story means and where it goes.
///
/// Deliberately styled as an interruption rather than as more article. Every
/// other block on the page is sourced reporting; this one is the model's own
/// inference, and a reader who skims must not be able to mistake the two. Hence
/// the rule above it, the standing label, and the confidence stated as a number
/// next to the forecast rather than buried in hedged prose.
///
/// The `watch` list is the part that makes it accountable — concrete signals a
/// reader can go and check, which is what separates a forecast from a horoscope.
#[component]
pub fn SkeinBlock(analysis: crate::model::AnalysisCard) -> impl IntoView {
    let watch = analysis.watch.clone();
    let has_watch = !watch.is_empty();
    let has_model = !analysis.model.is_empty();
    // Drives the badge colour. Kept as a class rather than an inline style so
    // the palette stays in one file.
    let stance_class = format!("analysis-stance s-{}", analysis.stance.to_lowercase());

    view! {
        <section class="analysis" aria-labelledby="analysis-h">
            <div class="analysis-head">
                <span class="analysis-mark">
                    <GooseMark size=16 />
                </span>
                <h2 id="analysis-h">"BitGoose 深度分析"</h2>
                <span class="analysis-tag">"AI analysis"</span>
            </div>

            <p class="analysis-sig">{analysis.significance.clone()}</p>

            <div class="analysis-dir">
                <div class="analysis-dir-head">
                    <span class="analysis-dir-label">"Where this goes"</span>
                    <span class=stance_class>
                        {analysis.stance.clone()}
                        <span class="analysis-pct">{analysis.confidence}"%"</span>
                    </span>
                    <span class="analysis-horizon">{analysis.horizon.clone()}</span>
                </div>
                <p>{analysis.direction.clone()}</p>
            </div>

            {has_watch
                .then(|| {
                    view! {
                        <div class="analysis-watch">
                            <span class="analysis-watch-label">"What would confirm it"</span>
                            <ul>
                                {watch
                                    .clone()
                                    .into_iter()
                                    .map(|w| view! { <li>{w}</li> })
                                    .collect_view()}
                            </ul>
                        </div>
                    }
                })}

            <p class="analysis-foot">
                "BitGoose 独立分析，依据下列来源；这部分是推断，而非来源已经报道或交叉证实的事实。 "
                {has_model
                    .then(|| {
                        view! {
                            <span class="analysis-model">
                                "Model: "
                                {analysis.model.clone()}
                            </span>
                        }
                    })}
            </p>
        </section>
    }
}

#[component]
pub fn ShareBar(title: String, url: String) -> impl IntoView {
    let x_intent = format!(
        "https://twitter.com/intent/tweet?text={}&url={}",
        urlencode(&title),
        urlencode(&url)
    );
    let li_share = format!(
        "https://www.linkedin.com/sharing/share-offsite/?url={}",
        urlencode(&url)
    );
    // Drawn here, not fetched. This used to be an <img> from api.qrserver.com,
    // which asked a third party for an image on every page view, told them
    // which story each reader was thinking of sharing, and — since the QR
    // exists specifically so a desktop reader can scan the page into WeChat —
    // failed for the one audience it was built for. See `crate::qr`.
    let qr = crate::qr::svg(&url, 180);

    let copy_url = url.clone();
    let copy = move |_| {
        let nav = window().navigator();
        let _ = nav.clipboard().write_text(&copy_url);
    };

    let share_title = title.clone();
    let share_url = url.clone();
    let native = move |_| {
        // `navigator.share` is the only route into WeChat, WhatsApp and the
        // rest from a mobile browser. Absent on desktop, where the QR covers it.
        let data = web_sys::ShareData::new();
        data.set_title(&share_title);
        data.set_url(&share_url);
        let _ = window().navigator().share_with_data(&data);
    };

    view! {
        <div class="sharebar">
            <span class="sharebar-label">"Share"</span>
            <a class="share-btn" href=x_intent target="_blank" rel="noopener noreferrer">
                "X"
            </a>
            // LinkedIn's sharing endpoint takes only the URL — title and
            // summary come from the page's own Open Graph tags, which is why
            // the `article:*` metadata in `ShareMeta` matters more here than
            // anything we could put in the link.
            <a class="share-btn" href=li_share target="_blank" rel="noopener noreferrer">
                "LinkedIn"
            </a>
            <button class="share-btn" on:click=native title="Share via your device">
                "Share…"
            </button>
            <button class="share-btn" on:click=copy title="Copy link">
                "Copy link"
            </button>
            <details class="share-qr">
                <summary class="share-btn">"WeChat"</summary>
                <div class="share-qr-pop">
                    // Inline markup rather than an <img>: no second request, and
                    // nothing to fail on a slow or filtered network.
                    <div class="share-qr-img" inner_html=qr.unwrap_or_default()></div>
                    <p>"Scan with WeChat to open and share this story."</p>
                </div>
            </details>
        </div>
    }
}

/// Percent-encode for a query string.
///
/// Hand-rolled to keep a URL crate out of the WASM bundle for one call site.
/// Encodes everything outside the unreserved set, so a headline containing
/// `&`, `#` or `?` cannot truncate or rewrite the intent URL it is placed in.
fn urlencode(s: &str) -> String {
    let mut out = String::with_capacity(s.len() * 2);
    for b in s.as_bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(*b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// Says what a card actually is, when it is not an ordinary news article.
///
/// Silent for `rss`, `finance` and `wire` sources, because "article" is the
/// default the layout already communicates and a tag on every card would be
/// noise. It speaks up for the three that would otherwise mislead:
///
/// * **Preprint** — no peer review, no editor, and authors who are the
///   interested party. A reader should know that before weighing the claim.
/// * **Discussion** — an argument among practitioners, not a report. Often the
///   earliest signal there is, and never corroboration.
/// * **Video** — the thing is watchable; saying so is the whole draw.
#[component]
pub fn KindTag(kind: String) -> impl IntoView {
    let (label, class) = match kind.as_str() {
        "research" => ("Preprint", "kind-research"),
        "forum" => ("Discussion", "kind-forum"),
        "video" => ("Video", "kind-video"),
        _ => return None::<AnyView>.into_any(),
    };
    view! { <span class=format!("kind-tag {class}")>{label}</span> }.into_any()
}

/// Which desk a story came from. Shown only where both are mixed together.
#[component]
pub fn BeatTag(beat: String) -> impl IntoView {
    let label = match beat.as_str() {
        "ai" => "AI",
        "crypto" => "Crypto",
        "markets" => "Markets",
        "tech" => "Tech",
        _ => return None::<AnyView>.into_any(),
    };
    view! { <span class=format!("beat-tag beat-{beat}")>{label}</span> }.into_any()
}

/// Verification badge.
#[component]
pub fn VerificationBadge(verification: String, label: String) -> impl IntoView {
    view! { <span class=format!("badge v-{verification}")>{label}</span> }
}

/// Confidence meter — the visual core of the claim ledger.
#[component]
pub fn Meter(confidence: f32, verification: String) -> impl IntoView {
    let pct = (confidence.clamp(0.0, 1.0) * 100.0).round() as i32;
    let color = format!("var(--v-{verification})");
    view! {
        <div
            class="meter"
            role="meter"
            aria-valuenow=pct
            aria-valuemin="0"
            aria-valuemax="100"
            aria-label="Confidence in this claim"
        >
            <div class="meter-fill" style=format!("width:{pct}%;background:{color}")></div>
        </div>
    }
}

/// Source chip with its trust score.
#[component]
pub fn SourceChip(source: SourceCard) -> impl IntoView {
    view! {
        <a
            class="chip out"
            href=source.url.clone()
            target="_blank"
            rel="noopener noreferrer"
            title=format!("{} — read the original", source.title)
        >
            {source.name.clone()}
            <span class="chip-trust">{source.trust}</span>
        </a>
    }
}

/// Percentage change, coloured and signed.
#[component]
pub fn Change(value: Option<f64>) -> impl IntoView {
    match value {
        None => view! { <span class="tick-chg" style="color:var(--faint)">"—"</span> }.into_any(),
        Some(v) => {
            let class = if v >= 0.0 {
                "tick-chg up"
            } else {
                "tick-chg down"
            };
            let sign = if v >= 0.0 { "+" } else { "" };
            view! { <span class=class>{format!("{sign}{v:.2}%")}</span> }.into_any()
        }
    }
}

#[component]
pub fn Ticker(prices: Vec<Tick>) -> impl IntoView {
    if prices.is_empty() {
        // `view! {}` is a unit expression, which clippy rejects. `None` over an
        // AnyView is the idiomatic "render nothing" and is genuinely clearer
        // about the intent.
        return None::<AnyView>.into_any();
    }
    // The marquee translates by -50%, so the list is rendered twice to make the
    // wrap seamless rather than snapping back to the start.
    let doubled: Vec<Tick> = prices.iter().chain(prices.iter()).cloned().collect();
    view! {
        <div class="ticker" aria-label="Market prices">
            <div class="ticker-track">
                {doubled
                    .into_iter()
                    .map(|t| {
                        view! {
                            <span class="tick">
                                <a href=format!("/asset/{}", t.symbol) class="tick-sym">
                                    {t.symbol.clone()}
                                </a>
                                <span class="tick-px">"$"{t.price.clone()}</span>
                                <Change value=t.change />
                            </span>
                        }
                    })
                    .collect_view()}
            </div>
        </div>
    }
    .into_any()
}

/// A publisher's image, shown on our page and credited back to them.
///
/// Hotlinked deliberately. These come out of the `<media:*>` and `<enclosure>`
/// fields of a feed — the parts a publisher populates *so that* aggregators
/// display them — so serving them from the publisher's own CDN keeps their
/// analytics and their control intact. Copying them onto our storage would take
/// both away, and would be the point at which showing someone's photograph
/// starts to look like appropriating it.
///
/// `shape` picks the aspect ratio, so a missing or slow image reserves its space
/// instead of shoving the headline down the page when it arrives.
#[component]
pub fn VideoEmbed(
    video_id: String,
    title: String,
    credit: String,
    credit_url: String,
) -> impl IntoView {
    if video_id.is_empty() {
        return None::<AnyView>.into_any();
    }
    // youtube-nocookie is YouTube's privacy-enhanced host: it holds off on
    // profiling cookies until the visitor actually presses play.
    //
    // The frame is emitted as markup because Leptos 0.8 has no typed `loading`,
    // `allow` or `allowfullscreen` for `iframe`, and an iframe without
    // `loading="lazy"` costs every reader a player download they may never
    // watch. This is only safe because `video_id` cannot contain anything that
    // escapes the attribute: `bg_ingest::video` accepts exactly 11 characters
    // of `[A-Za-z0-9_-]`, the database re-checks the same shape, and the title
    // below is escaped before it is interpolated.
    let frame = format!(
        r#"<iframe src="https://www.youtube-nocookie.com/embed/{id}?rel=0" title="{t}" loading="lazy" frameborder="0" referrerpolicy="strict-origin-when-cross-origin" allow="accelerometer; clipboard-write; encrypted-media; gyroscope; picture-in-picture" allowfullscreen></iframe>"#,
        id = video_id,
        t = escape_attr(&title),
    );
    let watch = format!("https://www.youtube.com/watch?v={video_id}");
    view! {
        <figure class="media media-video">
            <div class="video-frame" inner_html=frame></div>
            <figcaption>
                {(!credit.is_empty())
                    .then(|| {
                        view! {
                            <a href=credit_url rel="noopener">
                                {credit.clone()}
                            </a>
                            " — "
                        }
                    })}
                "plays on YouTube. "
                <a href=watch rel="noopener">"Watch there"</a>
            </figcaption>
        </figure>
    }
    .into_any()
}

/// Escape a value destined for a double-quoted HTML attribute.
fn escape_attr(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 8);
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            c => out.push(c),
        }
    }
    out
}

#[component]
pub fn SourcedImage(
    url: String,
    alt: String,
    credit: String,
    credit_url: String,
    #[prop(default = "media-wide")] shape: &'static str,
    #[prop(default = true)] show_credit: bool,
) -> impl IntoView {
    if url.is_empty() {
        return None::<AnyView>.into_any();
    }
    // Publishers move and expire images constantly, and a broken-image glyph
    // where a photograph should be looks worse than a clean text card. On error
    // the whole figure removes itself.
    let on_error = move |ev: leptos::ev::ErrorEvent| {
        // Via web-sys rather than the `wasm-bindgen` crate directly: that one
        // is an optional dependency enabled only by the `hydrate` feature, and
        // this component also compiles into the SSR build.
        use web_sys::wasm_bindgen::JsCast;
        if let Some(img) = ev
            .target()
            .and_then(|t| t.dyn_into::<web_sys::Element>().ok())
        {
            if let Some(fig) = img.closest("figure").ok().flatten() {
                let _ = fig.set_attribute("hidden", "hidden");
            }
        }
    };
    view! {
        <figure class=format!("media {shape}")>
            <img
                src=url
                alt=alt
                loading="lazy"
                decoding="async"
                on:error=on_error
            />
            {(show_credit && !credit.is_empty())
                .then(|| {
                    view! {
                        <figcaption>
                            "Image: "
                            <a href=credit_url.clone() target="_blank" rel="noopener nofollow">
                                {credit.clone()}
                            </a>
                        </figcaption>
                    }
                })}
        </figure>
    }
    .into_any()
}

/// Story card, used across every listing.
#[component]
pub fn Card(story: StoryCard) -> impl IntoView {
    let path = use_location().pathname.get();
    let prefix = edition_prefix(edition_code(&path));
    let href = format!("{prefix}/story/{}", story.slug);
    let section_href = format!("{prefix}/section/{}", story.category);
    let is_wire = story.kind == "wire";
    view! {
        <article class="card">
            <a href=href.clone() class="card-media-link" aria-hidden="true" tabindex="-1">
                <SourcedImage
                    url=story.image_url.clone()
                    alt=String::new()
                    credit=story.lead_source.clone()
                    credit_url=story.lead_url.clone()
                    shape="media-card"
                    show_credit=false
                />
            </a>
            <div class="meta">
                <a href=section_href class="kicker">
                    {story.category_label.clone()}
                </a>
                <span class="dot">"·"</span>
                <time>{story.ago.clone()}</time>
                {(story.source_count > 1)
                    .then(|| {
                        view! {
                            <>
                                <span class="dot">"·"</span>
                                <span class="src-count">
                                    <strong>{story.source_count}</strong>
                                    " sources"
                                </span>
                            </>
                        }
                    })}
                // Marks the stories that carry a take. Most do not — the Skein
                // stays quiet when the sources are thin — so this is a real
                // signal about which link is worth opening rather than a badge
                // on everything.
                {story
                    .has_analysis
                    .then(|| {
                        view! {
                            <>
                                <span class="dot">"·"</span>
                                <span class="has-analysis" title="Includes BitGoose analysis">
                                    "Analysis"
                                </span>
                            </>
                        }
                    })}
            </div>
            <h3>
                <a href=href.clone()>{story.title.clone()}</a>
            </h3>
            {(!story.dek.is_empty())
                .then(|| view! { <p class="dek">{story.dek.clone()}</p> })}
            {(is_wire && !story.lead_source.is_empty())
                .then(|| {
                    view! {
                        <div class="wire-foot">
                            <a
                                class="chip out"
                                href=story.lead_url.clone()
                                target="_blank"
                                rel="noopener noreferrer"
                            >
                                {story.lead_source.clone()}
                            </a>
                        </div>
                    }
                })}
        </article>
    }
}

/// Empty state that tells the operator how to fix it.
#[component]
pub fn Empty(#[prop(into)] message: String, #[prop(into, optional)] hint: String) -> impl IntoView {
    view! {
        <div class="empty">
            <p style="margin:0 0 .5rem">{message}</p>
            {(!hint.is_empty())
                .then(|| view! { <p style="margin:0"><code>{hint.clone()}</code></p> })}
        </div>
    }
}

#[component]
pub fn Loading() -> impl IntoView {
    view! { <p class="loading">"Loading…"</p> }
}

#[cfg(test)]
mod share_meta_tests {
    use super::declared_width;

    #[test]
    fn a_thumbnail_url_reports_its_real_size() {
        // The exact URL that shipped as a 1200x630 card.
        assert_eq!(
            declared_width("https://preview.redd.it/x4l5.jpg?width=140&height=105&auto=webp"),
            Some(140)
        );
        assert_eq!(
            declared_width("https://img.example.com/a.jpg?w=1600&q=80"),
            Some(1600)
        );
    }

    #[test]
    fn wordpress_size_suffixes_are_read_from_the_filename() {
        // A live story shipped this one, and the size is in the name, not the
        // query — so it read as "no size declared" and was accepted.
        assert_eq!(
            declared_width(
                "https://www.tbstat.com/wp/uploads/2020/03/IRS-e1746518301375-600x450.jpg"
            ),
            Some(600)
        );
        assert_eq!(
            declared_width("https://x.example/a/photo-150x150.png"),
            Some(150)
        );
    }

    #[test]
    fn a_filename_that_merely_looks_dimensional_is_ignored() {
        // Rejecting these would send perfectly good photographs to a generated
        // card, which is the opposite of the intent.
        assert_eq!(
            declared_width("https://x.example/photo-2x-retina.jpg"),
            None
        );
        assert_eq!(declared_width("https://x.example/chart-q3xq4.png"), None);
    }

    #[test]
    fn a_url_that_claims_nothing_is_left_alone() {
        // Most publisher images have no size hint; discarding those would throw
        // away every usable card to catch the few bad ones.
        assert_eq!(declared_width("https://example.com/photo.jpg"), None);
        assert_eq!(
            declared_width("https://example.com/photo.jpg?auto=webp"),
            None
        );
    }
}
