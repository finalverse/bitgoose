//! # bg-db
//!
//! PostgreSQL persistence for BitGoose.
//!
//! ## Why runtime queries, not `sqlx::query!`
//!
//! Every query here uses the runtime API rather than the compile-time-checked
//! macros. The macros need either a reachable `DATABASE_URL` or a committed
//! offline cache *at compile time*, which would mean `cargo build` fails on a
//! machine without the database up — including CI and the WASM pass. The
//! trade-off is that a column typo surfaces when the query runs rather than
//! when it compiles, so the repositories are exercised by integration tests
//! against a live schema.
//!
//! ## The private-body invariant
//!
//! `raw_items.body_raw` holds source text we may *read* but not republish. No
//! function in this crate returns it inside a type that gets serialized to a
//! client. The accessors that touch it are named for their purpose —
//! [`items::body_for_analysis`], [`items::bodies_for_story`] — and are called
//! only by the claim extractor and the policy engine's overlap check.

pub mod agents;
pub mod analyses;
pub mod articles;
pub mod claims;
pub mod convert;
pub mod declines;
pub mod entities;
pub mod gaggles;
pub mod items;
pub mod prices;
pub mod sources;
pub mod stories;
pub mod violations;

use sqlx::postgres::{PgArguments, PgPoolOptions};
use sqlx::query::Query;
use sqlx::{PgPool, Postgres, Row};
use std::time::Duration;
use thiserror::Error;

/// Build a query whose SQL was assembled at runtime.
///
/// sqlx 0.9 refuses non-`'static` SQL by default, which is a good default: it
/// makes accidental string-concatenated injection a compile error. Every call
/// site here interpolates exactly one thing — a `const` column list defined in
/// the same module — and never a value that came from a request, a feed, or a
/// model. Values are always bound with `$n`. Routing the assertion through this
/// one helper keeps that audit to a single place instead of scattering
/// `AssertSqlSafe` across thirty call sites where it would stop being read.
pub(crate) fn sql(s: String) -> Query<'static, Postgres, PgArguments> {
    sqlx::query(sqlx::AssertSqlSafe(s))
}

pub type Result<T, E = DbError> = std::result::Result<T, E>;

#[derive(Debug, Error)]
pub enum DbError {
    #[error(transparent)]
    Sql(#[from] sqlx::Error),

    #[error(transparent)]
    Migrate(#[from] sqlx::migrate::MigrateError),

    #[error("could not decode column `{column}`: {source}")]
    Decode {
        column: &'static str,
        source: bg_core::CoreError,
    },

    #[error("{0} not found")]
    NotFound(&'static str),
}

/// Embedding width the schema is built for. `vector(1536)` is fixed-width, so a
/// provider emitting a different dimension is rejected at write time with a
/// clear message rather than an opaque Postgres type error.
pub const EMBED_DIM: usize = 1536;

/// Connection handle. Cheap to clone — wraps an `Arc`'d pool.
#[derive(Clone, Debug)]
pub struct Db {
    pub pool: PgPool,
}

impl Db {
    /// Connect with pool limits sized for a web server and a batch agent
    /// pipeline sharing one database.
    pub async fn connect(url: &str) -> Result<Self> {
        let pool = PgPoolOptions::new()
            .max_connections(16)
            .min_connections(1)
            .acquire_timeout(Duration::from_secs(10))
            // Feed polling idles for minutes between bursts; recycling avoids
            // holding connections that a Postgres restart already killed.
            .idle_timeout(Duration::from_secs(600))
            .max_lifetime(Duration::from_secs(1800))
            .connect(url)
            .await?;
        Ok(Self { pool })
    }

    pub fn from_pool(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Applies every migration in `migrations/`. Idempotent.
    pub async fn migrate(&self) -> Result<()> {
        sqlx::migrate!("../../migrations").run(&self.pool).await?;
        Ok(())
    }

    pub async fn ping(&self) -> Result<()> {
        sqlx::query("SELECT 1").execute(&self.pool).await?;
        Ok(())
    }

    /// Installed pgvector version, or `None` if the extension is absent.
    pub async fn pgvector_version(&self) -> Result<Option<String>> {
        let row = sqlx::query("SELECT extversion FROM pg_extension WHERE extname = 'vector'")
            .fetch_optional(&self.pool)
            .await?;
        Ok(row.map(|r| r.get::<String, _>("extversion")))
    }

    pub async fn server_version(&self) -> Result<String> {
        let row = sqlx::query("SHOW server_version")
            .fetch_one(&self.pool)
            .await?;
        Ok(row.get::<String, _>(0))
    }

    /// Row counts for the tables `bg doctor` reports on.
    pub async fn counts(&self) -> Result<Vec<(String, i64)>> {
        // Fixed list — never interpolated from input.
        let tables = [
            "sources",
            "raw_items",
            "stories",
            "claims",
            "claim_sources",
            "articles",
            "agent_runs",
            "policy_violations",
            "price_ticks",
            "analyses",
        ];
        let mut out = Vec::with_capacity(tables.len());
        for t in tables {
            let n: i64 =
                sqlx::query_scalar(sqlx::AssertSqlSafe(format!("SELECT count(*) FROM {t}")))
                    .fetch_one(&self.pool)
                    .await?;
            out.push((t.to_string(), n));
        }
        Ok(out)
    }
}
