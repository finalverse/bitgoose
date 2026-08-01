//! Claims and their provenance edges.

use crate::{convert::*, Db, Result};
use bg_core::domain::{Claim, ClaimKind, ClaimSourceRef, ClaimWithSources, Stance, Verification};
use bg_core::ids::{ClaimId, RawItemId, RunId, StoryId};
use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use sqlx::postgres::PgRow;
use sqlx::Row;
use uuid::Uuid;

const COLS: &str = "id, story_id, text, kind, confidence, verification, numeric_value, unit, \
                    as_of, created_by_run, created_at";

fn from_row(r: &PgRow) -> Result<Claim> {
    Ok(Claim {
        id: claim_id(r, "id")?,
        story_id: story_id(r, "story_id")?,
        text: r.try_get("text")?,
        kind: enum_col::<ClaimKind>(r, "kind")?,
        confidence: r.try_get("confidence")?,
        verification: enum_col::<Verification>(r, "verification")?,
        numeric_value: r.try_get("numeric_value")?,
        unit: r.try_get("unit")?,
        as_of: r.try_get("as_of")?,
        created_by_run: run_id_opt(r, "created_by_run")?,
        created_at: r.try_get("created_at")?,
    })
}

#[derive(Debug, Clone)]
pub struct NewClaim {
    pub text: String,
    pub kind: ClaimKind,
    pub numeric_value: Option<Decimal>,
    pub unit: Option<String>,
    pub as_of: Option<DateTime<Utc>>,
}

/// Insert a claim in the `unverified` state.
///
/// Confidence starts at zero on purpose: Sentinel raises it after cross-source
/// checking. A claim that was never verified therefore reads as unverified
/// rather than inheriting an optimistic default from whatever wrote it.
pub async fn insert(
    db: &Db,
    story: StoryId,
    c: &NewClaim,
    run: Option<RunId>,
) -> Result<ClaimId> {
    let id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO claims (id, story_id, text, kind, confidence, verification,
                             numeric_value, unit, as_of, created_by_run)
         VALUES ($1,$2,$3,$4,0,'unverified',$5,$6,$7,$8)",
    )
    .bind(id)
    .bind(story.into_uuid())
    .bind(&c.text)
    .bind(c.kind.as_str())
    .bind(c.numeric_value)
    .bind(&c.unit)
    .bind(c.as_of)
    .bind(run.map(|r| r.into_uuid()))
    .execute(&db.pool)
    .await?;
    Ok(ClaimId::from_uuid(id))
}

/// Attach evidence. `excerpt` is truncated to the policy cap before it reaches
/// the database, so the CHECK constraint is a backstop rather than the thing
/// callers trip over.
pub async fn add_source(
    db: &Db,
    claim: ClaimId,
    item: RawItemId,
    stance: Stance,
    excerpt: Option<&str>,
) -> Result<()> {
    let trimmed =
        excerpt.map(|e| bg_core::text::truncate_words(e, bg_core::policy::MAX_QUOTE_WORDS));
    sqlx::query(
        "INSERT INTO claim_sources (claim_id, raw_item_id, stance, excerpt)
         VALUES ($1,$2,$3,$4)
         ON CONFLICT (claim_id, raw_item_id) DO UPDATE
            SET stance = EXCLUDED.stance, excerpt = EXCLUDED.excerpt",
    )
    .bind(claim.into_uuid())
    .bind(item.into_uuid())
    .bind(stance.as_str())
    .bind(trimmed)
    .execute(&db.pool)
    .await?;
    Ok(())
}

pub async fn set_verification(
    db: &Db,
    claim: ClaimId,
    v: Verification,
    confidence: f32,
) -> Result<()> {
    sqlx::query("UPDATE claims SET verification = $2, confidence = $3 WHERE id = $1")
        .bind(claim.into_uuid())
        .bind(v.as_str())
        .bind(confidence.clamp(0.0, 1.0))
        .execute(&db.pool)
        .await?;
    Ok(())
}

pub async fn by_story(db: &Db, story: StoryId) -> Result<Vec<Claim>> {
    let rows = crate::sql(format!(
        "SELECT {COLS} FROM claims WHERE story_id = $1 ORDER BY created_at ASC"
    ))
    .bind(story.into_uuid())
    .fetch_all(&db.pool)
    .await?;
    rows.iter().map(from_row).collect()
}

pub async fn by_id(db: &Db, id: ClaimId) -> Result<Claim> {
    let row = crate::sql(format!("SELECT {COLS} FROM claims WHERE id = $1"))
        .bind(id.into_uuid())
        .fetch_optional(&db.pool)
        .await?
        .ok_or(crate::DbError::NotFound("claim"))?;
    from_row(&row)
}

/// Distinct sources backing each claim on a story.
///
/// Counted per *source*, not per item: an outlet that syndicates the same wire
/// copy to three URLs is one corroborating voice, and treating it as three is
/// exactly the failure that makes aggregators look confident about nothing.
pub async fn source_counts(db: &Db, story: StoryId) -> Result<Vec<(ClaimId, i64)>> {
    let rows = sqlx::query(
        "SELECT c.id, count(DISTINCT r.source_id) AS n
         FROM claims c
         LEFT JOIN claim_sources cs ON cs.claim_id = c.id AND cs.stance = 'supports'
         LEFT JOIN raw_items r ON r.id = cs.raw_item_id
         WHERE c.story_id = $1
         GROUP BY c.id",
    )
    .bind(story.into_uuid())
    .fetch_all(&db.pool)
    .await?;
    rows.iter().map(|r| Ok((claim_id(r, "id")?, r.try_get("n")?))).collect()
}

/// Claims with full provenance — what the story page's ledger sidebar renders.
pub async fn with_sources(db: &Db, story: StoryId) -> Result<Vec<ClaimWithSources>> {
    let claims = by_story(db, story).await?;
    if claims.is_empty() {
        return Ok(Vec::new());
    }
    let ids: Vec<Uuid> = claims.iter().map(|c| c.id.into_uuid()).collect();
    let rows = sqlx::query(
        "SELECT cs.claim_id, cs.raw_item_id, cs.stance, cs.excerpt,
                s.name AS source_name, s.slug AS source_slug, s.trust AS source_trust,
                r.canonical_url AS url, r.title, r.published_at
         FROM claim_sources cs
         JOIN raw_items r ON r.id = cs.raw_item_id
         JOIN sources s   ON s.id = r.source_id
         WHERE cs.claim_id = ANY($1)
         ORDER BY s.trust DESC",
    )
    .bind(&ids)
    .fetch_all(&db.pool)
    .await?;

    let mut by_claim: std::collections::HashMap<ClaimId, Vec<ClaimSourceRef>> =
        std::collections::HashMap::new();
    for r in &rows {
        let cid = claim_id(r, "claim_id")?;
        by_claim.entry(cid).or_default().push(ClaimSourceRef {
            raw_item_id: raw_item_id(r, "raw_item_id")?,
            stance: enum_col::<Stance>(r, "stance")?,
            excerpt: r.try_get("excerpt")?,
            source_name: r.try_get("source_name")?,
            source_slug: r.try_get("source_slug")?,
            source_trust: r.try_get("source_trust")?,
            url: r.try_get("url")?,
            title: r.try_get("title")?,
            published_at: r.try_get("published_at")?,
        });
    }

    Ok(claims
        .into_iter()
        .map(|c| {
            let sources = by_claim.remove(&c.id).unwrap_or_default();
            ClaimWithSources { claim: c, sources }
        })
        .collect())
}
