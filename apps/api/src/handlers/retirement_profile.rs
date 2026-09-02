//! Perfil de jubilación **por usuario** (5.0.0, issue #207, decisión D13 del owner).
//!
//! Hasta 4.15.x la jubilación era una propiedad del HOGAR: `installation.fire_settings` guardaba
//! el modo del objetivo, el importe manual, el SWR y la edad límite del horizonte, y todo el
//! mundo compartía los cuatro. Con proyecciones independientes por miembro (D9) eso deja de
//! tener sentido: dos personas del mismo hogar pueden querer jubilarse a edades distintas, con
//! reglas de retirada distintas y con pensiones que empiezan en años distintos.
//!
//! Este módulo es el mismo patrón que `FireSettings` (`handlers/installation.rs`), pieza por
//! pieza y a propósito — es el patrón que el repo ya sabe mantener:
//!
//! * `RetirementProfile` con `#[serde(default)]` a nivel de struct: una clave ausente en el
//!   JSONB es su default, nunca un `null` que reviente la deserialización.
//! * `default_retirement_profile()` — el perfil de quien no ha tocado nada. Es exactamente la
//!   conducta de 4.15.x (`asap` = el cruce de líquido de siempre).
//! * `resolve_retirement_profile()` — defaults **y clamps** en LECTURA. La validación solo corre
//!   en las rutas de escritura; un valor fuera de rango llegado por otra vía (restore de un
//!   `.ffbackup`, edición directa de la BD, un fichero de otra versión) produciría índices de mes
//!   absurdos o un objetivo dividido por un SWR negativo. Clampar en el consumo lo hace imposible.
//! * `validate_retirement_profile()` — cotas y coherencia entre campos, con códigos estables
//!   `snake_case: mensaje` (los lee `ErrorBody.code`, ver `error.rs`).
//! * `RetirementProfilePatch` — DTO campo a campo con tri-estado: **omitir = no cambiar**. NUNCA
//!   se deserializa un `RetirementProfile` completo desde un PATCH: su `#[serde(default)]` a
//!   nivel de struct resetearía a defaults todo lo ausente (un PATCH «solo el SWR» borraría la
//!   pensión declarada). Es el mismo bug que `FireSettingsPatch` existe para esquivar.
//!
//! La columna es `users.retirement_profile jsonb NULL` (`NULL` = defaults). La migración
//! `20260902200000_users_retirement_profile.sql` la crea y **copia** los cuatro ejes movidos
//! desde `installation.fire_settings` al perfil de cada usuario, para que el upgrade no mueva
//! un número.

use crate::error::ApiError;
use crate::handlers::installation::{
    require_installation_member, FireNumberMode, MAX_HORIZON_LIFESPAN_AGE, MIN_HORIZON_LIFESPAN_AGE,
};
use crate::handlers::projection::refresh_projection_after_mutation;
use crate::handlers::session::require_session_user;
use crate::state::AppState;
use axum::extract::Extension;
use axum::routing::get;
use axum::{Json, Router};
use axum_extra::extract::cookie::CookieJar;
use chrono::NaiveDate;
use rust_decimal::Decimal;
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;
use sqlx::types::Json as SqlxJson;
use sqlx::PgPool;
use std::sync::Arc;
use utoipa::ToSchema;
use uuid::Uuid;

// ---------------------------------------------------------------------------
// Cotas
// ---------------------------------------------------------------------------

/// Edad mínima de cualquier hito del perfil. 18 no es una cifra financiera: por debajo no hay
/// nadie que pueda ser miembro de una instalación.
pub(crate) const MIN_PROFILE_AGE: u32 = 18;
/// Edad mínima a la que se puede declarar que empieza una pensión. Ninguna prestación pública
/// española arranca antes; permitir 30 convertiría el bloque `pension` en «una renta cualquiera»,
/// que ya existe como partida de presupuesto con `persists_after_retirement`.
pub(crate) const MIN_PENSION_AGE: u32 = 50;
/// Techo de los `pct` de las reglas de retirada (%). Es BRUTO de impuestos, igual que el SWR.
/// Un 20 % anual agota cualquier cartera; por encima el número deja de describir un plan.
pub(crate) const MAX_WITHDRAWAL_PCT: Decimal = Decimal::from_parts(20, 0, 0, false, 0);
/// Techo de la banda y del ajuste de `guardrails` (%). 50 % es ya un régimen extremo (Guyton-
/// Klinger usa 20 % de banda y 10 % de ajuste); por encima la regla no reacciona, oscila.
pub(crate) const MAX_GUARDRAIL_PCT: Decimal = Decimal::from_parts(50, 0, 0, false, 0);
/// Colchón de caja máximo, en meses de gasto. Cinco años es el límite útil: más allá, «colchón»
/// y «cartera» son la misma cosa.
pub(crate) const MAX_CASH_BUFFER_MONTHS: u32 = 60;
/// Cotas del umbral de éxito de Monte Carlo (%). Por debajo de 50 el veredicto verde no
/// significaría nada; 100 es inalcanzable con retornos estocásticos y sería un rojo perpetuo.
pub(crate) const MIN_SUCCESS_THRESHOLD_PCT: u32 = 50;
pub(crate) const MAX_SUCCESS_THRESHOLD_PCT: u32 = 99;
/// Techo del SWR (%). Mismo que tenía en `FireSettings`: el eje se movió, la cota no.
pub(crate) const MAX_SWR_PCT: Decimal = Decimal::from_parts(4, 0, 0, false, 0);

// ---------------------------------------------------------------------------
// Enumerados del perfil
// ---------------------------------------------------------------------------

/// Las cinco estrategias de jubilación (D15). **Una por usuario**: la estrategia decide el
/// trigger de la jubilación, la base del objetivo y qué lecturas tienen sentido.
///
/// El `Deserialize` es manual —como los de `FireSettings`— para que un literal desconocido dé
/// un error con la lista de variantes en vez de un `unknown variant` genérico, y para que la
/// superficie MCP pueda reusar EXACTAMENTE esta lista (`parse_enum_param`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ToSchema, Default)]
#[serde(rename_all = "snake_case")]
pub enum RetirementStrategy {
    /// «Cuanto antes (FIRE clásico)»: se jubila el primer mes en que el líquido cruza el
    /// objetivo. Es la conducta de 4.15.x y por eso es el default.
    #[default]
    Asap,
    /// «A una edad fija»: la edad manda (D17), llegue o no el capital.
    RetireAtAge,
    /// «Ahorrar ahora y dejar crecer (Coast FIRE)».
    Coast,
    /// «Media jornada».
    Partial,
    /// «Puente hasta la pensión».
    PensionBridge,
}

impl<'de> Deserialize<'de> for RetirementStrategy {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        use serde::de::Error;
        let s = String::deserialize(deserializer)?;
        match s.as_str() {
            "asap" => Ok(Self::Asap),
            "retire_at_age" => Ok(Self::RetireAtAge),
            "coast" => Ok(Self::Coast),
            "partial" => Ok(Self::Partial),
            "pension_bridge" => Ok(Self::PensionBridge),
            _ => Err(D::Error::unknown_variant(
                &s,
                &["asap", "retire_at_age", "coast", "partial", "pension_bridge"],
            )),
        }
    }
}

