//! Techo de concurrencia para el trabajo CPU-bound caro: el KDF de contraseñas (Argon2id), el
//! cripto de `.ffbackup` y las simulaciones de proyección.
//!
//! ## Por qué existe
//!
//! Mover Argon2id a `spawn_blocking` arregló un problema y creó otro peor, y las dos mitades
//! importan:
//!
//! - **Lo que arregló**: con el KDF corriendo inline en un worker del reactor, `num_cpus`
//!   peticiones concurrentes a `/v1/auth/register` —endpoint sin autenticación por diseño—
//!   paraban el proceso entero, `GET /v1/ready` incluido.
//! - **Lo que creó**: el pool de blocking de Tokio tiene **512 hilos** por defecto y una cola
//!   de espera ilimitada. El techo de Argon2 concurrente pasó de `num_cpus` a 512, y cada hash
//!   reserva 19 MiB: de ~38 MiB en un VPS de 2 vCPU a ~9,5 GiB. El fallo dejaba de ser «la API
//!   no responde y el healthcheck la reinicia» para pasar a ser **OOM-kill del contenedor con
//!   el PostgreSQL embebido dentro** — SIGKILL al postmaster a mitad de checkpoint, que es
//!   justo lo que el apagado ordenado existe para evitar.
//!
//! Peor aún: el healthcheck no lo habría notado. `sqlx` no usa el pool de blocking, así que
//! `/v1/ready` sigue devolviendo 200 mientras login, registro y backup están muertos. El fallo
//! pasaba de ruidoso y auto-recuperable a silencioso.
//!
//! ## Por qué un semáforo y no un `try_acquire` con 503
//!
//! La memoria solo se reserva **mientras** se hashea. Un permiso pendiente es un future
//! esperando: cuesta bytes, no megabytes. Así que esperar acota el pico de memoria sin
//! rechazar tráfico legítimo: bajo carga las peticiones se ponen lentas, que es la degradación
//! correcta, en vez de fallar o tumbar el contenedor.

use crate::error::ApiError;
use std::sync::OnceLock;
use tokio::sync::Semaphore;

/// Permisos para el KDF de contraseñas (login, registro, verificación del export).
///
/// Uno por CPU: es exactamente el techo que había antes de `spawn_blocking`, que era seguro en
/// memoria. Lo que cambia respecto a entonces es que ahora se espera **fuera** del reactor, así
/// que el resto de la API sigue respondiendo.
fn password_permits() -> &'static Semaphore {
    static S: OnceLock<Semaphore> = OnceLock::new();
    S.get_or_init(|| {
        let n = std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(2)
            .clamp(2, 8);
        Semaphore::const_new(n)
    })
}

/// Permisos para el cifrado/descifrado de `.ffbackup`. **Uno solo**, a propósito.
///
/// Una sola petición de import puede pedir, sumando, varios cientos de MiB: el KDF con los
/// parámetros máximos que se admiten, los 16 MiB del cuerpo en base64, los bytes decodificados,
/// el texto en claro tras descomprimir y el payload deserializado. Los topes de `crypto.rs`
/// acotan **una** petición; sin esto, veinte concurrentes seguían agotando un contenedor de
/// 2 GiB desde el endpoint de *preview*, que ni siquiera escribe. Exportar e importar una copia
/// de seguridad es una operación rara: serializarla no le cuesta nada a nadie.
fn backup_permits() -> &'static Semaphore {
    static S: OnceLock<Semaphore> = OnceLock::new();
    S.get_or_init(|| Semaphore::const_new(1))
}

