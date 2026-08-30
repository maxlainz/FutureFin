//! Fase 3.3 — `FireNumberMode` debe rechazar variantes desconocidas (no silenciar a default).
//! Fase 3.4 (compat) — el alias `annual_expense_adjusted` se conserva para importar backups antiguos.
//! 4.4.2 (issue #95) — `fire_settings` es un tri-estado de verdad: `null` borra el JSON almacenado.

mod common;

use common::{LoggedInOwner, TestApp};
use serde_json::json;

/// Un `fire_settings` completo y **claramente distinto de los defaults** en los tres ejes que el
/// test mira después: modo, SWR e impuestos. Sirve para que «se ha borrado» no se confunda con
/// «se ha guardado una copia de los defaults».
fn non_default_fire_settings() -> serde_json::Value {
    json!({
        "fire_number_mode": "current_income",
        "swr_pct": "2.0",
        "taxes_enabled": false,
        "tax_brackets": [],
        "fire_number_manual_amount": null,
        "fire_number_expense_adjustment_pct": null,
    })
}

/// El JSON crudo de la columna, que es la única forma de distinguir «NULL, defaults al leer» de
/// «hay una fila guardada que resulta ser igual a los defaults»: `resolve_fire_settings` las
/// devuelve idénticas al cliente.
async fn stored_fire_settings_json(app: &TestApp) -> Option<serde_json::Value> {
    sqlx::query_scalar::<_, Option<serde_json::Value>>(
        r#"SELECT fire_settings FROM installation LIMIT 1"#,
    )
    .fetch_one(&app.pool)
    .await
    .expect("la fila de installation existe")
}

async fn patch_ok(app: &TestApp, owner: &LoggedInOwner, body: serde_json::Value) {
    let r = app
        .patch_json_with_cookie("/v1/installation", body, &owner.cookie)
        .await;
    assert_eq!(r.status, http::StatusCode::OK, "{r:?}");
}

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

/// El trío del tri-estado de `PATCH /v1/installation` sobre `fire_settings`: **`null` borra**,
/// **clave ausente conserva**, **valor aplica**.
///
/// El `null` va SOLO en el cuerpo a propósito: hasta 4.4.2 (issue #95) ese cuerpo exacto salía por
/// el 400 `patch_empty` —«no has mandado ningún campo»— justo cuando habías mandado el único que
/// el doc-comment (y por tanto OpenAPI) documentaba para borrar. El tipo ya era
/// `Option<Option<FireSettings>>` desde el principio; le faltaba el `deserialize_with`, así que la
/// rama `Some(None) => None` del handler era código muerto.
#[tokio::test]
async fn patch_fire_settings_is_a_real_tristate() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;

    // Punto de partida: sin PATCH nunca, la columna está a NULL y el cliente ve los defaults.
    assert_eq!(stored_fire_settings_json(&app).await, None);

    // 1. Valor → aplica, y la columna deja de estar a NULL.
    patch_ok(&app, &owner, json!({"fire_settings": non_default_fire_settings()})).await;
    let stored = stored_fire_settings_json(&app)
        .await
        .expect("tras guardar, la columna lleva JSON");
    assert_eq!(stored["fire_number_mode"], "current_income", "{stored}");
    let seen = app.get_with_cookie("/v1/installation", &owner.cookie).await.json();
    assert_eq!(seen["installation"]["fire_settings"]["swr_pct"], "2.0", "{seen}");

    // 2. Clave ausente → intacto (el PATCH toca otro eje).
    patch_ok(&app, &owner, json!({"show_age_mode": "ages"})).await;
    let seen = app.get_with_cookie("/v1/installation", &owner.cookie).await.json();
    assert_eq!(
        seen["installation"]["fire_settings"]["fire_number_mode"], "current_income",
        "omitir fire_settings no puede tocarlo: {seen}"
    );
    assert_eq!(seen["installation"]["show_age_mode"], "ages", "{seen}");

    // 3. `null` como ÚNICO campo → 200 (ya no `patch_empty`) y la columna vuelve a NULL.
    let r = app
        .patch_json_with_cookie("/v1/installation", json!({"fire_settings": null}), &owner.cookie)
        .await;
    assert_eq!(
        r.status,
        http::StatusCode::OK,
        "un PATCH cuyo único contenido es `fire_settings: null` es válido y borra: {r:?}"
    );
    assert_eq!(
        stored_fire_settings_json(&app).await,
        None,
        "borrar es dejar la columna a NULL, no guardar una copia de los defaults"
    );

    // Y en la lectura vuelven a aplicar los defaults, en la MISMA respuesta del PATCH y en el GET.
    let after = r.json();
    assert_eq!(after["installation"]["fire_settings"]["fire_number_mode"], "annual_expense", "{after}");
    assert_eq!(after["installation"]["fire_settings"]["swr_pct"], "3.5", "{after}");
    assert_eq!(after["installation"]["fire_settings"]["taxes_enabled"], true, "{after}");
    let seen = app.get_with_cookie("/v1/installation", &owner.cookie).await.json();
    assert_eq!(seen["installation"]["fire_settings"]["fire_number_mode"], "annual_expense", "{seen}");
    assert!(
        !seen["installation"]["fire_settings"]["tax_brackets"]
            .as_array()
            .expect("tax_brackets es un array")
            .is_empty(),
        "los tramos de los defaults vuelven; los `[]` guardados en el paso 1 se han ido: {seen}"
    );

    // El cuerpo VACÍO sigue siendo 400: el tri-estado abre `null`, no la puerta de atrás.
    let r = app
        .patch_json_with_cookie("/v1/installation", json!({}), &owner.cookie)
        .await;
    assert_eq!(r.status, http::StatusCode::BAD_REQUEST, "{r:?}");
    assert_eq!(r.json()["code"], "patch_empty");
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
