//! Stories — the event layer, and the queries the front page runs on.

use crate::{convert::*, Db, DbError, Result};
use bg_core::domain::{Category, Story, StoryKind, StoryStatus, WireEntry};
use bg_core::ids::StoryId;
use sqlx::postgres::PgRow;
use sqlx::Row;
use uuid::Uuid;

const COLS: &str = "id, slug, kind, status, title, summary, category, newsworthiness, velocity, \
                    source_count, primary_asset, assets, beat, image_url, video_id, first_seen_at, published_at, \
                    updated_at, editor_note";

fn from_row(r: &PgRow) -> Result<Story> {
    Ok(Story {
        id: story_id(r, "id")?,
        slug: r.try_get("slug")?,
        kind: enum_col::<StoryKind>(r, "kind")?,
        status: enum_col::<StoryStatus>(r, "status")?,
        title: r.try_get("title")?,
        summary: r.try_get("summary")?,
        category: enum_col::<Category>(r, "category")?,
        newsworthiness: r.try_get("newsworthiness")?,
        velocity: r.try_get("velocity")?,
        source_count: r.try_get("source_count")?,
        primary_asset: r.try_get("primary_asset")?,
        assets: r.try_get("assets")?,
        beat: enum_col::<bg_core::domain::Beat>(r, "beat")?,
        image_url: r.try_get("image_url")?,
        video_id: r.try_get("video_id")?,
        first_seen_at: r.try_get("first_seen_at")?,
        published_at: r.try_get("published_at")?,
        updated_at: r.try_get("updated_at")?,
        editor_note: r.try_get("editor_note")?,
    })
}

/// Create a story, resolving slug collisions by suffixing.
///
/// Two unrelated events can produce the same slug ("solana-outage" happens more
/// than once), so the retry loop is a normal path rather than an error case.
pub async fn create(
    db: &Db,
    base_slug: &str,
    kind: StoryKind,
    title: &str,
    category: Category,
    beat: bg_core::domain::Beat,
) -> Result<Story> {
    for attempt in 0..25u32 {
        let slug = if attempt == 0 {
            base_slug.to_string()
        } else {
            bg_core::slug::slug_with_suffix(base_slug, attempt + 1)
        };
        let res = crate::sql(format!(
            "INSERT INTO stories (id, slug, kind, status, title, category, beat)
             VALUES ($1,$2,$3,'triage',$4,$5,$6)
             ON CONFLICT (slug) DO NOTHING
             RETURNING {COLS}"
        ))
        .bind(Uuid::new_v4())
        .bind(&slug)
        .bind(kind.as_str())
        .bind(title)
        .bind(category.as_str())
        .bind(beat.as_str())
        .fetch_optional(&db.pool)
        .await?;
        if let Some(row) = res {
            return from_row(&row);
        }
    }
    Err(DbError::NotFound("free story slug"))
}

pub async fn by_id(db: &Db, id: StoryId) -> Result<Story> {
    let row = crate::sql(format!("SELECT {COLS} FROM stories WHERE id = $1"))
        .bind(id.into_uuid())
        .fetch_optional(&db.pool)
        .await?
        .ok_or(DbError::NotFound("story"))?;
    from_row(&row)
}

/// Any story, whatever its status. **Internal use only** — agents and ops.
///
/// Every public surface must use [`published_by_slug`] instead. See its note.
pub async fn by_slug(db: &Db, slug: &str) -> Result<Story> {
    let row = crate::sql(format!("SELECT {COLS} FROM stories WHERE slug = $1"))
        .bind(slug)
        .fetch_optional(&db.pool)
        .await?
        .ok_or(DbError::NotFound("story"))?;
    from_row(&row)
}

