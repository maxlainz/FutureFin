//! `GET|PATCH /v1/auth/me/retirement-profile` — el perfil de jubilación POR USUARIO (5.0.0,
//! issue #207, decisión D13) en su forma **v2** (decisiones M2/M4/M5/M6/C3/C5/C7 del owner).
//!
//! Lo que se fija aquí, y por qué cada cosa:
//!
//! * **Defaults**: un usuario que nunca lo ha tocado tiene `retirement_profile IS NULL` y el
//!   servidor devuelve el plan por defecto (`asap`, SWR 3,5, edad límite 90, umbral 95). Si esto
//!   se moviera, el upgrade movería la jubilación de todo el mundo sin que nadie tocara nada.
//! * **Tri-estado del PATCH**: omitir ≠ `null`. Es el bug que `FireSettingsPatch` existe para
//!   esquivar, y este módulo repite el patrón: sin test, un PATCH «solo el SWR» podría borrar la
//!   pensión declarada y nadie se enteraría hasta ver la proyección.
//! * **Validación cruzada**: cada regla con su código estable. Los códigos son contrato (los
//!   traduce `errorMessages.ts`).
//! * **Compatibilidad hacia atrás del ALMACÉN**: un JSONB escrito por 5.0.0-WP5 —con
//!   `target_basis`, `bridge_discount_basis` o `cash_buffer_months`— tiene que seguir cargando, y
//!   `strategy: "pension_bridge"` tiene que resolver a `asap` con el puente encendido (C7). Un
//!   perfil que no carga no es un default: es un 500 en la pantalla de jubilación.
//! * **El umbral vuelve a mandar** (M2/C3): entre V7 y v2 se «aceptaba y se ignoraba»; ahora
//!   decide la fecha, se valida y se persiste. La migración borra los 95 que aquella promesa
//!   dejó almacenados.
//! * **Cualquier rol edita el SUYO**: el perfil es dato personal, no configuración del hogar.
//! * **Es input del motor**: toda escritura invalida la cache de proyección.

mod common;

use common::TestApp;

const PROFILE: &str = "/v1/auth/me/retirement-profile";

/// La migración de limpieza de v2, TAL CUAL se despliega. Se ejecuta desde el test para que lo
/// que se prueba sea el fichero que corre en producción y no una copia que se puede desincronizar.
const DROP_STORED_THRESHOLD_SQL: &str =
    include_str!("../migrations/20260906091500_drop_stored_success_threshold.sql");

#[tokio::test]
async fn a_fresh_user_gets_the_v2_defaults() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;

    let r = app.get_with_cookie(PROFILE, &owner.cookie).await;
    assert_eq!(r.status, http::StatusCode::OK, "{r:?}");
    let b = r.json();
    let p = &b["profile"];
    assert_eq!(p["strategy"], "asap", "{b}");
    assert_eq!(p["swr_pct"], "3.5", "{b}");
    assert_eq!(p["horizon_lifespan_age"], 90, "{b}");
    assert_eq!(p["fire_number_mode"], "annual_expense", "{b}");
    // v2 (M2/C3): el umbral vuelve al perfil como restricción, con default 95. No es un adorno
    // del veredicto: es lo que decide la fecha.
    assert_eq!(p["success_threshold_pct"], 95, "{b}");
    // v2 (M10/M11): los dos modos nuevos, con su default.
    assert_eq!(p["coast_mode"], "fixed_retirement_age", "{b}");
    assert!(p["coast_stop_age"].is_null(), "{b}");
    assert_eq!(p["withdrawal_rule"]["kind"], "fixed_real", "{b}");
    assert_eq!(p["withdrawal_rule"]["spend_mode"], "ceiling", "{b}");
    assert!(p["pension"].is_null(), "{b}");
    assert!(p["partial_retirement"].is_null(), "{b}");
    // Los tres ejes RETIRADOS no vuelven ni como cortesía: publicar un `null` sugeriría que la
    // clave sigue significando algo.
    for dead in ["target_basis", "bridge_discount_basis", "cash_buffer_months"] {
        assert!(p.get(dead).is_none(), "{dead} se retiró en v2: {b}");
    }
    assert!(
        b.get("target_basis_stored").is_none(),
        "murió con la base del objetivo (M4): {b}"
    );
    // La DOB viaja al lado porque es lo que convierte cada edad del perfil en un mes.
    assert_eq!(b["birth_date"], "1990-01-01", "{b}");

    // Y la columna sigue siendo NULL: leer no escribe.
    let stored: Option<serde_json::Value> =
        sqlx::query_scalar("SELECT retirement_profile FROM users WHERE id = $1")
            .bind(owner.user_id)
            .fetch_one(&app.pool)
            .await
            .expect("select profile");
    assert!(stored.is_none(), "un GET no debe materializar el perfil");
}

