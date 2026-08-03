//! Page components.

use crate::api::*;
use crate::model::*;
use crate::ui::*;
use leptos::prelude::*;
use leptos_meta::{Link, Meta, Title};
use leptos_router::hooks::use_params_map;

/// Load a resource and render it, with loading and empty states handled once.
macro_rules! loaded {
    ($res:expr, |$v:ident| $body:expr) => {
        view! {
            <Suspense fallback=|| view! { <Loading /> }>
                {move || {
                    $res.get()
                        .map(|r| match r {
                            Ok($v) => $body.into_any(),
                            Err(e) => {
                                view! {
                                    <Empty
                                        message="Could not load this page."
                                        hint=e.to_string()
                                    />
                                }
                                    .into_any()
                            }
                        })
                }}
            </Suspense>
        }
    };
}

// ---------------------------------------------------------------------------
// home
// ---------------------------------------------------------------------------

#[component]
pub fn Home() -> impl IntoView {
    Front(FrontProps { beat: None })
}

/// The AI desk.
#[component]
pub fn DeskAi() -> impl IntoView {
    Front(FrontProps { beat: Some("ai") })
}

/// The crypto desk.
#[component]
pub fn DeskCrypto() -> impl IntoView {
    Front(FrontProps {
        beat: Some("crypto"),
    })
}

/// The capital-markets desk.
#[component]
pub fn DeskMarkets() -> impl IntoView {
    Front(FrontProps {
        beat: Some("markets"),
    })
}

/// The high-technology desk.
#[component]
pub fn DeskTech() -> impl IntoView {
    Front(FrontProps { beat: Some("tech") })
}

/// The front page, blended or for one desk.
///
/// One component rather than three: a desk page *is* the front page with a
/// filter, and forking it would guarantee the two drift.
#[component]
fn Front(#[prop(optional)] beat: Option<&'static str>) -> impl IntoView {
    let data = Resource::new(move || beat, |b| get_front_page(b.map(|s| s.to_string())));
    let (title, blurb) = match beat {
        Some("ai") => (
            "AI — BitGoose",
            "Frontier AI: models, research, compute and policy, with every claim showing its sources.",
        ),
        Some("crypto") => (
            "Crypto — BitGoose",
            "Crypto markets, protocols and policy, with every claim showing its sources.",
        ),
        Some("markets") => (
            "Markets — BitGoose",
            "Capital markets: equities, rates, macro and earnings, with every claim showing \
             its sources.",
        ),
        Some("tech") => (
            "Tech — BitGoose",
            "High technology: chips, platforms, space and energy, with every claim showing \
             its sources.",
        ),
        _ => (
            "BitGoose — The AI-era newsroom",
            "Frontier technology written by AI agents, where every claim shows its sources and confidence.",
        ),
    };

    view! {
        <Title text=title />
        <Meta name="description" content=blurb />
        {loaded!(
            data,
            |fp| view! {
                {fp.honk.clone().map(|h| view! { <HonkBar story=h /> })}
                // The ticker is crypto spot prices. On the AI desk it is not
                // just irrelevant, it is misleading furniture — a reader could
                // reasonably read a price strip as being about what they are
                // reading. Shown on the blended front page and the crypto desk
                // only.
                {matches!(beat, None | Some("crypto"))
                    .then(|| view! { <Ticker prices=fp.prices.clone() /> })}
                <div class="shell page">
                    {match &fp.lead {
                        // No Desk lead. That does not mean nothing to read: a
                        // desk can be running entirely on the Wire, which is
                        // exactly the state a new one starts in. Only show the
                        // empty state when there is genuinely nothing.
                        None if fp.wire.is_empty() => {
                            view! {
                                <Empty
                                    message="Nothing published on this desk yet. Run the newsroom to fill it."
                                    hint="bg run"
                                />
                            }
                                .into_any()
                        }
                        // A desk running entirely on the Wire still deserves a
                        // front page rather than a flat list. Without a Desk
                        // story to lead on, the strongest Wire item is promoted
                        // to the lead slot and the next four become a card row:
                        // a reader arriving here should be able to tell in one
                        // glance what the most important thing is, which an
                        // undifferentiated column of twenty identical rows
                        // cannot do.
                        None => {
                            let mut rest = fp.wire.clone();
                            let promoted = rest.remove(0);
                            let feature: Vec<_> = rest.drain(..rest.len().min(4)).collect();
                            view! {
                                <LeadStory story=promoted />
                                {(!feature.is_empty())
                                    .then(|| {
                                        view! {
                                            <div class="rail-title">
                                                <span>"Also today"</span>
                                            </div>
                                            <div class="card-grid">
                                                {feature
                                                    .into_iter()
                                                    .map(|s| view! { <Card story=s /> })
                                                    .collect_view()}
                                            </div>
                                        }
                                    })}
                                {(!rest.is_empty())
                                    .then(|| {
                                        view! {
                                            <div class="rail-title">
                                                <span>"The Wire"</span>
                                                <a href="/wire">"All"</a>
                                            </div>
                                            <div class="wire-full">
                                                {rest
                                                    .into_iter()
                                                    .map(|s| view! { <WireRow story=s /> })
                                                    .collect_view()}
                                            </div>
                                        }
                                    })}
                            }
                                .into_any()
                        }
                        Some(lead) => {
                            let lead = lead.clone();
                            let desk = fp.desk.clone();
                            let wire = fp.wire.clone();
                            view! {
                                <div class="split">
                                    <div>
                                        <LeadStory story=lead />
                                        <div class="rail-title">
                                            <span>"More from the Desk"</span>
                                            <a href="/desk">"All"</a>
                                        </div>
                                        <div class="card-grid">
                                            {desk
                                                .into_iter()
                                                .map(|s| view! { <Card story=s /> })
                                                .collect_view()}
                                        </div>
                                    </div>
                                    <aside>
                                        <div class="rail-title">
                                            <span>"The Wire"</span>
                                            <a href="/wire">"All"</a>
                                        </div>
                                        {wire
                                            .into_iter()
                                            .map(|s| view! { <Card story=s /> })
                                            .collect_view()}
                                    </aside>
                                </div>
                            }
                                .into_any()
                        }
                    }}
                </div>
            }
        )}
    }
}