/// A story a reader is allowed to see.
///
/// Holding or killing a story used to remove it from the front page and the
/// feed while leaving it fully readable at its own URL — so a story withdrawn
/// for being wrong stayed up for anyone with the link, and for any crawler that
/// had already indexed it. Withdrawal has to mean withdrawn.
///
/// This exists as a separate function, rather than a flag on [`by_slug`], for
/// the same reason `items::recent` is split from `items::body_for_analysis`:
/// "can the public reach unpublished content?" should be answerable by grepping
/// for one name.
pub async fn published_by_slug(db: &Db, slug: &str) -> Result<Story> {
    let row = crate::sql(format!(
        "SELECT {COLS} FROM stories WHERE slug = $1 AND status = 'published'"
    ))
    .bind(slug)
    .fetch_optional(&db.pool)
    .await?
    .ok_or(DbError::NotFound("story"))?;
    from_row(&row)
}

pub async fn set_status(
    db: &Db,
    id: StoryId,
    status: StoryStatus,
    editor_note: Option<&str>,
) -> Result<()> {
    // `published_at` is set here and only here, because the schema's
    // stories_published_has_ts CHECK makes the two inseparable.
    sqlx::query(
        "UPDATE stories
         SET status = $2,
             editor_note = COALESCE($3, editor_note),
             published_at = CASE WHEN $2 = 'published' THEN COALESCE(published_at, now())
                                 ELSE NULL END,
             updated_at = now()
         WHERE id = $1",
    )
    .bind(id.into_uuid())
    .bind(status.as_str())
    .bind(editor_note)
    .execute(&db.pool)
    .await?;
    Ok(())
}

pub async fn set_scores(db: &Db, id: StoryId, newsworthiness: i16, velocity: f32) -> Result<()> {
    sqlx::query(
        "UPDATE stories SET newsworthiness = $2, velocity = $3, updated_at = now() WHERE id = $1",
    )
    .bind(id.into_uuid())
    .bind(newsworthiness.clamp(0, 100))
    .bind(velocity)
    .execute(&db.pool)
    .await?;
    Ok(())
}

pub async fn set_summary(db: &Db, id: StoryId, summary: &str) -> Result<()> {
    sqlx::query("UPDATE stories SET summary = $2, updated_at = now() WHERE id = $1")
        .bind(id.into_uuid())
        .bind(summary)
        .execute(&db.pool)
        .await?;
    Ok(())
}

pub async fn set_kind(db: &Db, id: StoryId, kind: StoryKind) -> Result<()> {
    sqlx::query("UPDATE stories SET kind = $2, updated_at = now() WHERE id = $1")
        .bind(id.into_uuid())
        .bind(kind.as_str())
        .execute(&db.pool)
        .await?;
    Ok(())
}

pub async fn set_meta(
    db: &Db,
    id: StoryId,
    title: Option<&str>,
    primary_asset: Option<&str>,
    assets: &[String],
    image_url: Option<&str>,
    video_id: Option<&str>,
) -> Result<()> {
    sqlx::query(
        "UPDATE stories SET
            title = COALESCE($2, title),
            primary_asset = COALESCE($3, primary_asset),
            assets = CASE WHEN cardinality($4::text[]) > 0 THEN $4 ELSE assets END,
            image_url = COALESCE($5, image_url),
            video_id = COALESCE($6, video_id),
            updated_at = now()
         WHERE id = $1",
    )
    .bind(id.into_uuid())
    .bind(title)
    .bind(primary_asset)
    .bind(assets)
    .bind(image_url)
    .bind(video_id)
    .execute(&db.pool)
    .await?;
    Ok(())
}

/// Stories still moving through the pipeline.
pub async fn open(db: &Db, limit: i64) -> Result<Vec<Story>> {
    let rows = crate::sql(format!(
        "SELECT {COLS} FROM stories
         WHERE status IN ('triage','clustering','drafting','review')
         ORDER BY newsworthiness DESC, updated_at DESC LIMIT $1"
    ))
    .bind(limit)
    .fetch_all(&db.pool)
    .await?;
    rows.iter().map(from_row).collect()
}

