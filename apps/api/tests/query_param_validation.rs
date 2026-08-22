//! REGRESIÓN — los enums de query que caían al default en silencio (issues #7 §4, 4.0.0).
//!
//! Hasta 4.0.0 tres parámetros se parseaban con un brazo comodín: `view` (todo el ledger),
//! `resolution` (cash-flow) y `density` (proyección). Un valor desconocido no daba error: daba
//! **el default**, y en el caso de `view` el default es el **hogar entero**.
//!
//! Con la SPA como único cliente eso nunca se notó — nunca manda otra cosa que `mine` o nada —,
//! pero un agente MCP que escribe `"MINE"` recibía los datos de todos los miembros creyendo haber
//! pedido solo los suyos. No es una frontera de autorización (D2: cualquier miembro puede pedir
//! `household` a la cara), pero sí una respuesta sobre otra población que la pedida, sin ninguna
//! señal. Estos tests fijan que ahora se rechaza.

mod common;

use common::TestApp;
use http::StatusCode;

/// Rutas que aceptan `?view=`. Basta una por familia de handler: el parseo es compartido
/// (`LedgerViewQuery::resolve`), así que lo que se prueba es que **todas** lo usen — no que cada
/// una reimplemente la validación. `projection/series` está en la lista precisamente porque tenía
/// su propia copia del `match` y por eso se le pasó por alto.
const VIEW_ROUTES: &[&str] = &[
    "/v1/summary",
    "/v1/assets",
    "/v1/liabilities",
    "/v1/budget",
    "/v1/planning/flows",
    "/v1/allocation-rules",
    "/v1/allocation-rules/resolution",
    "/v1/projection/series",
    "/v1/history/series",
    "/v1/history/cashflow",
    "/v1/transactions",
    "/v1/transactions/summary",
    "/v1/transactions/months",
    "/v1/transactions/imports",
];

/// `{error, code, message}` de una respuesta de error; falla con el cuerpo entero si no cuadra.
fn assert_bad_request(resp: &common::ResponseParts, code: &str, ctx: &str) {
    assert_eq!(resp.status, StatusCode::BAD_REQUEST, "{ctx}: {resp:?}");
    let body = resp.json();
    assert_eq!(body["code"], code, "{ctx}: código inesperado en {body}");
    assert_eq!(body["error"], "bad_request", "{ctx}: clase inesperada en {body}");
}

#[tokio::test]
async fn unknown_view_is_rejected_on_every_ledger_route() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("view_owner").await;

    for route in VIEW_ROUTES {
        // El caso exacto del issue: mayúsculas. Devolvía 200 con el hogar completo.
        let resp = app
            .get_with_cookie(&format!("{route}?view=MINE"), &owner.cookie)
            .await;
        assert_bad_request(&resp, "invalid_view", &format!("{route} ?view=MINE"));

        let resp = app
            .get_with_cookie(&format!("{route}?view=no-existe-esta-vista"), &owner.cookie)
            .await;
        assert_bad_request(&resp, "invalid_view", &format!("{route} ?view=desconocida"));
    }
}

#[tokio::test]
async fn known_views_and_absence_still_work_on_every_ledger_route() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("view_ok_owner").await;

    // `household` explícito nunca fue documentado como «cualquier cosa vale»: es un valor válido
    // del contrato y tiene que seguir funcionando ahora que el comodín ya no lo cubre.
    for qs in ["", "?view=mine", "?view=household", "?view=%20mine%20"] {
        for route in VIEW_ROUTES {
            let resp = app
                .get_with_cookie(&format!("{route}{qs}"), &owner.cookie)
                .await;
            assert_eq!(
                resp.status,
                StatusCode::OK,
                "{route}{qs} debería seguir sirviendo: {resp:?}"
            );
        }
    }
}

#[tokio::test]
async fn unknown_cashflow_resolution_is_rejected() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("res_owner").await;

    // `resolution` se ECOA en la respuesta, así que el fallo era doblemente engañoso: pedías
    // "hourly" y el payload te contestaba `"resolution":"weekly"` con un 200.
    let resp = app
        .get_with_cookie("/v1/history/cashflow?resolution=hourly", &owner.cookie)
        .await;
    assert_bad_request(&resp, "invalid_resolution", "resolution=hourly");

    let resp = app
        .get_with_cookie("/v1/history/cashflow?resolution=DAILY", &owner.cookie)
        .await;
    assert_bad_request(&resp, "invalid_resolution", "resolution=DAILY");

    // El eco de `resolution` vive dentro de `fine`, que solo aparece con transacciones vinculadas
    // a un activo; aquí basta con que los tres valores válidos sigan sirviendo. La cobertura del
    // eco y de `daily_window_too_large` ya está en `history_cashflow.rs`.
    for qs in ["", "?resolution=weekly", "?resolution=daily&window_months=1"] {
        let resp = app
            .get_with_cookie(&format!("/v1/history/cashflow{qs}"), &owner.cookie)
            .await;
        assert_eq!(resp.status, StatusCode::OK, "cashflow{qs}: {resp:?}");
    }
}

#[tokio::test]
async fn unknown_projection_density_is_rejected() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("dens_owner").await;

    let resp = app
        .get_with_cookie("/v1/projection/series?density=hourly", &owner.cookie)
        .await;
    assert_bad_request(&resp, "invalid_density", "density=hourly");

    // `hybrid` sirve ~82 puntos y `monthly` unos 841: caer al default en silencio multiplicaba
    // por 10 el payload de quien pidió lo contrario.
    let monthly = app
        .get_with_cookie("/v1/projection/series?density=monthly", &owner.cookie)
        .await;
    assert_eq!(monthly.status, StatusCode::OK, "density=monthly: {monthly:?}");
    let hybrid = app
        .get_with_cookie("/v1/projection/series?density=hybrid", &owner.cookie)
        .await;
    assert_eq!(hybrid.status, StatusCode::OK, "density=hybrid: {hybrid:?}");

    let n_monthly = monthly.json()["points"].as_array().expect("points").len();
    let n_hybrid = hybrid.json()["points"].as_array().expect("points").len();
    assert!(
        n_hybrid < n_monthly,
        "hybrid ({n_hybrid}) debería traer menos puntos que monthly ({n_monthly})"
    );
}
