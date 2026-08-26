//! Server functions.
//!
//! Each is a normal Rust `async fn` on the server and an RPC call from the
//! hydrated client. The database work sits behind `#[cfg(feature = "ssr")]`, so
//! `bg-db` and its native dependencies are never compiled into the WASM bundle.

use crate::model::*;
use leptos::prelude::*;

#[cfg(feature = "ssr")]
pub use server_impl::db;

#[cfg(feature = "ssr")]
mod server_impl {
    use bg_db::Db;
    use std::sync::OnceLock;

    static DB: OnceLock<Db> = OnceLock::new();

    /// Install the pool once at startup.
    pub fn set_db(db: Db) {
        let _ = DB.set(db);
    }

    /// The pool, for server functions.
    ///
    /// A process-wide handle rather than Leptos context: the pool is genuinely
    /// global, cheap to clone, and threading it through every render would add
    /// plumbing without adding safety.
    pub fn db() -> &'static Db {
        DB.get()
            .expect("database pool not initialised — call set_db() at startup")
    }
}

#[cfg(feature = "ssr")]
pub use server_impl::set_db;

#[cfg(feature = "ssr")]
fn e(err: impl std::fmt::Display) -> ServerFnError {
    tracing::error!(error = %err, "server function failed");
    ServerFnError::new(err.to_string())
}

/// Mark wire cards whose stories carry an analysis.
///
/// The Wire needs its own pass because its entries come from a different query
/// than `front_page` and carry their own `story_id`. Saying so in a comment and
/// not writing it — which is what happened first time — left the marker
/// invisible on the surface where most of the site's stories actually live.
#[cfg(feature = "ssr")]
async fn flag_analysis_wire(
    db: &bg_db::Db,
    entries: &[bg_core::domain::WireEntry],
    cards: &mut [StoryCard],
) {
    let ids: Vec<_> = entries.iter().map(|w| w.story_id).collect();
    let Ok(have) = bg_db::analyses::which_have_analysis(db, &ids).await else {
        return;
    };
    for c in cards.iter_mut() {
        if let Some(w) = entries.iter().find(|w| w.slug == c.slug) {
            c.has_analysis = have.contains(&w.story_id.into_uuid());
        }
    }
}

/// Mark the cards whose stories carry an analysis.
///
/// Done as a pass over a finished page rather than inside [`card`] so it costs
/// one query no matter how many cards there are, and so a caller that does not
/// want the badge simply does not call it.
#[cfg(feature = "ssr")]
async fn flag_analysis(
    db: &bg_db::Db,
    stories: &[bg_core::domain::Story],
    cards: &mut [StoryCard],
) {
    let ids: Vec<_> = stories.iter().map(|s| s.id).collect();
    let Ok(have) = bg_db::analyses::which_have_analysis(db, &ids).await else {
        return; // A badge is not worth failing a page over.
    };
    for c in cards.iter_mut() {
        if let Some(st) = stories.iter().find(|s| s.slug == c.slug) {
            c.has_analysis = have.contains(&st.id.into_uuid());
        }
    }
}

#[cfg(feature = "ssr")]
fn card(s: &bg_core::domain::Story, lead: Option<(&str, &str)>) -> StoryCard {
    StoryCard {
        slug: s.slug.clone(),
        kind: s.kind.as_str().into(),
        title: s.title.clone(),
        dek: s.summary.clone().unwrap_or_default(),
        category: s.category.as_str().into(),
        category_label: category_label(s.category, s.editorial_language).into(),
        source_count: s.source_count,
        newsworthiness: s.newsworthiness,
        published_at: s.published_at.map(|d| d.to_rfc3339()).unwrap_or_default(),
        ago: s.published_at.map(ago).unwrap_or_default(),
        assets: s.assets.clone(),
        lead_source: lead.map(|(n, _)| n.to_string()).unwrap_or_default(),
        lead_url: lead.map(|(_, u)| u.to_string()).unwrap_or_default(),
        // A feed can hand us a video-player URL where an image belongs; that
        // renders as a permanently broken image. Normalise at the boundary.
        image_url: s
            .image_url
            .as_deref()
            .and_then(bg_core::media::as_image)
            .unwrap_or_default(),
        beat: s.beat.as_str().into(),
        source_kind: String::new(),
        has_analysis: false,
    }
}

#[cfg(feature = "ssr")]
fn category_label(
    category: bg_core::domain::Category,
    language: bg_core::domain::EditorialLanguage,
) -> &'static str {
    match language {
        bg_core::domain::EditorialLanguage::En => category.label(),
        bg_core::domain::EditorialLanguage::Zh => category.label_zh(),
        bg_core::domain::EditorialLanguage::ZhHant => category.label_zh_hant(),
        bg_core::domain::EditorialLanguage::Fr => category.label_fr(),
        bg_core::domain::EditorialLanguage::Es => category.label_es(),
        bg_core::domain::EditorialLanguage::Ja => category.label_ja(),
        bg_core::domain::EditorialLanguage::Ko => category.label_ko(),
    }
}