/// Published Wire stories that never got a usable summary, newest first.
///
/// The offline stub could only restate the headline, and a dek that restates
/// the headline is dropped at publish time — which leaves the story page with
/// nothing on it but a source list. These are the ones worth re-running once a
/// real model is reachable.
pub async fn needing_summary(db: &Db, limit: i64) -> Result<Vec<Story>> {
    let rows = crate::sql(format!(
        "SELECT {COLS} FROM stories
         WHERE status = 'published' AND kind = 'wire'
           AND coalesce(length(summary), 0) = 0
         ORDER BY published_at DESC LIMIT $1"
    ))
    .bind(limit)
    .fetch_all(&db.pool)
    .await?;
    rows.iter().map(from_row).collect()
}

/// The published stories a reader is most likely to see, highest ranked first.
///
/// Used by `bg rescore` to re-judge the archive with a better model: the front
/// page is where a bad score does visible damage, so that is where re-scoring
/// should start rather than at the oldest or the newest.
pub async fn top_published(db: &Db, limit: i64) -> Result<Vec<Story>> {
    let rows = crate::sql(format!(
        "SELECT {COLS} FROM stories
         WHERE status = 'published'
         ORDER BY newsworthiness DESC, published_at DESC LIMIT $1"
    ))
    .bind(limit)
    .fetch_all(&db.pool)
    .await?;
    rows.iter().map(from_row).collect()
}

/// Published stories of a given kind, newest first.
pub async fn published(
    db: &Db,
    kind: Option<StoryKind>,
    limit: i64,
    offset: i64,
) -> Result<Vec<Story>> {
    let rows = crate::sql(format!(
        "SELECT {COLS} FROM stories
         WHERE status = 'published' AND ($1::text IS NULL OR kind = $1)
         ORDER BY published_at DESC LIMIT $2 OFFSET $3"
    ))
    .bind(kind.map(|k| k.as_str()))
    .bind(limit)
    .bind(offset)
    .fetch_all(&db.pool)
    .await?;
    rows.iter().map(from_row).collect()
}

pub async fn by_category(db: &Db, cat: Category, limit: i64) -> Result<Vec<Story>> {
    let rows = crate::sql(format!(
        "SELECT {COLS} FROM stories
         WHERE status = 'published' AND category = $1
         ORDER BY published_at DESC LIMIT $2"
    ))
    .bind(cat.as_str())
    .bind(limit)
    .fetch_all(&db.pool)
    .await?;
    rows.iter().map(from_row).collect()
}

pub async fn by_asset(db: &Db, ticker: &str, limit: i64) -> Result<Vec<Story>> {
    let rows = crate::sql(format!(
        "SELECT {COLS} FROM stories
         WHERE status = 'published' AND (primary_asset = $1 OR $1 = ANY(assets))
         ORDER BY published_at DESC LIMIT $2"
    ))
    .bind(ticker.to_uppercase())
    .bind(limit)
    .fetch_all(&db.pool)
    .await?;
    rows.iter().map(from_row).collect()
}

/// Front-page ranking.
///
/// Score = newsworthiness decayed by age, with a bonus for corroboration. The
/// half-life is deliberately short: in this market a six-hour-old lead story is
/// already stale, and a front page that does not move looks abandoned.
/// The ranked front page, optionally for one desk.
///
/// `None` blends both, which is what `/` shows: a reader who has not chosen a
/// desk should see whatever is most significant right now regardless of which
/// one it came from.
pub async fn front_page(
    db: &Db,
    beat: Option<bg_core::domain::Beat>,
    limit: i64,
) -> Result<Vec<Story>> {
    let rows = crate::sql(format!(
        "SELECT {COLS} FROM stories
         WHERE status = 'published' AND ($2::text IS NULL OR beat = $2)
         ORDER BY (
            newsworthiness
            * exp(-extract(epoch from (now() - published_at)) / 21600.0)
            + least(source_count, 6) * 3
         ) DESC, published_at DESC
         LIMIT $1"
    ))
    .bind(limit)
    .bind(beat.map(|b| b.as_str()))
    .fetch_all(&db.pool)
    .await?;
    rows.iter().map(from_row).collect()
}

