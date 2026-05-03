//! Shared primitives for financial correctness and stable identifiers across services.
//!
//! Currency amounts use [`rust_decimal::Decimal`] (no `f64` for currency or ledger-like values).

pub use rust_decimal::Decimal;
pub use uuid::Uuid;

/// Authenticated account (`AUTH_MODEL.md`: User — not a domain [`Person`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(transparent)]
pub struct UserId(pub Uuid);

impl UserId {
    #[must_use]
    pub fn random() -> Self {
        Self(Uuid::new_v4())
    }
}

impl From<Uuid> for UserId {
    fn from(value: Uuid) -> Self {
        Self(value)
    }
}