#[tokio::test]
async fn patch_roundtrips_and_only_touches_what_it_names() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;

    // Perfil completo: estrategia por edad + pensión con fecha (y su puente) + regla de retirada
    // + umbral propio.
    let r = app
        .patch_json_with_cookie(
            PROFILE,
            serde_json::json!({
                "strategy": "retire_at_age",
                "target_retirement_age": 58,
                "swr_pct": "3.25",
                "horizon_lifespan_age": 95,
                "success_threshold_pct": 90,
                "coast_mode": "fixed_stop_age",
                "coast_stop_age": 50,
                "pension": {
                    "monthly_amount_today": "1200",
                    "starts_at_age": 67,
                    "indexed": false,
                    "fraction_while_partial": "0.5",
                    "bridge_enabled": true,
                    "bridge_max_pct": "5",
                    "bridge_max_years": 8
                },
                "withdrawal_rule": {
                    "kind": "guardrails",
                    "pct": "4",
                    "band_pct": "20",
                    "adjust_pct": "10",
                    "spend_mode": "rule_is_spend"
                }
            }),
            &owner.cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::OK, "{r:?}");
    let p = r.json()["profile"].clone();
    assert_eq!(p["strategy"], "retire_at_age");
    assert_eq!(p["target_retirement_age"], 58);
    assert_eq!(p["swr_pct"], "3.25");
    assert_eq!(p["success_threshold_pct"], 90);
    assert_eq!(p["coast_mode"], "fixed_stop_age");
    assert_eq!(p["coast_stop_age"], 50);
    assert_eq!(p["pension"]["monthly_amount_today"], "1200");
    assert_eq!(p["pension"]["indexed"], false);
    assert_eq!(p["pension"]["fraction_while_partial"], "0.5");
    assert_eq!(p["pension"]["bridge_enabled"], true);
    assert_eq!(p["pension"]["bridge_max_pct"], "5");
    assert_eq!(p["pension"]["bridge_max_years"], 8);
    assert_eq!(p["withdrawal_rule"]["kind"], "guardrails");
    assert_eq!(p["withdrawal_rule"]["spend_mode"], "rule_is_spend");

    // Un GET devuelve lo mismo: lo que se guarda es lo que se lee.
    let again = app.get_with_cookie(PROFILE, &owner.cookie).await;
    assert_eq!(again.json()["profile"], p, "el GET no coincide con el PATCH");

    // TRI-ESTADO. Un PATCH que solo nombra el SWR NO puede resetear nada más.
    let r = app
        .patch_json_with_cookie(PROFILE, serde_json::json!({"swr_pct": "3.0"}), &owner.cookie)
        .await;
    assert_eq!(r.status, http::StatusCode::OK, "{r:?}");
    let after = r.json()["profile"].clone();
    assert_eq!(after["swr_pct"], "3.0");
    assert_eq!(after["pension"], p["pension"], "la pensión se ha reseteado: {after}");
    assert_eq!(after["withdrawal_rule"], p["withdrawal_rule"], "{after}");
    assert_eq!(after["success_threshold_pct"], 90, "{after}");
    assert_eq!(after["coast_stop_age"], 50, "{after}");

    // `null` explícito SÍ borra — y el puente se va con la pensión, porque vive DENTRO de ella
    // (C7): no hay puente sin destino.
    let r = app
        .patch_json_with_cookie(PROFILE, serde_json::json!({"pension": null}), &owner.cookie)
        .await;
    assert_eq!(r.status, http::StatusCode::OK, "{r:?}");
    let cleared = r.json()["profile"].clone();
    assert!(cleared["pension"].is_null(), "{cleared}");

    // …y la edad de parada también es tri-estado.
    let r = app
        .patch_json_with_cookie(PROFILE, serde_json::json!({"coast_stop_age": null}), &owner.cookie)
        .await;
    assert_eq!(r.status, http::StatusCode::OK, "{r:?}");
    assert!(r.json()["profile"]["coast_stop_age"].is_null(), "{r:?}");

    // Un PATCH vacío es un error, no un 200 silencioso.
    let r = app
        .patch_json_with_cookie(PROFILE, serde_json::json!({}), &owner.cookie)
        .await;
    assert_eq!(r.status, http::StatusCode::BAD_REQUEST, "{r:?}");
    assert_eq!(r.json()["code"], "patch_empty");
}

#[tokio::test]
async fn birth_date_travels_with_the_profile_and_is_tri_state() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;

    let r = app
        .patch_json_with_cookie(
            PROFILE,
            serde_json::json!({"birth_date": "1985-06-30"}),
            &owner.cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::OK, "{r:?}");
    assert_eq!(r.json()["birth_date"], "1985-06-30");
    // Y es la MISMA columna que `/v1/auth/me`: dos pantallas, un dato.
    let me = app.get_with_cookie("/v1/auth/me", &owner.cookie).await;
    assert_eq!(me.json()["birth_date"], "1985-06-30", "{me:?}");

    // `null` la borra.
    let r = app
        .patch_json_with_cookie(PROFILE, serde_json::json!({"birth_date": null}), &owner.cookie)
        .await;
    assert_eq!(r.status, http::StatusCode::OK, "{r:?}");
    assert!(r.json()["birth_date"].is_null(), "{r:?}");

    // Y una fecha imposible se rechaza con el mismo código que `/v1/auth/me`.
    let r = app
        .patch_json_with_cookie(
            PROFILE,
            serde_json::json!({"birth_date": "2999-01-01"}),
            &owner.cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::BAD_REQUEST, "{r:?}");
    assert_eq!(r.json()["code"], "birth_date_future");
}

