use crate::error::ApiError;
use crate::handlers::membership::MembershipRole;
use crate::handlers::projection::refresh_projection_after_mutation;
use crate::handlers::session::require_session_user;
use crate::state::AppState;
use axum::extract::Extension;
use axum::Json;
use axum_extra::extract::cookie::CookieJar;
use chrono::{DateTime, NaiveDate, Utc};
use chrono_tz::Tz;
use rust_decimal::Decimal;
use serde::{Deserialize, Deserializer, Serialize};
use std::str::FromStr;
use sqlx::{PgPool, Postgres, Transaction};
use sqlx::types::Json as SqlxJson;
use std::sync::Arc;
use utoipa::ToSchema;
use uuid::Uuid;

/// Modo de cálculo del FIRE number (presupuesto regular sin cuotas derivadas de pasivos × 12 como base en modos automáticos).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ToSchema, Default)]
#[serde(rename_all = "snake_case")]
pub enum FireNumberMode {
    Manual,
    #[default]
    AnnualExpense,
    CurrentIncome,
}

impl<'de> Deserialize<'de> for FireNumberMode {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        use serde::de::Error;
        let s = String::deserialize(deserializer)?;
        match s.as_str() {
            "manual" => Ok(Self::Manual),
            "annual_expense" => Ok(Self::AnnualExpense),
            // Alias preservado para importar backups antiguos cuyo schema usaba este modo.
            "annual_expense_adjusted" => Ok(Self::AnnualExpense),
            "current_income" => Ok(Self::CurrentIncome),
            _ => Err(D::Error::unknown_variant(
                &s,
                &["manual", "annual_expense", "current_income"],
            )),
        }
    }
}

/// Fuente del ahorro mensual de la simulación:
/// - `budget` (modo A, default): income y gasto del presupuesto.
/// - `transactions_avg` (modo B): promedio real 12m de las transacciones para income y gasto.
/// - `budget_income_real_expense` (modo C): income del presupuesto + gasto real (mismo promedio
///   ponderado 12m que el modo B). Útil con nómina estable pero gasto que se quiere medir de verdad.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ToSchema, Default)]
#[serde(rename_all = "snake_case")]
pub enum SavingsSource {
    #[default]
    Budget,
    TransactionsAvg,
    BudgetIncomeRealExpense,
}

impl SavingsSource {
    /// `true` para los modos cuyo ahorro deriva de las transacciones (promedio real 12m): modo B
    /// (`transactions_avg`) y modo C (`budget_income_real_expense`). Punto ÚNICO del gate compartido
    /// por la proyección, `/v1/summary` y la invalidación de cache de las mutaciones de transacciones.
    pub(crate) fn uses_transactions(self) -> bool {
        matches!(self, Self::TransactionsAvg | Self::BudgetIncomeRealExpense)
    }
}

impl<'de> Deserialize<'de> for SavingsSource {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        use serde::de::Error;
        let s = String::deserialize(deserializer)?;
        match s.as_str() {
            "budget" => Ok(Self::Budget),
            "transactions_avg" => Ok(Self::TransactionsAvg),
            "budget_income_real_expense" => Ok(Self::BudgetIncomeRealExpense),
            _ => Err(D::Error::unknown_variant(
                &s,
                &["budget", "transactions_avg", "budget_income_real_expense"],
            )),
        }
    }
}

/// Semántica de la ventana del promedio real de transacciones.
///
/// NO confundir con `AvgWindow` (`handlers/transactions/summary.rs`), que es el tramo POR REQUEST
/// de la comparativa de la pestaña Movimientos: ejes distintos, uno es configuración de la
/// simulación y el otro un query param.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ToSchema, Default)]
#[serde(rename_all = "snake_case")]
pub enum AvgWindowMode {
    /// Los N meses CON DATOS más recientes, saltando los vacíos. Garantiza N observaciones aunque
    /// haya huecos; a cambio puede alcanzar meses lejanos (la UI publica el rango real usado).
    Data,
    /// Solo los meses con datos dentro de los últimos N meses CIVILES. Horizonte acotado; puede
    /// devolver menos de N meses, o ninguno (→ ese lado cae al presupuesto).
    #[default]
    Calendar,
}

impl<'de> Deserialize<'de> for AvgWindowMode {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        use serde::de::Error;
        let s = String::deserialize(deserializer)?;
        match s.as_str() {
            "data" => Ok(Self::Data),
            "calendar" => Ok(Self::Calendar),
            _ => Err(D::Error::unknown_variant(&s, &["data", "calendar"])),
        }
    }
}

/// Cotas de las ventanas del promedio (meses). El suelo es 1 — una ventana de 0 meses dejaría el
/// promedio sin denominador y caería al presupuesto con la UI diciendo lo contrario.
pub const MIN_AVG_WINDOW_MONTHS: u32 = 1;
pub const MAX_AVG_WINDOW_MONTHS: u32 = 60;

/// Ventana ya resuelta y CLAMPADA de un lado del promedio.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AvgWindowSpec {
    pub months: u32,
    pub mode: AvgWindowMode,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct TaxBracket {
    #[serde(with = "rust_decimal::serde::str_option")]
    #[schema(value_type = Option<String>)]
    pub up_to: Option<Decimal>,
    #[serde(with = "rust_decimal::serde::str")]
    #[schema(value_type = String)]
    pub pct: Decimal,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(default)]
pub struct FireSettings {
    pub fire_number_mode: FireNumberMode,
    #[serde(with = "rust_decimal::serde::str_option")]
    #[schema(value_type = Option<String>)]
    pub fire_number_manual_amount: Option<Decimal>,
    #[serde(with = "rust_decimal::serde::str")]
    #[schema(value_type = String)]
    pub swr_pct: Decimal,
    pub taxes_enabled: bool,
    pub tax_brackets: Vec<TaxBracket>,
    /// Fuente del ahorro de la simulación. Ausente en JSON → `budget` (cubierto por el
    /// `#[serde(default)]` a nivel struct, que rellena los campos faltantes desde `default_fire_settings`).
    pub savings_source: SavingsSource,
    /// Ventana del promedio de INGRESO, en meses. Solo la usa el modo B (`transactions_avg`); el
    /// modo C toma el ingreso del presupuesto y el modo A no promedia nada.
    pub income_avg_window_months: u32,
    /// Semántica de la ventana de ingreso.
    pub income_avg_window_mode: AvgWindowMode,
    /// Ventana del promedio de GASTO, en meses. La usan los modos B y C.
    pub expense_avg_window_months: u32,
    /// Semántica de la ventana de gasto.
    pub expense_avg_window_mode: AvgWindowMode,
}

impl FireSettings {
    /// Ventana de ingreso ya clampada. Punto ÚNICO: nadie construye un `AvgWindowSpec` a mano.
    pub(crate) fn income_window(&self) -> AvgWindowSpec {
        AvgWindowSpec {
            months: self
                .income_avg_window_months
                .clamp(MIN_AVG_WINDOW_MONTHS, MAX_AVG_WINDOW_MONTHS),
            mode: self.income_avg_window_mode,
        }
    }
    /// Ventana de gasto ya clampada.
    pub(crate) fn expense_window(&self) -> AvgWindowSpec {
        AvgWindowSpec {
            months: self
                .expense_avg_window_months
                .clamp(MIN_AVG_WINDOW_MONTHS, MAX_AVG_WINDOW_MONTHS),
            mode: self.expense_avg_window_mode,
        }
    }
}

impl Default for FireSettings {
    fn default() -> Self {
        default_fire_settings()
    }
}

