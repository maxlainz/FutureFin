use crate::error::ApiError;
use crate::handlers::budget::{budget_line_removed_with_liability, BudgetLineRemoved};
use crate::handlers::installation::{installation_naive_today, require_installation_member};
use crate::handlers::membership::role_can_write;
use crate::handlers::person_view::{require_row_owner, LedgerView, LedgerViewQuery};
use crate::handlers::projection::{
    liability_monthly_payment, refresh_projection_after_mutation,
};
use crate::money::money_out;
use crate::handlers::session::require_session_user;
use crate::state::AppState;
use axum::extract::{Extension, Path, Query};
use axum::routing::{get, patch};
use axum::{Json, Router};
use axum_extra::extract::cookie::CookieJar;
use chrono::{Datelike, Months, NaiveDate};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use std::sync::Arc;
use utoipa::ToSchema;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum PaymentFrequency {
    Monthly,
    Weekly,
}

impl PaymentFrequency {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            PaymentFrequency::Monthly => "monthly",
            PaymentFrequency::Weekly => "weekly",
        }
    }

    pub(crate) fn parse(s: &str) -> Result<Self, ApiError> {
        match s.trim() {
            "monthly" => Ok(Self::Monthly),
            "weekly" => Ok(Self::Weekly),
            _ => Err(ApiError::BadRequest(
                "payment_frequency_invalid: payment_frequency must be monthly or weekly".into(),
            )),
        }
    }
}

/// Modelo de amortización del pasivo (4.2.0). Espejo del lado API de
/// `futurefin_engine::RepaymentModel`: los dos enums existen por separado a propósito —el engine
/// no conoce ni la columna SQL ni el wire, y este no conoce la recurrencia—, y `to_engine()` es el
/// único puente.
///
/// Serde `snake_case`, así que los literales del wire coinciden exactamente con los del CHECK de
/// `liabilities.repayment_model`. Un literal desconocido en un body HTTP lo rechaza **serde** con
/// un 422 (mismo comportamiento que `PaymentFrequency`); [`RepaymentModel::parse`] existe para el
/// camino MCP, donde el parámetro llega como `String` suelto y el error debe ser un 400 nuestro.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum RepaymentModel {
    /// Modelo histórico (pre-4.2.0) y default de la columna: la cuota va íntegra a principal.
    #[default]
    FixedPayments,
    /// Sistema francés: interés sobre el saldo de apertura, cuota a fin de mes.
    French,
    /// Solo intereses: la cuota paga el devengo y el principal queda constante.
    InterestOnly,
    /// Línea revolving: misma recurrencia que el francés en 4.2.0.
    Revolving,
}

impl RepaymentModel {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            RepaymentModel::FixedPayments => "fixed_payments",
            RepaymentModel::French => "french",
            RepaymentModel::InterestOnly => "interest_only",
            RepaymentModel::Revolving => "revolving",
        }
    }

    /// Parseo desde el literal del wire/SQL. Lo usan `row_to_response` (donde el CHECK ya
    /// garantiza el dominio) y las tools MCP (donde el usuario puede escribir cualquier cosa).
    pub(crate) fn parse(s: &str) -> Result<Self, ApiError> {
        match s.trim() {
            "fixed_payments" => Ok(Self::FixedPayments),
            "french" => Ok(Self::French),
            "interest_only" => Ok(Self::InterestOnly),
            "revolving" => Ok(Self::Revolving),
            _ => Err(ApiError::BadRequest(
                "repayment_model_invalid: repayment_model must be one of fixed_payments, french, interest_only, revolving".into(),
            )),
        }
    }

    /// Puente al enum del engine. Es la ÚNICA traducción: si algún día divergen los literales,
    /// diverge aquí y en ningún otro sitio.
    pub(crate) fn to_engine(self) -> futurefin_engine::RepaymentModel {
        match self {
            RepaymentModel::FixedPayments => futurefin_engine::RepaymentModel::FixedPayments,
            RepaymentModel::French => futurefin_engine::RepaymentModel::French,
            RepaymentModel::InterestOnly => futurefin_engine::RepaymentModel::InterestOnly,
            RepaymentModel::Revolving => futurefin_engine::RepaymentModel::Revolving,
        }
    }

    /// ¿Devenga intereses este modelo, y por tanto exige un TIN configurado? Desde la Ola 3
    /// (#144) `interest_only` DERIVA la cuota del TIN (saldo × TIN/1200): sin TIN cobraría 0 €
    /// en silencio — todos menos el préstamo sin intereses lo exigen.
    fn requires_apr(self) -> bool {
        self != RepaymentModel::FixedPayments
    }
}