/// Cada regla de validación con su código ESTABLE. Los códigos son contrato: la SPA los traduce
/// (`errorMessages.ts`) y `error_codes_parity` los congela.
#[tokio::test]
async fn every_validation_rule_has_its_own_stable_code() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;

    let cases: Vec<(serde_json::Value, &str)> = vec![
        // Estrategias por edad sin edad. `coast` la exige solo en el modo A (M10).
        (
            serde_json::json!({"strategy": "retire_at_age"}),
            "target_retirement_age_required",
        ),
        (
            serde_json::json!({"strategy": "coast"}),
            "target_retirement_age_required",
        ),
        // Coast modo B sin edad de parada: ahí el dato es cuándo dejas de aportar.
        (
            serde_json::json!({"strategy": "coast", "coast_mode": "fixed_stop_age"}),
            "coast_stop_age_required",
        ),
        // Media jornada sin fase de media jornada.
        (
            serde_json::json!({"strategy": "partial"}),
            "partial_retirement_required",
        ),
        // …y con fase pero sin edad de inicio en el modo `at_age` (en `asap` la calcula el solver).
        (
            serde_json::json!({
                "strategy": "partial",
                "partial_retirement": {"income_monthly_today": "800"}
            }),
            "partial_start_age_required",
        ),
        // Edades fuera de rango.
        (
            serde_json::json!({"strategy": "retire_at_age", "target_retirement_age": 12}),
            "retirement_age_out_of_range",
        ),
        (
            serde_json::json!({"coast_stop_age": 12}),
            "coast_stop_age_out_of_range",
        ),
        (
            serde_json::json!({"pension": {"monthly_amount_today": "1000", "starts_at_age": 40}}),
            "pension_age_out_of_range",
        ),
        // Pensión con importe no positivo.
        (
            serde_json::json!({"pension": {"monthly_amount_today": "0", "starts_at_age": 67}}),
            "pension_amount_not_positive",
        ),
        // Fracción fuera de [0,1].
        (
            serde_json::json!({"pension": {"monthly_amount_today": "900", "starts_at_age": 67, "fraction_while_partial": "1.5"}}),
            "pension_fraction_out_of_range",
        ),
        // El puente (C2/C7): tres cotas, tres códigos.
        (
            serde_json::json!({"pension": {"monthly_amount_today": "900", "starts_at_age": 67, "bridge_enabled": true, "bridge_max_pct": "25"}}),
            "bridge_max_pct_out_of_range",
        ),
        (
            serde_json::json!({"pension": {"monthly_amount_today": "900", "starts_at_age": 67, "bridge_enabled": true, "bridge_max_pct": "3"}}),
            "bridge_max_pct_not_above_swr",
        ),
        (
            serde_json::json!({"pension": {"monthly_amount_today": "900", "starts_at_age": 67, "bridge_enabled": true, "bridge_max_pct": "5", "bridge_max_years": 40}}),
            "bridge_max_years_out_of_range",
        ),
        // Parcial que no empieza antes de la total.
        (
            serde_json::json!({
                "strategy": "partial",
                "target_retirement_age": 60,
                "partial_retirement": {"starts_at_age": 62, "income_monthly_today": "800"}
            }),
            "partial_not_before_retirement",
        ),
        (
            serde_json::json!({
                "partial_retirement": {"starts_at_age": 12, "income_monthly_today": "800"}
            }),
            "partial_age_out_of_range",
        ),
        // Reglas de retirada: cada `kind` exige LOS SUYOS. Tras U4, `pct` y `start_pct` ya NO
        // están en esa lista (heredan `swr_pct`); sí siguen el `end_pct` del hybrid y la
        // banda/ajuste de guardrails, que no son porcentajes de retirada.
        (
            serde_json::json!({"withdrawal_rule": {"kind": "hybrid"}}),
            "withdrawal_pct_required",
        ),
        (
            serde_json::json!({"withdrawal_rule": {"kind": "guardrails", "pct": "4", "adjust_pct": "10"}}),
            "withdrawal_pct_required",
        ),
        (
            serde_json::json!({"withdrawal_rule": {"kind": "percent_of_balance", "pct": "25"}}),
            "withdrawal_pct_out_of_range",
        ),
        (
            serde_json::json!({"withdrawal_rule": {"kind": "hybrid", "start_pct": "3", "end_pct": "4"}}),
            "hybrid_end_pct_not_below_start",
        ),
        (
            serde_json::json!({"withdrawal_rule": {"kind": "guardrails", "pct": "4", "band_pct": "80", "adjust_pct": "10"}}),
            "withdrawal_band_out_of_range",
        ),
        // Los cuatro ejes MOVIDOS conservan los códigos que tenían en `fire_settings`. El techo
        // del SWR sube a 6 en v2 (M5), así que el que falla es un 9.
        (serde_json::json!({"swr_pct": "9"}), "swr_out_of_range"),
        (
            serde_json::json!({"horizon_lifespan_age": 200}),
            "horizon_lifespan_age_out_of_range",
        ),
        (
            serde_json::json!({"fire_number_mode": "manual"}),
            "fire_manual_amount_required",
        ),
        // El umbral (M2/C3): rango cerrado [80, 100], por los dos lados.
        (
            serde_json::json!({"success_threshold_pct": 79}),
            "success_threshold_out_of_range",
        ),
        (
            serde_json::json!({"success_threshold_pct": 101}),
            "success_threshold_out_of_range",
        ),
    ];

    for (body, code) in cases {
        let r = app.patch_json_with_cookie(PROFILE, body.clone(), &owner.cookie).await;
        assert_eq!(
            r.status,
            http::StatusCode::BAD_REQUEST,
            "{body} debía dar 400 y dio {r:?}"
        );
        assert_eq!(r.json()["code"], code, "código equivocado para {body}: {r:?}");
    }

    // El SWR sube a 6 (M5): un 5 que en 4.15.x era un 400 ahora entra.
    let r = app
        .patch_json_with_cookie(PROFILE, serde_json::json!({"swr_pct": "5"}), &owner.cookie)
        .await;
    assert_eq!(r.status, http::StatusCode::OK, "el techo del SWR es 6 en v2: {r:?}");

    // Y ninguno de esos rechazos había persistido nada (el 200 de arriba sí, y es el único).
    let stored: serde_json::Value =
        sqlx::query_scalar("SELECT retirement_profile FROM users WHERE id = $1")
            .bind(owner.user_id)
            .fetch_one(&app.pool)
            .await
            .expect("select profile");
    assert_eq!(stored["swr_pct"], "5", "solo el PATCH válido debió escribir: {stored}");
    assert!(stored["pension"].is_null(), "{stored}");
    assert!(stored["partial_retirement"].is_null(), "{stored}");
}

/// Un literal desconocido en un enum del perfil lo corta serde con un 422 (misma conducta que el
/// resto del wire HTTP: por MCP el mismo valor da un 400 con código nuestro).
#[tokio::test]
async fn an_unknown_strategy_is_rejected_by_serde() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    for body in [
        serde_json::json!({"strategy": "no_existe_esta_estrategia"}),
        serde_json::json!({"coast_mode": "no_existe_este_modo"}),
        serde_json::json!({
            "partial_retirement": {"starts_at_age": 55, "income_monthly_today": "800", "mode": "cuando_sea"}
        }),
    ] {
        let r = app.patch_json_with_cookie(PROFILE, body.clone(), &owner.cookie).await;
        assert_eq!(
            r.status,
            http::StatusCode::UNPROCESSABLE_ENTITY,
            "{body}: {r:?}"
        );
    }
}

/// El perfil es DATO PERSONAL, no configuración del hogar: cualquier rol edita el suyo.
///
/// Es la única superficie de escritura del API que un `viewer` puede usar, y no es una excepción
/// arbitraria: sin poder fijar su edad de jubilación no podría ver su propia proyección, que es
/// exactamente lo que un viewer sí puede hacer.
#[tokio::test]
async fn any_role_can_edit_its_own_profile_viewer_included() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let viewer = app.register_and_approve_member(&owner, "bob", "viewer").await;

    let r = app
        .patch_json_with_cookie(
            PROFILE,
            serde_json::json!({"strategy": "retire_at_age", "target_retirement_age": 55}),
            &viewer.cookie,
        )
        .await;
    assert_eq!(
        r.status,
        http::StatusCode::OK,
        "un viewer debe poder configurar SU jubilación: {r:?}"
    );
    assert_eq!(r.json()["profile"]["target_retirement_age"], 55);

    // Y no ha tocado el del owner: son dos filas distintas.
    let owners = app.get_with_cookie(PROFILE, &owner.cookie).await;
    assert_eq!(owners.json()["profile"]["strategy"], "asap", "{owners:?}");
    assert!(
        owners.json()["profile"]["target_retirement_age"].is_null(),
        "{owners:?}"
    );
}

