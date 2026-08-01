//! Row → domain conversions.
//!
//! Enum columns are `TEXT` with a CHECK constraint rather than a Postgres
//! `ENUM` type: readable in `psql`, and adding a variant is an ordinary
//! migration instead of a catalog rewrite. The cost is that decoding has to
//! parse, and a value that somehow got past the CHECK must fail loudly rather
//! than silently defaulting — a story that quietly becomes `triage` because its
//! status did not parse is a story that vanishes from the site with no error.

use crate::DbError;
use bg_core::ids::*;
use sqlx::postgres::PgRow;
use sqlx::Row;
use std::str::FromStr;
use uuid::Uuid;

/// Parse a `TEXT` enum column into its domain type.
pub fn enum_col<T: FromStr<Err = bg_core::CoreError>>(
    row: &PgRow,
    column: &'static str,
) -> Result<T, DbError> {
    let raw: String = row.try_get(column)?;
    T::from_str(&raw).map_err(|source| DbError::Decode { column, source })
}

/// Parse an optional `TEXT` enum column. A NULL yields `None`; a present but
/// unparseable value is still an error.
pub fn enum_col_opt<T: FromStr<Err = bg_core::CoreError>>(
    row: &PgRow,
    column: &'static str,
) -> Result<Option<T>, DbError> {
    let raw: Option<String> = row.try_get(column)?;
    match raw {
        None => Ok(None),
        Some(s) => T::from_str(&s).map(Some).map_err(|source| DbError::Decode { column, source }),
    }
}

macro_rules! id_col_fns {
    ($($fn_name:ident => $ty:ident),+ $(,)?) => {
        $(
            pub fn $fn_name(row: &PgRow, column: &'static str) -> Result<$ty, DbError> {
                Ok($ty::from_uuid(row.try_get::<Uuid, _>(column)?))
            }
        )+
    };
}

id_col_fns!(
    source_id => SourceId,
    raw_item_id => RawItemId,
    story_id => StoryId,
    claim_id => ClaimId,
    article_id => ArticleId,
    correction_id => CorrectionId,
    entity_id => EntityId,
    agent_id => AgentId,
    run_id => RunId,
    asset_id => AssetId,
    violation_id => ViolationId,
);

pub fn story_id_opt(row: &PgRow, column: &'static str) -> Result<Option<StoryId>, DbError> {
    Ok(row.try_get::<Option<Uuid>, _>(column)?.map(StoryId::from_uuid))
}

pub fn run_id_opt(row: &PgRow, column: &'static str) -> Result<Option<RunId>, DbError> {
    Ok(row.try_get::<Option<Uuid>, _>(column)?.map(RunId::from_uuid))
}

pub fn agent_id_opt(row: &PgRow, column: &'static str) -> Result<Option<AgentId>, DbError> {
    Ok(row.try_get::<Option<Uuid>, _>(column)?.map(AgentId::from_uuid))
}

/// Postgres has no unsigned 64-bit integer, so SimHash values round-trip
/// through `BIGINT` as their two's-complement bit pattern. Reinterpreting the
/// bits is lossless — and it must be `as`, not a checked cast, or every
/// fingerprint with the top bit set would be rejected.
pub const fn simhash_to_db(v: u64) -> i64 {
    v as i64
}

pub const fn simhash_from_db(v: i64) -> u64 {
    v as u64
}

/// Format a float vector as a pgvector literal, e.g. `[0.1,0.2]`.
pub fn vector_literal(v: &[f32]) -> String {
    let mut s = String::with_capacity(v.len() * 8 + 2);
    s.push('[');
    for (i, x) in v.iter().enumerate() {
        if i > 0 {
            s.push(',');
        }
        s.push_str(&format!("{x}"));
    }
    s.push(']');
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn simhash_round_trips_including_the_high_bit() {
        for v in [0u64, 1, u64::MAX, 1 << 63, 0xdead_beef_dead_beef] {
            assert_eq!(simhash_from_db(simhash_to_db(v)), v);
        }
    }

    #[test]
    fn vector_literal_matches_pgvector_syntax() {
        assert_eq!(vector_literal(&[]), "[]");
        assert_eq!(vector_literal(&[1.0, -0.5]), "[1,-0.5]");
    }
}