impl std::fmt::Display for RepaymentModel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Serialize, ToSchema)]
pub struct LiabilityResponse {
    #[schema(value_type = String, format = "uuid")]
    pub id: Uuid,
    #[schema(value_type = String, format = "uuid")]
    pub category_id: Uuid,
    /// Categoría de GASTO donde vive la cuota (atribución en presupuesto y comparativa).
    /// `null` solo en pasivos anteriores a 3.4.0 aún sin asignar — la API la exige al crear.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>, format = "uuid")]
    pub expense_category_id: Option<Uuid>,
    pub label: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub type_tag: Option<String>,
    pub principal_derived_from_plan: bool,
    /// Modelo de amortización (4.2.0). **Siempre presente**: la columna es NOT NULL con default
    /// `fixed_payments`, así que no hay pasivo sin modelo ni razón para omitirlo del wire.
    pub repayment_model: RepaymentModel,
    #[serde(with = "rust_decimal::serde::str")]
    #[schema(value_type = String)]
    pub principal: Decimal,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>)]
    pub apr_percent: Option<Decimal>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>)]
    pub payment_amount: Option<Decimal>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub payment_frequency: Option<PaymentFrequency>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>, format = "date")]
    pub payment_end_date: Option<NaiveDate>,
    /// Cuota mínima revolving (% del saldo de apertura). `null` en los demás modelos.
    #[serde(with = "rust_decimal::serde::str_option")]
    #[schema(value_type = Option<String>)]
    pub min_payment_pct: Option<Decimal>,
    /// Suelo en euros de la cuota mínima revolving. `null` en los demás modelos.
    #[serde(with = "rust_decimal::serde::str_option")]
    #[schema(value_type = Option<String>)]
    pub min_payment_eur: Option<Decimal>,
    /// «Plan vencido con saldo» (#145): `payment_end_date < hoy` y `principal > 0`. La deuda no
    /// se extinguió por calendario — el banco reclama, refinancia o lleva a impagado el residuo;
    /// aquí sigue visible, congelada y marcada. `false` para todo pasivo con plan vivo o saldado.
    pub plan_expired_with_balance: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
    pub sort_index: i32,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateLiabilityBody {
    #[schema(value_type = String, format = "uuid")]
    pub category_id: Uuid,
    /// Categoría de GASTO de la cuota — obligatoria desde 3.4.0 (sin `#[serde(default)]`:
    /// ausente = rechazo). El import de backups NO pasa por aquí (INSERT directo) y los
    /// pasivos previos conservan NULL hasta que el usuario la asigne por PATCH.
    #[schema(value_type = String, format = "uuid")]
    pub expense_category_id: Uuid,
    pub label: String,
    #[serde(default)]
    pub type_tag: Option<String>,
    #[serde(default)]
    pub derive_principal_from_plan: Option<bool>,
    /// Modelo de amortización. Ausente = `fixed_payments` (el histórico): un cliente que no sepa
    /// nada de 4.2.0 sigue creando exactamente los mismos pasivos que antes.
    #[serde(default)]
    pub repayment_model: Option<RepaymentModel>,
    #[serde(default)]
    #[serde(with = "rust_decimal::serde::str_option")]
    #[schema(value_type = Option<String>)]
    pub principal: Option<Decimal>,
    #[serde(default)]
    #[serde(with = "rust_decimal::serde::str_option")]
    #[schema(value_type = Option<String>)]
    pub apr_percent: Option<Decimal>,
    #[serde(default)]
    #[serde(with = "rust_decimal::serde::str_option")]
    #[schema(value_type = Option<String>)]
    pub payment_amount: Option<Decimal>,
    #[serde(default)]
    pub payment_frequency: Option<PaymentFrequency>,
    #[serde(default)]
    pub payment_end_date: Option<NaiveDate>,
    /// Cuota mínima revolving: % del saldo de apertura (0-100). Solo con `revolving`.
    #[serde(default)]
    #[serde(with = "rust_decimal::serde::str_option")]
    #[schema(value_type = Option<String>)]
    pub min_payment_pct: Option<Decimal>,
    /// Suelo en euros de la cuota mínima revolving. Solo con `revolving`.
    #[serde(default)]
    #[serde(with = "rust_decimal::serde::str_option")]
    #[schema(value_type = Option<String>)]
    pub min_payment_eur: Option<Decimal>,
    #[serde(default)]
    pub notes: Option<String>,
    #[serde(default)]
    pub sort_index: Option<i32>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct PatchLiabilityBody {
    #[serde(default)]
    #[schema(value_type = Option<String>, format = "uuid")]
    pub category_id: Option<Uuid>,
    /// Set-only (sin clear): asignar o cambiar la categoría de gasto de la cuota; `None` = sin
    /// tocar. Los NULL legacy solo salen de NULL asignando — nunca se vuelve a NULL vía API.
    #[serde(default)]
    #[schema(value_type = Option<String>, format = "uuid")]
    pub expense_category_id: Option<Uuid>,
    pub label: Option<String>,
    pub type_tag: Option<String>,
    #[serde(default)]
    pub derive_principal_from_plan: Option<bool>,
    /// Set-only (sin clear): `None` conserva el modelo actual. No hay «volver a NULL» porque la
    /// columna es NOT NULL — para deshacer se manda `fixed_payments` explícito (y desde #144,
    /// con `apr_percent: null` en el mismo PATCH si la fila tenía TIN).
    #[serde(default)]
    pub repayment_model: Option<RepaymentModel>,
    #[serde(default)]
    #[serde(with = "rust_decimal::serde::str_option")]
    #[schema(value_type = Option<String>)]
    pub principal: Option<Decimal>,
    /// Tri-estado desde #144 (mismo patrón que `purchase_price` en assets): ausente conserva,
    /// `null` LIMPIA el TIN, string decimal lo cambia. El clear existe porque volver a
    /// `fixed_payments` exige soltar el TIN en el mismo PATCH (`apr_forbidden_for_model`).
    #[serde(default, deserialize_with = "crate::handlers::deserialize_double_option")]
    #[schema(value_type = Option<Object>, nullable = true)]
    pub apr_percent: Option<serde_json::Value>,
    #[serde(default)]
    #[serde(with = "rust_decimal::serde::str_option")]
    #[schema(value_type = Option<String>)]
    pub payment_amount: Option<Decimal>,
    pub payment_frequency: Option<PaymentFrequency>,
    pub payment_end_date: Option<NaiveDate>,
    /// Cuota mínima revolving (% del saldo). Set-only; solo con `revolving`.
    #[serde(default)]
    #[serde(with = "rust_decimal::serde::str_option")]
    #[schema(value_type = Option<String>)]
    pub min_payment_pct: Option<Decimal>,
    /// Suelo en euros de la cuota mínima revolving. Set-only; solo con `revolving`.
    #[serde(default)]
    #[serde(with = "rust_decimal::serde::str_option")]
    #[schema(value_type = Option<String>)]
    pub min_payment_eur: Option<Decimal>,
    pub notes: Option<String>,
    pub sort_index: Option<i32>,
}

#[derive(Debug, FromRow)]
struct LiabilityRow {
    id: Uuid,
    category_id: Uuid,
    expense_category_id: Option<Uuid>,
    label: String,
    type_tag: Option<String>,
    principal: Decimal,
    apr_percent: Option<Decimal>,
    payment_amount: Option<Decimal>,
    payment_frequency: Option<String>,
    payment_end_date: Option<NaiveDate>,
    notes: Option<String>,
    sort_index: i32,
    principal_derived_from_plan: bool,
    repayment_model: String,
    min_payment_pct: Option<Decimal>,
    min_payment_eur: Option<Decimal>,
    /// `NOT NULL` desde la migración 5.0.0 (D14). Lo lee la puerta de D21 del PATCH.
    owner_user_id: Uuid,
}

fn normalize_label(raw: &str) -> Result<String, ApiError> {
    let t = raw.trim();
    if t.is_empty() {
        return Err(ApiError::BadRequest(
            "label_empty: label must not be empty".into(),
        ));
    }
    if t.len() > 200 {
        return Err(ApiError::BadRequest(
            "label_too_long: label must be at most 200 characters".into(),
        ));
    }
    Ok(t.into())
}

fn normalize_type_tag(raw: &Option<String>) -> Result<Option<String>, ApiError> {
    match raw {
        None => Ok(None),
        Some(s) => {
            let t = s.trim();
            if t.is_empty() {
                return Ok(None);
            }
            if t.len() > 120 {
                return Err(ApiError::BadRequest(
                    "type_tag_too_long: type_tag must be at most 120 characters".into(),
                ));
            }
            Ok(Some(t.into()))
        }
    }
}

fn normalize_notes(raw: &Option<String>) -> Result<Option<String>, ApiError> {
    match raw {
        None => Ok(None),
        Some(s) => {
            let t = s.trim();
            if t.is_empty() {
                return Ok(None);
            }
            if t.len() > 4000 {
                return Err(ApiError::BadRequest(
                    "notes_too_long: notes must be at most 4000 characters".into(),
                ));
            }
            Ok(Some(t.into()))
        }
    }
}

/// Cota superior del TIN (auditoría 2026-08, S3/#135): ningún tipo real de mercado se acerca
/// (usura ≈ TEDR+6pp ≈ 27 % TAE); el vector real es el desliz de coma es-ES (350 por 3,50), que
/// antes entraba, hacía crecer el saldo ×1,29/mes y acababa en el overflow tipado del engine.
/// Mejor rechazarlo en la puerta con nombre que dejarlo llegar al motor.
fn assert_apr_percent_range(d: Decimal) -> Result<(), ApiError> {
    if d > Decimal::from(100) {
        return Err(ApiError::BadRequest(
            "apr_out_of_range: apr_percent must be between 0 and 100".into(),
        ));
    }
    Ok(())
}

fn assert_non_negative(d: Decimal, field: &'static str) -> Result<(), ApiError> {
    if d.is_sign_negative() {
        return Err(ApiError::BadRequest(format!("amount_negative: {field} must be >= 0")));
    }
    Ok(())
}

fn validate_payment_pair(
    amount: Option<Decimal>,
    freq: Option<&str>,
) -> Result<(), ApiError> {
    match (amount, freq) {
        (None, None) => Ok(()),
        (Some(a), Some(f)) => {
            if a <= Decimal::ZERO {
                return Err(ApiError::BadRequest(
                    "payment_amount_not_positive: payment_amount must be > 0 when set".into(),
                ));
            }
            PaymentFrequency::parse(f)?;
            Ok(())
        }
        _ => Err(ApiError::BadRequest(
            "payment_pair_incomplete: payment_amount and payment_frequency must both be set or both omitted"
                .into(),
        )),
    }
}

/// Coherencia del **estado resultante** (4.2.0): qué combinaciones de modelo, TIN y plan de pago
/// tienen sentido económico. Se llama con los valores YA mergeados —en el PATCH, tras aplicar el
/// body sobre la fila actual—, porque las tres reglas hablan del pasivo que va a quedar guardado,
/// no de los campos que llegaron en la petición.
///
/// Las cuatro condiciones (orden determinista, para que el código de error sea predecible):
/// 1. `payment_plan_required_for_model` — sin cuota no hay ni interés ni amortización: el engine
///    exige plan activo para devengar (`liability_active`), así que un `french` sin cuota sería un
///    `fixed_payments` disfrazado que no mueve un solo número. Se rechaza en vez de mentir.
/// 2. `apr_required_for_model` — todo modelo salvo `fixed_payments` **exige** TIN > 0 (desde
///    #144 `interest_only` también: su cuota ES el interés del período, saldo × TIN/1200, y sin
///    TIN pagaría 0 € con la deuda congelada). Un TIN ausente o 0 degenera en la recurrencia
///    sin intereses: guardarlo sería ofrecer un «francés» que no cobra.
/// 3. `weekly_not_supported_for_model` — la recurrencia del engine es MENSUAL. Con `weekly` el
///    handler convierte la cuota a su equivalente mensual (×52/12), lo que para un modelo sin
///    intereses es exacto pero para uno que devenga cambiaría el devengo. No se admite.
/// 4. `derive_not_supported_for_model` — derivar el principal del plan solo tiene inversa cerrada
///    en `fixed_payments` (Σ cuotas) y `french` (valor actual). En `interest_only` la cuota no
///    amortiza (el principal no se deduce del plan) y en `revolving` el plan no describe un
///    calendario cerrado.
///
/// **Las cuatro se evalúan SIEMPRE antes de decidir el error** (4.4.0). Hasta entonces se devolvía
/// la primera y se salía: un `french` sin plan ni TIN gastaba tres turnos —
/// `payment_plan_required_for_model` → (añades la cuota) → `apr_required_for_model` → (añades el
/// TIN) → 201 —, y un `revolving` con `derive` los gastaba cuatro. El servidor conoce las cuatro
/// condiciones desde la PRIMERA llamada; devolverlas de una en una no es informar menos, es
/// **invitar a rellenar el hueco**: para un agente cada rebote es una oportunidad de inventarse un
/// TIN plausible para desatascarse, y aquí un TIN inventado mueve la amortización entera.
///
/// La política de códigos es deliberadamente conservadora:
/// - **Exactamente un problema** → el código específico de siempre, con su mensaje intacto. Es el
///   caso de todo lo que hoy existe (fixture `error-codes.json`, frases de `errorMessages.ts`,
///   `liabilities_repayment_model.rs`): cero ruptura y el código más accionable posible.
/// - **Dos o más** → el código nuevo `repayment_model_state_invalid`, cuyo mensaje ENUMERA todos.
///   Es exactamente el caso que antes mentía por omisión, así que no hay comportamiento anterior
///   que preservar: sustituye a un código que describía *parte* del problema.
///
/// El prefijo de cada mensaje va como literal (aunque el resto se componga con `format!`):
/// `error_codes_parity.rs` extrae el código del literal del fuente, y un prefijo interpolado sería
/// invisible para el catálogo.
fn validate_repayment_model_state(
    model: RepaymentModel,
    apr: Option<Decimal>,
    payment_amount: Option<Decimal>,
    payment_frequency: Option<PaymentFrequency>,
    derive: bool,
    min_payment_pct: Option<Decimal>,
    min_payment_eur: Option<Decimal>,
) -> Result<(), ApiError> {
    // Desde la Ola 3 (#144) el modelo sin intereses TAMBIÉN valida: un TIN sobre él era un
    // número inmóvil que el engine ignoraba — el préstamo gratis silencioso que la auditoría
    // señaló. «Rechazar, no defaultear» (§2.6).
    if model == RepaymentModel::FixedPayments {
        let mut problems: Vec<(String, &'static str)> = Vec::new();
        if matches!(apr, Some(a) if a > Decimal::ZERO) {
            problems.push((
                "apr_forbidden_for_model: repayment_model fixed_payments is an interest-free loan (0 %) — remove apr_percent or choose french/interest_only/revolving".to_string(),
                "apr_percent must be absent",
            ));
        }
        if min_payment_pct.is_some() || min_payment_eur.is_some() {
            problems.push((
                format!("revolving_minimum_forbidden_for_model: min_payment_pct/min_payment_eur only apply to repayment_model revolving (got {model})"),
                "min_payment_pct/min_payment_eur must be absent",
            ));
        }
        return match problems.len() {
            0 => Ok(()),
            1 => Err(ApiError::BadRequest(problems.remove(0).0)),
            _ => Err(ApiError::BadRequest(format!(
                "repayment_model_state_invalid: repayment_model {model} needs all of these fixed at once: {}",
                problems.iter().map(|(_, sh)| *sh).collect::<Vec<_>>().join("; ")
            ))),
        };
    }

    // `.0` = mensaje completo del código específico (el que se devuelve si es el ÚNICO problema);
    // `.1` = la exigencia en corto, para enumerarlas todas cuando hay más de una. Ninguna de las
    // frases cortas lleva ": ", para no fabricar códigos fantasma en el extractor de paridad.
    let mut problems: Vec<(String, &'static str)> = Vec::new();

    if payment_amount.is_none() || payment_frequency.is_none() {
        problems.push((
            format!(
                "payment_plan_required_for_model: repayment_model {model} requires payment_amount and payment_frequency"
            ),
            "payment_amount and payment_frequency are required",
        ));
    }

    if model.requires_apr() && !matches!(apr, Some(a) if a > Decimal::ZERO) {
        problems.push((
            format!("apr_required_for_model: repayment_model {model} requires apr_percent > 0"),
            "apr_percent > 0 is required",
        ));
    }

    if payment_frequency == Some(PaymentFrequency::Weekly) {
        problems.push((
            format!(
                "weekly_not_supported_for_model: payment_frequency weekly is only supported with repayment_model fixed_payments (got {model})"
            ),
            "payment_frequency must not be weekly",
        ));
    }

    if model == RepaymentModel::Revolving
        && !(matches!(min_payment_pct, Some(p) if p > Decimal::ZERO)
            || matches!(min_payment_eur, Some(e) if e > Decimal::ZERO))
    {
        problems.push((
            "revolving_minimum_required: repayment_model revolving needs min_payment_pct > 0 or min_payment_eur > 0 — the minimum instalment is a percentage of the balance with a floor in euros, not a fixed quota".to_string(),
            "min_payment_pct > 0 or min_payment_eur > 0 is required",
        ));
    }

    if model != RepaymentModel::Revolving && (min_payment_pct.is_some() || min_payment_eur.is_some()) {
        problems.push((
            format!("revolving_minimum_forbidden_for_model: min_payment_pct/min_payment_eur only apply to repayment_model revolving (got {model})"),
            "min_payment_pct/min_payment_eur must be absent",
        ));
    }

    if derive && matches!(model, RepaymentModel::InterestOnly | RepaymentModel::Revolving) {
        problems.push((
            format!(
                "derive_not_supported_for_model: derive_principal_from_plan is not supported with repayment_model {model}"
            ),
            "derive_principal_from_plan must be false",
        ));
    }

    match problems.len() {
        0 => Ok(()),
        1 => Err(ApiError::BadRequest(problems.remove(0).0)),
        _ => {
            let all = problems
                .iter()
                .map(|(_, short)| *short)
                .collect::<Vec<_>>()
                .join("; ");
            Err(ApiError::BadRequest(format!(
                "repayment_model_state_invalid: repayment_model {model} needs all of these fixed at once: {all}"
            )))
        }
    }
}

/// Principal = payment × intervals from **today**
/// (installation `calendar_tz` civil date) through `payment_end_date` inclusive — monthly steps one
/// calendar month at a time; weekly uses ceil(inclusive_days / 7).
fn payment_interval_count(
    frequency: PaymentFrequency,
    start: NaiveDate,
    end: NaiveDate,
) -> Result<u32, ApiError> {
    if end < start {
        return Err(ApiError::BadRequest(
            "payment_end_date_in_past: payment_end_date must be on or after today when deriving principal".into(),
        ));
    }
    match frequency {
        PaymentFrequency::Monthly => {
            // #123: cada vencimiento se recalcula DESDE EL ANCLA (`start + n meses`), nunca
            // encadenando el resultado del paso anterior. `checked_add_months` recorta al fin
            // del mes de destino, y encadenarlo degradaba el día ancla para siempre al pasar
            // por un mes corto: con ancla 31 el recibo real gira a fin de CADA mes (12 cuotas
            // en un año), pero la cadena degradada contaba 13 — 1.000 € de deuda inventada por
            // año en el escenario del issue. Anclado, `start + 7` desde el 31-08 vuelve a caer
            // en el 31-03: el día de cargo no se degrada tras febrero, como en la realidad.
            let mut n = 0u32;
            loop {
                let d = start.checked_add_months(Months::new(n)).ok_or_else(|| {
                    ApiError::BadRequest("payment_schedule_overflow: payment schedule date overflow".into())
                })?;
                if d > end {
                    break;
                }
                n += 1;
                if n > 1200 {
                    return Err(ApiError::BadRequest(
                        "payment_schedule_too_long: too many monthly payment intervals".into(),
                    ));
                }
            }
            Ok(n)
        }
        PaymentFrequency::Weekly => {
            let days = end.signed_duration_since(start).num_days() + 1;
            let days = days.max(1);
            let intervals = ((days + 6) / 7) as u32;
            if intervals > 5200 {
                return Err(ApiError::BadRequest(
                    "payment_schedule_too_long: too many weekly payment intervals".into(),
                ));
            }
            Ok(intervals)
        }
    }
}

/// Principal derivado del plan de pago. Desde 4.2.0 depende del **modelo**:
///
/// - `fixed_payments` → `Σ cuotas` = `cuota × n`, EXACTO y bit a bit igual al comportamiento
///   pre-4.2.0. No pasa por el engine ni por `round_dp`: el contrato histórico no se toca.
/// - `french` → **valor actual** de la renta al TIN (`present_value_of_payments`), que es el
///   capital pendiente de verdad: 200 cuotas de 500 € al 3 % son 100.000 € de caja pero solo
///   ~78.618 € de deuda hoy. `n` va en MESES; `weekly` no llega nunca aquí (lo rechaza antes
///   `weekly_not_supported_for_model`), así que no hay caso fraccionario.
/// - `interest_only` / `revolving` no llegan: `derive_not_supported_for_model` los ha rechazado.
///
/// El redondeo a 4 decimales (`MidpointAwayFromZero`, la escala de money del proyecto) vive AQUÍ
/// y no en el engine: el engine devuelve el valor exacto de la fórmula y es el handler quien lo
/// ajusta a la escala de la columna.
fn derive_principal_from_payment_plan(
    payment_amount: Decimal,
    frequency: PaymentFrequency,
    payment_end_date: NaiveDate,
    today: NaiveDate,
    apr_percent: Option<Decimal>,
) -> Result<Decimal, ApiError> {
    let n = payment_interval_count(frequency, today, payment_end_date)?;
    if n == 0 {
        return Err(ApiError::BadRequest(
            "payment_schedule_empty: derived principal requires at least one payment interval".into(),
        ));
    }
    // Rama única desde la Ola 3 (#144/#121): el principal derivado es SIEMPRE el valor actual
    // de las cuotas pendientes al TIN. Con `fixed_payments` el TIN es imposible por validación
    // (apr_forbidden_for_model) y la migración anuló el residuo, así que `apr_percent` llega
    // None y el helper devuelve Σ cuotas — bit-idéntico al comportamiento anterior. El `model`
    // ya no decide nada aquí; la firma ya no recibe el modelo.
    Ok(futurefin_engine::present_value_of_payments(
        payment_amount,
        Decimal::from(n),
        apr_percent,
    )
    .round_dp_with_strategy(4, rust_decimal::RoundingStrategy::MidpointAwayFromZero))
}

async fn assert_liability_category(
    pool: &sqlx::PgPool,
    installation_id: Uuid,
    category_id: Uuid,
) -> Result<(), ApiError> {
    let ok: bool = sqlx::query_scalar(
        r#"SELECT EXISTS (
            SELECT 1 FROM categories
            WHERE
                id = $1
                AND installation_id = $2
                AND scope = 'liability'
        )"#,
    )
    .bind(category_id)
    .bind(installation_id)
    .fetch_one(pool)
    .await?;

    if !ok {
        return Err(ApiError::BadRequest(
            "category_wrong_scope: category_id must reference a liability category in this installation".into(),
        ));
    }
    Ok(())
}

/// Espejo de `assert_liability_category` para la categoría de GASTO de la cuota (3.4.0): debe
/// existir, ser de esta instalación y tener scope `expense` — es la categoría a la que el
/// presupuesto y la comparativa de Movimientos atribuyen el equivalente mensual del plan.
async fn assert_expense_category(
    pool: &sqlx::PgPool,
    installation_id: Uuid,
    category_id: Uuid,
) -> Result<(), ApiError> {
    let ok: bool = sqlx::query_scalar(
        r#"SELECT EXISTS (
            SELECT 1 FROM categories
            WHERE
                id = $1
                AND installation_id = $2
                AND scope = 'expense'
        )"#,
    )
    .bind(category_id)
    .bind(installation_id)
    .fetch_one(pool)
    .await?;

    if !ok {
        return Err(ApiError::BadRequest(
            "expense_category_wrong_scope: expense_category_id must reference an expense category in this installation".into(),
        ));
    }
    Ok(())
}

fn row_to_response(r: LiabilityRow, today: NaiveDate) -> Result<LiabilityResponse, ApiError> {
    let payment_frequency = match r.payment_frequency.as_deref() {
        None => None,
        Some(s) => Some(PaymentFrequency::parse(s)?),
    };
    Ok(LiabilityResponse {
        id: r.id,
        category_id: r.category_id,
        expense_category_id: r.expense_category_id,
        label: r.label,
        type_tag: r.type_tag,
        principal_derived_from_plan: r.principal_derived_from_plan,
        // El CHECK de la columna acota el dominio, así que un `parse` fallido aquí solo puede
        // venir de una fila manipulada fuera de la API. Se propaga el 400 en vez de inventar un
        // default: si la BD miente, mejor un error visible que un número silenciosamente distinto.
        repayment_model: RepaymentModel::parse(&r.repayment_model)?,
        principal: r.principal,
        apr_percent: r.apr_percent,
        payment_amount: r.payment_amount,
        payment_frequency,
        plan_expired_with_balance: matches!(r.payment_end_date, Some(end) if end < today)
            && r.principal > Decimal::ZERO,
        payment_end_date: r.payment_end_date,
        min_payment_pct: r.min_payment_pct,
        min_payment_eur: r.min_payment_eur,
        notes: r.notes,
        sort_index: r.sort_index,
    })
}

#[utoipa::path(
    get,
    path = "/v1/liabilities",
    tag = "liabilities",
    params(
        ("view" = Option<String>, Query, description = "`mine` (default: `view` omitido o vacío) = filas atribuidas al usuario de la sesión; `household` = hogar completo, y hay que pedirlo EXPLÍCITAMENTE desde 5.0.0. Cualquier otro valor → 400 `invalid_view`."),
    ),
    responses(
        (status = 200, description = "Liabilities visibles: plan de pago vivo o saldo vivo (#145); el vencido con saldo viaja marcado plan_expired_with_balance. Nunca borra nada.", body = [LiabilityResponse]),
        (status = 401, description = "No valid session"),
        (status = 403, description = "Not an installation member"),
        (status = 404, description = "Installation missing"),
    )
)]
pub async fn list_liabilities(
    Extension(state): Extension<Arc<AppState>>,
    jar: CookieJar,
    Query(q): Query<LedgerViewQuery>,
) -> Result<Json<Vec<LiabilityResponse>>, ApiError> {
    let user = require_session_user(&jar, &state.pool).await?;
    let (iid, _) = require_installation_member(&state.pool, user.id.0).await?;
    let out = list_liabilities_core(&state.pool, iid, user.id.0, q.resolve()?).await?;
    Ok(Json(out))
}