/// El perfil es INPUT del motor (SWR, umbral, modo del objetivo, edad límite): toda escritura
/// invalida.
#[tokio::test]
async fn writing_the_profile_invalidates_the_projection_cache() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let iid = app.installation_id().await;
    let key = app.default_view_key(iid, owner.user_id);

    app.warm_default_view(&owner.cookie, &key).await;
    let r = app
        .patch_json_with_cookie(PROFILE, serde_json::json!({"swr_pct": "3.0"}), &owner.cookie)
        .await;
    assert_eq!(r.status, http::StatusCode::OK, "{r:?}");
    app.assert_invalidated(&key, "PATCH retirement-profile").await;
}

// ---------------------------------------------------------------------------------------------
// El umbral de éxito (M2/C3): vuelve al perfil y MANDA
// ---------------------------------------------------------------------------------------------

/// **El umbral se guarda, se acota y decide** — lo contrario exacto de lo que hacía entre V7 y v2,
/// cuando se «aceptaba y se ignoraba».
///
/// Lo que este test impide que vuelva: un 200 silencioso ante un umbral imposible. Un cliente que
/// manda 101 y recibe un 200 cree que su plan exige el 101 % y lee la fecha que sale como si lo
/// cumpliera.
#[tokio::test]
async fn the_success_threshold_is_stored_clamped_and_load_bearing() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;

    // 1) Un valor válido se PERSISTE y vuelve por el GET.
    let r = app
        .patch_json_with_cookie(
            PROFILE,
            serde_json::json!({"success_threshold_pct": 90}),
            &owner.cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::OK, "{r:?}");
    assert_eq!(r.json()["profile"]["success_threshold_pct"], 90, "{r:?}");

    let b = app.get_with_cookie(PROFILE, &owner.cookie).await.json();
    assert_eq!(b["profile"]["success_threshold_pct"], 90, "{b}");

    let stored: serde_json::Value =
        sqlx::query_scalar("SELECT retirement_profile FROM users WHERE id = $1")
            .bind(owner.user_id)
            .fetch_one(&app.pool)
            .await
            .expect("select profile");
    assert_eq!(stored["success_threshold_pct"], 90, "{stored}");

    // 2) Fuera de rango es 400, no un descarte silencioso.
    for bad in [0u32, 79, 101, 4_000] {
        let r = app
            .patch_json_with_cookie(
                PROFILE,
                serde_json::json!({"success_threshold_pct": bad}),
                &owner.cookie,
            )
            .await;
        assert_eq!(r.status, http::StatusCode::BAD_REQUEST, "{bad}: {r:?}");
        assert_eq!(r.json()["code"], "success_threshold_out_of_range", "{r:?}");
    }
    // …y el rechazo no ha movido el valor bueno.
    let b = app.get_with_cookie(PROFILE, &owner.cookie).await.json();
    assert_eq!(b["profile"]["success_threshold_pct"], 90, "{b}");

    // 3) Los dos extremos SÍ entran: el rango es cerrado.
    for ok in [80u32, 100] {
        let r = app
            .patch_json_with_cookie(
                PROFILE,
                serde_json::json!({"success_threshold_pct": ok}),
                &owner.cookie,
            )
            .await;
        assert_eq!(r.status, http::StatusCode::OK, "{ok}: {r:?}");
        assert_eq!(r.json()["profile"]["success_threshold_pct"], ok, "{r:?}");
    }

    // 4) Un valor imposible llegado por OTRA vía (restore, edición directa de la BD) se ACOTA al
    //    leer, nunca revienta la pantalla.
    sqlx::query(r#"UPDATE users SET retirement_profile = $1::jsonb WHERE id = $2"#)
        .bind(r#"{"strategy":"asap","success_threshold_pct":5}"#)
        .bind(owner.user_id)
        .execute(&app.pool)
        .await
        .expect("seed out-of-range threshold");
    let b = app.get_with_cookie(PROFILE, &owner.cookie).await.json();
    assert_eq!(b["profile"]["success_threshold_pct"], 80, "clamp de lectura: {b}");
}

/// **La migración borra el umbral almacenado que nadie leía** (y las tres claves retiradas con
/// él), y el perfil vuelve al DEFAULT — no al valor muerto.
///
/// El fallo que impide: entre V7 y v2 la clave existía en el JSONB de todo el que hubiera tocado
/// la pantalla, con el 95 que la SPA mandaba y el servidor descartaba. Al volver a ser
/// load-bearing, ese 95 se leería como «lo elegí yo»: idéntico al default de hoy, pero congelado
/// el día que el default se mueva.
#[tokio::test]
async fn the_migration_drops_the_deprecated_threshold() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;

    // Estado exacto de un perfil escrito entre V7 y v2: el umbral descartado + las tres claves
    // que v2 retira.
    sqlx::query(r#"UPDATE users SET retirement_profile = $1::jsonb WHERE id = $2"#)
        .bind(
            r#"{"strategy":"asap","swr_pct":"3.5","success_threshold_pct":95,
                "target_basis":"bridge_to_pension","bridge_discount_basis":"swr",
                "cash_buffer_months":24}"#,
        )
        .bind(owner.user_id)
        .execute(&app.pool)
        .await
        .expect("seed pre-v2 profile");

    // La migración TAL CUAL se despliega.
    sqlx::query(DROP_STORED_THRESHOLD_SQL)
        .execute(&app.pool)
        .await
        .expect("run the cleanup migration");

    let stored: serde_json::Value =
        sqlx::query_scalar("SELECT retirement_profile FROM users WHERE id = $1")
            .bind(owner.user_id)
            .fetch_one(&app.pool)
            .await
            .expect("select profile");
    for dead in [
        "success_threshold_pct",
        "target_basis",
        "bridge_discount_basis",
        "cash_buffer_months",
    ] {
        assert!(stored.get(dead).is_none(), "{dead} debió borrarse: {stored}");
    }
    assert_eq!(stored["swr_pct"], "3.5", "lo demás no se toca: {stored}");

    // Y lo que se publica es el DEFAULT, que hoy vale lo mismo — pero por default.
    let b = app.get_with_cookie(PROFILE, &owner.cookie).await.json();
    assert_eq!(b["profile"]["success_threshold_pct"], 95, "{b}");

    // Idempotente: volver a correrla no encuentra nada que hacer ni rompe.
    sqlx::query(DROP_STORED_THRESHOLD_SQL)
        .execute(&app.pool)
        .await
        .expect("second run is a no-op");
    let b = app.get_with_cookie(PROFILE, &owner.cookie).await.json();
    assert_eq!(b["profile"]["success_threshold_pct"], 95, "{b}");
}