#[component]
fn HonkBar(story: StoryCard) -> impl IntoView {
    view! {
        <div class="honk">
            <div class="shell">
                <span class="honk-tag">
                    <span class="honk-dot"></span>
                    "Honk"
                </span>
                <a href=format!("/story/{}", story.slug) class="honk-text">
                    {story.title.clone()}
                </a>
            </div>
        </div>
    }
}

#[component]
fn LeadStory(story: StoryCard) -> impl IntoView {
    view! {
        <article class="lead-story">
            <div class="meta">
                <span class="kicker">{story.category_label.clone()}</span>
                <span class="dot">"·"</span>
                <time>{story.ago.clone()}</time>
                <span class="dot">"·"</span>
                <span class="src-count">
                    <strong>{story.source_count}</strong>
                    " independent sources"
                </span>
            </div>
            <h2>
                <a href=format!("/story/{}", story.slug)>{story.title.clone()}</a>
            </h2>
            {(!story.dek.is_empty()).then(|| view! { <p class="dek">{story.dek.clone()}</p> })}
            <a href=format!("/story/{}", story.slug) class="lead-media-link">
                <SourcedImage
                    url=story.image_url.clone()
                    alt=story.title.clone()
                    credit=story.lead_source.clone()
                    credit_url=story.lead_url.clone()
                    shape="media-lead"
                />
            </a>
        </article>
    }
}

// ---------------------------------------------------------------------------
// listings
// ---------------------------------------------------------------------------

#[component]
pub fn Desk() -> impl IntoView {
    let data = Resource::new(|| (), |_| get_stories("desk".into(), 40));
    view! {
        <Title text="The Desk — BitGoose" />
        <div class="shell page">
            <div class="page-head">
                <h1>"The Desk"</h1>
                <p class="lede">
                    "Original reporting, synthesized across every source we have on the story.
                     Each one decomposes into individual claims you can check."
                </p>
            </div>
            <SectionNav />
            {loaded!(
                data,
                |stories| {
                    if stories.is_empty() {
                        view! {
                            <Empty
                                message="No Desk stories yet. The Desk needs at least two independent sources on one event."
                                hint="bg run"
                            />
                        }
                            .into_any()
                    } else {
                        view! {
                            <div class="card-grid">
                                {stories
                                    .into_iter()
                                    .map(|s| view! { <Card story=s /> })
                                    .collect_view()}
                            </div>
                        }
                            .into_any()
                    }
                }
            )}
        </div>
    }
}

#[component]
pub fn Wire() -> impl IntoView {
    let data = Resource::new(|| (), |_| get_stories("wire".into(), 60));
    view! {
        <Title text="The Wire — BitGoose" />
        <div class="shell page">
            <div class="page-head">
                <h1>"The Wire"</h1>
                <p class="lede">
                    "Everything crossing the feeds, summarized in our own words and linked
                     straight back to whoever did the reporting."
                </p>
            </div>
            <SectionNav />
            {loaded!(
                data,
                |stories| {
                    if stories.is_empty() {
                        view! { <Empty message="The Wire is empty." hint="bg run" /> }.into_any()
                    } else {
                        view! {
                            <div>
                                {stories
                                    .into_iter()
                                    .map(|s| view! { <WireRow story=s show_beat=true /> })
                                    .collect_view()}
                            </div>
                        }
                            .into_any()
                    }
                }
            )}
        </div>
    }
}