/// Core sin HTTP: lo comparten el handler GET y la tool MCP `list_liabilities`.
pub(crate) async fn list_liabilities_core(
    pool: &sqlx::PgPool,
    iid: Uuid,
    user_id: Uuid,
    view: LedgerView,
) -> Result<Vec<LiabilityResponse>, ApiError> {
    let today = installation_naive_today(pool, iid).await?;

    let scope = view.scope_where("");
    let today_ph = view.next_arg_index();
    let sql = format!(
        r#"SELECT id, category_id, expense_category_id, label, type_tag, principal, apr_percent,
                  payment_amount, payment_frequency, payment_end_date, notes,
                  sort_index, principal_derived_from_plan, repayment_model, min_payment_pct,
                  min_payment_eur, owner_user_id
           FROM liabilities
           WHERE {scope}
             AND (payment_end_date IS NULL OR payment_end_date >= ${today_ph} OR principal > 0)
           ORDER BY sort_index ASC, label ASC"#
    );
    let rows: Vec<LiabilityRow> = view
        .bind_scope_as(sqlx::query_as(&sql), iid, user_id)
        .bind(today)
        .fetch_all(pool)
        .await?;

    let mut out = Vec::with_capacity(rows.len());
    for r in rows {
        out.push(row_to_response(r, today)?);
    }
    Ok(out)
}

