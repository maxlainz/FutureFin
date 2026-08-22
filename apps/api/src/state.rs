use crate::handlers::person_view::LedgerView;
use crate::handlers::projection::ProjectionSeriesResponse;
use sqlx::PgPool;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;
use uuid::Uuid;

/// TTL sliding del cache de proyección. Se refresca en cada hit.
pub const PROJECTION_CACHE_TTL: Duration = Duration::from_secs(60 * 60);

/// Densidad de puntos serializados.
/// - `Monthly`: ~841 puntos (uno por mes del horizonte).
/// - `Hybrid`: mes 0..12 mensual + mes 24, 36, ..., 840 anual → ~82 puntos.
///
/// Ambas comparten el mismo compute interno del engine (840 meses).
/// Solo cambia la serialización del response (qué puntos se incluyen en
/// `points`, `fire_target_series`, `asset_series[].values`).
#[derive(Hash, Eq, PartialEq, Clone, Copy, Debug)]
pub enum Density {
    Monthly,
    Hybrid,
}

/// Clave de cache. `owner_user_id` es SIEMPRE `Some(_)`, también en `household`:
/// la respuesta depende de la fecha de nacimiento del **solicitante** (horizonte,
/// `viewer_birth_date`, `jubilacion_age`, eje de edades), así que una entrada
/// household compartida servía la demografía de un miembro a otro.
#[derive(Hash, Eq, PartialEq, Clone, Debug)]
pub struct ProjectionCacheKey {
    pub installation_id: Uuid,
    pub view: LedgerView,
    pub owner_user_id: Option<Uuid>,
    pub density: Density,
}

pub struct ProjectionCacheEntry {
    pub response: Arc<ProjectionSeriesResponse>,
    pub last_used: Instant,
}

pub type ProjectionCacheMap = HashMap<ProjectionCacheKey, ProjectionCacheEntry>;

pub struct AppState {
    pub version: &'static str,
    pub pool: PgPool,
    pub cookie_secure: bool,
    pub session_ttl_days: i64,
    /// `FUTUREFIN_MCP_ENABLED` (default true). Con `false` el router `/mcp` ni se monta.
    pub mcp_enabled: bool,
    /// `FUTUREFIN_PUBLIC_URL` (opcional): origen público canónico (`https://host`, sin
    /// barra final), validado al arrancar. `None` ⇒ el issuer OAuth se deriva de los
    /// headers del request (X-Forwarded-Proto / Host).
    pub public_url: Option<String>,
    pub projection_cache: RwLock<ProjectionCacheMap>,
}

impl AppState {
    pub fn new(
        version: &'static str,
        pool: PgPool,
        cookie_secure: bool,
        session_ttl_days: i64,
        mcp_enabled: bool,
        public_url: Option<String>,
    ) -> Self {
        Self {
            version,
            pool,
            cookie_secure,
            session_ttl_days,
            mcp_enabled,
            public_url,
            projection_cache: RwLock::new(HashMap::new()),
        }
    }

    /// Hit del cache con sliding TTL. Devuelve `None` si no existe o expiró
    /// (la entry expirada se elimina del cache lazy).
    pub async fn projection_cache_get(
        &self,
        key: &ProjectionCacheKey,
    ) -> Option<Arc<ProjectionSeriesResponse>> {
        // Fast path: read lock, comprobar TTL sin mutar.
        {
            let cache = self.projection_cache.read().await;
            let entry = cache.get(key)?;
            if entry.last_used.elapsed() < PROJECTION_CACHE_TTL {
                let response = entry.response.clone();
                drop(cache);
                // Refresh sliding TTL en write lock corto.
                let mut cache = self.projection_cache.write().await;
                if let Some(e) = cache.get_mut(key) {
                    e.last_used = Instant::now();
                }
                return Some(response);
            }
        }
        // Expired: borrar.
        let mut cache = self.projection_cache.write().await;
        cache.remove(key);
        None
    }

    pub async fn projection_cache_insert(
        &self,
        key: ProjectionCacheKey,
        response: Arc<ProjectionSeriesResponse>,
    ) {
        let mut cache = self.projection_cache.write().await;
        cache.insert(
            key,
            ProjectionCacheEntry {
                response,
                last_used: Instant::now(),
            },
        );
    }

    /// Tras una mutación: borra todas las entries del installation. Ambas
    /// vistas (`household` + `mine` de todos los miembros) se invalidan
    /// porque cualquier cambio afecta la simulación.
    pub async fn invalidate_projection_by_installation(&self, installation_id: Uuid) {
        let mut cache = self.projection_cache.write().await;
        let before = cache.len();
        cache.retain(|key, _| key.installation_id != installation_id);
        let removed = before - cache.len();
        if removed > 0 {
            tracing::info!(
                installation_id = %installation_id,
                removed,
                "projection cache invalidated by installation"
            );
        }
    }

    /// Al logout: borra las entries de ese usuario — `mine` y `household`, porque
    /// desde el arreglo de la clave ambas son suyas. Las de otros miembros no se tocan.
    pub async fn invalidate_projection_by_user(&self, user_id: Uuid) {
        let mut cache = self.projection_cache.write().await;
        let before = cache.len();
        cache.retain(|key, _| key.owner_user_id != Some(user_id));
        let removed = before - cache.len();
        if removed > 0 {
            tracing::info!(
                user_id = %user_id,
                removed,
                "projection cache invalidated by user (logout)"
            );
        }
    }
}
