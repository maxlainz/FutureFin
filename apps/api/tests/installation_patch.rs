//! Fase 3.3 — `FireNumberMode` debe rechazar variantes desconocidas (no silenciar a default).
//! Fase 3.4 (compat) — el alias `annual_expense_adjusted` se conserva para importar backups antiguos.

mod common;

use common::TestApp;

#[tokio::test]
async fn patch_installation_unknown_fire_number_mode_returns_client_error() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;

    let resp = app
        .patch_json_with_cookie(
            "/v1/installation",
            serde_json::json!({
                "fire_settings": {
                    "fire_number_mode": "foobar",
                    "swr_pct": "3.5",
                    "taxes_enabled": false,
                    "tax_brackets": [],
                    "fire_number_manual_amount": null,
                    "fire_number_expense_adjustment_pct": null,
                }
            }),
            &owner.cookie,
        )
        .await;
    // Axum devuelve 422 (Unprocessable Entity) cuando el JSON no se puede deserializar al tipo
    // target (variante desconocida). Antes el handler aceptaba 200 silenciosamente con default.
    assert_eq!(
        resp.status,
        http::StatusCode::UNPROCESSABLE_ENTITY,
        "modo desconocido debe ser rechazado (422), recibido {}",
        resp.status
    );
    let body_text = String::from_utf8_lossy(&resp.body);
    assert!(
        body_text.contains("unknown variant") && body_text.contains("foobar"),
        "cuerpo debe nombrar la variante desconocida: {body_text}"
    );
}

#[tokio::test]
async fn patch_installation_accepts_legacy_annual_expense_adjusted_alias() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("bob").await;

    // El alias se mapea silenciosamente al modo actual (`annual_expense`). Soporte a backups antiguos.
    let resp = app
        .patch_json_with_cookie(
            "/v1/installation",
            serde_json::json!({
                "fire_settings": {
                    "fire_number_mode": "annual_expense_adjusted",
                    "swr_pct": "3.5",
                    "taxes_enabled": false,
                    "tax_brackets": [],
                    "fire_number_manual_amount": null,
                    "fire_number_expense_adjustment_pct": null,
                }
            }),
            &owner.cookie,
        )
        .await;
    assert_eq!(resp.status, http::StatusCode::OK, "alias debe ser aceptado: {resp:?}");
    let body = resp.json();
    assert_eq!(body["installation"]["fire_settings"]["fire_number_mode"], "annual_expense");
}

#[tokio::test]
async fn patch_installation_accepts_valid_fire_mode_change() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("carol").await;

    let resp = app
        .patch_json_with_cookie(
            "/v1/installation",
            serde_json::json!({
                "fire_settings": {
                    "fire_number_mode": "current_income",
                    "swr_pct": "3.5",
                    "taxes_enabled": false,
                    "tax_brackets": [],
                    "fire_number_manual_amount": null,
                    "fire_number_expense_adjustment_pct": null,
                }
            }),
            &owner.cookie,
        )
        .await;
    assert_eq!(resp.status, http::StatusCode::OK, "{resp:?}");
    let body = resp.json();
    assert_eq!(body["installation"]["fire_settings"]["fire_number_mode"], "current_income");
}

#[tokio::test]
async fn mcp_write_enabled_defaults_true_and_owner_can_toggle() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;

    // Default TRUE en el snapshot desde el primer GET.
    let resp = app.get_with_cookie("/v1/installation", &owner.cookie).await;
    assert_eq!(resp.status, http::StatusCode::OK);
    assert_eq!(
        resp.json()["installation"]["mcp_write_enabled"], true,
        "default de la migración"
    );

    // El owner lo apaga; la respuesta del PATCH ya lo refleja.
    let resp = app
        .patch_json_with_cookie(
            "/v1/installation",
            serde_json::json!({"mcp_write_enabled": false}),
            &owner.cookie,
        )
        .await;
    assert_eq!(resp.status, http::StatusCode::OK, "{resp:?}");
    assert_eq!(resp.json()["installation"]["mcp_write_enabled"], false);

    // Y persiste (GET posterior).
    let resp = app.get_with_cookie("/v1/installation", &owner.cookie).await;
    assert_eq!(resp.json()["installation"]["mcp_write_enabled"], false);

    // Re-encender funciona igual.
    let resp = app
        .patch_json_with_cookie(
            "/v1/installation",
            serde_json::json!({"mcp_write_enabled": true}),
            &owner.cookie,
        )
        .await;
    assert_eq!(resp.json()["installation"]["mcp_write_enabled"], true);
}

