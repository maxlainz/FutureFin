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