impl RetirementStrategy {
    /// `true` para las estrategias cuyo trigger es una EDAD y que, por tanto, exigen
    /// `target_retirement_age`.
    pub(crate) fn requires_target_age(self) -> bool {
        matches!(self, Self::RetireAtAge | Self::Coast)
    }
}

/// Sobre qué se dimensiona el objetivo de jubilación.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ToSchema, Default)]
#[serde(rename_all = "snake_case")]
pub enum TargetBasis {
    /// Perpetuidad sobre la necesidad neta: `gross_up(12·need)/SWR`. Es lo de siempre.
    #[default]
    Perpetuity,
    /// Puente: capital para cubrir el gasto hasta que empieza la pensión + la perpetuidad
    /// sobre lo que la pensión NO cubra (P2).
    BridgeToPension,
}

impl<'de> Deserialize<'de> for TargetBasis {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        use serde::de::Error;
        let s = String::deserialize(deserializer)?;
        match s.as_str() {
            "perpetuity" => Ok(Self::Perpetuity),
            "bridge_to_pension" => Ok(Self::BridgeToPension),
            _ => Err(D::Error::unknown_variant(
                &s,
                &["perpetuity", "bridge_to_pension"],
            )),
        }
    }
}

/// Con qué tasa se descuentan los flujos del puente (D7).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ToSchema, Default)]
#[serde(rename_all = "snake_case")]
pub enum BridgeDiscountBasis {
    /// Rentabilidad esperada ponderada por valor de los activos líquidos.
    #[default]
    ExpectedReturn,
    /// El propio SWR del perfil.
    Swr,
    /// Sin descuento: el puente cuesta la suma nominal de sus flujos (conservador).
    None,
}

impl<'de> Deserialize<'de> for BridgeDiscountBasis {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        use serde::de::Error;
        let s = String::deserialize(deserializer)?;
        match s.as_str() {
            "expected_return" => Ok(Self::ExpectedReturn),
            "swr" => Ok(Self::Swr),
            "none" => Ok(Self::None),
            _ => Err(D::Error::unknown_variant(
                &s,
                &["expected_return", "swr", "none"],
            )),
        }
    }
}

/// Catálogo de reglas de retirada (D6).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ToSchema, Default)]
#[serde(rename_all = "snake_case")]
pub enum WithdrawalRuleKind {
    /// «Gasto fijo en euros de hoy»: se retira la necesidad declarada indexada, sin techo. Es
    /// EXACTAMENTE el drenaje de 4.15.x, y por eso es el default.
    #[default]
    FixedReal,
    /// `pct` % del líquido del mes anterior, anualizado.
    PercentOfBalance,
    /// `start_pct` hasta que el saldo permite bajar a `end_pct` (latch).
    Hybrid,
    /// Guyton-Klinger 2006 (capital-preservation + prosperity), sin la regla de los 15 años ni
    /// el salto de inflación — divergencia declarada en `financial-contracts.md`.
    Guardrails,
}

impl<'de> Deserialize<'de> for WithdrawalRuleKind {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        use serde::de::Error;
        let s = String::deserialize(deserializer)?;
        match s.as_str() {
            "fixed_real" => Ok(Self::FixedReal),
            "percent_of_balance" => Ok(Self::PercentOfBalance),
            "hybrid" => Ok(Self::Hybrid),
            "guardrails" => Ok(Self::Guardrails),
            _ => Err(D::Error::unknown_variant(
                &s,
                &["fixed_real", "percent_of_balance", "hybrid", "guardrails"],
            )),
        }
    }
}

/// Qué relación tiene la regla con el gasto declarado (D5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ToSchema, Default)]
#[serde(rename_all = "snake_case")]
pub enum SpendMode {
    /// La regla es un TECHO: se retira `min(necesidad, regla)`.
    #[default]
    Ceiling,
    /// La regla ES el gasto: se retira lo que dice la regla, haya o no necesidad.
    RuleIsSpend,
}

impl<'de> Deserialize<'de> for SpendMode {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        use serde::de::Error;
        let s = String::deserialize(deserializer)?;
        match s.as_str() {
            "ceiling" => Ok(Self::Ceiling),
            "rule_is_spend" => Ok(Self::RuleIsSpend),
            _ => Err(D::Error::unknown_variant(&s, &["ceiling", "rule_is_spend"])),
        }
    }
}

/// Base de gasto de la fase de media jornada (D10).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ToSchema, Default)]
#[serde(rename_all = "snake_case")]
pub enum PartialExpenseBasis {
    /// El gasto de jubilación (default: quien baja a media jornada ya vive como jubilado).
    #[default]
    Retirement,
    /// El gasto regular de hoy.
    Regular,
}

impl<'de> Deserialize<'de> for PartialExpenseBasis {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        use serde::de::Error;
        let s = String::deserialize(deserializer)?;
        match s.as_str() {
            "retirement" => Ok(Self::Retirement),
            "regular" => Ok(Self::Regular),
            _ => Err(D::Error::unknown_variant(&s, &["retirement", "regular"])),
        }
    }
}

// ---------------------------------------------------------------------------
// Bloques del perfil
// ---------------------------------------------------------------------------

/// Regla de retirada + su modo de gasto. Los `pct` son BRUTOS de impuestos (R9), igual que el
/// SWR: lo que se vende de la cartera antes de pasar por el gross-up.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(default)]
pub struct WithdrawalRule {
    pub kind: WithdrawalRuleKind,
    /// `percent_of_balance` y `guardrails`: % anual del líquido.
    #[serde(with = "rust_decimal::serde::str_option")]
    #[schema(value_type = Option<String>)]
    pub pct: Option<Decimal>,
    /// `hybrid`: % de partida.
    #[serde(with = "rust_decimal::serde::str_option")]
    #[schema(value_type = Option<String>)]
    pub start_pct: Option<Decimal>,
    /// `hybrid`: % al que se baja tras el latch (estrictamente menor que `start_pct`).
    #[serde(with = "rust_decimal::serde::str_option")]
    #[schema(value_type = Option<String>)]
    pub end_pct: Option<Decimal>,
    /// `guardrails`: banda alrededor de la tasa inicial que dispara el ajuste.
    #[serde(with = "rust_decimal::serde::str_option")]
    #[schema(value_type = Option<String>)]
    pub band_pct: Option<Decimal>,
    /// `guardrails`: cuánto se recorta/sube la retirada al tocar una banda.
    #[serde(with = "rust_decimal::serde::str_option")]
    #[schema(value_type = Option<String>)]
    pub adjust_pct: Option<Decimal>,
    pub spend_mode: SpendMode,
}

impl Default for WithdrawalRule {
    fn default() -> Self {
        WithdrawalRule {
            kind: WithdrawalRuleKind::FixedReal,
            pct: None,
            start_pct: None,
            end_pct: None,
            band_pct: None,
            adjust_pct: None,
            spend_mode: SpendMode::Ceiling,
        }
    }
}