pub(crate) fn default_fire_settings() -> FireSettings {
    FireSettings {
        fire_number_mode: FireNumberMode::AnnualExpense,
        fire_number_manual_amount: None,
        swr_pct: Decimal::new(35, 1),
        taxes_enabled: true,
        tax_brackets: default_es_tax_brackets(),
        savings_source: SavingsSource::Budget,
        // Ingreso corto (captura una subida de sueldo sin esperar un año), gasto largo (suaviza
        // los picos: una compra grande no debe redefinir tu gasto estructural). `calendar` en
        // ambos porque es lo que reproduce el comportamiento previo del lado gasto: así el
        // upgrade mueve UN solo eje (la ventana de ingreso), no dos.
        income_avg_window_months: 3,
        income_avg_window_mode: AvgWindowMode::Calendar,
        expense_avg_window_months: 12,
        expense_avg_window_mode: AvgWindowMode::Calendar,
    }
}

fn default_es_tax_brackets() -> Vec<TaxBracket> {
    vec![
        TaxBracket {
            up_to: Some(Decimal::from(6_000u32)),
            pct: Decimal::from(19u32),
        },
        TaxBracket {
            up_to: Some(Decimal::from(50_000u32)),
            pct: Decimal::from(21u32),
        },
        TaxBracket {
            up_to: Some(Decimal::from(200_000u32)),
            pct: Decimal::from(23u32),
        },
        TaxBracket {
            up_to: Some(Decimal::from(300_000u32)),
            pct: Decimal::from(27u32),
        },
        TaxBracket {
            up_to: None,
            pct: Decimal::from(30u32),
        },
    ]
}

/// Resuelve el `FireSettings` almacenado aplicando defaults **y clamps**.
///
/// El clamp vive AQUÍ y no solo en `validate_fire_settings` a propósito: la validación corre
/// únicamente en las dos rutas de ESCRITURA (`PATCH /v1/installation` y la tool MCP), mientras que
/// esta función está en los tres caminos de LECTURA del JSONB. Un valor fuera de rango que llegara
/// por otra vía (restore, edición directa de la BD, un fichero de otra versión) produciría una
/// ventana de 0 meses → promedio sin denominador → fallback silencioso al presupuesto con la UI
/// diciendo que está en modo real. Clampar en el consumo lo hace imposible.
pub(crate) fn resolve_fire_settings(stored: Option<FireSettings>) -> FireSettings {
    let mut fs = match stored {
        None => default_fire_settings(),
        Some(fs) => fs,
    };
    fs.income_avg_window_months = fs
        .income_avg_window_months
        .clamp(MIN_AVG_WINDOW_MONTHS, MAX_AVG_WINDOW_MONTHS);
    fs.expense_avg_window_months = fs
        .expense_avg_window_months
        .clamp(MIN_AVG_WINDOW_MONTHS, MAX_AVG_WINDOW_MONTHS);
    fs
}

pub(crate) fn validate_fire_settings(fs: &FireSettings) -> Result<(), ApiError> {
    if fs.swr_pct < Decimal::ZERO || fs.swr_pct > Decimal::from(4u32) {
        return Err(ApiError::BadRequest(
            "swr_out_of_range: swr_pct must be between 0 and 4 (percent)".into(),
        ));
    }
    match fs.fire_number_mode {
        FireNumberMode::Manual => {
            let Some(amt) = fs.fire_number_manual_amount else {
                return Err(ApiError::BadRequest(
                    "fire_manual_amount_required: fire_number_manual_amount is required when fire_number_mode is manual".into(),
                ));
            };
            if amt <= Decimal::ZERO {
                return Err(ApiError::BadRequest(
                    "fire_manual_amount_not_positive: fire_number_manual_amount must be > 0".into(),
                ));
            }
        }
        FireNumberMode::AnnualExpense | FireNumberMode::CurrentIncome => {}
    }
    for (label, months) in [
        ("income_avg_window_months", fs.income_avg_window_months),
        ("expense_avg_window_months", fs.expense_avg_window_months),
    ] {
        if !(MIN_AVG_WINDOW_MONTHS..=MAX_AVG_WINDOW_MONTHS).contains(&months) {
            return Err(ApiError::BadRequest(format!(
                "avg_window_out_of_range: {label} must be between {MIN_AVG_WINDOW_MONTHS} and {MAX_AVG_WINDOW_MONTHS} (months)"
            )));
        }
    }
    if fs.taxes_enabled {
        validate_tax_brackets(&fs.tax_brackets)?;
    }
    Ok(())
}

fn validate_tax_brackets(brackets: &[TaxBracket]) -> Result<(), ApiError> {
    if brackets.is_empty() {
        return Err(ApiError::BadRequest(
            "tax_brackets_empty: tax_brackets must be non-empty when taxes_enabled is true".into(),
        ));
    }
    let last = brackets.len().saturating_sub(1);
    for (i, b) in brackets.iter().enumerate() {
        if b.pct < Decimal::ZERO || b.pct > Decimal::from(99u32) {
            return Err(ApiError::BadRequest(
                "tax_bracket_pct_out_of_range: tax bracket pct must be between 0 and 99".into(),
            ));
        }
        let is_last = i == last;
        match (&b.up_to, is_last) {
            (None, true) => {}
            (Some(_), true) => {
                return Err(ApiError::BadRequest(
                    "tax_brackets_last_must_be_open: last tax bracket must have up_to null (open-ended)".into(),
                ));
            }
            (None, false) => {
                return Err(ApiError::BadRequest(
                    "tax_brackets_open_not_last: only the last tax bracket may have up_to null".into(),
                ));
            }
            (Some(th), false) => {
                if *th <= Decimal::ZERO {
                    return Err(ApiError::BadRequest(
                        "tax_bracket_threshold_not_positive: tax bracket up_to must be > 0 when set".into(),
                    ));
                }
                if i > 0 {
                    let prev = brackets[i - 1].up_to.as_ref().ok_or_else(|| {
                        ApiError::BadRequest("tax_brackets_ordering_invalid: invalid tax_brackets ordering".into())
                    })?;
                    if *th <= *prev {
                        return Err(ApiError::BadRequest(
                            "tax_brackets_not_increasing: tax bracket up_to values must be strictly increasing".into(),
                        ));
                    }
                }
            }
        }
    }
    Ok(())
}