// ---------------------------------------------------------------------------------------------
// El puente (C2/C7): un ajuste de la pensión, no una estrategia
// ---------------------------------------------------------------------------------------------

/// **El puente vive DENTRO de la pensión y viene apagado**, con cualquier estrategia.
#[tokio::test]
async fn the_bridge_lives_inside_the_pension_and_is_off_by_default() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;

    // Sin pensión no hay puente que declarar: no hay campos sueltos en la raíz del perfil.
    let b = app.get_with_cookie(PROFILE, &owner.cookie).await.json();
    for dead in ["bridge_enabled", "bridge_max_pct", "bridge_max_years"] {
        assert!(
            b["profile"].get(dead).is_none(),
            "{dead} vive dentro de `pension`: {b}"
        );
    }

    // Declarar la pensión NO enciende el puente: es una elección, no una consecuencia.
    let r = app
        .patch_json_with_cookie(
            PROFILE,
            serde_json::json!({"pension": {"monthly_amount_today": "1200", "starts_at_age": 67}}),
            &owner.cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::OK, "{r:?}");
    let pen = r.json()["profile"]["pension"].clone();
    assert_eq!(pen["bridge_enabled"], false, "{pen}");
    assert!(pen["bridge_max_pct"].is_null(), "{pen}");
    assert!(pen["bridge_max_years"].is_null(), "{pen}");

    // Y `pension_bridge` ya NO es una estrategia elegible: el literal solo sobrevive como alias
    // de compatibilidad en el almacén (ver el test del perfil guardado).
    let r = app
        .patch_json_with_cookie(
            PROFILE,
            serde_json::json!({"strategy": "pension_bridge"}),
            &owner.cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::OK, "{r:?}");
    assert_eq!(
        r.json()["profile"]["strategy"], "asap",
        "el alias resuelve a asap y NUNCA se re-emite: {r:?}"
    );
}

/// **Encender el puente sin números lo deja con los defaults del owner**: `max(5, swr + 1)` % y
/// 7 años. Un puente sin tope no es un puente: es una retirada inicial sin límite.
#[tokio::test]
async fn enabling_the_bridge_without_numbers_fills_the_defaults() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;

    // SWR 3,5 (default) → 5 % y 7 años.
    let r = app
        .patch_json_with_cookie(
            PROFILE,
            serde_json::json!({
                "pension": {"monthly_amount_today": "1200", "starts_at_age": 67, "bridge_enabled": true}
            }),
            &owner.cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::OK, "{r:?}");
    let pen = r.json()["profile"]["pension"].clone();
    assert_eq!(pen["bridge_enabled"], true, "{pen}");
    assert_eq!(pen["bridge_max_pct"], "5", "{pen}");
    assert_eq!(pen["bridge_max_years"], 7, "{pen}");

    // SWR 5 → 6 %: el default es siempre MAYOR que el SWR, o el puente no concedería nada.
    let r = app
        .patch_json_with_cookie(PROFILE, serde_json::json!({"swr_pct": "5"}), &owner.cookie)
        .await;
    assert_eq!(r.status, http::StatusCode::OK, "{r:?}");
    let pen = r.json()["profile"]["pension"].clone();
    assert_eq!(pen["bridge_max_pct"], "6", "{pen}");

    // …y el default NO se materializa en el JSONB: se resuelve en cada lectura, así que mover el
    // SWR mueve el tope. Si se persistiera, el puente se quedaría anclado al SWR de aquel día.
    let stored: serde_json::Value =
        sqlx::query_scalar("SELECT retirement_profile FROM users WHERE id = $1")
            .bind(owner.user_id)
            .fetch_one(&app.pool)
            .await
            .expect("select profile");
    assert!(
        stored["pension"]["bridge_max_pct"].is_null(),
        "el tope derivado no debe persistirse: {stored}"
    );
}

/// **Un puente que no levanta la tasa no es un puente**: `bridge_max_pct <= swr_pct` se rechaza,
/// y el mensaje dice contra qué número se ha comparado (B8).
#[tokio::test]
async fn a_bridge_pct_at_or_below_the_swr_is_rejected() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;

    let pension_with = |pct: &str| {
        serde_json::json!({
            "swr_pct": "4",
            "pension": {
                "monthly_amount_today": "1200",
                "starts_at_age": 67,
                "bridge_enabled": true,
                "bridge_max_pct": pct
            }
        })
    };

    for pct in ["3", "4"] {
        let r = app
            .patch_json_with_cookie(PROFILE, pension_with(pct), &owner.cookie)
            .await;
        assert_eq!(r.status, http::StatusCode::BAD_REQUEST, "{pct}: {r:?}");
        let b = r.json();
        assert_eq!(b["code"], "bridge_max_pct_not_above_swr", "{b}");
        assert!(
            b["message"].as_str().is_some_and(|m| m.contains('4')),
            "el mensaje debe nombrar la tasa de retirada real: {b}"
        );
    }

    // Estrictamente por encima entra.
    let r = app
        .patch_json_with_cookie(PROFILE, pension_with("4.5"), &owner.cookie)
        .await;
    assert_eq!(r.status, http::StatusCode::OK, "{r:?}");
    assert_eq!(r.json()["profile"]["pension"]["bridge_max_pct"], "4.5", "{r:?}");
}