/// Pensión pública (u otra renta vitalicia) **con fecha** (D3/D8). No es una partida de
/// presupuesto: su fecha de inicio cambia el OBJETIVO, no solo el flujo de caja.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct PensionPlan {
    /// Importe MENSUAL en euros de HOY (> 0).
    #[serde(with = "rust_decimal::serde::str")]
    #[schema(value_type = String)]
    pub monthly_amount_today: Decimal,
    /// Edad a la que empieza a cobrarse.
    pub starts_at_age: u32,
    /// `true` (default) = se indexa a la inflación de la instalación; `false` = importe plano.
    #[serde(default = "default_true")]
    pub indexed: bool,
    /// Fracción del importe que se cobra DURANTE la fase de media jornada, en `[0, 1]`.
    /// Default `0`: la jubilación parcial no da derecho a pensión salvo que se declare.
    #[serde(default, with = "rust_decimal::serde::str")]
    #[schema(value_type = String)]
    pub fraction_while_partial: Decimal,
}

fn default_true() -> bool {
    true
}

/// Fase de media jornada (P7). No lleva `ends_at_age` a propósito: termina en la jubilación
/// total, que ya tiene su propio trigger — dos fines chocarían.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct PartialRetirement {
    pub starts_at_age: u32,
    /// Ingreso MENSUAL en euros de HOY durante la fase (>= 0; `0` = año sabático).
    #[serde(with = "rust_decimal::serde::str")]
    #[schema(value_type = String)]
    pub income_monthly_today: Decimal,
    #[serde(default)]
    pub expense_basis: PartialExpenseBasis,
}

// ---------------------------------------------------------------------------
// El perfil
// ---------------------------------------------------------------------------

/// Perfil de jubilación de UN usuario. Todas las claves son opcionales en el wire: un JSONB
/// `{}` —o `NULL`— es el perfil por defecto, que reproduce la conducta de 4.15.x.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(default)]
pub struct RetirementProfile {
    pub strategy: RetirementStrategy,
    /// Edad de jubilación total. OBLIGATORIA en `retire_at_age` y `coast`; opcional en
    /// `partial` (fin de la fase parcial); ignorada por `asap` y `pension_bridge`, que se
    /// disparan por cruce.
    pub target_retirement_age: Option<u32>,

    // ---- Los cuatro ejes MOVIDOS desde `installation.fire_settings` (5.0.0) ----------------
    // Mismos tipos, mismos defaults y mismas cotas que tenían allí: el upgrade copia el valor
    // de la instalación al perfil de cada usuario y nadie ve moverse un número.
    pub fire_number_mode: FireNumberMode,
    #[serde(with = "rust_decimal::serde::str_option")]
    #[schema(value_type = Option<String>)]
    pub fire_number_manual_amount: Option<Decimal>,
    #[serde(with = "rust_decimal::serde::str")]
    #[schema(value_type = String)]
    pub swr_pct: Decimal,
    pub horizon_lifespan_age: u32,

    /// Base del objetivo. **`None` en el almacén = derivar** (R6): `bridge_to_pension` cuando
    /// hay `pension` declarada, `perpetuity` cuando no. `resolve_retirement_profile` lo rellena
    /// siempre, así que lo que sale por el API nunca es `null`. Fijarlo a `perpetuity` teniendo
    /// pensión es la opción explícita «ignorar la pensión» (conservadora).
    pub target_basis: Option<TargetBasis>,
    pub bridge_discount_basis: BridgeDiscountBasis,
    pub withdrawal_rule: WithdrawalRule,
    pub pension: Option<PensionPlan>,
    pub partial_retirement: Option<PartialRetirement>,
    /// Colchón de caja en meses de gasto (P4). Solo actúa en Monte Carlo; en el camino
    /// determinista es un no-op declarado.
    pub cash_buffer_months: Option<u32>,
    /// Umbral de éxito de Monte Carlo en % (D25).
    pub success_threshold_pct: u32,
}

impl Default for RetirementProfile {
    fn default() -> Self {
        default_retirement_profile()
    }
}

/// El perfil de quien no ha tocado nada. Reproduce EXACTAMENTE la jubilación de 4.15.x: cruce
/// de líquido, objetivo perpetuo, drenaje del gasto declarado sin techo.
pub(crate) fn default_retirement_profile() -> RetirementProfile {
    RetirementProfile {
        strategy: RetirementStrategy::Asap,
        target_retirement_age: None,
        fire_number_mode: FireNumberMode::AnnualExpense,
        fire_number_manual_amount: None,
        swr_pct: Decimal::new(35, 1),
        horizon_lifespan_age: 90,
        target_basis: None,
        bridge_discount_basis: BridgeDiscountBasis::ExpectedReturn,
        withdrawal_rule: WithdrawalRule::default(),
        pension: None,
        partial_retirement: None,
        cash_buffer_months: None,
        success_threshold_pct: 95,
    }
}

/// Clampa un `pct` de regla de retirada al `[0, max]`.
///
/// El suelo es **0 y no un épsilon**: la cota de ESCRITURA es `(0, max]` y la impone
/// `validate_retirement_profile`. Aquí solo hay que impedir que un valor imposible llegue al
/// motor, y un `0` colado por una vía no validada significa «esta regla no retira nada» — que
/// es una lectura honesta y acotada. Inventar un mínimo positivo para «arreglarlo» pondría en
/// el perfil un número que el usuario nunca escribió.
fn clamp_pct(v: Option<Decimal>, max: Decimal) -> Option<Decimal> {
    v.map(|p| p.clamp(Decimal::ZERO, max))
}

