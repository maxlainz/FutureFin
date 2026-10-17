//! Validación de los planes: supuestos (`PlanAssumptions`), parámetros por tipo de cambio
//! (`PlanChangeParams`) y la FORMA de un cambio sobre su estado resultante.
//!
//! Todo lo que se valida aquí se valida en la core, no en el extractor: el cuerpo HTTP trae
//! `assumptions` y `params` como `serde_json::Value` y se parsean dentro, para que el error salga
//! con código estable (`plan_assumptions_invalid`, `plan_change_invalid_shape`) y no como el 422
//! sin código de un `Json<T>` que no deserializa — y para que la tool MCP de F2-6 comparta los
//! mismos códigos sin hacer nada.
//!
//! **Lectura tolerante, escritura estricta.** Los tipos de ESCRITURA llevan
//! `deny_unknown_fields`; la LECTURA del JSONB guardado no: el precedente de
//! `retirement_profile.rs` (un campo retirado convertía en error la lectura de todos los perfiles
//! que lo tenían guardado) es exactamente el fallo que esta asimetría evita.

use crate::error::ApiError;
use chrono::NaiveDate;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use utoipa::ToSchema;
use uuid::Uuid;

/// Horizonte por defecto de un plan: edad final (RFC §4).
pub const DEFAULT_HORIZON_AGE: i32 = 90;
pub const HORIZON_AGE_MIN: i32 = 85;
pub const HORIZON_AGE_MAX: i32 = 105;
/// Cotas del override de inflación, las mismas que `annual_inflation_assumption_percent`.
pub const INFLATION_OVERRIDE_MIN: i64 = -2;
pub const INFLATION_OVERRIDE_MAX: i64 = 50;
/// Cotas del ajuste de mercado, en puntos porcentuales sobre cada rentabilidad.
pub const MARKET_ADJUSTMENT_MIN: i64 = -5;
pub const MARKET_ADJUSTMENT_MAX: i64 = 5;
/// Cotas de un cambio en `pct`: −100 % (desaparece) … +1.000 %.
pub const FLOW_PCT_MIN: i64 = -100;
pub const FLOW_PCT_MAX: i64 = 1000;
/// Plazo máximo de un préstamo del plan (50 años).
pub const LOAN_TERM_MONTHS_MAX: i32 = 600;

fn default_true() -> bool {
    true
}

fn default_horizon_age() -> i32 {
    DEFAULT_HORIZON_AGE
}

/// Supuestos de un plan (RFC §4). Es la forma GUARDADA y la PUBLICADA: toda clave ausente toma su
/// default al leer, así que un `{}` es un plan que usa todos los supuestos de Ajustes.
///
/// Los porcentajes viajan como strings decimales, igual que el resto de la API.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct PlanAssumptions {
    /// Inflación del plan en % anual (`"3"` = 3 %). `null` = la de Ajustes. Rango `[-2, 50]`.
    #[serde(default, with = "rust_decimal::serde::str_option")]
    #[schema(value_type = Option<String>)]
    pub inflation_override_pct: Option<Decimal>,
    /// Los ingresos siguen la inflación (default `true`).
    #[serde(default = "default_true")]
    pub income_follows_inflation: bool,
    /// Los gastos siguen la inflación (default `true`).
    #[serde(default = "default_true")]
    pub expense_follows_inflation: bool,
    /// Edad final del horizonte (default 90). Rango `[85, 105]`.
    #[serde(default = "default_horizon_age")]
    pub horizon_age: i32,
    /// Puntos porcentuales que se suman a la rentabilidad de cada activo (default `"0"`).
    /// Rango `[-5, 5]`.
    #[serde(default, with = "rust_decimal::serde::str")]
    #[schema(value_type = String)]
    pub market_return_adjustment_pp: Decimal,
}