/// Render the article body, resolving `[^cN]` markers into ledger links.
///
/// Done server-side so the citation links exist in the initial HTML — they are
/// the primary navigation of a story page, and a reader with JavaScript off
/// should still be able to walk from a sentence to its evidence.
#[cfg(feature = "ssr")]
pub(crate) fn render_body(md: &str) -> String {
    use pulldown_cmark::{html, Options, Parser};
    let mut opts = Options::empty();
    opts.insert(Options::ENABLE_STRIKETHROUGH);
    opts.insert(Options::ENABLE_TABLES);

    let mut out = String::new();
    html::push_html(&mut out, Parser::new_ext(md, opts));

    // Markdown leaves `[^c1]` untouched; turn each into an anchor.
    let mut result = String::with_capacity(out.len());
    let chars: Vec<char> = out.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if i + 3 < chars.len() && chars[i] == '[' && chars[i + 1] == '^' {
            let mut j = i + 2;
            let mut tok = String::new();
            while j < chars.len() && chars[j] != ']' && tok.len() < 8 {
                tok.push(chars[j]);
                j += 1;
            }
            if j < chars.len()
                && chars[j] == ']'
                && !tok.is_empty()
                && tok.chars().all(|c| c.is_ascii_alphanumeric())
            {
                result.push_str(&format!(
                    "<a class=\"cite\" href=\"#claim-{tok}\" title=\"Jump to the evidence for this statement\">{tok}</a>"
                ));
                i = j + 1;
                continue;
            }
        }
        result.push(chars[i]);
        i += 1;
    }
    result
}

// ---------------------------------------------------------------------------
// front page
// ---------------------------------------------------------------------------

/// The front page, for one desk or blended.
///
/// `beat` is a plain string because it crosses the server-function boundary;
/// an unrecognised value blends rather than erroring, so a stale or hand-typed
/// URL degrades to the full front page instead of a 500.
#[server(name = GetFrontPage, prefix = "/rpc")]
pub async fn get_front_page(
    beat: Option<String>,
    language: String,
) -> Result<FrontPage, ServerFnError> {
    use std::str::FromStr;
    let beat = beat
        .as_deref()
        .and_then(|b| bg_core::domain::Beat::from_str(b).ok());
    let language = bg_core::domain::EditorialLanguage::from_str(&language)
        .unwrap_or(bg_core::domain::EditorialLanguage::En);
    let db = db();
    let ranked = bg_db::stories::front_page_for_language(db, beat, language, 40)
        .await
        .map_err(e)?;

    // The lead is the best story on the site right now, whatever form it took.
    //
    // It used to be the best *Desk* story, which quietly froze the front page:
    // a Desk story needs two independent sources, clustering rarely produces
    // that (1,407 of 1,438 published stories carry exactly one), so only two
    // Desk stories existed and the newest was seven days old. The site was
    // publishing thirty-three stories every six hours and leading with a
    // week-old one. Tying the most prominent slot to the rarest category is
    // the kind of coupling that looks fine until the rare thing stops
    // happening.
    //
    // A newsroom leads with the biggest news. `ranked` is already ordered by
    // newsworthiness decayed against age, so its head is exactly that.
    // …but not one whose headline arrived already cut off. Some syndicators
    // truncate at a fixed width and Google News passes that through verbatim,
    // so "Bitcoin falls below $76K, wiping out $100M in l..." led the front
    // page with a word missing. The story is fine and stays in the Wire; it
    // just cannot be the first thing a reader sees, because a severed headline
    // reads as our error rather than the publisher's.
    let mut lead = ranked
        .iter()
        .find(|s| !bg_core::text::looks_truncated(&s.title))
        .or_else(|| ranked.first())
        .map(|s| card(s, None));

    // The Desk row keeps its own character — original synthesis, not pointers —
    // minus whatever became the lead.
    let mut desk: Vec<StoryCard> = ranked
        .iter()
        .filter(|s| s.kind == bg_core::domain::StoryKind::Desk)
        .filter(|s| Some(&s.slug) != lead.as_ref().map(|l| &l.slug))
        .take(8)
        .map(|s| card(s, None))
        .collect();

    let wire_entries = bg_db::stories::wire_for_language(db, beat, language, 14, 0)
        .await
        .map_err(e)?;
    let mut wire: Vec<StoryCard> = wire_entries
        .iter()
        .filter(|w| Some(&w.slug) != lead.as_ref().map(|l| &l.slug))
        .map(|w| StoryCard {
            slug: w.slug.clone(),
            kind: "wire".into(),
            title: w.title.clone(),
            dek: w.summary.clone(),
            category: w.category.as_str().into(),
            category_label: category_label(w.category, language).into(),
            source_count: w.source_count,
            newsworthiness: w.newsworthiness,
            published_at: w.published_at.to_rfc3339(),
            ago: ago(w.published_at),
            assets: w.assets.clone(),
            lead_source: w.source_name.clone(),
            lead_url: w.source_url.clone(),
            beat: w.beat.as_str().into(),
            source_kind: w.source_kind.as_str().into(),
            has_analysis: false,
            image_url: w
                .image_url
                .as_deref()
                .and_then(bg_core::media::as_image)
                .unwrap_or_default(),
        })
        .collect();
    flag_analysis_wire(db, &wire_entries, &mut wire).await;

    let prices = bg_db::prices::latest_all(db)
        .await
        .map_err(e)?
        .iter()
        // Twelve coins, three indices and four equities. The cap was 14, which
        // silently dropped whatever sorted last — which was every equity.
        .take(20)
        .map(|t| Tick {
            symbol: t.symbol.clone(),
            price: fmt_price(t.price_usd.to_string().parse().unwrap_or(0.0)),
            change: t.change_24h_pct,
        })
        .collect();

    // The Honk bar is for genuinely breaking news, so it is gated on both
    // score and recency. A stale banner that never changes is worse than none.
    let honk = ranked
        .iter()
        .find(|s| {
            s.newsworthiness >= 80
                && !bg_core::text::looks_truncated(&s.title)
                && s.published_at
                    .is_some_and(|p| (chrono::Utc::now() - p).num_hours() < 6)
        })
        .map(|s| card(s, None));

    // One lookup covers the lead and the desk row; the Wire cards come from a
    // different query and are flagged with their own.
    {
        let mut all: Vec<StoryCard> = lead.iter().cloned().chain(desk.iter().cloned()).collect();
        flag_analysis(db, &ranked, &mut all).await;
        let mut it = all.into_iter();
        if lead.is_some() {
            lead = it.next();
        }
        desk = it.collect();
    }

    // Live only — a topic nobody has written about for two days is an archive
    // page, not a special topic, and offering it as one is how a news site ends
    // up looking abandoned.
    let gaggles: Vec<GaggleCard> = bg_db::gaggles::live_for_language(db, language, 48, 3)
        .await
        .unwrap_or_default()
        .into_iter()
        .map(|g| GaggleCard {
            slug: g.slug,
            title: g.title,
            standfirst: g.standfirst,
            sources: g.source_count,
            stories: g.story_count,
            model: g.model.unwrap_or_default(),
            language: g.editorial_language,
            pinned: g.pinned,
            analysis_html: render_body(&g.analysis_md),
            watchpoints: g.watchpoints,
            primary_sources: g
                .primary_source_names
                .into_iter()
                .zip(g.primary_source_urls)
                .map(|(name, url)| TopicSource { name, url })
                .collect(),
            last_updated: g
                .last_briefed_at
                .map(|at| at.to_rfc3339())
                .unwrap_or_default(),
        })
        .collect();

    Ok(FrontPage {
        lead,
        desk,
        wire,
        gaggles,
        prices,
        honk,
    })
}