#[utoipa::path(
    post,
    path = "/v1/liabilities",
    tag = "liabilities",
    request_body = CreateLiabilityBody,
    responses(
        (status = 201, description = "Created", body = LiabilityResponse),
        (status = 400, description = "Validation error"),
        (status = 401, description = "No valid session"),
        (status = 403, description = "Viewer or not a member"),
        (status = 404, description = "Installation missing"),
    )
)]
pub async fn create_liability(
    Extension(state): Extension<Arc<AppState>>,
    jar: CookieJar,
    Json(body): Json<CreateLiabilityBody>,
) -> Result<(axum::http::StatusCode, Json<LiabilityResponse>), ApiError> {
    let user = require_session_user(&jar, &state.pool).await?;
    let (iid, role) = require_installation_member(&state.pool, user.id.0).await?;
    if !role_can_write(role.as_str()) {
        return Err(ApiError::Forbidden);
    }
    let resp = create_liability_core(&state, iid, user.id.0, body).await?;
    Ok((axum::http::StatusCode::CREATED, Json(resp)))
}

/// Core sin HTTP: lo comparten el handler POST y la tool MCP `create_liability`. Dos modos:
/// `principal` explícito o `derive_principal_from_plan` (cuota + frecuencia + fecha fin →
/// principal = Σ cuotas en `fixed_payments`, valor actual al TIN en `french`). Invalidación FULL
/// dentro.
/// Cota superior de `payment_end_date`. Literal completo en el error a propósito: ver la nota en
/// `handlers::max_user_settable_future_date`.
async fn validate_payment_end_date_range(
    pool: &sqlx::PgPool,
    iid: Uuid,
    payment_end_date: Option<NaiveDate>,
) -> Result<(), ApiError> {
    let Some(d) = payment_end_date else {
        return Ok(());
    };
    let today = crate::handlers::installation::installation_naive_today(pool, iid).await?;
    if d > crate::handlers::max_user_settable_future_date(today) {
        return Err(ApiError::BadRequest(
            "payment_end_date_out_of_range: payment_end_date must not be more than 100 years in the future".into(),
        ));
    }
    Ok(())
}

pub(crate) async fn create_liability_core(
    state: &Arc<AppState>,
    iid: Uuid,
    user_id: Uuid,
    body: CreateLiabilityBody,
) -> Result<LiabilityResponse, ApiError> {
    assert_liability_category(&state.pool, iid, body.category_id).await?;
    assert_expense_category(&state.pool, iid, body.expense_category_id).await?;
    validate_payment_end_date_range(&state.pool, iid, body.payment_end_date).await?;

    let label = normalize_label(&body.label)?;
    let type_tag = normalize_type_tag(&body.type_tag)?;
    let derive = body.derive_principal_from_plan.unwrap_or(false);
    let freq_str = body.payment_frequency.map(|f| f.as_str().to_string());
    let model = body.repayment_model.unwrap_or_default();

    // Coherencia modelo↔TIN↔plan ANTES de derivar: la derivación en `french` usa el TIN, así que
    // no puede correr sobre un estado que vamos a rechazar.
    validate_repayment_model_state(
        model,
        body.apr_percent,
        body.payment_amount,
        body.payment_frequency,
        derive,
        body.min_payment_pct,
        body.min_payment_eur,
    )?;

    let (principal, principal_derived) = if derive {
        let amt = body.payment_amount.ok_or_else(|| {
            ApiError::BadRequest(
                "payment_amount_required_for_derived_principal: payment_amount is required when derive_principal_from_plan is true".into(),
            )
        })?;
        let fs = freq_str.as_deref().ok_or_else(|| {
            ApiError::BadRequest(
                "payment_frequency_required_for_derived_principal: payment_frequency is required when derive_principal_from_plan is true".into(),
            )
        })?;
        let end = body.payment_end_date.ok_or_else(|| {
            ApiError::BadRequest(
                "payment_end_date_required_for_derived_principal: payment_end_date is required when derive_principal_from_plan is true".into(),
            )
        })?;
        validate_payment_pair(Some(amt), Some(fs))?;
        let pf = PaymentFrequency::parse(fs)?;
        let today = installation_naive_today(&state.pool, iid).await?;
        (
            derive_principal_from_payment_plan(amt, pf, end, today, body.apr_percent)?,
            true,
        )
    } else {
        let p = body.principal.ok_or_else(|| {
            ApiError::BadRequest(
                "principal_required: principal is required unless derive_principal_from_plan is true".into(),
            )
        })?;
        assert_non_negative(p, "principal")?;
        validate_payment_pair(body.payment_amount, freq_str.as_deref())?;
        (p, false)
    };

    if let Some(apr) = body.apr_percent {
        assert_non_negative(apr, "apr_percent")?;
        assert_apr_percent_range(apr)?;
    }

    let notes = normalize_notes(&body.notes)?;
    let sort_index = body.sort_index.unwrap_or(0);

    let row: LiabilityRow = sqlx::query_as(
        r#"INSERT INTO liabilities (
               installation_id, category_id, expense_category_id, label, type_tag, principal,
               apr_percent, payment_amount, payment_frequency,
               payment_end_date, notes, sort_index, principal_derived_from_plan,
               owner_user_id, repayment_model, min_payment_pct, min_payment_eur
           )
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17)
           RETURNING id, category_id, expense_category_id, label, type_tag, principal, apr_percent,
                     payment_amount, payment_frequency, payment_end_date, notes,
                     sort_index, principal_derived_from_plan, repayment_model, min_payment_pct,
                     min_payment_eur, owner_user_id"#,
    )
    .bind(iid)
    .bind(body.category_id)
    .bind(body.expense_category_id)
    .bind(&label)
    .bind(&type_tag)
    .bind(principal)
    .bind(body.apr_percent)
    .bind(body.payment_amount)
    .bind(freq_str.as_deref())
    .bind(body.payment_end_date)
    .bind(&notes)
    .bind(sort_index)
    .bind(principal_derived)
    .bind(user_id)
    .bind(model.as_str())
    .bind(body.min_payment_pct)
    .bind(body.min_payment_eur)
    .fetch_one(&state.pool)
    .await?;

    refresh_projection_after_mutation(&state, iid, user_id).await;
    let today = installation_naive_today(&state.pool, iid).await?;
    row_to_response(row, today)
}