#[component]
fn WireRow(story: StoryCard, #[prop(optional)] show_beat: bool) -> impl IntoView {
    view! {
        <article class="wire-item">
            <time class="wire-time">{story.ago.clone()}</time>
            <a href=format!("/story/{}", story.slug) class="wire-thumb-link" aria-hidden="true" tabindex="-1">
                <SourcedImage
                    url=story.image_url.clone()
                    alt=String::new()
                    credit=story.lead_source.clone()
                    credit_url=story.lead_url.clone()
                    shape="media-thumb"
                    show_credit=false
                />
            </a>
            <div>
                <h3 class="wire-title">
                    <a href=format!("/story/{}", story.slug)>{story.title.clone()}</a>
                </h3>
                {(!story.dek.is_empty())
                    .then(|| view! { <p class="wire-summary">{story.dek.clone()}</p> })}
                <div class="wire-foot">
                    <span class="kicker">{story.category_label.clone()}</span>
                    <KindTag kind=story.source_kind.clone() />
                    // Only on blended surfaces. On /ai every card is AI, and a
                    // tag repeated down the whole page is noise that competes
                    // with the one tag that does carry information.
                    {show_beat.then(|| view! { <BeatTag beat=story.beat.clone() /> })}
                    {(!story.lead_source.is_empty())
                        .then(|| {
                            view! {
                                <a
                                    class="chip out"
                                    href=story.lead_url.clone()
                                    target="_blank"
                                    rel="noopener noreferrer"
                                >
                                    {story.lead_source.clone()}
                                </a>
                            }
                        })}
                    {(story.source_count > 1)
                        .then(|| {
                            view! {
                                <span class="src-count">
                                    <strong>{story.source_count}</strong>
                                    " sources"
                                </span>
                            }
                        })}
                </div>
            </div>
        </article>
    }
}

/// A section (desk) page — Markets, Policy, DeFi and so on.
///
/// Every card's kicker links here. Without these pages a reader who wants only
/// policy coverage has no way to get it, which is table stakes for a news site.
#[component]
pub fn Section() -> impl IntoView {
    let params = use_params_map();
    let data = Resource::new(
        move || params.read().get("category").unwrap_or_default(),
        get_section,
    );
    view! {
        {loaded!(
            data,
            |pair| {
                let (label, stories) = pair;
                view! {
                    <Title text=format!("{label} — BitGoose") />
                    <div class="shell page">
                        <div class="page-head">
                            <h1>{label.clone()}</h1>
                            <p class="lede">
                                {format!("Everything the newsroom has filed under {label}.")}
                            </p>
                        </div>
                        <SectionNav />
                        {if stories.is_empty() {
                            view! {
                                <Empty
                                    message="Nothing filed to this section yet."
                                    hint="bg run"
                                />
                            }
                                .into_any()
                        } else {
                            view! {
                                <div class="card-grid">
                                    {stories
                                        .into_iter()
                                        .map(|s| view! { <Card story=s /> })
                                        .collect_view()}
                                </div>
                            }
                                .into_any()
                        }}
                    </div>
                }
            }
        )}
    }
}

/// Chips for every section. Rendered from the enum so a new desk cannot be
/// added to the domain and silently left out of the navigation.
#[component]
pub fn SectionNav() -> impl IntoView {
    view! {
        <div class="chip-row" style="margin-bottom:1.5rem">
            {bg_core::domain::Category::ALL
                .iter()
                .map(|c| {
                    view! {
                        <a class="chip" href=format!("/section/{}", c.as_str())>
                            {c.label()}
                        </a>
                    }
                })
                .collect_view()}
        </div>
    }
}

// ---------------------------------------------------------------------------
// story
// ---------------------------------------------------------------------------

#[component]
pub fn Story() -> impl IntoView {
    let params = use_params_map();
    let data = Resource::new(
        move || params.read().get("slug").unwrap_or_default(),
        get_story,
    );

    view! {
        {loaded!(
            data,
            |maybe| match maybe {
                None => {
                    view! {
                        <div class="shell page">
                            <Empty message="That story does not exist." hint="" />
                        </div>
                    }
                        .into_any()
                }
                Some(s) => view! { <StoryView story=s /> }.into_any(),
            }
        )}
    }
}

