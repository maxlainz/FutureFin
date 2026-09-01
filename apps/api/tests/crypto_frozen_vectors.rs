//! Cinturón de vectores CONGELADOS del stack criptográfico: un `.ffbackup` y un hash PHC
//! fabricados con las versiones de HOY de `aes-gcm`, `argon2` y `flate2`, que cualquier versión
//! futura de esas cajas tiene que seguir entendiendo.
//!
//! ## Por qué existe
//!
//! Un `cargo test` en verde tras subir `aes-gcm` o `argon2` prueba **una sola cosa**: que la
//! versión nueva se entiende consigo misma. Cifra y descifra en el mismo proceso, hashea y
//! verifica en el mismo proceso. Los dos formatos que este binario PERSISTE fuera de sí mismo no
//! los mira nadie:
//!
//! - el **`.ffbackup`** que un usuario tiene guardado en su disco desde hace meses — la única vía
//!   de recuperación que tiene, y que se importa contra el servidor de mañana;
//! - el **hash PHC** de `users.password_hash`, escrito el día del registro y leído en cada login
//!   durante años.
//!
//! Si un bump cambia el layout del nonce/tag de AES-GCM, la codificación del salt, el orden de
//! los bytes del KDF, los parámetros por defecto de `Argon2::default()` o el encabezado gzip, el
//! fallo NO aparece en un round-trip: aparece en producción, después del upgrade, como «tu copia
//! de seguridad no se puede importar» y «tu contraseña ya no es correcta». Y no hay marcha atrás:
//! el hash está en la base de datos y el fichero está en el disco del usuario.
//!
//! Este fichero es la evidencia que faltaba para poder mergear un bump de esas cajas.
//!
//! ## Qué congela exactamente el fixture binario
//!
//! `tests/fixtures/frozen-vector-v12.ffbackup` es una tubería entera, no solo AES:
//!
//! 1. el **framing** propio (`FFBK` + `format_version` + `manifest_len` LE + manifiesto JSON);
//! 2. el **KDF Argon2id** de `crypto.rs` (m=19456, t=2, p=1, salida 32 B, `Version::V0x13`),
//!    derivando la clave del salt del manifiesto y de los BYTES UTF-8 de la contraseña;
//! 3. **AES-256-GCM** con AAD (`ffbk:v{schema}:{user_id}:{exported_at}`), que autentica el
//!    manifiesto parcialmente y hace que cualquier deriva de un byte falle en cerrado;
//! 4. **gzip (`flate2`)**: el texto en claro va comprimido DENTRO del sobre cifrado, así que este
//!    fixture protege también los bumps de `flate2` — si un `GzDecoder` futuro dejara de leer
//!    este stream, el usuario no podría importar su backup. Es deliberado que el mismo vector
//!    cubra las tres cajas: en el fichero real van las tres en serie.
//!
//! ## Qué NO congela
//!
//! No es un test de la ruta HTTP: no toca base de datos, no pasa por `/v1/backup/user-import` ni
//! por `/v1/auth/login`, y no valida la lógica de import (índices, re-link, rollback) — de eso ya
//! se ocupa `backup_user_roundtrip.rs`. Tampoco es un test de seguridad del KDF: no dice nada
//! sobre si los parámetros son suficientes hoy, solo sobre si siguen siendo los MISMOS.
//!
//! ## Cómo se regeneran los vectores (y por qué casi nunca se debe)
//!
//! Los dos generadores viven abajo, marcados `#[ignore]`. **Regenerarlos destruye el valor del
//! cinturón**: un vector generado con la caja nueva vuelve a probar solo que la caja nueva se
//! entiende consigo misma, que es exactamente el agujero que esto viene a tapar. Se regeneran
//! únicamente cuando el FORMATO cambia a propósito y con la nota de por qué; en ese caso el
//! vector viejo se queda además como caso de compatibilidad hacia atrás.
//!
//! El fichero no es reproducible bit a bit (salt y nonce son aleatorios por diseño): cada
//! ejecución del generador produce un `.ffbackup` distinto y igual de válido. Lo que se congela
//! es el que está en el repositorio.
//!
//! ```bash
//! # NO ejecutar salvo cambio deliberado de formato (ver arriba).
//! cargo test -p futurefin-api --test crypto_frozen_vectors -- --ignored --nocapture
//! ```
//!
//! ## Higiene de datos
//!
//! El payload es ÍNTEGRAMENTE fabricado (`futurefin-data-hygiene`): usuaria inventada, importes
//! inventados y coherentes, ninguna cifra procedente de una instalación real. La contraseña es
//! fija y está a la vista en el código a propósito — es el único modo de que el vector sea
//! verificable, y no protege nada.
//!
//! OJO con la asimetría: el fixture es un BINARIO cifrado, y `scripts/scan-sensitive.sh` salta los
//! binarios (`grep -I`), así que **ese gate no puede auditar su contenido** — pasa por omisión, no
//! por inspección. Lo que lo hace auditable es que el claro NO vive solo dentro del cifrado: el
//! payload entero está escrito en texto plano en el generador de este mismo fichero, que sí es
//! trackeado y sí es escaneado. Si alguna vez se regenera el vector con otro contenido, ese
//! contenido tiene que seguir estando aquí a la vista.
//!
//! No requiere Postgres: es un test puro sobre `futurefin_api::handlers::backup_user::crypto` y
//! `futurefin_api::auth::password`.