/// Defaults **y clamps** en lectura. Ver el porqué en la cabecera del módulo.
pub(crate) fn resolve_retirement_profile(stored: Option<RetirementProfile>) -> RetirementProfile {
    let mut p = stored.unwrap_or_else(default_retirement_profile);

    // El horizonte va PRIMERO: es el techo de todas las edades del perfil.
    p.horizon_lifespan_age = p
        .horizon_lifespan_age
        .clamp(MIN_HORIZON_LIFESPAN_AGE, MAX_HORIZON_LIFESPAN_AGE);
    p.swr_pct = p.swr_pct.clamp(Decimal::ZERO, MAX_SWR_PCT);
    p.success_threshold_pct = p
        .success_threshold_pct
        .clamp(MIN_SUCCESS_THRESHOLD_PCT, MAX_SUCCESS_THRESHOLD_PCT);
    p.cash_buffer_months = p.cash_buffer_months.map(|m| m.min(MAX_CASH_BUFFER_MONTHS));
    p.target_retirement_age = p
        .target_retirement_age
        .map(|a| a.clamp(MIN_PROFILE_AGE, p.horizon_lifespan_age));

    p.withdrawal_rule.pct = clamp_pct(p.withdrawal_rule.pct, MAX_WITHDRAWAL_PCT);
    p.withdrawal_rule.start_pct = clamp_pct(p.withdrawal_rule.start_pct, MAX_WITHDRAWAL_PCT);
    p.withdrawal_rule.end_pct = clamp_pct(p.withdrawal_rule.end_pct, MAX_WITHDRAWAL_PCT);
    p.withdrawal_rule.band_pct = clamp_pct(p.withdrawal_rule.band_pct, MAX_GUARDRAIL_PCT);
    p.withdrawal_rule.adjust_pct = clamp_pct(p.withdrawal_rule.adjust_pct, MAX_GUARDRAIL_PCT);

    let horizon = p.horizon_lifespan_age;
    if let Some(pen) = p.pension.as_mut() {
        pen.monthly_amount_today = pen.monthly_amount_today.max(Decimal::ZERO);
        pen.starts_at_age = pen.starts_at_age.clamp(MIN_PENSION_AGE.min(horizon), horizon);
        pen.fraction_while_partial = pen.fraction_while_partial.clamp(Decimal::ZERO, Decimal::ONE);
    }
    if let Some(par) = p.partial_retirement.as_mut() {
        par.income_monthly_today = par.income_monthly_today.max(Decimal::ZERO);
        par.starts_at_age = par.starts_at_age.clamp(MIN_PROFILE_AGE, horizon);
    }

    // R6 — la base del objetivo se DERIVA cuando no está fijada, y `pension_bridge` la fuerza:
    // esa estrategia ES el puente, así que un `perpetuity` guardado ahí describiría otra cosa.
    p.target_basis = Some(match (p.strategy, p.target_basis, p.pension.is_some()) {
        (RetirementStrategy::PensionBridge, _, _) => TargetBasis::BridgeToPension,
        (_, Some(b), _) => b,
        (_, None, true) => TargetBasis::BridgeToPension,
        (_, None, false) => TargetBasis::Perpetuity,
    });

    p
}