#[component]
fn StoryView(story: StoryPage) -> impl IntoView {
    let claims = story.claims.clone();
    let sources = story.sources.clone();
    let corrections = story.corrections.clone();
    let runs = story.runs.clone();
    let has_claims = !claims.is_empty();

    view! {
        <Title text=format!("{} — BitGoose", story.headline) />
        <StoryMeta story=story.clone() />
        <div class="shell page">
            <div class="split">
                <div>
                    <header class="article-head">
                        <div class="meta">
                            <span class="kicker">{story.category_label.clone()}</span>
                            <span class="dot">"·"</span>
                            <time>{story.published_at.clone()}</time>
                            <span class="dot">"·"</span>
                            <span>{story.reading_time_min}" min read"</span>
                        </div>
                        <h1>{story.headline.clone()}</h1>
                        {(!story.dek.is_empty())
                            .then(|| view! { <p class="dek">{story.dek.clone()}</p> })}
                        <div class="byline">
                            <GooseMark size=18 />
                            <span>"By the BitGoose Flock"</span>
                            <span class="dot">"·"</span>
                            <span class="src-count">
                                <strong>{sources.len()}</strong>
                                " sources"
                            </span>
                            {has_claims
                                .then(|| {
                                    view! {
                                        <>
                                            <span class="dot">"·"</span>
                                            <span class="src-count">
                                                <strong>{claims.len()}</strong>
                                                " verified claims"
                                            </span>
                                        </>
                                    }
                                })}
                        </div>
                    </header>
                    // A video story leads with the player; everything else
                    // leads with the still. Showing both would push the story
                    // itself below the fold for no gain.
                    {if story.video_id.is_empty() {
                        view! {
                            <SourcedImage
                                url=story.image_url.clone()
                                alt=story.headline.clone()
                                credit=story.image_credit.clone()
                                credit_url=story.image_credit_url.clone()
                                shape="media-hero"
                            />
                        }
                            .into_any()
                    } else {
                        view! {
                            <VideoEmbed
                                video_id=story.video_id.clone()
                                title=story.headline.clone()
                                credit=story.image_credit.clone()
                                credit_url=story.image_credit_url.clone()
                            />
                        }
                            .into_any()
                    }}

                    {(!corrections.is_empty())
                        .then(|| {
                            let cs = corrections.clone();
                            view! {
                                <div class="callout mb-1">
                                    <strong>"Corrected. "</strong>
                                    {cs
                                        .into_iter()
                                        .map(|c| {
                                            view! {
                                                <span>
                                                    {c.reason.clone()}
                                                    " ("
                                                    {c.issued_at.clone()}
                                                    ") "
                                                </span>
                                            }
                                        })
                                        .collect_view()}
                                </div>
                            }
                        })}

                    <div class="prose" inner_html=story.body_html.clone()></div>
                </div>

                <aside>
                    {if has_claims {
                        let cl = claims.clone();
                        view! {
                            <div class="ledger">
                                <div class="rail-title">
                                    <span>"Claim ledger"</span>
                                    <span style="color:var(--faint);font-weight:600">
                                        {cl.len()}
                                    </span>
                                </div>
                                <p
                                    style="font-size:.78rem;color:var(--muted);margin:0 0 1rem;line-height:1.5"
                                >
                                    "Every assertion in this story, with the independent sources
                                     behind it. Confidence is capped by how many outlets
                                     confirmed it."
                                </p>
                                {cl
                                    .into_iter()
                                    .map(|c| view! { <ClaimBlock claim=c /> })
                                    .collect_view()}
                            </div>
                        }
                            .into_any()
                    } else {
                        view! {
                            <div class="ledger">
                                <div class="rail-title">
                                    <span>"Sources"</span>
                                </div>
                                <p style="font-size:.82rem;color:var(--muted);line-height:1.55">
                                    "This is a Wire entry — a pointer to reporting done
                                     elsewhere, not original synthesis. Read the original:"
                                </p>
                                <div class="chip-row">
                                    {sources
                                        .clone()
                                        .into_iter()
                                        .map(|s| view! { <SourceChip source=s /> })
                                        .collect_view()}
                                </div>
                            </div>
                        }
                            .into_any()
                    }}
                </aside>
            </div>

            <ProvenanceStrip runs=runs />
        </div>
    }
}

/// Head metadata for a story: canonical URL, OpenGraph, Twitter card, JSON-LD.
///
/// Every one of these is load-bearing for a news property. Without the canonical
/// a story is duplicated across query-string variants; without OpenGraph it
/// shares as a bare URL; without the `NewsArticle` JSON-LD it is invisible to
/// Google News.
#[component]
fn StoryMeta(story: StoryPage) -> impl IntoView {
    let desc = if story.dek.is_empty() {
        story.headline.clone()
    } else {
        story.dek.clone()
    };
    view! {
        <Link rel="canonical" href=story.canonical.clone() />
        <Meta name="description" content=desc.clone() />

        <Meta property="og:type" content="article" />
        <Meta property="og:site_name" content="BitGoose" />
        <Meta property="og:title" content=story.headline.clone() />
        <Meta property="og:description" content=desc.clone() />
        <Meta property="og:url" content=story.canonical.clone() />
        <Meta property="article:published_time" content=story.iso_published.clone() />
        <Meta property="article:modified_time" content=story.iso_modified.clone() />
        <Meta property="article:section" content=story.category_label.clone() />

        // `summary_large_image` was already declared, but with no image to go
        // with it every share fell back to a bare text card.
        {(!story.image_url.is_empty())
            .then(|| {
                view! {
                    <>
                        <Meta property="og:image" content=story.image_url.clone() />
                        <Meta name="twitter:image" content=story.image_url.clone() />
                    </>
                }
            })}
        <Meta name="twitter:card" content="summary_large_image" />
        <Meta name="twitter:title" content=story.headline.clone() />
        <Meta name="twitter:description" content=desc />

        // Rendered as a raw script body: JSON-LD must reach the crawler as
        // literal JSON, and escaping it as text content would break it.
        <script type="application/ld+json" inner_html=story.json_ld.clone()></script>
    }
}

#[component]
fn ClaimBlock(claim: ClaimCard) -> impl IntoView {
    let disputed = claim.disputed_by.clone();
    let sources = claim.sources.clone();
    view! {
        <div class=format!("claim v-{}", claim.verification) id=format!("claim-{}", claim.marker)>
            <div class="claim-head">
                <span class="claim-marker">{claim.marker.clone()}</span>
                <VerificationBadge
                    verification=claim.verification.clone()
                    label=claim.verification_label.clone()
                />
            </div>
            <p class="claim-text">{claim.text.clone()}</p>
            <Meter confidence=claim.confidence verification=claim.verification.clone() />
            <div class="claim-foot">
                <span>{format!("{:.0}% confidence", claim.confidence * 100.0)}</span>
                <span>{sources.len()}" src"</span>
            </div>
            {claim.excerpt.clone().filter(|x| !x.is_empty()).map(|x| {
                view! { <p class="excerpt">"“"{x}"”"</p> }
            })}
            <div class="chip-row" style="margin-top:.5rem">
                {sources.into_iter().map(|s| view! { <SourceChip source=s /> }).collect_view()}
            </div>
            {(!disputed.is_empty())
                .then(|| {
                    view! {
                        <div style="margin-top:.55rem">
                            <span
                                style="font-size:.65rem;text-transform:uppercase;letter-spacing:.1em;color:var(--v-disputed);font-weight:700"
                            >
                                "Contradicted by"
                            </span>
                            <div class="chip-row" style="margin-top:.3rem">
                                {disputed
                                    .into_iter()
                                    .map(|s| view! { <SourceChip source=s /> })
                                    .collect_view()}
                            </div>
                        </div>
                    }
                })}
        </div>
    }
}

