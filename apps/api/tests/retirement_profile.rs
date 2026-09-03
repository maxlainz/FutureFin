//! `GET|PATCH /v1/auth/me/retirement-profile` — el perfil de jubilación POR USUARIO (5.0.0,
//! issue #207, decisión D13).
//!
//! Lo que se fija aquí, y por qué cada cosa:
//!
//! * **Defaults**: un usuario que nunca lo ha tocado tiene `retirement_profile IS NULL` y el
//!   servidor devuelve la conducta de 4.15.x (`asap`, SWR 3,5, edad límite 90). Si esto se
//!   moviera, el upgrade movería la jubilación de todo el mundo sin que nadie tocara nada.
//! * **Tri-estado del PATCH**: omitir ≠ `null`. Es el bug que `FireSettingsPatch` existe para
//!   esquivar, y este módulo repite el patrón: sin test, un PATCH «solo el SWR» podría borrar la
//!   pensión declarada y nadie se enteraría hasta ver la proyección.
//! * **Validación cruzada**: cada regla con su código estable. Los códigos son contrato (los
//!   traduce `errorMessages.ts`).
//! * **Cualquier rol edita el SUYO**: el perfil es dato personal, no configuración del hogar. Un
//!   `viewer` que no pudiera fijar su edad de jubilación no podría ver su propia proyección.
//! * **Es input del motor**: toda escritura invalida la cache de proyección.
//! * **El upgrade no mueve un número**: el test `an_installation_upgraded_from_4_15_keeps_its_fire_target`
//!   simula el estado que deja la migración e insiste en que el objetivo publicado es el mismo
//!   que producía `fire_settings` con esos cuatro ejes dentro.

mod common;

use common::TestApp;

const PROFILE: &str = "/v1/auth/me/retirement-profile";