use futurefin_api::handlers::backup_user::crypto::{
    decrypt_payload, parse_frame, CryptoError,
};
use futurefin_api::handlers::backup_user::schema::{
    migrate_to_current, parse_payload, CURRENT_SCHEMA_VERSION,
};

/// El `.ffbackup` dorado, embebido en el binario de test para que el vector viaje con él.
///
/// Generado el 2026-09-01 con `futurefin-api` 4.12.2 y, sobre todo, con
/// **aes-gcm 0.10.3 · argon2 0.5.3 · flate2 1.1.9** — las versiones ANTERIORES a los bumps que
/// este cinturón existe para poder mergear (#160 aes-gcm 0.11.1, #162 argon2 0.6.0, #174
/// flate2 1.1.10).
const GOLDEN_FFBACKUP: &[u8] = include_bytes!("fixtures/frozen-vector-v12.ffbackup");

/// La contraseña del vector. Fija, visible y sin ningún valor: protege datos inventados.
///
/// Lleva una `ñ` A PROPÓSITO. El KDF hashea los BYTES UTF-8 de la contraseña, no sus caracteres:
/// si un bump cambiara esa codificación, una contraseña ASCII no lo notaría y media Europa se
/// quedaría fuera de su cuenta.
const GOLDEN_PASSWORD: &str = "contraseña-de-prueba-congelada";

/// Hash PHC de [`GOLDEN_PASSWORD`] producido por la ruta REAL de registro
/// (`auth::password::hash_password_blocking`, la que llama `POST /v1/auth/register`) con los
/// parámetros de producción de `Argon2::default()` en argon2 0.5.3.
///
/// Es un string PHC autodescriptivo: lleva dentro algoritmo, versión, parámetros y salt, así que
/// congelarlo congela las cinco cosas a la vez.
const GOLDEN_PASSWORD_HASH: &str = "$argon2id$v=19$m=19456,t=2,p=1$LbE14aRvVmyCb0RJh+s6Xg$zFUTjm9lkc1VGImSrByvYwQ5qhqFuFE7RlPf3yIq+eI";

/// Prefijo del PHC hasta el salt: algoritmo, versión del formato y los tres parámetros de coste.
/// Se compara aparte porque responde a una pregunta distinta que la verificación (ver el test).
const GOLDEN_PHC_PARAMS_PREFIX: &str = "$argon2id$v=19$m=19456,t=2,p=1$";

// ---------------------------------------------------------------------------
// (a) El `.ffbackup` que el usuario tiene en el disco sigue descifrándose
// ---------------------------------------------------------------------------

