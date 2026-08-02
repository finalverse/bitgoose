//! Integration coverage for every repository function.
//!
//! These queries are built at runtime, so the compiler cannot catch a
//! misspelled column or a type that does not line up with the schema — only
//! execution can. The test therefore walks the entire graph
//! (source → item → story → claim → article → correction) and *calls every read
//! path*, including the ones the pipeline uses rarely. A function that is never
//! executed here is a function whose first run is in production.
//!
//! Requires a live Postgres. Set `TEST_DATABASE_URL`, or it falls back to the
//! docker-compose instance. Skips with a warning if nothing is reachable, so
//! `cargo test` on a machine without Docker still passes.

use bg_core::domain::*;
use bg_db::*;
use rust_decimal::Decimal;
use std::str::FromStr;

const DEFAULT_URL: &str = "postgres://bitgoose:bitgoose@127.0.0.1:55434/bitgoose_test";

/// Connect to a scratch database, recreating it so each run starts clean.
async fn setup() -> Option<Db> {
    let url = std::env::var("TEST_DATABASE_URL").unwrap_or_else(|_| DEFAULT_URL.to_string());
    let admin_url = url
        .rsplit_once('/')
        .map(|(base, _)| format!("{base}/postgres"))?;
    let dbname = url.rsplit_once('/')?.1.to_string();

    let admin = match Db::connect(&admin_url).await {
        Ok(d) => d,
        Err(e) => {
            eprintln!("SKIP: no Postgres at {admin_url}: {e}");
            return None;
        }
    };
    // Terminate stragglers so DROP does not block on a leaked connection.
    let _ =
        sqlx::query("SELECT pg_terminate_backend(pid) FROM pg_stat_activity WHERE datname = $1")
            .bind(&dbname)
            .execute(&admin.pool)
            .await;
    let _ = sqlx::query(sqlx::AssertSqlSafe(format!(
        "DROP DATABASE IF EXISTS {dbname}"
    )))
    .execute(&admin.pool)
    .await;
    sqlx::query(sqlx::AssertSqlSafe(format!("CREATE DATABASE {dbname}")))
        .execute(&admin.pool)
        .await
        .expect("create test database");
    admin.pool.close().await;

    let db = Db::connect(&url).await.expect("connect to test database");
    db.migrate().await.expect("migrations must apply cleanly");
    Some(db)
}

