use crate::handlers::person_view::LedgerView;
use crate::handlers::projection::ProjectionSeriesResponse;
use crate::handlers::projection_bands::ProjectionBandsResponse;
use sqlx::PgPool;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{Mutex, RwLock};
use uuid::Uuid;

/// TTL sliding del cache de proyección. Se refresca en cada hit.
pub const PROJECTION_CACHE_TTL: Duration = Duration::from_secs(60 * 60);

/// Densidad de puntos serializados.
/// - `Monthly`: ~841 puntos (uno por mes del horizonte).
/// - `Hybrid`: mes 0..12 mensual + mes 24, 36, ..., 840 anual → ~82 puntos.
///
/// Ambas comparten el mismo compute interno del engine (840 meses).
/// Solo cambia la serialización del response (qué puntos se incluyen en
/// `points`, `asset_series[].values`).
#[derive(Hash, Eq, PartialEq, Clone, Copy, Debug)]
pub enum Density {
    Monthly,
    Hybrid,
}

/// Clave de cache. `owner_user_id` es SIEMPRE `Some(_)`, también en `household`:
/// la respuesta depende de la fecha de nacimiento del **solicitante** (horizonte,
/// `viewer_birth_date`, eje de edades), así que una entrada
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

/// Clave del cache de **bandas de Monte Carlo** (5.0.0, §F del plan de #207).
///
/// No lleva `view` y no es un olvido: las bandas solo existen en `view=mine`
/// (`household_bands_unavailable`, ver `projection_bands.rs`), así que un campo con un solo valor
/// posible solo serviría para que alguien creyera que hay una entrada `household` que buscar.
///
/// Sí llevan `paths` y `seed`: los dos son ENTRADA del sorteo, no del entorno. Dos peticiones con
/// semillas distintas describen dos mercados distintos y compartir entrada entre ellas serviría
/// una respuesta que no corresponde a la pregunta — el mismo error que la clave de proyección
/// arregló con `owner_user_id`.
#[derive(Hash, Eq, PartialEq, Clone, Debug)]
pub struct BandsCacheKey {
    pub installation_id: Uuid,
    pub user_id: Uuid,
    pub paths: u32,
    pub seed: u64,
}

pub struct BandsCacheEntry {
    pub response: Arc<ProjectionBandsResponse>,
    pub last_used: Instant,
}

