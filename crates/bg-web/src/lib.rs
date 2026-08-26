#![recursion_limit = "1024"]
//! # bg-web — the BitGoose site
//!
//! Leptos, server-rendered and hydrated. This crate's library half compiles to
//! `wasm32-unknown-unknown`, so every native dependency it needs (`bg-db`,
//! `bg-api`, tokio) is optional and gated behind the `ssr` feature.

pub mod api;
pub mod model;
#[cfg(feature = "ssr")]
pub mod ogcard;
#[cfg(feature = "ssr")]
pub mod ogroute;
pub mod pages;
pub mod qr;
pub mod ui;
// Server-only: it holds a database handle and an axum layer, neither of which
// belongs in the hydrate bundle.
#[cfg(feature = "ssr")]
pub mod unfurl;

use leptos::prelude::*;
use leptos_meta::{provide_meta_context, HashedStylesheet, Link, MetaTags, Title};
use leptos_router::components::{Route, Router, Routes};
use leptos_router::path;
use leptos_router::SsrMode;

/// The HTML document. cargo-leptos calls this to server-render every page.
pub fn shell(options: LeptosOptions) -> impl IntoView {
    view! {
        <!DOCTYPE html>
        // No `data-theme` here on purpose. Hardcoding one made the stylesheet's
        // `@media (prefers-color-scheme: light)` block unreachable, since the
        // `:root[data-theme=…]` rules are written to override it — a reader who
        // prefers light got dark until they found the toggle. Absent the
        // attribute the media query governs, and the toggle sets it only when
        // someone makes an explicit choice.
        // The active edition sets `lang` through `leptos_meta::Html` inside
        // the router, where the request path is available.
        <html>
            <head>
                <meta charset="utf-8" />
                <meta name="viewport" content="width=device-width, initial-scale=1" />
                <meta name="color-scheme" content="dark light" />
                <AutoReload options=options.clone() />
                <HydrationScripts options=options.clone() />
                // Resolves to /pkg/bitgoose.<hash>.css. The hash comes from
                // hash.txt, which Leptos reads from the directory holding the
                // running binary — so the deploy bundle must ship it next to
                // bin/bg-web or the stylesheet silently 404s.
                <HashedStylesheet options=options.clone() id="leptos" />
                // Restores a saved theme choice before first paint. Doing this
                // from the hydrated app instead would show one theme and then
                // swap it, which is worse than not remembering at all.
                <script inner_html=r#"try{var t=localStorage.getItem('bg-theme');if(t==='light'||t==='dark'){document.documentElement.setAttribute('data-theme',t);}}catch(e){}"#></script>
                <MetaTags />
            </head>
            <body>
                <App />
            </body>
        </html>
    }
}

