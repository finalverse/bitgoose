//! Live network + database test for the ingestion path.
//!
//! Deliberately hits the real feeds. Mocked HTTP would confirm that our parser
//! agrees with our own fixtures, which is not the failure mode that actually
//! bites: publishers change feed URLs, add redirects, start returning 403 to
//! non-browser agents, or emit timestamps in a format `feed-rs` reads
//! differently. Only real traffic catches that.
//!
//! Tolerant by design — individual sources go down, and that must not fail the
//! build. It asserts on the aggregate: most sources reachable, items landing,
//! no duplicate keys.
//!
//! Skips cleanly when Postgres or the network is unavailable.

use bg_ingest::{feeds, http, market, seed};
use bg_db::Db;

const DEFAULT_URL: &str = "postgres://bitgoose:bitgoose@127.0.0.1:55434/bitgoose_ingest_test";

async fn setup() -> Option<Db> {
    let url = std::env::var("TEST_DATABASE_URL")
        .map(|u| u.replace("bitgoose_test", "bitgoose_ingest_test"))
        .unwrap_or_else(|_| DEFAULT_URL.to_string());
    let (base, dbname) = url.rsplit_once('/')?;
    let admin = Db::connect(&format!("{base}/postgres")).await.ok()?;
    let _ = sqlx::query("SELECT pg_terminate_backend(pid) FROM pg_stat_activity WHERE datname = $1")
        .bind(dbname)
        .execute(&admin.pool)
        .await;
    let _ = sqlx::query(sqlx::AssertSqlSafe(format!("DROP DATABASE IF EXISTS {dbname}")))
        .execute(&admin.pool)
        .await;
    sqlx::query(sqlx::AssertSqlSafe(format!("CREATE DATABASE {dbname}")))
        .execute(&admin.pool)
        .await
        .ok()?;
    admin.pool.close().await;
    let db = Db::connect(&url).await.ok()?;
    db.migrate().await.ok()?;
    Some(db)
}

#[tokio::test]
async fn real_feeds_ingest_end_to_end() {
    let Some(db) = setup().await else {
        eprintln!("SKIP: no Postgres");
        return;
    };
    let client = http::client(http::DEFAULT_UA).unwrap();

    // Bail out politely if this machine has no network.
    if client.get("https://decrypt.co/feed").send().await.is_err() {
        eprintln!("SKIP: no network");
        return;
    }

    let n = seed::seed_sources(&db).await.unwrap();
    assert_eq!(n, 9);
    seed::seed_assets(&db).await.unwrap();
    seed::seed_entities(&db).await.unwrap();

    let srcs = bg_db::sources::all(&db).await.unwrap();
    let reports = feeds::poll_all(&db, &client, &srcs, 4).await;
    assert_eq!(reports.len(), 9);

    let mut ok = 0;
    let mut total_inserted = 0;
    for r in &reports {
        if let Some(e) = &r.error {
            eprintln!("  {:<18} FAILED: {e}", r.source_slug);
            continue;
        }
        ok += 1;
        total_inserted += r.inserted;
        eprintln!(
            "  {:<18} fetched={:<4} inserted={:<4} dupes={:<4} stale={:<4}{}",
            r.source_slug,
            r.fetched,
            r.inserted,
            r.duplicates,
            r.stale,
            if r.is_stale_feed() { "  <- FEED IS STALE" } else { "" }
        );
    }
    eprintln!("{ok}/9 sources parsed, {total_inserted} items inserted");

    assert!(ok >= 6, "expected at least 6 of 9 feeds reachable, got {ok}");
    assert!(total_inserted > 20, "expected a real haul of items, got {total_inserted}");

    // A feed that parses but yields nothing must be flagged, not silently
    // ignored — otherwise a dead source sits in the roster indefinitely.
    for r in reports.iter().filter(|r| r.is_stale_feed()) {
        let s = bg_db::sources::by_slug(&db, &r.source_slug).await.unwrap();
        assert!(
            s.last_error.as_deref().is_some_and(|e| e.contains("freshness window")),
            "stale feed {} was not recorded as unhealthy",
            r.source_slug
        );
    }

    // Every inserted item must be well formed.
    let items = bg_db::items::untriaged(&db, 500).await.unwrap();
    assert_eq!(items.len() as i64, bg_db::items::count(&db).await.unwrap());
    for it in &items {
        assert!(!it.title.trim().is_empty(), "empty title survived ingestion");
        assert!(it.canonical_url.starts_with("http"), "bad url: {}", it.canonical_url);
        assert_eq!(it.url_hash.len(), 64, "url_hash must be a sha256 hex digest");
        assert!(!it.canonical_url.contains("utm_"), "tracking param survived canonicalization");
        assert!(!it.title.contains('<'), "html survived into a title");
    }

    // The dedupe key must actually be unique.
    let distinct: i64 = sqlx::query_scalar("SELECT count(DISTINCT url_hash) FROM raw_items")
        .fetch_one(&db.pool)
        .await
        .unwrap();
    assert_eq!(distinct, items.len() as i64, "duplicate url_hash slipped through");

    // Reload from the database: the first pass stored ETag / Last-Modified
    // validators, and reusing the in-memory rows would send none of them —
    // silently testing an unconditional GET instead of a conditional one.
    let refreshed = bg_db::sources::all(&db).await.unwrap();
    let with_validators =
        refreshed.iter().filter(|s| s.etag.is_some() || s.last_modified.is_some()).count();
    eprintln!("{with_validators}/9 sources returned conditional-GET validators");
    assert!(
        with_validators >= 3,
        "expected several publishers to send ETag/Last-Modified, got {with_validators}"
    );

    let second = feeds::poll_all(&db, &client, &refreshed, 4).await;
    let reinserted: usize = second.iter().map(|r| r.inserted).sum();
    let not_modified = second.iter().filter(|r| r.not_modified).count();
    eprintln!("second pass: {reinserted} new, {not_modified} sources returned 304");
    assert!(
        reinserted < 5,
        "a repeat poll re-inserted {reinserted} items — deduplication is not working"
    );
    assert!(
        not_modified >= 1,
        "no source returned 304 — conditional GET is not reaching the wire"
    );

    // Market data.
    let written = market::refresh(&db, &client).await;
    assert!(written >= 5, "expected prices for the majors, wrote {written}");
    let btc = bg_db::prices::latest(&db, "BTC").await.unwrap().expect("BTC price");
    assert!(btc.price_usd > rust_decimal::Decimal::ZERO);
    eprintln!("BTC = ${}", btc.price_usd);

    db.pool.close().await;
}

#[tokio::test]
async fn robots_is_checked_against_live_hosts() {
    let client = http::client(http::DEFAULT_UA).unwrap();
    if client.get("https://decrypt.co/robots.txt").send().await.is_err() {
        eprintln!("SKIP: no network");
        return;
    }
    // Not asserting a particular verdict — publishers change robots.txt. What
    // matters is that the check completes and returns a decision rather than
    // hanging or panicking on real-world files.
    for url in ["https://decrypt.co/feed", "https://www.coindesk.com/arc/outboundfeeds/rss"] {
        let allowed = bg_ingest::robots::allows(&client, "BitGooseBot", url).await;
        eprintln!("robots {url} -> {allowed}");
    }
}