/// **Si este test falla, un usuario que actualice FutureFin no puede importar la copia de
/// seguridad que exportó con la versión anterior** — el único camino de vuelta que tiene si
/// perdió sus datos. El fallo se manifiesta como `backup_wrong_password` o
/// `backup_file_corrupt` sobre un fichero que está perfectamente bien, y la causa está en el
/// bump de `aes-gcm`, `argon2` o `flate2`, no en el fichero.
///
/// Reproduce paso a paso lo que hace `import::decode_request_blocking`: `parse_frame` →
/// `decrypt_payload` (Argon2id + AES-256-GCM + gunzip) → `parse_payload` → `migrate_to_current`.
#[test]
fn el_ffbackup_dorado_sigue_descifrandose_y_parseando() {
    let parsed = parse_frame(GOLDEN_FFBACKUP).expect("el framing FFBK del vector dorado");

    // El manifiesto viaja EN CLARO: congelarlo detecta una deriva de parámetros aunque el
    // descifrado siguiera funcionando por casualidad.
    assert_eq!(parsed.manifest.schema_version, 12, "schema_version del vector");
    assert_eq!(parsed.manifest.kdf.alg, "argon2id");
    assert_eq!(parsed.manifest.kdf.m_cost, 19_456);
    assert_eq!(parsed.manifest.kdf.t_cost, 2);
    assert_eq!(parsed.manifest.kdf.p_cost, 1);
    assert_eq!(parsed.manifest.kdf.out_len, 32);
    assert_eq!(parsed.manifest.cipher.alg, "aes-256-gcm");
    // gzip: el aspecto que protege los bumps de flate2 (#174). El claro va comprimido DENTRO
    // del sobre cifrado, así que si `GzDecoder` dejara de leer este stream el import muere.
    assert_eq!(parsed.manifest.compression, "gzip");

    // El servidor de hoy sigue siendo capaz de leer esta versión de payload: la cadena de
    // migración v1..N es perpetua (change-control §5).
    assert!(
        parsed.manifest.schema_version <= CURRENT_SCHEMA_VERSION,
        "el vector dorado (v{}) no puede ser más nuevo que el servidor (v{CURRENT_SCHEMA_VERSION})",
        parsed.manifest.schema_version,
    );

    let plain = decrypt_payload(&parsed, GOLDEN_PASSWORD)
        .expect("descifrar el vector dorado con su contraseña fija");

    let any = parse_payload(parsed.manifest.schema_version, &plain)
        .expect("el payload descifrado sigue siendo JSON válido para su schema_version");
    let payload = migrate_to_current(any);

    // Campos conocidos: si el descifrado devolviera basura que por milagro pasara el AEAD, esto
    // lo caza. Todos los valores son fabricados (ver la cabecera).
    assert_eq!(payload.user.username, "usuaria-ficticia");
    assert_eq!(payload.assets.len(), 1);
    assert_eq!(payload.assets[0].name, "Cuenta de prueba");
    // `.to_string()` y no `==` sobre Decimal: congela también la escala serializada.
    assert_eq!(payload.assets[0].current_value.to_string(), "12345.67");
    assert_eq!(payload.budget_entries.len(), 1);
    assert_eq!(payload.budget_entries[0].amount.to_string(), "850.00");
    assert_eq!(
        payload
            .installation_snapshot_informative
            .fire_settings
            .swr_pct
            .to_string(),
        "3.5"
    );
}

/// Control negativo del test de arriba: sin él, un `decrypt_payload` que devolviera el claro
/// pase lo que pase haría pasar el cinturón entero sin probar nada.
///
/// **Si este test falla, el AEAD ha dejado de autenticar**: se estaría aceptando un fichero con
/// una contraseña que no es la suya, que es peor que no poder importarlo.
#[test]
fn el_ffbackup_dorado_no_se_descifra_con_otra_contrasena() {
    let parsed = parse_frame(GOLDEN_FFBACKUP).expect("el framing FFBK del vector dorado");
    let err = decrypt_payload(&parsed, "contraseña-que-no-es-la-suya")
        .expect_err("una contraseña incorrecta no puede descifrar el vector");
    assert!(
        matches!(err, CryptoError::Decrypt),
        "se esperaba CryptoError::Decrypt, llegó {err:?}"
    );
}

// ---------------------------------------------------------------------------
// (b) El hash PHC ya guardado en `users` sigue verificando
// ---------------------------------------------------------------------------