/// **Un perfil guardado con la estrategia retirada `pension_bridge` se lee como `asap` con el
/// puente encendido** (C7). Es el único camino de migración de esos perfiles: sin él, quien
/// eligió «Puente hasta la pensión» se despertaría jubilándose con el SWR de siempre.
#[tokio::test]
async fn a_stored_pension_bridge_profile_reads_as_asap_with_the_bridge_on() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;

    // El JSONB EXACTO que dejaba 5.0.0-WP5 para esa estrategia (con su base del objetivo y su
    // descuento, que v2 ignora).
    sqlx::query(r#"UPDATE users SET retirement_profile = $1::jsonb WHERE id = $2"#)
        .bind(
            r#"{"strategy":"pension_bridge","swr_pct":"3.5","target_basis":"bridge_to_pension",
                "bridge_discount_basis":"expected_return",
                "pension":{"monthly_amount_today":"1200","starts_at_age":67,"indexed":true}}"#,
        )
        .bind(owner.user_id)
        .execute(&app.pool)
        .await
        .expect("seed pension_bridge profile");

    let r = app.get_with_cookie(PROFILE, &owner.cookie).await;
    assert_eq!(r.status, http::StatusCode::OK, "{r:?}");
    let p = r.json()["profile"].clone();
    assert_eq!(p["strategy"], "asap", "el alias resuelve a asap: {p}");
    assert_eq!(p["pension"]["bridge_enabled"], true, "y enciende el puente: {p}");
    assert_eq!(p["pension"]["bridge_max_pct"], "5", "{p}");
    assert_eq!(p["pension"]["bridge_max_years"], 7, "{p}");

    // El literal NUNCA vuelve por el wire, ni siquiera en el mismo GET.
    assert_ne!(p["strategy"], "pension_bridge", "{p}");

    // Y la primera escritura lo deja migrado en el almacén: el alias es de LECTURA.
    let r = app
        .patch_json_with_cookie(PROFILE, serde_json::json!({"swr_pct": "3.0"}), &owner.cookie)
        .await;
    assert_eq!(r.status, http::StatusCode::OK, "{r:?}");
    let stored: serde_json::Value =
        sqlx::query_scalar("SELECT retirement_profile FROM users WHERE id = $1")
            .bind(owner.user_id)
            .fetch_one(&app.pool)
            .await
            .expect("select profile");
    assert_eq!(stored["strategy"], "asap", "{stored}");
    // El puente encendido por la migración se persiste como lo que es: una elección del usuario,
    // que dijo «mi plan es el puente» con el vocabulario de la versión anterior.
    assert_eq!(stored["pension"]["bridge_enabled"], true, "{stored}");
}

/// **Un perfil de 5.0.0-WP5 con las tres claves retiradas sigue cargando**, y ninguna de ellas
/// vuelve por la respuesta. El struct no lleva `deny_unknown_fields` justo por esto.
#[tokio::test]
async fn a_stored_v1_profile_ignores_the_three_retired_fields() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;

    sqlx::query(r#"UPDATE users SET retirement_profile = $1::jsonb WHERE id = $2"#)
        .bind(
            r#"{"strategy":"retire_at_age","target_retirement_age":58,"swr_pct":"3.25",
                "target_basis":"perpetuity","bridge_discount_basis":"none","cash_buffer_months":36}"#,
        )
        .bind(owner.user_id)
        .execute(&app.pool)
        .await
        .expect("seed v1 profile");

    let r = app.get_with_cookie(PROFILE, &owner.cookie).await;
    assert_eq!(r.status, http::StatusCode::OK, "un perfil v1 debe cargar: {r:?}");
    let b = r.json();
    let p = &b["profile"];
    assert_eq!(p["strategy"], "retire_at_age", "{b}");
    assert_eq!(p["target_retirement_age"], 58, "{b}");
    assert_eq!(p["swr_pct"], "3.25", "{b}");
    // Y los defaults de v2 se aplican sobre lo que el JSONB no dice.
    assert_eq!(p["success_threshold_pct"], 95, "{b}");
    assert_eq!(p["coast_mode"], "fixed_retirement_age", "{b}");
    for dead in ["target_basis", "bridge_discount_basis", "cash_buffer_months"] {
        assert!(p.get(dead).is_none(), "{dead} no debe re-emitirse: {b}");
    }

    // Y un PATCH sobre ese perfil no las resucita: lo que se escribe es el perfil v2.
    let r = app
        .patch_json_with_cookie(PROFILE, serde_json::json!({"swr_pct": "3.0"}), &owner.cookie)
        .await;
    assert_eq!(r.status, http::StatusCode::OK, "{r:?}");
    let stored: serde_json::Value =
        sqlx::query_scalar("SELECT retirement_profile FROM users WHERE id = $1")
            .bind(owner.user_id)
            .fetch_one(&app.pool)
            .await
            .expect("select profile");
    for dead in ["target_basis", "bridge_discount_basis", "cash_buffer_months"] {
        assert!(
            stored.get(dead).is_none(),
            "{dead} debe desaparecer en la primera escritura: {stored}"
        );
    }
}

// ---------------------------------------------------------------------------------------------
// Los dos modos nuevos (M10/M11)
// ---------------------------------------------------------------------------------------------

/// **Coast modo B**: el dato es cuándo dejas de aportar, y la edad de jubilación deja de ser
/// obligatoria — la calcula el umbral.
#[tokio::test]
async fn coast_mode_b_needs_a_stop_age_and_releases_the_retirement_age() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;

    // Modo A (default): la edad de jubilación es obligatoria.
    let r = app
        .patch_json_with_cookie(PROFILE, serde_json::json!({"strategy": "coast"}), &owner.cookie)
        .await;
    assert_eq!(r.status, http::StatusCode::BAD_REQUEST, "{r:?}");
    assert_eq!(r.json()["code"], "target_retirement_age_required", "{r:?}");

    // Modo B sin edad de parada: el error es OTRO, y nombra el dato que falta de verdad.
    let r = app
        .patch_json_with_cookie(
            PROFILE,
            serde_json::json!({"strategy": "coast", "coast_mode": "fixed_stop_age"}),
            &owner.cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::BAD_REQUEST, "{r:?}");
    assert_eq!(r.json()["code"], "coast_stop_age_required", "{r:?}");

    // Modo B con edad de parada y SIN edad de jubilación: entra.
    let r = app
        .patch_json_with_cookie(
            PROFILE,
            serde_json::json!({
                "strategy": "coast",
                "coast_mode": "fixed_stop_age",
                "coast_stop_age": 45
            }),
            &owner.cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::OK, "{r:?}");
    let p = r.json()["profile"].clone();
    assert_eq!(p["coast_mode"], "fixed_stop_age", "{p}");
    assert_eq!(p["coast_stop_age"], 45, "{p}");
    assert!(p["target_retirement_age"].is_null(), "{p}");

    // Dejar de aportar DESPUÉS de jubilarse no describe nada: el techo es la edad de jubilación.
    let r = app
        .patch_json_with_cookie(
            PROFILE,
            serde_json::json!({"target_retirement_age": 60, "coast_stop_age": 70}),
            &owner.cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::BAD_REQUEST, "{r:?}");
    assert_eq!(r.json()["code"], "coast_stop_age_out_of_range", "{r:?}");
}

