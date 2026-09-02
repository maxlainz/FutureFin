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
pub mod retirement_profile;
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

/// Hace ALCANZABLE el `null` presente en un cuerpo de PATCH (issues #95/#113).
///
/// La impl estándar de serde para `Option<T>` colapsa `"campo": null` con «clave ausente»
/// (ambos llegan como `None`), así que toda rama `Value::Null` escrita tras un
/// `Option<serde_json::Value>` plano es código muerto: el contrato publicado prometía «`null`
/// borra» y el binario devolvía 200 sin efecto. Con
/// `#[serde(default, deserialize_with = "crate::handlers::deserialize_double_option")]`:
/// clave ausente → `None` (por el `default`), `"campo": null` → `Some(Value::Null)` (la rama
/// revive), valor → `Some(valor)`.
///
/// El tri-estado NO es expresable en JSON Schema, así que las tools MCP siguen con sus flags
/// `clear_*` explícitos (doctrina Fase 2 del MCP); este helper es solo para el wire HTTP.
pub(crate) fn deserialize_double_option<'de, D>(
    de: D,
) -> Result<Option<serde_json::Value>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    <serde_json::Value as serde::Deserialize>::deserialize(de).map(Some)
}

/// Variante tipada de [`deserialize_double_option`] para campos `Option<Option<T>>` (p. ej.
/// `fire_settings`): ausente → `None`, `"campo": null` → `Some(None)`, valor → `Some(Some(v))`.
/// Sin este deserializador, serde colapsa el `null` presente en el `None` exterior y la rama
/// `Some(None) => …` del handler es código muerto — el mismo bug de #95/#113 en su forma tipada.
pub(crate) fn deserialize_double_option_typed<'de, D, T>(
    de: D,
) -> Result<Option<Option<T>>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: serde::Deserialize<'de>,
{
    <Option<T> as serde::Deserialize>::deserialize(de).map(Some)
}