/// The Wire: every published story with its lead source, for the fast feed.
pub async fn wire(
    db: &Db,
    beat: Option<bg_core::domain::Beat>,
    limit: i64,
    offset: i64,
) -> Result<Vec<WireEntry>> {
    let rows = sqlx::query(
        "SELECT st.id, st.slug, st.title, st.summary, st.category, st.source_count,
                st.published_at, st.newsworthiness, st.image_url, st.assets, st.beat,
                src.name AS source_name, src.slug AS source_slug, src.kind AS source_kind,
                ri.canonical_url AS source_url
         FROM stories st
         JOIN LATERAL (
            SELECT r.* FROM story_items si
            JOIN raw_items r ON r.id = si.raw_item_id
            WHERE si.story_id = st.id
            ORDER BY (si.role = 'seed') DESC, r.published_at ASC
            LIMIT 1
         ) ri ON TRUE
         JOIN sources src ON src.id = ri.source_id
         WHERE st.status = 'published' AND ($3::text IS NULL OR st.beat = $3)
         ORDER BY st.published_at DESC
         LIMIT $1 OFFSET $2",
    )
    .bind(limit)
    .bind(offset)
    .bind(beat.map(|b| b.as_str()))
    .fetch_all(&db.pool)
    .await?;

    rows.iter()
        .map(|r| {
            Ok(WireEntry {
                story_id: story_id(r, "id")?,
                slug: r.try_get("slug")?,
                title: r.try_get("title")?,
                summary: r
                    .try_get::<Option<String>, _>("summary")?
                    .unwrap_or_default(),
                category: enum_col::<Category>(r, "category")?,
                source_name: r.try_get("source_name")?,
                source_slug: r.try_get("source_slug")?,
                source_url: r.try_get("source_url")?,
                source_kind: enum_col::<bg_core::domain::SourceKind>(r, "source_kind")?,
                beat: enum_col::<bg_core::domain::Beat>(r, "beat")?,
                source_count: r.try_get("source_count")?,
                published_at: r.try_get("published_at")?,
                newsworthiness: r.try_get("newsworthiness")?,
                image_url: r.try_get("image_url")?,
                assets: r.try_get("assets")?,
            })
        })
        .collect()
}

/// Sources backing a story, for the byline strip and the policy link-out check.
pub async fn source_refs(db: &Db, id: StoryId) -> Result<Vec<bg_core::domain::SourceRef>> {
    let rows = sqlx::query(
        "SELECT s.name, s.slug, s.trust, r.canonical_url AS url, r.title, r.published_at, si.role
         FROM story_items si
         JOIN raw_items r ON r.id = si.raw_item_id
         JOIN sources s   ON s.id = r.source_id
         WHERE si.story_id = $1
         ORDER BY (si.role = 'seed') DESC, s.trust DESC, r.published_at ASC",
    )
    .bind(id.into_uuid())
    .fetch_all(&db.pool)
    .await?;
    rows.iter()
        .map(|r| {
            Ok(bg_core::domain::SourceRef {
                name: r.try_get("name")?,
                slug: r.try_get("slug")?,
                url: r.try_get("url")?,
                title: r.try_get("title")?,
                trust: r.try_get("trust")?,
                role: enum_col::<bg_core::domain::ItemRole>(r, "role")?,
                published_at: r.try_get("published_at")?,
            })
        })
        .collect()
}

/// Narrative trend data for `/flyway`: coverage volume per category per day.
pub async fn flyway(db: &Db, days: i32) -> Result<Vec<(String, chrono::NaiveDate, i64)>> {
    let rows = sqlx::query(
        "SELECT category, published_at::date AS day, count(*) AS n
         FROM stories
         WHERE status = 'published' AND published_at > now() - make_interval(days => $1)
         GROUP BY category, day
         ORDER BY day ASC, n DESC",
    )
    .bind(days)
    .fetch_all(&db.pool)
    .await?;
    Ok(rows
        .iter()
        .map(|r| (r.get("category"), r.get("day"), r.get("n")))
        .collect())
}