/// **Si este test falla, NADIE puede iniciar sesión después del upgrade.** Los hashes de
/// `users.password_hash` se escribieron el día del registro y no se pueden recalcular: sin la
/// contraseña en claro no hay migración posible, así que un cambio de formato aquí deja la
/// instalación cerrada por fuera y sin arreglo por delante.
///
/// Usa la ruta de verificación REAL —`auth::password::verify_password_blocking`, la misma que
/// llama `POST /v1/auth/login`—, con su semáforo y su `spawn_blocking`, no un `Argon2` montado
/// a mano en el test.
#[tokio::test]
async fn el_hash_phc_dorado_sigue_verificando_por_la_ruta_de_login() {
    futurefin_api::auth::password::verify_password_blocking(
        GOLDEN_PASSWORD,
        Some(GOLDEN_PASSWORD_HASH.to_string()),
    )
    .await
    .expect("el hash PHC dorado tiene que seguir verificando con su contraseña");
}

/// Control negativo del anterior: una verificación que dijera «sí» a todo también pasaría el
/// test de arriba. **Si este falla, cualquier contraseña abre cualquier cuenta.**
#[tokio::test]
async fn el_hash_phc_dorado_rechaza_otra_contrasena() {
    let out = futurefin_api::auth::password::verify_password_blocking(
        "contraseña-que-no-es-la-suya",
        Some(GOLDEN_PASSWORD_HASH.to_string()),
    )
    .await;
    assert!(out.is_err(), "una contraseña incorrecta no puede verificar");
}

/// Pregunta DISTINTA de la anterior: los hashes viejos siguen verificando **aunque el bump haya
/// cambiado `Params::DEFAULT`** (verificar usa los parámetros que vienen escritos en el propio
/// PHC, no los del proceso). Lo que un cambio de defaults sí mueve es lo que se escribe HOY al
/// registrarse.
///
/// **Si este test falla, `Argon2::default()` ya no es lo que era**: los registros nuevos pasan a
/// otro coste sin que nadie lo haya decidido. No rompe el login de nadie —por eso es un test
/// aparte y no un `assert` dentro del anterior—, pero es un cambio de postura de seguridad y de
/// consumo de memoria por login (`heavy.rs` dimensiona su semáforo contando 19 MiB por hash) que
/// tiene que ser deliberado.
#[tokio::test]
async fn el_registro_de_hoy_conserva_los_parametros_del_hash_dorado() {
    let recien_hecho = futurefin_api::auth::password::hash_password_blocking(GOLDEN_PASSWORD)
        .await
        .expect("hashear por la ruta real de registro");

    assert!(
        GOLDEN_PASSWORD_HASH.starts_with(GOLDEN_PHC_PARAMS_PREFIX),
        "el vector dorado dejó de casar con su propio prefijo declarado"
    );
    assert!(
        recien_hecho.starts_with(GOLDEN_PHC_PARAMS_PREFIX),
        "un registro de hoy produce {recien_hecho}, que no empieza por \
         {GOLDEN_PHC_PARAMS_PREFIX} — Argon2::default() ha cambiado de parámetros"
    );
    // El salt es aleatorio por registro: dos hashes de la MISMA contraseña no pueden coincidir.
    assert_ne!(
        recien_hecho, GOLDEN_PASSWORD_HASH,
        "el salt dejó de ser aleatorio: dos registros con la misma contraseña dan el mismo hash"
    );
}

// ---------------------------------------------------------------------------
// Generadores — `#[ignore]`. Leer la cabecera del fichero ANTES de ejecutarlos.
// ---------------------------------------------------------------------------