/// **Media jornada modo `asap`**: la edad de inicio la calcula el solver, así que deja de ser
/// obligatoria — pero sigue siéndolo en el modo `at_age`.
#[tokio::test]
async fn partial_mode_asap_makes_the_start_age_optional() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;

    // Modo A (default) sin edad: 400 con su propio código.
    let r = app
        .patch_json_with_cookie(
            PROFILE,
            serde_json::json!({
                "strategy": "partial",
                "partial_retirement": {"income_monthly_today": "900"}
            }),
            &owner.cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::BAD_REQUEST, "{r:?}");
    assert_eq!(r.json()["code"], "partial_start_age_required", "{r:?}");

    // Modo B sin edad: entra, y la fase la coloca el solver.
    let r = app
        .patch_json_with_cookie(
            PROFILE,
            serde_json::json!({
                "strategy": "partial",
                "partial_retirement": {"income_monthly_today": "900", "mode": "asap"}
            }),
            &owner.cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::OK, "{r:?}");
    let par = r.json()["profile"]["partial_retirement"].clone();
    assert_eq!(par["mode"], "asap", "{par}");
    assert!(par["starts_at_age"].is_null(), "{par}");
    assert_eq!(par["income_monthly_today"], "900", "{par}");

    // Modo A con edad: entra, y el modo publicado es el default.
    let r = app
        .patch_json_with_cookie(
            PROFILE,
            serde_json::json!({
                "strategy": "partial",
                "partial_retirement": {"starts_at_age": 55, "income_monthly_today": "900"}
            }),
            &owner.cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::OK, "{r:?}");
    let par = r.json()["profile"]["partial_retirement"].clone();
    assert_eq!(par["mode"], "at_age", "{par}");
    assert_eq!(par["starts_at_age"], 55, "{par}");
}

// ---------------------------------------------------------------------------------------------
// U4 — el porcentaje de retirada es ÚNICO
// ---------------------------------------------------------------------------------------------

/// **`withdrawal_rule.pct` omitido HEREDA `swr_pct`, y el perfil dice de dónde salió.**
///
/// Decisión del owner (U4): el usuario declara UN porcentaje de retirada. `swr_pct` es el tope de
/// venta anual y es a la vez el % de las reglas basadas en saldo; la SPA deja de mandar `pct`. Lo
/// que este test fija es que la ausencia no es un error ni un cero, sino una herencia
/// **declarada** (`pct_source`), y que un valor explícito sigue mandando —el wire de 4.15.x no
/// se rompe.
#[tokio::test]
async fn a_withdrawal_pct_left_out_inherits_the_swr_and_declares_it() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;

    // 1) `percent_of_balance` sin `pct`: 200, y el % resuelto ES el SWR.
    let r = app
        .patch_json_with_cookie(
            PROFILE,
            serde_json::json!({
                "swr_pct": "3.2",
                "withdrawal_rule": {"kind": "percent_of_balance"}
            }),
            &owner.cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::OK, "{r:?}");
    let w = &r.json()["profile"]["withdrawal_rule"];
    assert_eq!(w["pct"], "3.2", "el % debe heredarse del SWR: {w}");
    assert_eq!(w["pct_source"], "swr", "{w}");

    // …y lo ALMACENADO sigue sin `pct`: la herencia se resuelve en lectura, no se materializa en
    // el JSONB. Si se materializara, mover el SWR dejaría de mover la regla.
    let stored: serde_json::Value =
        sqlx::query_scalar("SELECT retirement_profile FROM users WHERE id = $1")
            .bind(owner.user_id)
            .fetch_one(&app.pool)
            .await
            .expect("select profile");
    assert!(
        stored["withdrawal_rule"]["pct"].is_null(),
        "el pct heredado no debe persistirse: {stored}"
    );
    assert!(
        stored["withdrawal_rule"].get("pct_source").is_none(),
        "pct_source es derivado y no se guarda: {stored}"
    );

    // 2) Mover el SWR mueve el % de la regla, porque es el MISMO número.
    let r = app
        .patch_json_with_cookie(PROFILE, serde_json::json!({"swr_pct": "2.5"}), &owner.cookie)
        .await;
    assert_eq!(r.status, http::StatusCode::OK, "{r:?}");
    let w = &r.json()["profile"]["withdrawal_rule"];
    assert_eq!(w["pct"], "2.5", "{w}");
    assert_eq!(w["pct_source"], "swr", "{w}");

    // 3) Un `pct` explícito se honra y se declara como tal (compatibilidad con 4.15.x/MCP).
    let r = app
        .patch_json_with_cookie(
            PROFILE,
            serde_json::json!({"withdrawal_rule": {"kind": "percent_of_balance", "pct": "5"}}),
            &owner.cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::OK, "{r:?}");
    let w = &r.json()["profile"]["withdrawal_rule"];
    assert_eq!(w["pct"], "5", "{w}");
    assert_eq!(w["pct_source"], "explicit", "{w}");

    // 4) Cómo se SUELTA: la regla se sustituye entera, así que volver a mandarla sin `pct` lo
    //    re-acopla al SWR. No hay `clear_pct` ni hace falta.
    let r = app
        .patch_json_with_cookie(
            PROFILE,
            serde_json::json!({"withdrawal_rule": {"kind": "percent_of_balance"}}),
            &owner.cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::OK, "{r:?}");
    let w = &r.json()["profile"]["withdrawal_rule"];
    assert_eq!(w["pct"], "2.5", "{w}");
    assert_eq!(w["pct_source"], "swr", "{w}");

    // 5) `fixed_real` no tiene porcentaje: `pct_source` NO viaja (ni siquiera como `null`).
    //    Publicar «swr» ahí sugeriría un % en juego que esa regla no usa.
    let r = app
        .patch_json_with_cookie(
            PROFILE,
            serde_json::json!({"withdrawal_rule": {"kind": "fixed_real"}}),
            &owner.cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::OK, "{r:?}");
    let w = &r.json()["profile"]["withdrawal_rule"];
    assert!(w.get("pct_source").is_none(), "{w}");

    // 6) `guardrails` hereda por `pct` igual que `percent_of_balance`; su banda y su ajuste
    //    siguen siendo obligatorios (no son porcentajes de retirada).
    let r = app
        .patch_json_with_cookie(
            PROFILE,
            serde_json::json!({
                "withdrawal_rule": {"kind": "guardrails", "band_pct": "20", "adjust_pct": "10"}
            }),
            &owner.cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::OK, "{r:?}");
    let w = &r.json()["profile"]["withdrawal_rule"];
    assert_eq!(w["pct"], "2.5", "{w}");
    assert_eq!(w["pct_source"], "swr", "{w}");
}