// ---------------------------------------------------------------------------
// listings
// ---------------------------------------------------------------------------

#[server(name = GetStories, prefix = "/rpc")]
pub async fn get_stories(
    kind: String,
    limit: i64,
    language: String,
) -> Result<Vec<StoryCard>, ServerFnError> {
    use std::str::FromStr;
    let db = db();
    let language = bg_core::domain::EditorialLanguage::from_str(&language)
        .unwrap_or(bg_core::domain::EditorialLanguage::En);
    let k = bg_core::domain::StoryKind::from_str(&kind).ok();
    if k == Some(bg_core::domain::StoryKind::Wire) {
        let entries = bg_db::stories::wire_for_language(db, None, language, limit, 0)
            .await
            .map_err(e)?;
        let mut cards: Vec<StoryCard> = entries
            .iter()
            .map(|w| StoryCard {
                slug: w.slug.clone(),
                kind: "wire".into(),
                title: w.title.clone(),
                dek: w.summary.clone(),
                category: w.category.as_str().into(),
                category_label: category_label(w.category, language).into(),
                source_count: w.source_count,
                newsworthiness: w.newsworthiness,
                published_at: w.published_at.to_rfc3339(),
                ago: ago(w.published_at),
                assets: w.assets.clone(),
                lead_source: w.source_name.clone(),
                lead_url: w.source_url.clone(),
                image_url: w
                    .image_url
                    .as_deref()
                    .and_then(bg_core::media::as_image)
                    .unwrap_or_default(),
                beat: w.beat.as_str().into(),
                source_kind: w.source_kind.as_str().into(),
                has_analysis: false,
            })
            .collect();
        flag_analysis_wire(db, &entries, &mut cards).await;
        return Ok(cards);
    }
    let stories = bg_db::stories::published_for_language(db, k, language, limit, 0)
        .await
        .map_err(e)?;
    let mut cards: Vec<StoryCard> = stories.iter().map(|s| card(s, None)).collect();
    flag_analysis(db, &stories, &mut cards).await;
    Ok(cards)
}

#[server(name = GetGaggle, prefix = "/rpc")]
pub async fn get_gaggle(
    slug: String,
    language: String,
) -> Result<Option<GagglePage>, ServerFnError> {
    use std::str::FromStr;
    let db = db();
    let language = bg_core::domain::EditorialLanguage::from_str(&language)
        .unwrap_or(bg_core::domain::EditorialLanguage::En);
    let Some(g) = bg_db::gaggles::by_slug(db, &slug, language)
        .await
        .map_err(e)?
    else {
        return Ok(None);
    };
    let ids = bg_db::gaggles::story_ids(db, &slug, language)
        .await
        .map_err(e)?;

    let mut stories = Vec::with_capacity(ids.len());
    for id in &ids {
        if let Ok(s) = bg_db::stories::by_id(db, *id).await {
            stories.push(s);
        }
    }
    let mut cards: Vec<StoryCard> = stories.iter().map(|s| card(s, None)).collect();
    flag_analysis(db, &stories, &mut cards).await;

    Ok(Some(GagglePage {
        card: GaggleCard {
            slug: g.slug,
            title: g.title,
            standfirst: g.standfirst,
            sources: g.source_count,
            stories: g.story_count,
            model: g.model.unwrap_or_default(),
            language: g.editorial_language,
            pinned: g.pinned,
            analysis_html: render_body(&g.analysis_md),
            watchpoints: g.watchpoints,
            primary_sources: g
                .primary_source_names
                .into_iter()
                .zip(g.primary_source_urls)
                .map(|(name, url)| TopicSource { name, url })
                .collect(),
            last_updated: g
                .last_briefed_at
                .map(|at| at.to_rfc3339())
                .unwrap_or_default(),
        },
        stories: cards,
    }))
}