impl Default for PlanAssumptions {
    fn default() -> Self {
        PlanAssumptions {
            inflation_override_pct: None,
            income_follows_inflation: true,
            expense_follows_inflation: true,
            horizon_age: DEFAULT_HORIZON_AGE,
            market_return_adjustment_pp: Decimal::ZERO,
        }
    }
}

/// Espejo ESTRICTO de [`PlanAssumptions`] para escribir: una clave desconocida es un error, no
/// un supuesto ignorado en silencio (un `inflation_pct` mal escrito dejaría el plan con la
/// inflación de Ajustes y el usuario creyendo que puso la suya).
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct PlanAssumptionsWrite {
    #[serde(default, with = "rust_decimal::serde::str_option")]
    inflation_override_pct: Option<Decimal>,
    #[serde(default = "default_true")]
    income_follows_inflation: bool,
    #[serde(default = "default_true")]
    expense_follows_inflation: bool,
    #[serde(default = "default_horizon_age")]
    horizon_age: i32,
    #[serde(default, with = "rust_decimal::serde::str")]
    market_return_adjustment_pp: Decimal,
}

fn assumptions_err(detail: &str) -> ApiError {
    ApiError::BadRequest(format!("plan_assumptions_invalid: {detail}"))
}

/// Parsea y acota los supuestos que llegan en una escritura. El objeto REEMPLAZA al guardado
/// entero: lo que no se manda vuelve a su default.
pub(crate) fn parse_assumptions_write(v: &Value) -> Result<PlanAssumptions, ApiError> {
    if !v.is_object() {
        return Err(assumptions_err("assumptions must be a JSON object"));
    }
    let w: PlanAssumptionsWrite =
        serde_json::from_value(v.clone()).map_err(|e| assumptions_err(&e.to_string()))?;
    if let Some(p) = w.inflation_override_pct {
        if p < Decimal::from(INFLATION_OVERRIDE_MIN) || p > Decimal::from(INFLATION_OVERRIDE_MAX) {
            return Err(assumptions_err(
                "inflation_override_pct must be between -2 and 50",
            ));
        }
    }
    if !(HORIZON_AGE_MIN..=HORIZON_AGE_MAX).contains(&w.horizon_age) {
        return Err(assumptions_err("horizon_age must be between 85 and 105"));
    }
    let adj = w.market_return_adjustment_pp;
    if adj < Decimal::from(MARKET_ADJUSTMENT_MIN) || adj > Decimal::from(MARKET_ADJUSTMENT_MAX) {
        return Err(assumptions_err(
            "market_return_adjustment_pp must be between -5 and 5",
        ));
    }
    Ok(PlanAssumptions {
        inflation_override_pct: w.inflation_override_pct.map(|d| d.normalize()),
        income_follows_inflation: w.income_follows_inflation,
        expense_follows_inflation: w.expense_follows_inflation,
        horizon_age: w.horizon_age,
        market_return_adjustment_pp: adj.normalize(),
    })
}

/// Lectura TOLERANTE del JSONB guardado (claves desconocidas ignoradas, ausentes a su default).
/// Un JSONB que ni así se lee solo puede venir de SQL a mano: es un 500, no un default inventado.
pub(crate) fn assumptions_from_stored(v: Value) -> Result<PlanAssumptions, ApiError> {
    serde_json::from_value(v).map_err(|e| ApiError::Db(sqlx::Error::Decode(Box::new(e))))
}

pub(crate) fn assumptions_to_stored(a: &PlanAssumptions) -> Value {
    serde_json::to_value(a).expect("PlanAssumptions serializa")
}

// ─── Cambios ────────────────────────────────────────────────────────────────────────────────

/// Tipo de un cambio (RFC §3, D-23): verbos sobre un sujeto.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum PlanChangeKind {
    Income,
    Expense,
    AssetIn,
    AssetSale,
    Loan,
}

