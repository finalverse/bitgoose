//! Raw source items.
//!
//! Note the split between [`recent`] (public projection, safe to serialize) and
//! [`body_for_analysis`] / [`bodies_for_story`] (private working text). Keeping
//! them as separate functions with separate return types means "did we just
//! leak source text?" is answerable by grepping for two names.

use crate::{convert::*, Db, Result};
use bg_core::domain::{ItemRole, RawItem, RawItemPublic};
use bg_core::ids::{RawItemId, SourceId, StoryId};
use chrono::{DateTime, Utc};
use sqlx::postgres::PgRow;
use sqlx::Row;
use uuid::Uuid;

/// Columns for the full record. `body_raw` is included only because
/// [`from_row`] is used by the analysis paths; the public paths use [`PUB_COLS`].
const COLS: &str = "id, source_id, external_id, canonical_url, url_hash, title, dek, authors, \
                    published_at, fetched_at, summary_raw, body_raw, body_hash, simhash, lang, \
                    image_url, video_id, beat, story_id, triaged";

const PUB_COLS: &str =
    "id, source_id, canonical_url, title, authors, published_at, image_url, video_id";

fn from_row(r: &PgRow) -> Result<RawItem> {
    Ok(RawItem {
        id: raw_item_id(r, "id")?,
        source_id: source_id(r, "source_id")?,
        external_id: r.try_get("external_id")?,
        canonical_url: r.try_get("canonical_url")?,
        url_hash: r.try_get("url_hash")?,
        title: r.try_get("title")?,
        dek: r.try_get("dek")?,
        authors: r.try_get("authors")?,
        published_at: r.try_get("published_at")?,
        fetched_at: r.try_get("fetched_at")?,
        summary_raw: r.try_get("summary_raw")?,
        body_raw: r.try_get("body_raw")?,
        body_hash: r.try_get("body_hash")?,
        simhash: r.try_get("simhash")?,
        lang: r.try_get("lang")?,
        image_url: r.try_get("image_url")?,
        video_id: r.try_get("video_id")?,
        beat: enum_col_opt::<bg_core::domain::Beat>(r, "beat")?,
        story_id: story_id_opt(r, "story_id")?,
        triaged: r.try_get("triaged")?,
    })
}

fn pub_from_row(r: &PgRow) -> Result<RawItemPublic> {
    Ok(RawItemPublic {
        id: raw_item_id(r, "id")?,
        source_id: source_id(r, "source_id")?,
        canonical_url: r.try_get("canonical_url")?,
        title: r.try_get("title")?,
        authors: r.try_get("authors")?,
        published_at: r.try_get("published_at")?,
        image_url: r.try_get("image_url")?,
        video_id: r.try_get("video_id")?,
    })
}

/// What the ingest layer produces before an ID is assigned.
#[derive(Debug, Clone)]
pub struct NewItem {
    pub source_id: SourceId,
    pub external_id: Option<String>,
    pub canonical_url: String,
    pub url_hash: String,
    pub title: String,
    pub dek: Option<String>,
    pub authors: Vec<String>,
    pub published_at: DateTime<Utc>,
    pub summary_raw: Option<String>,
    pub body_raw: Option<String>,
    pub body_hash: Option<String>,
    pub simhash: u64,
    pub lang: String,
    pub image_url: Option<String>,
    pub video_id: Option<String>,
    pub beat: Option<bg_core::domain::Beat>,
}