#[server(name = GetSection, prefix = "/rpc")]
pub async fn get_section(
    category: String,
    language: String,
) -> Result<(String, Vec<StoryCard>), ServerFnError> {
    use std::str::FromStr;
    let db = db();
    let cat = bg_core::domain::Category::from_str(&category)
        .map_err(|_| ServerFnError::new(format!("no such section: {category}")))?;
    let language = bg_core::domain::EditorialLanguage::from_str(&language)
        .unwrap_or(bg_core::domain::EditorialLanguage::En);
    let stories = bg_db::stories::by_category_for_language(db, cat, language, 60)
        .await
        .map_err(e)?;
    let mut cards: Vec<StoryCard> = stories.iter().map(|s| card(s, None)).collect();
    flag_analysis(db, &stories, &mut cards).await;
    Ok((cat.label().to_string(), cards))
}

// ---------------------------------------------------------------------------
// story page
// ---------------------------------------------------------------------------

#[server(name = GetStory, prefix = "/rpc")]
pub async fn get_story(slug: String) -> Result<Option<StoryPage>, ServerFnError> {
    let db = db();
    let story = match bg_db::stories::published_by_slug(db, &slug).await {
        Ok(s) => s,
        Err(bg_db::DbError::NotFound(_)) => {
            // Not here — but it may have been folded into the story it was
            // always part of, in which case this URL is somebody's shared link
            // and the reporting it points at still exists. A permanent redirect
            // sends the reader to it and tells search engines the two were one.
            //
            // Only folds redirect. A story retracted for being wrong must go
            // nowhere: pointing it at another story would read as a correction
            // of that one.
            if let Ok(Some(to)) = bg_db::stories::folded_to(db, &slug).await {
                // Set explicitly rather than through `leptos_axum::redirect`,
                // which issues a 302 and only when the request advertises
                // `text/html`. Neither suits this. A fold is permanent, so 301
                // is what consolidates the two URLs in a search index — and a
                // crawler that omits an Accept header would otherwise receive a
                // Location with no status to act on.
                if let Some(res) = use_context::<leptos_axum::ResponseOptions>() {
                    res.set_status(axum::http::StatusCode::MOVED_PERMANENTLY);
                    if let Ok(v) = axum::http::HeaderValue::from_str(&format!("/story/{to}")) {
                        res.insert_header(axum::http::header::LOCATION, v);
                    }
                }
            } else if let Some(res) = use_context::<leptos_axum::ResponseOptions>() {
                // Otherwise say so properly. Every unknown URL was answering
                // 200 with an empty shell, which is a soft 404: a search engine
                // files it as thin content and keeps coming back, and a reader
                // who mistypes a slug gets a blank page rather than a message.
                res.set_status(axum::http::StatusCode::NOT_FOUND);
            }
            return Ok(None);
        }
        Err(err) => return Err(e(err)),
    };

    let article = bg_db::articles::latest_for_story(db, story.id)
        .await
        .map_err(e)?;
    let claims = bg_db::claims::with_sources(db, story.id).await.map_err(e)?;
    let refs = bg_db::stories::source_refs(db, story.id).await.map_err(e)?;
    let corrections = bg_db::articles::corrections_for_story(db, story.id)
        .await
        .map_err(e)?;
    let runs = bg_db::agents::runs_for_story(db, story.id)
        .await
        .map_err(e)?;
    let analysis = bg_db::analyses::for_story(db, story.id).await.map_err(e)?;

    // Quotes are rendered as pull-quotes in the article with their attribution
    // and a link out, so repeating them in the ledger says the same thing twice
    // in the same eyeful. They stay in the claim graph and in the API — this is
    // a decision about the sidebar, not about the record.
    let claim_cards = claims
        .iter()
        .filter(|c| c.claim.kind != bg_core::domain::ClaimKind::Quote)
        .enumerate()
        .map(|(i, c)| {
            let (supporting, disputing): (Vec<_>, Vec<_>) = c
                .sources
                .iter()
                .partition(|s| s.stance != bg_core::domain::Stance::Contradicts);
            ClaimCard {
                marker: format!("c{}", i + 1),
                text: c.claim.text.clone(),
                kind: c.claim.kind.as_str().into(),
                verification: c.claim.verification.as_str().into(),
                verification_label: c.claim.verification.label().into(),
                confidence: c.claim.confidence,
                excerpt: c.sources.iter().find_map(|s| s.excerpt.clone()),
                sources: supporting
                    .iter()
                    .map(|s| SourceCard {
                        name: s.source_name.clone(),
                        slug: s.source_slug.clone(),
                        url: s.url.clone(),
                        title: s.title.clone(),
                        trust: s.source_trust,
                    })
                    .collect(),
                disputed_by: disputing
                    .iter()
                    .map(|s| SourceCard {
                        name: s.source_name.clone(),
                        slug: s.source_slug.clone(),
                        url: s.url.clone(),
                        title: s.title.clone(),
                        trust: s.source_trust,
                    })
                    .collect(),
            }
        })
        .collect();

    // Quotes are already in the claim graph. Pulling them into their own list
    // is presentation, not a second store — the excerpt and its attribution
    // still come from `claim_sources`, so a quote cannot appear on the page
    // without the source link that justifies it.
    let quote_cards: Vec<QuoteCard> = claims
        .iter()
        .filter(|c| c.claim.kind == bg_core::domain::ClaimKind::Quote)
        .filter_map(|c| {
            let src = c.sources.first()?;
            let excerpt = src.excerpt.clone()?;
            // `text` is stored as `Speaker: "quote"`; the speaker is the part
            // before the excerpt, if the model attributed one.
            let speaker = c
                .claim
                .text
                .split_once(": \u{201c}")
                .map(|(who, _)| who.trim().to_string())
                .unwrap_or_default();
            Some(QuoteCard {
                text: excerpt,
                speaker,
                source_name: src.source_name.clone(),
                source_url: src.url.clone(),
            })
        })
        .collect();

    let analysis_card = analysis.as_ref().map(|a| AnalysisCard {
        significance: a.significance.clone(),
        direction: a.direction.clone(),
        horizon: a.horizon.clone(),
        confidence: a.confidence,
        stance: a.stance().into(),
        watch: a.watch.clone(),
        model: a.model.clone().unwrap_or_default(),
    });

    let published = story.published_at.unwrap_or(story.first_seen_at);
    let headline = article
        .as_ref()
        .map(|a| a.headline.clone())
        .unwrap_or(story.title.clone());
    let base = std::env::var("BG_PUBLIC_BASE_URL")
        .unwrap_or_else(|_| format!("https://{}", bg_core::brand::DOMAIN));
    let language = story.editorial_language;
    let edition_prefix = match language {
        bg_core::domain::EditorialLanguage::En => "",
        bg_core::domain::EditorialLanguage::Zh => "/zh",
        bg_core::domain::EditorialLanguage::ZhHant => "/zh-hant",
        bg_core::domain::EditorialLanguage::Fr => "/fr",
        bg_core::domain::EditorialLanguage::Es => "/es",
        bg_core::domain::EditorialLanguage::Ja => "/ja",
        bg_core::domain::EditorialLanguage::Ko => "/ko",
    };
    let canonical = format!(
        "{}{edition_prefix}/story/{}",
        base.trim_end_matches('/'),
        story.slug
    );

    // The lead image and who to credit for it. `refs` is ordered seed-first, so
    // its head is the outlet the image most likely came from.
    let image_url = story
        .image_url
        .as_deref()
        .and_then(bg_core::media::as_image)
        .unwrap_or_default();
    let (image_credit, image_credit_url) = refs
        .first()
        .map(|r| (r.name.clone(), r.url.clone()))
        .unwrap_or_default();

    // Our copy or our card, never a hotlink.
    //
    // The two differ in what they do while the mirror is cold. A *reader* is
    // better served by the publisher's URL — the picture appears now, and if
    // their network cannot reach that CDN they still have the whole article. A
    // *crawler* is not: it gets one fetch, and pointing it at a host it may not
    // be able to reach is how a shared link came to render as a grey
    // placeholder. So the crawler is given the card we can always serve, and
    // the mirror fills in behind for the next share.
    let (image_url, share_image) = if image_url.is_empty() {
        // No publisher picture. Show the card we already draw for sharing
        // rather than nothing at all.
        //
        // 46% of the corpus is arXiv preprints and similar, which have no
        // image by nature — so nearly half of every desk rendered as a wall of
        // text next to sites where every story carries a photograph. The card
        // is generated from the story's own facts (headline, desk, source
        // count) and is already on disk for the share path, so this costs a
        // static file read and makes the page look like a news page.
        (
            format!("{}/og/{}.png", base.trim_end_matches('/'), story.slug),
            String::new(),
        )
        // NB: the credit is cleared below. Attributing a card BitGoose drew to
        // the outlet that reported the story would be a false credit on the
        // most visible part of the page.
    } else if crate::ogroute::mirrored(&story.slug).is_some() {
        let ours = format!("{}/img/{}", base.trim_end_matches('/'), story.slug);
        (ours.clone(), ours)
    } else {
        crate::ogroute::warm(db.clone(), story.slug.clone());
        (image_url, String::new())
    };

    // WeChat crops previews to a square. Asking it here, from the request that
    // is reading the page, rather than at image-fetch time: the agent that
    // fetches the picture is often not the one that parsed the HTML.
    let square_card = leptos_axum::extract::<axum::http::HeaderMap>()
        .await
        .ok()
        .and_then(|h| {
            h.get(axum::http::header::USER_AGENT)
                .and_then(|v| v.to_str().ok())
                .map(|ua| ua.contains("MicroMessenger"))
        })
        .unwrap_or(false);

    // schema.org NewsArticle. `citation` carries every source URL, which is
    // both honest and the structured-data way to say "this is synthesis over
    // other people's reporting, and here is whose".
    let category_label = category_label(story.category, language);
    let author = match language {
        bg_core::domain::EditorialLanguage::Zh => "BitGoose AI 编辑部",
        bg_core::domain::EditorialLanguage::ZhHant => "BitGoose AI 編輯部",
        bg_core::domain::EditorialLanguage::Fr => "Rédaction IA BitGoose",
        bg_core::domain::EditorialLanguage::Es => "Redacción IA BitGoose",
        bg_core::domain::EditorialLanguage::Ja => "BitGoose AI編集部",
        bg_core::domain::EditorialLanguage::Ko => "BitGoose AI 편집국",
        bg_core::domain::EditorialLanguage::En => "BitGoose AI Desk",
    };
    let json_ld = serde_json::json!({
        "@context": "https://schema.org",
        "@type": "NewsArticle",
        "headline": headline,
        "description": article.as_ref().map(|a| a.dek.clone()).unwrap_or_default(),
        "url": canonical,
        "mainEntityOfPage": { "@type": "WebPage", "@id": canonical },
        "datePublished": published.to_rfc3339(),
        "dateModified": story.updated_at.to_rfc3339(),
        // Google News and most aggregators will not feature an article that
        // has no image in its structured data.
        "image": if image_url.is_empty() { serde_json::Value::Null }
                 else { serde_json::json!([image_url]) },
        "articleSection": category_label,
        "inLanguage": language.html_lang(),
        "isAccessibleForFree": true,
        "author": {
            "@type": "Organization",
            "name": author,
            "description": bg_core::brand::AI_DISCLOSURE,
            "url": format!("{}{edition_prefix}/flock", base.trim_end_matches('/')),
        },
        "publisher": {
            "@type": "Organization",
            "name": bg_core::brand::NAME,
            "url": base,
        },
        "citation": refs.iter().map(|r| serde_json::json!({
            "@type": "CreativeWork",
            "name": r.title,
            "url": r.url,
            "publisher": { "@type": "Organization", "name": r.name },
        })).collect::<Vec<_>>(),
    })
    .to_string();

    // A card must always say something; the page shows a dek only when one was
    // written. Roughly a quarter of published stories have neither dek nor
    // summary, and each of those was sharing as a headline over a blank space.
    let share_description = bg_core::share::description(
        article.as_ref().map(|a| a.dek.as_str()).unwrap_or(""),
        story.summary.as_deref().unwrap_or(""),
        &refs.iter().map(|r| r.name.clone()).collect::<Vec<_>>(),
        analysis.is_some(),
    );

    // Ours, not theirs: no credit line under a card we generated.
    let own_card = image_url.contains("/og/");
    let (image_credit, image_credit_url) = if own_card {
        (String::new(), String::new())
    } else {
        (image_credit, image_credit_url)
    };

    Ok(Some(StoryPage {
        slug: story.slug.clone(),
        image_url,
        image_credit,
        image_credit_url,
        share_image,
        share_description,
        square_card,
        video_id: story.video_id.clone().unwrap_or_default(),
        headline,
        dek: article
            .as_ref()
            .map(|a| a.dek.clone())
            .filter(|d| !d.is_empty())
            .or(story.summary.clone())
            .unwrap_or_default(),
        body_html: article
            .as_ref()
            .map(|a| render_body(&a.body_md))
            .unwrap_or_default(),
        category_label: category_label.into(),
        published_at: published.format("%B %-d, %Y at %H:%M UTC").to_string(),
        ago: ago(published),
        reading_time_min: article
            .as_ref()
            .map(|a| (a.reading_time_s / 60).max(1))
            .unwrap_or(1),
        kind: story.kind.as_str().into(),
        claims: claim_cards,
        quotes: quote_cards,
        analysis: analysis_card,
        sources: refs
            .iter()
            .map(|r| SourceCard {
                name: r.name.clone(),
                slug: r.slug.clone(),
                url: r.url.clone(),
                title: r.title.clone(),
                trust: r.trust,
            })
            .collect(),
        corrections: corrections
            .iter()
            .map(|c| CorrectionCard {
                reason: c.reason.clone(),
                issued_at: c.issued_at.format("%b %-d, %Y").to_string(),
                from_version: c.from_version,
                to_version: c.to_version,
            })
            .collect(),
        runs: runs.iter().map(run_line).collect(),
        assets: story.assets.clone(),
        json_ld,
        canonical,
        iso_published: published.to_rfc3339(),
        iso_modified: story.updated_at.to_rfc3339(),
    }))
}