impl PlanChangeKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            PlanChangeKind::Income => "income",
            PlanChangeKind::Expense => "expense",
            PlanChangeKind::AssetIn => "asset_in",
            PlanChangeKind::AssetSale => "asset_sale",
            PlanChangeKind::Loan => "loan",
        }
    }

    pub fn parse(raw: &str) -> Result<Self, ApiError> {
        match raw {
            "income" => Ok(PlanChangeKind::Income),
            "expense" => Ok(PlanChangeKind::Expense),
            "asset_in" => Ok(PlanChangeKind::AssetIn),
            "asset_sale" => Ok(PlanChangeKind::AssetSale),
            "loan" => Ok(PlanChangeKind::Loan),
            _ => Err(shape_err(
                "kind must be income, expense, asset_in, asset_sale or loan",
            )),
        }
    }
}

/// Unidad del importe: euros de hoy o porcentaje (solo en cambios mensuales).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum PlanAmountUnit {
    Eur,
    Pct,
}

impl PlanAmountUnit {
    pub fn as_str(&self) -> &'static str {
        match self {
            PlanAmountUnit::Eur => "eur",
            PlanAmountUnit::Pct => "pct",
        }
    }

    pub fn parse(raw: &str) -> Result<Self, ApiError> {
        match raw {
            "eur" => Ok(PlanAmountUnit::Eur),
            "pct" => Ok(PlanAmountUnit::Pct),
            _ => Err(shape_err("amount_unit must be eur or pct")),
        }
    }
}

/// Cadencia: cada mes de `[start_date, end_date]` o una sola vez en `start_date`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum PlanCadence {
    Monthly,
    OneOff,
}

impl PlanCadence {
    pub fn as_str(&self) -> &'static str {
        match self {
            PlanCadence::Monthly => "monthly",
            PlanCadence::OneOff => "one_off",
        }
    }

    pub fn parse(raw: &str) -> Result<Self, ApiError> {
        match raw {
            "monthly" => Ok(PlanCadence::Monthly),
            "one_off" => Ok(PlanCadence::OneOff),
            _ => Err(shape_err("cadence must be monthly or one_off")),
        }
    }
}

/// Parámetros de un `asset_in`: el activo que entra. Mismos nombres y cotas que `POST /v1/assets`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct AssetInParams {
    pub name: String,
    /// % anual compuesto; > −100. `null` = sin rentabilidad.
    #[serde(default, with = "rust_decimal::serde::str_option")]
    #[schema(value_type = Option<String>)]
    pub expected_annual_return_percent: Option<Decimal>,
    /// σ anual en %, `[0, 100]`. Solo la usa Monte Carlo.
    #[serde(default, with = "rust_decimal::serde::str_option")]
    #[schema(value_type = Option<String>)]
    pub annual_volatility_percent: Option<Decimal>,
    #[serde(default = "default_true")]
    pub is_liquid: bool,
    /// Coste fiscal de adquisición (euros), ≥ 0. `null` = el valor de entrada.
    #[serde(default, with = "rust_decimal::serde::str_option")]
    #[schema(value_type = Option<String>)]
    pub purchase_price: Option<Decimal>,
}

fn default_repayment_model() -> String {
    "french".into()
}

/// Parámetros de un `loan`: el préstamo que se firma. El principal es el `amount` del cambio.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct LoanParams {
    #[serde(default)]
    pub name: Option<String>,
    /// TAE en % anual, `[0, 100]`.
    #[serde(with = "rust_decimal::serde::str")]
    #[schema(value_type = String)]
    pub apr_percent: Decimal,
    /// Plazo en meses, `[1, 600]`.
    pub term_months: i32,
    /// Sistema de amortización. Solo `french` en 6.0.
    #[serde(default = "default_repayment_model")]
    pub repayment_model: String,
}

/// Ingreso, gasto y venta no llevan parámetros: `{}`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct NoParams {}

/// `params` de un cambio, validado POR TIPO: `asset_in` → [`AssetInParams`], `loan` →
/// [`LoanParams`], el resto → `{}`. El tipo lo decide la columna `kind`, no la forma del JSON.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema)]
#[serde(untagged)]
pub enum PlanChangeParams {
    AssetIn(AssetInParams),
    Loan(LoanParams),
    None(NoParams),
}

