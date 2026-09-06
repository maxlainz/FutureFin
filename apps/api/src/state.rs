use crate::handlers::person_view::LedgerView;
use crate::handlers::projection::ProjectionSeriesResponse;
use crate::handlers::projection_bands::ProjectionBandsResponse;
use crate::handlers::retirement_solver::{PlanExtras, PlanKey, PLAN_CACHE_MAX_ENTRIES, PLAN_CACHE_TTL};
use sqlx::PgPool;
use std::collections::{HashMap, HashSet};
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
    /// **La clave del PLAN que esta respuesta describe** (5.0.0, WP A3). `None` = esta entrada no
    /// tiene plan resuelto: horizonte a medida (`?months=`), miembro del hogar sin solve, o
    /// usuario sin fecha de nacimiento — los casos que publican `plan_absent_reason`.
    ///
    /// Existe para que un **HIT** de la serie pueda mirar el nivel 2 (`plan_cache`) sin reconstruir
    /// el `ProjectionInput`: sin este campo, servir la curva de capital desde una respuesta
    /// cacheada obligaría a rehacer el ensamblado entero —decenas de queries— solo para volver a
    /// calcular una huella que ya se calculó una vez.
    ///
    /// Es `Option` y no un `PlanKey` a secas porque **hay respuestas sin plan**, y una clave
    /// inventada para rellenar el hueco apuntaría a los extras de otro hogar.
    pub plan_key: Option<PlanKey>,
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
    /// **El umbral de éxito del perfil** (5.0.0, modelo v2). Está en la clave porque está en la
    /// RESPUESTA: desde 5.0.0 las bandas publican el veredicto contra el umbral del usuario
    /// (`success_verdict`) y lo ecoan (`success_threshold_pct`), así que dos umbrales distintos
    /// describen dos respuestas distintas del mismo sorteo. Sin este eje, cambiar el umbral en
    /// Ajustes devolvería el veredicto del umbral anterior — verde donde tocaba ámbar — sin que
    /// ningún campo lo dijera.
    pub threshold_pct: u32,
}

pub struct BandsCacheEntry {
    pub response: Arc<ProjectionBandsResponse>,
    pub last_used: Instant,
}

pub type BandsCacheMap = HashMap<BandsCacheKey, BandsCacheEntry>;

/// El estado de una entrada de la **cache de plan** (nivel 2 de `retirement_solver`).
///
/// Dos estados y ninguno más. La ausencia de entrada es un tercer caso —«nadie lo ha pedido»— y
/// significa otra cosa que [`PlanCacheSlot::Pending`]: quien lee publica `unavailable` en el
/// primero y `computing` en el segundo, y confundirlos le dice al usuario «no se puede» mientras
/// se está calculando.
#[derive(Clone)]
pub enum PlanCacheSlot {
    /// El nivel 2 está en vuelo. Se registra **antes** de que la petición que lo lanzó responda,
    /// para que ningún lector caiga en la ventana en que la tarea existe y la entrada no.
    Pending,
    /// Terminó. `PlanExtras::state` dice si con resultado o con su razón de fallo: una entrada
    /// fallida **también se guarda**, porque reintentar un sorteo de veinticinco segundos en cada
    /// GET sería peor que decir «no disponible» durante el TTL.
    Done(Arc<PlanExtras>),
}

pub struct PlanCacheEntry {
    pub slot: PlanCacheSlot,
    pub last_used: Instant,
}

