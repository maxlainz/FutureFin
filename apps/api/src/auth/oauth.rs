//! OAuth 2.0 / OIDC extension points (not wired in MVP).
//!
//! Local authentication uses username + password ([`crate::handlers::auth`]).
//! Future self-hosted SSO (Authentik, Keycloak, …) or SaaS IdPs should:
//! 1. Validate tokens or authorization codes at the issuer.
//! 2. Map [`ExternalSubject`] → local [`futurefin_domain::UserId`] via `ensure_local_user`.

#![allow(dead_code)] // SSO hooks are intentional stubs until an IdP is wired up.

use async_trait::async_trait;
use futurefin_domain::UserId;

/// Stable identifier issued by an external authorization server.
#[derive(Debug, Clone)]
pub struct ExternalSubject {
    pub issuer: String,
    pub subject: String,
}

#[derive(Debug, thiserror::Error)]
pub enum OAuthIntegrationError {
    #[error("OAuth / OIDC integration is not configured")]
    NotConfigured,
    #[error("invalid token")]
    InvalidToken,
}

/// Validates ID tokens or opaque tokens at the issuer (implement per provider).
#[async_trait]
pub trait OidcTokenValidator: Send + Sync {
    async fn validate_id_token(
        &self,
        token: &str,
    ) -> Result<ExternalSubject, OAuthIntegrationError>;
}

/// Links or provisions a local user for SSO subjects (implement when enabling OAuth).
#[async_trait]
pub trait ExternalIdentityLinker: Send + Sync {
    async fn ensure_local_user(
        &self,
        external: &ExternalSubject,
    ) -> Result<UserId, OAuthIntegrationError>;
}