#[tokio::test]
async fn mcp_write_enabled_patch_is_owner_only() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let member = app.register_and_approve_member(&owner, "bob", "member").await;

    let resp = app
        .patch_json_with_cookie(
            "/v1/installation",
            serde_json::json!({"mcp_write_enabled": false}),
            &member.cookie,
        )
        .await;
    assert_eq!(
        resp.status,
        http::StatusCode::FORBIDDEN,
        "el PATCH de instalación entero es owner-only: {resp:?}"
    );
}

/// #146 (Ola 5): el rango de la inflación pasa de [0, 50] a **[−2, 50]** — España tuvo IPC
/// anual medio negativo cinco veces este siglo y el suelo 0 impedía estresar el propio plan.
/// Tabla de frontera por HTTP (la cota nunca se había probado por esta ruta): −2 entra y se
/// LEE tal cual (sin clamp silencioso), −2,01 y 50,01 devuelven 400 `inflation_out_of_range`
/// con el rango nuevo en el mensaje, 50 sigue entrando.
#[tokio::test]
async fn inflation_bounds_are_minus_two_to_fifty() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;

    for (pct, ok) in [("-2", true), ("-2.01", false), ("50", true), ("50.01", false)] {
        let r = app
            .patch_json_with_cookie(
                "/v1/installation",
                serde_json::json!({ "annual_inflation_assumption_percent": pct }),
                &owner.cookie,
            )
            .await;
        if ok {
            assert_eq!(r.status, http::StatusCode::OK, "{pct}: {r:?}");
        } else {
            assert_eq!(r.status, http::StatusCode::BAD_REQUEST, "{pct}: {r:?}");
            let msg = r.json()["message"].as_str().unwrap().to_string();
            assert!(
                msg.contains("inflation_out_of_range") && msg.contains("between -2 and 50"),
                "{pct}: {msg}"
            );
        }
    }

    // El último PATCH válido fue 50; vuelve a −2 y comprueba el eco SIN clamp.
    let r = app
        .patch_json_with_cookie(
            "/v1/installation",
            serde_json::json!({ "annual_inflation_assumption_percent": "-2" }),
            &owner.cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::OK, "{r:?}");
    let g = app.get_with_cookie("/v1/installation", &owner.cookie).await.json();
    let echoed: f64 = g["installation"]["annual_inflation_assumption_percent"]
        .as_str()
        .unwrap()
        .parse()
        .unwrap();
    assert_eq!(echoed, -2.0, "el −2 almacenado se sirve tal cual, sin suelo a 0: {g}");
}

/// #146 (Ola 5): una instalación NUEVA nace asumiendo **2,5 %** de inflación — el default de la
/// columna cambió de 0 (el valor más optimista del rango, que solo el asistente saltable
/// corregía) al objetivo del BCE. Flujo CRUDO a propósito: el helper del arnés
/// (`register_and_login_owner`) normaliza la inflación a 0 para los pins históricos, así que
/// este test registra y lee sin pasar por él. Las instalaciones EXISTENTES no se tocan (la
/// migración solo cambia el DEFAULT; eso no es observable por HTTP y lo garantiza el SQL).
#[tokio::test]
async fn new_installations_are_born_assuming_two_and_a_half_percent() {
    let app = TestApp::spawn().await;
    let reg = app
        .post_json(
            "/v1/auth/register",
            serde_json::json!({
                "username": "cruda",
                "password": "correct horse battery staple",
                "birth_date": "1990-01-01",
            }),
        )
        .await;
    assert_eq!(reg.status, http::StatusCode::CREATED, "{reg:?}");
    let login = app
        .post_json(
            "/v1/auth/login",
            serde_json::json!({"username": "cruda", "password": "correct horse battery staple"}),
        )
        .await;
    assert_eq!(login.status, http::StatusCode::OK, "{login:?}");
    let cookie = login.session_cookie().expect("ff_session");

    let g = app.get_with_cookie("/v1/installation", &cookie).await.json();
    let pct: f64 = g["installation"]["annual_inflation_assumption_percent"]
        .as_str()
        .unwrap()
        .parse()
        .unwrap();
    assert_eq!(pct, 2.5, "el default de una instalación nueva es 2,5: {g}");
}