/// Withdraw stories that exist only because we ignored a robots.txt.
///
/// A story with at least one permitted source stands: the event is real and
/// somebody we were allowed to read reported it. A story whose every source is
/// one that told us not to crawl has no such footing, and publishing it while
/// claiming to honour robots.txt is the contradiction worth removing.
///
/// Killed, not deleted — the status is a flag, the record of what we published
/// and then withdrew stays intact, and it is reversible if a publisher's terms
/// change.
pub async fn retract_disallowed(db: &Db) -> Result<u64> {
    let r = sqlx::query(
        "UPDATE stories st
            SET status = 'killed',
                editor_note = 'retracted: every source disallows crawling',
                -- The stories_published_has_ts CHECK ties status to the
                -- timestamp; leaving published_at set on a killed story
                -- violates it and the whole statement rolls back.
                published_at = NULL,
                updated_at = now()
          WHERE st.status = 'published'
            AND EXISTS (SELECT 1 FROM raw_items r JOIN sources s ON s.id = r.source_id
                         WHERE r.story_id = st.id AND NOT s.robots_ok)
            AND NOT EXISTS (SELECT 1 FROM raw_items r JOIN sources s ON s.id = r.source_id
                             WHERE r.story_id = st.id AND s.robots_ok)",
    )
    .execute(&db.pool)
    .await?;
    Ok(r.rows_affected())
}

/// Withdraw stories that merge too many unrelated events to be one story.
///
/// Artifacts of a single early run whose clustering was adjudicated by the
/// deterministic stub, which answers "same event?" with a constant yes. The
/// worst welded twenty separate events — an Argentine court ruling, a Grayscale
/// filing, the ECB digital euro — under one headline.
///
/// On a site whose whole claim is that every assertion shows its sources, a
/// page that is not about one thing cannot show them honestly. Killed rather
/// than re-clustered: the items are months stale now, and re-running them would
/// spend a scarce token budget to recover news nobody needs.
pub async fn retract_incoherent(db: &Db, max_items: i64) -> Result<u64> {
    let r = sqlx::query(
        "UPDATE stories st
            SET status = 'killed',
                editor_note = 'retracted: merged unrelated events (stub-era clustering)',
                published_at = NULL,
                updated_at = now()
          WHERE st.status = 'published'
            AND (SELECT count(*) FROM raw_items r WHERE r.story_id = st.id) > $1",
    )
    .bind(max_items)
    .execute(&db.pool)
    .await?;
    Ok(r.rows_affected())
}

/// Single-source published stories in a recent window, with their seed title.
///
/// The population `bg recluster` works over. Restricted to one source because
/// a story that already has corroboration has been through the merge path and
/// succeeded; the ones worth revisiting are the ones that never found a match.
pub async fn singletons(db: &Db, hours: i64, limit: i64) -> Result<Vec<(StoryId, String, i64)>> {
    let rows = sqlx::query(
        // `first_seen_at`, not the publication time: the story that saw the
        // event first is the one whose URL should survive a fold, whatever
        // order the desk got round to publishing them in.
        "SELECT s.id, s.title, extract(epoch FROM s.first_seen_at)::bigint AS ts
           FROM stories s
          WHERE s.status = 'published'
            AND s.source_count <= 1
            AND s.first_seen_at > now() - make_interval(hours => $1::int)
          ORDER BY s.first_seen_at DESC
          LIMIT $2",
    )
    .bind(hours as i32)
    .bind(limit)
    .fetch_all(&db.pool)
    .await?;
    rows.iter()
        .map(|r| {
            Ok((
                StoryId::from(r.try_get::<uuid::Uuid, _>("id")?),
                r.try_get::<String, _>("title")?,
                r.try_get::<i64, _>("ts")?,
            ))
        })
        .collect()
}