/// **El `end_pct` del hybrid se compara contra el `start_pct` RESUELTO**, y el mensaje dice contra
/// qué número (B8).
///
/// Es el caso que la herencia podría haber roto en silencio: sin `start_pct` explícito, un
/// `end_pct` mayor o igual que el SWR describiría un latch que sube en vez de bajar. El código
/// es el que ya existía —`hybrid_end_pct_not_below_start`—, porque la regla no ha cambiado: lo
/// que ha cambiado es contra qué número se comprueba.
#[tokio::test]
async fn a_hybrid_without_start_pct_compares_end_pct_against_the_swr() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;

    // SWR 3,5 (default) y `end_pct` 3,5: no es MENOR que el arranque resuelto → rechazo.
    let r = app
        .patch_json_with_cookie(
            PROFILE,
            serde_json::json!({"withdrawal_rule": {"kind": "hybrid", "end_pct": "3.5"}}),
            &owner.cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::BAD_REQUEST, "{r:?}");
    let b = r.json();
    assert_eq!(b["code"], "hybrid_end_pct_not_below_start", "{b}");
    assert!(
        b["message"].as_str().is_some_and(|m| m.contains("3.5")),
        "el mensaje debe nombrar la tasa contra la que se compara: {b}"
    );

    // Por encima del SWR, lo mismo.
    let r = app
        .patch_json_with_cookie(
            PROFILE,
            serde_json::json!({"withdrawal_rule": {"kind": "hybrid", "end_pct": "4"}}),
            &owner.cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::BAD_REQUEST, "{r:?}");
    assert_eq!(r.json()["code"], "hybrid_end_pct_not_below_start", "{r:?}");

    // Por debajo, entra — y el arranque publicado es el SWR heredado.
    let r = app
        .patch_json_with_cookie(
            PROFILE,
            serde_json::json!({"withdrawal_rule": {"kind": "hybrid", "end_pct": "2.5"}}),
            &owner.cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::OK, "{r:?}");
    let w = &r.json()["profile"]["withdrawal_rule"];
    assert_eq!(w["start_pct"], "3.5", "{w}");
    assert_eq!(w["end_pct"], "2.5", "{w}");
    assert_eq!(w["pct_source"], "swr", "{w}");
}

/// **El upgrade desde 4.15.x no mueve un número.**
///
/// Se reproduce el estado exacto que deja la migración `20260902200000_…`: los cuatro ejes
/// copiados al perfil del usuario y retirados del JSONB de la instalación. Con un SWR distinto
/// del default, el número FIRE clásico publicado tiene que ser el que producía `fire_settings` —
/// si el handler se hubiera quedado leyendo el eje del sitio viejo (o del default), saldría
/// dimensionado con 3,5 % en vez de con el 2 % que esa persona había configurado.
///
/// **Anclado a `fire_number_classic_today`** (contrato v2): en v2 no hay objetivo descontado, y
/// `jubilacion_target_net_worth` —lo que este test miraba— murió con él (M4). Lo que sobrevive
/// como escalar informativo es el número FIRE clásico, que sigue saliendo del SWR del PERFIL.
#[tokio::test]
async fn an_installation_upgraded_from_4_15_keeps_its_fire_number() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;

    // Un gasto de jubilación con el que el número existe (modo `annual_expense`, sin impuestos
    // para que la cifra sea aritmética limpia: 12·1.000 / 0,02 = 600.000).
    let cat = app.create_category(&owner, "expense", "Vivienda").await;
    let create = app
        .post_json_with_cookie(
            "/v1/budget/entries",
            serde_json::json!({
                "category_id": cat,
                "amount": "1000",
                "persists_after_retirement": true
            }),
            &owner.cookie,
        )
        .await;
    assert_eq!(create.status, http::StatusCode::CREATED, "{create:?}");
    let r = app
        .patch_json_with_cookie(
            "/v1/installation",
            serde_json::json!({"fire_settings": {"taxes_enabled": false}}),
            &owner.cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::OK, "{r:?}");

    // ESTADO POST-MIGRACIÓN, escrito a mano en la columna: es lo que la migración deja para un
    // usuario cuya instalación tenía `swr_pct: "2"`.
    sqlx::query(
        r#"UPDATE users SET retirement_profile = $1::jsonb WHERE id = $2"#,
    )
    .bind(r#"{"strategy":"asap","fire_number_mode":"annual_expense","swr_pct":"2","horizon_lifespan_age":90}"#)
    .bind(owner.user_id)
    .execute(&app.pool)
    .await
    .expect("seed migrated profile");

    let series = app
        .get_with_cookie("/v1/projection/series", &owner.cookie)
        .await;
    assert_eq!(series.status, http::StatusCode::OK, "{series:?}");
    let b = series.json();
    assert_eq!(
        b["fire_number_classic_today"], "600000.0000",
        "el número clásico debe salir del SWR del PERFIL (2 %), no del default 3,5 %: {b}"
    );
    assert_eq!(b["horizon_lifespan_age"], 90, "{b}");

    // Y con el SWR de vuelta a 3,5 la cifra se mueve: la de arriba no es una casualidad.
    let r = app
        .patch_json_with_cookie(PROFILE, serde_json::json!({"swr_pct": "3.5"}), &owner.cookie)
        .await;
    assert_eq!(r.status, http::StatusCode::OK, "{r:?}");
    let b = app
        .get_with_cookie("/v1/projection/series", &owner.cookie)
        .await
        .json();
    assert_ne!(b["fire_number_classic_today"], "600000.0000", "{b}");
}
