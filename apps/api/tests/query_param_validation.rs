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
///
/// No es la lista completa de rutas GET con `view`: `/v1/transactions/category-series` también lo
/// acepta pero exige además `kind` (obligatorio), así que no encaja en el bucle genérico de abajo
/// sin parámetros extra por ruta; queda fuera a propósito. Fuente de verdad de qué handler declara
/// el parámetro: `grep -rn '("view" = Option<String>, Query' apps/api/src/handlers/`.
const VIEW_ROUTES: &[&str] = &[
    "/v1/summary",
    "/v1/assets",
    "/v1/liabilities",
    "/v1/budget",
    "/v1/planning/flows",
    "/v1/allocation-rules",
    "/v1/allocation-rules/resolution",
    "/v1/allocation-rules/goals",
    "/v1/projection/series",
    "/v1/projection/bands",
    "/v1/history/series",
    "/v1/history/cashflow",
    "/v1/transactions",
    "/v1/transactions/aggregate",
    "/v1/transactions/duplicates",
    "/v1/transactions/summary",
    "/v1/transactions/months",
    "/v1/transactions/imports",
    "/v1/changes",
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
            // `/v1/changes` exige `since` (obligatorio, aparte de `view`); sin él el handler
            // devuelve 400 `date_required` antes incluso de llegar al resto — así que el bucle
            // genérico necesita añadírselo solo a esta ruta.
            let uri = if *route == "/v1/changes" {
                let sep = if qs.is_empty() { '?' } else { '&' };
                format!("{route}{qs}{sep}since=2020-01-01")
            } else {
                format!("{route}{qs}")
            };
            let resp = app.get_with_cookie(&uri, &owner.cookie).await;

            // `/v1/projection/bands` es la única ruta cuya vista `household` está documentada
            // como error propio (`household_bands_unavailable`): los percentiles p10/p50/p90 no
            // se suman entre miembros, así que `household` no es «sin implementar todavía», es
            // rechazado a propósito. El resto de rutas siguen sirviendo 200 con `household`.
            if *route == "/v1/projection/bands" && qs == "?view=household" {
                assert_bad_request(&resp, "household_bands_unavailable", &uri);
                continue;
            }

            assert_eq!(
                resp.status,
                StatusCode::OK,
                "{uri} debería seguir sirviendo: {resp:?}"
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

// ---------------------------------------------------------------------------
// Cotas numéricas: rechazo, no clamp silencioso (Fase 2, issue #83)
// ---------------------------------------------------------------------------

/// Misma clase de fallo que los enums de arriba, con otra cara. Cuatro parámetros DECLARABAN su
/// rango en el JSON Schema de su tool (`range(min, max)`) y luego el handler **clampaba**: pedir
/// 1.200 meses de proyección devolvía 840 puntos etiquetados `horizon_basis: "months_override"`
/// —«te he hecho caso»— y pedir 500 meses de cash-flow devolvía 120. La respuesta describe una
/// pregunta distinta de la que se hizo, y nada en ella lo dice.
///
/// Lo más caro era la discrepancia entre hermanas: `get_projection` y `simulate_projection`
/// declaran el MISMO rango 12–840 y contestaban distinto al mismo valor (una clampaba, la otra
/// devolvía `months_out_of_range`). Ahora las cuatro rechazan, que es lo que el esquema ya decía.
///
/// La SPA no envía ninguno de estos parámetros fuera de rango: `projectionSeriesUrl` no manda
/// `months` nunca, y las dos llamadas de cash-flow son `window_months=24` y `window_months=6`.
#[tokio::test]
async fn out_of_range_numeric_windows_are_rejected_not_clamped() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("bounds_owner").await;

    // (ruta, código esperado) — un caso por debajo y otro por encima de cada cota.
    for (uri, code) in [
        // `?months=` de la proyección: 12–840. Comparte código y mensaje con
        // `simulate_projection.months`, que ya rechazaba (helper `validate_months_override`).
        ("/v1/projection/series?months=11", "months_out_of_range"),
        ("/v1/projection/series?months=841", "months_out_of_range"),
        // Serie histórica: 1–1200.
        ("/v1/history/series?window_months=0", "window_months_out_of_range"),
        ("/v1/history/series?window_months=1201", "window_months_out_of_range"),
        // Cash-flow: 1–120.
        ("/v1/history/cashflow?window_months=0", "window_months_out_of_range"),
        ("/v1/history/cashflow?window_months=500", "window_months_out_of_range"),
        // Serie mensual por categoría: 1–60.
        (
            "/v1/transactions/category-series?kind=expense&window_months=0",
            "window_months_out_of_range",
        ),
        (
            "/v1/transactions/category-series?kind=expense&window_months=61",
            "window_months_out_of_range",
        ),
    ] {
        let resp = app.get_with_cookie(uri, &owner.cookie).await;
        assert_bad_request(&resp, code, uri);
    }

    // Un valor NEGATIVO también (las ventanas de `history` son `i64`, así que llega al handler en
    // vez de morir en el parseo). Antes `clamp(1, MAX)` lo subía a 1 y devolvía 200.
    let resp = app
        .get_with_cookie("/v1/history/cashflow?window_months=-3", &owner.cookie)
        .await;
    assert_bad_request(&resp, "window_months_out_of_range", "cashflow window_months=-3");
}

/// Y los extremos EXACTOS del rango siguen siendo válidos: el rechazo es de lo que está fuera, no
/// un off-by-one que se come el borde. (`months=840` recomputa la proyección entera, así que solo
/// se prueba el borde inferior de la proyección; las ventanas son baratas en ambos bordes.)
#[tokio::test]
async fn the_exact_bounds_of_every_numeric_window_still_work() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("bounds_ok_owner").await;

    for uri in [
        "/v1/projection/series?months=12",
        "/v1/history/series?window_months=1",
        "/v1/history/series?window_months=1200",
        "/v1/history/cashflow?window_months=1",
        "/v1/history/cashflow?window_months=120",
        "/v1/transactions/category-series?kind=expense&window_months=1",
        "/v1/transactions/category-series?kind=expense&window_months=60",
        // Omitirlo sigue cayendo al default, que es lo que hace la SPA.
        "/v1/history/cashflow",
        "/v1/transactions/category-series?kind=expense",
    ] {
        let resp = app.get_with_cookie(uri, &owner.cookie).await;
        assert_eq!(resp.status, StatusCode::OK, "{uri}: {resp:?}");
    }
}