/// How this story was produced. No conventional outlet shows this.
#[component]
fn ProvenanceStrip(runs: Vec<RunLine>) -> impl IntoView {
    if runs.is_empty() {
        return None::<AnyView>.into_any();
    }
    view! {
        <section class="mt-2">
            <div class="rail-title">
                <span>"How this story was made"</span>
                <a href="/flock">"The Flock"</a>
            </div>
            <div class="panel scroll-x">
                <div class="activity">
                    {runs
                        .into_iter()
                        .map(|r| {
                            view! {
                                <div class="activity-row">
                                    <span class="activity-role">{r.role_name.clone()}</span>
                                    <span class=format!("status-{}", r.status)>
                                        {r.status.clone()}
                                    </span>
                                    <span class="activity-note">
                                        {r.note.clone().unwrap_or_default()}
                                    </span>
                                    <span class="activity-cost">
                                        {r.cost.clone()}" · "{r.latency_ms}"ms"
                                    </span>
                                </div>
                            }
                        })
                        .collect_view()}
                </div>
            </div>
        </section>
    }
    .into_any()
}

// ---------------------------------------------------------------------------
// the flock
// ---------------------------------------------------------------------------

#[component]
pub fn Flock() -> impl IntoView {
    let data = Resource::new(|| (), |_| get_flock());
    view! {
        <Title text="The Flock — BitGoose" />
        <div class="shell page">
            <div class="page-head">
                <h1>"The Flock"</h1>
                <p class="lede">
                    "Ten AI agents run this newsroom. There are no humans in the publishing path,
                     so here is exactly what each one did, what it cost, and how often it failed.
                     Updated live."
                </p>
            </div>
            {loaded!(
                data,
                |f| {
                    let agents = f.agents.clone();
                    let recent = f.recent.clone();
                    view! {
                        <div class="stat-row">
                            <div class="stat">
                                <div class="stat-label">"Runs · 24h"</div>
                                <div class="stat-value">{f.runs_24h}</div>
                            </div>
                            <div class="stat">
                                <div class="stat-label">"Failures"</div>
                                <div class="stat-value">{f.failures_24h}</div>
                            </div>
                            <div class="stat">
                                <div class="stat-label">"Tokens"</div>
                                <div class="stat-value">{f.tokens_24h}</div>
                            </div>
                            <div class="stat">
                                <div class="stat-label">"Cost"</div>
                                <div class="stat-value gold">{f.cost_24h.clone()}</div>
                            </div>
                            <div class="stat">
                                <div class="stat-label">"Published"</div>
                                <div class="stat-value">{f.published_24h}</div>
                            </div>
                            <div class="stat">
                                <div class="stat-label">"Claims"</div>
                                <div class="stat-value">{f.claims_24h}</div>
                            </div>
                            <div class="stat">
                                <div class="stat-label">"Policy blocks"</div>
                                <div class="stat-value">{f.blocks_24h}</div>
                            </div>
                        </div>

                        <div class="flock-grid">
                            {agents
                                .into_iter()
                                .map(|a| view! { <AgentTile agent=a /> })
                                .collect_view()}
                        </div>

                        <section class="mt-2">
                            <div class="rail-title">
                                <span>"Live activity"</span>
                            </div>
                            <div class="panel scroll-x">
                                <div class="activity">
                                    {recent
                                        .into_iter()
                                        .map(|r| {
                                            view! {
                                                <div class="activity-row">
                                                    <span class="activity-role">
                                                        {r.role_name.clone()}
                                                    </span>
                                                    <span class=format!("status-{}", r.status)>
                                                        {r.status.clone()}
                                                    </span>
                                                    <span class="activity-note">
                                                        {r.note.clone().unwrap_or_default()}
                                                    </span>
                                                    <span class="activity-cost">
                                                        {r.at.clone()}" · "{r.cost.clone()}
                                                    </span>
                                                </div>
                                            }
                                        })
                                        .collect_view()}
                                </div>
                            </div>
                        </section>
                    }
                }
            )}
        </div>
    }
}

#[component]
fn AgentTile(agent: AgentCard) -> impl IntoView {
    let class = if agent.failed_24h > 0 {
        "agent failing"
    } else if agent.runs_24h > 0 {
        "agent active"
    } else {
        "agent"
    };
    view! {
        <div class=class>
            <div class="agent-name">
                <span>{agent.name.clone()}</span>
                <span class="agent-tier">{agent.tier.clone()}</span>
            </div>
            <p class="agent-beat">{agent.beat.clone()}</p>
            <div class="agent-stats">
                <div>
                    <div class="agent-stat-label">"Runs"</div>
                    <div>{agent.runs_24h}</div>
                </div>
                <div>
                    <div class="agent-stat-label">"Failed"</div>
                    <div>{agent.failed_24h}</div>
                </div>
                <div>
                    <div class="agent-stat-label">"Cost"</div>
                    <div>{agent.cost_24h.clone()}</div>
                </div>
            </div>
            {agent
                .last_note
                .clone()
                .map(|n| view! { <p class="agent-note">"Last: "{n}</p> })}
        </div>
    }
}