/// Fold `from` into `into`: move its items across, then retire the husk.
///
/// `killed`, the same state a retraction leaves behind — the status set has no
/// "withdrawn", and `stories_published_has_ts` requires `published_at` to be
/// null for anything not published. Killed rather than deleted: a published URL
/// that starts returning 404 is a broken link in somebody's timeline, and the
/// editor note says where its reporting went.
pub async fn merge_into(db: &Db, from: StoryId, into: StoryId) -> Result<u64> {
    let mut tx = db.pool.begin().await?;
    // An item already attached to the target would violate the join's primary
    // key, so those are dropped rather than moved — the corroboration is
    // already recorded.
    sqlx::query(
        "DELETE FROM story_items a
          WHERE a.story_id = $1
            AND EXISTS (SELECT 1 FROM story_items b
                         WHERE b.story_id = $2 AND b.raw_item_id = a.raw_item_id)",
    )
    .bind(from.into_uuid())
    .bind(into.into_uuid())
    .execute(&mut *tx)
    .await?;

    let moved = sqlx::query(
        "UPDATE story_items SET story_id = $2, role = 'corroborating' WHERE story_id = $1",
    )
    .bind(from.into_uuid())
    .bind(into.into_uuid())
    .execute(&mut *tx)
    .await?
    .rows_affected();

    sqlx::query("UPDATE raw_items SET story_id = $2 WHERE story_id = $1")
        .bind(from.into_uuid())
        .bind(into.into_uuid())
        .execute(&mut *tx)
        .await?;

    sqlx::query(
        "UPDATE stories
            SET status = 'killed',
                editor_note = 'folded into another story: same event, reported separately',
                published_at = NULL,
                updated_at = now()
          WHERE id = $1",
    )
    .bind(from.into_uuid())
    .execute(&mut *tx)
    .await?;

    // `source_count` is denormalised, and a fold that moves the evidence but
    // leaves the count behind is worse than not folding: the page would then
    // list three outlets and claim one. Recomputed from the items themselves,
    // in the same transaction, so the two can never disagree.
    //
    // Newsworthiness and velocity are deliberately not touched — those are the
    // Curator's arithmetic and the next pass will redo them.
    sqlx::query(
        "UPDATE stories SET source_count = (
             SELECT count(DISTINCT r.source_id) FROM raw_items r WHERE r.story_id = $1
         ), updated_at = now() WHERE id = $1",
    )
    .bind(into.into_uuid())
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;
    Ok(moved)
}

/// Published stories, and how many of them nobody else corroborated.
///
/// The number that matters most on a site whose proposition is *how many
/// independent outlets confirm this*. It sat at 1,407 of 1,438 for weeks
/// without anything saying so, because nothing was looking.
pub async fn corroboration_health(db: &Db, days: i64) -> Result<(i64, i64)> {
    let row = sqlx::query(
        "SELECT count(*) FILTER (WHERE source_count <= 1)::bigint AS alone,
                count(*)::bigint AS total
           FROM stories
          WHERE status = 'published'
            AND first_seen_at > now() - make_interval(days => $1::int)",
    )
    .bind(days as i32)
    .fetch_one(&db.pool)
    .await?;
    Ok((row.try_get("alone")?, row.try_get("total")?))
}

/// Bring every published story's `source_count` back in line with its items.
///
/// The column is denormalised and therefore able to drift, and it is not a
/// cosmetic number here — it is the corroboration claim the whole site rests
/// on. Cheap, idempotent, and run before every recluster so a fold that
/// predates the count being maintained cannot leave a story understating its
/// own evidence.
pub async fn reconcile_source_counts(db: &Db) -> Result<u64> {
    let r = sqlx::query(
        "UPDATE stories s
            SET source_count = c.n, updated_at = now()
           FROM (SELECT st.id, count(DISTINCT r.source_id)::int AS n
                   FROM stories st JOIN raw_items r ON r.story_id = st.id
                  GROUP BY st.id) c
          WHERE s.id = c.id AND s.source_count IS DISTINCT FROM c.n",
    )
    .execute(&db.pool)
    .await?;
    Ok(r.rows_affected())
}