#[utoipa::path(
    patch,
    path = "/v1/liabilities/{id}",
    tag = "liabilities",
    request_body = PatchLiabilityBody,
    params(
        ("id" = Uuid, Path, description = "Liability id"),
    ),
    responses(
        (status = 200, description = "Updated", body = LiabilityResponse),
        (status = 400, description = "Validation error"),
        (status = 401, description = "No valid session"),
        (status = 403, description = "Viewer or not a member"),
        (status = 404, description = "Liability missing"),
    )
)]
pub async fn patch_liability(
    Extension(state): Extension<Arc<AppState>>,
    jar: CookieJar,
    Path(id): Path<Uuid>,
    Json(body): Json<PatchLiabilityBody>,
) -> Result<Json<LiabilityResponse>, ApiError> {
    let user = require_session_user(&jar, &state.pool).await?;
    let (iid, role) = require_installation_member(&state.pool, user.id.0).await?;
    if !role_can_write(role.as_str()) {
        return Err(ApiError::Forbidden);
    }
    let resp = patch_liability_core(&state, iid, user.id.0, id, body).await?;
    Ok(Json(resp))
}

/// Core sin HTTP: lo comparten el handler PATCH y la tool MCP `update_liability`. Merge campo a
/// campo sobre la fila actual; si `derive_principal_from_plan` queda activo, el principal se
/// rederiva del plan de pago. Invalidación FULL dentro.
///
/// **D21 (5.0.0)**: 403 `not_row_owner` si el pasivo es de otro miembro; 404 si no existe.
pub(crate) async fn patch_liability_core(
    state: &Arc<AppState>,
    iid: Uuid,
    user_id: Uuid,
    id: Uuid,
    body: PatchLiabilityBody,
) -> Result<LiabilityResponse, ApiError> {
    if body.category_id.is_none()
        && body.expense_category_id.is_none()
        && body.label.is_none()
        && body.type_tag.is_none()
        && body.derive_principal_from_plan.is_none()
        && body.repayment_model.is_none()
        && body.principal.is_none()
        && body.apr_percent.is_none()
        && body.payment_amount.is_none()
        && body.payment_frequency.is_none()
        && body.payment_end_date.is_none()
        && body.min_payment_pct.is_none()
        && body.min_payment_eur.is_none()
        && body.notes.is_none()
        && body.sort_index.is_none()
    {
        return Err(ApiError::BadRequest(
            "patch_empty: provide at least one field to update".into(),
        ));
    }

    let row: Option<LiabilityRow> = sqlx::query_as(
        r#"SELECT id, category_id, expense_category_id, label, type_tag, principal, apr_percent,
                  payment_amount, payment_frequency, payment_end_date, notes,
                  sort_index, principal_derived_from_plan, repayment_model, min_payment_pct,
                  min_payment_eur, owner_user_id
           FROM liabilities
           WHERE id = $1 AND installation_id = $2"#,
    )
    .bind(id)
    .bind(iid)
    .fetch_optional(&state.pool)
    .await?;

    let Some(current) = row else {
        return Err(ApiError::NotFound);
    };
    require_row_owner(current.owner_user_id, user_id)?;

    let new_cat = body.category_id.unwrap_or(current.category_id);
    if new_cat != current.category_id {
        assert_liability_category(&state.pool, iid, new_cat).await?;
    }

    // Set-only (sin clear): `None` conserva el valor actual — incluidos los NULL legacy, que solo
    // salen de NULL asignando una categoría. Revalidar solo cuando cambia.
    let new_expense_cat = body.expense_category_id.or(current.expense_category_id);
    if body.expense_category_id.is_some() && new_expense_cat != current.expense_category_id {
        assert_expense_category(&state.pool, iid, body.expense_category_id.unwrap()).await?;
    }

    let new_label = match &body.label {
        Some(s) => normalize_label(s)?,
        None => current.label.clone(),
    };

    let new_type_tag = match &body.type_tag {
        Some(_) => normalize_type_tag(&body.type_tag)?,
        None => current.type_tag.clone(),
    };

    let derived_flag = body
        .derive_principal_from_plan
        .unwrap_or(current.principal_derived_from_plan);

    let new_apr = match &body.apr_percent {
        None => current.apr_percent,
        Some(v) if v.is_null() => None,
        Some(serde_json::Value::String(raw)) => {
            let apr: Decimal = raw.trim().parse().map_err(|_| {
                ApiError::BadRequest(
                    "decimal_invalid: apr_percent must be a valid decimal string".into(),
                )
            })?;
            assert_non_negative(apr, "apr_percent")?;
            assert_apr_percent_range(apr)?;
            Some(apr)
        }
        // El wire de los importes es string decimal (§2.1); un número JSON aquí siempre fue 422
        // vía serde y el tri-estado no lo relaja.
        Some(_) => {
            return Err(ApiError::BadRequest(
                "decimal_invalid: apr_percent must be a valid decimal string".into(),
            ))
        }
    };

    // Set-only. Se resuelve ANTES del bloque de derivación a propósito: cambiar el modelo (o el
    // TIN de arriba) con `derive_principal_from_plan` activo debe **re-derivar** el principal con
    // los valores nuevos, no con los de la fila.
    let new_model = match body.repayment_model {
        Some(m) => m,
        None => RepaymentModel::parse(&current.repayment_model)?,
    };

    let new_pay_amt = body.payment_amount.or(current.payment_amount);
    let new_pay_freq_str = body
        .payment_frequency
        .map(|f| f.as_str().to_string())
        .or(current.payment_frequency.clone());

    validate_payment_pair(new_pay_amt, new_pay_freq_str.as_deref())?;

    // Sobre el estado RESULTANTE (mismo criterio que `validate_payment_pair` con los pares
    // incompletos): lo que se valida es el pasivo que va a quedar guardado.
    let new_pay_freq = new_pay_freq_str
        .as_deref()
        .map(PaymentFrequency::parse)
        .transpose()?;
    // Los mínimos son estado EXCLUSIVO de revolving: al salir del modelo caen solos (como el
    // TIN residual en la migración de #144) — sin esto el merge set-only arrastraría los
    // mínimos guardados, `revolving_minimum_forbidden_for_model` rechazaría el PATCH y la fila
    // quedaría atrapada en revolving para siempre. Volver a revolving exige re-declararlos.
    let (new_min_pct, new_min_eur) = if new_model == RepaymentModel::Revolving {
        (
            body.min_payment_pct.or(current.min_payment_pct),
            body.min_payment_eur.or(current.min_payment_eur),
        )
    } else {
        (body.min_payment_pct, body.min_payment_eur)
    };
    validate_repayment_model_state(
        new_model,
        new_apr,
        new_pay_amt,
        new_pay_freq,
        derived_flag,
        new_min_pct,
        new_min_eur,
    )?;

    let new_pay_end = body.payment_end_date.or(current.payment_end_date);
    // Solo lo que el patch INTRODUCE: una fila antigua fuera de cota sigue siendo editable.
    if body.payment_end_date.is_some() {
        validate_payment_end_date_range(&state.pool, iid, new_pay_end).await?;
    }

    let new_notes = match &body.notes {
        Some(_) => normalize_notes(&body.notes)?,
        None => current.notes.clone(),
    };

    let new_sort = body.sort_index.unwrap_or(current.sort_index);

    let new_principal_derived = derived_flag;

    // Re-derivar solo cuando el PATCH toca un INPUT de la derivación (modelo, TIN, cuota,
    // frecuencia, fecha fin o el propio flag). Es lo que el contrato promete («cambiar el
    // modelo o el TIN con derive activo RE-DERIVA») — y sin este gate, editar el LABEL de una
    // fila derivada con el plan ya vencido devolvía `payment_end_date_in_past`: una fila que
    // #145 volvió visible y editable quedaba atrapada por un campo que el PATCH no tocó.
    let touches_derivation_inputs = body.repayment_model.is_some()
        || body.apr_percent.is_some()
        || body.payment_amount.is_some()
        || body.payment_frequency.is_some()
        || body.payment_end_date.is_some()
        || body.derive_principal_from_plan.is_some()
        || body.principal.is_some();

    let new_principal = if derived_flag && touches_derivation_inputs {
        let amt = new_pay_amt.ok_or_else(|| {
            ApiError::BadRequest(
                "payment_amount_required_for_derived_principal: payment_amount is required when principal is derived from plan".into(),
            )
        })?;
        let fs = new_pay_freq_str.as_deref().ok_or_else(|| {
            ApiError::BadRequest(
                "payment_frequency_required_for_derived_principal: payment_frequency is required when principal is derived from plan".into(),
            )
        })?;
        let end = new_pay_end.ok_or_else(|| {
            ApiError::BadRequest(
                "payment_end_date_required_for_derived_principal: payment_end_date is required when principal is derived from plan".into(),
            )
        })?;
        let pf = PaymentFrequency::parse(fs)?;
        let today = installation_naive_today(&state.pool, iid).await?;
        derive_principal_from_payment_plan(amt, pf, end, today, new_apr)?
    } else {
        match body.principal {
            Some(p) => {
                assert_non_negative(p, "principal")?;
                p
            }
            None => current.principal,
        }
    };

    let updated: LiabilityRow = sqlx::query_as(
        r#"UPDATE liabilities
           SET category_id = $1,
               expense_category_id = $2,
               label = $3,
               type_tag = $4,
               principal = $5,
               apr_percent = $6,
               payment_amount = $7,
               payment_frequency = $8,
               payment_end_date = $9,
               notes = $10,
               sort_index = $11,
               principal_derived_from_plan = $12,
               repayment_model = $13,
               min_payment_pct = $16,
               min_payment_eur = $17,
               updated_at = now()
           WHERE id = $14 AND installation_id = $15 AND owner_user_id = $18
           RETURNING id, category_id, expense_category_id, label, type_tag, principal, apr_percent,
                     payment_amount, payment_frequency, payment_end_date, notes,
                     sort_index, principal_derived_from_plan, repayment_model, min_payment_pct,
                     min_payment_eur, owner_user_id"#,
    )
    .bind(new_cat)
    .bind(new_expense_cat)
    .bind(&new_label)
    .bind(&new_type_tag)
    .bind(new_principal)
    .bind(new_apr)
    .bind(new_pay_amt)
    .bind(new_pay_freq_str.as_deref())
    .bind(new_pay_end)
    .bind(&new_notes)
    .bind(new_sort)
    .bind(new_principal_derived)
    .bind(new_model.as_str())
    .bind(id)
    .bind(iid)
    .bind(new_min_pct)
    .bind(new_min_eur)
    .bind(user_id)
    .fetch_one(&state.pool)
    .await?;

    refresh_projection_after_mutation(&state, iid, user_id).await;
    let today = installation_naive_today(&state.pool, iid).await?;
    row_to_response(updated, today)
}