#[tokio::test]
async fn a_fresh_user_gets_the_4_15_defaults() {
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
    assert_eq!(p["success_threshold_pct"], 95, "{b}");
    assert_eq!(p["withdrawal_rule"]["kind"], "fixed_real", "{b}");
    assert_eq!(p["withdrawal_rule"]["spend_mode"], "ceiling", "{b}");
    // Sin pensión declarada el objetivo es perpetuo (R6). El campo NUNCA sale `null`: la
    // respuesta publica la base RESUELTA, no la almacenada.
    assert_eq!(p["target_basis"], "perpetuity", "{b}");
    assert!(p["pension"].is_null(), "{b}");
    assert!(p["partial_retirement"].is_null(), "{b}");
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

    // Perfil completo: estrategia por edad + pensión con fecha + regla de retirada.
    let r = app
        .patch_json_with_cookie(
            PROFILE,
            serde_json::json!({
                "strategy": "retire_at_age",
                "target_retirement_age": 58,
                "swr_pct": "3.25",
                "horizon_lifespan_age": 95,
                "pension": {
                    "monthly_amount_today": "1200",
                    "starts_at_age": 67,
                    "indexed": false,
                    "fraction_while_partial": "0.5"
                },
                "withdrawal_rule": {
                    "kind": "guardrails",
                    "pct": "4",
                    "band_pct": "20",
                    "adjust_pct": "10",
                    "spend_mode": "rule_is_spend"
                },
                "cash_buffer_months": 24,
                "success_threshold_pct": 90
            }),
            &owner.cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::OK, "{r:?}");
    let p = r.json()["profile"].clone();
    assert_eq!(p["strategy"], "retire_at_age");
    assert_eq!(p["target_retirement_age"], 58);
    assert_eq!(p["swr_pct"], "3.25");
    assert_eq!(p["pension"]["monthly_amount_today"], "1200");
    assert_eq!(p["pension"]["indexed"], false);
    assert_eq!(p["pension"]["fraction_while_partial"], "0.5");
    assert_eq!(p["withdrawal_rule"]["kind"], "guardrails");
    assert_eq!(p["withdrawal_rule"]["spend_mode"], "rule_is_spend");
    assert_eq!(p["cash_buffer_months"], 24);
    assert_eq!(p["success_threshold_pct"], 90);
    // Con pensión declarada y sin base explícita, el objetivo pasa a ser el PUENTE (R6).
    assert_eq!(p["target_basis"], "bridge_to_pension", "{p}");

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
    assert_eq!(after["cash_buffer_months"], 24, "{after}");

    // `null` explícito SÍ borra… pero la estrategia sigue siendo por edad, así que la pensión se
    // puede quitar sin más. Y al quitarla, la base del objetivo vuelve a derivarse a perpetuity.
    let r = app
        .patch_json_with_cookie(PROFILE, serde_json::json!({"pension": null}), &owner.cookie)
        .await;
    assert_eq!(r.status, http::StatusCode::OK, "{r:?}");
    let cleared = r.json()["profile"].clone();
    assert!(cleared["pension"].is_null(), "{cleared}");
    assert_eq!(cleared["target_basis"], "perpetuity", "{cleared}");

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
        // Estrategias por edad sin edad.
        (
            serde_json::json!({"strategy": "retire_at_age"}),
            "target_retirement_age_required",
        ),
        (
            serde_json::json!({"strategy": "coast"}),
            "target_retirement_age_required",
        ),
        // Puente sin pensión.
        (
            serde_json::json!({"strategy": "pension_bridge"}),
            "pension_required_for_bridge",
        ),
        // …y media jornada sin fase de media jornada: la tercera pata de la misma familia.
        // Sin ella, `partial` se comportaba como `asap` en silencio mientras la UI enseñaba
        // «Media jornada» sobre una proyección que no la tenía.
        (
            serde_json::json!({"strategy": "partial"}),
            "partial_retirement_required",
        ),
        // Edades fuera de rango.
        (
            serde_json::json!({"strategy": "retire_at_age", "target_retirement_age": 12}),
            "retirement_age_out_of_range",
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
        // Parcial que no empieza antes de la total.
        (
            serde_json::json!({
                "strategy": "partial",
                "target_retirement_age": 60,
                "partial_retirement": {"starts_at_age": 62, "income_monthly_today": "800"}
            }),
            "partial_not_before_retirement",
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
        // Los cuatro ejes MOVIDOS conservan los códigos que tenían en `fire_settings`.
        (serde_json::json!({"swr_pct": "9"}), "swr_out_of_range"),
        (
            serde_json::json!({"horizon_lifespan_age": 200}),
            "horizon_lifespan_age_out_of_range",
        ),
        (
            serde_json::json!({"fire_number_mode": "manual"}),
            "fire_manual_amount_required",
        ),
        // Colchón y umbral.
        (
            serde_json::json!({"cash_buffer_months": 999}),
            "cash_buffer_out_of_range",
        ),
        (
            serde_json::json!({"success_threshold_pct": 10}),
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

    // Y ninguno de esos rechazos ha persistido nada.
    let stored: Option<serde_json::Value> =
        sqlx::query_scalar("SELECT retirement_profile FROM users WHERE id = $1")
            .bind(owner.user_id)
            .fetch_one(&app.pool)
            .await
            .expect("select profile");
    assert!(stored.is_none(), "una validación fallida no debe escribir: {stored:?}");
}

/// Un literal desconocido en un enum del perfil lo corta serde con un 422 (misma conducta que el
/// resto del wire HTTP: por MCP el mismo valor da un 400 con código nuestro).
#[tokio::test]
async fn an_unknown_strategy_is_rejected_by_serde() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let r = app
        .patch_json_with_cookie(
            PROFILE,
            serde_json::json!({"strategy": "no_existe_esta_estrategia"}),
            &owner.cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::UNPROCESSABLE_ENTITY, "{r:?}");
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

/// El perfil es INPUT del motor (SWR, modo del objetivo, edad límite): toda escritura invalida.
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

/// **El upgrade desde 4.15.x no mueve un número.**
///
/// Se reproduce el estado exacto que deja la migración `20260902200000_…`: los cuatro ejes
/// copiados al perfil del usuario y retirados del JSONB de la instalación. Con un SWR distinto
/// del default, el objetivo publicado tiene que ser el que producía `fire_settings` — si el
/// handler se hubiera quedado leyendo el eje del sitio viejo (o del default), el objetivo saldría
/// dimensionado con 3,5 % en vez de con el 2 % que esa persona había configurado, y la única
/// señal sería que la jubilación se va años.
#[tokio::test]
async fn an_installation_upgraded_from_4_15_keeps_its_fire_target() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;

    // Un gasto de jubilación con el que el objetivo existe (modo `annual_expense`, sin impuestos
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
        b["jubilacion_target_net_worth"], "600000.0000",
        "el objetivo debe salir del SWR del PERFIL (2 %), no del default 3,5 %: {b}"
    );
    assert_eq!(b["horizon_lifespan_age"], 90, "{b}");

    // Y con el SWR de vuelta a 3,5 el objetivo se mueve: la cifra de arriba no es una casualidad.
    let r = app
        .patch_json_with_cookie(PROFILE, serde_json::json!({"swr_pct": "3.5"}), &owner.cookie)
        .await;
    assert_eq!(r.status, http::StatusCode::OK, "{r:?}");
    let b = app
        .get_with_cookie("/v1/projection/series", &owner.cookie)
        .await
        .json();
    assert_ne!(b["jubilacion_target_net_worth"], "600000.0000", "{b}");
}

/// **La elección ALMACENADA de `target_basis` viaja aparte de la resuelta** (deuda de contrato
/// detectada por WP7-2 y cerrada en WP5-2).
///
/// El fallo que cierra: `profile.target_basis` sale SIEMPRE resuelto, así que un formulario que
/// lee el perfil y lo reescribe entero persistía el `perpetuity` DERIVADO de no tener pensión
/// como si el usuario lo hubiera elegido. Al declarar la pensión después, la base se quedaba en
/// perpetuidad —la opción conservadora que nadie pidió— sin un solo aviso. Con
/// `target_basis_stored: null` el cliente sabe que no hay elección y puede no mandar el campo.
#[tokio::test]
async fn the_stored_target_basis_travels_next_to_the_resolved_one() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;

    // 1) Sin tocar nada: resuelto `perpetuity`, almacenado `null` = «no elegido, se deriva».
    let b = app.get_with_cookie(PROFILE, &owner.cookie).await.json();
    assert_eq!(b["profile"]["target_basis"], "perpetuity", "{b}");
    assert!(b["target_basis_stored"].is_null(), "nadie ha elegido nada: {b}");

    // 2) Un PATCH que NO nombra `target_basis` no elige por el usuario, ni siquiera cuando toca
    //    otra cosa del mismo formulario.
    let r = app
        .patch_json_with_cookie(PROFILE, serde_json::json!({"swr_pct": "3"}), &owner.cookie)
        .await;
    assert_eq!(r.status, http::StatusCode::OK, "{r:?}");
    let b = r.json();
    assert!(b["target_basis_stored"].is_null(), "el PATCH no debe fijar la base: {b}");

    // 3) Declarar la pensión DESPUÉS voltea la base resuelta a `bridge_to_pension` (R6) — que es
    //    exactamente lo que la derivación congelada impedía.
    let r = app
        .patch_json_with_cookie(
            PROFILE,
            serde_json::json!({"pension": {"monthly_amount_today": "1200", "starts_at_age": 67}}),
            &owner.cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::OK, "{r:?}");
    let b = r.json();
    assert_eq!(b["profile"]["target_basis"], "bridge_to_pension", "{b}");
    assert!(b["target_basis_stored"].is_null(), "sigue derivada, no elegida: {b}");

    // 4) Elegirla explícitamente sí la fija, y entonces manda sobre la derivación.
    let r = app
        .patch_json_with_cookie(
            PROFILE,
            serde_json::json!({"target_basis": "perpetuity"}),
            &owner.cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::OK, "{r:?}");
    let b = r.json();
    assert_eq!(b["profile"]["target_basis"], "perpetuity", "{b}");
    assert_eq!(b["target_basis_stored"], "perpetuity", "elegida a mano: {b}");

    // 5) …y `null` explícito la suelta: se vuelve a derivar (y con pensión eso es el puente).
    let r = app
        .patch_json_with_cookie(
            PROFILE,
            serde_json::json!({"target_basis": null}),
            &owner.cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::OK, "{r:?}");
    let b = r.json();
    assert_eq!(b["profile"]["target_basis"], "bridge_to_pension", "{b}");
    assert!(b["target_basis_stored"].is_null(), "soltada: {b}");

    // El GET dice lo mismo que dijo el PATCH: una sola verdad, dos rutas.
    let g = app.get_with_cookie(PROFILE, &owner.cookie).await.json();
    assert_eq!(g["profile"]["target_basis"], "bridge_to_pension", "{g}");
    assert!(g["target_basis_stored"].is_null(), "{g}");
}