pub type PlanCacheMap = HashMap<PlanKey, PlanCacheEntry>;

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
    /// **Cache del NIVEL 2 del plan de jubilación** (5.0.0, WP A3): las dos fechas de referencia,
    /// la curva de capital por edad, la tira anual de éxito y el fallo acumulado por edad. Del
    /// orden de veinticinco segundos de CPU por entrada, así que se calcula en segundo plano y se
    /// guarda.
    ///
    /// # Por qué este mapa NO se invalida
    ///
    /// Los otros dos se invalidan porque su clave nombra un HOGAR (`installation_id`,
    /// `user_id`) y el contenido cuelga de unos datos que pueden cambiar: tras una mutación, la
    /// entrada sigue siendo alcanzable y ya no describe la realidad.
    ///
    /// La clave de este es un **hash del contenido** (`plan_fingerprint`: la entrada entera del
    /// motor, las volatilidades, el umbral, los caminos y la semilla). Cambiar un activo, el
    /// umbral o la semilla produce **otra clave**, así que la entrada vieja deja de tener quien la
    /// pida: no hay ninguna petición que pueda servirse de ella por error. Una entrada obsoleta
    /// aquí es **inalcanzable, nunca peligrosa** — que es la razón por la que un `retain` por
    /// instalación no compraría nada, y además no podría escribirse: la clave no lleva
    /// `installation_id`, y llevarlo la haría dejar de ser una huella del contenido.
    ///
    /// Lo único que hay que evitar es que crezca sin límite, y de eso se ocupan el TTL (el mismo
    /// de la proyección) y el tope LRU de `PLAN_CACHE_MAX_ENTRIES`, los dos aplicados en
    /// [`AppState::plan_cache_insert`].
    pub plan_cache: RwLock<PlanCacheMap>,
    /// **Claves del nivel 2 en vuelo**, para que dos peticiones concurrentes del mismo hogar
    /// lancen **un** solo cálculo. Un `Mutex` y no un `RwLock` porque toda operación aquí escribe
    /// (`insert` / `remove`); un lock de lectura no tendría usuarios.
    pub plan_inflight: Mutex<HashSet<PlanKey>>,
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
            plan_cache: RwLock::new(HashMap::new()),
            plan_inflight: Mutex::new(HashSet::new()),
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

    /// Guarda una respuesta de proyección **con la clave de su plan**.
    ///
    /// `plan_key` es un parámetro y no un campo opcional que se rellena luego a propósito: es la
    /// única forma de que sea **imposible olvidarlo**. Una entrada guardada sin su clave publicaría
    /// `computing` para siempre —el nivel 2 nunca se buscaría ni se lanzaría— y ese fallo no
    /// levanta ningún assert de tipo. `None` es una respuesta legítima (sin plan: `?months=`,
    /// miembro del hogar sin solve, usuario sin fecha de nacimiento), pero hay que escribirla.
    pub async fn projection_cache_insert(
        &self,
        key: ProjectionCacheKey,
        response: Arc<ProjectionSeriesResponse>,
        plan_key: Option<PlanKey>,
    ) {
        let mut cache = self.projection_cache.write().await;
        cache.insert(
            key,
            ProjectionCacheEntry {
                response,
                last_used: Instant::now(),
                plan_key,
            },
        );
    }

    /// La clave del plan de una entrada cacheada, si la tiene. **No refresca el TTL**: quien lo
    /// refresca es `projection_cache_get`, que es quien de verdad sirve la respuesta.
    pub async fn projection_cache_plan_key(&self, key: &ProjectionCacheKey) -> Option<PlanKey> {
        self.projection_cache.read().await.get(key)?.plan_key
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

    pub async fn bands_cache_insert(
        &self,
        key: BandsCacheKey,
        response: Arc<ProjectionBandsResponse>,
    ) {
        let mut cache = self.bands_cache.write().await;
        cache.insert(
            key,
            BandsCacheEntry {
                response,
                last_used: Instant::now(),
            },
        );
    }

    // ---------------------------------------------------------------------------------------
    // Cache de plan (nivel 2) — **direccionada por CONTENIDO**, ver el doc de `plan_cache`.
    // ---------------------------------------------------------------------------------------

    /// Hit de la cache de plan, con el MISMO TTL sliding que la proyección. `None` = no hay
    /// entrada (que **no** es lo mismo que [`PlanCacheSlot::Pending`]: uno se publica como
    /// `unavailable` y el otro como `computing`).
    pub async fn plan_cache_get(&self, key: &PlanKey) -> Option<PlanCacheSlot> {
        {
            let cache = self.plan_cache.read().await;
            let entry = cache.get(key)?;
            if entry.last_used.elapsed() < PLAN_CACHE_TTL {
                let slot = entry.slot.clone();
                drop(cache);
                let mut cache = self.plan_cache.write().await;
                if let Some(e) = cache.get_mut(key) {
                    e.last_used = Instant::now();
                }
                return Some(slot);
            }
        }
        let mut cache = self.plan_cache.write().await;
        cache.remove(key);
        None
    }

    /// Refresca el TTL de una entrada **sin leerla**, y dice si existía y sigue viva.
    ///
    /// Lo usa un HIT de la serie: la respuesta ya está cacheada y el nivel 2 no hace falta
    /// clonarlo, pero la entrada del plan se está usando y desalojarla por LRU mientras su serie
    /// sigue caliente obligaría a recalcular veinticinco segundos de sorteos.
    pub async fn plan_cache_touch(&self, key: &PlanKey) -> bool {
        let mut cache = self.plan_cache.write().await;
        match cache.get_mut(key) {
            Some(e) if e.last_used.elapsed() < PLAN_CACHE_TTL => {
                e.last_used = Instant::now();
                true
            }
            Some(_) => {
                cache.remove(key);
                false
            }
            None => false,
        }
    }

    /// Inserta (o reemplaza) una entrada y **acota el mapa**, en este orden:
    ///
    /// 1. se caen las entradas **expiradas** por TTL — barrer aquí es lo que hace que una cache
    ///    que nadie vuelve a leer no se quede con su memoria retenida para siempre;
    /// 2. se inserta la nueva;
    /// 3. mientras sobren entradas, se desaloja la de `last_used` más antiguo (**LRU**).
    ///
    /// El desalojo mira `last_used` y no distingue [`PlanCacheSlot::Pending`] de `Done` a
    /// propósito: desalojar un `Pending` solo hace que un lector publique `unavailable` en vez de
    /// `computing` durante unos segundos —la tarea sigue viva y volverá a insertar—, mientras que
    /// protegerlos abriría la puerta a un mapa lleno de `Pending` que el tope no puede acotar.
    pub async fn plan_cache_insert(&self, key: PlanKey, slot: PlanCacheSlot) {
        let mut cache = self.plan_cache.write().await;
        cache.retain(|_, e| e.last_used.elapsed() < PLAN_CACHE_TTL);
        cache.insert(
            key,
            PlanCacheEntry {
                slot,
                last_used: Instant::now(),
            },
        );
        while cache.len() > PLAN_CACHE_MAX_ENTRIES {
            let Some(oldest) = cache
                .iter()
                .min_by_key(|(_, e)| e.last_used)
                .map(|(k, _)| *k)
            else {
                break;
            };
            cache.remove(&oldest);
        }
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
        let mut cache = self.projection_cache.write().await;
        let before = cache.len();
        cache.retain(|key, _| key.installation_id != installation_id);
        let removed = before - cache.len();
        drop(cache);
        let mut bands = self.bands_cache.write().await;
        let bands_before = bands.len();
        bands.retain(|key, _| key.installation_id != installation_id);
        let bands_removed = bands_before - bands.len();
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
        let mut cache = self.projection_cache.write().await;
        let before = cache.len();
        cache.retain(|key, _| key.owner_user_id != Some(user_id));
        let removed = before - cache.len();
        drop(cache);
        // Mismo criterio para las bandas: son del usuario por construcción (`view=mine`).
        let mut bands = self.bands_cache.write().await;
        let bands_before = bands.len();
        bands.retain(|key, _| key.user_id != user_id);
        let bands_removed = bands_before - bands.len();
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
