//! **El reparto de los caminos entre núcleos** (E12, 5.0.0) — y la única razón por la que se puede
//! hacer sin mover un dígito.
//!
//! # Por qué esto NO cambia ningún número
//!
//! El sorteo de un camino depende de `(seed, path_index)` y de nada más
//! ([`crate::mc::path_rng`]), y el motor es una función pura sin estado global. Dos caminos no se
//! ven; el orden en que se ejecutan no existe como variable del modelo. Lo único que sí importa —y
//! lo que este módulo NO decide— es el orden en que sus resultados se **pliegan**: una suma de
//! `f64` no es asociativa, así que sumar los caminos en el orden en que van terminando daría un
//! número distinto cada vez.
//!
//! La disciplina, en una frase: **rayon decide QUIÉN ejecuta cada camino; el índice del camino
//! decide DÓNDE cae su resultado.** `crate::mc::for_each_path` entrega los resultados al hilo
//! llamante en orden de índice (`p = 0, 1, 2, …`); toda reducción posterior (sumas, conteos,
//! ordenaciones, percentiles) se hace secuencialmente sobre ellos, exactamente como antes. Los conteos son enteros
//! y los percentiles son un índice entero sobre una muestra ordenada con un orden total, así que
//! ninguno de los dos podría moverse aunque quisiera; las únicas sumas de `f64` del pliegue
//! —cobertura y `months_below`— se hacen dentro de UN camino, nunca entre caminos.
//!
//! Guardián: `tests/parallel_determinism.rs`, que compara **todas** las salidas con `==` exacto
//! (`f64::to_bits`) entre 1, 2, 4 y 8 hilos.
//!
//! # El pool: uno solo, acotado, compartido
//!
//! El pool por defecto **no** es el global de rayon y **no** se dimensiona por petición: es un
//! [`rayon::ThreadPool`] propio de este crate, creado una sola vez y compartido por todas las
//! simulaciones en vuelo. Esa es la propiedad que la API necesita, y conviene decirla entera
//! porque es la parte que se rompería sola:
//!
//! `apps/api/src/heavy.rs` acota las simulaciones concurrentes con un semáforo de
//! `available_parallelism()` permisos (`[2, 8]`). Si cada simulación abriera su propio pool de
//! `N` hilos, el techo real de CPU pasaría a ser **permisos × N** —hasta 64 hilos en una máquina
//! de 8 núcleos— y el semáforo dejaría de proteger lo que existe para proteger: los workers del
//! reactor, y con ellos `/v1/ready` y el contenedor entero. Con un pool compartido, `M` peticiones
//! concurrentes reparten los MISMOS [`pool_threads`] hilos: cada una va más lenta, ninguna roba
//! núcleos de más, y el total de CPU dedicada a Monte Carlo está acotado por construcción.
//!
//! No hay riesgo de bloqueo mutuo: [`rayon::ThreadPool::install`] llamado desde fuera del pool
//! bloquea el hilo llamante (que ya está fuera del reactor, dentro de un `spawn_blocking`) y el
//! trabajo lo ejecutan los workers; si todos están ocupados, el `join` interno de rayon ejecuta
//! las dos mitades en el mismo worker en vez de esperar a que alguien las robe. Degrada a
//! secuencial, nunca a parado.
//!
//! # Apagarlo
//!
//! [`crate::McConfig::threads`] `= Some(1)` recorre los caminos en el hilo llamante, sin tocar
//! rayon ni el pool. Es el modo con el que la batería de determinismo compara, y el que hay que
//! usar para medir «antes» y «después» sin reconstruir nada.

use std::sync::OnceLock;

use rayon::ThreadPool;

use crate::McConfig;

/// **Techo de hilos del pool compartido.**
///
/// El mismo `8` que el techo del semáforo de `heavy.rs`, y por la misma razón: por encima, más
/// hilos no terminan antes ninguna simulación —son CPU pura— y solo le quitan núcleos al reactor.
/// Dejar fuera del techo la mitad de una máquina grande es exactamente lo que mantiene vivo
/// `/v1/ready` bajo carga.
pub const MAX_POOL_THREADS: usize = 8;