pub type BandsCacheMap = HashMap<BandsCacheKey, BandsCacheEntry>;

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
    /// `FUTUREFIN_BASE_PATH` (opcional, normalizado, `""` = raíz): prefijo fijo para
    /// despliegues tras proxy con subpath. Los headers `X-Ingress-Path` /
    /// `X-Forwarded-Prefix` tienen precedencia por request (ver `crate::prefix`).
    pub base_path: String,
    /// `FUTUREFIN_TRUSTED_PROXY_IPS`: peers cuya palabra sobre identidad y embebido en
    /// iframe se acepta. `Disabled` (default) = nadie.
    pub trusted_peers: crate::prefix::PeerPolicy,
    /// `FUTUREFIN_TRUSTED_PROXY_AUTH` (default false): habilita `POST /v1/auth/sso`
    /// (identidad por cabeceras `X-Remote-User-*` desde un peer de confianza).
    pub trusted_header_auth: bool,
    /// «Entrar con Home Assistant» (`FUTUREFIN_HA_SSO_URL` + `FUTUREFIN_HA_ADDON=1`).
    /// `None` (el default) = la instalación no ofrece ese login y `/v1/auth/ha/start`
    /// responde `ha_sso_disabled`. Predicado único: `ha_idp::ha_login_available`.
    pub ha_sso: Option<HaSso>,
    pub projection_cache: RwLock<ProjectionCacheMap>,
    /// Cache de las bandas de Monte Carlo. **Propio y no una densidad más del de proyección**: su
    /// clave lleva dos ejes que la serie no tiene (`paths`, `seed`) y su contenido cuesta un orden
    /// de magnitud más (500 simulaciones f64 frente a una `Decimal`), así que mezclarlos habría
    /// hecho que un cambio de semilla tirara la serie determinista por el suelo.
    ///
    /// Comparte TTL (`PROJECTION_CACHE_TTL`) y —lo que de verdad importa— **las dos
    /// invalidaciones**: `invalidate_projection_by_installation` y `..._by_user` borran los dos
    /// mapas. Una banda calculada sobre unos activos que ya no existen es peor que no tener banda.
    pub bands_cache: RwLock<BandsCacheMap>,
    /// **Generación del cache de proyección, por instalación** (5.0.0, WP A12).
    ///
    /// # El fallo que cierra
    ///
    /// Un miss de proyección (o un warm-up) computa durante segundos y DESPUÉS inserta. Si entre
    /// medias una mutación invalida, la inserción llegaba tarde y **repoblaba la cache con la foto
    /// de antes**: el usuario editaba un activo y seguía viendo la cifra vieja hasta el TTL. En
    /// 4.x la ventana era de ~500 ms y se toleraba; desde 5.0.0 el nivel 1 vive DENTRO de ese
    /// permiso y la ventana es de segundos — el arnés de tests ya la vio (el warm-up del login
    /// repoblando la cache después de la invalidación de un PATCH).
    ///
    /// # Cómo se cierra
    ///
    /// Todo camino que inserte captura la generación **antes** de computar y la presenta al
    /// insertar; si no coincide, la entrada se descarta. Las dos invalidaciones incrementan.
    ///
    /// **El orden de los locks es lo que lo hace hermético**, y es siempre el mismo: primero este
    /// mutex, después el mapa. La invalidación incrementa y limpia **sosteniendo los dos**, y la
    /// inserción comprueba e inserta **sosteniendo los dos**, así que no existe el intervalo en el
    /// que una comprobación ya pasó y la limpieza todavía no ha llegado. Con la generación en un
    /// lock aparte que se soltara antes de insertar, ese intervalo volvería y con él el bug.
    /// Nadie toma los locks en el orden contrario (el `get` del cache no mira la generación), así
    /// que no hay ciclo posible.
    ///
    /// Es un `HashMap` y no un `AtomicU64` global para que una mutación en una instalación no tire
    /// el cómputo en vuelo de otra. Una instalación que no está en el mapa vale 0.
    projection_generation: Mutex<HashMap<Uuid, u64>>,
}