/// Cotas y coherencia entre campos. Corre en las rutas de ESCRITURA (PATCH HTTP y tool MCP)
/// sobre el perfil YA mergeado, nunca sobre el patchset: una regla cruzada (parcial antes que
/// total, pensión exigida por la estrategia) solo tiene sentido sobre el estado resultante.
pub(crate) fn validate_retirement_profile(p: &RetirementProfile) -> Result<(), ApiError> {
    // ---- Los cuatro ejes movidos conservan sus códigos de error de 4.15.x -------------------
    // Son los mismos códigos que devolvía `validate_fire_settings`, a propósito: la SPA ya los
    // traduce y el eje es el mismo, solo ha cambiado de dueño.
    if p.swr_pct < Decimal::ZERO || p.swr_pct > MAX_SWR_PCT {
        return Err(ApiError::BadRequest(
            "swr_out_of_range: swr_pct must be between 0 and 4 (percent)".into(),
        ));
    }
    match p.fire_number_mode {
        FireNumberMode::Manual => {
            let Some(amt) = p.fire_number_manual_amount else {
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
    if !(MIN_HORIZON_LIFESPAN_AGE..=MAX_HORIZON_LIFESPAN_AGE).contains(&p.horizon_lifespan_age) {
        return Err(ApiError::BadRequest(format!(
            "horizon_lifespan_age_out_of_range: horizon_lifespan_age must be between {MIN_HORIZON_LIFESPAN_AGE} and {MAX_HORIZON_LIFESPAN_AGE} (years)"
        )));
    }

    // ---- Estrategia ------------------------------------------------------------------------
    if p.strategy.requires_target_age() && p.target_retirement_age.is_none() {
        return Err(ApiError::BadRequest(
            "target_retirement_age_required: strategies retire_at_age and coast need target_retirement_age".into(),
        ));
    }
    if p.strategy == RetirementStrategy::PensionBridge && p.pension.is_none() {
        return Err(ApiError::BadRequest(
            "pension_required_for_bridge: strategy pension_bridge needs a pension block".into(),
        ));
    }

    // ---- Edades ----------------------------------------------------------------------------
    let horizon = p.horizon_lifespan_age;
    if let Some(age) = p.target_retirement_age {
        if !(MIN_PROFILE_AGE..=horizon).contains(&age) {
            return Err(ApiError::BadRequest(format!(
                "retirement_age_out_of_range: target_retirement_age must be between {MIN_PROFILE_AGE} and horizon_lifespan_age ({horizon})"
            )));
        }
    }
    if let Some(pen) = &p.pension {
        if !(MIN_PENSION_AGE..=horizon).contains(&pen.starts_at_age) {
            return Err(ApiError::BadRequest(format!(
                "pension_age_out_of_range: pension.starts_at_age must be between {MIN_PENSION_AGE} and horizon_lifespan_age ({horizon})"
            )));
        }
        if pen.monthly_amount_today <= Decimal::ZERO {
            return Err(ApiError::BadRequest(
                "pension_amount_not_positive: pension.monthly_amount_today must be > 0".into(),
            ));
        }
        if pen.fraction_while_partial < Decimal::ZERO || pen.fraction_while_partial > Decimal::ONE {
            return Err(ApiError::BadRequest(
                "pension_fraction_out_of_range: pension.fraction_while_partial must be between 0 and 1 (fraction)".into(),
            ));
        }
    }
    if let Some(par) = &p.partial_retirement {
        if !(MIN_PROFILE_AGE..=horizon).contains(&par.starts_at_age) {
            return Err(ApiError::BadRequest(format!(
                "partial_age_out_of_range: partial_retirement.starts_at_age must be between {MIN_PROFILE_AGE} and horizon_lifespan_age ({horizon})"
            )));
        }
        if par.income_monthly_today < Decimal::ZERO {
            return Err(ApiError::BadRequest(
                "partial_income_negative: partial_retirement.income_monthly_today must be >= 0"
                    .into(),
            ));
        }
        // La fase parcial termina en la jubilación total: empezar después (o el mismo mes) la
        // dejaría vacía, y una fase vacía que la UI dibuja es peor que un error.
        if let Some(total) = p.target_retirement_age {
            if par.starts_at_age >= total {
                return Err(ApiError::BadRequest(
                    "partial_not_before_retirement: partial_retirement.starts_at_age must be lower than target_retirement_age".into(),
                ));
            }
        }
    }

    // ---- Colchón y umbral ------------------------------------------------------------------
    if let Some(m) = p.cash_buffer_months {
        if m > MAX_CASH_BUFFER_MONTHS {
            return Err(ApiError::BadRequest(format!(
                "cash_buffer_out_of_range: cash_buffer_months must be between 0 and {MAX_CASH_BUFFER_MONTHS}"
            )));
        }
    }
    if !(MIN_SUCCESS_THRESHOLD_PCT..=MAX_SUCCESS_THRESHOLD_PCT).contains(&p.success_threshold_pct) {
        return Err(ApiError::BadRequest(format!(
            "success_threshold_out_of_range: success_threshold_pct must be between {MIN_SUCCESS_THRESHOLD_PCT} and {MAX_SUCCESS_THRESHOLD_PCT}"
        )));
    }

    validate_withdrawal_rule(&p.withdrawal_rule)
}

/// Cada `kind` exige SUS campos y no los de otro. Un `percent_of_balance` sin `pct` no tiene
/// regla que aplicar, y aceptarlo devolvería una simulación que no retira nada sin decir por qué.
fn validate_withdrawal_rule(r: &WithdrawalRule) -> Result<(), ApiError> {
    let need_pct = |label: &str, v: Option<Decimal>, max: Decimal| -> Result<Decimal, ApiError> {
        let Some(v) = v else {
            return Err(ApiError::BadRequest(format!(
                "withdrawal_pct_required: withdrawal_rule.{label} is required for this rule kind"
            )));
        };
        if v <= Decimal::ZERO || v > max {
            return Err(ApiError::BadRequest(format!(
                "withdrawal_pct_out_of_range: withdrawal_rule.{label} must be greater than 0 and at most {max} (percent)"
            )));
        }
        Ok(v)
    };

    match r.kind {
        WithdrawalRuleKind::FixedReal => {}
        WithdrawalRuleKind::PercentOfBalance => {
            need_pct("pct", r.pct, MAX_WITHDRAWAL_PCT)?;
        }
        WithdrawalRuleKind::Hybrid => {
            let start = need_pct("start_pct", r.start_pct, MAX_WITHDRAWAL_PCT)?;
            let end = need_pct("end_pct", r.end_pct, MAX_WITHDRAWAL_PCT)?;
            if end >= start {
                return Err(ApiError::BadRequest(
                    "hybrid_end_pct_not_below_start: withdrawal_rule.end_pct must be lower than start_pct".into(),
                ));
            }
        }
        WithdrawalRuleKind::Guardrails => {
            need_pct("pct", r.pct, MAX_WITHDRAWAL_PCT)?;
            for (label, v) in [("band_pct", r.band_pct), ("adjust_pct", r.adjust_pct)] {
                let Some(v) = v else {
                    return Err(ApiError::BadRequest(format!(
                        "withdrawal_pct_required: withdrawal_rule.{label} is required for this rule kind"
                    )));
                };
                if v <= Decimal::ZERO || v > MAX_GUARDRAIL_PCT {
                    return Err(ApiError::BadRequest(format!(
                        "withdrawal_band_out_of_range: withdrawal_rule.{label} must be greater than 0 and at most {MAX_GUARDRAIL_PCT} (percent)"
                    )));
                }
            }
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Patchset campo a campo (tri-estado)
// ---------------------------------------------------------------------------

/// Cambios campo a campo del perfil. **Omitir = no cambiar**; el `Option<Option<T>>` de los
/// campos opcionales distingue además `null` (= borrar) de un valor.
///
/// `withdrawal_rule` se sustituye ENTERA y no campo a campo a propósito: cuáles de sus `pct` son
/// obligatorios depende de `kind`, así que un merge parcial permitiría llegar a estados como
/// «guardrails con el `pct` del percent_of_balance anterior» que nadie escribió.
#[derive(Debug, Default)]
pub(crate) struct RetirementProfilePatch {
    pub strategy: Option<RetirementStrategy>,
    pub target_retirement_age: Option<Option<u32>>,
    pub fire_number_mode: Option<FireNumberMode>,
    pub fire_number_manual_amount: Option<Option<Decimal>>,
    pub swr_pct: Option<Decimal>,
    pub horizon_lifespan_age: Option<u32>,
    pub target_basis: Option<Option<TargetBasis>>,
    pub bridge_discount_basis: Option<BridgeDiscountBasis>,
    pub withdrawal_rule: Option<WithdrawalRule>,
    pub pension: Option<Option<PensionPlan>>,
    pub partial_retirement: Option<Option<PartialRetirement>>,
    pub cash_buffer_months: Option<Option<u32>>,
    pub success_threshold_pct: Option<u32>,
}

impl RetirementProfilePatch {
    /// Aplica el patchset sobre una base y devuelve el resultado, **sin validar ni persistir**.
    /// Lo comparten el PATCH HTTP y la tool MCP — dos aplicadores se separan sin que ningún
    /// test lo note (la lección de `FireSettingsPatch::apply_to`).
    pub(crate) fn apply_to(&self, base: &RetirementProfile) -> RetirementProfile {
        let mut after = base.clone();
        if let Some(v) = self.strategy {
            after.strategy = v;
        }
        if let Some(v) = self.target_retirement_age {
            after.target_retirement_age = v;
        }
        if let Some(v) = self.fire_number_mode {
            after.fire_number_mode = v;
        }
        if let Some(v) = self.fire_number_manual_amount {
            after.fire_number_manual_amount = v;
        }
        if let Some(v) = self.swr_pct {
            after.swr_pct = v;
        }
        if let Some(v) = self.horizon_lifespan_age {
            after.horizon_lifespan_age = v;
        }
        if let Some(v) = self.target_basis {
            after.target_basis = v;
        }
        if let Some(v) = self.bridge_discount_basis {
            after.bridge_discount_basis = v;
        }
        if let Some(v) = self.withdrawal_rule.clone() {
            after.withdrawal_rule = v;
        }
        if let Some(v) = self.pension.clone() {
            after.pension = v;
        }
        if let Some(v) = self.partial_retirement.clone() {
            after.partial_retirement = v;
        }
        if let Some(v) = self.cash_buffer_months {
            after.cash_buffer_months = v;
        }
        if let Some(v) = self.success_threshold_pct {
            after.success_threshold_pct = v;
        }
        after
    }

    pub(crate) fn is_empty(&self) -> bool {
        // Destructuring exhaustivo y sin `..`: un campo nuevo deja de compilar hasta que alguien
        // decida si cuenta como «algo que cambiar».
        let RetirementProfilePatch {
            strategy,
            target_retirement_age,
            fire_number_mode,
            fire_number_manual_amount,
            swr_pct,
            horizon_lifespan_age,
            target_basis,
            bridge_discount_basis,
            withdrawal_rule,
            pension,
            partial_retirement,
            cash_buffer_months,
            success_threshold_pct,
        } = self;
        strategy.is_none()
            && target_retirement_age.is_none()
            && fire_number_mode.is_none()
            && fire_number_manual_amount.is_none()
            && swr_pct.is_none()
            && horizon_lifespan_age.is_none()
            && target_basis.is_none()
            && bridge_discount_basis.is_none()
            && withdrawal_rule.is_none()
            && pension.is_none()
            && partial_retirement.is_none()
            && cash_buffer_months.is_none()
            && success_threshold_pct.is_none()
    }
}

// ---------------------------------------------------------------------------
// Carga y persistencia
// ---------------------------------------------------------------------------

/// Carga y resuelve el perfil de UN usuario por un **único** camino de deserialización.
/// Fuente de verdad del perfil para todo el que lo necesite fuera de este módulo (la
/// proyección, `/v1/summary`, el export de backup).
pub async fn load_retirement_profile(
    pool: &PgPool,
    user_id: Uuid,
) -> Result<RetirementProfile, ApiError> {
    let stored: Option<SqlxJson<RetirementProfile>> =
        sqlx::query_scalar(r#"SELECT retirement_profile FROM users WHERE id = $1"#)
            .bind(user_id)
            .fetch_one(pool)
            .await?;
    Ok(resolve_retirement_profile(stored.map(|j| j.0)))
}

/// Perfil almacenado **sin resolver** (`None` = la columna es `NULL`). Lo necesitan el export de
/// backup —para no escribir un perfil que el usuario nunca configuró— y el import, que solo
/// siembra desde un fichero viejo si aquí no hay nada.
pub(crate) async fn stored_retirement_profile(
    pool: &PgPool,
    user_id: Uuid,
) -> Result<Option<RetirementProfile>, ApiError> {
    let stored: Option<SqlxJson<RetirementProfile>> =
        sqlx::query_scalar(r#"SELECT retirement_profile FROM users WHERE id = $1"#)
            .bind(user_id)
            .fetch_one(pool)
            .await?;
    Ok(stored.map(|j| j.0))
}

// ---------------------------------------------------------------------------
// HTTP
// ---------------------------------------------------------------------------

/// Respuesta de las dos rutas: el perfil YA resuelto (defaults y clamps aplicados) más la fecha
/// de nacimiento, que es la que convierte cada edad del perfil en un índice de mes.
///
/// Van juntas porque se editan juntas: una estrategia por edad sin `birth_date` degrada a `asap`
/// y la SPA tiene que poder decirlo en la misma pantalla.
#[derive(Debug, Serialize, ToSchema)]
pub struct RetirementProfileResponse {
    pub profile: RetirementProfile,
    #[schema(value_type = Option<String>, format = "date")]
    pub birth_date: Option<NaiveDate>,
}

/// Cuerpo del PATCH. Tri-estado en todo lo opcional: **omitir = no cambiar**, `null` = borrar.
#[derive(Debug, Deserialize, ToSchema)]
pub struct PatchRetirementProfileBody {
    #[serde(default)]
    pub strategy: Option<RetirementStrategy>,
    #[serde(default, deserialize_with = "crate::handlers::deserialize_double_option_typed")]
    #[schema(value_type = Option<u32>, nullable = true)]
    pub target_retirement_age: Option<Option<u32>>,
    #[serde(default)]
    pub fire_number_mode: Option<FireNumberMode>,
    #[serde(default, deserialize_with = "crate::handlers::deserialize_double_option_typed")]
    #[schema(value_type = Option<String>, nullable = true)]
    pub fire_number_manual_amount: Option<Option<String>>,
    #[serde(default)]
    #[schema(value_type = Option<String>)]
    pub swr_pct: Option<String>,
    #[serde(default)]
    pub horizon_lifespan_age: Option<u32>,
    #[serde(default, deserialize_with = "crate::handlers::deserialize_double_option_typed")]
    #[schema(value_type = Option<String>, nullable = true)]
    pub target_basis: Option<Option<TargetBasis>>,
    #[serde(default)]
    pub bridge_discount_basis: Option<BridgeDiscountBasis>,
    #[serde(default)]
    pub withdrawal_rule: Option<WithdrawalRule>,
    #[serde(default, deserialize_with = "crate::handlers::deserialize_double_option_typed")]
    #[schema(value_type = Option<PensionPlan>, nullable = true)]
    pub pension: Option<Option<PensionPlan>>,
    #[serde(default, deserialize_with = "crate::handlers::deserialize_double_option_typed")]
    #[schema(value_type = Option<PartialRetirement>, nullable = true)]
    pub partial_retirement: Option<Option<PartialRetirement>>,
    #[serde(default, deserialize_with = "crate::handlers::deserialize_double_option_typed")]
    #[schema(value_type = Option<u32>, nullable = true)]
    pub cash_buffer_months: Option<Option<u32>>,
    #[serde(default)]
    pub success_threshold_pct: Option<u32>,
    /// Misma columna que `PATCH /v1/auth/me` (`users.birth_date`): `null` la borra,
    /// `"YYYY-MM-DD"` la fija, omitirla no la toca. Vive también aquí porque la fecha de
    /// nacimiento es lo que convierte las edades del perfil en meses — pedirla en otra pantalla
    /// es garantizar que la mitad de los perfiles por edad se queden sin ella.
    #[serde(default, deserialize_with = "crate::handlers::deserialize_double_option")]
    #[schema(nullable = true, value_type = Object)]
    pub birth_date: Option<Value>,
}

impl PatchRetirementProfileBody {
    /// Convierte el cuerpo HTTP en el patchset de dominio (parseando los decimales de string).
    fn to_patch(&self) -> Result<RetirementProfilePatch, ApiError> {
        Ok(RetirementProfilePatch {
            strategy: self.strategy,
            target_retirement_age: self.target_retirement_age,
            fire_number_mode: self.fire_number_mode,
            fire_number_manual_amount: match &self.fire_number_manual_amount {
                None => None,
                Some(None) => Some(None),
                Some(Some(raw)) => Some(Some(parse_profile_decimal(
                    "fire_number_manual_amount",
                    raw,
                )?)),
            },
            swr_pct: self
                .swr_pct
                .as_deref()
                .map(|v| parse_profile_decimal("swr_pct", v))
                .transpose()?,
            horizon_lifespan_age: self.horizon_lifespan_age,
            target_basis: self.target_basis,
            bridge_discount_basis: self.bridge_discount_basis,
            withdrawal_rule: self.withdrawal_rule.clone(),
            pension: self.pension.clone(),
            partial_retirement: self.partial_retirement.clone(),
            cash_buffer_months: self.cash_buffer_months,
            success_threshold_pct: self.success_threshold_pct,
        })
    }
}

/// Parseo de un decimal del wire. Reusa el código `decimal_invalid` que ya existe en el catálogo
/// (y que la SPA ya traduce) en vez de inventar uno nuevo para decir lo mismo. El prefijo va como
/// literal —solo se interpola la etiqueta— para que `error_codes_parity` siga viéndolo.
pub(crate) fn parse_profile_decimal(label: &str, raw: &str) -> Result<Decimal, ApiError> {
    use std::str::FromStr;
    Decimal::from_str(raw.trim()).map_err(|_| {
        ApiError::BadRequest(format!("decimal_invalid: {label} must be a valid decimal string"))
    })
}

pub fn retirement_profile_router() -> Router {
    Router::new().route(
        "/me/retirement-profile",
        get(get_retirement_profile).patch(patch_retirement_profile),
    )
}

/// Core sin HTTP del GET: lo comparten el handler y la tool MCP `get_retirement_profile`.
pub(crate) async fn get_retirement_profile_core(
    pool: &PgPool,
    user_id: Uuid,
) -> Result<RetirementProfileResponse, ApiError> {
    let row: Option<(Option<SqlxJson<RetirementProfile>>, Option<NaiveDate>)> =
        sqlx::query_as(r#"SELECT retirement_profile, birth_date FROM users WHERE id = $1"#)
            .bind(user_id)
            .fetch_optional(pool)
            .await?;
    let (stored, birth_date) = row.ok_or(ApiError::NotFound)?;
    Ok(RetirementProfileResponse {
        profile: resolve_retirement_profile(stored.map(|j| j.0)),
        birth_date,
    })
}

#[utoipa::path(
    get,
    path = "/v1/auth/me/retirement-profile",
    tag = "auth",
    responses(
        (status = 200, description = "Perfil de jubilación del usuario de la sesión (resuelto) + su fecha de nacimiento", body = RetirementProfileResponse),
        (status = 401, description = "No valid session"),
    )
)]
pub async fn get_retirement_profile(
    Extension(state): Extension<Arc<AppState>>,
    jar: CookieJar,
) -> Result<Json<RetirementProfileResponse>, ApiError> {
    let user = require_session_user(&jar, &state.pool).await?;
    Ok(Json(
        get_retirement_profile_core(&state.pool, user.id.0).await?,
    ))
}

/// Core del PATCH, compartido por HTTP y por la tool MCP `update_retirement_profile`.
///
/// **Cualquier rol puede editar su PROPIO perfil, `viewer` incluido.** No es una excepción a la
/// política de roles: el perfil de jubilación es un dato personal del usuario del token, no
/// configuración del hogar. Un viewer que no puede fijar su propia edad de jubilación no puede
/// ver su propia proyección — que es justo lo que un viewer sí puede hacer.
///
/// Con `apply = false` valida y devuelve el before/after sin tocar nada (preview de la tool).
pub(crate) async fn patch_retirement_profile_core(
    state: &Arc<AppState>,
    user_id: Uuid,
    patchset: RetirementProfilePatch,
    birth_date_patch: Option<Option<NaiveDate>>,
    apply: bool,
) -> Result<RetirementProfilePatchOutcome, ApiError> {
    if patchset.is_empty() && birth_date_patch.is_none() {
        return Err(ApiError::BadRequest(
            "patch_empty: provide at least one retirement-profile field to change".into(),
        ));
    }

    let row: Option<(Option<SqlxJson<RetirementProfile>>, Option<NaiveDate>)> =
        sqlx::query_as(r#"SELECT retirement_profile, birth_date FROM users WHERE id = $1"#)
            .bind(user_id)
            .fetch_optional(&state.pool)
            .await?;
    let (stored, birth_before) = row.ok_or(ApiError::NotFound)?;
    let stored = stored.map(|j| j.0);

    // **El merge va sobre lo ALMACENADO, no sobre lo resuelto.** Es la diferencia entre «no lo he
    // elegido» y «he elegido esto», y `target_basis` es justo el campo donde importa: su valor
    // resuelto se DERIVA de si hay pensión (R6). Mergeando sobre el resuelto, el `perpetuity`
    // derivado de un perfil sin pensión se persistiría como si el usuario lo hubiera pedido, y al
    // declarar después su pensión el objetivo se quedaría en perpetuidad — la opción conservadora
    // que nadie pidió, sin ningún aviso. Lo mismo valdría para cualquier campo derivado futuro.
    let base = stored.clone().unwrap_or_else(default_retirement_profile);
    let before = resolve_retirement_profile(stored);

    let after_stored = patchset.apply_to(&base);
    // La validación corre sobre el mergeado SIN clamps: un valor fuera de rango se RECHAZA, no se
    // reescribe en silencio (el clamp es solo la red de las vías no validadas — ver `resolve_*`).
    validate_retirement_profile(&after_stored)?;
    let after = resolve_retirement_profile(Some(after_stored.clone()));

    let birth_after = match birth_date_patch {
        None => birth_before,
        Some(v) => {
            if let Some(d) = v {
                crate::handlers::auth::validate_birth_date(d)?;
            }
            v
        }
    };

    if apply {
        sqlx::query(
            r#"UPDATE users SET retirement_profile = $1, birth_date = $2 WHERE id = $3"#,
        )
        .bind(SqlxJson(&after_stored))
        .bind(birth_after)
        .bind(user_id)
        .execute(&state.pool)
        .await?;
        // El perfil es un INPUT del motor (SWR, modo del objetivo, edad límite del horizonte y,
        // desde WP5, la fase entera): toda escritura invalida la proyección. `birth_date` lo es
        // también — mueve el eje de edad y el horizonte.
        if let Ok((iid, _)) = require_installation_member(&state.pool, user_id).await {
            refresh_projection_after_mutation(state, iid, user_id).await;
        }
    }

    Ok(RetirementProfilePatchOutcome {
        before,
        after,
        birth_date_before: birth_before,
        birth_date_after: birth_after,
    })
}

/// Before/after del merge (el preview de la tool los enseña; el apply además persiste).
#[derive(Debug, Serialize)]
pub(crate) struct RetirementProfilePatchOutcome {
    pub before: RetirementProfile,
    pub after: RetirementProfile,
    pub birth_date_before: Option<NaiveDate>,
    pub birth_date_after: Option<NaiveDate>,
}

#[utoipa::path(
    patch,
    path = "/v1/auth/me/retirement-profile",
    tag = "auth",
    request_body = PatchRetirementProfileBody,
    responses(
        (status = 200, description = "Perfil actualizado (resuelto) + fecha de nacimiento", body = RetirementProfileResponse),
        (status = 400, description = "Validation error"),
        (status = 401, description = "No valid session"),
    )
)]
pub async fn patch_retirement_profile(
    Extension(state): Extension<Arc<AppState>>,
    jar: CookieJar,
    Json(body): Json<PatchRetirementProfileBody>,
) -> Result<Json<RetirementProfileResponse>, ApiError> {
    let user = require_session_user(&jar, &state.pool).await?;
    let patch = body.to_patch()?;
    let birth_patch = match &body.birth_date {
        None => None,
        Some(raw) => Some(crate::handlers::auth::parse_me_birth_patch(raw)?),
    };
    let outcome =
        patch_retirement_profile_core(&state, user.id.0, patch, birth_patch, true).await?;
    Ok(Json(RetirementProfileResponse {
        profile: outcome.after,
        birth_date: outcome.birth_date_after,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_absent_profile_is_the_4_15_behaviour() {
        let p = resolve_retirement_profile(None);
        assert_eq!(p.strategy, RetirementStrategy::Asap);
        assert_eq!(p.swr_pct, Decimal::new(35, 1));
        assert_eq!(p.horizon_lifespan_age, 90);
        assert_eq!(p.fire_number_mode, FireNumberMode::AnnualExpense);
        assert_eq!(p.withdrawal_rule.kind, WithdrawalRuleKind::FixedReal);
        assert_eq!(p.withdrawal_rule.spend_mode, SpendMode::Ceiling);
        assert_eq!(p.target_basis, Some(TargetBasis::Perpetuity));
        assert_eq!(p.success_threshold_pct, 95);
    }

    #[test]
    fn an_empty_json_object_resolves_to_the_defaults() {
        let stored: RetirementProfile = serde_json::from_str("{}").expect("{} es un perfil válido");
        assert_eq!(resolve_retirement_profile(Some(stored)), resolve_retirement_profile(None));
    }

    /// La forma EXACTA que escribe la migración 5.0.0 para los usuarios existentes.
    #[test]
    fn the_migration_shape_parses_and_keeps_the_four_moved_axes() {
        let stored: RetirementProfile = serde_json::from_str(
            r#"{"strategy":"asap","fire_number_mode":"current_income","swr_pct":"3.0","horizon_lifespan_age":95}"#,
        )
        .expect("forma de la migración");
        let p = resolve_retirement_profile(Some(stored));
        assert_eq!(p.fire_number_mode, FireNumberMode::CurrentIncome);
        assert_eq!(p.swr_pct, Decimal::new(30, 1));
        assert_eq!(p.horizon_lifespan_age, 95);
    }

    #[test]
    fn target_basis_is_derived_from_the_pension_when_not_set() {
        let mut p = default_retirement_profile();
        p.pension = Some(PensionPlan {
            monthly_amount_today: Decimal::from(1200u32),
            starts_at_age: 67,
            indexed: true,
            fraction_while_partial: Decimal::ZERO,
        });
        assert_eq!(
            resolve_retirement_profile(Some(p.clone())).target_basis,
            Some(TargetBasis::BridgeToPension)
        );
        // …y `perpetuity` explícito gana: es la opción «ignorar la pensión».
        p.target_basis = Some(TargetBasis::Perpetuity);
        assert_eq!(
            resolve_retirement_profile(Some(p)).target_basis,
            Some(TargetBasis::Perpetuity)
        );
    }

    #[test]
    fn pension_bridge_forces_the_bridge_basis() {
        let mut p = default_retirement_profile();
        p.strategy = RetirementStrategy::PensionBridge;
        p.target_basis = Some(TargetBasis::Perpetuity);
        p.pension = Some(PensionPlan {
            monthly_amount_today: Decimal::from(900u32),
            starts_at_age: 65,
            indexed: false,
            fraction_while_partial: Decimal::ZERO,
        });
        assert_eq!(
            resolve_retirement_profile(Some(p)).target_basis,
            Some(TargetBasis::BridgeToPension)
        );
    }

    #[test]
    fn out_of_range_values_are_clamped_on_read_never_rejected() {
        let mut p = default_retirement_profile();
        p.swr_pct = Decimal::from(99u32);
        p.horizon_lifespan_age = 200;
        p.success_threshold_pct = 5;
        p.cash_buffer_months = Some(999);
        p.target_retirement_age = Some(3);
        let r = resolve_retirement_profile(Some(p));
        assert_eq!(r.swr_pct, MAX_SWR_PCT);
        assert_eq!(r.horizon_lifespan_age, MAX_HORIZON_LIFESPAN_AGE);
        assert_eq!(r.success_threshold_pct, MIN_SUCCESS_THRESHOLD_PCT);
        assert_eq!(r.cash_buffer_months, Some(MAX_CASH_BUFFER_MONTHS));
        assert_eq!(r.target_retirement_age, Some(MIN_PROFILE_AGE));
    }

    #[test]
    fn strategies_by_age_require_the_age() {
        for s in [RetirementStrategy::RetireAtAge, RetirementStrategy::Coast] {
            let mut p = default_retirement_profile();
            p.strategy = s;
            let err = validate_retirement_profile(&p).expect_err("sin edad debe fallar");
            assert!(
                matches!(&err, ApiError::BadRequest(m) if m.starts_with("target_retirement_age_required: ")),
                "{err:?}"
            );
        }
    }

    #[test]
    fn pension_bridge_requires_a_pension() {
        let mut p = default_retirement_profile();
        p.strategy = RetirementStrategy::PensionBridge;
        let err = validate_retirement_profile(&p).expect_err("sin pensión debe fallar");
        assert!(
            matches!(&err, ApiError::BadRequest(m) if m.starts_with("pension_required_for_bridge: ")),
            "{err:?}"
        );
    }

    #[test]
    fn each_withdrawal_kind_demands_its_own_fields() {
        let mut p = default_retirement_profile();

        p.withdrawal_rule = WithdrawalRule {
            kind: WithdrawalRuleKind::PercentOfBalance,
            ..WithdrawalRule::default()
        };
        assert!(validate_retirement_profile(&p).is_err(), "percent sin pct");

        p.withdrawal_rule.pct = Some(Decimal::from(4u32));
        validate_retirement_profile(&p).expect("percent con pct");

        p.withdrawal_rule = WithdrawalRule {
            kind: WithdrawalRuleKind::Hybrid,
            start_pct: Some(Decimal::from(3u32)),
            end_pct: Some(Decimal::from(5u32)),
            ..WithdrawalRule::default()
        };
        let err = validate_retirement_profile(&p).expect_err("end >= start");
        assert!(
            matches!(&err, ApiError::BadRequest(m) if m.starts_with("hybrid_end_pct_not_below_start: ")),
            "{err:?}"
        );

        p.withdrawal_rule.end_pct = Some(Decimal::from(2u32));
        validate_retirement_profile(&p).expect("hybrid coherente");

        p.withdrawal_rule = WithdrawalRule {
            kind: WithdrawalRuleKind::Guardrails,
            pct: Some(Decimal::from(4u32)),
            band_pct: Some(Decimal::from(20u32)),
            ..WithdrawalRule::default()
        };
        assert!(validate_retirement_profile(&p).is_err(), "guardrails sin adjust");
        p.withdrawal_rule.adjust_pct = Some(Decimal::from(10u32));
        validate_retirement_profile(&p).expect("guardrails completo");
    }

    #[test]
    fn the_partial_phase_must_start_before_the_full_retirement() {
        let mut p = default_retirement_profile();
        p.strategy = RetirementStrategy::Partial;
        p.target_retirement_age = Some(60);
        p.partial_retirement = Some(PartialRetirement {
            starts_at_age: 60,
            income_monthly_today: Decimal::from(1000u32),
            expense_basis: PartialExpenseBasis::Retirement,
        });
        let err = validate_retirement_profile(&p).expect_err("parcial no anterior");
        assert!(
            matches!(&err, ApiError::BadRequest(m) if m.starts_with("partial_not_before_retirement: ")),
            "{err:?}"
        );
        p.partial_retirement.as_mut().unwrap().starts_at_age = 55;
        validate_retirement_profile(&p).expect("parcial antes de la total");
    }

    #[test]
    fn the_patch_only_touches_what_it_names() {
        let base = RetirementProfile {
            swr_pct: Decimal::new(30, 1),
            pension: Some(PensionPlan {
                monthly_amount_today: Decimal::from(1100u32),
                starts_at_age: 67,
                indexed: true,
                fraction_while_partial: Decimal::ZERO,
            }),
            ..default_retirement_profile()
        };
        let patch = RetirementProfilePatch {
            swr_pct: Some(Decimal::new(35, 1)),
            ..RetirementProfilePatch::default()
        };
        let after = patch.apply_to(&base);
        assert_eq!(after.swr_pct, Decimal::new(35, 1));
        assert_eq!(after.pension, base.pension, "la pensión NO se resetea");

        // `null` explícito sí borra.
        let clear = RetirementProfilePatch {
            pension: Some(None),
            ..RetirementProfilePatch::default()
        };
        assert_eq!(clear.apply_to(&base).pension, None);
        assert!(RetirementProfilePatch::default().is_empty());
        assert!(!clear.is_empty());
    }
}