#[cfg(feature = "ssr")]
fn run_line(r: &bg_core::domain::AgentRunSummary) -> RunLine {
    RunLine {
        role: r.role.as_str().into(),
        role_name: r.role.display_name().into(),
        status: r.status.as_str().into(),
        model: r.model.clone(),
        cost: format!("${:.4}", r.cost_usd),
        latency_ms: r.latency_ms,
        at: ago(r.started_at),
        note: r.note.clone(),
    }
}

// ---------------------------------------------------------------------------
// the flock
// ---------------------------------------------------------------------------

#[server(name = GetFlock, prefix = "/rpc")]
pub async fn get_flock() -> Result<FlockPage, ServerFnError> {
    let db = db();
    let stats = bg_db::agents::flock_stats(db).await.map_err(e)?;
    let totals = bg_db::agents::newsroom_totals(db).await.map_err(e)?;
    let recent = bg_db::agents::recent_runs(db, 30).await.map_err(e)?;
    let blocks = bg_db::violations::count_blocks_24h(db).await.unwrap_or(0);

    // The same numbers the worker enforces with, computed from the same
    // ledger. Read from the environment so the page states the mandate that is
    // actually in force rather than a default compiled in months ago.
    let budget_ccc: bg_core::mandate::Wei = std::env::var("BG_AGENT_BUDGET_CCC")
        .ok()
        .and_then(|v| v.trim().parse::<f64>().ok())
        .filter(|c| *c >= 0.0 && c.is_finite())
        .map(|c| (c * bg_core::mandate::CCC as f64) as u128)
        .unwrap_or(bg_core::mandate::CCC / 10);

    Ok(FlockPage {
        agents: stats
            .iter()
            .map(|a| AgentCard {
                role: a.role.as_str().into(),
                name: a.name.clone(),
                beat: a.role.beat().into(),
                tier: a.role.tier().as_str().into(),
                runs_24h: a.runs_24h,
                failed_24h: a.failed_24h,
                tokens_24h: a.tokens_24h,
                cost_24h: format!("${:.4}", a.cost_24h_usd),
                avg_latency_ms: a.avg_latency_ms,
                last_note: a.last_note.clone(),
                // Only when it is mostly failing. An agent turned away by the
                // rate limiter now and then is a free tier working as designed,
                // and labelling that "trouble" would train the reader — and us
                // — to ignore the label.
                trouble: if bg_core::trouble::is_troubled(a.ok_24h, a.failed_24h) {
                    a.last_error.as_deref().map(|raw| {
                        bg_core::trouble::explain(raw)
                            .map(str::to_string)
                            // Unrecognised: show what the provider actually
                            // said rather than a summary we made up.
                            .unwrap_or_else(|| raw.chars().take(140).collect())
                    })
                } else {
                    None
                },
                enabled: a.enabled,
                mandate_budget: bg_core::mandate::format_ccc(budget_ccc),
                mandate_spent: bg_core::mandate::format_ccc(bg_core::mandate::tokens_to_ccc(
                    a.tokens_24h.max(0) as u64,
                    bg_core::mandate::DEFAULT_CCC_PER_MTOK,
                )),
                // A zero budget reads as fully spent: the bar is a claim about
                // how much of an authorisation is gone, and an agent with no
                // authorisation has no room left.
                mandate_pct: bg_core::mandate::tokens_to_ccc(
                    a.tokens_24h.max(0) as u64,
                    bg_core::mandate::DEFAULT_CCC_PER_MTOK,
                )
                .checked_mul(100)
                .and_then(|spent| spent.checked_div(budget_ccc))
                .map_or(100, |pct| pct.min(100) as i32),
            })
            .collect(),
        recent: recent.iter().map(run_line).collect(),
        runs_24h: totals.runs_24h,
        failures_24h: totals.failures_24h,
        tokens_24h: totals.tokens_24h,
        cost_24h: format!("${:.4}", totals.cost_24h),
        published_24h: totals.stories_published_24h,
        claims_24h: totals.claims_24h,
        blocks_24h: blocks,
    })
}