/// Insert unless `url_hash` already exists.
///
/// Returns `None` on conflict, which is the common case — most of a feed on any
/// given poll is items we already have. Callers use the `None` count as the
/// "nothing new" signal rather than diffing.
pub async fn insert_new(db: &Db, it: &NewItem) -> Result<Option<RawItemId>> {
    let id = Uuid::new_v4();
    let row = sqlx::query(
        "INSERT INTO raw_items
           (id, source_id, external_id, canonical_url, url_hash, title, dek, authors,
            published_at, summary_raw, body_raw, body_hash, simhash, lang, image_url, video_id, beat)
         VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17)
         ON CONFLICT (url_hash) DO NOTHING
         RETURNING id",
    )
    .bind(id)
    .bind(it.source_id.into_uuid())
    .bind(&it.external_id)
    .bind(&it.canonical_url)
    .bind(&it.url_hash)
    .bind(&it.title)
    .bind(&it.dek)
    .bind(&it.authors)
    .bind(it.published_at)
    .bind(&it.summary_raw)
    .bind(&it.body_raw)
    .bind(&it.body_hash)
    .bind(simhash_to_db(it.simhash))
    .bind(&it.lang)
    .bind(&it.image_url)
    .bind(&it.video_id)
    .bind(it.beat.map(|b| b.as_str()))
    .fetch_optional(&db.pool)
    .await?;
    Ok(row.map(|r| RawItemId::from_uuid(r.get::<Uuid, _>("id"))))
}

/// Mark a story's items untriaged so they are judged again.
///
/// The scores behind story ranking were produced by whatever model was
/// configured at the time. When that model changes, re-judging is the only way
/// to make the archive reflect it — nothing else recomputes those numbers.
pub async fn reset_triage_for_story(db: &Db, story: StoryId) -> Result<u64> {
    let r = sqlx::query("UPDATE raw_items SET triaged = FALSE WHERE story_id = $1")
        .bind(story.into_uuid())
        .execute(&db.pool)
        .await?;
    Ok(r.rows_affected())
}

pub async fn count(db: &Db) -> Result<i64> {
    Ok(sqlx::query_scalar("SELECT count(*) FROM raw_items")
        .fetch_one(&db.pool)
        .await?)
}

/// Items Gosling has not yet read, newest first.
/// Items waiting to be triaged, taken fairly across the desks.
///
/// Newest-first sounds neutral and is not. Intake is wildly uneven — AI and
/// Crypto bring in some 2,200 items a week between them against a few hundred
/// for the rest — and triage can only reach about a hundred a pass, so a global
/// ordering by recency hands every batch to whichever desk publishes most.
///
/// The result was not subtle: World, Science and Culture were fed, had items
/// waiting, and had **published nothing at all**, while their pages sat in the
/// navigation. The desks were not broken and the scheduler was not broken. The
/// queue was simply a single line that the loudest desks stood at the front of.
///
/// So each desk is ranked internally by recency and the ranks are interleaved:
/// the newest item from every desk, then the second newest from every desk, and
/// so on. A desk with three items still gets those three looked at today; a
/// desk with nine hundred no longer crowds it out.
///
/// **That was not enough, because the same thing happens inside a desk.** The
/// AI queue held 3,163 arXiv preprints against 136 items from Decrypt, so
/// arXiv won every AI slot on recency and the front page filled with
/// "Logit-Guided Neural Routing for Billion-Scale Vector Search" while Bitcoin
/// rose 7.9% in a day and the newsroom said nothing about it. One firehose
/// silences a desk exactly as effectively as one desk silences another.
///
/// So the interleave is now by **source** as well: the newest item from every
/// source, then the second newest from every source. Ties break toward the desk
/// that has had the fewest items so far, which keeps the original between-desk
/// fairness intact. Within one source, recency still decides.
///
/// Note this is fairness in *attention*, not in publication — every item still
/// has to earn its way past triage. It only guarantees that a quiet source gets
/// looked at, which is the difference between a wire service and a scraper.
pub async fn untriaged(db: &Db, limit: i64) -> Result<Vec<RawItem>> {
    let cols = COLS
        .split(',')
        .map(|c| format!("t.{}", c.trim()))
        .collect::<Vec<_>>()
        .join(", ");
    let rows = crate::sql(format!(
        "SELECT {cols} FROM (
           SELECT r.*,
                  row_number() OVER (
                    PARTITION BY r.source_id
                    ORDER BY r.published_at DESC
                  ) AS src_rank,
                  row_number() OVER (
                    PARTITION BY coalesce(s.beat, 'unrouted')
                    ORDER BY r.published_at DESC
                  ) AS desk_rank
             FROM raw_items r
             JOIN sources s ON s.id = r.source_id
            WHERE NOT r.triaged AND r.aged_out_at IS NULL
         ) t
         ORDER BY t.src_rank ASC, t.desk_rank ASC, t.published_at DESC
         LIMIT $1"
    ))
    .bind(limit)
    .fetch_all(&db.pool)
    .await?;
    rows.iter().map(from_row).collect()
}