/// Configuración viva del login con Home Assistant: el origen público de HA y el proveedor.
///
/// El proveedor va tras un `Arc<dyn …>` para que los tests de integración puedan inyectar un
/// doble sin levantar un Home Assistant — el mismo patrón por el que el resto del estado no
/// guarda clientes concretos.
pub struct HaSso {
    /// Origen público de Home Assistant (`https://ha.example.org`, sin barra final).
    pub base_url: String,
    pub idp: Arc<dyn crate::ha_idp::HaIdp>,
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
            base_path: String::new(),
            trusted_peers: crate::prefix::PeerPolicy::Disabled,
            trusted_header_auth: false,
            ha_sso: None,
            projection_cache: RwLock::new(HashMap::new()),
            bands_cache: RwLock::new(HashMap::new()),
            projection_generation: Mutex::new(HashMap::new()),
        }
    }

    /// Proveedor de «Entrar con Home Assistant». Aparte de `new()` por la misma razón que
    /// `with_trusted_proxy`: el default (`None`) es el comportamiento histórico y los call
    /// sites que no lo necesitan no se enteran.
    pub fn with_ha_idp(mut self, ha_sso: Option<HaSso>) -> Self {
        self.ha_sso = ha_sso;
        self
    }

    /// Configuración de proxy inverso (subpath + confianza). Aparte de `new()` para no
    /// tocar los call sites que no la necesitan (los defaults son el comportamiento
    /// histórico: sin prefijo, sin peers de confianza, sin SSO).
    pub fn with_trusted_proxy(
        mut self,
        base_path: String,
        trusted_peers: crate::prefix::PeerPolicy,
        trusted_header_auth: bool,
    ) -> Self {
        self.base_path = base_path;
        self.trusted_peers = trusted_peers;
        self.trusted_header_auth = trusted_header_auth;
        self
    }

    /// Prefijo efectivo de una request (ver `crate::prefix::request_prefix`).
    pub fn request_prefix(&self, headers: &http::HeaderMap) -> String {
        crate::prefix::request_prefix(&self.base_path, headers)
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

    /// **La generación actual** de una instalación, para capturarla ANTES de computar. Una
    /// instalación que nadie ha invalidado todavía vale 0. Ver [`Self::projection_generation`].
    pub async fn projection_generation(&self, installation_id: Uuid) -> u64 {
        self.projection_generation
            .lock()
            .await
            .get(&installation_id)
            .copied()
            .unwrap_or(0)
    }

    /// Guarda una respuesta de proyección **solo si la generación que el llamante capturó antes
    /// de computar sigue vigente**. Devuelve `false` si se descartó.
    ///
    /// **`generation` es un parámetro a propósito** (5.0.0, WP A12): no existe una versión
    /// sin guardia de este método, así que ningún camino nuevo puede olvidarse de la carrera. Lo
    /// que se descarta es una respuesta **correcta pero vieja**: el siguiente GET la recalcula
    /// contra los datos de ahora, que es exactamente lo que la mutación pidió.
    pub async fn projection_cache_insert_if_current(
        &self,
        key: ProjectionCacheKey,
        generation: u64,
        response: Arc<ProjectionSeriesResponse>,
    ) -> bool {
        // Orden de locks: generación → mapa, el MISMO que usan las dos invalidaciones. Se sostiene
        // la generación hasta después de insertar, así que una invalidación no puede colarse entre
        // la comprobación y la escritura (ver el doc de `projection_generation`).
        let generations = self.projection_generation.lock().await;
        let current = generations.get(&key.installation_id).copied().unwrap_or(0);
        if current != generation {
            tracing::info!(
                installation_id = %key.installation_id,
                captured = generation,
                current,
                "projection compute discarded: a mutation invalidated while it was computing"
            );
            return false;
        }
        let mut cache = self.projection_cache.write().await;
        cache.insert(
            key,
            ProjectionCacheEntry {
                response,
                last_used: Instant::now(),
            },
        );
        drop(cache);
        drop(generations);
        true
    }

    /// Hit del cache de bandas, con el MISMO TTL sliding que la proyección.
    pub async fn bands_cache_get(&self, key: &BandsCacheKey) -> Option<Arc<ProjectionBandsResponse>> {
        {
            let cache = self.bands_cache.read().await;
            let entry = cache.get(key)?;
            if entry.last_used.elapsed() < PROJECTION_CACHE_TTL {
                let response = entry.response.clone();
                drop(cache);
                let mut cache = self.bands_cache.write().await;
                if let Some(e) = cache.get_mut(key) {
                    e.last_used = Instant::now();
                }
                return Some(response);
            }
        }
        let mut cache = self.bands_cache.write().await;
        cache.remove(key);
        None
    }

    /// Gemelo de [`Self::projection_cache_insert_if_current`] para las bandas, y por la misma
    /// razón: un sorteo de 2.500 caminos tarda lo suyo, y una banda insertada después de la
    /// invalidación describiría unos activos que ya no existen **junto a** una línea determinista
    /// ya actualizada. Las dos caches comparten generación porque comparten `ProjectionInput`.
    pub async fn bands_cache_insert_if_current(
        &self,
        key: BandsCacheKey,
        generation: u64,
        response: Arc<ProjectionBandsResponse>,
    ) -> bool {
        let generations = self.projection_generation.lock().await;
        let current = generations.get(&key.installation_id).copied().unwrap_or(0);
        if current != generation {
            tracing::info!(
                installation_id = %key.installation_id,
                captured = generation,
                current,
                "bands compute discarded: a mutation invalidated while it was computing"
            );
            return false;
        }
        let mut cache = self.bands_cache.write().await;
        cache.insert(
            key,
            BandsCacheEntry {
                response,
                last_used: Instant::now(),
            },
        );
        drop(cache);
        drop(generations);
        true
    }

    /// Tras una mutación: borra todas las entries del installation. Ambas
    /// vistas (`household` + `mine` de todos los miembros) se invalidan
    /// porque cualquier cambio afecta la simulación.
    ///
    /// **Desde 5.0.0 borra también las bandas** (`bands_cache`). Van juntas a propósito: las
    /// bandas salen del MISMO `ProjectionInput` que la serie, así que toda mutación que
    /// invalide una invalida la otra por construcción. Separarlas dejaría un fan chart calculado
    /// sobre activos borrados junto a una línea determinista ya actualizada — dos cifras que se
    /// contradicen en la misma pantalla, que es el peor fallo de cache posible.
    pub async fn invalidate_projection_by_installation(&self, installation_id: Uuid) {
        // **Primero la generación, y sin soltarla hasta haber limpiado los dos mapas** (WP A12).
        // Un cómputo en vuelo cae por fuerza en uno de dos lados: si comprueba ANTES de esta
        // línea, su generación queda obsoleta al incrementarla y su inserción se descarta; si
        // comprueba DESPUÉS, se queda esperando este mutex y no llega a insertar hasta que los dos
        // `retain` han pasado, así que lo que escriba ya no lo borra nadie — y no hace falta que
        // nadie lo borre, porque a esas alturas su generación tampoco cuadra. Lo que NO puede
        // ocurrir es lo de en medio: comprobar con la generación buena e insertar después de la
        // limpieza. Ver el doc de `projection_generation`.
        let mut generations = self.projection_generation.lock().await;
        let generation = generations.entry(installation_id).or_insert(0);
        *generation = generation.wrapping_add(1);
        let mut cache = self.projection_cache.write().await;
        let before = cache.len();
        cache.retain(|key, _| key.installation_id != installation_id);
        let removed = before - cache.len();
        drop(cache);
        let mut bands = self.bands_cache.write().await;
        let bands_before = bands.len();
        bands.retain(|key, _| key.installation_id != installation_id);
        let bands_removed = bands_before - bands.len();
        drop(bands);
        drop(generations);
        if removed > 0 || bands_removed > 0 {
            tracing::info!(
                installation_id = %installation_id,
                removed,
                bands_removed,
                "projection cache invalidated by installation"
            );
        }
    }

    /// Al logout: borra las entries de ese usuario — `mine` y `household`, porque
    /// desde el arreglo de la clave ambas son suyas. Las de otros miembros no se tocan.
    pub async fn invalidate_projection_by_user(&self, user_id: Uuid) {
        // Misma disciplina que la invalidación por instalación, con una diferencia que hay que
        // decir: aquí el llamante trae un USUARIO y la generación se lleva por INSTALACIÓN, así
        // que se incrementa la de **todas las instalaciones que el mapa conoce**. Es barato (una
        // instalación por despliegue, y el mapa solo crece con las que ya se han invalidado o
        // computado alguna vez) y cubre el caso real. Queda fuera una instalación que este proceso
        // no ha visto nunca: entonces su primer warm-up puede insertar después de este logout, y
        // el resultado es una entrada de datos CORRECTOS para un usuario que acaba de salir —
        // higiene, no una cifra equivocada, que es lo que la generación protege.
        let mut generations = self.projection_generation.lock().await;
        let mut cache = self.projection_cache.write().await;
        // Las instalaciones de las entradas que se van a borrar entran en el mapa antes de
        // incrementar, para que una instalación cuya primera invalidación sea ESTA también quede
        // cubierta.
        for key in cache.keys() {
            if key.owner_user_id == Some(user_id) {
                generations.entry(key.installation_id).or_insert(0);
            }
        }
        for generation in generations.values_mut() {
            *generation = generation.wrapping_add(1);
        }
        let before = cache.len();
        cache.retain(|key, _| key.owner_user_id != Some(user_id));
        let removed = before - cache.len();
        drop(cache);
        // Mismo criterio para las bandas: son del usuario por construcción (`view=mine`).
        let mut bands = self.bands_cache.write().await;
        let bands_before = bands.len();
        bands.retain(|key, _| key.user_id != user_id);
        let bands_removed = bands_before - bands.len();
        drop(bands);
        drop(generations);
        if removed > 0 || bands_removed > 0 {
            tracing::info!(
                user_id = %user_id,
                removed,
                bands_removed,
                "projection cache invalidated by user (logout)"
            );
        }
    }
}