// ---------------------------------------------------------------------------------------------
// U4 — el porcentaje de retirada es ÚNICO
// ---------------------------------------------------------------------------------------------

/// **`withdrawal_rule.pct` omitido HEREDA `swr_pct`, y el perfil dice de dónde salió.**
///
/// Decisión del owner (U4): el usuario declara UN porcentaje de retirada. `swr_pct` dimensiona
/// el objetivo FIRE y es a la vez el % de las reglas basadas en saldo; la SPA deja de mandar
/// `pct`. Lo que este test fija es que la ausencia no es un error ni un cero, sino una herencia
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

/// **El `end_pct` del hybrid se compara contra el `start_pct` RESUELTO.**
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
    assert_eq!(r.json()["code"], "hybrid_end_pct_not_below_start", "{r:?}");

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

/// **S4 — quitar la pensión suelta el `target_basis` almacenado.**
///
/// Medido en vivo antes del arreglo: tras `PATCH {"pension": null}` el perfil seguía devolviendo
/// `target_basis_stored: "bridge_to_pension"` y resolvía `bridge_to_pension`, mientras
/// `/v1/projection/series` publicaba `target_basis: null` (perpetuidad). Dos superficies
/// contando dos planes distintos sobre los mismos datos, sin un campo que lo delatara.
#[tokio::test]
async fn removing_the_pension_releases_the_stored_target_basis() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;

    // Pensión declarada + base fijada a mano al puente.
    let r = app
        .patch_json_with_cookie(
            PROFILE,
            serde_json::json!({
                "pension": {"monthly_amount_today": "1200", "starts_at_age": 67},
                "target_basis": "bridge_to_pension"
            }),
            &owner.cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::OK, "{r:?}");
    assert_eq!(r.json()["target_basis_stored"], "bridge_to_pension", "{r:?}");

    // Quitarla suelta la base: vuelve a derivarse, y sin pensión eso es la perpetuidad.
    let r = app
        .patch_json_with_cookie(PROFILE, serde_json::json!({"pension": null}), &owner.cookie)
        .await;
    assert_eq!(r.status, http::StatusCode::OK, "{r:?}");
    let b = r.json();
    assert!(b["profile"]["pension"].is_null(), "{b}");
    assert!(
        b["target_basis_stored"].is_null(),
        "la base debe soltarse al quitar la pensión: {b}"
    );
    assert_eq!(b["profile"]["target_basis"], "perpetuity", "{b}");

    // Y el GET dice lo mismo: la soltura se PERSISTIÓ, no es un adorno de la respuesta.
    let g = app.get_with_cookie(PROFILE, &owner.cookie).await.json();
    assert!(g["target_basis_stored"].is_null(), "{g}");
    assert_eq!(g["profile"]["target_basis"], "perpetuity", "{g}");
    let stored: serde_json::Value =
        sqlx::query_scalar("SELECT retirement_profile FROM users WHERE id = $1")
            .bind(owner.user_id)
            .fetch_one(&app.pool)
            .await
            .expect("select profile");
    assert!(
        stored["target_basis"].is_null(),
        "el JSONB no debe conservar la base fijada: {stored}"
    );

    // Elegir la base en el MISMO patch que quita la pensión GANA: quien dice las dos cosas está
    // eligiendo, no arrastrando un valor viejo.
    let r = app
        .patch_json_with_cookie(
            PROFILE,
            serde_json::json!({
                "pension": {"monthly_amount_today": "1200", "starts_at_age": 67}
            }),
            &owner.cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::OK, "{r:?}");
    let r = app
        .patch_json_with_cookie(
            PROFILE,
            serde_json::json!({"pension": null, "target_basis": "perpetuity"}),
            &owner.cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::OK, "{r:?}");
    assert_eq!(r.json()["target_basis_stored"], "perpetuity", "{r:?}");
}
