pub mod allocation_rules;
pub mod api_tokens;
pub mod assets;
pub mod auth;
pub mod backup_user;
pub mod budget;
pub mod categories;
pub mod changes;
pub mod fallback;
pub mod frame;
pub mod ha_sso;
pub mod health;
pub mod history;
pub mod installation;
pub mod liabilities;
pub mod members;
pub mod membership;
pub mod oauth_consent;
pub mod pending_users;
pub mod person_view;
pub mod planning;
pub mod projection;
pub mod session;
pub mod spa;
pub mod sso;
pub mod summary;
pub mod transactions;

/// Cota superior de las fechas FUTURAS que el usuario puede fijar a mano
/// (`planning_flows.due_date`, `liabilities.payment_end_date`): hoy + 100 años.
///
/// Existe porque `due_date: "9999-12-31"` se aceptaba y entraba tal cual en
/// `upcoming_outflows_total` / `upcoming_coverage_ratio` de `GET /v1/summary`: un flujo a ocho mil
/// años vista movía una cifra de portada sin ningún aviso. Se validaba el FORMATO de la fecha,
/// nunca su rango. Un modelo se equivoca con fechas relativas mucho más que con importes («dentro
/// de dos años» mal resuelto, un año tecleado con un dígito de más) y aquí no había barrera.
///
/// 100 años es deliberadamente generoso: el horizonte de proyección tope del motor son 1.200 meses,
/// así que nada por encima de esta cota puede afectar a ninguna serie — solo a los agregados.
///
/// NOTA para quien añada un caso: el código de error debe ir como **literal completo** en el sitio
/// de la llamada (`"due_date_out_of_range: …"`), nunca compuesto con `format!`. `error_codes_parity`
/// extrae los códigos de los literales del fuente: uno compuesto es invisible para el catálogo y
/// degrada en silencio al mensaje genérico de la SPA.
pub(crate) fn max_user_settable_future_date(today: chrono::NaiveDate) -> chrono::NaiveDate {
    today
        .checked_add_months(chrono::Months::new(1200))
        .unwrap_or(chrono::NaiveDate::MAX)
}

/// Un `window_months` fuera de rango se **rechaza**, no se clampa (4.4.0). Lo comparten las tres
/// ventanas del producto: `/v1/history/series` (1..=1200), `/v1/history/cashflow` (1..=120) y
/// `/v1/transactions/category-series` (1..=60), con sus tools MCP hermanas.
///
/// Hasta 4.3.1 las tres hacían `clamp(1, MAX)` y devolvían 200. El problema no es que el número
/// resultante sea otro: es que la respuesta ECOA `window_months` (o su rejilla) y describe una
/// ventana **distinta de la pedida** sin decirlo, así que quien pidió 500 meses lee 120 puntos
/// como si fueran los 500 que existen. El JSON Schema de las tools ya declaraba el rango
/// (`range(min = 1, max = …)`); esto es cumplirlo en vez de contestar otra pregunta.
///
/// Un único código para las tres: el manejo del cliente es idéntico y el mensaje lleva la cota
/// real. El prefijo va como literal (solo se interpola la cota) para que `error_codes_parity`
/// siga viendo el código — misma regla que la nota de `max_user_settable_future_date`.
pub(crate) fn validate_window_months(
    window_months: Option<i64>,
    max: i64,
) -> Result<(), crate::error::ApiError> {
    if let Some(w) = window_months {
        if !(1..=max).contains(&w) {
            return Err(crate::error::ApiError::BadRequest(format!(
                "window_months_out_of_range: window_months must be between 1 and {max}"
            )));
        }
    }
    Ok(())
}

/// Tri-estado de un campo de PATCH: **clave ausente** (`None`), **`null` explícito**
/// (`Some(None)`) y **valor** (`Some(Some(v))`). Es la única forma de que un cuerpo de PATCH pueda
/// decir «borra este campo» sin inventar un flag paralelo.
///
/// **El gotcha de serde que hace falta conocer**: la implementación por defecto de `Option<T>`
/// **colapsa las dos primeras en `None`** — un `null` presente es indistinguible de no mandar la
/// clave. Declarar el campo `Option<Option<T>>` con solo `#[serde(default)]` NO lo arregla: serde
/// sigue resolviendo el `Option` externo con esa misma implementación, el `null` sigue cayendo en
/// `None`, y la rama `Some(None)` queda como **código muerto que compila y se lee como
/// implementado**. Solo este `deserialize_with` lleva el `null` al `Some(None)`, porque
/// deserializa el `Option` **interno** y envuelve el resultado en `Some` incondicionalmente: si
/// serde llama a esta función es que la clave estaba.
///
/// INCIDENTE (issue #95, 4.4.2): dos campos prometían en su doc-comment —que viaja a OpenAPI, así
/// que es contrato **publicado**— que `null` borra el valor almacenado
/// (`PatchAssetBody.purchase_price`, `PatchInstallationBody.fire_settings`). Los dos tenían la
/// rama de borrado escrita y ninguno podía alcanzarla; peor, el `null` caía en el guard
/// `patch_empty` y la respuesta era un 400 diciendo que no habías mandado ningún campo, justo
/// cuando habías mandado el único que la doc documentaba para borrar. `fire_settings` ya estaba
/// tecleado `Option<Option<…>>` desde el principio: al autor solo le faltó este atributo, y nada
/// —ni el compilador, ni un test, ni el schema— avisó en años.
///
/// Doctrina **D14-adyacente**: el contrato lo fija el tipo, no la prosa que lo acompaña. Úsalo
/// SIEMPRE en pareja, `#[serde(default, deserialize_with = "…")]` — el `default` es lo que produce
/// el `None` de «clave ausente», y sin él un PATCH que omite el campo falla en vez de conservarlo.
/// Y recuerda que el guard `patch_empty` del handler debe seguir usando `.is_none()` sobre el
/// `Option` **externo**: `Some(None)` es un campo presente y un PATCH cuyo único contenido es
/// `{"campo": null}` es válido y borra.
///
/// Nota de superficie: MCP **no** converge aquí. Sus tools conservan los flags `clear_*`
/// (`clear_purchase_price`, `clear_cap`, `clear_due_date`) porque el tri-estado no es expresable
/// en JSON Schema — doctrina de la Fase 2, intacta.
pub(crate) fn deserialize_double_option<'de, D, T>(de: D) -> Result<Option<Option<T>>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: serde::Deserialize<'de>,
{
    <Option<T> as serde::Deserialize>::deserialize(de).map(Some)
}
