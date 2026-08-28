pub mod allocation_rules;
pub mod api_tokens;
pub mod assets;
pub mod auth;
pub mod backup_user;
pub mod budget;
pub mod categories;
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