#[utoipa::path(
    delete,
    path = "/v1/liabilities/{id}",
    tag = "liabilities",
    params(
        ("id" = Uuid, Path, description = "Liability id"),
    ),
    responses(
        (status = 204, description = "Deleted"),
        (status = 401, description = "No valid session"),
        (status = 403, description = "Viewer or not a member"),
        (status = 404, description = "Liability missing"),
    )
)]
pub async fn delete_liability(
    Extension(state): Extension<Arc<AppState>>,
    jar: CookieJar,
    Path(id): Path<Uuid>,
) -> Result<axum::http::StatusCode, ApiError> {
    let user = require_session_user(&jar, &state.pool).await?;
    let (iid, role) = require_installation_member(&state.pool, user.id.0).await?;
    if !role_can_write(role.as_str()) {
        return Err(ApiError::Forbidden);
    }

    delete_liability_core(&state, iid, user.id.0, id).await?;
    Ok(axum::http::StatusCode::NO_CONTENT)
}

/// Efectos colaterales de borrar un pasivo, para el preview de la tool MCP `delete_liability`.
#[derive(Debug, serde::Serialize)]
pub struct LiabilityDeleteEffects {
    /// Transacciones cuyo `linked_liability_id` pasa a NULL (`SET NULL`, no se borran).
    pub transactions_unlinked: i64,
    /// **La cuota que desaparece del presupuesto**, con el efecto exacto sobre los totales.
    ///
    /// Era el efecto que el preview callaba. Un pasivo con plan de pago activo publica una partida
    /// derivada en `GET /v1/budget` (`source = "liability"`); al borrarlo, esa partida se va y el
    /// gasto mensual presupuestado baja. Para una hipoteca son cientos de euros al mes, y el
    /// llamante confirmaba el borrado sin haberlo visto — la misma omisión que en su día tuvo
    /// `delete_asset` con las reglas de reparto.
    ///
    /// `None` ⇒ el pasivo NO genera cuota (sin plan de pago, o con `payment_end_date` ya pasada):
    /// entonces el presupuesto no se mueve, y decirlo también es informar.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub budget_entry_removed: Option<BudgetLineRemoved>,
}

/// Efectos completos del borrado: los links que se sueltan **y** la partida de presupuesto que
/// desaparece con sus totales antes/después. Solo lee (D5: un preview jamás muta).
///
/// `pub` (y no `pub(crate)` como su hermana de activos) para que la suite de integración pueda
/// clavar el contenido del preview **antes** de que `mcp/server.rs` lo cablee: la promesa que
/// importa es que los números del preview sean los que el borrado cumple después.
pub async fn liability_delete_effects(
    pool: &sqlx::PgPool,
    iid: Uuid,
    session_user_id: Uuid,
    id: Uuid,
) -> Result<LiabilityDeleteEffects, ApiError> {
    // D21 también aquí: este preview emite el `confirm_token` de `delete_liability`. Mismo
    // criterio que `asset_delete_effects` — ver su doc.
    let owner: Option<Uuid> = sqlx::query_scalar(
        r#"SELECT owner_user_id FROM liabilities WHERE id = $1 AND installation_id = $2"#,
    )
    .bind(id)
    .bind(iid)
    .fetch_optional(pool)
    .await?;
    require_row_owner(owner.ok_or(ApiError::NotFound)?, session_user_id)?;

    let transactions_unlinked: i64 = sqlx::query_scalar(
        r#"SELECT COUNT(*)::bigint FROM transactions
           WHERE installation_id = $1 AND linked_liability_id = $2"#,
    )
    .bind(iid)
    .bind(id)
    .fetch_one(pool)
    .await?;
    let budget_entry_removed = budget_line_removed_with_liability(pool, iid, id).await?;
    Ok(LiabilityDeleteEffects {
        transactions_unlinked,
        budget_entry_removed,
    })
}

/// Core sin HTTP: lo comparten el handler DELETE y la tool MCP `delete_liability`.
pub(crate) async fn delete_liability_core(
    state: &Arc<AppState>,
    iid: Uuid,
    user_id: Uuid,
    id: Uuid,
) -> Result<(), ApiError> {
    // D21: sin SELECT previo no hay forma de distinguir «no existe» (404) de «es de otro
    // miembro» (403). Una sola columna, por clave primaria.
    let owner: Option<Uuid> = sqlx::query_scalar(
        r#"SELECT owner_user_id FROM liabilities WHERE id = $1 AND installation_id = $2"#,
    )
    .bind(id)
    .bind(iid)
    .fetch_optional(&state.pool)
    .await?;
    require_row_owner(owner.ok_or(ApiError::NotFound)?, user_id)?;

    let res = sqlx::query(
        r#"DELETE FROM liabilities WHERE id = $1 AND installation_id = $2 AND owner_user_id = $3"#,
    )
    .bind(id)
    .bind(iid)
    .bind(user_id)
    .execute(&state.pool)
    .await?;

    if res.rows_affected() == 0 {
        return Err(ApiError::NotFound);
    }

    refresh_projection_after_mutation(&state, iid, user_id).await;
    Ok(())
}

// ---------------------------------------------------------------------------
// Calendario de amortización (4.4.0) — GET /v1/liabilities/{id}/schedule
// ---------------------------------------------------------------------------

/// Meses que se simulan SIEMPRE por dentro, con independencia de cuántos se publiquen. Es el
/// tope del engine ([`futurefin_engine::MAX_LIABILITY_SCHEDULE_MONTHS`], 70 años = el horizonte
/// máximo de proyección): los agregados —interés total, mes de extinción— tienen que describir el
/// préstamo entero, no la ventana que el llamante pidió mirar.
///
/// `pub(crate)` **solo** para que `mcp::schema_bounds_parity` pueda compararla con el literal del
/// `#[schemars(range(...))]` de `LiabilityScheduleParams`: la macro exige un literal, así que la
/// única red posible es un test que los enfrente.
pub(crate) const SCHEDULE_HORIZON_MONTHS: u32 = futurefin_engine::MAX_LIABILITY_SCHEDULE_MONTHS;