// ---------------------------------------------------------------------------
// markets
// ---------------------------------------------------------------------------

#[component]
pub fn Prices() -> impl IntoView {
    let data = Resource::new(|| (), |_| get_prices());
    view! {
        <Title text="Markets — BitGoose" />
        <div class="shell page">
            <div class="page-head">
                <h1>"Markets"</h1>
                <p class="lede">
                    "Live prices, and how much coverage each asset is getting right now."
                </p>
            </div>
            {loaded!(
                data,
                |p| {
                    if p.ticks.is_empty() {
                        view! { <Empty message="No market data yet." hint="bg prices" /> }
                            .into_any()
                    } else {
                        view! {
                            <div class="panel scroll-x">
                                <table>
                                    <thead>
                                        <tr>
                                            <th>"Asset"</th>
                                            <th class="n">"Price"</th>
                                            <th class="n">"24h"</th>
                                            <th class="n">"Market cap"</th>
                                            <th class="n">"Volume"</th>
                                            <th class="n">"Stories"</th>
                                        </tr>
                                    </thead>
                                    <tbody>
                                        {p
                                            .ticks
                                            .into_iter()
                                            .map(|t| {
                                                view! {
                                                    <tr>
                                                        <td>
                                                            <a href=format!("/asset/{}", t.symbol)>
                                                                <strong>{t.symbol.clone()}</strong>
                                                                " "
                                                                <span style="color:var(--muted)">
                                                                    {t.name.clone()}
                                                                </span>
                                                            </a>
                                                        </td>
                                                        <td class="n">"$"{t.price.clone()}</td>
                                                        <td class="n">
                                                            <Change value=t.change />
                                                        </td>
                                                        <td class="n">
                                                            {t.market_cap.clone().unwrap_or("—".into())}
                                                        </td>
                                                        <td class="n">
                                                            {t.volume.clone().unwrap_or("—".into())}
                                                        </td>
                                                        <td class="n">{t.story_count}</td>
                                                    </tr>
                                                }
                                            })
                                            .collect_view()}
                                    </tbody>
                                </table>
                            </div>
                        }
                            .into_any()
                    }
                }
            )}
        </div>
    }
}

#[component]
pub fn Asset() -> impl IntoView {
    let params = use_params_map();
    let data = Resource::new(
        move || params.read().get("ticker").unwrap_or_default(),
        get_asset,
    );
    view! {
        {loaded!(
            data,
            |pair| {
                let (price, stories) = pair;
                let symbol = price
                    .as_ref()
                    .map(|p| p.symbol.clone())
                    .unwrap_or_else(|| "Asset".into());
                view! {
                    <Title text=format!("{symbol} — BitGoose") />
                    <div class="shell page">
                        <div class="page-head">
                            <h1>
                                {price
                                    .as_ref()
                                    .map(|p| format!("{} · {}", p.symbol, p.name))
                                    .unwrap_or(symbol.clone())}
                            </h1>
                            {price
                                .as_ref()
                                .map(|p| {
                                    view! {
                                        <p class="lede">
                                            <span class="price" style="font-size:1.5rem;color:var(--paper)">
                                                "$"{p.price.clone()}
                                            </span>
                                            " "
                                            <Change value=p.change />
                                        </p>
                                    }
                                })}
                        </div>
                        {if stories.is_empty() {
                            view! {
                                <Empty
                                    message="No coverage for this asset yet."
                                    hint=""
                                />
                            }
                                .into_any()
                        } else {
                            view! {
                                <div class="card-grid">
                                    {stories
                                        .into_iter()
                                        .map(|s| view! { <Card story=s /> })
                                        .collect_view()}
                                </div>
                            }
                                .into_any()
                        }}
                    </div>
                }
            }
        )}
    }
}

// ---------------------------------------------------------------------------
// flyway
// ---------------------------------------------------------------------------

#[component]
pub fn Flyway() -> impl IntoView {
    let data = Resource::new(|| (), |_| get_flyway());
    view! {
        <Title text="Flyway — BitGoose" />
        <div class="shell page">
            <div class="page-head">
                <h1>"Flyway"</h1>
                <p class="lede">
                    "Which stories are migrating up. Coverage volume by desk over the last two
                     weeks, and the names showing up most often."
                </p>
            </div>
            {loaded!(
                data,
                |f| {
                    if f.categories.is_empty() {
                        view! {
                            <Empty message="Not enough published history yet." hint="bg run" />
                        }
                            .into_any()
                    } else {
                        let cats = f.categories.clone();
                        let ents = f.entities.clone();
                        view! {
                            <div class="split">
                                <div>
                                    {cats
                                        .into_iter()
                                        .map(|c| view! { <TrendRow trend=c /> })
                                        .collect_view()}
                                </div>
                                <aside>
                                    <div class="rail-title">
                                        <span>"In the news"</span>
                                    </div>
                                    {if ents.is_empty() {
                                        view! {
                                            <p class="loading">
                                                "No entities linked yet."
                                            </p>
                                        }
                                            .into_any()
                                    } else {
                                        view! {
                                            <div class="chip-row">
                                                {ents
                                                    .into_iter()
                                                    .map(|(name, _slug, n)| {
                                                        view! {
                                                            <span class="chip">
                                                                {name}
                                                                <span class="chip-trust">{n}</span>
                                                            </span>
                                                        }
                                                    })
                                                    .collect_view()}
                                            </div>
                                        }
                                            .into_any()
                                    }}
                                </aside>
                            </div>
                        }
                            .into_any()
                    }
                }
            )}
        </div>
    }
}