/// **Hilos del pool compartido**: `available_parallelism()` acotado a `[1, MAX_POOL_THREADS]`.
///
/// El suelo es `1`, no `2`: a diferencia del semáforo de la API —donde una petición consume dos
/// permisos y con uno solo perdería el paralelismo intra-petición— aquí un solo hilo es
/// simplemente el modo secuencial, que es correcto, no un bloqueo.
///
/// Se resuelve **una sola vez** y se recuerda: `install` lo consulta una vez por bloque de
/// reparto, y `available_parallelism()` es una llamada al sistema. Que además sea estable durante
/// la vida del proceso es lo que hace que «el pool compartido» sea uno y no dos.
pub fn pool_threads() -> usize {
    static N: OnceLock<usize> = OnceLock::new();
    *N.get_or_init(|| {
        std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(1)
            .clamp(1, MAX_POOL_THREADS)
    })
}

/// El pool compartido. `None` si rayon no pudo crearlo (sin hilos disponibles en el sistema): en
/// ese caso todo el crate cae a secuencial en vez de panicar, porque una banda lenta es un
/// resultado y un pánico no.
fn shared_pool() -> Option<&'static ThreadPool> {
    static POOL: OnceLock<Option<ThreadPool>> = OnceLock::new();
    POOL.get_or_init(|| {
        rayon::ThreadPoolBuilder::new()
            .num_threads(pool_threads())
            .thread_name(|i| format!("ff-mc-{i}"))
            .build()
            .ok()
    })
    .as_ref()
}

/// Cuántos hilos usa esta ejecución: [`McConfig::threads`] si lo pide, [`pool_threads`] si no.
///
/// Se acota a `[1, MAX_POOL_THREADS]` **también cuando lo pide el llamante**: el campo es una
/// palanca de medición y de tests, no una vía para abrir doscientos hilos desde un query param.
pub(crate) fn resolve_threads(config: &McConfig) -> usize {
    match config.threads {
        Some(n) => n.clamp(1, MAX_POOL_THREADS),
        None => pool_threads(),
    }
}

/// Ejecuta `f` con el paralelismo pedido.
///
/// - `threads == 1` ⇒ se llama directamente, sin tocar rayon.
/// - `threads == pool_threads()` (el caso de producción) ⇒ el pool COMPARTIDO.
/// - cualquier otro valor ⇒ un pool efímero de ese tamaño, que es lo que la batería de tests y el
///   arnés de tiempos necesitan para barrer 1/2/4/8 hilos. Construirlo cuesta un `spawn` por hilo
///   (decenas de µs) frente a los cientos de ms de la ejecución que lo usa, y **nunca ocurre en el
///   camino de la API**, que no rellena `McConfig::threads`.
pub(crate) fn install<R: Send>(threads: usize, f: impl FnOnce() -> R + Send) -> R {
    if threads <= 1 {
        return f();
    }
    if threads == pool_threads() {
        if let Some(pool) = shared_pool() {
            return pool.install(f);
        }
    }
    match rayon::ThreadPoolBuilder::new()
        .num_threads(threads)
        .thread_name(|i| format!("ff-mc-adhoc-{i}"))
        .build()
    {
        Ok(pool) => pool.install(f),
        // Sin pool no hay paralelismo, pero sí resultado — y es el MISMO resultado.
        Err(_) => f(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn el_pool_nunca_es_mas_grande_que_el_techo_ni_mas_pequeno_que_uno() {
        let n = pool_threads();
        assert!(
            (1..=MAX_POOL_THREADS).contains(&n),
            "el pool compartido debe estar acotado a [1, {MAX_POOL_THREADS}]: {n}"
        );
    }

    #[test]
    fn los_hilos_pedidos_se_acotan_al_mismo_techo() {
        let cfg = |t| McConfig {
            threads: t,
            ..Default::default()
        };
        assert_eq!(resolve_threads(&cfg(Some(0))), 1);
        assert_eq!(resolve_threads(&cfg(Some(1))), 1);
        assert_eq!(resolve_threads(&cfg(Some(4))), 4.min(MAX_POOL_THREADS));
        assert_eq!(resolve_threads(&cfg(Some(10_000))), MAX_POOL_THREADS);
        assert_eq!(resolve_threads(&cfg(None)), pool_threads());
    }

    /// `install` es transparente: devuelve lo que devuelve `f`, con uno o con muchos hilos.
    #[test]
    fn install_devuelve_lo_mismo_con_cualquier_numero_de_hilos() {
        for t in [1usize, 2, 4, 8, 64] {
            assert_eq!(install(t, || 40 + 2), 42);
        }
    }
}