// ---------------------------------------------------------------------------
// markets
// ---------------------------------------------------------------------------

#[server(name = GetPrices, prefix = "/rpc")]
pub async fn get_prices() -> Result<PricesPage, ServerFnError> {
    let db = db();
    let ticks = bg_db::prices::latest_all(db).await.map_err(e)?;
    let assets = bg_db::prices::assets(db).await.unwrap_or_default();

    let mut rows = Vec::new();
    for t in &ticks {
        let name = assets
            .iter()
            .find(|a| a.symbol == t.symbol)
            .map(|a| a.name.clone())
            .unwrap_or_else(|| t.symbol.clone());
        let story_count = bg_db::stories::by_asset(db, &t.symbol, 100)
            .await
            .map(|v| v.len() as i64)
            .unwrap_or(0);
        rows.push(PriceRow {
            symbol: t.symbol.clone(),
            name,
            price: fmt_price(t.price_usd.to_string().parse().unwrap_or(0.0)),
            change: t.change_24h_pct,
            market_cap: t
                .market_cap
                .map(|m| compact_usd(m.to_string().parse().unwrap_or(0.0))),
            volume: t
                .volume_24h
                .map(|v| compact_usd(v.to_string().parse().unwrap_or(0.0))),
            story_count,
        });
    }
    Ok(PricesPage { ticks: rows })
}

