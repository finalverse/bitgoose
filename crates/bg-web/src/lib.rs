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
        <html lang="en" data-theme="dark">
            <head>
                <meta charset="utf-8" />
                <meta name="viewport" content="width=device-width, initial-scale=1" />
                <meta name="color-scheme" content="dark light" />
                <AutoReload options=options.clone() />
                <HydrationScripts options=options.clone() />
                <Stylesheet id="leptos" href="/pkg/bitgoose.css" />
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