pub async fn mark_triaged(
    db: &Db,
    id: RawItemId,
    category: Option<&str>,
    assets: &[String],
    score: i16,
) -> Result<()> {
    sqlx::query(
        "UPDATE raw_items SET triaged = TRUE, category = $2, assets = $3, triage_score = $4
         WHERE id = $1",
    )
    .bind(id.into_uuid())
    .bind(category)
    .bind(assets)
    .bind(score)
    .execute(&db.pool)
    .await?;
    Ok(())
}

/// Triaged items not yet attached to a story — the clustering input.
pub async fn unclustered(db: &Db, limit: i64) -> Result<Vec<RawItem>> {
    let rows = crate::sql(format!(
        "SELECT {COLS} FROM raw_items
         WHERE triaged AND story_id IS NULL AND aged_out_at IS NULL
         ORDER BY published_at DESC LIMIT $1"
    ))
    .bind(limit)
    .fetch_all(&db.pool)
    .await?;
    rows.iter().map(from_row).collect()
}

/// Recent items that already belong to a story — the clustering *candidates*.
///
/// Restricted to a time window because near-duplicate matching across the whole
/// archive would both cost more and be wrong: the same headline six months
/// apart is two events, not one.
pub async fn clustering_candidates(db: &Db, hours: i64, limit: i64) -> Result<Vec<RawItem>> {
    let rows = crate::sql(format!(
        "SELECT {COLS} FROM raw_items
         WHERE story_id IS NOT NULL
           AND published_at > now() - make_interval(hours => $1)
         ORDER BY published_at DESC LIMIT $2"
    ))
    .bind(hours as i32)
    .bind(limit)
    .fetch_all(&db.pool)
    .await?;
    rows.iter().map(from_row).collect()
}

/// Attach an item to a story and record how it relates to it.
pub async fn attach_to_story(
    db: &Db,
    item: RawItemId,
    story: StoryId,
    role: ItemRole,
) -> Result<()> {
    let mut tx = db.pool.begin().await?;
    sqlx::query("UPDATE raw_items SET story_id = $2 WHERE id = $1")
        .bind(item.into_uuid())
        .bind(story.into_uuid())
        .execute(&mut *tx)
        .await?;
    sqlx::query(
        "INSERT INTO story_items (story_id, raw_item_id, role) VALUES ($1,$2,$3)
         ON CONFLICT (story_id, raw_item_id) DO UPDATE SET role = EXCLUDED.role",
    )
    .bind(story.into_uuid())
    .bind(item.into_uuid())
    .bind(role.as_str())
    .execute(&mut *tx)
    .await?;
    // Denormalized so front-page queries never need the join.
    sqlx::query(
        "UPDATE stories SET
            source_count = (SELECT count(DISTINCT r.source_id)
                            FROM story_items si JOIN raw_items r ON r.id = si.raw_item_id
                            WHERE si.story_id = $1),
            updated_at = now()
         WHERE id = $1",
    )
    .bind(story.into_uuid())
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(())
}

pub async fn by_story(db: &Db, story: StoryId) -> Result<Vec<RawItem>> {
    let rows = crate::sql(format!(
        "SELECT {COLS} FROM raw_items WHERE story_id = $1 ORDER BY published_at ASC"
    ))
    .bind(story.into_uuid())
    .fetch_all(&db.pool)
    .await?;
    rows.iter().map(from_row).collect()
}