#[derive(Debug, Serialize, ToSchema)]
pub struct InstallationSnapshot {
    #[schema(value_type = String, format = "uuid")]
    pub id: Uuid,
    pub base_currency: String,
    /// IANA time zone id (e.g. `Europe/Madrid`, `UTC`) for civil calendar operations such as liability derive-principal "today".
    pub calendar_tz: String,
    /// Supuesto anual % aplicado al target FIRE móvil (no a ingresos/gastos/aportaciones). `0` = target plano.
    #[serde(with = "rust_decimal::serde::str")]
    #[schema(value_type = String)]
    pub annual_inflation_assumption_percent: Decimal,
    pub show_age_mode: String,
    pub fire_settings: FireSettings,
    /// Kill-switch vivo de la escritura vía MCP (issue #3): con `false` las tools de escritura
    /// devuelven error tipado en el siguiente request. Editable solo por el owner (Ajustes → MCP).
    pub mcp_write_enabled: bool,
    /// `false` mientras el hogar no haya pasado por el asistente de primera vez. La SPA lo usa
    /// para lanzarlo; el owner puede volver a abrirlo cuando quiera desde Ajustes.
    pub onboarding_completed: bool,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct InstallationAccess {
    pub installation: InstallationSnapshot,
    pub role: MembershipRole,
}

/// Distinguishes first-time setup (no installation row), pending approval (installation exists but no membership), and active membership.
#[derive(Debug, Serialize, ToSchema)]
pub struct InstallationSessionContext {
    /// True when at least one row exists in `installation`.
    pub installation_initialized: bool,
    pub access: Option<InstallationAccess>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct SetupInstallationBody {
    /// ISO 4217 alphabetic code; MVP allows EUR, USD, GBP.
    pub base_currency: String,
    #[serde(default = "default_calendar_tz")]
    pub calendar_tz: String,
    #[serde(default = "default_show_age_mode")]
    pub show_age_mode: String,
}

fn default_show_age_mode() -> String {
    "dates".into()
}

fn default_calendar_tz() -> String {
    "UTC".into()
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct PatchInstallationBody {
    /// When omitted, the time zone is left unchanged.
    #[serde(default)]
    pub calendar_tz: Option<String>,
    /// When omitted, `show_age_mode` is left unchanged (`dates` or `ages`).
    #[serde(default)]
    pub show_age_mode: Option<String>,
    /// Omit = unchanged; string percent (e.g. `"2.5"`). `0` desactiva el target móvil.
    #[serde(default)]
    pub annual_inflation_assumption_percent: Option<String>,
    /// Omit = unchanged; JSON `null` clears stored JSON (defaults apply on read).
    #[serde(default, deserialize_with = "crate::handlers::deserialize_double_option_typed")]
    pub fire_settings: Option<Option<FireSettings>>,
    /// Omit = unchanged. Kill-switch de la escritura vía MCP (owner-only como todo el PATCH).
    #[serde(default)]
    pub mcp_write_enabled: Option<bool>,
    /// Omit = unchanged. Divisa base del hogar (EUR, USD o GBP). **Una sola por instalación**:
    /// FutureFin no convierte entre divisas ni las mezcla. Hasta 3.10.0 estaba clavada a EUR en
    /// el código y no había forma de cambiarla — el único selector que existía era código muerto
    /// e inalcanzable, así que un usuario fuera de la eurozona se quedaba en euros para siempre.
    #[serde(default)]
    pub base_currency: Option<String>,
    /// Omit = unchanged. `true` cierra el asistente de primera vez; `false` lo vuelve a abrir.
    #[serde(default)]
    pub onboarding_completed: Option<bool>,
}

#[derive(Debug, sqlx::FromRow)]
struct InstallationMemberRow {
    id: Uuid,
    base_currency: String,
    calendar_tz: String,
    annual_inflation_assumption_percent: Decimal,
    show_age_mode: String,
    fire_settings: Option<SqlxJson<FireSettings>>,
    mcp_write_enabled: bool,
    onboarding_completed_at: Option<DateTime<Utc>>,
    role: String,
}

fn installation_access_from_row(r: InstallationMemberRow) -> Result<InstallationAccess, ApiError> {
    let role = MembershipRole::parse(&r.role)?;
    Ok(InstallationAccess {
        installation: InstallationSnapshot {
            id: r.id,
            base_currency: r.base_currency,
            calendar_tz: r.calendar_tz,
            // Sin clamp desde 4.9.0 (#146): la validación de escritura acota [−2, 50] y ningún
            // valor almacenado pudo nacer fuera (el validador histórico rechazaba TODO negativo).
            annual_inflation_assumption_percent: r.annual_inflation_assumption_percent,
            show_age_mode: r.show_age_mode,
            fire_settings: resolve_fire_settings(r.fire_settings.map(|j| j.0)),
            mcp_write_enabled: r.mcp_write_enabled,
            onboarding_completed: r.onboarding_completed_at.is_some(),
        },
        role,
    })
}

/// Divisa base del hogar. Una sola por instalación: FutureFin no convierte ni mezcla divisas.
///
/// La usa el import de CSV para decidir qué filas puede aceptar. Antes ese control estaba a fuego
/// en `"EUR"`, así que una instalación en libras no podía importar sus propios extractos.
pub(crate) async fn installation_base_currency(
    pool: &PgPool,
    installation_id: Uuid,
) -> Result<String, ApiError> {
    let cur: String =
        sqlx::query_scalar(r#"SELECT base_currency FROM installation WHERE id = $1"#)
            .bind(installation_id)
            .fetch_one(pool)
            .await?;
    Ok(cur)
}

fn normalize_currency(code: &str) -> Result<String, ApiError> {
    let trimmed = code.trim();
    if trimmed.len() != 3 || !trimmed.chars().all(|c| c.is_ascii_alphabetic()) {
        return Err(ApiError::BadRequest(
            "currency_format_invalid: base_currency must be a 3-letter alphabetic code".into(),
        ));
    }
    let upper = trimmed.to_ascii_uppercase();
    if !matches!(upper.as_str(), "EUR" | "USD" | "GBP") {
        return Err(ApiError::BadRequest(
            "currency_unsupported: unsupported base_currency for MVP (use EUR, USD, or GBP)".into(),
        ));
    }
    Ok(upper)
}

fn validate_show_age_mode(mode: &str) -> Result<(), ApiError> {
    if matches!(mode, "dates" | "ages") {
        Ok(())
    } else {
        Err(ApiError::BadRequest(
            "show_age_mode_invalid: show_age_mode must be \"dates\" or \"ages\"".into(),
        ))
    }
}

pub(crate) fn validate_annual_inflation_assumption(pct: Decimal) -> Result<(), ApiError> {
    // Rango [−2, 50] desde 4.9.0 (#146): España tuvo IPC anual medio negativo cinco veces este
    // siglo (mínimo interanual −1,4 %, jul-2009) — el suelo 0 tenía una base histórica falsa y
    // impedía estresar el propio plan con deflación. El −2 acota el peor escenario razonable.
    if pct < Decimal::from(-2) || pct > Decimal::from(50) {
        return Err(ApiError::BadRequest(
            "inflation_out_of_range: annual_inflation_assumption_percent must be between -2 and 50".into(),
        ));
    }
    Ok(())
}

pub(crate) fn normalize_calendar_tz(raw: &str) -> Result<String, ApiError> {
    let t = raw.trim();
    if !(3..=64).contains(&t.len()) {
        return Err(ApiError::BadRequest(
            "timezone_invalid: calendar_tz must be between 3 and 64 characters".into(),
        ));
    }
    let _: Tz = t.parse().map_err(|_| {
        ApiError::BadRequest(
            "timezone_invalid: calendar_tz must be a valid IANA time zone name (e.g. Europe/Madrid, America/New_York, UTC)"
                .into(),
        )
    })?;
    Ok(t.into())
}

/// Today's date in the installation civil calendar (IANA `calendar_tz`).
pub(crate) async fn installation_naive_today(
    pool: &PgPool,
    installation_id: Uuid,
) -> Result<NaiveDate, ApiError> {
    let tz_str: String =
        sqlx::query_scalar(r#"SELECT calendar_tz FROM installation WHERE id = $1"#)
            .bind(installation_id)
            .fetch_optional(pool)
            .await?
            .ok_or(ApiError::NotFound)?;
    naive_date_in_calendar_tz(&tz_str)
}

pub(crate) fn naive_date_in_calendar_tz(tz_name: &str) -> Result<NaiveDate, ApiError> {
    let tz: Tz = tz_name.trim().parse().map_err(|_| {
        ApiError::BadRequest(
            "installation_timezone_broken: installation calendar_tz is invalid; update it via PATCH /v1/installation".into(),
        )
    })?;
    Ok(Utc::now().with_timezone(&tz).date_naive())
}

/// Singleton installation row id, if one exists.
pub async fn singleton_installation_id(pool: &PgPool) -> Result<Option<Uuid>, ApiError> {
    let id: Option<Uuid> =
        sqlx::query_scalar(r#"SELECT id FROM installation ORDER BY created_at ASC LIMIT 1"#)
            .fetch_optional(pool)
            .await?;
    Ok(id)
}

pub async fn require_singleton_installation_id(pool: &PgPool) -> Result<Uuid, ApiError> {
    singleton_installation_id(pool)
        .await?
        .ok_or(ApiError::NotFound)
}

/// First registered user: create singleton installation + owner membership (same transaction).
pub(crate) async fn bootstrap_installation_as_owner_if_empty(
    tx: &mut Transaction<'_, Postgres>,
    user_id: &Uuid,
) -> Result<(), ApiError> {
    let count: i64 = sqlx::query_scalar(r#"SELECT COUNT(*)::bigint FROM installation"#)
        .fetch_one(&mut **tx)
        .await?;
    if count > 0 {
        return Ok(());
    }

    let iid: Uuid = sqlx::query_scalar(
        r#"INSERT INTO installation (
               base_currency,
               show_age_mode
           )
           VALUES ('EUR', 'dates')
           RETURNING id"#,
    )
    .fetch_one(&mut **tx)
    .await?;

    sqlx::query(
        r#"INSERT INTO installation_memberships (installation_id, user_id, role)
           VALUES ($1, $2, $3)"#,
    )
    .bind(iid)
    .bind(user_id)
    .bind(MembershipRole::Owner.as_str())
    .execute(&mut **tx)
    .await?;

    seed_default_categories(tx, iid).await?;

    Ok(())
}

/// Juego de categorías con el que arranca un hogar nuevo.
///
/// La migración original decía «No server-side seeding; clients create categories as needed», y el
/// resultado era una app que no se podía usar recién instalada: sin categorías, `AssetsView`
/// **escondía** el botón de añadir y la única pista era una miga de pan de dos palabras
/// («Activos · Ajustes → Categorías») que ni siquiera era un enlace. Sembrar es más honesto que
/// pedirle a alguien que adivine el orden de los pasos.
///
/// Son un punto de partida, no un dogma: se renombran y se borran desde Ajustes como cualquier
/// otra. Por eso la lista es corta — cubre lo que casi todo el mundo tiene y deja fuera lo que
/// depende de cada uno.
const DEFAULT_CATEGORIES: &[(&str, &[&str])] = &[
    ("asset", &["Cuenta corriente", "Ahorro", "Inversión", "Inmuebles"]),
    ("liability", &["Hipoteca", "Préstamo", "Tarjeta de crédito"]),
    ("income", &["Nómina", "Otros ingresos"]),
    (
        "expense",
        &[
            "Vivienda",
            "Supermercado",
            "Suministros",
            "Transporte",
            "Ocio",
            "Salud",
            "Otros gastos",
        ],
    ),
];

/// Inserta `DEFAULT_CATEGORIES` en la instalación recién creada, dentro de la MISMA transacción
/// que la crea: o hay hogar con categorías, o no hay hogar.
///
/// `ON CONFLICT DO NOTHING` sobre el único `(installation_id, scope, name)`: la función solo se
/// llama al crear la instalación, pero si algún día se reutiliza no debe romper por un nombre que
/// ya exista.
pub(crate) async fn seed_default_categories(
    tx: &mut Transaction<'_, Postgres>,
    installation_id: Uuid,
) -> Result<(), ApiError> {
    for (scope, names) in DEFAULT_CATEGORIES {
        for (i, name) in names.iter().enumerate() {
            sqlx::query(
                r#"INSERT INTO categories (installation_id, scope, name, sort_index)
                   VALUES ($1, $2, $3, $4)
                   ON CONFLICT (installation_id, scope, name) DO NOTHING"#,
            )
            .bind(installation_id)
            .bind(scope)
            .bind(name)
            .bind(i as i32)
            .execute(&mut **tx)
            .await?;
        }
    }
    Ok(())
}

#[utoipa::path(
    get,
    path = "/v1/installation/session-context",
    tag = "installation",
    responses(
        (status = 200, description = "Whether an installation exists and the caller's membership, if any",
            body = InstallationSessionContext),
        (status = 401, description = "No valid session"),
    )
)]
pub async fn get_installation_session_context(
    Extension(state): Extension<Arc<AppState>>,
    jar: CookieJar,
) -> Result<Json<InstallationSessionContext>, ApiError> {
    let user = require_session_user(&jar, &state.pool).await?;
    let installation_initialized: bool =
        sqlx::query_scalar(r#"SELECT EXISTS(SELECT 1 FROM installation)"#)
            .fetch_one(&state.pool)
            .await?;

    let row: Option<InstallationMemberRow> = sqlx::query_as(
        r#"SELECT i.id, i.base_currency, i.calendar_tz,
                  i.annual_inflation_assumption_percent,
                  i.show_age_mode, i.fire_settings, i.mcp_write_enabled,
                  i.onboarding_completed_at, m.role
           FROM installation_memberships m
           JOIN installation i ON i.id = m.installation_id
           WHERE m.user_id = $1
           ORDER BY i.created_at ASC
           LIMIT 1"#,
    )
    .bind(user.id.0)
    .fetch_optional(&state.pool)
    .await?;

    let access = match row {
        Some(r) => Some(installation_access_from_row(r)?),
        None => None,
    };

    Ok(Json(InstallationSessionContext {
        installation_initialized,
        access,
    }))
}

#[utoipa::path(
    get,
    path = "/v1/installation",
    tag = "installation",
    responses(
        (status = 200, description = "Installation context if the user is a member; JSON null if not", body = Option<InstallationAccess>),
        (status = 401, description = "No valid session"),
    )
)]
pub async fn get_my_installation(
    Extension(state): Extension<Arc<AppState>>,
    jar: CookieJar,
) -> Result<Json<Option<InstallationAccess>>, ApiError> {
    let user = require_session_user(&jar, &state.pool).await?;
    Ok(Json(installation_access_core(&state.pool, user.id.0).await?))
}

/// Core sin HTTP: lo comparten el handler GET y la tool MCP `get_settings`.
/// `None` = el usuario no es miembro de ninguna instalación.
pub(crate) async fn installation_access_core(
    pool: &PgPool,
    user_id: Uuid,
) -> Result<Option<InstallationAccess>, ApiError> {
    let row: Option<InstallationMemberRow> = sqlx::query_as(
        r#"SELECT i.id, i.base_currency, i.calendar_tz,
                  i.annual_inflation_assumption_percent,
                  i.show_age_mode, i.fire_settings, i.mcp_write_enabled,
                  i.onboarding_completed_at, m.role
           FROM installation_memberships m
           JOIN installation i ON i.id = m.installation_id
           WHERE m.user_id = $1
           ORDER BY i.created_at ASC
           LIMIT 1"#,
    )
    .bind(user_id)
    .fetch_optional(pool)
    .await?;

    let Some(r) = row else {
        return Ok(None);
    };

    Ok(Some(installation_access_from_row(r)?))
}

/// Cambios campo a campo de la tool MCP `update_fire_settings`. NUNCA se deserializa a
/// `FireSettings` directamente: su `#[serde(default)]` a nivel de struct resetearía los campos
/// ausentes a defaults (un PATCH «solo swr» borraría los tramos fiscales personalizados) — este
/// DTO existe para esquivar exactamente ese bug.
#[derive(Debug, Default)]
pub(crate) struct FireSettingsPatch {
    pub swr_pct: Option<Decimal>,
    pub taxes_enabled: Option<bool>,
    pub tax_brackets: Option<Vec<TaxBracket>>,
    pub fire_number_mode: Option<FireNumberMode>,
    pub fire_number_manual_amount: Option<Decimal>,
    pub savings_source: Option<SavingsSource>,
    pub income_avg_window_months: Option<u32>,
    pub income_avg_window_mode: Option<AvgWindowMode>,
    pub expense_avg_window_months: Option<u32>,
    pub expense_avg_window_mode: Option<AvgWindowMode>,
    /// Columna aparte de la instalación (no vive en el JSONB), pero es un eje FIRE más.
    pub annual_inflation_assumption_percent: Option<Decimal>,
}

impl FireSettingsPatch {
    /// Aplica el patchset sobre una base y devuelve el resultado, **sin validar ni persistir**.
    ///
    /// Lo comparten el PATCH real y el override what-if de `simulate_projection`. Que sea el mismo
    /// código no es estética: el sentido de poder simular «¿y si cambio de modo de ahorro?» es que
    /// prediga lo que pasaría al cambiarlo de verdad, y dos copias del aplicador se separan sin
    /// que ningún test lo note.
    ///
    /// `annual_inflation_assumption_percent` NO se aplica aquí: vive en una columna de
    /// `installation`, no en el JSONB, y cada caller lo resuelve por su lado.
    pub(crate) fn apply_to(&self, base: &FireSettings) -> FireSettings {
        let mut after = base.clone();
        if let Some(v) = self.swr_pct {
            after.swr_pct = v;
        }
        if let Some(v) = self.taxes_enabled {
            after.taxes_enabled = v;
        }
        if let Some(v) = self.tax_brackets.clone() {
            after.tax_brackets = v;
        }
        if let Some(v) = self.fire_number_mode {
            after.fire_number_mode = v;
        }
        if let Some(v) = self.fire_number_manual_amount {
            after.fire_number_manual_amount = Some(v);
        }
        if let Some(v) = self.savings_source {
            after.savings_source = v;
        }
        if let Some(v) = self.income_avg_window_months {
            after.income_avg_window_months = v;
        }
        if let Some(v) = self.income_avg_window_mode {
            after.income_avg_window_mode = v;
        }
        if let Some(v) = self.expense_avg_window_months {
            after.expense_avg_window_months = v;
        }
        if let Some(v) = self.expense_avg_window_mode {
            after.expense_avg_window_mode = v;
        }
        after
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.swr_pct.is_none()
            && self.taxes_enabled.is_none()
            && self.tax_brackets.is_none()
            && self.fire_number_mode.is_none()
            && self.fire_number_manual_amount.is_none()
            && self.savings_source.is_none()
            && self.income_avg_window_months.is_none()
            && self.income_avg_window_mode.is_none()
            && self.expense_avg_window_months.is_none()
            && self.expense_avg_window_mode.is_none()
            && self.annual_inflation_assumption_percent.is_none()
    }
}

/// Before/after del merge (el preview de la tool los enseña; el apply además persiste).
#[derive(Debug, Serialize)]
pub(crate) struct FireSettingsPatchOutcome {
    pub before: FireSettings,
    pub after: FireSettings,
    #[serde(with = "rust_decimal::serde::str")]
    pub annual_inflation_before: Decimal,
    #[serde(with = "rust_decimal::serde::str")]
    pub annual_inflation_after: Decimal,
}

/// Core de la tool MCP `update_fire_settings` (no hay endpoint HTTP equivalente: el PATCH de la
/// SPA envía siempre el objeto completo). Lee el estado actual, aplica SOLO los campos presentes
/// del patchset, re-valida con las mismas cotas del PATCH real y — con `apply = true` — escribe
/// el objeto COMPLETO e invalida FULL. Con `apply = false` valida y devuelve el before/after sin
/// tocar nada (preview).
pub(crate) async fn patch_fire_settings_core(
    state: &Arc<AppState>,
    iid: Uuid,
    user_id: Uuid,
    patchset: FireSettingsPatch,
    apply: bool,
) -> Result<FireSettingsPatchOutcome, ApiError> {
    // Owner-only EN LA CORE, no en la tool (D14, issue #99): así cualquier llamante futuro
    // (HTTP, otra tool, un job) queda protegido por construcción. El dual-branch drift ya
    // mordió dos veces (Fase 2: clear_* solo en MCP; Fase 6: SinkPolicy solo en una core).
    if !user_is_installation_owner(&state.pool, user_id, iid).await? {
        return Err(ApiError::Forbidden);
    }
    if patchset.is_empty() {
        return Err(ApiError::BadRequest(
            "patch_empty: provide at least one FIRE setting to change".into(),
        ));
    }
    let (stored, inflation_before): (Option<SqlxJson<FireSettings>>, Decimal) = sqlx::query_as(
        r#"SELECT fire_settings, annual_inflation_assumption_percent
           FROM installation WHERE id = $1"#,
    )
    .bind(iid)
    .fetch_one(&state.pool)
    .await?;
    let before = resolve_fire_settings(stored.map(|j| j.0));

    let after = patchset.apply_to(&before);
    validate_fire_settings(&after)?;

    let annual_inflation_after = match patchset.annual_inflation_assumption_percent {
        Some(v) => {
            validate_annual_inflation_assumption(v)?;
            v
        }
        None => inflation_before,
    };

    if apply {
        sqlx::query(
            r#"UPDATE installation
               SET fire_settings = $1, annual_inflation_assumption_percent = $2
               WHERE id = $3"#,
        )
        .bind(SqlxJson(after.clone()))
        .bind(annual_inflation_after)
        .bind(iid)
        .execute(&state.pool)
        .await?;
        // FULL: así es como un cambio de modo A↔B/C o de SWR surte efecto en la proyección.
        refresh_projection_after_mutation(&state, iid, user_id).await;
    }

    Ok(FireSettingsPatchOutcome {
        before,
        after,
        annual_inflation_before: inflation_before,
        annual_inflation_after,
    })
}

// ---------------------------------------------------------------------------
// Ejes de PRESENTACIÓN de la instalación (allowlist estricta)
// ---------------------------------------------------------------------------

/// Los tres ejes de presentación del hogar, tal y como están guardados.
#[derive(Debug, Clone, Serialize)]
pub(crate) struct PresentationSettings {
    /// Zona horaria IANA con la que se resuelve el «hoy» civil de la instalación.
    pub calendar_tz: String,
    /// `dates` | `ages`: cómo rotula la app el eje temporal.
    pub show_age_mode: String,
    /// `EUR` | `USD` | `GBP`. **Una sola por instalación**: FutureFin no convierte ni mezcla.
    pub base_currency: String,
}

/// Patchset campo a campo de los ejes de presentación. Omitir = conservar.
///
/// **Allowlist estricta, y las dos ausencias son deliberadas**:
/// - `mcp_write_enabled` **jamás**: es el kill-switch de la escritura por MCP, y un interruptor
///   que puede reencenderse a sí mismo es decorativo (rubrica §2.1 de la skill de paridad).
/// - `onboarding_completed` **tampoco**: es estado de la UI (marca que la SPA ya no debe enseñar
///   el asistente de alta). Ponerlo a `true` no cambia ni un dato del hogar; solo le quita a una
///   persona una pantalla que quizá no había visto.
/// - `fire_settings` y `annual_inflation_assumption_percent` no están porque ya los cubre
///   `patch_fire_settings_core`, campo a campo y con su propio preview.
#[allow(dead_code)]
#[derive(Debug, Default, Clone)]
pub(crate) struct PresentationSettingsPatch {
    pub calendar_tz: Option<String>,
    pub show_age_mode: Option<String>,
    pub base_currency: Option<String>,
}

impl PresentationSettingsPatch {
    #[allow(dead_code)]
    pub(crate) fn is_empty(&self) -> bool {
        // Destructuring exhaustivo y sin `..`: añadir un eje a la allowlist deja de compilar hasta
        // que alguien decida si cuenta como «algo que actualizar».
        let PresentationSettingsPatch {
            calendar_tz,
            show_age_mode,
            base_currency,
        } = self;
        calendar_tz.is_none() && show_age_mode.is_none() && base_currency.is_none()
    }
}

/// Before/after del merge (el preview de la tool los enseña; el apply además persiste).
#[allow(dead_code)]
#[derive(Debug, Serialize)]
pub(crate) struct PresentationSettingsOutcome {
    pub before: PresentationSettings,
    pub after: PresentationSettings,
}

/// Core de la tool MCP `update_installation_settings` (no hay endpoint HTTP equivalente: el
/// `PATCH /v1/installation` de la SPA manda el objeto entero, incluidos los dos ejes que esta
/// allowlist prohíbe). Lee el estado actual, aplica SOLO los campos presentes con **las mismas
/// validaciones** que el PATCH real (`normalize_calendar_tz`, `validate_show_age_mode`,
/// `normalize_currency`) y — con `apply = true` — persiste e invalida FULL. Con `apply = false`
/// valida y devuelve el before/after sin tocar nada (preview).
///
/// **Owner-only, comprobado AQUÍ DENTRO** (y no solo en la superficie que llama): el `PATCH`
/// HTTP exige `MembershipRole::Owner` y estos ejes son los mismos. Que la comprobación viva en la
/// core es lo que impide que una superficie nueva se la deje.
///
/// **Cache FULL** cuando aplica: `calendar_tz` mueve el «hoy» civil —o sea el mes 0 de la
/// proyección entera— y `show_age_mode` viaja DENTRO de `ProjectionSeriesResponse`. Solo
/// `base_currency` sería inocua, y no vale la pena una invalidación condicional por eje.
#[allow(dead_code)]
pub(crate) async fn patch_presentation_settings_core(
    state: &Arc<AppState>,
    iid: Uuid,
    user_id: Uuid,
    patchset: PresentationSettingsPatch,
    apply: bool,
) -> Result<PresentationSettingsOutcome, ApiError> {
    if patchset.is_empty() {
        return Err(ApiError::BadRequest(
            "patch_empty: provide at least one of calendar_tz, show_age_mode, base_currency".into(),
        ));
    }
    if !user_is_installation_owner(&state.pool, user_id, iid).await? {
        return Err(ApiError::Forbidden);
    }

    let (calendar_tz, show_age_mode, base_currency): (String, String, String) = sqlx::query_as(
        r#"SELECT calendar_tz, show_age_mode, base_currency FROM installation WHERE id = $1"#,
    )
    .bind(iid)
    .fetch_one(&state.pool)
    .await?;
    let before = PresentationSettings {
        calendar_tz,
        show_age_mode,
        base_currency,
    };

    let after = PresentationSettings {
        calendar_tz: match &patchset.calendar_tz {
            Some(raw) => normalize_calendar_tz(raw)?,
            None => before.calendar_tz.clone(),
        },
        show_age_mode: match &patchset.show_age_mode {
            Some(raw) => {
                let trimmed = raw.trim();
                validate_show_age_mode(trimmed)?;
                trimmed.to_string()
            }
            None => before.show_age_mode.clone(),
        },
        base_currency: match &patchset.base_currency {
            Some(raw) => normalize_currency(raw)?,
            None => before.base_currency.clone(),
        },
    };

    if apply {
        sqlx::query(
            r#"UPDATE installation
               SET calendar_tz = $1, show_age_mode = $2, base_currency = $3
               WHERE id = $4"#,
        )
        .bind(&after.calendar_tz)
        .bind(&after.show_age_mode)
        .bind(&after.base_currency)
        .bind(iid)
        .execute(&state.pool)
        .await?;
        refresh_projection_after_mutation(state, iid, user_id).await;
    }

    Ok(PresentationSettingsOutcome { before, after })
}

/// Identidad mínima del usuario del token para la tool MCP `get_settings` (el endpoint HTTP
/// `GET /v1/installation` NO la incluye — la sesión web ya conoce a su usuario).
#[derive(Debug, Serialize)]
pub(crate) struct SettingsUser {
    pub id: Uuid,
    pub username: String,
    /// La DOB que fija el horizonte de proyección (si está definida).
    pub birth_date: Option<chrono::NaiveDate>,
}

/// Core sin HTTP: identidad del usuario para `get_settings` (MCP).
pub(crate) async fn settings_user_core(
    pool: &PgPool,
    user_id: Uuid,
) -> Result<SettingsUser, ApiError> {
    let (username, birth_date): (String, Option<chrono::NaiveDate>) =
        sqlx::query_as(r#"SELECT username, birth_date FROM users WHERE id = $1"#)
            .bind(user_id)
            .fetch_one(pool)
            .await?;
    Ok(SettingsUser {
        id: user_id,
        username,
        birth_date,
    })
}

#[utoipa::path(
    patch,
    path = "/v1/installation",
    tag = "installation",
    request_body = PatchInstallationBody,
    responses(
        (status = 200, description = "Updated", body = InstallationAccess),
        (status = 400, description = "Validation error"),
        (status = 401, description = "No valid session"),
        (status = 403, description = "Not installation owner"),
        (status = 404, description = "Installation missing"),
    )
)]
pub async fn patch_my_installation(
    Extension(state): Extension<Arc<AppState>>,
    jar: CookieJar,
    Json(body): Json<PatchInstallationBody>,
) -> Result<Json<InstallationAccess>, ApiError> {
    let user = require_session_user(&jar, &state.pool).await?;
    let (iid, role) = require_installation_member(&state.pool, user.id.0).await?;
    if role != MembershipRole::Owner {
        return Err(ApiError::Forbidden);
    }

    if body.calendar_tz.is_none()
        && body.show_age_mode.is_none()
        && body.annual_inflation_assumption_percent.is_none()
        && body.fire_settings.is_none()
        && body.mcp_write_enabled.is_none()
        && body.base_currency.is_none()
        && body.onboarding_completed.is_none()
    {
        return Err(ApiError::BadRequest(
            "patch_empty: provide at least one of calendar_tz, show_age_mode, annual_inflation_assumption_percent, fire_settings, mcp_write_enabled, base_currency, onboarding_completed".into(),
        ));
    }

    let row_before: InstallationMemberRow = sqlx::query_as(
        r#"SELECT i.id, i.base_currency, i.calendar_tz,
                  i.annual_inflation_assumption_percent,
                  i.show_age_mode, i.fire_settings, i.mcp_write_enabled,
                  i.onboarding_completed_at, m.role
           FROM installation_memberships m
           JOIN installation i ON i.id = m.installation_id
           WHERE m.user_id = $1 AND i.id = $2"#,
    )
    .bind(user.id.0)
    .bind(iid)
    .fetch_one(&state.pool)
    .await?;

    let new_tz = if let Some(ref raw) = body.calendar_tz {
        normalize_calendar_tz(raw)?
    } else {
        row_before.calendar_tz.clone()
    };

    let new_show_age = if let Some(ref raw) = body.show_age_mode {
        let trimmed = raw.trim();
        validate_show_age_mode(trimmed)?;
        trimmed.to_string()
    } else {
        row_before.show_age_mode.clone()
    };

    let new_ann_inf = match &body.annual_inflation_assumption_percent {
        None => row_before.annual_inflation_assumption_percent,
        Some(raw) => {
            let t = raw.trim();
            if t.is_empty() {
                Decimal::ZERO
            } else {
                let pct = Decimal::from_str(t).map_err(|_| {
                    ApiError::BadRequest(
                        "inflation_not_a_number: annual_inflation_assumption_percent must be a decimal number".into(),
                    )
                })?;
                validate_annual_inflation_assumption(pct)?;
                pct
            }
        }
    };

    let new_fire_settings_json: Option<SqlxJson<FireSettings>> = match &body.fire_settings {
        None => row_before.fire_settings.clone(),
        Some(None) => None,
        Some(Some(fs)) => {
            validate_fire_settings(fs)?;
            Some(SqlxJson(fs.clone()))
        }
    };

    let new_mcp_write = body.mcp_write_enabled.unwrap_or(row_before.mcp_write_enabled);

    let new_currency = match &body.base_currency {
        None => row_before.base_currency.clone(),
        Some(raw) => normalize_currency(raw)?,
    };

    // `true` sella el momento; `false` lo borra y el asistente vuelve a salir. Reenviar `true`
    // sobre un hogar ya configurado no mueve la fecha original: es un estado, no un contador.
    let new_onboarding_at = match body.onboarding_completed {
        None => row_before.onboarding_completed_at,
        Some(true) => row_before.onboarding_completed_at.or_else(|| Some(Utc::now())),
        Some(false) => None,
    };

    sqlx::query(
        r#"UPDATE installation SET calendar_tz = $1,
               show_age_mode = $2,
               annual_inflation_assumption_percent = $3,
               fire_settings = $4,
               mcp_write_enabled = $5,
               base_currency = $6,
               onboarding_completed_at = $7
           WHERE id = $8"#,
    )
    .bind(&new_tz)
    .bind(&new_show_age)
    .bind(new_ann_inf)
    .bind(new_fire_settings_json)
    .bind(new_mcp_write)
    .bind(&new_currency)
    .bind(new_onboarding_at)
    .bind(iid)
    .execute(&state.pool)
    .await?;

    let row: InstallationMemberRow = sqlx::query_as(
        r#"SELECT i.id, i.base_currency, i.calendar_tz,
                  i.annual_inflation_assumption_percent,
                  i.show_age_mode, i.fire_settings, i.mcp_write_enabled,
                  i.onboarding_completed_at, m.role
           FROM installation_memberships m
           JOIN installation i ON i.id = m.installation_id
           WHERE m.user_id = $1 AND i.id = $2"#,
    )
    .bind(user.id.0)
    .bind(iid)
    .fetch_one(&state.pool)
    .await?;

    refresh_projection_after_mutation(&state, iid, user.id.0).await;
    Ok(Json(installation_access_from_row(row)?))
}

#[utoipa::path(
    post,
    path = "/v1/installation/setup",
    tag = "installation",
    request_body = SetupInstallationBody,
    responses(
        (status = 201, description = "Installation created; caller becomes owner", body = InstallationAccess),
        (status = 400, description = "Validation error"),
        (status = 401, description = "No valid session"),
        (status = 409, description = "Installation already exists or user already has access"),
    )
)]
pub async fn setup_installation(
    Extension(state): Extension<Arc<AppState>>,
    jar: CookieJar,
    Json(body): Json<SetupInstallationBody>,
) -> Result<(axum::http::StatusCode, Json<InstallationAccess>), ApiError> {
    let user = require_session_user(&jar, &state.pool).await?;
    let currency = normalize_currency(&body.base_currency)?;
    let calendar_tz = normalize_calendar_tz(&body.calendar_tz)?;
    validate_show_age_mode(&body.show_age_mode)?;

    let mut tx = state.pool.begin().await?;

    let hc: i64 = sqlx::query_scalar(r#"SELECT COUNT(*)::bigint FROM installation"#)
        .fetch_one(&mut *tx)
        .await?;
    if hc > 0 {
        return Err(ApiError::Conflict);
    }

    let mc: i64 = sqlx::query_scalar(
        r#"SELECT COUNT(*)::bigint FROM installation_memberships WHERE user_id = $1"#,
    )
    .bind(user.id.0)
    .fetch_one(&mut *tx)
    .await?;
    if mc > 0 {
        return Err(ApiError::Conflict);
    }

    let iid: Uuid = sqlx::query_scalar(
        r#"INSERT INTO installation (
               base_currency,
               show_age_mode,
               calendar_tz,
               onboarding_completed_at
           )
           VALUES ($1, $2, $3, now())
           RETURNING id"#,
    )
    .bind(&currency)
    .bind(&body.show_age_mode)
    .bind(&calendar_tz)
    .fetch_one(&mut *tx)
    .await?;

    sqlx::query(
        r#"INSERT INTO installation_memberships (installation_id, user_id, role)
           VALUES ($1, $2, $3)"#,
    )
    .bind(iid)
    .bind(user.id.0)
    .bind(MembershipRole::Owner.as_str())
    .execute(&mut *tx)
    .await?;

    seed_default_categories(&mut tx, iid).await?;

    tx.commit().await?;

    Ok((
        axum::http::StatusCode::CREATED,
        Json(InstallationAccess {
            installation: InstallationSnapshot {
                id: iid,
                base_currency: currency,
                calendar_tz,
                annual_inflation_assumption_percent: Decimal::ZERO,
                show_age_mode: body.show_age_mode,
                fire_settings: default_fire_settings(),
                mcp_write_enabled: true,
                // Este endpoint ES la configuración inicial, así que el hogar nace configurado.
                onboarding_completed: true,
            },
            role: MembershipRole::Owner,
        }),
    ))
}