/// Permisos para las **simulaciones de proyección** (`project_net_worth_series` y el marker de
/// «compound supera ahorro»), el tercer trabajo CPU-bound del binario.
///
/// ## El agujero que tapa
///
/// `heavy.rs` acotaba el KDF y el cripto del backup, pero las proyecciones lanzaban
/// `spawn_blocking` a pelo — dos por petición, en `tokio::join!`. Sin techo, el límite efectivo
/// volvía a ser el pool de blocking de Tokio (512 hilos) y el fallo es peor que el de Argon2
/// porque **no hace falta un atacante**: un agente MCP en bucle emitiendo `simulate_projection`
/// —o cualquier cliente pidiendo `GET /v1/projection/series?months=…`, que **salta la cache por
/// diseño** (D7)— basta para poner N simulaciones en vuelo. Cada una es CPU pura durante cientos
/// de milisegundos, así que a partir de unas pocas los hilos de blocking se comen los núcleos que
/// necesitan los workers del reactor: los `/v1` normales empiezan a agotar el `acquire_timeout`
/// de 5 s del pool de 10 conexiones y devuelven 500. Y como `/v1/ready` usa **ese mismo pool**,
/// el healthcheck falla y el contenedor —con el PostgreSQL embebido dentro— se reinicia. La
/// diferencia con Argon2: allí el síntoma era memoria (OOM), aquí es CPU, pero el destino es el
/// mismo contenedor reiniciándose a mitad de checkpoint.
///
/// ## Por qué ESTE número
///
/// `available_parallelism()` acotado a `[2, 8]`, igual que el KDF, porque el recurso escaso es el
/// mismo (núcleos) y el trabajo es igual de indivisible. Las dos cotas son deliberadas:
///
/// - **Suelo 2, nunca 1**: una petición de proyección usa DOS permisos (serie principal + marker;
///   baseline + escenario en el what-if) y los suelta por separado, cuando termina cada tarea. Con
///   un solo permiso no habría deadlock —cada permiso se libera al acabar SU tarea, y el semáforo
///   de tokio es FIFO—, pero se perdería el paralelismo intra-petición que el `tokio::join!`
///   existe para explotar: una proyección tardaría el doble incluso en una máquina ociosa.
/// - **Techo 8**: por encima, más simulaciones concurrentes no terminan antes ninguna (son CPU
///   pura), solo le quitan núcleos al reactor. Dejar la mitad de una máquina grande fuera del
///   techo es exactamente lo que mantiene vivo `/v1/ready` bajo carga.
///
/// Lo que **no** se serializa: la proyección **cacheada**. El permiso se pide alrededor de la
/// simulación, no del handler, así que un HIT de `projection_series_cached` no toca el semáforo
/// (regresión: `projection_concurrency.rs`).
fn projection_permits() -> &'static Semaphore {
    static S: OnceLock<Semaphore> = OnceLock::new();
    S.get_or_init(|| {
        let n = std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(2)
            .clamp(2, 8);
        Semaphore::const_new(n)
    })
}

async fn run_with<T, F>(sem: &'static Semaphore, f: F) -> Result<T, ApiError>
where
    F: FnOnce() -> T + Send + 'static,
    T: Send + 'static,
{
    let _permit = sem.acquire().await.map_err(|_| ApiError::Unavailable)?;
    tokio::task::spawn_blocking(f)
        .await
        .map_err(|_| ApiError::Unavailable)
}

/// Ejecuta un KDF de contraseña fuera del reactor y bajo el techo de concurrencia.
pub async fn run_password_kdf<T, F>(f: F) -> Result<T, ApiError>
where
    F: FnOnce() -> T + Send + 'static,
    T: Send + 'static,
{
    run_with(password_permits(), f).await
}

/// Ejecuta el cifrado/descifrado de un `.ffbackup` fuera del reactor, de uno en uno.
pub async fn run_backup_crypto<T, F>(f: F) -> Result<T, ApiError>
where
    F: FnOnce() -> T + Send + 'static,
    T: Send + 'static,
{
    run_with(backup_permits(), f).await
}

/// Ejecuta una simulación de proyección fuera del reactor y bajo el techo de concurrencia.
///
/// A diferencia de `run_password_kdf` / `run_backup_crypto` **no** convierte el pánico de la tarea
/// en `Unavailable`: los dos llamantes ya publican el código estable `task_panic`, que vive en el
/// catálogo de errores (`tests/fixtures/error-codes.json`) y tiene frase en español en la SPA.
/// Cambiarlo aquí retiraría un código publicado a cambio de nada. `label` solo compone el detalle
/// técnico del mensaje; el prefijo `task_panic:` es un literal completo, como exige
/// `error_codes_parity`.
pub async fn run_projection_sim<T, F>(label: &str, f: F) -> Result<T, ApiError>
where
    F: FnOnce() -> T + Send + 'static,
    T: Send + 'static,
{
    let _permit = projection_permits()
        .acquire()
        .await
        .map_err(|_| ApiError::Unavailable)?;
    tokio::task::spawn_blocking(f)
        .await
        .map_err(|e| ApiError::BadRequest(format!("task_panic: {label} task panic: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn el_techo_de_contrasenas_no_es_el_pool_entero_de_tokio() {
        let n = password_permits().available_permits();
        assert!(
            (2..=8).contains(&n),
            "el techo debe seguir siendo del orden del nº de CPU, no los 512 hilos del pool: {n}"
        );
    }

    #[test]
    fn el_backup_va_de_uno_en_uno() {
        assert_eq!(backup_permits().available_permits(), 1);
    }

    /// El techo de proyección existe, es del orden del nº de CPU y **nunca es 1**: una petición
    /// consume dos permisos (serie + marker, o baseline + escenario) y con uno solo perdería el
    /// paralelismo intra-petición del `tokio::join!`.
    #[test]
    fn el_techo_de_proyeccion_permite_el_paralelismo_intra_peticion() {
        let n = projection_permits().available_permits();
        assert!(
            (2..=8).contains(&n),
            "el techo debe ser del orden del nº de CPU y >= 2 (dos simulaciones por petición): {n}"
        );
    }
}