#[component]
pub fn App() -> impl IntoView {
    provide_meta_context();

    view! {
        <Title text="BitGoose — The AI-era newsroom" />
        <Link rel="alternate" type_="application/rss+xml" href="/feed.xml" attr:title="BitGoose" />
        // An SVG favicon on its own was the whole icon story here, and a great
        // many clients cannot use one. WeChat, iOS, Android and most link
        // unfurlers want a raster icon, and when the site does not offer one
        // they show a generic placeholder — which is exactly the grey chain
        // link a shared BitGoose story rendered as, next to a Reuters link
        // showing its roundel.
        <Link rel="icon" type_="image/png" href="/icon-192.png" sizes="192x192" />
        <Link rel="apple-touch-icon" href="/apple-touch-icon.png" sizes="180x180" />
        <Link rel="manifest" href="/site.webmanifest" />
        // No site-wide description here on purpose: pages set their own via
        // `ShareMeta`, and emitting one at both levels left two in the document
        // — a crawler takes the first, so every shared story was described with
        // the generic site blurb instead of its own.
        <Router>
            <ui::Masthead />
            <main>
                <Routes fallback=pages::NotFound>
                    <Route path=path!("/") view=pages::HomeEn />
                    <Route path=path!("/zh") view=pages::Home />
                    <Route path=path!("/fr") view=pages::HomeFr />
                    <Route path=path!("/es") view=pages::HomeEs />
                    <Route path=path!("/zh-hant") view=pages::HomeZhHant />
                    <Route path=path!("/ja") view=pages::HomeJa />
                    <Route path=path!("/ko") view=pages::HomeKo />
                    <Route path=path!("/ai") view=pages::DeskAiEn />
                    <Route path=path!("/zh/ai") view=pages::DeskAi />
                    <Route path=path!("/fr/ai") view=pages::DeskAiFr />
                    <Route path=path!("/es/ai") view=pages::DeskAiEs />
                    <Route path=path!("/zh-hant/ai") view=pages::DeskAiZhHant />
                    <Route path=path!("/ja/ai") view=pages::DeskAiJa />
                    <Route path=path!("/ko/ai") view=pages::DeskAiKo />
                    <Route path=path!("/crypto") view=pages::DeskCryptoEn />
                    <Route path=path!("/zh/crypto") view=pages::DeskCrypto />
                    <Route path=path!("/fr/crypto") view=pages::DeskCryptoFr />
                    <Route path=path!("/es/crypto") view=pages::DeskCryptoEs />
                    <Route path=path!("/zh-hant/crypto") view=pages::DeskCryptoZhHant />
                    <Route path=path!("/ja/crypto") view=pages::DeskCryptoJa />
                    <Route path=path!("/ko/crypto") view=pages::DeskCryptoKo />
                    <Route path=path!("/markets") view=pages::DeskMarketsEn />
                    <Route path=path!("/zh/markets") view=pages::DeskMarkets />
                    <Route path=path!("/fr/markets") view=pages::DeskMarketsFr />
                    <Route path=path!("/es/markets") view=pages::DeskMarketsEs />
                    <Route path=path!("/zh-hant/markets") view=pages::DeskMarketsZhHant />
                    <Route path=path!("/ja/markets") view=pages::DeskMarketsJa />
                    <Route path=path!("/ko/markets") view=pages::DeskMarketsKo />
                    <Route path=path!("/tech") view=pages::DeskTechEn />
                    <Route path=path!("/zh/tech") view=pages::DeskTech />
                    <Route path=path!("/fr/tech") view=pages::DeskTechFr />
                    <Route path=path!("/es/tech") view=pages::DeskTechEs />
                    <Route path=path!("/zh-hant/tech") view=pages::DeskTechZhHant />
                    <Route path=path!("/ja/tech") view=pages::DeskTechJa />
                    <Route path=path!("/ko/tech") view=pages::DeskTechKo />
                    <Route path=path!("/world") view=pages::DeskWorldEn />
                    <Route path=path!("/zh/world") view=pages::DeskWorld />
                    <Route path=path!("/fr/world") view=pages::DeskWorldFr />
                    <Route path=path!("/es/world") view=pages::DeskWorldEs />
                    <Route path=path!("/zh-hant/world") view=pages::DeskWorldZhHant />
                    <Route path=path!("/ja/world") view=pages::DeskWorldJa />
                    <Route path=path!("/ko/world") view=pages::DeskWorldKo />
                    <Route path=path!("/science") view=pages::DeskScienceEn />
                    <Route path=path!("/zh/science") view=pages::DeskScience />
                    <Route path=path!("/fr/science") view=pages::DeskScienceFr />
                    <Route path=path!("/es/science") view=pages::DeskScienceEs />
                    <Route path=path!("/zh-hant/science") view=pages::DeskScienceZhHant />
                    <Route path=path!("/ja/science") view=pages::DeskScienceJa />
                    <Route path=path!("/ko/science") view=pages::DeskScienceKo />
                    <Route path=path!("/culture") view=pages::DeskCultureEn />
                    <Route path=path!("/zh/culture") view=pages::DeskCulture />
                    <Route path=path!("/fr/culture") view=pages::DeskCultureFr />
                    <Route path=path!("/es/culture") view=pages::DeskCultureEs />
                    <Route path=path!("/zh-hant/culture") view=pages::DeskCultureZhHant />
                    <Route path=path!("/ja/culture") view=pages::DeskCultureJa />
                    <Route path=path!("/ko/culture") view=pages::DeskCultureKo />
                    <Route path=path!("/wire") view=pages::WireEn />
                    <Route path=path!("/zh/wire") view=pages::Wire />
                    <Route path=path!("/fr/wire") view=pages::WireFr />
                    <Route path=path!("/es/wire") view=pages::WireEs />
                    <Route path=path!("/zh-hant/wire") view=pages::WireZhHant />
                    <Route path=path!("/ja/wire") view=pages::WireJa />
                    <Route path=path!("/ko/wire") view=pages::WireKo />
                    <Route path=path!("/gaggle/:slug") view=pages::Gaggle ssr=SsrMode::Async />
                    <Route path=path!("/zh/gaggle/:slug") view=pages::Gaggle ssr=SsrMode::Async />
                    <Route path=path!("/fr/gaggle/:slug") view=pages::Gaggle ssr=SsrMode::Async />
                    <Route path=path!("/es/gaggle/:slug") view=pages::Gaggle ssr=SsrMode::Async />
                    <Route path=path!("/zh-hant/gaggle/:slug") view=pages::Gaggle ssr=SsrMode::Async />
                    <Route path=path!("/ja/gaggle/:slug") view=pages::Gaggle ssr=SsrMode::Async />
                    <Route path=path!("/ko/gaggle/:slug") view=pages::Gaggle ssr=SsrMode::Async />
                    <Route path=path!("/desk") view=pages::DeskEn />
                    <Route path=path!("/zh/desk") view=pages::Desk />
                    <Route path=path!("/fr/desk") view=pages::DeskFr />
                    <Route path=path!("/es/desk") view=pages::DeskEs />
                    <Route path=path!("/zh-hant/desk") view=pages::DeskZhHant />
                    <Route path=path!("/ja/desk") view=pages::DeskJa />
                    <Route path=path!("/ko/desk") view=pages::DeskKo />
                    <Route path=path!("/section/:category") view=pages::SectionEn />
                    <Route path=path!("/zh/section/:category") view=pages::Section />
                    <Route path=path!("/fr/section/:category") view=pages::SectionFr />
                    <Route path=path!("/es/section/:category") view=pages::SectionEs />
                    <Route path=path!("/zh-hant/section/:category") view=pages::SectionZhHant />
                    <Route path=path!("/ja/section/:category") view=pages::SectionJa />
                    <Route path=path!("/ko/section/:category") view=pages::SectionKo />
                    // The one route that must not stream out of order.
                    //
                    // A story's `og:image`, `og:title` and JSON-LD all depend on
                    // data that lives under `Suspense`, and with the default
                    // out-of-order mode the `<head>` is flushed before that data
                    // exists — so none of it reached the initial HTML. Crawlers
                    // for X, Telegram, Discord and Google News do not run JS and
                    // read only that first response, which meant every share of
                    // a BitGoose story rendered as a bare text card no matter
                    // what the page contained. `Async` waits for the data and
                    // sends one complete document.
                    <Route path=path!("/story/:slug") view=pages::Story ssr=SsrMode::Async />
                    <Route path=path!("/zh/story/:slug") view=pages::Story ssr=SsrMode::Async />
                    <Route path=path!("/zh-hant/story/:slug") view=pages::Story ssr=SsrMode::Async />
                    <Route path=path!("/fr/story/:slug") view=pages::Story ssr=SsrMode::Async />
                    <Route path=path!("/es/story/:slug") view=pages::Story ssr=SsrMode::Async />
                    <Route path=path!("/ja/story/:slug") view=pages::Story ssr=SsrMode::Async />
                    <Route path=path!("/ko/story/:slug") view=pages::Story ssr=SsrMode::Async />
                    <Route path=path!("/flock") view=pages::Flock />
                    <Route path=path!("/zh/flock") view=pages::Flock />
                    <Route path=path!("/zh-hant/flock") view=pages::Flock />
                    <Route path=path!("/fr/flock") view=pages::Flock />
                    <Route path=path!("/es/flock") view=pages::Flock />
                    <Route path=path!("/ja/flock") view=pages::Flock />
                    <Route path=path!("/ko/flock") view=pages::Flock />
                    <Route path=path!("/prices") view=pages::Prices />
                    <Route path=path!("/zh/prices") view=pages::Prices />
                    <Route path=path!("/zh-hant/prices") view=pages::Prices />
                    <Route path=path!("/fr/prices") view=pages::Prices />
                    <Route path=path!("/es/prices") view=pages::Prices />
                    <Route path=path!("/ja/prices") view=pages::Prices />
                    <Route path=path!("/ko/prices") view=pages::Prices />
                    <Route path=path!("/asset/:ticker") view=pages::Asset />
                    <Route path=path!("/zh/asset/:ticker") view=pages::Asset />
                    <Route path=path!("/zh-hant/asset/:ticker") view=pages::Asset />
                    <Route path=path!("/fr/asset/:ticker") view=pages::Asset />
                    <Route path=path!("/es/asset/:ticker") view=pages::Asset />
                    <Route path=path!("/ja/asset/:ticker") view=pages::Asset />
                    <Route path=path!("/ko/asset/:ticker") view=pages::Asset />
                    <Route path=path!("/flyway") view=pages::FlywayEn />
                    <Route path=path!("/zh/flyway") view=pages::Flyway />
                    <Route path=path!("/fr/flyway") view=pages::FlywayFr />
                    <Route path=path!("/es/flyway") view=pages::FlywayEs />
                    <Route path=path!("/zh-hant/flyway") view=pages::FlywayZhHant />
                    <Route path=path!("/ja/flyway") view=pages::FlywayJa />
                    <Route path=path!("/ko/flyway") view=pages::FlywayKo />
                    <Route path=path!("/standards") view=pages::Standards />
                    <Route path=path!("/zh/standards") view=pages::Standards />
                    <Route path=path!("/zh-hant/standards") view=pages::Standards />
                    <Route path=path!("/fr/standards") view=pages::Standards />
                    <Route path=path!("/es/standards") view=pages::Standards />
                    <Route path=path!("/ja/standards") view=pages::Standards />
                    <Route path=path!("/ko/standards") view=pages::Standards />
                    <Route path=path!("/developers") view=pages::Developers />
                    <Route path=path!("/zh/developers") view=pages::Developers />
                    <Route path=path!("/zh-hant/developers") view=pages::Developers />
                    <Route path=path!("/fr/developers") view=pages::Developers />
                    <Route path=path!("/es/developers") view=pages::Developers />
                    <Route path=path!("/ja/developers") view=pages::Developers />
                    <Route path=path!("/ko/developers") view=pages::Developers />
                </Routes>
            </main>
            <ui::Footer />
        </Router>
    }
}

/// Client entry point. cargo-leptos wires this up in the hydrate build.
#[cfg(feature = "hydrate")]
#[wasm_bindgen::prelude::wasm_bindgen]
pub fn hydrate() {
    console_error_panic_hook::set_once();
    leptos::mount::hydrate_body(App);
}