/// Ventana publicada por defecto. Doce meses es «el próximo año, mes a mes», que es la pregunta
/// concreta; el préstamo entero se lee en `years`, que resume 40 años en 40 filas en vez de en
/// 480. Misma disciplina de coste de contexto que las cotas de `/v1/history/series`.
const DEFAULT_SCHEDULE_WINDOW_MONTHS: u32 = 12;

/// Tope duro de la ventana. 480 meses = 40 años, el plazo de la hipoteca más larga que se firma.
///
/// `pub(crate)` **solo** para el test de paridad `mcp::schema_bounds_parity` (ver
/// [`SCHEDULE_HORIZON_MONTHS`]). Ojo: este valor está TRIPLICADO — aquí, en el
/// `#[schemars(range(max = 480))]` de `LiabilityScheduleParams` y en el literal del mensaje
/// `schedule_window_out_of_range` unas líneas más abajo.
pub(crate) const MAX_SCHEDULE_WINDOW_MONTHS: u32 = 480;

#[derive(Debug, Deserialize, utoipa::IntoParams)]
pub struct LiabilityScheduleQuery {
    /// Vista del ledger (`mine` | `household`).
    pub view: Option<String>,
    /// Primer mes de la ventana publicada (1-based, default 1). No afecta a los agregados.
    pub from_month_index: Option<u32>,
    /// Meses de la ventana publicada (1..=480, default 12). No afecta a los agregados.
    pub months: Option<u32>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct LiabilityScheduleMonthResponse {
    /// Mes 1-based desde `anchor_date_ymd`. Es un número de MES, no una posición de array: la
    /// ventana puede empezar en cualquier mes.
    pub month_index: u32,
    /// Primero del mes civil correspondiente (`YYYY-MM-DD`).
    pub month_ymd: String,
    #[serde(with = "rust_decimal::serde::str")]
    #[schema(value_type = String)]
    pub opening_principal: Decimal,
    /// Interés devengado ese mes. Siempre `0` con `repayment_model = fixed_payments` (no
    /// devenga) o sin TIN. En `interest_only` es la cuota entera.
    #[serde(with = "rust_decimal::serde::str")]
    #[schema(value_type = String)]
    pub interest_accrued: Decimal,
    /// Principal amortizado. **Puede ser negativo** cuando la cuota no cubre el devengo: la deuda
    /// crece ese mes, y publicarlo como 0 escondería justo eso.
    #[serde(with = "rust_decimal::serde::str")]
    #[schema(value_type = String)]
    pub principal_repaid: Decimal,
    /// Caja que sale ese mes. El último mes de un préstamo es de **cuota parcial**: solo lo que
    /// queda por pagar.
    #[serde(with = "rust_decimal::serde::str")]
    #[schema(value_type = String)]
    pub payment: Decimal,
    #[serde(with = "rust_decimal::serde::str")]
    #[schema(value_type = String)]
    pub closing_principal: Decimal,
}

/// Resumen por año **civil** (no por bloques de 12 meses desde hoy): es como el usuario piensa el
/// gasto financiero y es lo que hace legible una hipoteca de 40 años sin servir 480 filas.
#[derive(Debug, Serialize, ToSchema)]
pub struct LiabilityScheduleYearResponse {
    pub year: i32,
    /// Meses del calendario que caen en ese año (el primero y el último suelen ser parciales).
    pub months_count: u32,
    #[serde(with = "rust_decimal::serde::str")]
    #[schema(value_type = String)]
    pub interest_accrued: Decimal,
    #[serde(with = "rust_decimal::serde::str")]
    #[schema(value_type = String)]
    pub principal_repaid: Decimal,
    #[serde(with = "rust_decimal::serde::str")]
    #[schema(value_type = String)]
    pub paid: Decimal,
    /// Saldo al cerrar el último mes del año.
    #[serde(with = "rust_decimal::serde::str")]
    #[schema(value_type = String)]
    pub closing_principal: Decimal,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct LiabilityScheduleResponse {
    #[schema(value_type = String, format = "uuid")]
    pub liability_id: Uuid,
    pub label: String,
    /// Vista efectivamente aplicada: `household` | `mine`. Eco de `?view`.
    pub view: &'static str,
    /// Mes 0 del calendario (`YYYY-MM-DD`), en el calendario civil de la instalación.
    pub anchor_date_ymd: String,
    pub repayment_model: RepaymentModel,
    /// TIN nominal anual en % (`i = apr/1200`). Ausente = sin interés.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>)]
    pub apr_percent: Option<Decimal>,
    /// Cuota **mensual equivalente** que simula el calendario. Con `payment_frequency = weekly` es
    /// `payment_amount × 52 / 12` — la misma normalización que usa la proyección, así que el
    /// calendario y el chart hablan de la misma cuota.
    #[serde(with = "rust_decimal::serde::str")]
    #[schema(value_type = String)]
    pub monthly_payment: Decimal,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub payment_frequency: Option<PaymentFrequency>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>, format = "date")]
    pub payment_end_date: Option<NaiveDate>,
    /// Saldo de HOY. El calendario arranca aquí, no en el principal original del préstamo.
    #[serde(with = "rust_decimal::serde::str")]
    #[schema(value_type = String)]
    pub opening_principal: Decimal,
    /// Saldo tras el último mes simulado. `0` ⟺ hay `payoff_month_index`.
    #[serde(with = "rust_decimal::serde::str")]
    #[schema(value_type = String)]
    pub final_principal: Decimal,
    /// Interés que **queda por pagar** desde hoy hasta el final del calendario.
    #[serde(with = "rust_decimal::serde::str")]
    #[schema(value_type = String)]
    pub total_interest_remaining: Decimal,
    /// Todo lo que saldrá de la caja: `opening_principal + total_interest_remaining` cuando el
    /// préstamo se salda dentro del calendario.
    #[serde(with = "rust_decimal::serde::str")]
    #[schema(value_type = String)]
    pub total_to_pay: Decimal,
    /// Mes en que el saldo llega a cero. `0` = ya saldado hoy. `null` ⟺ hay
    /// `payoff_absent_reason`. Es un número de MES desde `anchor_date_ymd`.
    pub payoff_month_index: Option<u32>,
    /// Fecha civil de ese mes (`YYYY-MM-DD`). `null` ⟺ `payoff_month_index` es `null`.
    pub payoff_date_ymd: Option<String>,
    /// Por qué no hay mes de extinción, con remedios distintos cada uno:
    /// `no_payment_plan` (no hay cuota activa: da de alta `payment_amount`),
    /// `payment_plan_ends_before_payoff` (tu `payment_end_date` llega antes que el saldo cero),
    /// `payment_does_not_reduce_principal` (`interest_only`, o cuota por debajo del interés: la
    /// deuda no baja) y `not_within_horizon` (baja, pero tarda más de 70 años).
    /// `null` ⟺ hay `payoff_month_index`.
    pub payoff_absent_reason: Option<&'static str>,
    /// Meses que tiene el calendario COMPLETO (no los publicados en `months`).
    pub months_total: u32,
    pub window_from_month_index: u32,
    pub window_months: u32,
    /// `true` ⟺ `months` no contiene el calendario entero. El resumen anual (`years`) sí lo
    /// cubre siempre.
    pub window_truncated: bool,
    /// Ventana mes a mes.
    pub months: Vec<LiabilityScheduleMonthResponse>,
    /// Resumen por año civil del calendario COMPLETO.
    pub years: Vec<LiabilityScheduleYearResponse>,
    pub model_note: String,
}

const SCHEDULE_MODEL_NOTE: &str = "Calendario proyectado con la MISMA recurrencia que el chart de proyección (interés sobre el saldo de apertura, cuota a fin de mes: P' = P·(1+i) − M, con i = apr_percent/1200), arrancando en el saldo de HOY y no en el principal original. Solo devenga con plan de pago activo y TIN > 0: fixed_payments no cobra intereses; en interest_only la cuota del mes ES el interes del periodo (saldo x TIN/1200, la declarada solo topa por arriba) y el principal no baja; en revolving la cuota es max(min_payment_pct x saldo, min_payment_eur), no la declarada. Los importes son nominales (euros del momento), sin deflactar. No modela comisiones, seguros vinculados, revisiones de tipo variable ni carencias.";

pub(crate) fn payoff_absence_code(a: futurefin_engine::LiabilityPayoffAbsence) -> &'static str {
    match a {
        futurefin_engine::LiabilityPayoffAbsence::NoPaymentPlan => "no_payment_plan",
        futurefin_engine::LiabilityPayoffAbsence::PaymentPlanEndsBeforePayoff => {
            "payment_plan_ends_before_payoff"
        }
        futurefin_engine::LiabilityPayoffAbsence::PaymentDoesNotReducePrincipal => {
            "payment_does_not_reduce_principal"
        }
        futurefin_engine::LiabilityPayoffAbsence::NotWithinHorizon => "not_within_horizon",
    }
}

