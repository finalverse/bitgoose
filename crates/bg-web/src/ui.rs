//! Shared components.

use crate::model::*;
use leptos::prelude::*;
use leptos_router::components::A;

/// The goose mark. Inline SVG — no external request, scales cleanly, and
/// inherits `currentColor` so it works in both themes without a second asset.
#[component]
pub fn GooseMark(#[prop(default = 26)] size: u32) -> impl IntoView {
    view! {
        <svg
            width=size
            height=size
            viewBox="0 0 32 32"
            fill="none"
            aria-hidden="true"
            role="presentation"
        >
            // head and neck: one continuous stroke, goose-gold
            <path
                d="M21.5 7.5a5 5 0 1 0-7.2 4.5c.6 3.4-.4 5.9-2.6 8.1-2.4 2.4-5.5 3.4-5.5 6.4h17c0-4.2-2.3-6.6-4.4-8.6-1.6-1.5-2.4-3-2.3-5.1a5 5 0 0 0 5-5.3Z"
                fill="var(--gold)"
            />
            // bill
            <path d="M25.5 6.2 30 8l-4.5 1.8Z" fill="var(--gold-warm)" />
            // eye
            <circle cx="20.4" cy="6.6" r="1.05" fill="var(--ink-900)" />
        </svg>
    }
}

#[component]
pub fn Masthead() -> impl IntoView {
    view! {
        <header class="masthead">
            <div class="shell">
                <A href="/" attr:class="brand">
                    <GooseMark size=26 />
                    <span>
                        <span class="brand-bit">"Bit"</span>
                        <span class="brand-goose">"Goose"</span>
                    </span>
                </A>
                <nav class="nav" aria-label="Sections">
                    <A href="/desk">"Desk"</A>
                    <A href="/wire">"Wire"</A>
                    <A href="/prices">"Markets"</A>
                    <A href="/flyway">"Flyway"</A>
                    <A href="/flock">"The Flock"</A>
                    <A href="/standards">"Standards"</A>
                </nav>
                <div class="masthead-right">
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
                            "An AI-run newsroom for crypto. Every story decomposes into claims,
                             and every claim shows the independent sources behind it."
                        </p>
                    </div>
                    <div>
                        <h4>"Read"</h4>
                        <ul>
                            <li><A href="/desk">"The Desk"</A></li>
                            <li><A href="/wire">"The Wire"</A></li>
                            <li><A href="/prices">"Markets"</A></li>
                            <li><A href="/flyway">"Flyway"</A></li>
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
                    <span>"We link out to every source. We never republish their text."</span>
                </div>
            </div>
        </footer>
    }
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
    let href = format!("/story/{}", story.slug);
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
                <a href=format!("/section/{}", story.category) class="kicker">
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