pub async fn recent_public(db: &Db, limit: i64) -> Result<Vec<RawItemPublic>> {
    let rows = crate::sql(format!(
        "SELECT {PUB_COLS} FROM raw_items ORDER BY published_at DESC LIMIT $1"
    ))
    .bind(limit)
    .fetch_all(&db.pool)
    .await?;
    rows.iter().map(pub_from_row).collect()
}

// -- full-text extraction ---------------------------------------------------

/// Let go of items that stopped being news before we reached them.
///
/// The newsroom ingests about three times what a free inference tier can
/// triage, so a queue builds — and almost all of it is stale. Measured on the
/// live archive: of 3,764 waiting items, 137 were from the last day and 3,627
/// were older. Working through that in order would spend today's token budget
/// on last week's news and then publish it as new.
///
/// So past the horizon an item lapses. It stays in the database, keeps its
/// place in URL de-duplication, and simply stops competing for attention it can
/// no longer justify. Returns how many lapsed, for the log.
pub async fn expire_stale_untriaged(db: &Db, horizon_hours: i64) -> Result<u64> {
    let r = sqlx::query(
        "UPDATE raw_items SET aged_out_at = now()
          WHERE NOT triaged AND aged_out_at IS NULL
            AND published_at < now() - make_interval(hours => $1)",
    )
    .bind(horizon_hours as i32)
    .execute(&db.pool)
    .await?;
    Ok(r.rows_affected())
}

/// How many items are waiting, and how many have lapsed. For `bg doctor`.
pub async fn queue_health(db: &Db) -> Result<(i64, i64)> {
    let row = sqlx::query(
        "SELECT count(*) FILTER (WHERE NOT triaged AND aged_out_at IS NULL) AS waiting,
                count(*) FILTER (WHERE aged_out_at IS NOT NULL) AS lapsed
           FROM raw_items",
    )
    .fetch_one(&db.pool)
    .await?;
    Ok((row.try_get("waiting")?, row.try_get("lapsed")?))
}

/// How many times we will try a URL that errors before giving up on it.
///
/// Three, so a blip gets another chance and a publisher who blocks us stops
/// consuming the queue. Exhausted items keep `extracted_at` NULL: they are not
/// "done", they are "not reachable from here", and that difference matters if
/// the block is ever lifted.
pub const MAX_EXTRACT_ATTEMPTS: i16 = 3;

/// Items whose article page we have not tried to fetch yet.
///
/// Restricted to items attached to a story: an unclustered item may still be
/// dropped, and fetching a publisher's page for something we will never print
/// spends their bandwidth for nothing.
pub async fn needing_extraction(db: &Db, limit: i64) -> Result<Vec<(RawItemId, String)>> {
    let rows = sqlx::query(
        // Excluding disallowed sources here rather than relying on the
        // per-URL robots check means we never even open a connection to a
        // publisher who has told us no — the check downstream is the backstop,
        // not the gate.
        //
        // `extract_attempts` bounds the retries. Without it the newest-first
        // ordering parks a wall of permanently-failing URLs at the head of the
        // queue and nothing behind them is ever reached.
        //
        // Ordered by the parent story's front-page rank, not by recency.
        // Extraction exists to feed analysis, analysis follows attention, and
        // only four pages are fetched per pass on a 15 KB/s link — so those
        // four should be the ones a reader is about to open. Newest-first spent
        // them on whatever landed last, which is usually the thinnest item of
        // the hour.
        "SELECT r.id, r.canonical_url FROM raw_items r
           JOIN sources s ON s.id = r.source_id
           JOIN stories st ON st.id = r.story_id
          WHERE r.extracted_at IS NULL AND r.story_id IS NOT NULL
            AND r.extract_attempts < $2
            AND s.robots_ok AND s.enabled
            -- The gate for the publisher's AI posture, in the same place as
            -- the gate for robots.txt and for the same reason: a site that
            -- blocks the AI crawlers by name has said what it objects to, and
            -- it is not the fetching. Extraction exists only to feed the
            -- Skein, so for these sources there is nothing to fetch *for*.
            -- They stay in the Wire, ranked and linked, as they should.
            AND s.ai_input_ok
          ORDER BY r.extract_attempts ASC,
                   (st.newsworthiness
                    * exp(-extract(epoch from (now() - st.published_at)) / 21600.0)
                    + least(st.source_count, 6) * 3) DESC,
                   r.published_at DESC
          LIMIT $1",
    )
    .bind(limit)
    .bind(MAX_EXTRACT_ATTEMPTS)
    .fetch_all(&db.pool)
    .await?;
    rows.iter()
        .map(|r| {
            Ok((
                RawItemId::from_uuid(r.try_get::<Uuid, _>("id")?),
                r.try_get("canonical_url")?,
            ))
        })
        .collect()
}