impl PlanChangeParams {
    /// Parseo estricto según `kind`, con cotas. Devuelve la forma canónica que se guarda.
    pub(crate) fn parse(kind: PlanChangeKind, v: &Value) -> Result<Self, ApiError> {
        if !v.is_object() {
            return Err(shape_err("params must be a JSON object"));
        }
        let bad = |e: serde_json::Error| shape_err(&format!("invalid params ({e})"));
        match kind {
            PlanChangeKind::AssetIn => {
                let mut p: AssetInParams = serde_json::from_value(v.clone()).map_err(bad)?;
                p.name = normalize_label(&p.name)
                    .ok_or_else(|| shape_err("params.name must not be empty"))?;
                if p.name.chars().count() > 200 {
                    return Err(shape_err("params.name must be at most 200 characters"));
                }
                if let Some(r) = p.expected_annual_return_percent {
                    if r <= Decimal::from(-100) {
                        return Err(shape_err(
                            "params.expected_annual_return_percent must be greater than -100",
                        ));
                    }
                }
                if let Some(s) = p.annual_volatility_percent {
                    if s < Decimal::ZERO || s > Decimal::from(100) {
                        return Err(shape_err(
                            "params.annual_volatility_percent must be between 0 and 100",
                        ));
                    }
                }
                if let Some(pp) = p.purchase_price {
                    if pp < Decimal::ZERO {
                        return Err(shape_err("params.purchase_price must not be negative"));
                    }
                }
                Ok(PlanChangeParams::AssetIn(p))
            }
            PlanChangeKind::Loan => {
                let mut p: LoanParams = serde_json::from_value(v.clone()).map_err(bad)?;
                p.name = match p.name.as_deref() {
                    Some(n) => normalize_label(n),
                    None => None,
                };
                if p.name.as_ref().is_some_and(|n| n.chars().count() > 200) {
                    return Err(shape_err("params.name must be at most 200 characters"));
                }
                if p.apr_percent < Decimal::ZERO || p.apr_percent > Decimal::from(100) {
                    return Err(shape_err("params.apr_percent must be between 0 and 100"));
                }
                if !(1..=LOAN_TERM_MONTHS_MAX).contains(&p.term_months) {
                    return Err(shape_err("params.term_months must be between 1 and 600"));
                }
                if p.repayment_model != "french" {
                    return Err(shape_err("params.repayment_model must be french"));
                }
                Ok(PlanChangeParams::Loan(p))
            }
            PlanChangeKind::Income | PlanChangeKind::Expense | PlanChangeKind::AssetSale => {
                let p: NoParams = serde_json::from_value(v.clone()).map_err(bad)?;
                Ok(PlanChangeParams::None(p))
            }
        }
    }

    /// Default de `params` cuando el alta no lo trae.
    pub(crate) fn default_json_for(kind: PlanChangeKind) -> Option<Value> {
        match kind {
            PlanChangeKind::AssetIn | PlanChangeKind::Loan => None,
            _ => Some(Value::Object(Default::default())),
        }
    }

    pub(crate) fn to_stored(&self) -> Value {
        serde_json::to_value(self).expect("PlanChangeParams serializa")
    }
}

fn normalize_label(raw: &str) -> Option<String> {
    let t = raw.trim();
    if t.is_empty() {
        None
    } else {
        Some(t.to_string())
    }
}

pub(crate) fn shape_err(detail: &str) -> ApiError {
    ApiError::BadRequest(format!("plan_change_invalid_shape: {detail}"))
}