/// Regenera `tests/fixtures/frozen-vector-v12.ffbackup`.
///
/// **No lo ejecutes para «arreglar» un test rojo.** Un vector regenerado con la caja nueva vuelve
/// a probar solo que la caja nueva se entiende consigo misma; si el cinturón se pone rojo tras un
/// bump, el rojo ES el resultado y hay que investigar el bump, no rehacer el vector.
///
/// Cifra con `encrypt_payload` + `frame_file`, exactamente las mismas funciones que usa
/// `POST /v1/backup/user-export` (`export.rs`), con `app_version` y `exported_at` fijos para que
/// el fichero no cambie de forma por el reloj. El payload es un v12 mínimo y ENTERAMENTE
/// fabricado.
///
/// ```bash
/// cargo test -p futurefin-api --test crypto_frozen_vectors -- --ignored --nocapture
/// ```
#[test]
#[ignore = "generador de fixture: reescribe el vector dorado, ver la cabecera del fichero"]
fn generar_fixture_ffbackup_dorado() {
    use futurefin_api::handlers::backup_user::crypto::{encrypt_payload, frame_file};

    // UUID fabricado (no procede de ninguna instalación) y fecha fija: los dos entran en el AAD.
    let user_id = "11111111-2222-4333-8444-555555555555";
    let exported_at = "2026-09-01T10:15:30.000000000+00:00";

    let payload = serde_json::json!({
        "user": { "username": "usuaria-ficticia", "birth_date": "1988-03-14" },
        "categories_used": [
            { "scope": "asset",   "name": "Cuenta corriente", "sort_index": 0 },
            { "scope": "expense", "name": "Vivienda",         "sort_index": 0 }
        ],
        "assets": [{
            "category_ref": { "scope": "asset", "name": "Cuenta corriente" },
            "name": "Cuenta de prueba",
            "current_value": "12345.67",
            "purchase_price": null,
            "is_liquid": true,
            "expected_annual_return_percent": "1.25",
            "notes": null,
            "sort_index": 0
        }],
        "allocation_rules": [],
        "liabilities": [],
        "budget_entries": [{
            "category_ref": { "scope": "expense", "name": "Vivienda" },
            "amount": "850.00",
            "persists_after_retirement": true,
            "ends_at_retirement": false,
            "expense_end_date": null,
            "notes": null,
            "sort_index": 0
        }],
        "planning_flows": [],
        "ui_preferences": { "person_scope": "household", "projection_focus": null },
        "installation_snapshot_informative": {
            "base_currency": "EUR",
            "calendar_tz": "Europe/Madrid",
            "annual_inflation_assumption_percent": "2.5",
            "show_age_mode": "dates",
            "fire_settings": {
                "fire_number_mode": "annual_expense",
                "fire_number_manual_amount": null,
                "swr_pct": "3.5",
                "taxes_enabled": true,
                "tax_brackets": [
                    { "up_to": "6000", "pct": "19" },
                    { "up_to": null,   "pct": "21" }
                ],
                "savings_source": "budget",
                "income_avg_window_months": 12,
                "income_avg_window_mode": "calendar",
                "expense_avg_window_months": 12,
                "expense_avg_window_mode": "calendar",
                "taxable_gain_ratio": "1",
                "horizon_lifespan_age": 90
            }
        },
        "snapshots": [],
        "transaction_imports": [],
        "transactions": [],
        "categorization_rules": [],
        "recurring_transaction_rules": [],
        "transfer_match_rejections": []
    });

    let plaintext = serde_json::to_vec(&payload).expect("serializar el payload fabricado");

    // Se comprueba ANTES de cifrar que el payload es un v12 legítimo: un vector que descifra
    // pero no parsea no protege nada.
    let any = parse_payload(12, &plaintext).expect("el payload fabricado tiene que ser un v12 válido");
    let _ = migrate_to_current(any);

    let enc = encrypt_payload(
        &plaintext,
        GOLDEN_PASSWORD,
        "4.12.2",
        12,
        user_id,
        "usuaria-ficticia",
        exported_at,
    )
    .expect("cifrar el vector dorado");
    let framed = frame_file(&enc.manifest, &enc.ciphertext).expect("enmarcar el vector dorado");

    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/frozen-vector-v12.ffbackup"
    );
    std::fs::write(path, &framed).expect("escribir el fixture");
    println!("fixture escrito en {path} ({} bytes)", framed.len());
}

/// Regenera el valor de [`GOLDEN_PASSWORD_HASH`]: lo imprime para copiarlo A MANO a la constante.
///
/// No se escribe solo a propósito — un vector congelado que se reescribe solo deja de estar
/// congelado. Pasa por `hash_password_blocking`, la ruta REAL de registro, así que hereda la
/// validación de fortaleza, el `Argon2::default()` de producción y el salt aleatorio.
///
/// ```bash
/// cargo test -p futurefin-api --test crypto_frozen_vectors -- --ignored --nocapture
/// ```
#[tokio::test]
#[ignore = "generador de vector: imprime un hash PHC nuevo, ver la cabecera del fichero"]
async fn generar_hash_phc_dorado() {
    let phc = futurefin_api::auth::password::hash_password_blocking(GOLDEN_PASSWORD)
        .await
        .expect("hashear por la ruta real de registro");
    println!("GOLDEN_PASSWORD_HASH = {phc}");
}