/// Record a failed fetch without marking the item done.
///
/// Distinct from [`record_extraction`] with `None`, which means "we looked and
/// there was no article" — a permanent answer. This means "we could not look",
/// which deserves another go, but not an unlimited number of them.
pub async fn record_extract_failure(db: &Db, id: RawItemId) -> Result<()> {
    sqlx::query("UPDATE raw_items SET extract_attempts = extract_attempts + 1 WHERE id = $1")
        .bind(id.into_uuid())
        .execute(&db.pool)
        .await?;
    Ok(())
}

/// Record an extraction attempt.
///
/// `body` of `None` marks the attempt without changing the text — a paywall or
/// a video page is a permanent answer, and retrying it every run would be a
/// slow-motion hammering of a site that already told us no.
/// Take the page's own share image, but only where the feed gave us none.
///
/// `COALESCE` rather than overwrite: a feed that troubled to include an image
/// chose that one for this story, and a page's `og:image` is sometimes a
/// section banner. The feed wins where it spoke; this fills the silence — which
/// on this corpus was 56% of published stories having no picture at all.
pub async fn record_page_image(db: &Db, id: RawItemId, image: &str) -> Result<()> {
    sqlx::query(
        "UPDATE raw_items SET image_url = COALESCE(NULLIF(image_url, \'\'), $2) WHERE id = $1",
    )
    .bind(id.into_uuid())
    .bind(image)
    .execute(&db.pool)
    .await?;
    Ok(())
}

pub async fn record_extraction(
    db: &Db,
    id: RawItemId,
    body: Option<&str>,
    via: &str,
) -> Result<()> {
    match body {
        Some(text) => {
            sqlx::query(
                "UPDATE raw_items
                    SET body_raw = $2, body_hash = encode(sha256($2::bytea), 'hex'),
                        extracted_at = now(), extract_via = $3
                  WHERE id = $1",
            )
            .bind(id.into_uuid())
            .bind(text)
            .bind(via)
            .execute(&db.pool)
            .await?;
        }
        None => {
            sqlx::query(
                "UPDATE raw_items SET extracted_at = now(), extract_via = $2 WHERE id = $1",
            )
            .bind(id.into_uuid())
            .bind(via)
            .execute(&db.pool)
            .await?;
        }
    }
    Ok(())
}

/// How extraction is going, by winning selector. Powers `bg doctor`.
pub async fn extraction_stats(db: &Db) -> Result<Vec<(String, i64)>> {
    let rows = sqlx::query(
        "SELECT coalesce(extract_via, 'not attempted') AS via, count(*) AS n
           FROM raw_items GROUP BY 1 ORDER BY 2 DESC",
    )
    .fetch_all(&db.pool)
    .await?;
    Ok(rows.iter().map(|r| (r.get("via"), r.get("n"))).collect())
}