#[server(name = GetAsset, prefix = "/rpc")]
pub async fn get_asset(
    ticker: String,
) -> Result<(Option<PriceRow>, Vec<StoryCard>), ServerFnError> {
    let db = db();
    let stories = bg_db::stories::by_asset(db, &ticker, 40).await.map_err(e)?;
    let tick = bg_db::prices::latest(db, &ticker).await.map_err(e)?;
    let assets = bg_db::prices::assets(db).await.unwrap_or_default();

    let row = tick.map(|t| PriceRow {
        symbol: t.symbol.clone(),
        name: assets
            .iter()
            .find(|a| a.symbol == t.symbol)
            .map(|a| a.name.clone())
            .unwrap_or_else(|| t.symbol.clone()),
        price: fmt_price(t.price_usd.to_string().parse().unwrap_or(0.0)),
        change: t.change_24h_pct,
        market_cap: t
            .market_cap
            .map(|m| compact_usd(m.to_string().parse().unwrap_or(0.0))),
        volume: t
            .volume_24h
            .map(|v| compact_usd(v.to_string().parse().unwrap_or(0.0))),
        story_count: stories.len() as i64,
    });

    let mut cards: Vec<StoryCard> = stories.iter().map(|s| card(s, None)).collect();
    flag_analysis(db, &stories, &mut cards).await;
    Ok((row, cards))
}

