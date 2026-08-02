#![recursion_limit = "1024"]
//! # bg-web — the BitGoose site
//!
//! Leptos, server-rendered and hydrated. This crate's library half compiles to
//! `wasm32-unknown-unknown`, so every native dependency it needs (`bg-db`,
//! `bg-api`, tokio) is optional and gated behind the `ssr` feature.

pub mod api;
pub mod model;
pub mod pages;
pub mod ui;

use leptos::prelude::*;
use leptos_meta::{provide_meta_context, Link, Meta, MetaTags, Stylesheet, Title};
use leptos_router::components::{Route, Router, Routes};
use leptos_router::path;

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
        <html lang="en">
            <head>
                <meta charset="utf-8" />
                <meta name="viewport" content="width=device-width, initial-scale=1" />
                <meta name="color-scheme" content="dark light" />
                <AutoReload options=options.clone() />
                <HydrationScripts options=options.clone() />
                <Stylesheet id="leptos" href="/pkg/bitgoose.css" />
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
        <Title text="BitGoose — The AI newsroom for crypto" />
        <Link rel="alternate" type_="application/rss+xml" href="/feed.xml" attr:title="BitGoose" />
        <Link rel="icon" type_="image/svg+xml" href="/favicon.svg" />
        <Meta
            name="description"
            content="Crypto news written by AI agents, where every claim shows its sources and \
                     confidence. Original reporting on the Desk, fast aggregation on the Wire."
        />
        <Router>
            <ui::Masthead />
            <main>
                <Routes fallback=pages::NotFound>
                    <Route path=path!("/") view=pages::Home />
                    <Route path=path!("/wire") view=pages::Wire />
                    <Route path=path!("/desk") view=pages::Desk />
                    <Route path=path!("/section/:category") view=pages::Section />
                    <Route path=path!("/story/:slug") view=pages::Story />
                    <Route path=path!("/flock") view=pages::Flock />
                    <Route path=path!("/prices") view=pages::Prices />
                    <Route path=path!("/asset/:ticker") view=pages::Asset />
                    <Route path=path!("/flyway") view=pages::Flyway />
                    <Route path=path!("/standards") view=pages::Standards />
                    <Route path=path!("/developers") view=pages::Developers />
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