#[tokio::test]
async fn the_whole_graph_round_trips() {
    let Some(db) = setup().await else { return };

    // -- schema is live -----------------------------------------------------
    db.ping().await.unwrap();
    assert!(
        db.pgvector_version().await.unwrap().is_some(),
        "pgvector must be installed"
    );
    assert!(!db.server_version().await.unwrap().is_empty());
    assert_eq!(db.counts().await.unwrap().len(), 9);

    // -- sources ------------------------------------------------------------
    let src = sources::upsert(
        &db,
        "decrypt",
        "Decrypt",
        SourceKind::Rss,
        "https://decrypt.co/feed",
        "https://decrypt.co",
        78,
        300,
    )
    .await
    .unwrap();
    let src2 = sources::upsert(
        &db,
        "theblock",
        "The Block",
        SourceKind::Rss,
        "https://theblock.co/rss.xml",
        "https://theblock.co",
        82,
        300,
    )
    .await
    .unwrap();

    // Upsert must be idempotent and must not mint a second row.
    let again = sources::upsert(
        &db,
        "decrypt",
        "Decrypt",
        SourceKind::Rss,
        "https://decrypt.co/feed",
        "https://decrypt.co",
        80,
        300,
    )
    .await
    .unwrap();
    assert_eq!(src.id, again.id);
    assert_eq!(again.trust, 80, "upsert should update trust");
    assert_eq!(sources::all(&db).await.unwrap().len(), 2);
    assert_eq!(sources::by_slug(&db, "decrypt").await.unwrap().id, src.id);

    // A never-polled source is due immediately.
    assert_eq!(sources::due_for_poll(&db, 10).await.unwrap().len(), 2);
    sources::record_success(
        &db,
        src.id,
        Some("W/\"abc\""),
        Some("Mon, 01 Jan 2026 00:00:00 GMT"),
    )
    .await
    .unwrap();
    assert_eq!(
        sources::due_for_poll(&db, 10).await.unwrap().len(),
        1,
        "a just-polled source must not be due again"
    );
    sources::record_failure(&db, src2.id, "connection reset")
        .await
        .unwrap();
    sources::set_robots_ok(&db, src2.id, false).await.unwrap();
    assert!(
        sources::due_for_poll(&db, 10).await.unwrap().is_empty(),
        "a robots-blocked source must never be scheduled"
    );
    sources::set_robots_ok(&db, src2.id, true).await.unwrap();
    sources::set_enabled(&db, "theblock", true).await.unwrap();
    assert_eq!(sources::health(&db).await.unwrap().len(), 2);

    // -- raw items ----------------------------------------------------------
    let mk = |sid, url: &str, title: &str| items::NewItem {
        source_id: sid,
        external_id: None,
        canonical_url: url.to_string(),
        url_hash: format!("{:x}", bg_core::text::simhash64(url)),
        title: title.to_string(),
        dek: None,
        authors: vec!["A. Reporter".into()],
        published_at: chrono::Utc::now(),
        summary_raw: Some("summary".into()),
        body_raw: Some("The exchange said it froze the funds within four minutes.".into()),
        body_hash: Some("h".into()),
        simhash: bg_core::text::simhash64(title),
        lang: "en".into(),
        image_url: None,
        video_id: None,
    };

    let i1 = items::insert_new(
        &db,
        &mk(src.id, "https://a.example/1", "Exchange freezes funds"),
    )
    .await
    .unwrap()
    .expect("first insert returns an id");
    let i2 = items::insert_new(
        &db,
        &mk(src2.id, "https://b.example/1", "Funds frozen at exchange"),
    )
    .await
    .unwrap()
    .unwrap();

    let dupe = items::insert_new(
        &db,
        &mk(src.id, "https://a.example/1", "Exchange freezes funds"),
    )
    .await
    .unwrap();
    assert!(
        dupe.is_none(),
        "a repeat url_hash must be silently skipped, not duplicated"
    );
    assert_eq!(items::count(&db).await.unwrap(), 2);

    assert_eq!(items::untriaged(&db, 10).await.unwrap().len(), 2);
    items::mark_triaged(&db, i1, Some("security"), &["BTC".to_string()], 71)
        .await
        .unwrap();
    items::mark_triaged(&db, i2, Some("security"), &["BTC".to_string()], 68)
        .await
        .unwrap();
    assert!(items::untriaged(&db, 10).await.unwrap().is_empty());
    assert_eq!(items::unclustered(&db, 10).await.unwrap().len(), 2);

    // -- stories ------------------------------------------------------------
    let story = stories::create(
        &db,
        "exchange-freezes-funds",
        StoryKind::Desk,
        "Exchange freezes funds",
        Category::Security,
    )
    .await
    .unwrap();

    // A colliding slug must resolve rather than error.
    let other = stories::create(
        &db,
        "exchange-freezes-funds",
        StoryKind::Wire,
        "Same slug, other event",
        Category::Markets,
    )
    .await
    .unwrap();
    assert_ne!(
        other.slug, story.slug,
        "colliding slugs must be disambiguated"
    );

    items::attach_to_story(&db, i1, story.id, ItemRole::Seed)
        .await
        .unwrap();
    items::attach_to_story(&db, i2, story.id, ItemRole::Corroborating)
        .await
        .unwrap();

    let reloaded = stories::by_id(&db, story.id).await.unwrap();
    assert_eq!(
        reloaded.source_count, 2,
        "source_count must be maintained on attach"
    );
    assert_eq!(
        stories::by_slug(&db, &story.slug).await.unwrap().id,
        story.id
    );
    assert_eq!(items::by_story(&db, story.id).await.unwrap().len(), 2);
    assert_eq!(stories::source_refs(&db, story.id).await.unwrap().len(), 2);
    assert_eq!(
        items::clustering_candidates(&db, 24, 50)
            .await
            .unwrap()
            .len(),
        2
    );

    stories::set_scores(&db, story.id, 77, 2.5).await.unwrap();
    stories::set_summary(&db, story.id, "An exchange froze attacker funds.")
        .await
        .unwrap();
    stories::set_meta(
        &db,
        story.id,
        None,
        Some("BTC"),
        &["BTC".to_string()],
        None,
        None,
    )
    .await
    .unwrap();
    stories::set_kind(&db, story.id, StoryKind::Desk)
        .await
        .unwrap();
    assert!(stories::open(&db, 10)
        .await
        .unwrap()
        .iter()
        .any(|s| s.id == story.id));

    // -- claims -------------------------------------------------------------
    let c1 = claims::insert(
        &db,
        story.id,
        &claims::NewClaim {
            text: "The exchange froze the attacker's funds.".into(),
            kind: ClaimKind::Fact,
            numeric_value: None,
            unit: None,
            as_of: None,
        },
        None,
    )
    .await
    .unwrap();
    let c2 = claims::insert(
        &db,
        story.id,
        &claims::NewClaim {
            text: "Losses total about $70 million.".into(),
            kind: ClaimKind::Figure,
            numeric_value: Some(Decimal::from_str("70000000").unwrap()),
            unit: Some("USD".into()),
            as_of: Some(chrono::Utc::now()),
        },
        None,
    )
    .await
    .unwrap();

    claims::add_source(&db, c1, i1, Stance::Supports, Some("froze the funds"))
        .await
        .unwrap();
    claims::add_source(&db, c1, i2, Stance::Supports, None)
        .await
        .unwrap();
    claims::add_source(&db, c2, i1, Stance::Contradicts, None)
        .await
        .unwrap();

    // An over-long excerpt must be truncated by the repository before it can
    // reach the CHECK constraint.
    let long = (1..=60)
        .map(|n| format!("w{n}"))
        .collect::<Vec<_>>()
        .join(" ");
    claims::add_source(&db, c2, i2, Stance::Supports, Some(&long))
        .await
        .expect("repository must truncate rather than let the DB reject it");

    claims::set_verification(&db, c1, Verification::Corroborated, 0.91)
        .await
        .unwrap();
    claims::set_verification(&db, c2, Verification::SingleSource, 0.55)
        .await
        .unwrap();

    assert_eq!(claims::by_story(&db, story.id).await.unwrap().len(), 2);
    assert_eq!(claims::by_id(&db, c1).await.unwrap().id, c1);

    let counts = claims::source_counts(&db, story.id).await.unwrap();
    let c1_n = counts.iter().find(|(id, _)| *id == c1).unwrap().1;
    let c2_n = counts.iter().find(|(id, _)| *id == c2).unwrap().1;
    assert_eq!(c1_n, 2, "two supporting outlets");
    assert_eq!(
        c2_n, 1,
        "the contradicting source must not count as support"
    );

    let with_src = claims::with_sources(&db, story.id).await.unwrap();
    assert_eq!(with_src.len(), 2);
    let ledger_c1 = with_src.iter().find(|c| c.claim.id == c1).unwrap();
    assert_eq!(ledger_c1.sources.len(), 2);
    assert!(ledger_c1.sources.iter().all(|s| !s.source_name.is_empty()));
    let stored_excerpt = with_src
        .iter()
        .find(|c| c.claim.id == c2)
        .unwrap()
        .sources
        .iter()
        .find_map(|s| s.excerpt.clone())
        .unwrap();
    assert!(
        bg_core::text::word_count(&stored_excerpt) <= bg_core::policy::MAX_QUOTE_WORDS,
        "stored excerpt exceeded the quote cap: {stored_excerpt}"
    );

    // -- articles -----------------------------------------------------------
    let art = articles::insert_version(
        &db,
        story.id,
        &articles::NewArticle {
            headline: "Exchange freezes attacker funds".into(),
            dek: "The venue moved within minutes.".into(),
            slug: story.slug.clone(),
            body_md: "The exchange confirmed the freeze.[^c1]".into(),
            seo_title: "Exchange freezes attacker funds".into(),
            seo_desc: "The venue moved within minutes.".into(),
            content_hash: "abc123".into(),
        },
        None,
    )
    .await
    .unwrap();
    assert_eq!(art.version, 1);
    assert!(art.reading_time_s >= 30, "reading time must have a floor");

    let v2 = articles::insert_version(
        &db,
        story.id,
        &articles::NewArticle {
            headline: "Exchange freezes attacker funds (updated)".into(),
            dek: "The venue moved within minutes.".into(),
            slug: story.slug.clone(),
            body_md: "Revised.[^c1]".into(),
            seo_title: "t".into(),
            seo_desc: "d".into(),
            content_hash: "def456".into(),
        },
        None,
    )
    .await
    .unwrap();
    assert_eq!(
        v2.version, 2,
        "versions must increment without a read-modify-write race"
    );
    assert_eq!(
        articles::latest_for_story(&db, story.id)
            .await
            .unwrap()
            .unwrap()
            .version,
        2
    );
    assert_eq!(articles::by_id(&db, art.id).await.unwrap().id, art.id);

    articles::add_citations(&db, v2.id, &[("c1".to_string(), c1)])
        .await
        .unwrap();
    assert_eq!(articles::citations(&db, v2.id).await.unwrap().len(), 1);

    articles::add_correction(
        &db,
        v2.id,
        1,
        2,
        "Restated the loss figure.",
        "- old\n+ new",
        None,
    )
    .await
    .unwrap();
    assert_eq!(
        articles::corrections_for_story(&db, story.id)
            .await
            .unwrap()
            .len(),
        1
    );

    // -- publish ------------------------------------------------------------
    stories::set_status(&db, story.id, StoryStatus::Published, Some("cleared"))
        .await
        .unwrap();
    articles::publish(&db, v2.id).await.unwrap();
    let published = stories::by_id(&db, story.id).await.unwrap();
    assert_eq!(published.status, StoryStatus::Published);
    assert!(
        published.published_at.is_some(),
        "publishing must stamp published_at"
    );

    assert_eq!(
        stories::published(&db, Some(StoryKind::Desk), 10, 0)
            .await
            .unwrap()
            .len(),
        1
    );
    assert_eq!(stories::published(&db, None, 10, 0).await.unwrap().len(), 1);
    assert_eq!(
        stories::by_category(&db, Category::Security, 10)
            .await
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        stories::by_asset(&db, "btc", 10).await.unwrap().len(),
        1,
        "asset lookup is case-insensitive"
    );
    assert_eq!(stories::front_page(&db, 10).await.unwrap().len(), 1);
    assert!(!stories::flyway(&db, 7).await.unwrap().is_empty());

    let wire = stories::wire(&db, 10, 0).await.unwrap();
    assert_eq!(wire.len(), 1);
    assert_eq!(wire[0].source_count, 2);
    assert!(
        !wire[0].source_name.is_empty(),
        "wire entries must carry their lead source"
    );
    assert!(
        wire[0].source_url.starts_with("http"),
        "wire entries must link out"
    );

    // Unpublishing must clear the timestamp, or the CHECK constraint trips.
    stories::set_status(&db, other.id, StoryStatus::Killed, Some("duplicate"))
        .await
        .unwrap();
    assert!(stories::by_id(&db, other.id)
        .await
        .unwrap()
        .published_at
        .is_none());

    // -- the private-body invariant -----------------------------------------
    let body = items::body_for_analysis(&db, i1).await.unwrap();
    assert!(body.is_some(), "analysis paths can read source text");
    assert_eq!(
        items::bodies_for_story(&db, story.id).await.unwrap().len(),
        2
    );

    let public = items::recent_public(&db, 10).await.unwrap();
    let json = serde_json::to_string(&public).unwrap();
    assert!(
        !json.contains("froze the funds within four minutes"),
        "source body text must never reach a serializable projection"
    );

    // -- agents and the run ledger ------------------------------------------
    for role in AgentRole::ALL {
        agents::upsert(&db, *role, role.display_name(), "sys", 0.2)
            .await
            .unwrap();
    }
    assert_eq!(agents::all(&db).await.unwrap().len(), 10);
    let scribe = agents::by_role(&db, AgentRole::Scribe).await.unwrap();

    let run = agents::start_run(&db, scribe.id, AgentRole::Scribe, Some(story.id), "draft")
        .await
        .unwrap();
    agents::finish_run(
        &db,
        run,
        &agents::RunOutcome {
            status: Some(RunStatus::Ok),
            provider: "stub".into(),
            model: "stub-mid".into(),
            prompt_tokens: 1200,
            completion_tokens: 400,
            cost_usd: Decimal::from_str("0.0125").unwrap(),
            latency_ms: 830,
            note: Some("drafted 2 claims".into()),
            ..Default::default()
        },
    )
    .await
    .unwrap();

    let stats = agents::flock_stats(&db).await.unwrap();
    assert_eq!(
        stats.len(),
        10,
        "every agent appears even with no runs today"
    );
    let s = stats.iter().find(|s| s.role == AgentRole::Scribe).unwrap();
    assert_eq!(s.runs_24h, 1);
    assert_eq!(s.ok_24h, 1);
    assert_eq!(s.tokens_24h, 1600);
    assert_eq!(s.last_note.as_deref(), Some("drafted 2 claims"));
    let idle = stats.iter().find(|s| s.role == AgentRole::Ombuds).unwrap();
    assert_eq!(idle.runs_24h, 0);
    assert_eq!(idle.cost_24h_usd, Decimal::ZERO);

    assert_eq!(agents::recent_runs(&db, 10).await.unwrap().len(), 1);
    assert_eq!(
        agents::runs_for_story(&db, story.id).await.unwrap().len(),
        1
    );
    assert_eq!(
        agents::cost_since(&db, 60).await.unwrap(),
        Decimal::from_str("0.0125").unwrap()
    );

    let totals = agents::newsroom_totals(&db).await.unwrap();
    assert_eq!(totals.runs_24h, 1);
    assert_eq!(totals.claims_24h, 2);
    assert_eq!(totals.stories_published_24h, 1);
    assert_eq!(totals.failures_24h, 0);

    // -- entities -----------------------------------------------------------
    let e = entities::upsert(
        &db,
        EntityKind::Token,
        "Bitcoin",
        "bitcoin",
        Some("btc"),
        &["XBT".into()],
    )
    .await
    .unwrap();
    assert_eq!(
        e.ticker.as_deref(),
        Some("BTC"),
        "tickers are normalized upper-case"
    );
    let e2 = entities::upsert(
        &db,
        EntityKind::Token,
        "Bitcoin",
        "bitcoin",
        None,
        &["₿".into()],
    )
    .await
    .unwrap();
    assert_eq!(e.id, e2.id);
    assert_eq!(e2.aliases.len(), 2, "aliases must union, not overwrite");
    assert_eq!(
        e2.ticker.as_deref(),
        Some("BTC"),
        "a NULL ticker must not erase a known one"
    );
    entities::link_story(&db, e.id, story.id, 0.9)
        .await
        .unwrap();
    assert_eq!(entities::by_slug(&db, "bitcoin").await.unwrap().id, e.id);
    assert_eq!(entities::all(&db).await.unwrap().len(), 1);
    let trending = entities::trending(&db, 7, 10).await.unwrap();
    assert_eq!(trending.len(), 1);
    assert_eq!(trending[0].1, 1);

    // -- prices -------------------------------------------------------------
    prices::upsert_asset(&db, "btc", "Bitcoin", Some("bitcoin"), Some(1))
        .await
        .unwrap();
    prices::upsert_asset(&db, "ETH", "Ethereum", Some("ethereum"), Some(2))
        .await
        .unwrap();
    let now = chrono::Utc::now();
    for (sym, px, cap) in [
        ("BTC", "62994.62", "1200000000000"),
        ("ETH", "3300.10", "400000000000"),
    ] {
        prices::insert_tick(
            &db,
            &PriceTick {
                symbol: sym.into(),
                ts: now,
                price_usd: Decimal::from_str(px).unwrap(),
                change_24h_pct: Some(1.5),
                volume_24h: Some(Decimal::from_str("1000").unwrap()),
                market_cap: Some(Decimal::from_str(cap).unwrap()),
            },
        )
        .await
        .unwrap();
    }
    assert_eq!(prices::assets(&db).await.unwrap().len(), 2);
    let latest = prices::latest_all(&db).await.unwrap();
    assert_eq!(latest.len(), 2);
    assert_eq!(
        latest[0].symbol, "BTC",
        "ticker strip leads with the largest cap"
    );
    assert!(
        prices::latest(&db, "btc").await.unwrap().is_some(),
        "symbol lookup is case-insensitive"
    );
    assert_eq!(prices::history(&db, "BTC", 24).await.unwrap().len(), 1);

    // -- policy violations --------------------------------------------------
    let report = bg_core::policy::PolicyReport {
        violations: vec![bg_core::policy::Violation {
            code: bg_core::policy::ViolationCode::QuoteTooLong,
            severity: bg_core::policy::Severity::Block,
            detail: "quote was 40 words".into(),
            subject: Some(c1.to_string()),
        }],
    };
    assert_eq!(
        violations::record(&db, &report, Some(story.id), Some(v2.id), None)
            .await
            .unwrap(),
        1
    );
    assert_eq!(violations::count_blocks_24h(&db).await.unwrap(), 1);
    assert_eq!(violations::recent(&db, 10).await.unwrap().len(), 1);
    assert_eq!(
        violations::tally(&db, 7).await.unwrap()[0],
        ("quote_too_long".into(), 1)
    );

    db.pool.close().await;
}