pub(crate) async fn user_is_installation_owner(
    pool: &PgPool,
    user_id: Uuid,
    installation_id: Uuid,
) -> Result<bool, ApiError> {
    let role: Option<String> = sqlx::query_scalar(
        r#"SELECT role FROM installation_memberships
           WHERE installation_id = $1 AND user_id = $2"#,
    )
    .bind(installation_id)
    .bind(user_id)
    .fetch_optional(pool)
    .await?;
    Ok(matches!(role.as_deref(), Some("owner")))
}

/// Carga y resuelve el `FireSettings` de la instalación por un **único** camino de deserialización
/// (el del struct + `resolve_fire_settings`, que aplica los defaults y clamps). Fuente de verdad de
/// la config FIRE para los handlers que la necesitan fuera de `compute_projection_series_response`.
pub async fn load_fire_settings(
    pool: &PgPool,
    installation_id: Uuid,
) -> Result<FireSettings, ApiError> {
    let stored: Option<SqlxJson<FireSettings>> =
        sqlx::query_scalar(r#"SELECT fire_settings FROM installation WHERE id = $1"#)
            .bind(installation_id)
            .fetch_one(pool)
            .await?;
    Ok(resolve_fire_settings(stored.map(|j| j.0)))
}

/// Fuente del ahorro efectiva de la proyección. Lo usan las mutaciones de `transactions` para decidir
/// si invalidar la cache de proyección (las transacciones son inputs del engine SOLO en modo
/// `transactions_avg`) y `get_summary`. Deserializa el `FireSettings` completo por el mismo camino
/// que el resto del código (`resolve_fire_settings`), de modo que cualquier variante futura del enum
/// se resuelve por un único parser en lugar de caer silenciosamente a `Budget`.
pub async fn projection_savings_source(
    pool: &PgPool,
    installation_id: Uuid,
) -> Result<SavingsSource, ApiError> {
    Ok(load_fire_settings(pool, installation_id).await?.savings_source)
}

/// Los escalares de instalación que `/v1/summary` necesita, en **una** query:
/// `(hoy en el calendario civil, inflación anual %, fire_settings resueltos)`. De los
/// `FireSettings` salen el `savings_source` (base de gasto de los modos B/C) **y** el
/// `swr_pct` + tramos fiscales que deciden el caso «infinito» del runway.
///
/// Sustituye a `installation_naive_today` + `projection_savings_source` (dos round-trips) sin
/// duplicar parseos: la fecha sale de [`naive_date_in_calendar_tz`] y los settings del
/// mismo camino de deserialización que el resto del código ([`resolve_fire_settings`]).
/// La inflación se **clampa a ≥ 0**, mismo criterio que `compute_projection_series_response`
/// (una inflación negativa guardada no debe alargar el runway ni encoger el target FIRE).
pub(crate) async fn installation_calendar_inflation_fire(
    pool: &PgPool,
    installation_id: Uuid,
) -> Result<(NaiveDate, Decimal, FireSettings), ApiError> {
    type Row = (String, Decimal, Option<SqlxJson<FireSettings>>);
    let row: Option<Row> = sqlx::query_as(
        r#"SELECT calendar_tz, annual_inflation_assumption_percent, fire_settings
           FROM installation WHERE id = $1"#,
    )
    .bind(installation_id)
    .fetch_optional(pool)
    .await?;
    let (tz_str, inflation, fire_settings) = row.ok_or(ApiError::NotFound)?;
    Ok((
        naive_date_in_calendar_tz(&tz_str)?,
        // Sin clamp desde 4.9.0 (#146): rango garantizado por la validación de escritura.
        inflation,
        resolve_fire_settings(fire_settings.map(|j| j.0)),
    ))
}

/// Resolves the singleton installation and membership role, or `NotFound` / `Forbidden`.
pub async fn require_installation_member(
    pool: &PgPool,
    user_id: Uuid,
) -> Result<(Uuid, MembershipRole), ApiError> {
    let iid = require_singleton_installation_id(pool).await?;
    let Some(role_str) =
        crate::handlers::membership::membership_role(pool, user_id, iid).await?
    else {
        return Err(ApiError::Forbidden);
    };
    let role = MembershipRole::parse(&role_str)?;
    Ok((iid, role))
}

#[cfg(test)]
mod avg_window_parity_tests {
    //! PIN C7 (lado Rust) — las cotas y los defaults de las ventanas del promedio contra el
    //! fixture compartido con el frontend.
    //!
    //! Las cuatro cifras (`MIN`/`MAX_AVG_WINDOW_MONTHS` y los dos defaults) están **duplicadas a
    //! mano** en TypeScript: `clampWindowMonths` reimplementa la cota («acotados a 1–60 igual que
    //! el servidor») y los defaults aparecen DOS veces más en `apps/web/src/lib/fire.ts`
    //! (`defaultFireSettingsApi` y los fallbacks de `normalizeInstallationFireSettings`).
    //! `fire-parity.json` **no** las cubre: ese fixture pinea el cálculo FIRE, no estas ventanas.
    //!
    //! El fixture `avg-window-parity.json` es la fuente única. Su pareja al otro lado es
    //! `apps/web/src/lib/fire.avg-window.test.ts`. Si un lado cambia sin el otro, UN test falla —
    //! misma disciplina que `fire-parity.json`.
    //!
    //! Coste de no tenerlo: subir el techo del servidor a 120 deja al cliente devolviendo su
    //! `fallback` en silencio para todo lo que pase de 60, así que la UI enseñaría 12 donde el
    //! usuario guardó 90 — sin error, sin aviso, y con el servidor calculando el promedio con la
    //! ventana buena. Divergencia visible solo comparando dos pantallas.

    use super::{default_fire_settings, MAX_AVG_WINDOW_MONTHS, MIN_AVG_WINDOW_MONTHS};

    const FIXTURE_JSON: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/avg-window-parity.json"
    ));

    fn fixture() -> serde_json::Value {
        serde_json::from_str(FIXTURE_JSON).expect("avg-window-parity.json es JSON válido")
    }

    fn u32_field(f: &serde_json::Value, key: &str) -> u32 {
        f[key]
            .as_u64()
            .unwrap_or_else(|| {
                panic!(
                    "avg-window-parity.json no tiene el entero `{key}`. Si lo renombraste, \
                     actualiza LOS DOS consumidores: este test y \
                     apps/web/src/lib/fire.avg-window.test.ts"
                )
            })
            .try_into()
            .unwrap_or_else(|_| panic!("`{key}` no cabe en u32"))
    }

    #[test]
    fn avg_window_bounds_match_the_shared_fixture() {
        let f = fixture();
        assert_eq!(
            MIN_AVG_WINDOW_MONTHS,
            u32_field(&f, "min"),
            "MIN_AVG_WINDOW_MONTHS ya no coincide con `min` de \
             apps/api/tests/fixtures/avg-window-parity.json.\n    \
             Si el suelo cambió a propósito, actualiza A LA VEZ: (1) esta const; \
             (2) el fixture; (3) `clampWindowMonths` en apps/web/src/lib/fire.ts (el `n < 1`); \
             (4) el comentario «acotados a 1–60 igual que el servidor» de esa misma función.\n    \
             Si no cambió a propósito, el cliente y el servidor están acotando distinto: el \
             cliente descartará en silencio (cae al fallback) valores que el servidor acepta."
        );
        assert_eq!(
            MAX_AVG_WINDOW_MONTHS,
            u32_field(&f, "max"),
            "MAX_AVG_WINDOW_MONTHS ya no coincide con `max` de \
             apps/api/tests/fixtures/avg-window-parity.json.\n    \
             Actualiza A LA VEZ: (1) esta const; (2) el fixture; (3) el `n > 60` de \
             `clampWindowMonths` en apps/web/src/lib/fire.ts; (4) su comentario «1–60».\n    \
             Coste de divergir: con el techo del servidor por encima del del cliente, la SPA \
             devuelve su `fallback` (3 o 12) al normalizar la respuesta, así que el usuario ve \
             una ventana que NO es la que el servidor está usando para calcular su promedio."
        );
    }

    #[test]
    fn avg_window_defaults_match_the_shared_fixture() {
        let f = fixture();
        let d = default_fire_settings();
        assert_eq!(
            d.income_avg_window_months,
            u32_field(&f, "income_default_months"),
            "El default de la ventana de INGRESO divergió del fixture.\n    \
             Actualiza A LA VEZ: (1) `default_fire_settings()` aquí; (2) el fixture; \
             (3) `defaultFireSettingsApi()` en apps/web/src/lib/fire.ts; \
             (4) el fallback `clampWindowMonths(raw?.income_avg_window_months, 3)` de \
             `normalizeInstallationFireSettings` — es una TERCERA copia del mismo número."
        );
        assert_eq!(
            d.expense_avg_window_months,
            u32_field(&f, "expense_default_months"),
            "El default de la ventana de GASTO divergió del fixture.\n    \
             Actualiza A LA VEZ: (1) `default_fire_settings()` aquí; (2) el fixture; \
             (3) `defaultFireSettingsApi()` en apps/web/src/lib/fire.ts; \
             (4) el fallback `clampWindowMonths(raw?.expense_avg_window_months, 12)` de \
             `normalizeInstallationFireSettings`."
        );
    }

    #[test]
    fn the_defaults_live_inside_the_bounds_they_are_clamped_to() {
        // Invariante barata pero real: un default fuera de rango sería reescrito por el propio
        // `clamp` de `resolve_fire_settings`, y entonces el fixture describiría un número que la
        // instalación nunca llega a tener.
        let d = default_fire_settings();
        for (label, months) in [
            ("income", d.income_avg_window_months),
            ("expense", d.expense_avg_window_months),
        ] {
            assert!(
                (MIN_AVG_WINDOW_MONTHS..=MAX_AVG_WINDOW_MONTHS).contains(&months),
                "el default de la ventana de {label} ({months}) cae fuera de \
                 {MIN_AVG_WINDOW_MONTHS}..={MAX_AVG_WINDOW_MONTHS}: el `clamp` lo reescribiría y \
                 el fixture estaría documentando un valor inalcanzable"
            );
        }
    }
}