#[component]
fn TrendRow(trend: CategoryTrend) -> impl IntoView {
    let peak = trend.series.iter().copied().max().unwrap_or(1).max(1);
    view! {
        <div style="padding:.9rem 0;border-bottom:1px solid var(--line-soft)">
            <div
                style="display:flex;justify-content:space-between;align-items:baseline;margin-bottom:.5rem"
            >
                <strong style="font-family:var(--serif);font-size:1.05rem">
                    {trend.label.clone()}
                </strong>
                <span class="num" style="color:var(--muted);font-size:.8rem">
                    {trend.total}" stories"
                </span>
            </div>
            <div style="display:flex;align-items:flex-end;gap:3px;height:44px">
                {trend
                    .series
                    .iter()
                    .map(|v| {
                        // Zero days keep a 2px stub so the gap is visible as a
                        // gap rather than as missing data.
                        let h = if *v == 0 { 2 } else { (*v * 44 / peak).max(4) };
                        let bg = if *v == 0 { "var(--line)" } else { "var(--gold)" };
                        view! {
                            <div
                                style=format!(
                                    "flex:1;height:{h}px;background:{bg};border-radius:1px",
                                )
                                title=format!("{v} stories")
                            ></div>
                        }
                    })
                    .collect_view()}
            </div>
        </div>
    }
}

// ---------------------------------------------------------------------------
// standards
// ---------------------------------------------------------------------------

#[component]
pub fn Standards() -> impl IntoView {
    let data = Resource::new(|| (), |_| get_standards());
    view! {
        <Title text="Standards — BitGoose" />
        <div class="shell page">
            <div class="page-head">
                <h1>"Editorial standards"</h1>
                <p class="lede">
                    "BitGoose is written entirely by AI agents. That only deserves your trust if
                     the rules are mechanical and the record is public, so both are."
                </p>
            </div>

            <div class="split">
                <div>
                    <div class="prose" style="max-width:none">
                        <h2>"What we publish"</h2>
                        <p>
                            "We read other people's journalism. We do not republish it. Source
                             text is stored privately for analysis and never served. Everything
                             on this site is original synthesis, with a link out to every source
                             it drew on."
                        </p>

                        <h2>"How the rules are enforced"</h2>
                        <p>
                            "These are not guidelines an agent is asked to follow. They are
                             checked in code on the path to publication, and a draft that fails
                             any of them cannot be published — the attempt is recorded instead."
                        </p>
                        {loaded!(
                            data,
                            |s| {
                                view! {
                                    <ul>
                                        <li>
                                            "Quotes are capped at "<strong>{s.max_quote_words}</strong>
                                            " words, attributed, with a link out."
                                        </li>
                                        <li>
                                            "No run longer than "<strong>{s.max_verbatim_run}</strong>
                                            " words may match any source, which catches lifted
                                             wording even when it was never marked as a quote."
                                        </li>
                                        <li>"Every claim carries at least one source, or it does not ship."</li>
                                        <li>"A refuted claim can never appear in published prose."</li>
                                        <li>
                                            "An original story needs at least "
                                            <strong>{s.min_desk_sources}</strong>
                                            " independent sources."
                                        </li>
                                        <li>"Confidence is capped by source count — one outlet is never 'corroborated'."</li>
                                        <li>"Corrections are append-only. We never silently edit a published page."</li>
                                    </ul>
                                }
                            }
                        )}

                        <h2>"Who writes this"</h2>
                        <p>
                            "Ten agents, each with one job, listed on "<a href="/flock">"The Flock"</a>
                            " along with their running cost and error rate. Every story also shows
                             which agents touched it."
                        </p>
                    </div>
                </div>

                <aside>
                    {loaded!(
                        data,
                        |s| {
                            let sources = s.sources.clone();
                            let enf = s.enforcement.clone();
                            view! {
                                <div class="rail-title">
                                    <span>"Enforcement · 30 days"</span>
                                </div>
                                {if enf.is_empty() {
                                    view! {
                                        <p class="loading">
                                            "No violations recorded."
                                        </p>
                                    }
                                        .into_any()
                                } else {
                                    view! {
                                        <div class="panel mb-1">
                                            <table>
                                                <tbody>
                                                    {enf
                                                        .into_iter()
                                                        .map(|(code, n)| {
                                                            view! {
                                                                <tr>
                                                                    <td style="font-family:var(--mono);font-size:.78rem">
                                                                        {code}
                                                                    </td>
                                                                    <td class="n">{n}</td>
                                                                </tr>
                                                            }
                                                        })
                                                        .collect_view()}
                                                </tbody>
                                            </table>
                                        </div>
                                    }
                                        .into_any()
                                }}

                                <div class="rail-title">
                                    <span>"Sources"</span>
                                    <span style="color:var(--faint);font-weight:600">
                                        {sources.len()}
                                    </span>
                                </div>
                                <div class="panel">
                                    <table>
                                        <thead>
                                            <tr>
                                                <th>"Outlet"</th>
                                                <th class="n">"Trust"</th>
                                                <th class="n">"Items"</th>
                                            </tr>
                                        </thead>
                                        <tbody>
                                            {sources
                                                .into_iter()
                                                .map(|s| {
                                                    let mark = if !s.robots_ok {
                                                        "robots"
                                                    } else if !s.healthy {
                                                        "stale"
                                                    } else {
                                                        ""
                                                    };
                                                    view! {
                                                        <tr>
                                                            <td>
                                                                <a
                                                                    href=s.homepage.clone()
                                                                    target="_blank"
                                                                    rel="noopener noreferrer"
                                                                    class="out"
                                                                >
                                                                    {s.name.clone()}
                                                                </a>
                                                                {(!mark.is_empty())
                                                                    .then(|| {
                                                                        view! {
                                                                            <span
                                                                                style="margin-left:.4rem;font-size:.62rem;color:var(--v-single);text-transform:uppercase;letter-spacing:.08em"
                                                                            >
                                                                                {mark}
                                                                            </span>
                                                                        }
                                                                    })}
                                                            </td>
                                                            <td class="n">{s.trust}</td>
                                                            <td class="n">{s.items}</td>
                                                        </tr>
                                                    }
                                                })
                                                .collect_view()}
                                        </tbody>
                                    </table>
                                </div>
                            }
                        }
                    )}
                </aside>
            </div>
        </div>
    }
}