/// Estado RESULTANTE de un cambio (alta, o fila guardada + patch), el que se valida entero.
#[derive(Debug, Clone)]
pub(crate) struct ChangeShape {
    pub kind: PlanChangeKind,
    pub start_date: NaiveDate,
    pub end_date: Option<NaiveDate>,
    pub amount: Option<Decimal>,
    pub amount_unit: PlanAmountUnit,
    pub cadence: PlanCadence,
    pub keeps_purchasing_power: bool,
    pub asset_id: Option<Uuid>,
    pub target_change_id: Option<Uuid>,
}

/// La forma que exige cada tipo. Espejo de los `CHECK` de `plan_changes`, más estricto en un
/// punto: una venta necesita EXACTAMENTE un sujeto (la tabla admite ninguno, el estado que deja
/// borrar el activo vendido).
pub(crate) fn validate_shape(s: &ChangeShape) -> Result<(), ApiError> {
    if let Some(end) = s.end_date {
        if s.cadence == PlanCadence::OneOff {
            return Err(shape_err("a one_off change has no end_date"));
        }
        if end < s.start_date {
            return Err(shape_err("end_date must not be before start_date"));
        }
    }
    if s.amount_unit == PlanAmountUnit::Pct && s.cadence != PlanCadence::Monthly {
        return Err(shape_err(
            "amount_unit pct is only valid with cadence monthly",
        ));
    }
    let is_sale = s.kind == PlanChangeKind::AssetSale;
    if !is_sale && (s.asset_id.is_some() || s.target_change_id.is_some()) {
        return Err(shape_err(
            "asset_id and target_change_id only apply to kind asset_sale",
        ));
    }
    match s.kind {
        PlanChangeKind::Income | PlanChangeKind::Expense => {
            let Some(a) = s.amount else {
                return Err(shape_err("income and expense changes require amount"));
            };
            if a.is_zero() {
                return Err(shape_err("amount must not be zero"));
            }
            if s.amount_unit == PlanAmountUnit::Pct
                && (a < Decimal::from(FLOW_PCT_MIN) || a > Decimal::from(FLOW_PCT_MAX))
            {
                return Err(shape_err("a pct amount must be between -100 and 1000"));
            }
        }
        PlanChangeKind::AssetIn => {
            if !s.amount.is_some_and(|a| a > Decimal::ZERO) {
                return Err(shape_err("asset_in requires a positive amount"));
            }
            if s.amount_unit != PlanAmountUnit::Eur || s.cadence != PlanCadence::OneOff {
                return Err(shape_err("asset_in is one_off and in eur"));
            }
        }
        PlanChangeKind::AssetSale => {
            if s.asset_id.is_some() == s.target_change_id.is_some() {
                return Err(shape_err(
                    "asset_sale requires exactly one of asset_id or target_change_id",
                ));
            }
            match s.amount {
                None => {
                    if s.cadence != PlanCadence::OneOff || s.amount_unit != PlanAmountUnit::Eur {
                        return Err(shape_err(
                            "selling the whole asset (amount null) is one_off and in eur",
                        ));
                    }
                }
                Some(a) => {
                    if a <= Decimal::ZERO {
                        return Err(shape_err("asset_sale amount must be positive"));
                    }
                    if s.amount_unit == PlanAmountUnit::Pct && a > Decimal::from(100) {
                        return Err(shape_err("a pct sale must be at most 100"));
                    }
                }
            }
        }
        PlanChangeKind::Loan => {
            if !s.amount.is_some_and(|a| a > Decimal::ZERO) {
                return Err(shape_err("loan requires a positive amount (the principal)"));
            }
            if s.amount_unit != PlanAmountUnit::Eur || s.cadence != PlanCadence::OneOff {
                return Err(shape_err("loan is one_off and in eur"));
            }
            if s.keeps_purchasing_power {
                return Err(shape_err(
                    "a loan is always nominal: keeps_purchasing_power must be false",
                ));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn d(s: &str) -> Decimal {
        s.parse().unwrap()
    }

    fn base(kind: PlanChangeKind) -> ChangeShape {
        ChangeShape {
            kind,
            start_date: NaiveDate::from_ymd_opt(2030, 1, 1).unwrap(),
            end_date: None,
            amount: Some(d("100")),
            amount_unit: PlanAmountUnit::Eur,
            cadence: PlanCadence::Monthly,
            keeps_purchasing_power: true,
            asset_id: None,
            target_change_id: None,
        }
    }

    #[test]
    fn empty_assumptions_are_all_defaults() {
        let a = parse_assumptions_write(&json!({})).unwrap();
        assert_eq!(a, PlanAssumptions::default());
        assert_eq!(assumptions_from_stored(json!({})).unwrap(), a);
    }

    #[test]
    fn writes_reject_unknown_assumption_keys_but_reads_ignore_them() {
        assert!(parse_assumptions_write(&json!({"inflation_pct": "3"})).is_err());
        let a = assumptions_from_stored(json!({"retired_key": 1, "horizon_age": 95})).unwrap();
        assert_eq!(a.horizon_age, 95);
    }

    #[test]
    fn assumption_bounds() {
        assert!(parse_assumptions_write(&json!({"horizon_age": 84})).is_err());
        assert!(parse_assumptions_write(&json!({"horizon_age": 105})).is_ok());
        assert!(parse_assumptions_write(&json!({"inflation_override_pct": "-2.01"})).is_err());
        assert!(parse_assumptions_write(&json!({"inflation_override_pct": "50"})).is_ok());
        assert!(parse_assumptions_write(&json!({"market_return_adjustment_pp": "5.5"})).is_err());
        assert!(parse_assumptions_write(&json!([])).is_err());
    }

    #[test]
    fn pct_only_monthly_and_one_off_has_no_end() {
        let mut s = base(PlanChangeKind::Income);
        s.amount_unit = PlanAmountUnit::Pct;
        s.cadence = PlanCadence::OneOff;
        assert!(validate_shape(&s).is_err());
        let mut s = base(PlanChangeKind::Expense);
        s.cadence = PlanCadence::OneOff;
        s.end_date = Some(NaiveDate::from_ymd_opt(2031, 1, 1).unwrap());
        assert!(validate_shape(&s).is_err());
    }

    #[test]
    fn sale_needs_exactly_one_subject() {
        let mut s = base(PlanChangeKind::AssetSale);
        s.cadence = PlanCadence::OneOff;
        assert!(validate_shape(&s).is_err());
        s.asset_id = Some(Uuid::nil());
        assert!(validate_shape(&s).is_ok());
        s.target_change_id = Some(Uuid::nil());
        assert!(validate_shape(&s).is_err());
    }

    #[test]
    fn loan_is_always_nominal() {
        let mut s = base(PlanChangeKind::Loan);
        s.cadence = PlanCadence::OneOff;
        assert!(validate_shape(&s).is_err());
        s.keeps_purchasing_power = false;
        assert!(validate_shape(&s).is_ok());
    }

    #[test]
    fn params_are_validated_by_kind() {
        assert!(PlanChangeParams::parse(PlanChangeKind::Income, &json!({"x": 1})).is_err());
        assert!(PlanChangeParams::parse(PlanChangeKind::AssetIn, &json!({})).is_err());
        let p = PlanChangeParams::parse(
            PlanChangeKind::AssetIn,
            &json!({"name": " Casa ", "expected_annual_return_percent": "2"}),
        )
        .unwrap();
        assert_eq!(
            p.to_stored(),
            json!({"name": "Casa", "expected_annual_return_percent": "2",
                   "annual_volatility_percent": null, "is_liquid": true, "purchase_price": null})
        );
        assert!(PlanChangeParams::parse(
            PlanChangeKind::Loan,
            &json!({"apr_percent": "3", "term_months": 601})
        )
        .is_err());
        assert!(PlanChangeParams::parse(
            PlanChangeKind::Loan,
            &json!({"apr_percent": "3", "term_months": 300})
        )
        .is_ok());
    }
}
