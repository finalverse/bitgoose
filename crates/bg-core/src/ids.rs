//! Newtyped UUIDs.
//!
//! Every table gets its own ID type so the compiler catches a `ClaimId` handed
//! to something expecting a `StoryId` — a class of bug that is otherwise
//! invisible in a graph this interconnected.

use serde::{Deserialize, Serialize};
use std::fmt;
use uuid::Uuid;

macro_rules! typed_id {
    ($($name:ident),+ $(,)?) => {
        $(
            #[derive(
                Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash,
                Serialize, Deserialize,
            )]
            #[serde(transparent)]
            pub struct $name(pub Uuid);

            impl $name {
                pub fn new() -> Self { Self(Uuid::new_v4()) }
                pub const fn from_uuid(u: Uuid) -> Self { Self(u) }
                pub const fn as_uuid(&self) -> &Uuid { &self.0 }
                pub const fn into_uuid(self) -> Uuid { self.0 }
            }

            impl Default for $name {
                fn default() -> Self { Self::new() }
            }

            impl fmt::Display for $name {
                fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                    fmt::Display::fmt(&self.0, f)
                }
            }

            impl From<Uuid> for $name {
                fn from(u: Uuid) -> Self { Self(u) }
            }

            impl From<$name> for Uuid {
                fn from(v: $name) -> Uuid { v.0 }
            }

            impl std::str::FromStr for $name {
                type Err = crate::error::CoreError;
                fn from_str(s: &str) -> Result<Self, Self::Err> {
                    Uuid::parse_str(s)
                        .map(Self)
                        .map_err(|_| crate::error::CoreError::parse(stringify!($name), s))
                }
            }
        )+
    };
}

typed_id!(
    SourceId,
    RawItemId,
    StoryId,
    ClaimId,
    ArticleId,
    CorrectionId,
    EntityId,
    AgentId,
    RunId,
    AssetId,
    ViolationId,
    AnalysisId,
);