// ---------------------------------------------------------------------------
// developers
// ---------------------------------------------------------------------------

#[component]
pub fn Developers() -> impl IntoView {
    view! {
        <Title text="Developers — BitGoose" />
        <div class="shell page">
            <div class="page-head">
                <h1>"Build on BitGoose"</h1>
                <p class="lede">
                    "The claim graph is the product, so it ships machine-readable. Every story,
                     claim, source and confidence score is available over REST — and over MCP,
                     so an AI agent can query the newsroom as a tool instead of scraping it."
                </p>
            </div>

            <div class="split">
                <div class="prose" style="max-width:none">
                    <h2>"REST"</h2>
                    <p>"Public, unauthenticated, CORS-open."</p>
                    <pre>
                        r#"GET /v1/stories?kind=desk&limit=20
GET /v1/stories/{slug}      # full claim ledger
GET /v1/wire
GET /v1/claims/{id}         # one claim, every source
GET /v1/prices
GET /v1/assets/{ticker}
GET /v1/flock               # live agent cost and error rate
GET /v1/standards           # policy + enforcement record"#
                    </pre>
                    <p>
                        <a href="/v1" class="out">"Browse the API index"</a>
                        " · "
                        <a href="/openapi.json" class="out">"OpenAPI"</a>
                    </p>

                    <h2>"MCP"</h2>
                    <p>
                        "Point an MCP client at "<code>"POST /mcp"</code>
                        ". Five tools: "<code>"search_stories"</code>", "<code>"get_story"</code>
                        ", "<code>"verify_claim"</code>", "<code>"get_prices"</code>" and "
                        <code>"newsroom_status"</code>"."
                    </p>
                    <pre>
                        r#"curl -s localhost:3000/mcp \
    -H 'content-type: application/json' \
  -d '{"jsonrpc":"2.0","id":1,"method":"tools/call",
       "params":{"name":"verify_claim",
                 "arguments":{"query":"exchange froze attacker funds"}}}'"#
                    </pre>
                    <p>
                        <code>"verify_claim"</code>
                        " is the one to reach for. Instead of a headline, it returns the matching
                         claims with their verification state, confidence score, and the
                         independent outlets behind each — so an agent can tell the difference
                         between something two newsrooms confirmed and something one account
                         posted."
                    </p>

                    <h2>"Terms"</h2>
                    <p>
                        "Claims and metadata are freely reusable with attribution to BitGoose.
                         Source text is never redistributed through this API, because it is not
                         ours to redistribute."
                    </p>
                </div>

                <aside>
                    <div class="callout">
                        <strong>"Why an API at all?"</strong>
                        <p style="margin:.6rem 0 0">
                            "Most crypto news is written for people and consumed by machines. We
                             think the honest response is to publish the structure directly
                             rather than making everyone reverse-engineer it out of HTML."
                        </p>
                    </div>
                </aside>
            </div>
        </div>
    }
}

// ---------------------------------------------------------------------------

#[component]
pub fn NotFound() -> impl IntoView {
    view! {
        <Title text="Not found — BitGoose" />
        <div class="shell page">
            <div class="page-head">
                <h1>"Nothing here"</h1>
                <p class="lede">
                    "That page does not exist. Try "<a href="/">"the front page"</a>" or "
                    <a href="/wire">"the Wire"</a>"."
                </p>
            </div>
        </div>
    }
}