#[tokio::test]
async fn an_unknown_enum_token_fails_loudly_instead_of_defaulting() {
    let Some(db) = setup().await else { return };

    let src = sources::upsert(
        &db,
        "x",
        "X",
        SourceKind::Rss,
        "https://x.test/feed",
        "https://x.test",
        50,
        300,
    )
    .await
    .unwrap();
    let story = stories::create(&db, "s", StoryKind::Wire, "T", Category::Markets)
        .await
        .unwrap();

    // Write a value the CHECK constraint does not cover, simulating drift
    // between a future migration and this binary.
    sqlx::query("ALTER TABLE stories DROP CONSTRAINT stories_kind_check")
        .execute(&db.pool)
        .await
        .unwrap();
    sqlx::query("UPDATE stories SET kind = 'newsletter' WHERE id = $1")
        .bind(story.id.into_uuid())
        .execute(&db.pool)
        .await
        .unwrap();

    match stories::by_id(&db, story.id).await {
        Err(DbError::Decode { column, .. }) => assert_eq!(column, "kind"),
        Err(e) => panic!("wrong error: {e}"),
        Ok(s) => panic!("unparseable kind silently became {:?}", s.kind),
    }

    let _ = src;
    db.pool.close().await;
}

/// Withdrawing a story must actually withdraw it.
///
/// Holding or killing removed a story from the front page and the feed but left
/// it fully readable at its own URL, so a story pulled for being wrong stayed up
/// for anyone holding the link. Every public surface now resolves slugs through
/// `published_by_slug`; this pins that behaviour.
#[tokio::test]
async fn held_stories_are_not_reachable_from_public_surfaces() {
    let Some(db) = setup().await else { return };

    let story = stories::create(
        &db,
        "withdrawal-test",
        StoryKind::Wire,
        "T",
        Category::Markets,
    )
    .await
    .unwrap();

    stories::set_status(&db, story.id, StoryStatus::Published, None)
        .await
        .unwrap();
    assert!(
        stories::published_by_slug(&db, &story.slug).await.is_ok(),
        "a published story must be publicly reachable"
    );

    for withdrawn in [StoryStatus::Held, StoryStatus::Killed] {
        stories::set_status(&db, story.id, withdrawn, Some("withdrawn"))
            .await
            .unwrap();
        assert!(
            matches!(
                stories::published_by_slug(&db, &story.slug).await,
                Err(bg_db::DbError::NotFound(_))
            ),
            "a {withdrawn:?} story must not be reachable from a public surface"
        );
        assert!(
            stories::by_slug(&db, &story.slug).await.is_ok(),
            "but internal callers must still see it"
        );
    }

    db.pool.close().await;
}
