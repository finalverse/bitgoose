#![recursion_limit = "1024"]
//! The BitGoose server.
//!
//! One binary serves three things on one port: the server-rendered site, the
//! public `/v1` REST API, and the `/mcp` endpoint. Keeping them together means
//! the API is never a stale afterthought — it reads the same database the pages
//! render from, in the same process.

#[cfg(feature = "ssr")]
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    use axum::Router;
    use bg_web::{shell, App};
    use leptos::prelude::*;
    use leptos_axum::{generate_route_list, LeptosRoutes};
    use tower_http::compression::CompressionLayer;
    use tower_http::trace::TraceLayer;
    use tracing_subscriber::EnvFilter;

    let _ = dotenvy::dotenv();
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info,sqlx=warn")),
        )
        .with_target(false)
        .init();

    let database_url = std::env::var("DATABASE_URL")
        .map_err(|_| "DATABASE_URL is not set (copy .env.example to .env)")?;
    let db = bg_db::Db::connect(&database_url).await?;
    db.migrate().await?;
    bg_web::api::set_db(db.clone());

    let conf = get_configuration(None)?;
    let addr = conf.leptos_options.site_addr;
    let leptos_options = conf.leptos_options;
    let routes = generate_route_list(App);

    // The two routers are built separately and merged once both have their
    // state applied. Merging the API router into the Leptos one first would pin
    // the combined state to `()` before `leptos_routes` could set it to
    // `LeptosOptions`, which is a trait-bound error rather than a runtime bug.
    let site = Router::new()
        .leptos_routes(&leptos_options, routes, {
            let opts = leptos_options.clone();
            move || shell(opts.clone())
        })
        .fallback(leptos_axum::file_and_error_handler(shell))
        .with_state(leptos_options);

    let app = Router::new()
        // Explicit API routes are registered ahead of the site's catch-all, so
        // `/v1/...` can never be swallowed by client-side routing.
        .merge(bg_api::router(db.clone()))
        // Password-gated human direction desk. It can increase discovery
        // attention, but cannot publish or bypass the autonomous editors.
        .merge(bg_web::editor::router(db.clone()))
        // Generated share cards. Registered before the site's catch-all so
        // `/og/...` reaches the renderer rather than client-side routing.
        .merge(bg_web::ogroute::router(db.clone()))
        .merge(site)
        // Link unfurlers get a small document instead of the full page.
        //
        // A layer rather than a route, because it has to sit in front of
        // everything and then step aside: for any request that is not from a
        // recognised crawler it calls straight through, so a reader's path is
        // unchanged. Registered after `.merge(site)` so it wraps the site too —
        // `/story/:slug` is the page it matters most for, and that is the
        // Leptos router's.
        .layer(axum::middleware::from_fn_with_state(
            (db, bg_web::unfurl::UnfurlCache::default()),
            bg_web::unfurl::layer,
        ))
        .layer(CompressionLayer::new())
        .layer(TraceLayer::new_for_http());

    tracing::info!("BitGoose listening on http://{addr}");
    tracing::info!("  site  http://{addr}/");
    tracing::info!("  api   http://{addr}/v1");
    tracing::info!("  mcp   http://{addr}/mcp");

    let listener = tokio::net::TcpListener::bind(&addr).await?;
    axum::serve(listener, app.into_make_service()).await?;
    Ok(())
}

/// The binary only exists in the SSR build; the hydrate build compiles the lib
/// to WASM and never links this.
#[cfg(not(feature = "ssr"))]
fn main() {}