// ---------------------------------------------------------------------------
// standards & trends
// ---------------------------------------------------------------------------

#[server(name = GetStandards, prefix = "/rpc")]
pub async fn get_standards() -> Result<StandardsPage, ServerFnError> {
    let db = db();
    let health = bg_db::sources::health(db).await.map_err(e)?;
    let all = bg_db::sources::all(db).await.map_err(e)?;

    Ok(StandardsPage {
        sources: health
            .iter()
            .map(|h| {
                let meta = all.iter().find(|s| s.slug == h.slug);
                SourceHealthRow {
                    slug: h.slug.clone(),
                    name: h.name.clone(),
                    homepage: meta.map(|m| m.homepage.clone()).unwrap_or_default(),
                    trust: meta.map(|m| m.trust).unwrap_or(50),
                    items: h.items,
                    enabled: h.enabled,
                    robots_ok: h.robots_ok,
                    healthy: h.last_error.is_none(),
                }
            })
            .collect(),
        enforcement: bg_db::violations::tally(db, 30).await.unwrap_or_default(),
        blocks_24h: bg_db::violations::count_blocks_24h(db).await.unwrap_or(0),
        max_quote_words: bg_core::policy::MAX_QUOTE_WORDS,
        max_verbatim_run: bg_core::policy::MAX_VERBATIM_RUN,
        min_desk_sources: bg_core::policy::MIN_DESK_SOURCES,
    })
}

#[server(name = GetFlyway, prefix = "/rpc")]
pub async fn get_flyway(language: String) -> Result<FlywayPage, ServerFnError> {
    use std::collections::BTreeMap;
    use std::str::FromStr;
    let db = db();
    const DAYS: i32 = 14;
    let language = bg_core::domain::EditorialLanguage::from_str(&language)
        .unwrap_or(bg_core::domain::EditorialLanguage::En);

    let rows = bg_db::stories::flyway_for_language(db, language, DAYS)
        .await
        .map_err(e)?;
    let mut by_cat: BTreeMap<String, BTreeMap<chrono::NaiveDate, i64>> = BTreeMap::new();
    for (cat, day, n) in rows {
        *by_cat.entry(cat).or_default().entry(day).or_insert(0) += n;
    }

    // A dense series: days with no coverage must render as gaps, not be
    // dropped, or the chart implies continuous activity that did not happen.
    let today = chrono::Utc::now().date_naive();
    let mut categories: Vec<CategoryTrend> = by_cat
        .into_iter()
        .map(|(cat, days)| {
            let series: Vec<i64> = (0..DAYS)
                .rev()
                .map(|d| {
                    let day = today - chrono::Duration::days(d as i64);
                    days.get(&day).copied().unwrap_or(0)
                })
                .collect();
            let label = bg_core::domain::Category::from_str(&cat)
                .map(|c| category_label(c, language).to_string())
                .unwrap_or_else(|_| cat.clone());
            CategoryTrend {
                category: cat,
                label,
                total: series.iter().sum(),
                series,
            }
        })
        .collect();
    categories.sort_by_key(|c| std::cmp::Reverse(c.total));

    let entities = bg_db::entities::trending_for_language(db, language, DAYS, 14)
        .await
        .unwrap_or_default()
        .into_iter()
        .map(|(ent, n)| (ent.name, ent.slug, n))
        .collect();

    let topics = bg_db::gaggles::live_for_language(db, language, 24 * 14, 20)
        .await
        .unwrap_or_default()
        .into_iter()
        .map(|g| GaggleCard {
            slug: g.slug,
            title: g.title,
            standfirst: g.standfirst,
            sources: g.source_count,
            stories: g.story_count,
            model: g.model.unwrap_or_default(),
            language: g.editorial_language,
            pinned: g.pinned,
            analysis_html: render_body(&g.analysis_md),
            watchpoints: g.watchpoints,
            primary_sources: g
                .primary_source_names
                .into_iter()
                .zip(g.primary_source_urls)
                .map(|(name, url)| TopicSource { name, url })
                .collect(),
            last_updated: g
                .last_briefed_at
                .map(|at| at.to_rfc3339())
                .unwrap_or_default(),
        })
        .collect();

    Ok(FlywayPage {
        categories,
        entities,
        topics,
        days: DAYS,
    })
}