// -- private working text ---------------------------------------------------
// The two functions below are the ONLY ones that hand out `body_raw`. They
// return bare strings rather than a serializable struct so the text cannot
// accidentally ride along inside an API response type.

/// Source text for claim extraction. Never rendered, never serialized.
pub async fn body_for_analysis(db: &Db, id: RawItemId) -> Result<Option<String>> {
    Ok(
        sqlx::query_scalar("SELECT body_raw FROM raw_items WHERE id = $1")
            .bind(id.into_uuid())
            .fetch_optional(&db.pool)
            .await?
            .flatten(),
    )
}

/// `(source_slug, body)` for every item on a story, for the policy engine's
/// verbatim-overlap check.
pub async fn bodies_for_story(db: &Db, story: StoryId) -> Result<Vec<(String, String)>> {
    let rows = sqlx::query(
        "SELECT s.slug AS slug, r.body_raw AS body
         FROM raw_items r JOIN sources s ON s.id = r.source_id
         WHERE r.story_id = $1 AND r.body_raw IS NOT NULL",
    )
    .bind(story.into_uuid())
    .fetch_all(&db.pool)
    .await?;
    Ok(rows
        .iter()
        .map(|r| (r.get("slug"), r.get("body")))
        .collect())
}

// -- retraction --------------------------------------------------------------

/// Discard the working text taken from sources that disallow us.
///
/// `refresh_robots` was written and never called, so `robots_ok` held its
/// seed-time default and seven sources were polled for weeks against a
/// `Disallow: /`. Stopping was the first half of the fix; this is the second.
///
/// The row survives with its URL and title. That is deliberate: the URL hash is
/// what stops a re-post being ingested twice, and throwing it away would mean
/// re-fetching the same disallowed page the moment anything changed. What goes
/// is the material we should never have held — the body and the summary.
pub async fn purge_disallowed_text(db: &Db) -> Result<u64> {
    let r = sqlx::query(
        "UPDATE raw_items r
            SET body_raw = NULL, summary_raw = NULL, body_hash = NULL
           FROM sources s
          WHERE s.id = r.source_id
            AND NOT s.robots_ok
            AND (r.body_raw IS NOT NULL OR r.summary_raw IS NOT NULL)",
    )
    .execute(&db.pool)
    .await?;
    Ok(r.rows_affected())
}

/// Erase stored body text belonging to publishers who decline model input.
///
/// The flag stops new fetches; this deals with what is already here. BitGoose
/// held full extracted text from nine sources — CoinDesk, the FT, CNBC, The
/// Verge among them — that block the AI crawlers by name. That text was
/// gathered before there was any code capable of noticing, which explains it
/// and does not excuse keeping it.
///
/// The item survives: headline, link, canonical URL, its place in a cluster and
/// its citation on the page. Only `body_raw` goes, because only `body_raw` was
/// ever destined for a prompt. `extracted_at` is cleared so the item does not
/// read as "we looked and found nothing", and the source gate keeps it out of
/// the queue rather than re-fetching it forever.
pub async fn purge_declined_text(db: &Db) -> Result<u64> {
    let r = sqlx::query(
        "UPDATE raw_items r
            SET body_raw = NULL, extracted_at = NULL, extract_via = 'declined'
           FROM sources s
          WHERE s.id = r.source_id
            AND NOT s.ai_input_ok
            AND r.body_raw IS NOT NULL",
    )
    .execute(&db.pool)
    .await?;
    Ok(r.rows_affected())
}

/// How much text we are holding that its publisher has asked not be used this
/// way. Reported by `bg doctor`, so it cannot quietly return.
pub async fn declined_text_held(db: &Db) -> Result<i64> {
    let row = sqlx::query(
        "SELECT count(*)::bigint AS n FROM raw_items r JOIN sources s ON s.id = r.source_id
          WHERE NOT s.ai_input_ok AND r.body_raw IS NOT NULL",
    )
    .fetch_one(&db.pool)
    .await?;
    Ok(row.try_get("n")?)
}