/// Core compartida con la tool MCP `get_liability_schedule`.
///
/// El calendario se simula SIEMPRE entero (`SCHEDULE_HORIZON_MONTHS`) y la ventana solo recorta lo
/// que se publica: un agregado que dependiera de cuántos meses pidió mirar el llamante sería un
/// «interés total» distinto en cada llamada.
pub(crate) async fn liability_schedule_core(
    pool: &sqlx::PgPool,
    iid: Uuid,
    user_id: Uuid,
    view: LedgerView,
    id: Uuid,
    from_month_index: Option<u32>,
    window_months: Option<u32>,
) -> Result<LiabilityScheduleResponse, ApiError> {
    let from = from_month_index.unwrap_or(1);
    if from < 1 {
        return Err(ApiError::BadRequest(
            "schedule_from_month_out_of_range: from_month_index must be >= 1".into(),
        ));
    }
    let window = window_months.unwrap_or(DEFAULT_SCHEDULE_WINDOW_MONTHS);
    if window < 1 || window > MAX_SCHEDULE_WINDOW_MONTHS {
        return Err(ApiError::BadRequest(format!(
            "schedule_window_out_of_range: months must be between 1 and {MAX_SCHEDULE_WINDOW_MONTHS}"
        )));
    }

    let today = installation_naive_today(pool, iid).await?;
    let scope = view.scope_where("");
    let id_ph = view.next_arg_index();
    let today_ph = id_ph + 1;
    // Mismo predicado de visibilidad que TODAS las lecturas (#145): un plan vencido con SALDO
    // VIVO sigue existiendo — su calendario se sirve congelado, con `payoff_absent_reason:
    // no_payment_plan` y cero meses (nada devenga sin plan). Solo el vencido Y saldado
    // (`principal = 0`) es un 404: esa deuda sí se extinguió.
    let sql = format!(
        r#"SELECT id, category_id, expense_category_id, label, type_tag, principal, apr_percent,
                  payment_amount, payment_frequency, payment_end_date, notes,
                  sort_index, principal_derived_from_plan, repayment_model, min_payment_pct,
                  min_payment_eur, owner_user_id
           FROM liabilities
           WHERE {scope}
             AND id = ${id_ph}
             AND (payment_end_date IS NULL OR payment_end_date >= ${today_ph} OR principal > 0)"#
    );
    let row: LiabilityRow = view
        .bind_scope_as(sqlx::query_as(&sql), iid, user_id)
        .bind(id)
        .bind(today)
        .fetch_optional(pool)
        .await?
        .ok_or(ApiError::NotFound)?;

    let model = RepaymentModel::parse(&row.repayment_model)?;
    let payment_frequency = row
        .payment_frequency
        .as_deref()
        .map(PaymentFrequency::parse)
        .transpose()?;
    let monthly_payment =
        liability_monthly_payment(row.payment_amount, row.payment_frequency.as_deref());

    // Sin campos de amortización extra: el calendario de un pasivo GUARDADO es el real. El
    // what-if de «¿me compensa amortizar antes?» vive en `simulate_projection`, que sí los mueve.
    let liab = futurefin_engine::ProjectionLiabilityInput {
        principal: row.principal.max(Decimal::ZERO),
        monthly_payment,
        payment_end: row.payment_end_date,
        repayment_model: model.to_engine(),
        apr_percent: row.apr_percent,
        min_payment_pct: row.min_payment_pct,
        min_payment_eur: row.min_payment_eur,
        extra_principal_monthly: Decimal::ZERO,
        extra_principal_lump_sums: Vec::new(),
        early_repayment_fee_pct: None,
        early_repayment_effect: Default::default(),
    };
    let sch =
        futurefin_engine::liability_amortization_schedule(&liab, today, SCHEDULE_HORIZON_MONTHS);

    let anchor = NaiveDate::from_ymd_opt(today.year(), today.month(), 1).unwrap_or(today);
    let month_date = |k: u32| -> NaiveDate {
        anchor.checked_add_months(Months::new(k)).unwrap_or(anchor)
    };

    let months_total = sch.months.len() as u32;
    let months: Vec<LiabilityScheduleMonthResponse> = sch
        .months
        .iter()
        .filter(|m| m.month_index >= from && m.month_index < from.saturating_add(window))
        .map(|m| LiabilityScheduleMonthResponse {
            month_index: m.month_index,
            // El mes `k` es el que empieza en `primero_de_mes(hoy) + (k−1)`, exactamente igual
            // que el índice `k` de la serie de proyección.
            month_ymd: month_date(m.month_index - 1).format("%Y-%m-%d").to_string(),
            opening_principal: money_out(m.opening_principal),
            interest_accrued: money_out(m.interest_accrued),
            principal_repaid: money_out(m.principal_repaid),
            payment: money_out(m.payment),
            closing_principal: money_out(m.closing_principal),
        })
        .collect();

    // Resumen por año civil sobre el calendario COMPLETO.
    let mut years: Vec<LiabilityScheduleYearResponse> = Vec::new();
    for m in &sch.months {
        let year = month_date(m.month_index - 1).year();
        match years.last_mut() {
            Some(last) if last.year == year => {
                last.months_count += 1;
                last.interest_accrued += m.interest_accrued;
                last.principal_repaid += m.principal_repaid;
                last.paid += m.payment + m.extra_principal;
                last.closing_principal = m.closing_principal;
            }
            _ => years.push(LiabilityScheduleYearResponse {
                year,
                months_count: 1,
                interest_accrued: m.interest_accrued,
                principal_repaid: m.principal_repaid,
                paid: m.payment + m.extra_principal,
                closing_principal: m.closing_principal,
            }),
        }
    }
    for y in &mut years {
        y.interest_accrued = money_out(y.interest_accrued);
        y.principal_repaid = money_out(y.principal_repaid);
        y.paid = money_out(y.paid);
        y.closing_principal = money_out(y.closing_principal);
    }

    Ok(LiabilityScheduleResponse {
        liability_id: row.id,
        label: row.label,
        view: view.as_str(),
        anchor_date_ymd: today.format("%Y-%m-%d").to_string(),
        repayment_model: model,
        apr_percent: row.apr_percent,
        monthly_payment: money_out(monthly_payment),
        payment_frequency,
        payment_end_date: row.payment_end_date,
        opening_principal: money_out(sch.opening_principal),
        final_principal: money_out(sch.final_principal),
        total_interest_remaining: money_out(sch.total_interest),
        total_to_pay: money_out(sch.total_cash_out),
        payoff_month_index: sch.payoff_month_index,
        payoff_date_ymd: sch
            .payoff_month_index
            .map(|k| month_date(k.saturating_sub(1)).format("%Y-%m-%d").to_string()),
        payoff_absent_reason: sch.payoff_absent.map(payoff_absence_code),
        months_total,
        window_from_month_index: from,
        window_months: window,
        window_truncated: (months.len() as u32) < months_total,
        months,
        years,
        model_note: SCHEDULE_MODEL_NOTE.into(),
    })
}

#[utoipa::path(
    get,
    path = "/v1/liabilities/{id}/schedule",
    tag = "liabilities",
    params(
        ("id" = Uuid, Path, description = "Liability id"),
        LiabilityScheduleQuery
    ),
    responses(
        (status = 200, description = "Calendario de amortización", body = LiabilityScheduleResponse),
        (status = 400, description = "Parámetros de ventana fuera de rango"),
        (status = 401, description = "Unauthorized"),
        (status = 404, description = "Pasivo inexistente, fuera de la vista, o vencido Y saldado (principal 0) — el vencido con saldo vivo sí se sirve, congelado (#145)")
    )
)]
pub async fn get_liability_schedule(
    Extension(state): Extension<Arc<AppState>>,
    jar: CookieJar,
    Path(id): Path<Uuid>,
    Query(q): Query<LiabilityScheduleQuery>,
) -> Result<Json<LiabilityScheduleResponse>, ApiError> {
    let user = require_session_user(&jar, &state.pool).await?;
    let (iid, _role) = require_installation_member(&state.pool, user.id.0).await?;
    let view = LedgerViewQuery {
        view: q.view.clone(),
    }
    .resolve()?;
    let res = liability_schedule_core(
        &state.pool,
        iid,
        user.id.0,
        view,
        id,
        q.from_month_index,
        q.months,
    )
    .await?;
    Ok(Json(res))
}

pub fn liabilities_router() -> Router {
    Router::new()
        .route("/", get(list_liabilities).post(create_liability))
        .route("/{id}", patch(patch_liability).delete(delete_liability))
        .route("/{id}/schedule", get(get_liability_schedule))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Escenario del issue #123, calculado a mano. Ancla 31-08-2026, fin 30-08-2027:
    /// vencimientos reales 31-08-2026 … 31-07-2027 = **12** recibos (el 31-08-2027 cae tras el
    /// fin). El conteo encadenado degradaba el ancla al pasar por febrero (…-28 para siempre)
    /// y contaba 13 — 1.000 € de deuda derivada inventada por año con día ancla 29-31.
    #[test]
    fn monthly_interval_count_keeps_the_anchor_day() {
        let start = NaiveDate::from_ymd_opt(2026, 8, 31).unwrap();
        let end = NaiveDate::from_ymd_opt(2027, 8, 30).unwrap();
        assert_eq!(
            payment_interval_count(PaymentFrequency::Monthly, start, end).unwrap(),
            12
        );

        // Un día más de ventana y el 13.º recibo (31-08-2027) sí entra.
        let end = NaiveDate::from_ymd_opt(2027, 8, 31).unwrap();
        assert_eq!(
            payment_interval_count(PaymentFrequency::Monthly, start, end).unwrap(),
            13
        );

        // Ancla ≤ 28: sin degradación posible — mismo resultado que siempre.
        let start = NaiveDate::from_ymd_opt(2026, 1, 15).unwrap();
        let end = NaiveDate::from_ymd_opt(2026, 12, 15).unwrap();
        assert_eq!(
            payment_interval_count(PaymentFrequency::Monthly, start, end).unwrap(),
            12
        );
    }
}
