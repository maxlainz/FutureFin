//! Monthly projection via `futurefin-engine`: presupuesto regular, cuotas de pasivos activos,
//! aportaciones a activos / drenaje / crecimiento. Los «Próximos» ajustan la caja por mes (fechas
//! explícitas en su mes civil, las vencidas íntegras en el mes ancla; sin fecha repartidas en
//! 90 días desde el día 1 del mes ancla — #126).

use crate::error::ApiError;
use crate::handlers::budget::ledger_regular_monthly_income_and_expense;
use crate::handlers::installation::{
    load_fire_settings, naive_date_in_calendar_tz, require_installation_member,
    resolve_fire_settings, FireNumberMode, FireSettings, SavingsSource,
};
use futurefin_engine::gross_up_net_annual_fire;
use crate::handlers::retirement_profile::{
    resolve_retirement_profile, BridgeDiscountBasis, PartialExpenseBasis, RetirementProfile,
    RetirementStrategy, SpendMode as ProfileSpendMode, TargetBasis as ProfileTargetBasis,
    WithdrawalRule as ProfileWithdrawalRule, WithdrawalRuleKind as ProfileWithdrawalRuleKind,
};
use crate::handlers::person_view::LedgerView;
/// Monte Carlo (5.0.0, WP6b): el eje `monte_carlo` de `simulate_projection` reusa las MISMAS
/// conversiones y el mismo semáforo que `GET /v1/projection/bands`. Ninguna cifra estadística
/// tiene dos caminos.
use crate::handlers::projection_bands::{map_mc_err, probability_out, volatilities_f64};
/// Alias local: `RepaymentModel` a secas es el del **engine** en este fichero (ver el `use` de
/// `futurefin_engine`); este es el del lado API, que sabe hablar con la columna SQL.
use crate::handlers::liabilities::{payoff_absence_code, RepaymentModel as LiabRepaymentModel};
use crate::handlers::installation::AvgWindowMode;
use crate::handlers::transactions::summary::{transactions_avg, AvgSide, TransactionsAvg};
use crate::handlers::session::require_session_user;
use crate::state::{AppState, Density, ProjectionCacheKey};
use axum::extract::{Extension, Query};
use axum::routing::get;
use axum::{Json, Router};
use axum_extra::extract::cookie::CookieJar;
use chrono::{Datelike, Duration, Months, NaiveDate};
use futurefin_engine::{
    fire_target_at_month_index_with_plan, first_month_per_asset_contribution_nominals, project_net_worth_series, AllocationCap,
    AllocationKind, AllocationRule, CoastSolve, EngineError, ExpenseBasis as EngineExpenseBasis,
    FireTarget, PartialPhase, PensionSchedule, Phase, PhasePlan, ProjectionInput,
    ProjectionLiabilityInput, RepaymentModel, SimAsset, SolveResult,
    SpendMode as EngineSpendMode, TargetBasis as EngineTargetBasis,
    WithdrawalRule as EngineWithdrawalRule,
};
use rust_decimal::MathematicalOps;
use rust_decimal::prelude::ToPrimitive;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use std::collections::HashMap;
use std::sync::Arc;
use utoipa::ToSchema;
use uuid::Uuid;

#[derive(Debug, Deserialize)]
pub struct ProjectionSeriesQuery {
    #[serde(default)]
    pub view: Option<String>,
    /// Meses a proyectar (12–840; fuera de rango es 400 `months_out_of_range`, NO se clampa).
    /// Si se omite: horizonte derivado de la instalación (véase `horizon_basis`).
    pub months: Option<u32>,
    /// `monthly` (default) o `hybrid` (mes 0..12 mensual + anual desde 24). Reduce el JSON ~5× con `hybrid`.
    #[serde(default)]
    pub density: Option<String>,
}

/// `hybrid` o `monthly` (default). Cualquier otro valor es un error, no un `monthly` silencioso:
/// pedir una densidad que no existe y recibir 841 puntos sin aviso es la misma clase de fallo que
/// `view` (auditoría MCP §4). Solo alcanzable por HTTP — la tool MCP fuerza `Hybrid`.
/// Escala de salida de los importes: 4 decimales, la misma de `NUMERIC(18,4)` en la base y la que
/// ya publican `net_worth` y compañía.
///
/// Aplica **solo a la copia que se serializa**, nunca a la que entra al motor. El engine capitaliza
/// con `annual_factor.powd(1/12)` —una raíz duodécima irracional— y el target FIRE sale de
/// `gross / (swr/100)`; ninguna de las dos se redondeaba, así que la escala saturaba en los ~28
/// dígitos significativos de `rust_decimal` y salían al wire cosas como
/// `"69946992.976753373554690255548"` (auditoría MCP §7). Eso es ruido y tokens, y empuja al consumidor
/// a presentar cifras con precisión falsa.
///
/// Redondear la copia que alimenta al motor movería el cruce FIRE: `fire_target_base` es
/// `FireTarget.base_amount`. Por eso el redondeo vive aquí, en la construcción de la respuesta.
///
/// Precedente: 3.8.0 hizo exactamente esto con los ratios (`round_ratio`, 6 dp) y el runway (1 dp)
/// y dejó fuera los importes de proyección y FIRE. Esto cierra el hueco.
use crate::money::money_out;

fn resolve_density(q: &ProjectionSeriesQuery) -> Result<Density, ApiError> {
    match q.density.as_deref().map(str::trim) {
        None | Some("") | Some("monthly") => Ok(Density::Monthly),
        Some("hybrid") => Ok(Density::Hybrid),
        Some(_) => Err(ApiError::BadRequest(
            "invalid_density: density must be 'monthly' or 'hybrid'".into(),
        )),
    }
}

/// Indices a incluir en el response según la densidad. Para `Hybrid`: mes 0..12
/// mensual + mes 24, 36, … y **siempre el último mes del horizonte**.
///
/// Ese último empujón no es cosmético. El bucle anual solo emite múltiplos de 12, así que con
/// un horizonte que no lo fuera la serie se cortaba antes de tiempo **sin decir nada**: con
/// `?months=100&density=hybrid` el último punto era el mes 96 y los meses 97–100 no existían
/// en `points`, ni en `fire_target_series`, ni en `asset_series[].values`. Desaparecía además
/// el punto que cualquiera lee como «patrimonio al final». Con `?months=19` se perdía el 32 %
/// del horizonte pedido. Invisible desde la web (el horizonte derivado siempre es años × 12),
/// pero alcanzable por `?months=N` y por la tool MCP `get_projection`, que fuerza `hybrid`.
pub(crate) fn density_month_indices(density: Density, months: u32) -> Vec<u32> {
    match density {
        Density::Monthly => (0..months).collect(),
        Density::Hybrid => {
            let cap = months.saturating_sub(1);
            let mut v: Vec<u32> = (0..=12u32.min(cap)).collect();
            let mut k = 24u32;
            while k <= cap {
                v.push(k);
                k += 12;
            }
            if v.last() != Some(&cap) {
                v.push(cap);
            }
            v
        }
    }
}



/// Por qué NO hay target FIRE. Devolver `None` a secas se tragaba tres causas muy distintas, y
/// desde 4.0.0 la simulación puede llegar a ellas por caminos nuevos (un recorte que deja la base
/// de gasto en 0, un override de modo a `manual` sin importe, `swr_pct: 0`). Sin la razón, esos
/// escenarios se leen como un fallo de la herramienta (auditoría de simulate_projection §8).
pub(crate) const FIRE_ABSENT_MANUAL_AMOUNT_MISSING: &str = "manual_amount_missing";
pub(crate) const FIRE_ABSENT_NET_NEED_NOT_POSITIVE: &str = "net_need_not_positive";
pub(crate) const FIRE_ABSENT_SWR_NOT_POSITIVE: &str = "swr_not_positive";

/// `Err(razón)` en vez de `None`: el valor ausente y su causa viajan juntos, así que ningún caller
/// puede publicar el hueco sin poder explicarlo.
/// La NECESIDAD del objetivo por modo (#170) — devuelve los ingredientes, no el resultado: el
/// engine evalúa `gross_up(need(k))/SWR` mes a mes. Las tres razones de ausencia se deciden
/// AQUÍ, una vez, sobre el estado de HOY (la puerta k=0 del engine las respalda).
fn compute_fire_need(
    profile: &RetirementProfile,
    income_monthly: Decimal,
    income_retirement_monthly: Decimal,
    expense_monthly: Decimal,
) -> Result<futurefin_engine::FireNeed, &'static str> {
    use futurefin_engine::FireNeed;
    // 5.0.0: el modo del objetivo, el importe manual y el SWR son del PERFIL del usuario, no del
    // hogar (D13). El resto de la fiscalidad (`taxes_enabled`, tramos, g) sigue siendo compartida.
    let need = match profile.fire_number_mode {
        FireNumberMode::Manual => {
            let amt = profile
                .fire_number_manual_amount
                .ok_or(FIRE_ABSENT_MANUAL_AMOUNT_MISSING)?;
            if amt <= Decimal::ZERO {
                return Err(FIRE_ABSENT_MANUAL_AMOUNT_MISSING);
            }
            FireNeed::Indexed { annual_net_today: amt }
        }
        FireNumberMode::AnnualExpense => {
            let net = expense_monthly - income_retirement_monthly;
            if net <= Decimal::ZERO {
                return Err(FIRE_ABSENT_NET_NEED_NOT_POSITIVE);
            }
            // El gasto se indexa, la pensión queda plana (#139) — el engine evalúa
            // `max(0, E·f(k) − I)·12` mes a mes, no un neto pre-restado inflado entero.
            FireNeed::ExpenseMinusPension {
                expense_monthly,
                pension_monthly: income_retirement_monthly,
            }
        }
        FireNumberMode::CurrentIncome => {
            let net = income_monthly - income_retirement_monthly;
            if net <= Decimal::ZERO {
                return Err(FIRE_ABSENT_NET_NEED_NOT_POSITIVE);
            }
            // Cifra en euros de HOY que se indexa entera: descomponerla con los ingresos
            // planos de #139 dejaría el objetivo PLANO — cambio semántico que nadie pidió.
            FireNeed::Indexed { annual_net_today: net * Decimal::from(12u32) }
        }
    };
    if profile.swr_pct <= Decimal::ZERO {
        return Err(FIRE_ABSENT_SWR_NOT_POSITIVE);
    }
    Ok(need)
}

/// Serializa un Decimal como f64 (~15 dígitos de precisión, suficiente para
/// display de horizontes de 70 años). Reduce ~30 KB JSON y elimina ~5.000
/// llamadas a parseDisplayDecimal en el cliente. Los KPIs/totales escalares
/// que requieren precisión decimal siguen usando `rust_decimal::serde::str`.
/// `pub(crate)`: la ÚNICA otra superficie autorizada a usarla son los arrays
/// por punto de `/v1/history/series` (`handlers/history.rs`) — misma
/// justificación chart-only (excepción D4/I3 del architecture contract).
pub(crate) fn serialize_decimal_as_f64<S: serde::Serializer>(d: &Decimal, s: S) -> Result<S::Ok, S::Error> {
    s.serialize_f64(d.to_f64().unwrap_or(0.0))
}

/// Punto **servido** de la serie. Los tres importes son números f64 (excepción chart-only D4/I3).
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct ProjectionPoint {
    /// Número de MES desde `anchor_date_ymd`, **nunca** la posición en el array: con
    /// `density=hybrid` los puntos no son equidistantes.
    pub month_index: u32,
    /// Patrimonio neto en euros **NOMINALES** del mes `month_index` (euros del momento).
    #[serde(serialize_with = "serialize_decimal_as_f64")]
    #[schema(value_type = f64)]
    pub net_worth: Decimal,
    #[serde(serialize_with = "serialize_decimal_as_f64")]
    #[schema(value_type = f64)]
    pub contributed_capital: Decimal,
    /// El mismo patrimonio en **euros de HOY**: `net_worth / (1 + inflación%)^(month_index/12)`,
    /// con la `deflation_annual_inflation_percent` que la respuesta declara al lado.
    ///
    /// **Esto es capa de PRESENTACIÓN, no un cambio de modelo.** El motor sigue simulando en
    /// nominal y solo el objetivo FIRE se ajusta por inflación (`fire_target_at_month_index`);
    /// aquí se divide un resultado ya calculado. Simular EN euros de hoy es el modelo «real puro»
    /// de la v1.0.12, **rechazado** en la v1.2.0 porque mezclaba marcos —deflactaba las
    /// rentabilidades y dejaba los flujos y el objetivo fijos— y drenaba los activos ANTES de la
    /// jubilación con la inflación encendida (guardia viva:
    /// `fire_target_with_inflation_does_not_trigger_early_drain`). Deflactar el output no reabre
    /// nada de eso: la comparación patrimonio↔objetivo, el mes de cruce y la cascada se siguen
    /// decidiendo en el mismo marco nominal de siempre, y el cruce es además invariante
    /// (`nominal ≥ base·(1+i)^(k/12)` ⟺ `deflactado ≥ base`).
    ///
    /// Se sirve **siempre**, con inflación 0 incluida: ahí el deflactor es exactamente `1` y este
    /// campo es el mismo valor que `net_worth`. Omitirlo cuando no hay inflación dejaría a un
    /// consumidor sin poder distinguir «no hay inflación» de «esta versión no publica el campo»
    /// — el mismo fallo que ya costó los cuatro campos de jubilación.
    ///
    /// Deliberadamente NO hay `contributed_capital_real`: la pregunta es «mi patrimonio en euros
    /// de hoy», el coste aportado en euros de hoy no lo pide nadie, y `milestones_real` ya sienta
    /// el precedente de deflactar solo el patrimonio.
    #[serde(serialize_with = "serialize_decimal_as_f64")]
    #[schema(value_type = f64)]
    pub net_worth_real: Decimal,
    /// Patrimonio LÍQUIDO nominal del mes (4.8.0, #143): Σ activos `is_liquid` + caja sin
    /// repartir, SIN restar pasivos. **Es la base que decide el cruce FIRE** — la línea que hay
    /// que comparar contra `fire_target_series`; `net_worth` sigue siendo el total del chart.
    #[serde(serialize_with = "serialize_decimal_as_f64")]
    #[schema(value_type = f64)]
    pub net_worth_liquid: Decimal,
    /// **Retirada NETA del mes** (5.0.0, §B.8): los euros que de verdad salieron de los activos
    /// para cubrir el déficit de caja, en euros NOMINALES del mes. `0` en los meses de
    /// acumulación y en el mes 0 (que es el estado de hoy, no un mes simulado).
    ///
    /// No es el gasto: el gasto se cubre primero con el ingreso de la fase. Y no es la venta
    /// BRUTA: el impuesto de la plusvalía realizada se paga vendiendo de más, y ese exceso vive
    /// dentro del patrimonio, no aquí.
    #[serde(serialize_with = "serialize_decimal_as_f64")]
    #[schema(value_type = f64)]
    pub withdrawal: Decimal,
    /// **Recorte de la regla de retirada**: `max(0, necesidad − permitido)` (D22/D24). Es
    /// INFORMATIVO — no resta patrimonio, no cuenta como fracaso y **no es**
    /// `uncovered_deficit_total`, que mide lo que los activos no pudieron vender. Todo ceros
    /// mientras la regla sea `fixed_real` (no tiene techo): lo llena WP2.
    #[serde(serialize_with = "serialize_decimal_as_f64")]
    #[schema(value_type = f64)]
    pub withdrawal_shortfall: Decimal,
    /// **Exceso de la regla sobre la necesidad** en modo `rule_is_spend` (se vende y se gasta).
    /// Todo ceros con `fixed_real` / `ceiling`, por la misma razón. Lo llena WP2.
    #[serde(serialize_with = "serialize_decimal_as_f64")]
    #[schema(value_type = f64)]
    pub withdrawal_excess: Decimal,
    /// **Gasto del mes que los activos NO pudieron financiar**, neto y `≥ 0`: el incremento
    /// mensual de `uncovered_deficit_total`. **El recorte de la REGLA es `withdrawal_shortfall`,
    /// no esto** — son dos magnitudes distintas y confundirlas fue el hallazgo B2 de la revisión
    /// adversarial del motor.
    ///
    /// Es la tercera columna del mes y la que faltaba: `withdrawal` es lo que se obtuvo,
    /// `withdrawal_shortfall` lo que la regla rechazó y `unmet_need` lo que la CARTERA no dio;
    /// su suma es la necesidad neta del mes. Sin ella, cualquier cociente de cobertura miente en
    /// el caso que más importa —la cartera agotada— porque con `fixed_real` el recorte es cero
    /// por construcción. `0` en el mes 0 (estado de hoy, no un mes simulado) y en todo mes que
    /// la cartera pudo pagar entero.
    ///
    /// El motor conserva en el acumulador el operando literal de 4.15.0 —`after_tax(gross_up(n))`
    /// devuelve `n` solo hasta el redondeo a 28 dígitos—, así que su serie llega con una polvareda
    /// de ±1e-25 € incluso en un hogar perfectamente solvente. **Lo que se publica aquí va
    /// clampado a 0 y redondeado a la escala monetaria** (4 decimales): es la única columna del
    /// punto cuyo signo se lee como un veredicto, y servir la polvareda encendería meses en rojo
    /// al azar. `> 0` significa euros de verdad.
    #[serde(serialize_with = "serialize_decimal_as_f64")]
    #[schema(value_type = f64)]
    pub unmet_need: Decimal,
}

/// Una transición de fase de la simulación: en qué mes de la rejilla empieza cada fase.
///
/// Es la fuente del «carril de fases» del chart (D29). Va como lista y no como tres índices
/// sueltos porque el orden ES el dato: las fases son monótonas (acumulación → media jornada →
/// jubilado) y una fase que no ocurre simplemente no aparece.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct PhaseTransition {
    /// `accumulating` | `partial` | `retired`.
    pub phase: &'static str,
    /// Mes de la REJILLA (misma base que `points[].month_index`) en que empieza la fase.
    pub month_index: u32,
}

/// Lecturas de UN miembro dentro del agregado del hogar (D9 / §D).
///
/// **No lleva series**: el hogar publica UNA curva (la suma) y esto explica de quién es cada
/// marcador. Servir N series completas multiplicaría el payload por el número de miembros para
/// responder a una pregunta —«¿cuándo se jubila cada uno?»— que se contesta con seis enteros.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct HouseholdMemberProjection {
    #[schema(value_type = String, format = "uuid")]
    pub user_id: Uuid,
    pub username: String,
    /// Estrategia de jubilación de ESTE miembro (`asap` | `retire_at_age` | `coast` | `partial`
    /// | `pension_bridge`). Cada uno corre la suya: el hogar no tiene una.
    pub strategy: String,
    /// Mes EFECTIVO de jubilación de este miembro, en la rejilla común del hogar. `null` = no se
    /// jubila dentro del horizonte.
    pub jubilacion_month_index: Option<u32>,
    /// Años cumplidos de ESTE miembro en ese mes (con SU fecha de nacimiento, no la del
    /// solicitante). `null` si no se jubila o si no tiene fecha declarada.
    pub jubilacion_age: Option<u32>,
    /// Cruce del líquido con su objetivo — LECTURA, aunque su estrategia se dispare por edad.
    pub liquid_crossing_month_index: Option<u32>,
    /// El mes efectivo otra vez, con el nombre del motor. Igual a `jubilacion_month_index` (R8);
    /// viaja porque es el nombre que usa el resto del contrato de fases.
    pub retirement_month_index: Option<u32>,
    /// Mes «coast» de ESTE miembro (dejar de aportar y llegar igual), en la rejilla común.
    /// `null` con cualquier estrategia que no sea `coast`, y también con `coast` cuando el plan
    /// no llega ni aportando siempre (entonces viaja `coast_not_reachable` en `warnings`).
    pub coast_fire_month_index: Option<u32>,
    /// `true` ⟺ **ni invirtiendo cada euro de sobrante** llega al objetivo en su edad (D17, el
    /// rojo). `null` = la pregunta no aplica a su estrategia (`asap`, `pension_bridge`, o una
    /// estrategia por edad degradada por falta de fecha de nacimiento): **nunca `false` para
    /// decir «no aplica»** — un `false` ahí se lee como «va bien».
    pub underfunded: Option<bool>,
    /// Aportación mensual mínima que le hace llegar a su edad objetivo, en euros. `null` con las
    /// estrategias por cruce. Con `underfunded: true` es su TECHO de búsqueda: «todo lo que
    /// tienes, y aun así no llega».
    #[serde(with = "rust_decimal::serde::str_option")]
    #[schema(value_type = Option<String>)]
    pub required_contribution_monthly: Option<Decimal>,
    /// Margen mensual de ESTE miembro (ver el campo homónimo de la respuesta para la base exacta
    /// de cada estrategia). `null` cuando su estrategia no publica margen.
    #[serde(with = "rust_decimal::serde::str_option")]
    #[schema(value_type = Option<String>)]
    pub disposable_monthly: Option<Decimal>,
    /// Mes de inicio de la media jornada (rejilla 0-based). `null` sin fase parcial en el perfil.
    pub partial_retirement_month_index: Option<u32>,
    /// Mes de inicio de la pensión con fecha (rejilla 0-based). `null` sin pensión con fecha.
    pub pension_start_month_index: Option<u32>,
    /// Mes en que la cartera de ESTE miembro se vacía **y eso le cuesta dinero**, en la rejilla
    /// común del hogar (#210) — misma definición de dos condiciones que el campo homónimo de la
    /// respuesta. El agregado publica el MÍNIMO; aquí se ve de quién es.
    pub assets_depleted_month_index: Option<u32>,
    /// Avisos de este miembro (p. ej. `birth_date_missing`).
    pub warnings: Vec<String>,
    /// **Horizonte PROPIO de este miembro en meses**, derivado de SU fecha de nacimiento y de SU
    /// `horizon_lifespan_age`. El agregado se corre al horizonte COMÚN `max(horizontes)`
    /// (`horizon_basis: "household_max_lifespan"`), así que este número puede ser MENOR que
    /// `months`: desde ahí, la curva de esta persona describe años que ella no declaró vivir.
    /// Sin el campo, el chart no puede distinguir «su plan llega hasta aquí» de «su plan se
    /// acaba aquí», y las dos cosas se dibujan igual.
    pub horizon_months: u32,
    /// **Serie de ESTE miembro** (D32), paralela a `points[]`: los mismos `month_index`, la
    /// misma decimación y los mismos f64 (excepción chart-only D4/I3). Es lo que dibuja la
    /// «línea fina por miembro» bajo la suma en grueso.
    ///
    /// Lleva `month_index` propio —y no solo dos arrays alineados por posición como
    /// `fire_target_series`— porque estas series se leen POR SEPARADO de `points`: un chart que
    /// pinta cuatro líneas de dos fuentes distintas no puede depender de que ambas se hayan
    /// decimado igual, y aquí el coste de decirlo son cuatro bytes por punto.
    pub series: Vec<MemberSeriesPoint>,
}

/// Un punto de la serie de un miembro del hogar. Deliberadamente **dos importes y no siete**: el
/// chart del hogar dibuja patrimonio y líquido por persona; el resto de columnas de
/// [`ProjectionPoint`] (aportado, deflactado, retirada, recorte, exceso) solo se leen del
/// agregado, y publicarlas por miembro multiplicaría el payload por 3,5 para responder algo que
/// nadie pregunta. La regla es la misma que la de `members[]`: el detalle por persona existe
/// para explicar la suma, no para duplicarla.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct MemberSeriesPoint {
    /// Mismo número de MES que `points[].month_index` (misma rejilla, misma decimación).
    pub month_index: u32,
    /// Patrimonio neto de este miembro en euros NOMINALES de ese mes.
    #[serde(serialize_with = "serialize_decimal_as_f64")]
    #[schema(value_type = f64)]
    pub net_worth: Decimal,
    /// Su patrimonio LÍQUIDO nominal — la línea que hay que comparar con SU objetivo, no con el
    /// del hogar (que no existe).
    #[serde(serialize_with = "serialize_decimal_as_f64")]
    #[schema(value_type = f64)]
    pub net_worth_liquid: Decimal,
}

/// Punto **interno** para los cálculos que recorren la serie mensual completa (milestones,
/// deflactado). Existe para que [`ProjectionPoint`] sea exclusivamente el tipo que se serializa:
/// mientras `points_full` era un `ProjectionPoint`, añadirle `net_worth_real` obligaba a rellenar
/// ese campo con un valor que nadie lee — y un campo con un valor inventado en una estructura de
/// dinero es exactamente cómo nacen las cifras plausibles y falsas de este repo.
#[derive(Debug, Clone, Copy)]
struct NwPoint {
    month_index: u32,
    net_worth: Decimal,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct AssetSeries {
    pub asset_id: Uuid,
    pub asset_name: String,
    /// Decimal values serializados como f64 (paralelo a `points`).
    pub values: Vec<f64>,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct ProjectionSeriesResponse {
    /// Vista efectivamente aplicada: `household` | `mine`. Eco de `?view` — ver
    /// `SummaryResponse::view` para el porqué. Aquí importa además porque el horizonte y la
    /// demografía (`viewer_birth_date`, `jubilacion_age`) son SIEMPRE del solicitante, también en
    /// `household`: sin este campo, dos respuestas con el mismo horizonte y distinto scope de
    /// patrimonio se leen igual.
    pub view: &'static str,
    pub points: Vec<ProjectionPoint>,
    pub months: u32,
    /// Años de horizonte efectivos (`months / 12`).
    pub horizon_years: u32,
    /// **De dónde sale `months`.** Enumeración cerrada, exactamente tres valores:
    ///
    /// - `lifespan_age` — derivado de una fecha de nacimiento: los meses que faltan hasta los
    ///   `horizon_lifespan_age` años del solicitante (o del primer miembro del hogar si él no
    ///   tiene DOB). Hasta 4.8.0 el literal era `lifespan_90` — un número congelado en un enum
    ///   publicado; con la edad configurable (#149) el valor viaja en el campo de al lado.
    /// - `fallback_no_demographics` — no hay ninguna fecha de nacimiento en el hogar: 30 años
    ///   (360 meses) por convención.
    /// - `months_override` — lo pidió el llamante con `?months=` / `months`, y se sirvió tal cual
    ///   (fuera de 12..=840 es un 400 `months_out_of_range`, nunca un clamp silencioso).
    ///
    /// Sin este campo, un horizonte de 360 meses es indistinguible de uno elegido a ciegas, y una
    /// respuesta de 360 meses «porque no sabemos la edad» se lee como una decisión del usuario.
    pub horizon_basis: String,
    /// La edad límite configurada (`fire_settings.horizon_lifespan_age`, 85..=105, default 90).
    /// Se ecoa SIEMPRE (también con `months_override` o fallback): es configuración, no derivada.
    /// El margen al final del horizonte NO tiene campo propio: es `points[último].net_worth`
    /// (el último punto viaja en ambas densidades) — «el plan NO llegó» ⟺
    /// `assets_depleted_month_index != null` o `uncovered_deficit_total > 0`.
    pub horizon_lifespan_age: u32,
    /// Patrimonio del último mes del horizonte en euros de HOY (paridad con
    /// `simulate_projection.final_net_worth_real`): el margen es una pregunta de poder
    /// adquisitivo, no de euros nominales de dentro de 40 años.
    #[serde(with = "rust_decimal::serde::str")]
    #[schema(value_type = String)]
    pub final_net_worth_real: Decimal,
    #[serde(with = "rust_decimal::serde::str")]
    #[schema(value_type = String)]
    pub starting_net_worth: Decimal,
    /// Ingresos regulares − gastos regulares (sin líneas derivadas de deuda).
    #[serde(with = "rust_decimal::serde::str")]
    #[schema(value_type = String)]
    pub monthly_delta_assumption: Decimal,
    pub model_note: String,
    /// Fecha civil del mes 0 de la serie (misma que `installation_naive_today`).
    pub anchor_date_ymd: String,
    /// Modo UI instalación: `dates` | `ages` (eje temporal en la app web).
    pub show_age_mode: String,
    /// `true` cuando `show_age_mode == ages` y hay fecha de nacimiento para el eje (la web no debe inferir esto sola).
    pub use_age_on_x_axis: bool,
    /// DOB usada para años cumplidos en el eje (perfil y/o personas del hogar).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub viewer_birth_date: Option<String>,
    /// Próximos hitos de patrimonio en euros **nominales**: umbrales 1/2.5/5*10^n, deduplicados por año.
    /// La web los usa cuando el toggle «Inflation Adjusted» está apagado.
    pub milestones: Vec<ProjectionMilestone>,
    /// Mismos umbrales que `milestones` pero cruzados sobre el patrimonio **deflactado** (euros de
    /// hoy): el hito de 1.000.000 € se alcanza cuando el patrimonio nominal vale 1.000.000 € *en
    /// poder adquisitivo de hoy*, no en euros del futuro. La web los usa cuando el toggle
    /// «Inflation Adjusted» está encendido. Vacío cuando la inflación es 0 (coincide con `milestones`).
    pub milestones_real: Vec<ProjectionMilestone>,
    /// **«El mes en que tu dinero empieza a trabajar más que tú»**: primer mes de la simulación en
    /// que el rendimiento del patrimonio (intereses/mercado del mes) supera el ahorro mensual base.
    ///
    /// El ahorro base es el neto recurrente del modelo — **sin** los Próximos (`planning_flows`) ni
    /// el plan de amortización de las deudas: los dos son flujos puntuales o decrecientes y harían
    /// que el cruce dependiera de un pago suelto. `null` = no cruza dentro del horizonte, ni
    /// siquiera en el último mes; no es «no calculado».
    ///
    /// Es un **número de MES** (misma base que `points[].month_index`), no una posición de array:
    /// con `density=hybrid` casi nunca coincide con un punto servido — la misma trampa que
    /// `jubilacion_month_index`, que por eso lleva su `jubilacion_series_position` al lado. Aquí no
    /// hay posición equivalente porque la cifra no se lee de la serie.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub compound_outpaces_true_savings_month_index: Option<u32>,
    /// **El mes en que la cartera se quedó sin nada Y eso costó dinero.** Número de MES (misma
    /// base que `points[].month_index`), nunca una posición de array. `null` explícito = no se
    /// agota dentro del horizonte — no «no calculado». (#119)
    ///
    /// Se publica solo si se cumplen las DOS condiciones, y en este orden (pase de correcciones
    /// de la revisión adversarial del motor):
    ///
    /// 1. Es el PRIMER mes cuya venta dejó lo vendible a cero, o no se pudo fundar. Lo decide la
    ///    VENTA, medida sobre los saldos DESPUÉS de vender — no el viejo predicado «venta bruta ≥
    ///    drenable», que comparaba dos cantidades calculadas por caminos distintos y que `Decimal`
    ///    y `f64` resolvían al revés justo en el aterrizaje exacto.
    /// 2. Desde ese mes en adelante, **alguna venta se quedó sin fundar**. Sin esta segunda
    ///    condición, un puente que se vacía EXACTAMENTE el mes en que entra una pensión que cubre
    ///    todo el gasto posterior —un plan perfecto— se publicaba como «cartera agotada» con
    ///    `uncovered_deficit_total = 0`. Ese aterrizaje exacto es hoy `null`.
    ///
    /// El corolario también se arregló: hasta 4.15.0 se publicaba `uncovered_deficit_total > 0`
    /// junto a «nunca agotado» por una cola de un ULP (47 casos por 3.000 medidos).
    ///
    /// **Breaking en 5.0.0 (#210): va en la rejilla 0-based como el resto de `*_month_index`.**
    /// Hasta 4.15.x se publicaba en la convención 1-based del bucle del motor, así que era el
    /// único índice de la respuesta desplazado un mes respecto de `jubilacion_month_index` y de
    /// `points[].month_index`. Un consumidor de 4.x que lo compare con otro índice o lo use para
    /// buscar un punto debe restarle 1 al migrar… o mejor, dejar de restar nada.
    pub assets_depleted_month_index: Option<u32>,
    /// Déficit acumulado NO cubierto al final del horizonte, en euros. `"0.0000"` significa cero
    /// euros descubiertos, no «no aplica». Ya se restaba de `net_worth`; ahora se declara. (#119)
    #[serde(with = "rust_decimal::serde::str")]
    #[schema(value_type = String)]
    pub uncovered_deficit_total: Decimal,
    /// Ahorro que ninguna regla de la cascada absorbió, acumulado (4.12.1): NO entra en
    /// `net_worth`, NO compone y NO cuenta como aportado — el modelo se niega a simular un euro
    /// sin destino declarado. `"0.0000"` = cero euros varados (caso normal: con activos vivos el
    /// sumidero es indestructible, #176).
    #[serde(with = "rust_decimal::serde::str")]
    #[schema(value_type = String)]
    pub unallocated_savings_total: Decimal,
    /// Por qué hay ahorro varado: `null` = no lo hay; `"no_assets"` = el scope no tiene activos
    /// (crea tu primer activo); `"no_sink"` = hay activos sin sumidero habilitado (residual).
    #[schema(value_type = Option<String>)]
    pub unallocated_savings_reason: Option<&'static str>,
    /// Pasivos cuya cuota no cubre el devengo: la deuda CRECE mes a mes (amortización negativa).
    /// Vacío = ninguno. Deliberadamente más estrecho que el `payment_does_not_reduce_principal`
    /// del calendario: un `interest_only` congela el principal y NO aparece aquí — esa
    /// distinción es el valor del campo. (#119)
    pub liabilities_negative_amortization: Vec<LiabilityNegativeAmortization>,
    /// Por qué NO hay objetivo FIRE (`manual_amount_missing` | `net_need_not_positive` |
    /// `swr_not_positive` — este último es también el caso `swr_pct = 0`). `null` ⟺ sí lo hay.
    /// Mismo campo y literales que `simulate_projection`: hasta #119 la superficie HTTP lo
    /// descartaba y la SPA no podía explicar un objetivo ausente. (#119)
    pub fire_target_absent_reason: Option<&'static str>,
    // Los cuatro campos de jubilación viajan como `null` EXPLÍCITO, sin `skip_serializing_if`.
    //
    // Con `skip` el campo simplemente desaparecía cuando no había cruce, así que un consumidor no
    // podía distinguir «no se alcanza el objetivo» de «esta versión no publica el campo» — y la
    // descripción de la tool lo prometía sin condiciones. `simulate_projection` ya devolvía `null`
    // explícito en `jubilacion_month_index`, de modo que las dos superficies se contradecían para
    // el mismo dato (auditoría MCP §8). Ahora las dos dicen `null`.
    /// Primer mes en que el patrimonio neto ≥ objetivo FIRE móvil del mes en curso. `null` si no hay objetivo o no se alcanza.
    pub jubilacion_month_index: Option<u32>,
    /// Fecha civil del cruce (`YYYY-MM-DD`) = `anchor_date_ymd` + `jubilacion_month_index` meses,
    /// conservando el día del ancla con recorte a fin de mes. `null` ⟺ no hay cruce.
    pub jubilacion_date_ymd: Option<String>,
    /// Años cumplidos en `jubilacion_date_ymd`. `null` si no hay cruce o no hay fecha de
    /// nacimiento resuelta (independiente de `show_age_mode`).
    pub jubilacion_age: Option<u32>,
    /// Objetivo FIRE base **en euros de HOY** (gross-up de impuestos aplicado), **sin el término
    /// de deuda** (que viaja aparte en `fire_target_debt_component`). Sirve como referencia y como
    /// anclaje del target móvil. `null` cuando no hay configuración FIRE válida.
    ///
    /// **5.0.0 WP5-2b: es la base del objetivo del PLAN**, `T(0) − deuda(0)`, no la de la
    /// perpetuidad. Sin pensión con fecha las dos expresiones son la misma cifra por construcción
    /// (`fire_target_at_month_index` = base + deuda) y ningún hogar de 4.15.x ve moverse este
    /// número; con base PUENTE, la de antes contradecía `fire_target_series[0]`, que es la línea
    /// que el chart pinta.
    pub jubilacion_target_net_worth: Option<String>,
    /// Término FINITO de deuda del objetivo A DÍA DE HOY (4.8.0, #142): Σ de cuotas que quedan
    /// por pagar de todos los planes + saldos residuales, en euros nominales. El objetivo de
    /// cada mes es `base·(1+π)^(k/12) + término(k)`, con el término decreciente (el objetivo ya
    /// NO es monótono). `"0.0000"` = sin deuda; `null` = no hay objetivo. La vista previa del
    /// formulario debe SUMARLO a su base recalculada — no puede derivarlo en cliente (necesita
    /// el calendario completo de cada plan).
    #[serde(with = "rust_decimal::serde::str_option")]
    #[schema(value_type = Option<String>)]
    pub fire_target_debt_component: Option<Decimal>,
    /// **Posición** (índice de array, base 0) dentro de `points` / `fire_target_series` /
    /// `asset_series[].values` que corresponde al mes de jubilación. `null` ⟺ no hay cruce.
    ///
    /// Existe porque `jubilacion_month_index` **no indexa nada**: es un número de MES, y con
    /// `density=hybrid` (la que fuerza la tool MCP `get_projection`) los arrays llevan ~42 puntos
    /// para 361 meses. Indexar con el mes daba basura o se salía del array; caer en `[0]`
    /// presentaba el objetivo de hoy como si fuera el de dentro de décadas.
    ///
    /// **Convención: el punto servido inmediatamente ANTERIOR o igual** — la última posición `p`
    /// con `points[p].month_index <= jubilacion_month_index`. Invariantes:
    /// - `points[p].month_index <= jubilacion_month_index` **siempre**, con igualdad ⟺ el mes del
    ///   cruce es un punto servido (siempre con `density=monthly`).
    /// - `p+1 == points.len()` o `points[p+1].month_index > jubilacion_month_index`: el cruce cae
    ///   en el segmento `[p, p+1)`, que es donde un chart pinta el marcador.
    ///
    /// Se eligió «anterior» y no «siguiente» porque es la semántica estándar de «en qué bucket cae
    /// este mes», porque hace comprobable la exactitud (`month_index` del punto vs el del cruce) y
    /// porque su error es **conservador**: leer el patrimonio de ese punto lo infravalora en vez de
    /// inflarlo. Para las cifras exactas del mes del cruce no uses la serie: usa
    /// `jubilacion_target_net_worth_nominal`, que va calculado, no interpolado.
    pub jubilacion_series_position: Option<u32>,
    /// Objetivo FIRE **del mes del cruce**, en euros **NOMINALES** de ese mes. `null` ⟺ no hay cruce.
    ///
    /// Es el número que faltaba: `jubilacion_target_net_worth` está en euros de HOY, y el objetivo
    /// crece con la inflación, así que a décadas vista los dos difieren por más de 2×. Se evalúa
    /// **exacto** con el evaluador CONSCIENTE DEL PLAN del motor (`PlanFireTarget`, el mismo que
    /// produce `fire_target_series` y contra el que el bucle decide el cruce) sobre el mes de la
    /// jubilación — no se interpola entre puntos de la serie ni se lee de `fire_target_series`,
    /// que con `density=hybrid` puede no tener ese mes.
    pub jubilacion_target_net_worth_nominal: Option<String>,
    /// Serie mensual del target FIRE ajustado por inflación, paralela a `points`. Cada valor =
    /// `target_base × (1 + inflación%)^(month_index/12)`. Vacío cuando no hay FIRE configurado.
    /// Serializado como f64 (ver `serialize_decimal_as_f64`).
    pub fire_target_series: Vec<f64>,
    /// Valor de cada activo mes a mes (paralelo a `points`). Un elemento por activo, en el mismo orden que la consulta de activos.
    pub asset_series: Vec<AssetSeries>,
    /// Densidad de los puntos serializados: `monthly` (default en HTTP) o `hybrid` (la que fuerza
    /// la tool MCP `get_projection`). Es un **eco de la decisión del servidor**, no un dato del
    /// dominio: con `hybrid` los puntos son mensuales hasta el mes 12 y anuales a partir del 24,
    /// más siempre el último mes del horizonte, así que entre dos puntos consecutivos pueden caber
    /// doce meses. Cuando ese hueco esconda un salto, la explicación está en `events` — no en
    /// pedir más puntos.
    pub density: String,
    /// Saltos puntuales de la curva: los Próximos **con fecha** que caen dentro del horizonte, con
    /// su mes, su rótulo, su importe y su dirección. Vacío si no hay ninguno. Ver
    /// [`ProjectionEvent`] para qué entra y qué no.
    pub events: Vec<ProjectionEvent>,
    /// `true` ⟺ había más de `PROJECTION_EVENTS_MAX` (100) eventos y `events` está recortado.
    /// Los que faltan son los de los meses más lejanos (el orden es cronológico).
    pub events_truncated: bool,
    /// Fuente del ahorro **efectiva** que produjo `monthly_delta_assumption` (tras el fallback: en
    /// modo `transactions_avg` / `budget_income_real_expense` sin meses reales cae a `budget`).
    /// Mismo naming y semántica que el campo homónimo de `/v1/summary`.
    pub savings_source: SavingsSource,
    /// Meses reales usados por el promedio cuando `savings_source` deriva de transacciones; `0` en
    /// modo `budget` (configurado o por fallback).
    /// Procedencia del lado INGRESO del ahorro efectivo (ventana, meses usados, rango real).
    pub savings_income_basis: SavingsAvgBasis,
    /// Procedencia del lado GASTO.
    pub savings_expense_basis: SavingsAvgBasis,
    /// Inflación anual (%) con la que se calculó `points[].net_worth_real` y `milestones_real`.
    /// Es la asunción de la instalación, clampada a ≥ 0. **Sin este campo `net_worth_real` sería
    /// una cifra sin base declarada**: la misma disciplina que `basis` en `/v1/budget` y
    /// `/v1/summary` (declarar la base, no renombrar el campo).
    #[serde(with = "rust_decimal::serde::str")]
    #[schema(value_type = String)]
    pub deflation_annual_inflation_percent: Decimal,
    /// Qué gobernó la fiscalidad del DRENAJE en esta simulación (#178): `cost_basis` = todos los
    /// activos declaran coste y la `g` de cada venta se deriva de su base viva; `declared_ratio`
    /// = ninguno lo declara y rige el escalar `taxable_gain_ratio`; `mixed` = conviven. El
    /// OBJETIVO y el umbral de Autonomía usan siempre el escalar (perpetuidades — el reparto de
    /// regímenes está declarado en el contrato financiero §2.4).
    pub drawdown_gain_basis: &'static str,
    /// `g₀` de la cartera de HOY — `Σ max(0, v−coste)/Σ v` sobre los activos CON coste declarado
    /// — **solo informativo** («tu cartera es hoy un 20 % ganancia»); no es la entrada de nada.
    /// `null` cuando ningún activo declara coste.
    #[serde(with = "rust_decimal::serde::str_option")]
    #[schema(value_type = Option<String>)]
    pub taxable_gain_ratio_today: Option<Decimal>,

    // -----------------------------------------------------------------------------------------
    // 5.0.0 — estrategia, fases y agregado del hogar (§B.8, §C, §D del plan de #207)
    // -----------------------------------------------------------------------------------------
    /// Estrategia de jubilación con la que se simuló: `asap` | `retire_at_age` | `coast` |
    /// `partial` | `pension_bridge`. **`null` en `view=household`**: el agregado suma N
    /// simulaciones y cada miembro corre la suya — la de cada uno viaja en `members[]`.
    ///
    /// Se ecoa por el mismo motivo que `view`: dos respuestas con las mismas cifras y distinta
    /// estrategia se leen igual, y la estrategia decide QUÉ significa `jubilacion_month_index`
    /// (un cruce alcanzado o una edad impuesta).
    pub strategy: Option<String>,
    /// Qué DISPARÓ la jubilación: `liquid_crossing` (el patrimonio alcanzó el objetivo) o
    /// `target_age` (la edad manda, llegue o no el capital — D17). `null` en `household`.
    ///
    /// Con `target_age` el objetivo FIRE **sigue publicándose** (`fire_target_series`,
    /// `jubilacion_target_net_worth`) pero ya no decide nada: es la línea de referencia contra la
    /// que leer `liquid_crossing_month_index`. Sin este campo, un consumidor no puede distinguir
    /// «se jubila porque llegó» de «se jubila porque cumplió años y no llegó».
    #[schema(value_type = Option<String>)]
    pub retirement_trigger: Option<&'static str>,
    /// Mes EFECTIVO de jubilación del motor (§B.8), en la rejilla de `points[].month_index`. Es
    /// **el mismo valor** que `jubilacion_month_index` (R8): viaja con los dos nombres porque
    /// `jubilacion_*` es el contrato publicado desde 1.x y `retirement_month_index` es el nombre
    /// del motor y del resto de las lecturas de fase. `null` en `household`.
    pub retirement_month_index: Option<u32>,
    /// **Posición** (índice de array) del mes de jubilación dentro de `points`. Gemelo exacto de
    /// `jubilacion_series_position` y con su misma convención (el último punto servido cuyo
    /// `month_index` no pasa del mes de jubilación); existe para que quien lea los campos con
    /// nombre de motor no tenga que saltar a los `jubilacion_*` para poder indexar.
    pub retirement_series_position: Option<u32>,
    /// **Cruce del líquido con el objetivo FIRE** — LECTURA PURA desde 5.0.0. Con `asap` coincide
    /// con `retirement_month_index` (es lo que dispara); con una estrategia por edad es el mes en
    /// que el capital habría bastado, que puede ser posterior (te jubilas antes de llegar) o
    /// anterior (podrías haberte jubilado antes).
    ///
    /// `null` + `liquid_crossing_absent_reason` cuando no hay objetivo contra el que cruzar o la
    /// vista es agregada; `null` **sin** razón = hay objetivo y no se cruza dentro del horizonte.
    pub liquid_crossing_month_index: Option<u32>,
    /// `household_aggregate` | `no_fire_target`. `null` ⟺ el cruce es una pregunta con sentido en
    /// esta respuesta (se haya alcanzado o no).
    #[schema(value_type = Option<String>)]
    pub liquid_crossing_absent_reason: Option<&'static str>,
    /// Por qué los `jubilacion_*` y `retirement_*` están vacíos POR CONSTRUCCIÓN:
    /// `household_aggregate` (la vista suma N planes y no tiene uno) | `no_retirement_trigger`
    /// (ni objetivo FIRE válido ni edad objetivo: esta simulación no se jubila nunca).
    /// `null` ⟺ hay un trigger; entonces un `jubilacion_month_index` nulo significa «no se
    /// alcanza dentro del horizonte», que es un resultado, no un hueco.
    #[schema(value_type = Option<String>)]
    pub jubilacion_absent_reason: Option<&'static str>,
    /// Por qué falta el marcador «tu dinero trabaja más que tú»: `household_aggregate` (el
    /// marcador es una propiedad del ahorro de UNA persona; sumar N cascadas no produce uno).
    /// `null` ⟺ la pregunta tiene sentido aquí.
    #[schema(value_type = Option<String>)]
    pub compound_outpaces_true_savings_absent_reason: Option<&'static str>,
    /// Fases atravesadas y el mes de la rejilla en que empieza cada una. Siempre arranca con
    /// `accumulating` en el mes 0. Vacío en `household`.
    pub phase_transitions: Vec<PhaseTransition>,
    /// Primer mes con pensión pública con fecha. `null` sin pensión con fecha (la pensión sin fecha de hoy
    /// viaja dentro del ingreso de jubilación y no tiene mes propio).
    pub pension_start_month_index: Option<u32>,
    /// Primer mes de media jornada. `null` sin fase parcial en el perfil.
    pub partial_retirement_month_index: Option<u32>,
    /// Avisos de esta simulación. Literales cerrados: `birth_date_missing` /
    /// `target_retirement_age_missing` = una estrategia por edad degradó a `asap` porque le
    /// faltaba el dato (nunca un 500 en una lectura); `bridge_discount_no_liquid_assets` /
    /// `bridge_discount_clamped` = el descuento del puente cayó a 0 (sin líquido del que sacarlo,
    /// o porque salía NEGATIVO y un descuento negativo encarece el puente en vez de abaratarlo);
    /// `retire_at_age_underfunded`, `coast_not_reachable`, `partial_phase_capital_shrinking` =
    /// los del motor y del solve. Vacío = nada que advertir. En `household` va vacío y los avisos
    /// viajan por miembro en `members[]`.
    pub warnings: Vec<String>,
    /// **Un elemento por miembro del hogar, y solo en `view=household`** (D9): el agregado es la
    /// SUMA de N simulaciones independientes, así que esto es lo que dice de quién es cada
    /// marcador. Vacío en `view=mine` — ahí la respuesta entera es de una sola persona.
    pub members: Vec<HouseholdMemberProjection>,

    // -----------------------------------------------------------------------------------------
    // 5.0.0 WP5-2b — pensión con fecha, puente, media jornada y SOLVES (§B.3, §B.7 de #207).
    //
    // Todo este bloque es `null`/vacío en `view=household`: el agregado suma N planes y ninguno
    // de estos números tiene una versión «del hogar» (¿el margen de quién?, ¿el capital
    // necesario para la edad de quién?). Lo que sí viaja por miembro está en `members[]`.
    // -----------------------------------------------------------------------------------------
    /// **Tasa ANUAL (%) con la que el puente descontó sus flujos** (D7), ya resuelta desde
    /// `bridge_discount_basis`: rentabilidad esperada ponderada por valor de los activos LÍQUIDOS
    /// de hoy (`None` en la tasa de un activo cuenta como 0 %), el propio `swr_pct`, o `0` sin
    /// descuento. `null` ⟺ el objetivo no es puente — ahí no hay tasa que aplicar, y un `0` se
    /// leería como «puente sin descontar» en vez de «no hay puente».
    #[serde(with = "rust_decimal::serde::str_option")]
    #[schema(value_type = Option<String>)]
    pub bridge_discount_annual_pct: Option<Decimal>,
    /// **Tasa de retirada efectiva del puente**, en % ANUAL: `100 · 12·need_full_m(R−1)/L(R−1)`
    /// en el mes efectivo de jubilación. Responde a lo que la perpetuidad esconde — mientras la
    /// pensión no llega hay que sacar de la cartera el gasto ENTERO, y eso puede estar muy por
    /// encima del SWR, legítimamente, porque dura pocos años.
    ///
    /// **Es un PORCENTAJE** (`6.5` = 6,5 %), no una fracción. `null` sin pensión con fecha, sin
    /// base puente, sin objetivo, sin jubilación dentro del horizonte o con líquido no positivo
    /// en ese mes: en ninguno de esos casos hay una tasa que medir.
    #[serde(with = "rust_decimal::serde::str_option")]
    #[schema(value_type = Option<String>)]
    pub bridge_effective_withdrawal_pct: Option<Decimal>,
    /// **Qué fracción del gasto cubre la pensión el mes en que empieza**: `P_m(P)/(E·f(P))`.
    /// **Es una FRACCIÓN** (`0.6` = 60 %), no un porcentaje — la regla de oro de los sufijos.
    /// Hace explícitos los dos escenarios de D15 sin asumir ninguno: `≥ 1` = la pensión cubre el
    /// gasto entero y el término perpetuo del objetivo es 0 exacto. `null` sin pensión con fecha,
    /// sin objetivo o con gasto no positivo en ese mes.
    #[serde(with = "rust_decimal::serde::str_option")]
    #[schema(value_type = Option<String>)]
    pub pension_coverage_ratio: Option<Decimal>,
    /// **Capital que sostendría a perpetuidad el HUECO de la media jornada**:
    /// `gross_up(12·gap_m(X))/SWR`, con `gap_m(X) = max(0, gasto_base·f(X) − ingreso_parcial −
    /// pensión·fracción)`. Informativo: no dispara nada y no es el objetivo de la simulación.
    /// `"0.0000"` = la media jornada se paga sola; `null` = no hay fase parcial o no hay objetivo.
    #[serde(with = "rust_decimal::serde::str_option")]
    #[schema(value_type = Option<String>)]
    pub partial_gap_target: Option<Decimal>,
    /// `true` ⟺ hubo fase parcial y el patrimonio LÍQUIDO no bajó ni un mes durante ella.
    /// `false` = hubo y menguó (y viaja `partial_phase_capital_shrinking` en `warnings`).
    /// **`null` = no hubo fase parcial**: publicar `false` ahí diría «tu media jornada se come el
    /// capital» de un hogar que no ha declarado ninguna.
    pub partial_phase_capital_growing: Option<bool>,
    /// **Aportación mensual mínima para llegar al objetivo en la edad objetivo** (§B.7), en euros
    /// nominales. Es un TECHO sobre lo que la cascada invierte cada mes, no un importe que se
    /// aporte pase lo que pase: en un mes con menos sobrante, se aporta el sobrante (R5).
    ///
    /// `null` con las estrategias por cruce (`asap`, `pension_bridge`) y con una estrategia por
    /// edad degradada por falta de fecha de nacimiento — ahí no hay `R` contra el que resolver.
    /// Con `underfunded: true` esta cifra ES `required_contribution_search_ceiling`.
    #[serde(with = "rust_decimal::serde::str_option")]
    #[schema(value_type = Option<String>)]
    pub required_contribution_monthly: Option<Decimal>,
    /// **Techo de la búsqueda** del solve: el máximo sobrante mensual del horizonte, con el
    /// sobrante del mes 1 como suelo. Por encima de él, poner techo a la cascada ES no ponerlo.
    /// Se publica para que `required_contribution_monthly` tenga denominador — «cuánto de mi
    /// margen se lleva el plan» — sin obligar a deducirlo. `null` cuando no hay solve.
    #[serde(with = "rust_decimal::serde::str_option")]
    #[schema(value_type = Option<String>)]
    pub required_contribution_search_ceiling: Option<Decimal>,
    /// **El rojo de D17**: `true` ⟺ ni invirtiendo cada euro de sobrante se alcanza el objetivo
    /// en la edad elegida. No es un error — la simulación existe, se jubila igual y se publica
    /// entera; lo que dice es que se jubila POR DEBAJO del objetivo. Viaja además como
    /// `retire_at_age_underfunded` en `warnings`.
    ///
    /// **`null` = la pregunta no aplica** (estrategia por cruce, o por edad sin fecha de
    /// nacimiento). Nunca `false` para decir «no aplica»: ahí un `false` se lee como «va bien».
    pub underfunded: Option<bool>,
    /// **Serie del capital NECESARIO**, paralela a `points[]` (f64, misma decimación): el
    /// patrimonio líquido de la simulación que aporta exactamente `required_contribution_monthly`
    /// cada mes. Es una serie SIMULADA de verdad —una ejecución más del motor—, no el objetivo
    /// descontado a una tasa escalar: ese número sería plausible y ninguna simulación lo produce
    /// (hallazgo M8). Vacía cuando no hay solve.
    pub required_capital_path: Vec<f64>,
    /// **Margen mensual disponible** (D16/D31), en euros. La base depende de la estrategia y por
    /// eso va declarada aquí:
    ///
    /// - `retire_at_age` / `partial` con edad: `techo_de_búsqueda − aportación_necesaria`,
    ///   clampado a ≥ 0 — lo que sobra del máximo sobrante mensual una vez cubierto el plan.
    /// - `coast`: el sobrante del mes 1 (R5) **desde el mes coast**, y `"0.0000"` antes. Este
    ///   campo es el valor de HOY, así que solo es distinto de cero cuando el mes coast ya llegó.
    ///
    /// `null` = la estrategia no publica margen (`asap`, `pension_bridge`) o no hubo solve.
    #[serde(with = "rust_decimal::serde::str_option")]
    #[schema(value_type = Option<String>)]
    pub disposable_monthly: Option<Decimal>,
    /// **Serie del capital SOBRANTE**, paralela a `points[]` (f64): `líquido(k) −
    /// capital_necesario(k)` con `retire_at_age`/`partial`, y `líquido(k) − coast_path(k)` desde
    /// el mes coast (0 antes) con `coast`. Es el colchón por encima de lo que el plan exige.
    /// Vacía cuando no hay solve.
    pub disposable_capital: Vec<f64>,
    /// El último valor de `disposable_capital` **en el mes efectivo de jubilación**, en euros
    /// NOMINALES de ese mes. `null` sin solve o sin jubilación dentro del horizonte.
    #[serde(with = "rust_decimal::serde::str_option")]
    #[schema(value_type = Option<String>)]
    pub disposable_capital_at_retirement: Option<Decimal>,
    /// El mismo capital sobrante del mes de jubilación llevado a **euros de HOY**, con el mismo
    /// deflactor que `points[].net_worth_real` (`deflation_annual_inflation_percent`). Es la
    /// mitad legible del tile «Margen disponible»: el nominal de dentro de 25 años impresiona y
    /// no dice nada. `null` en los mismos casos que su gemelo nominal.
    #[serde(with = "rust_decimal::serde::str_option")]
    #[schema(value_type = Option<String>)]
    pub disposable_capital_today: Option<Decimal>,
    /// **Mes «coast»** (§B.7): el primero a partir del cual se puede dejar de aportar y aun así
    /// alcanzar el objetivo en la edad elegida. Número de MES de la rejilla, como el resto.
    /// `null` con cualquier estrategia que no sea `coast`; con `coast`, `null` significa que el
    /// plan no llega ni aportando siempre — y entonces viaja `coast_not_reachable` en `warnings`.
    pub coast_fire_month_index: Option<u32>,
    /// **El número coast**: el patrimonio LÍQUIDO con el que se ENTRA en el mes coast (el cierre
    /// del mes anterior), en euros nominales. Desde ahí, sin un euro más invertido, la cartera
    /// sola llega al objetivo. Es el valor de la serie simulada, no un descuento cerrado.
    #[serde(with = "rust_decimal::serde::str_option")]
    #[schema(value_type = Option<String>)]
    pub coast_number: Option<Decimal>,
    /// **Serie «si dejas de aportar en el mes coast»**, paralela a `points[]` (f64) — la línea
    /// discontinua de D29. Cuando el coast no es alcanzable, es la serie de la ejecución que
    /// aporta TODOS los meses: la mejor que el plan da. Vacía sin estrategia `coast`.
    pub coast_path: Vec<f64>,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct ProjectionMilestone {
    #[serde(with = "rust_decimal::serde::str")]
    #[schema(value_type = String)]
    pub target: Decimal,
    pub reached_month_index: u32,
    pub reached_date_ymd: String,
}

/// Un pasivo en amortización NEGATIVA: su cuota no cubre el devengo y la deuda crece mes a mes
/// (#119). El predicado es «algún mes con `principal_repaid < 0`» del calendario canónico —
/// deliberadamente más estrecho que `payment_does_not_reduce_principal`, que también atrapa a
/// `interest_only` (principal congelado, deuda que NO crece).
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct LiabilityNegativeAmortization {
    #[schema(value_type = String, format = "uuid")]
    pub liability_id: Uuid,
    pub label: String,
    /// Saldo de partida (hoy), en euros.
    #[serde(with = "rust_decimal::serde::str")]
    #[schema(value_type = String)]
    pub opening_principal: Decimal,
    /// Saldo al final del horizonte simulado — mayor que el de partida: eso ES el hallazgo.
    #[serde(with = "rust_decimal::serde::str")]
    #[schema(value_type = String)]
    pub final_principal: Decimal,
    /// Meses simulados para `final_principal` (el horizonte de esta respuesta).
    pub horizon_months: u32,
}

#[derive(Debug, FromRow)]
struct AssetEngineRow {
    id: Uuid,
    name: String,
    current_value: Decimal,
    purchase_price: Option<Decimal>,
    is_liquid: bool,
    expected_annual_return_percent: Option<Decimal>,
    /// Desviación típica ANUAL de los retornos, en puntos porcentuales (`[0, 100]`, `NULL` =
    /// activo determinista). **El camino `Decimal` la ignora por completo**: entra en el
    /// ensamblado únicamente para viajar, alineada con `input.assets`, hasta Monte Carlo
    /// (`projection_bands.rs`). Cargarla aquí y no en una segunda query es lo que garantiza la
    /// alineación — el orden de los activos es una propiedad de ESTE `SELECT`.
    annual_volatility_percent: Option<Decimal>,
}

#[derive(Debug, FromRow)]
struct AllocationRuleEngineRow {
    id: Uuid,
    target_asset_id: Uuid,
    kind: String,
    amount: Option<Decimal>,
    cap_kind: Option<String>,
    cap_value: Option<Decimal>,
}

#[derive(Debug, FromRow)]
struct LiabEngineRow {
    /// Necesario desde 4.4.0 para que `simulate_projection` pueda apuntar sus overrides a UN
    /// pasivo concreto (mismo patrón que `asset_id_name` para las rentabilidades por activo).
    id: Uuid,
    label: String,
    principal: Decimal,
    payment_amount: Option<Decimal>,
    payment_frequency: Option<String>,
    payment_end_date: Option<NaiveDate>,
    /// TIN nominal anual en puntos porcentuales. `None` (o ≤ 0) ⇒ el engine no devenga.
    apr_percent: Option<Decimal>,
    /// Literal de la columna (`fixed_payments` | `french` | `interest_only` | `revolving`),
    /// acotado por el CHECK de la migración `20260825120000_liabilities_repayment_model`.
    repayment_model: String,
    /// Cuota mínima revolving (Ola 3/#144): % del saldo y suelo en €. Solo `revolving`.
    min_payment_pct: Option<Decimal>,
    min_payment_eur: Option<Decimal>,
}

#[derive(Debug, Clone, FromRow)]
pub(crate) struct PlanningFlowProjRow {
    pub scope: String,
    pub expected_amount: Decimal,
    /// `one_off` | `per_month` (#148). Con `per_month`, `expected_amount` son €/MES durante la
    /// ventana y el flujo carga MES COMPLETO (sin prorrateo por días): todos los términos
    /// recurrentes del modelo son magnitudes de mes entero, y prorratear solo este devolvería el
    /// término dependiente del día que #126 retiró.
    pub amount_basis: String,
    pub due_date: Option<NaiveDate>,
    pub window_start_date: Option<NaiveDate>,
    pub window_end_date: Option<NaiveDate>,
    /// Rótulo del flujo. No entra en ninguna cuenta: existe para que
    /// `ProjectionSeriesResponse::events` pueda **nombrar** el salto que produce.
    pub title: String,
}

impl PlanningFlowProjRow {
    /// Constructor de conveniencia para el caso histórico (puntual, sin ventana) — lo usan los
    /// tests y el flujo sintético del what-if.
    pub(crate) fn one_off(
        scope: &str,
        expected_amount: Decimal,
        due_date: Option<NaiveDate>,
        title: &str,
    ) -> Self {
        Self {
            scope: scope.into(),
            expected_amount,
            amount_basis: "one_off".into(),
            due_date,
            window_start_date: None,
            window_end_date: None,
            title: title.into(),
        }
    }
}

/// Días civiles: reparto equitativo del total entre el día 1 del mes ancla y +89 días
/// (90 días inclusive). Anclado al mes civil desde #126: el vector resultante es idéntico
/// se pregunte el día que se pregunte dentro del mismo mes.
const PLANNING_UNDATED_SPREAD_DAYS: i64 = 90;
const PROJECTION_MILESTONE_MINIMUM: i64 = 1_000;
const PROJECTION_MILESTONE_SEARCH_COUNT: usize = 64;
const PROJECTION_MILESTONE_LIMIT: usize = 3;

fn proj_month_first(d: NaiveDate) -> NaiveDate {
    NaiveDate::from_ymd_opt(d.year(), d.month(), 1).unwrap_or(d)
}

fn proj_add_months(d: NaiveDate, n: u32) -> NaiveDate {
    d.checked_add_months(Months::new(n)).unwrap_or(d)
}

fn proj_month_last(m_first: NaiveDate) -> NaiveDate {
    proj_add_months(m_first, 1)
        .pred_opt()
        .unwrap_or(m_first)
}

fn overlap_inclusive_days(
    a_start: NaiveDate,
    a_end: NaiveDate,
    b_start: NaiveDate,
    b_end: NaiveDate,
) -> i64 {
    let start = a_start.max(b_start);
    let end = a_end.min(b_end);
    if start > end {
        return 0;
    }
    end.signed_duration_since(start).num_days() + 1
}

/// Un salto puntual de la serie de proyección: el mes en que un **Próximo con fecha**
/// (`planning_flows.due_date`) entra en la caja del modelo.
///
/// Existe porque `density=hybrid` —la que sirve la tool MCP— emite un punto por AÑO a partir del
/// mes 24: entre dos puntos consecutivos caben doce meses, y una caída de decenas de miles de euros
/// entre ellos no tiene en la respuesta **nada** que la explique. Subir la densidad no lo arregla:
/// `density=monthly` multiplica el payload por ~5 y sigue sin decir POR QUÉ cayó, solo dónde. Un
/// evento son ~90 bytes y contesta la pregunta entera.
///
/// **Solo flujos CON fecha.** Los Próximos sin `due_date` se reparten a partes iguales sobre 90
/// días naturales (`PLANNING_UNDATED_SPREAD_DAYS`), así que por construcción no producen un salto:
/// listarlos aquí llamaría «evento» a una rampa. Tampoco entran los pasivos ni las partidas de
/// presupuesto con fecha de fin: esos cambian la PENDIENTE de la curva, no producen un escalón.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct ProjectionEvent {
    /// Mes de la serie en que impacta, **misma base que `points[].month_index`** (0 = mes ancla).
    /// Es un número de MES, no una posición de array: con `density=hybrid` el mes del evento
    /// normalmente NO es un punto servido — ése es justamente el motivo de que este array exista.
    pub month_index: u32,
    /// `due_date` del flujo, `YYYY-MM-DD`.
    pub date_ymd: String,
    /// Rótulo que el usuario le puso al Próximo.
    pub title: String,
    /// Importe como **magnitud ≥ 0**; el signo lo lleva `direction`. Mismo criterio que la
    /// comparativa de movimientos, y evita que un `-0` o un doble signo cambien la lectura.
    #[serde(with = "rust_decimal::serde::str")]
    #[schema(value_type = String)]
    pub amount: Decimal,
    /// `inflow` (categoría de scope `income`) | `outflow` (scope `expense`).
    pub direction: &'static str,
    /// `true` = el Próximo venció antes del mes ancla y se carga íntegro en el mes 0 (#126: la
    /// deuda vencida se arrastra, no se borra). El `date_ymd` sigue siendo su fecha REAL
    /// (pasada): el mes que se señala y la fecha que se muestra dejan de coincidir a propósito,
    /// y este flag es lo que lo declara.
    pub overdue: bool,
}

/// Tope de eventos publicados. Los Próximos son pocos por naturaleza, pero nada en el modelo de
/// datos lo garantiza y este array viaja en el endpoint más caliente de la app.
const PROJECTION_EVENTS_MAX: usize = 100;

/// Eventos de la serie a partir de los flujos ya cargados. **Comparte la regla de mapeo
/// fecha→mes con `planning_monthly_cash_adjustments_from_flows`** (mismo `anchor_month_first`,
/// misma carga de lo vencido en el mes ancla con `overdue: true`, misma búsqueda del mes que
/// contiene la fecha): si divergieran, la respuesta señalaría un mes distinto de aquel en el que
/// la curva salta.
///
/// Devuelve `(eventos, truncados)` ordenados por `month_index` ASC y, dentro del mes, por importe
/// descendente — el que más mueve la curva primero.
fn projection_events_from_flows(
    ref_date: NaiveDate,
    horizon_months: u32,
    flows: &[PlanningFlowProjRow],
) -> (Vec<ProjectionEvent>, bool) {
    let anchor_month_first = proj_month_first(ref_date);
    let mut out: Vec<ProjectionEvent> = Vec::new();

    for flow in flows {
        let direction = match flow.scope.as_str() {
            "income" => "inflow",
            "expense" => "outflow",
            _ => continue,
        };
        // #148: un `per_month` es una rampa recurrente, no un escalón — misma razón por la que
        // los sin-fecha nunca entraron aquí.
        if flow.amount_basis == "per_month" {
            continue;
        }
        // Sin fecha no hay escalón: se reparte sobre 90 días (ver el doc de `ProjectionEvent`).
        let Some(due) = flow.due_date else { continue };
        if due < anchor_month_first {
            // #126: el vencido carga íntegro en el mes ancla, declarado — sin cota de
            // antigüedad, igual que el instrumento real arrastra la deuda en vez de borrarla.
            if horizon_months > 0 {
                out.push(ProjectionEvent {
                    month_index: 0,
                    date_ymd: due.format("%Y-%m-%d").to_string(),
                    title: flow.title.clone(),
                    amount: money_out(flow.expected_amount.abs()),
                    direction,
                    overdue: true,
                });
            }
            continue;
        }
        for idx in 0..horizon_months {
            let m_first = proj_add_months(anchor_month_first, idx);
            let m_last = proj_month_last(m_first);
            if due >= m_first && due <= m_last {
                out.push(ProjectionEvent {
                    month_index: idx,
                    date_ymd: due.format("%Y-%m-%d").to_string(),
                    title: flow.title.clone(),
                    amount: money_out(flow.expected_amount.abs()),
                    direction,
                    overdue: false,
                });
                break;
            }
        }
    }

    out.sort_by(|a, b| {
        a.month_index
            .cmp(&b.month_index)
            .then_with(|| b.amount.cmp(&a.amount))
            .then_with(|| a.title.cmp(&b.title))
    });
    let truncated = out.len() > PROJECTION_EVENTS_MAX;
    out.truncate(PROJECTION_EVENTS_MAX);
    (out, truncated)
}

fn planning_monthly_cash_adjustments_from_flows(
    ref_date: NaiveDate,
    horizon_months: u32,
    flows: &[PlanningFlowProjRow],
) -> Vec<Decimal> {
    let mut adj = vec![Decimal::ZERO; horizon_months as usize];
    let anchor_month_first = proj_month_first(ref_date);

    // #126: la ventana de la rampa sin fecha se ancla al día 1 del mes civil, no al día de la
    // consulta — el mes civil ES la rejilla del motor, y este era el único término de caja
    // prorrateado por el día en que se pregunta.
    let undated_win_first = anchor_month_first;
    let undated_win_last = anchor_month_first
        .checked_add_signed(Duration::days(PLANNING_UNDATED_SPREAD_DAYS - 1))
        .unwrap_or(anchor_month_first);

    for flow in flows {
        let signed = match flow.scope.as_str() {
            "income" => flow.expected_amount,
            "expense" => -flow.expected_amount,
            _ => continue,
        };

        // #148: un flujo `per_month` carga su €/mes en cada mes civil que su ventana toca — MES
        // COMPLETO, sin prorrateo por días (coherente con presupuesto, servicio de deuda y los
        // ajustes por end-date; prorratear los meses frontera reintroduciría la dependencia del
        // día que #126 retiró). `window_end_date` NULL = hasta el horizonte.
        if flow.amount_basis == "per_month" {
            let Some(win_start) = flow.window_start_date else { continue };
            for (idx, slot) in adj.iter_mut().enumerate() {
                let m_first = proj_add_months(anchor_month_first, idx as u32);
                let m_last = proj_month_last(m_first);
                let live = win_start <= m_last
                    && flow.window_end_date.is_none_or(|e| e >= m_first);
                if live {
                    *slot += signed;
                }
            }
            continue;
        }

        match flow.due_date {
            Some(due) => {
                if due < anchor_month_first {
                    // #126: lo vencido no desaparece — carga íntegro en el mes ancla.
                    if let Some(first) = adj.first_mut() {
                        *first += signed;
                    }
                    continue;
                }
                for idx in 0..horizon_months as usize {
                    let m_first = proj_add_months(anchor_month_first, idx as u32);
                    let m_last = proj_month_last(m_first);
                    if due >= m_first && due <= m_last {
                        adj[idx] += signed;
                        break;
                    }
                }
            }
            None => {
                let daily = signed / Decimal::from(PLANNING_UNDATED_SPREAD_DAYS);
                for idx in 0..horizon_months as usize {
                    let m_first = proj_add_months(anchor_month_first, idx as u32);
                    let m_last = proj_month_last(m_first);
                    let days = overlap_inclusive_days(
                        m_first,
                        m_last,
                        undated_win_first,
                        undated_win_last,
                    );
                    if days > 0 {
                        adj[idx] += daily * Decimal::from(days);
                    }
                }
            }
        }
    }
    adj
}

fn expense_end_date_monthly_adjustments(
    today: NaiveDate,
    horizon_months: u32,
    entries: &[(Decimal, NaiveDate)],
) -> Vec<Decimal> {
    let mut adj = vec![Decimal::ZERO; horizon_months as usize];
    let anchor = proj_month_first(today);
    for (amount, end_date) in entries {
        for idx in 0..horizon_months as usize {
            let m_first = proj_add_months(anchor, idx as u32);
            if m_first > *end_date {
                // Cancel the expense from this month onwards (base rate already deducts it).
                for i in idx..horizon_months as usize {
                    adj[i] += amount;
                }
                break;
            }
        }
    }
    adj
}

/// Neto de Próximos para el baseline de hitos: los TRES primeros meses ancla del MISMO mapeo
/// fecha→mes que alimenta la caja del motor (#126). Antes tenía su propia regla (ventana
/// `[hoy, hoy+89]` para los datados, íntegro para los sin fecha), que era la última superficie
/// del handler dependiente del día de la consulta.
/// #178: qué régimen fiscal gobierna el drenaje, derivado de los INPUTS (los mismos que ve el
/// engine). Un activo «declara coste» ⟺ `purchase_price` presente (incluido 0 — «todo es
/// ganancia» es una declaración, no una ausencia).
/// 4.12.1: la razón del ahorro varado — solo hay dos causas posibles (demostrado: un sumidero
/// habilitado no tiene tope y se lleva `remaining` entero, así que con él el leftover es 0).
fn unallocated_reason_of(
    assets: &[futurefin_engine::SimAsset],
    total: Decimal,
) -> Option<&'static str> {
    if total <= Decimal::ZERO {
        None
    } else if assets.is_empty() {
        Some("no_assets")
    } else {
        Some("no_sink")
    }
}

fn drawdown_gain_basis_of(assets: &[futurefin_engine::SimAsset]) -> &'static str {
    let declared = assets.iter().filter(|a| a.purchase_price.is_some()).count();
    if declared == 0 {
        "declared_ratio"
    } else if declared == assets.len() {
        "cost_basis"
    } else {
        "mixed"
    }
}

/// #178: `g₀` informativa de la cartera de hoy, SOLO sobre activos con coste declarado
/// (mezclar los no declarados fabricaría una cifra con el escalar dentro). `None` si ninguno
/// declara o si su valor agregado es ≤ 0.
fn taxable_gain_ratio_today_of(assets: &[futurefin_engine::SimAsset]) -> Option<Decimal> {
    let mut total = Decimal::ZERO;
    let mut gains = Decimal::ZERO;
    for a in assets {
        if let Some(pp) = a.purchase_price {
            if a.value > Decimal::ZERO {
                total += a.value;
                gains += (a.value - pp).max(Decimal::ZERO);
            }
        }
    }
    if total > Decimal::ZERO {
        Some(money_out(gains / total))
    } else {
        None
    }
}

fn planning_upcoming_net_for_milestone_baseline(
    ref_date: NaiveDate,
    flows: &[PlanningFlowProjRow],
) -> Decimal {
    planning_monthly_cash_adjustments_from_flows(ref_date, 3, flows)
        .iter()
        .copied()
        .sum()
}

fn projection_next_milestone(after: Decimal) -> Decimal {
    let steps = [Decimal::ONE, Decimal::new(25, 1), Decimal::from(5i64)];
    let minimum = Decimal::from(PROJECTION_MILESTONE_MINIMUM);
    let safe_value = after.max(minimum);
    let safe_f64 = safe_value.to_f64().unwrap_or(PROJECTION_MILESTONE_MINIMUM as f64);
    let power = safe_f64.log10().floor() as i32;
    let magnitude = Decimal::from(10i64).powi(power.into());
    for step in steps {
        let candidate = step * magnitude;
        if candidate > safe_value {
            return candidate;
        }
    }
    Decimal::from(10i64).powi((power + 1).into())
}

fn projection_next_milestones(from: Decimal, count: usize) -> Vec<Decimal> {
    let mut out = Vec::with_capacity(count);
    let mut current = from;
    for _ in 0..count {
        let next = projection_next_milestone(current);
        // Los tres pasos son `1`, `2.5` y `5` por magnitud: `2.5 × 10⁴` hereda la escala 1 del
        // literal y salía `"25000.0"` en el mismo array que `"50000"` y `"100000"`. Un hito es un
        // umbral redondo; se publica con escala 0 y el array deja de mezclar formatos (auditoría MCP §7).
        let mut next = next.round_dp(0);
        next.rescale(0);
        out.push(next);
        current = next;
    }
    out
}

/// Factor que lleva un importe nominal del mes `month_index` a euros de hoy:
/// `1 / (1 + inflación%)^(month_index/12)`. Con inflación ≤ 0 devuelve exactamente `1`, así que el
/// importe deflactado es **el mismo valor**, no uno aproximadamente igual.
///
/// El exponente sale del `month_index` del punto, **jamás de su posición en el array**. Hoy
/// coinciden cuando se recorre la serie mensual completa, pero bajo densidad `hybrid` los puntos no
/// son equidistantes y la versión ingenua deflacta 70 años como si fueran 30 — es el bug que la
/// v1.4.2 arregló en el chart. Un único helper con varios callers en este fichero — cuéntalos con
/// un `grep -c` sobre el nombre de esta función seguido de un paréntesis abierto, en
/// `apps/api/src/handlers/*.rs` (no lo escribo pegado aquí para que este comentario no se cuente
/// a sí mismo; el resultado incluye la propia definición, así que son callers + 1) —, por el
/// mismo motivo por el que existe `fire_target_at_month_index`.
pub(crate) fn deflator_at_month_index(
    annual_inflation_percent: Decimal,
    month_index: u32,
) -> Decimal {
    // `is_zero()`, NO `<= ZERO` (#146): con inflación NEGATIVA el deflactor es > 1 — los euros
    // futuros valen MÁS en euros de hoy, y `net_worth_real` queda por encima del nominal.
    if annual_inflation_percent.is_zero() || month_index == 0 {
        return Decimal::ONE;
    }
    let infl_factor = Decimal::ONE + annual_inflation_percent / Decimal::from(100u32);
    let years = Decimal::from(month_index) / Decimal::from(12u32);
    Decimal::ONE / infl_factor.powd(years)
}

/// Factor de capitalización de una tasa REAL anual `g` a los `month_index` meses:
/// `(1 + g/100)^(month_index/12)`. Con `g == 0` o `month_index == 0` devuelve `ONE` **exacto**,
/// sin pasar por `powd`.
///
/// Es la gemela de `deflator_at_month_index` (misma base, exponente positivo) y comparte forma
/// con `inflation_factor_at_month_index` del motor, que **no está re-exportada** por
/// `futurefin_engine` (vive en un `mod projection` privado). Se escribe aquí en vez de deducirla
/// como `1 / deflator(...)`: esa vuelta introduce un error de redondeo en un factor que multiplica
/// dinero, y el redondeo de presentación no es sitio para meterlo. Si algún día la del motor se
/// exporta, este helper se retira y se llama a aquella — hay una petición abierta para eso.
///
/// El eje es `month_index/12`, el mismo que indexa el gasto y el objetivo: el mes 1 del bucle
/// (índice 0) tiene factor 1 y crecimiento extra exactamente 0.
fn real_growth_factor_at_month_index(annual_percent: Decimal, month_index: u32) -> Decimal {
    if annual_percent.is_zero() || month_index == 0 {
        return Decimal::ONE;
    }
    let years = Decimal::from(month_index) / Decimal::from(12u32);
    (Decimal::ONE + annual_percent / Decimal::from(100u32)).powd(years)
}

/// Deflacta una serie de puntos a euros de hoy. Es la versión a resolución mensual completa de la
/// deflactación **visual** que hace el chart de la web (`ProjectionNetWorthChart.baseSeries`);
/// calcularla aquí preserva la precisión del `reached_month_index` de los milestones bajo densidad
/// `hybrid`, donde el cliente solo recibe puntos anuales. Con inflación 0 devuelve una copia sin
/// cambios.
fn deflate_points_to_today(
    points: &[NwPoint],
    annual_inflation_percent: Decimal,
) -> Vec<NwPoint> {
    if annual_inflation_percent.is_zero() {
        return points.to_vec();
    }
    points
        .iter()
        .map(|p| NwPoint {
            month_index: p.month_index,
            net_worth: p.net_worth
                * deflator_at_month_index(annual_inflation_percent, p.month_index),
        })
        .collect()
}

fn projection_unique_reached_milestones(
    points: &[NwPoint],
    anchor_date: NaiveDate,
    baseline_adjustment: Decimal,
    limit: usize,
    search_count: usize,
) -> Vec<ProjectionMilestone> {
    if points.is_empty() || limit == 0 {
        return vec![];
    }
    let baseline = points[0].net_worth + baseline_adjustment;
    let generated = projection_next_milestones(baseline, search_count.max(limit));
    let mut events: Vec<ProjectionMilestone> = Vec::with_capacity(limit);
    let mut last_year: Option<i32> = None;

    for milestone in generated {
        let Some(reached_idx) = points.iter().position(|p| p.net_worth >= milestone) else {
            break;
        };
        let reached_month_index = points[reached_idx].month_index;
        let reached_date = proj_add_months(proj_month_first(anchor_date), reached_month_index);
        let event = ProjectionMilestone {
            target: milestone,
            reached_month_index,
            reached_date_ymd: reached_date.format("%Y-%m-%d").to_string(),
        };

        if let Some(prev_year) = last_year {
            if prev_year == reached_date.year() {
                let replace_index = events.len() - 1;
                events[replace_index] = event;
            } else {
                events.push(event);
                last_year = Some(reached_date.year());
            }
        } else {
            events.push(event);
            last_year = Some(reached_date.year());
        }

        if events.len() >= limit {
            break;
        }
    }

    events
}

fn compound_outpaces_true_savings_month(
    input: &ProjectionInput,
    monthly_delta_assumption: Decimal,
) -> Result<Option<u32>, EngineError> {
    if monthly_delta_assumption <= Decimal::ZERO {
        return Ok(None);
    }
    let mut neutral = input.clone();
    neutral.planning_monthly_cash_adjustment =
        vec![Decimal::ZERO; input.horizon_months as usize];
    for liab in neutral.liabilities.iter_mut() {
        liab.monthly_payment = Decimal::ZERO;
    }
    let out = project_net_worth_series(&neutral)?;
    let mut consecutive = 0u32;
    const REQUIRED_CONSECUTIVE_MONTHS: u32 = 3;
    for k in 1..out.net_worth.len() {
        let nw_delta = out.net_worth[k] - out.net_worth[k - 1];
        let savings_delta = out.contributed_capital[k] - out.contributed_capital[k - 1];
        if savings_delta <= Decimal::ZERO {
            consecutive = 0;
            continue;
        }
        let market_delta = nw_delta - savings_delta;
        if market_delta > savings_delta {
            consecutive += 1;
            if consecutive >= REQUIRED_CONSECUTIVE_MONTHS {
                return Ok(Some(k as u32 + 1 - REQUIRED_CONSECUTIVE_MONTHS));
            }
        } else {
            consecutive = 0;
        }
    }
    Ok(None)
}

/// Cuota mensual equivalente de un importe periódico: `weekly → ×52/12`, cualquier otra
/// frecuencia (incluida `monthly`) → el importe tal cual. Fuente única de esta conversión;
/// también la usa `history.rs` al construir [`futurefin_engine::LoanTerms`].
pub(crate) fn monthly_payment_from(amount: Decimal, frequency: Option<&str>) -> Decimal {
    match frequency {
        Some("weekly") => (amount * Decimal::from(52u32)) / Decimal::from(12u32),
        _ => amount,
    }
}

/// Cuota mensual equivalente de un pasivo a partir de sus campos crudos (`payment_amount`,
/// `payment_frequency`). Sin importe → `0`. Fuente única compartida por `projection.rs` (filas
/// `LiabEngineRow` ya cargadas para el engine) y `summary.rs` (filas de una SELECT dedicada).
pub(crate) fn liability_monthly_payment(
    amount: Option<Decimal>,
    frequency: Option<&str>,
) -> Decimal {
    match amount {
        Some(amt) => monthly_payment_from(amt, frequency),
        None => Decimal::ZERO,
    }
}

/// Predicado de "pasivo activo" (misma semántica que el `WHERE payment_end_date IS NULL OR >= today`
/// de las lecturas SQL): un pasivo sin fecha de fin de pago o cuya fecha aún no ha pasado sigue vivo.
pub(crate) fn liability_is_active(payment_end_date: Option<NaiveDate>, today: NaiveDate) -> bool {
    payment_end_date.map_or(true, |d| d >= today)
}

/// Completed calendar age in years (`today` inclusive), used for horizon.
fn age_completed_years(today: NaiveDate, birth: NaiveDate) -> i32 {
    if birth > today {
        return 0;
    }
    let mut y = today.year() - birth.year();
    let bd_month = birth.month();
    let bd_day = birth.day();
    let td_month = today.month();
    let td_day = today.day();
    if (td_month, td_day) < (bd_month, bd_day) {
        y -= 1;
    }
    y
}

/// Lectura civil de un índice de mes de la proyección: `(fecha YYYY-MM-DD, años cumplidos)`.
///
/// Convierte el «mes 137» —único dato que teníamos— en las dos cifras que de verdad se leen. El
/// índice NO se sustituye: sigue siendo la clave para indexar las series.
///
/// **Regla de calendario**: se suman `mi` meses al ancla CONSERVANDO su día, con recorte a fin de
/// mes (31 ene + 1 mes = 28 feb). Es exactamente `addMonthsCivil` de la web
/// (`apps/web/src/lib/dates.ts`), y por eso `age` coincide con la etiqueta «N a» del chart.
/// Anclar al día 1 en su lugar restaría UN AÑO a la edad cuando el cruce cae en el mes de
/// cumpleaños y el nacimiento no es día 1 — una discrepancia silenciosa y anual.
///
/// `mi` se suma entero (no `mi − 1`): el mes 0 es el estado de HOY y el mes `k` nombra la frontera
/// en la que el cruce ya es cierto. Misma convención que los hitos y que la web.
///
/// Nota: `ProjectionMilestone::reached_date_ymd` sí ancla al día 1 (contrato ya publicado, se deja
/// como está). Ambas coinciden siempre en año y mes; solo difieren en el día.
pub(crate) fn jubilacion_civil(
    today: NaiveDate,
    birth: Option<NaiveDate>,
    mi: Option<u32>,
) -> (Option<String>, Option<u32>) {
    // `mi == 0` es válido (ya-FIRE hoy) → fecha = hoy. Nunca tratarlo como «no alcanzado».
    let Some(mi) = mi else { return (None, None) };
    let at = proj_add_months(today, mi);
    let age = birth.map(|b| age_completed_years(at, b).max(0) as u32);
    (Some(at.format("%Y-%m-%d").to_string()), age)
}

/// Máximo años hasta `lifespan_age` años de edad por fecha de nacimiento; acotado [5, 70];
/// sin DOB → 30 años. Desde 4.9.0 (#149) la edad límite es configurable
/// (`fire_settings.horizon_lifespan_age`, 85..=105, default 90) y el basis pasa de
/// `"lifespan_90"` (un número congelado en un literal publicado) a `"lifespan_age"` + el campo
/// `horizon_lifespan_age` al lado. OJO: el clamp [5, 70] NO se toca, así que el eje solo tiene
/// efecto si `edad ≥ lifespan_age − 70` — a 105 años, un treintañero ya está contra el techo
/// del sistema (840 meses).
pub(crate) fn projection_horizon_months(
    today: NaiveDate,
    birth_dates: &[Option<NaiveDate>],
    lifespan_age: u32,
) -> (u32, &'static str) {
    const MIN_YEARS: u32 = 5;
    const MAX_YEARS: u32 = 70;
    const FALLBACK_YEARS: u32 = 30;

    let mut max_remaining: Option<i32> = None;
    let mut any_birth = false;
    for bd in birth_dates {
        let Some(birth) = *bd else {
            continue;
        };
        any_birth = true;
        let age = age_completed_years(today, birth);
        let rem = (lifespan_age as i32 - age).max(0);
        max_remaining = Some(max_remaining.map_or(rem, |m| m.max(rem)));
    }

    if !any_birth {
        return (FALLBACK_YEARS * 12, "fallback_no_demographics");
    }

    let years_raw = max_remaining.unwrap_or(0).max(0) as u32;
    let clamped_years = years_raw.clamp(MIN_YEARS, MAX_YEARS);
    (clamped_years * 12, "lifespan_age")
}

/// **Mes de la rejilla en que el usuario cumple `age`** (5.0.0, D2/D17): el menor `m` tal que en
/// la fecha civil de `points[m]` los años cumplidos ya son `age`. `0` si ya los tiene hoy.
///
/// Se define sobre la MISMA aritmética que publica la lectura civil de la jubilación
/// (`proj_add_months` + [`age_completed_years`], la de `addMonthsCivil` de la web), no sobre una
/// resta de años × 12. Eso compra un invariante comprobable: con una estrategia por edad, la
/// respuesta cumple `jubilacion_age == target_retirement_age` exactamente — sin él, un nacimiento
/// a final de mes podía publicar «te jubilas a los 54» habiendo pedido 55.
///
/// La estimación inicial (diferencia de edades × 12) está siempre a ≤ 12 meses del resultado, así
/// que los dos bucles de corrección son O(1) — nunca un escaneo del horizonte.
pub(crate) fn months_until_target_age(today: NaiveDate, birth: NaiveDate, age: u32) -> u32 {
    let target = age as i32;
    let hoy = age_completed_years(today, birth);
    if hoy >= target {
        return 0;
    }
    let mut m = ((target - hoy).max(0) as u32).saturating_mul(12);
    while m > 0 && age_completed_years(proj_add_months(today, m - 1), birth) >= target {
        m -= 1;
    }
    while age_completed_years(proj_add_months(today, m), birth) < target {
        m += 1;
    }
    m
}

/// Regla de retirada del PERFIL → regla del MOTOR. Traducción total y sin brazo comodín: si
/// mañana el catálogo del perfil gana una variante, esto deja de compilar — que es exactamente
/// lo que debe pasar, en vez de mapearla en silencio a `fixed_real` y simular otra cosa.
///
/// **U4 — el porcentaje de retirada es el SWR del perfil cuando la regla no trae uno propio.**
/// La herencia no se hace aquí: se delega en `resolve_withdrawal_rule`, el resolvedor único del
/// módulo del perfil, para que lo que simula el motor sea exactamente lo que publica
/// `GET /v1/auth/me/retirement-profile`. Llamarlo aquí es además una red: los perfiles que
/// llegan ya vienen resueltos (todos los caminos pasan por `resolve_retirement_profile`), y
/// como el resolvedor es idempotente, volver a pasarlo no mueve un valor explícito y sí evita
/// que un futuro camino sin resolver mande un `pct` ausente al motor.
///
/// Un `pct` que siga ausente tras resolver solo puede venir de una fila escrita fuera de la API
/// con un `kind` que no lo usa: se traduce a `0`, que hace que el motor no retire nada y sea
/// evidente en la serie. Inventar un default «razonable» publicaría un plan que nadie configuró.
fn withdrawal_rule_to_engine(
    rule: &ProfileWithdrawalRule,
    swr_pct: Decimal,
) -> EngineWithdrawalRule {
    let rule = &crate::handlers::retirement_profile::resolve_withdrawal_rule(rule, swr_pct);
    let z = Decimal::ZERO;
    match rule.kind {
        ProfileWithdrawalRuleKind::FixedReal => EngineWithdrawalRule::FixedReal,
        ProfileWithdrawalRuleKind::PercentOfBalance => EngineWithdrawalRule::PercentOfBalance {
            pct: rule.pct.unwrap_or(z),
        },
        ProfileWithdrawalRuleKind::Hybrid => EngineWithdrawalRule::Hybrid {
            start_pct: rule.start_pct.unwrap_or(z),
            end_pct: rule.end_pct.unwrap_or(z),
        },
        ProfileWithdrawalRuleKind::Guardrails => EngineWithdrawalRule::Guardrails {
            pct: rule.pct.unwrap_or(z),
            band_pct: rule.band_pct.unwrap_or(z),
            adjust_pct: rule.adjust_pct.unwrap_or(z),
        },
    }
}

/// Literal público de una estrategia del perfil. Se deriva del `Serialize` del enum para que no
/// haya dos listas de strings que puedan divergir.
pub(crate) fn strategy_label(s: RetirementStrategy) -> String {
    serde_json::to_value(s)
        .ok()
        .and_then(|v| v.as_str().map(str::to_string))
        // Inalcanzable: el enum serializa siempre a una cadena. El fallback existe para no
        // meter un `unwrap` en un camino de lectura.
        .unwrap_or_else(|| "asap".to_string())
}

/// **Mes del BUCLE (1-based) → mes de la REJILLA publicada (0-based).**
///
/// El motor cuenta meses simulados: su mes `k` cubre el mes civil `ancla + (k−1)` y su cierre es
/// la casilla `k` de las series. Las salidas de fase (`retirement_month_index`,
/// `liquid_crossing_month_index`, `phase_transitions`) hablan en meses del BUCLE; todo lo que la
/// API publica —`points[].month_index`, `jubilacion_month_index`, `jubilacion_date_ymd`— habla en
/// la REJILLA, donde el 0 es hoy y el mes `m` nombra la frontera en la que el hecho ya es cierto.
///
/// El primer mes VIVIDO como jubilado es el `k` del motor, y su mes civil es `ancla + (k−1)` —
/// exactamente la fecha que `jubilacion_civil` publica para la casilla `k − 1`. Por eso la
/// conversión es `k − 1` y no una elección de estilo: con `k` a pelo, `jubilacion_date_ymd` se
/// iría un mes al futuro y los pins de 4.15.x se moverían sin que nada del modelo cambiara.
///
/// **Sin excepciones desde 5.0.0 (#210).** `assets_depleted_month_index` se publicó en meses del
/// BUCLE desde #119 y era la única salida de la respuesta que hablaba otro idioma: compararla con
/// `jubilacion_month_index` o usarla para indexar `points[]` daba un mes de más. Se pasa por aquí
/// como el resto — breaking declarado en el CHANGELOG de 5.0.0.
pub(crate) fn engine_month_to_grid(k: Option<u32>) -> Option<u32> {
    k.map(|k| k.saturating_sub(1))
}

/// Eco cerrado del trigger de jubilación del `PhasePlan`. Dos literales, sin brazo comodín.
pub(crate) const RETIREMENT_TRIGGER_CROSSING: &str = "liquid_crossing";
pub(crate) const RETIREMENT_TRIGGER_AGE: &str = "target_age";

/// La estrategia degradó a `asap` porque el usuario no tiene fecha de nacimiento: sin ella no hay
/// edad que convertir en mes, y un 500 por un campo opcional del perfil sería el peor cambio
/// posible (§A del plan de #207).
pub(crate) const WARN_BIRTH_DATE_MISSING: &str = "birth_date_missing";
/// Ídem sin `target_retirement_age`. La validación del PATCH lo exige en `retire_at_age`/`coast`,
/// así que solo es alcanzable con un perfil escrito antes de esa validación o a mano en la BD;
/// se degrada igualmente en vez de reventar una LECTURA.
pub(crate) const WARN_TARGET_AGE_MISSING: &str = "target_retirement_age_missing";
/// El puente se pidió con descuento por **rentabilidad esperada** y el scope no tiene ni un euro
/// de activo LÍQUIDO del que sacarla, así que la tasa cae a 0 (puente sin descuento: la suma
/// llana de sus flujos, la lectura conservadora). Se avisa porque un objetivo puente sin
/// descontar es sensiblemente MAYOR que el mismo puente descontado, y la diferencia no se
/// explicaría por ningún campo de la respuesta. Solo viaja cuando la tasa se iba a usar de
/// verdad (base puente + pensión con fecha resuelta).
pub(crate) const WARN_BRIDGE_DISCOUNT_NO_LIQUID_ASSETS: &str = "bridge_discount_no_liquid_assets";
/// El descuento derivado de la **rentabilidad esperada** salió NEGATIVO (la cartera líquida
/// declara pérdidas esperadas) y se ha subido a 0 antes de entrar al motor.
///
/// Dos razones, y ninguna es cosmética:
///
/// 1. **Un descuento negativo hace el puente MÁS CARO que no descontar nada.** Descontar es
///    responder «cuánto capital necesito HOY para pagar un flujo futuro»; con `d < 0` el factor
///    `(1+d/100)^{j/12}` es menor que 1 y cada euro futuro cuesta más de un euro hoy. Eso no es
///    un objetivo conservador: es un objetivo que crece sin límite conforme se aleja la pensión.
/// 2. **Con `d` suficientemente negativo la tabla desborda `Decimal`** y el motor devuelve
///    [`futurefin_engine::EngineError::BridgeDiscountOverflow`] (−53,8 % a 840 meses de puente,
///    −41,8 % a 1200). El clamp acota el input antes de que el motor tenga que rechazarlo.
///
/// Solo se emite cuando la tasa se iba a usar de verdad (base puente + pensión con fecha
/// resuelta), igual que [`WARN_BRIDGE_DISCOUNT_NO_LIQUID_ASSETS`]. Las otras dos bases no pueden
/// producirlo: `none` es 0 por definición y `swr` está acotada a `(0, 4]` por el PATCH.
pub(crate) const WARN_BRIDGE_DISCOUNT_CLAMPED: &str = "bridge_discount_clamped";

/// **Rentabilidad esperada anual (%) de la cartera LÍQUIDA de hoy, ponderada por valor** (D7).
///
/// `Σ vᵢ·rᵢ / Σ vᵢ` sobre los activos con `is_liquid`, con `rᵢ = 0` para los que no declaran
/// rentabilidad (misma convención que `net_return_percentages`: «None cuenta como 0 %» sin salir
/// del denominador — un activo sin tasa declarada es una cuenta corriente, no un hueco).
///
/// `None` ⟺ **no hay base sobre la que ponderar**: ni un activo líquido, o su valor total no es
/// positivo. Nunca un `0` inventado: el llamante decide qué hacer con la ausencia y, si la tasa
/// iba a usarse, la declara con [`WARN_BRIDGE_DISCOUNT_NO_LIQUID_ASSETS`].
///
/// **No se reutiliza `net_return_percentages`** (`crates/engine/src/net_return.rs`) a propósito:
/// aquella pondera sobre el PATRIMONIO NETO y resta el coste de la deuda, así que con
/// apalancamiento devuelve una tasa amplificada — correcta para «cuánto renta tu patrimonio», y
/// equivocada como tasa de descuento de un puente que se paga vendiendo cartera. Aquí la
/// pregunta es «a qué tasa crece el dinero que voy a ir gastando», y ese dinero es el líquido.
fn liquid_expected_return_pct(assets: &[SimAsset]) -> Option<Decimal> {
    let mut total = Decimal::ZERO;
    let mut weighted = Decimal::ZERO;
    for a in assets.iter().filter(|a| a.is_liquid) {
        let v = a.value.max(Decimal::ZERO);
        total += v;
        weighted += v * a.expected_annual_return_percent.unwrap_or(Decimal::ZERO);
    }
    if total <= Decimal::ZERO {
        return None;
    }
    weighted.checked_div(total)
}

pub(crate) fn map_engine_err(e: EngineError) -> ApiError {
    match e {
        // 5.0.0: el `PhasePlan` pidió algo que este motor todavía no ejecuta (una regla de
        // retirada ≠ `fixed_real`, o una fase de WP3). No es un input INVÁLIDO —el perfil que lo
        // produjo es legítimo y está validado—, es una capacidad que aún no existe, así que
        // merece su propio código: `engine_rejected_input` mandaría al usuario a corregir unos
        // datos que están bien.
        EngineError::UnsupportedWithdrawalRule | EngineError::UnsupportedPhase => {
            ApiError::BadRequest(format!("engine_feature_unavailable: {e}"))
        }
        // La tabla del puente desbordó `Decimal` porque el descuento derivado es tan negativo que
        // `G(m)/(1+d/100)^{m/12}` explota (la cota depende de los meses hasta la pensión: −53,8 %
        // a 840 meses, −41,8 % a 1200). **422 y no 400**: la petición está bien formada y los
        // datos son válidos uno a uno — lo que no cabe es el número que sale de combinarlos, que
        // es exactamente el 422 del catálogo. Hasta el pase de correcciones de la revisión
        // adversarial esto era un `panic` dentro de `powd` y salía como un 500 opaco.
        //
        // **Aguas arriba está el clamp** (`build_installation_projection_input`), que impide
        // producir un descuento negativo desde la API; esta rama existe para el input que
        // llegue por otro camino y para que el fallo tenga nombre en vez de tumbar el proceso.
        EngineError::BridgeDiscountOverflow => ApiError::Unprocessable(
            "bridge_discount_out_of_range: the derived bridge discount (bridge_discount_annual_pct) \
             is too negative for the number of months until the dated pension (pension_start_month_index) \
             — the discounted bridge target overflows; raise the expected return of the liquid assets \
             or switch bridge_discount_basis to `swr` or `none`"
                .into(),
        ),
        _ => ApiError::BadRequest(format!("engine_rejected_input: {e}")),
    }
}

// `fire_crossover_month` MURIÓ en WP5-2b (5.0.0). Escaneaba la serie líquida contra
// `fire_target_at_month_index` porque a las estrategias por edad se les pasaba `fire_target:
// None` para que el cruce no las jubilara, y sin objetivo en el input el motor no podía anotar su
// propia lectura. Con `crossing_is_reading_only` el objetivo entra siempre y el cruce lo publica
// el motor (`ProjectionOutput::liquid_crossing_month_index`), evaluado sobre el objetivo
// CONSCIENTE DEL PLAN. La diferencia no era de estilo: con `bridge_to_pension` el handler cruzaba
// contra la perpetuidad y el motor contra el puente — dos cruces distintos para la misma línea.

// ---------------------------------------------------------------------------
// Resolución compartida de los inputs de ahorro (proyección ↔ /v1/summary)
// ---------------------------------------------------------------------------

/// Procedencia de UN lado del ahorro. Sustituye al escalar `savings_source_months_with_data`: con
/// dos ventanas no existe *un* número de meses, y servir uno solo mal-etiquetaría la mitad de la
/// UI (un «3» mientras el otro lado promedió 12).
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct SavingsAvgBasis {
    /// `budget` = este lado salió del presupuesto (el modo no lo promedia, o cayó por falta de
    /// datos). `average` = promedio real de transacciones.
    pub basis: &'static str,
    /// Denominador REALMENTE usado — los meses que se promediaron. `0` ⟺ `basis == "budget"`.
    ///
    /// Se llamaba `months_with_data`, y ese nombre significa **lo contrario** en la otra
    /// familia de respuestas: en `GET /v1/transactions/summary`, `months_with_data` son los
    /// meses que HAY en el tramo y el denominador es `avg_months`. Un consumidor que
    /// preguntara «¿sobre cuántos meses está calculada mi media?» citaba 9 (los que hay)
    /// cuando el motor promedió 6 (los reales) — y con esa cifra justificaba un ahorro
    /// proyectado que no cuadraba. Ahora las dos familias usan `avg_months` para el
    /// denominador y `months_with_data` solo donde significa «lo que hay».
    pub avg_months: u32,
    /// Ventana configurada tras el clamp (permite decir «pediste 12, hay 7»). `0` si no aplica.
    pub window_months: u32,
    /// `"data"` | `"calendar"`; ausente cuando el lado no promedia.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub window_mode: Option<&'static str>,
    /// Mes más antiguo incluido (`YYYY-MM`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub first_month: Option<String>,
    /// Mes más reciente incluido (`YYYY-MM`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_month: Option<String>,
    /// `true` ⟺ los meses incluidos NO son consecutivos: la UI no puede pintarlos como un rango.
    pub has_gaps: bool,
}

impl SavingsAvgBasis {
    pub(crate) fn budget() -> Self {
        Self {
            basis: "budget",
            avg_months: 0,
            window_months: 0,
            window_mode: None,
            first_month: None,
            last_month: None,
            has_gaps: false,
        }
    }
    fn from_side(side: &AvgSide) -> Self {
        Self {
            basis: "average",
            avg_months: side.months_with_data,
            window_months: side.window.months,
            window_mode: Some(match side.window.mode {
                AvgWindowMode::Data => "data",
                AvgWindowMode::Calendar => "calendar",
            }),
            first_month: side.first_month.clone(),
            last_month: side.last_month.clone(),
            has_gaps: side.has_gaps,
        }
    }
}

/// Inputs regulares efectivos del mes 0, con el override de modo y el fallback ya resueltos.
pub(crate) struct EffectiveSavingsInputs {
    pub income: Decimal,
    pub expense: Decimal,
    /// `true` ⟺ el GASTO viene del promedio real. Gobierna la anulación de `payment_amount` de los
    /// pasivos, el zeroing de `end_adj` y la base del target FIRE — las tres son afirmaciones
    /// sobre la base de GASTO. Cablearlo al lado ingreso haría desaparecer la cuota del horizonte
    /// entero en modo B con datos de ingreso y sin datos de gasto.
    pub expense_from_avg: bool,
    /// Fuente efectiva. Colapsa a `Budget` **⟺ AMBOS** lados cayeron al presupuesto — es lo que
    /// preserva la garantía de que en ese caso el bloque es idéntico al modo A.
    pub effective_source: SavingsSource,
    pub income_basis: SavingsAvgBasis,
    pub expense_basis: SavingsAvgBasis,
}

/// Resolución ÚNICA del override B/C y del fallback POR LADO, compartida por la proyección y
/// `/v1/summary`. **Pura**: sin BD y sin reloj, así que las combinaciones de modo × fallback son
/// testeables sin Postgres.
///
/// Los escalares de presupuesto entran como PARÁMETROS porque los dos call-sites usan bases
/// distintas: `projection.rs` sin cuotas de pasivo (`budget.rs` avisa del doble conteo) y
/// `summary.rs` con ellas. Buscarlos aquí dentro reintroduciría esa divergencia.
///
/// **El fallback es POR LADO, no todo-o-nada.** Con `income = 3` y `expense = 12`, un hogar que
/// deja de importar cuatro meses tendría 0 meses de ingreso y 8 de gasto: tirar 8 meses de gasto
/// realmente medido para volver al presupuesto sería peor que la asimetría. Y deja de ser
/// silencioso porque cada lado publica su `basis`.
pub(crate) fn resolve_effective_savings_inputs(
    source: SavingsSource,
    budget_income: Decimal,
    budget_expense: Decimal,
    avg: Option<&TransactionsAvg>,
) -> EffectiveSavingsInputs {
    // `match` exhaustivo a propósito (sin `_ =>`): una variante nueva del enum debe romper la
    // compilación aquí en vez de heredar el comportamiento de otra en silencio.
    let (use_income_avg, use_expense_avg) = match source {
        SavingsSource::Budget => (false, false),
        SavingsSource::TransactionsAvg => (true, true),
        SavingsSource::BudgetIncomeRealExpense => (false, true),
    };
    let inc_side = avg.map(|a| &a.income).filter(|_| use_income_avg);
    let exp_side = avg.map(|a| &a.expense).filter(|_| use_expense_avg);
    let inc_ok = inc_side.map(|s| s.months_with_data > 0).unwrap_or(false);
    let exp_ok = exp_side.map(|s| s.months_with_data > 0).unwrap_or(false);

    EffectiveSavingsInputs {
        income: match inc_side.filter(|_| inc_ok) {
            Some(s) => s.avg,
            None => budget_income,
        },
        expense: match exp_side.filter(|_| exp_ok) {
            Some(s) => s.avg,
            None => budget_expense,
        },
        expense_from_avg: exp_ok,
        effective_source: if inc_ok || exp_ok {
            source
        } else {
            SavingsSource::Budget
        },
        income_basis: match inc_side.filter(|_| inc_ok) {
            Some(s) => SavingsAvgBasis::from_side(s),
            None => SavingsAvgBasis::budget(),
        },
        expense_basis: match exp_side.filter(|_| exp_ok) {
            Some(s) => SavingsAvgBasis::from_side(s),
            None => SavingsAvgBasis::budget(),
        },
    }
}

pub(crate) struct BuiltProjection {
    pub input: ProjectionInput,
    pub monthly_net_regular: Decimal,
    /// Ids de las reglas de asignación **alineados posición a posición** con
    /// `input.allocation_rules`. Imprescindible porque el constructor descarta las reglas cuyo
    /// activo destino queda fuera del scope, así que el índice del engine NO coincide con el
    /// orden de la tabla. Se rellena en el mismo `filter_map` que construye las reglas, para que
    /// la alineación sea una propiedad de construcción y no algo que haya que re-derivar.
    pub allocation_rule_ids: Vec<Uuid>,
    /// `(id, name)` por activo en el mismo orden que `input.assets` — evita un segundo SELECT.
    pub asset_id_name: Vec<(Uuid, String)>,
    /// **Volatilidad anual (%) por activo, alineada posición a posición con `input.assets`**
    /// (5.0.0, §A.2/§B.5). `None` = activo determinista (cuenta corriente, depósito).
    ///
    /// Va aquí y no dentro de `SimAsset` porque el motor `Decimal` **no la usa**: es entrada
    /// exclusiva de `futurefin_engine_stochastic::project_percentile_bands`, que la recibe como
    /// un vector paralelo y **falla** (`VolatilityLengthMismatch`) si no coincide en longitud con
    /// los activos. Se construye en el MISMO `map` que `assets` y `asset_id_name`, así que la
    /// alineación es una propiedad de construcción y no algo que haya que re-derivar: una
    /// volatilidad que se descoloca produce bandas estrechas y creíbles, el peor fallo posible
    /// aquí (regresión: `projection_bands.rs::the_volatility_vector_follows_the_asset_order`).
    pub asset_volatility_percent: Vec<Option<Decimal>>,
    /// `(id, label)` por pasivo, **alineado posición a posición** con `input.liabilities`. Misma
    /// razón que `allocation_rule_ids`: el índice del engine es una propiedad de construcción, no
    /// algo que se pueda re-derivar de la tabla sin volver a la BD y arriesgarse a otro orden.
    pub liability_id_label: Vec<(Uuid, String)>,
    /// Flujos de planificación crudos (scope + amount + due_date) — los reusa el handler para
    /// calcular el baseline de milestones sin tener que volver a la BD.
    pub planning_rows: Vec<PlanningFlowProjRow>,
    /// Fuente del ahorro **efectiva** (tras el fallback: modo B/C sin meses reales → `budget`).
    /// Es la que produjo `input.income_regular_monthly` / `input.expense_regular_monthly`.
    pub effective_savings_source: SavingsSource,
    /// Meses reales que alimentaron el promedio cuando la fuente efectiva usa transacciones; `0` en
    /// modo A y en el fallback.
    pub savings_income_basis: SavingsAvgBasis,
    pub savings_expense_basis: SavingsAvgBasis,
    /// Por qué este lado no tiene target FIRE, cuando no lo tiene. `None` ⟺ hay target, o no hay
    /// `fire_settings` que interpretar.
    pub fire_target_absent_reason: Option<&'static str>,
    /// Servicio de deuda mensual de los pasivos **activos** (`payment_end_date` nula o ≥ hoy), con
    /// la cuota nominal normalizada a mensual. No entra en `expense_regular_monthly` (el engine
    /// amortiza los pasivos aparte); lo consumen los caps `months_expense` de `assets.rs`. Desde
    /// 4.8.0 (#142, opción 3) es un importe REAL en los tres modos: en B/C el gasto efectivo ya
    /// restó la cuota declarada del promedio, así que cobrarla aquí la cuenta UNA vez.
    pub debt_service_monthly: Decimal,
    /// Desde 4.8.0 siempre `None`: la cuota es medible en los tres modos (ver arriba). El campo
    /// sobrevive porque las superficies que lo publican conservan su forma; el literal
    /// `included_in_real_expense` se retiró con el contrato de 3.4.0 que lo justificaba.
    pub debt_service_absent_reason: Option<&'static str>,
    /// **El objetivo FIRE como LECTURA** (5.0.0, D17). Es el mismo `FireTarget` que hasta 4.15.x
    /// viajaba dentro de `input.fire_target`, pero publicado aparte porque desde 5.0.0 los dos
    /// papeles se separan:
    ///
    /// - `input.fire_target` es el **trigger**: si está, el motor se jubila al cruzarlo.
    /// - `fire_target_reading` es lo que se DIBUJA y se lee (`fire_target_series`,
    ///   `jubilacion_target_net_worth*`, `liquid_crossing_month_index`).
    ///
    /// En las estrategias por cruce (`asap`, `pension_bridge` y `partial` sin edad total) son el
    /// mismo objeto. En las estrategias por EDAD el motor recibe `fire_target: None` —un solo
    /// trigger por simulación (D17)— y esto sigue siendo `Some(..)`: la línea discontinua del
    /// chart no desaparece porque la edad haya tomado el mando, solo deja de decidir.
    pub fire_target_reading: Option<FireTarget>,
    /// Qué dispara la jubilación en ESTE ensamblado: `liquid_crossing` | `target_age`. Eco
    /// cerrado del `PhasePlan::retirement_trigger`, para que la respuesta no obligue a deducirlo
    /// de la combinación (`strategy`, `fire_target_absent_reason`).
    pub retirement_trigger: &'static str,
    /// Avisos del ENSAMBLADO (no del motor): `birth_date_missing`,
    /// `target_retirement_age_missing` (una estrategia por edad degrada a `asap` en vez de
    /// reventar — §A del plan: «nunca un 500»), `bridge_discount_no_liquid_assets` y
    /// `bridge_discount_clamped` (la rentabilidad esperada del líquido era negativa).
    pub warnings: Vec<String>,
    /// **Mes del BUCLE (1-based) en que manda la edad**, `None` con trigger por cruce. Es la `R`
    /// de §B.7: el `target_month` que los solves necesitan, y el único sitio donde se resolvió
    /// (`edad → mes` con la fecha de nacimiento de ESTE miembro). Recalcularlo fuera obligaría a
    /// arrastrar la DOB y a duplicar la aritmética civil.
    pub forced_retirement_month: Option<u32>,
    /// **Tasa ANUAL (%) con la que el puente descontó sus flujos** — la que de verdad entró al
    /// `PhasePlan`, ya resuelta desde `bridge_discount_basis` (rentabilidad esperada ponderada
    /// del líquido, el propio SWR, o 0). `None` ⟺ el objetivo no es puente: ahí no hay tasa que
    /// publicar, y un `0` se leería como «puente sin descuento» en vez de «no hay puente».
    pub bridge_discount_annual_pct: Option<Decimal>,
    /// **El colchón de caja resuelto** (5.0.0, V6/P2): lo que se le pide al Monte Carlo y de
    /// dónde salió. Se resuelve AQUÍ y no en `resolve_retirement_profile` porque necesita
    /// activos, reglas y gasto de jubilación, y aquel es ledger-free por contrato (D25).
    ///
    /// Sus dos consumidores —`GET /v1/projection/bands` y el eje `monte_carlo` del what-if— leen
    /// este campo en vez de `retirement_profile.cash_buffer_months`: si cada uno derivara por su
    /// cuenta, la banda y el what-if podrían simular colchones distintos sin que nada lo dijera.
    pub cash_buffer: crate::handlers::cash_buffer::ResolvedCashBuffer,
}

/// Overrides what-if de `simulate_projection` que deben aplicarse DENTRO del ensamblado, en el
/// punto semántico correcto (antes de derivar target FIRE y bases de caps). Los overrides
/// post-build (ajustes de caja, one-off, tasas por activo) NO van aquí: se aplican mutando el
/// `ProjectionInput` clonado (patrón `compound_outpaces_true_savings_month`).
#[derive(Debug, Default, Clone)]
pub(crate) struct SimOverrides {
    /// Gasto mensual extra «real», **con signo**: se suma al gasto efectivo y al gasto de
    /// jubilación ANTES de derivar el target FIRE y las bases de caps. Negativo = recorte, que es
    /// el caso de uso más frecuente que existe y que hasta 4.0.0 se rechazaba (auditoría de simulate_projection §1).
    /// Mueve las bases de caps en los tres modos; el objetivo, solo en `annual_expense`.
    pub extra_monthly_expense: Decimal,
    /// Gasto MENSUAL de jubilación (el caller ya dividió el anual entre 12): sustituye por
    /// completo la base de gasto del target FIRE y el gasto post-jubilación (gana sobre
    /// `extra_monthly_expense` en ese tramo).
    pub retirement_monthly_expense: Option<Decimal>,
}

pub(crate) async fn build_installation_projection_input(
    pool: &sqlx::PgPool,
    iid: Uuid,
    session_user_id: Uuid,
    view: LedgerView,
    today: NaiveDate,
    horizon_months: u32,
    inflation_annual_percent: Decimal,
    fire_settings: Option<&FireSettings>,
    // `retirement_profile`: perfil de jubilación del usuario de la SESIÓN (5.0.0, D13). De aquí
    // salen el modo del objetivo, el importe manual y el SWR. No es `Option` porque siempre hay
    // uno resuelto — `NULL` en la columna es el perfil por defecto, no la ausencia de perfil.
    retirement_profile: &RetirementProfile,
    // `birth_date`: la del usuario CUYO perfil se está simulando (`session_user_id`), no la
    // «demografía resuelta» de la respuesta. Es lo que convierte `target_retirement_age` en un
    // mes del bucle; sin ella una estrategia por edad degrada a `asap` con aviso.
    birth_date: Option<NaiveDate>,
    overrides: Option<&SimOverrides>,
) -> Result<BuiltProjection, ApiError> {
    let (income_reg, income_retirement, expense_reg, expense_retirement, expense_end_entries) =
        ledger_regular_monthly_income_and_expense(pool, iid, session_user_id, view, today).await?;

    // Liabilities: una sola carga para todos los consumidores (debt service de modo A, caps de
    // assets.rs y el input del engine). En modo real (B/C) se les anula la cuota más abajo.
    // Solo pasivos ACTIVOS (mismo predicado que /v1/summary y /v1/liabilities): hasta la 3.4.0
    // esta query no filtraba y el principal de un pasivo ya vencido seguía restando net worth en
    // toda la serie — `projection.starting_net_worth` divergía de `summary.net_worth` (contra el
    // contrato D5/I5 de la arquitectura).
    let liab_scope = view.scope_where("");
    let liab_today_ph = view.next_arg_index();
    let liab_sql = format!(
        r#"SELECT id, label, principal, payment_amount, payment_frequency, payment_end_date,
                  apr_percent, repayment_model, min_payment_pct, min_payment_eur
           FROM liabilities
           WHERE {liab_scope}
             AND (payment_end_date IS NULL OR payment_end_date >= ${liab_today_ph} OR principal > 0)"#
    );
    let liabs: Vec<LiabEngineRow> = view
        .bind_scope_as(sqlx::query_as(&liab_sql), iid, session_user_id)
        .bind(today)
        .fetch_all(pool)
        .await?;

    // Fuente del ahorro efectiva: resuelta por la core compartida con `/v1/summary` (ver
    // `resolve_effective_savings_inputs`), de modo que el KPI y el gráfico no puedan divergir.
    let savings_source = fire_settings
        .map(|fs| fs.savings_source)
        .unwrap_or_default();
    let avg = if savings_source.uses_transactions() {
        let fs = fire_settings.expect("uses_transactions implies fire_settings present");
        Some(
            transactions_avg(
                pool,
                iid,
                session_user_id,
                view,
                today,
                fs.income_window(),
                fs.expense_window(),
            )
            .await?,
        )
    } else {
        None
    };
    let inputs = resolve_effective_savings_inputs(savings_source, income_reg, expense_reg, avg.as_ref());
    // Overrides what-if pre-target: el gasto extra entra ANTES de derivar el target FIRE y las
    // bases de caps (semántica «gasto real»); el gasto de jubilación explícito sustituye al
    // derivado. Sin overrides (`None`, todos los callers no-simulación) nada cambia.
    let ov = overrides.cloned().unwrap_or_default();
    let mut inputs = inputs;
    inputs.expense += ov.extra_monthly_expense;
    let mut expense_retirement = match ov.retirement_monthly_expense {
        Some(v) => v,
        None => expense_retirement + ov.extra_monthly_expense,
    };
    // Suelo cero para un recorte que se pasa de largo. Un gasto negativo actuaría como ingreso
    // extra en la caja del motor, dejaría techos de caps negativos (la regla se saltaría entera,
    // en silencio) y produciría un target FIRE y un runway sin sentido económico.
    //
    // GATEADO a que el override recorte, no incondicional: `inputs.expense` puede ser negativo
    // hoy sin ningún override —modo B con filas de gasto de signo invertido—, y un `.max(ZERO)`
    // a secas cambiaría `GET /v1/projection/series` y `GET /v1/summary`, no solo esta tool. El
    // test `baseline_without_overrides_matches_get_projection_and_scenario` es la prueba de que
    // no se ha escapado.
    //
    // Las dos bases se clampan por separado porque son magnitudes distintas: en modo A un recorte
    // puede anular una y dejar la otra en pie. Y `retirement_monthly_expense` explícito NO se
    // clampa: ya viene validado > 0 por el core.
    if ov.extra_monthly_expense < Decimal::ZERO {
        inputs.expense = inputs.expense.max(Decimal::ZERO);
        if ov.retirement_monthly_expense.is_none() {
            expense_retirement = expense_retirement.max(Decimal::ZERO);
        }
    }

    let expense_from_avg = inputs.expense_from_avg;
    let effective_savings_source = inputs.effective_source;
    let savings_income_basis = inputs.income_basis;
    let savings_expense_basis = inputs.expense_basis;

    // Modo real (B/C con datos), contrato 4.8.0 (#142, opción 3 firmada por el owner): la cuota
    // SALE del promedio y el plan de pago queda VIVO. El promedio real contiene las cuotas
    // pagadas; hasta 4.7.0 se anulaba el plan («resta constante»), lo que congelaba la deuda
    // para siempre y capitalizaba la cuota a perpetuidad en el objetivo. Ahora:
    //   gasto efectivo = promedio − Σ cuotas activas HOY  (estimación acotada: depende de que
    //   el promedio contenga ≈ la cuota; el error estructural de las alternativas era mayor),
    // y el motor cobra la cuota con su modelo, su devengo y su VENCIMIENTO — el paréntesis
    // literal del owner: «restar la cuota del pasivo a los gastos llegado su vencimiento». La
    // caja neta es idéntica a la de hoy mientras el plan vive (income − (avg−M) − M) y la cuota
    // vuelve sola al ahorro cuando el plan muere. Conteo ÚNICO: la base del objetivo nunca
    // lleva la cuota; la deuda entra solo por el término finito de `FireTarget`.
    if expense_from_avg {
        let active_quotas: Decimal = liabs
            .iter()
            .filter(|r| liability_is_active(r.payment_end_date, today))
            .map(|r| liability_monthly_payment(r.payment_amount, r.payment_frequency.as_deref()))
            .filter(|p| *p > Decimal::ZERO)
            .sum();
        // Suelo cero: si el promedio no llegaba a las cuotas, restar de más inventaría ahorro.
        inputs.expense = (inputs.expense - active_quotas).max(Decimal::ZERO);
    }

    // Servicio de deuda mensual de los pasivos activos (mismo filtro `payment_end_date` que las
    // lecturas SQL). No es un input del engine (que amortiza los pasivos por su cuenta): lo exporta
    // `BuiltProjection` para los caps `months_expense` de `assets.rs`. En modo real es 0 por el
    // bloque anterior.
    let debt_service_monthly: Decimal = liabs
        .iter()
        .filter(|r| liability_is_active(r.payment_end_date, today))
        .map(|r| liability_monthly_payment(r.payment_amount, r.payment_frequency.as_deref()))
        .filter(|p| *p > Decimal::ZERO)
        .sum();
    // Desde 4.8.0 (#142, opción 3) la cuota es servicio de deuda REAL en los tres modos: en B/C
    // el gasto efectivo ya la ha restado del promedio, así que publicarla aparte es contarla UNA
    // vez, no dos. El literal `included_in_real_expense` se retira con su modo.
    let debt_service_absent_reason: Option<&'static str> = None;

    let monthly_net_regular = inputs.income - inputs.expense;

    // Base del FIRE number: income = income efectivo (modo C → presupuesto; modo B → promedio real);
    // expense = gasto efectivo en modo B/C (el gasto ya no es el del presupuesto), o
    // `expense_retirement` en modo A (comportamiento histórico).
    // El `map` sigue colgando de `fire_settings`: es la presencia de configuración FIRE del
    // HOGAR lo que decide si hay objetivo que explicar. Los ingredientes personales (modo,
    // importe manual, SWR) salen del perfil desde 5.0.0.
    let fire_target_outcome = fire_settings.map(|_fs| {
        // El override explícito de gasto de jubilación ancla el target en TODOS los modos; sin
        // él, la base histórica (gasto efectivo en B/C, gasto de jubilación en A).
        let fire_expense = match ov.retirement_monthly_expense {
            Some(v) => v,
            None if inputs.expense_from_avg => inputs.expense,
            None => expense_retirement,
        };
        compute_fire_need(
            retirement_profile,
            inputs.income,
            income_retirement,
            fire_expense,
        )
    });
    // `None` cuando no hay `fire_settings` (no hay configuración FIRE que explicar), `Some(razón)`
    // cuando la hay y aun así no sale target.
    let fire_target_absent_reason = fire_target_outcome
        .as_ref()
        .and_then(|r| r.as_ref().err().copied());
    let mut fire_target = fire_target_outcome.and_then(|r| r.ok()).map(|need| {
        let fs = fire_settings.expect("need solo existe con fire_settings");
        FireTarget {
            need,
            swr_pct: retirement_profile.swr_pct,
            // La MISMA escala y el MISMO switch que el drenaje (#140).
            tax_brackets: fs.tax_brackets.clone(),
            taxes_enabled: fs.taxes_enabled,
            taxable_gain_ratio: fs.taxable_gain_ratio,
            // Sin clamp desde 4.9.0 (#146): el rango [−2, 50] lo garantiza la escritura, y una
            // inflación negativa DEBE llegar al engine (objetivo decreciente, no plano).
            annual_inflation_percent: inflation_annual_percent,
            // El término finito de deuda (#142) se rellena MÁS ABAJO, cuando los pasivos del
            // engine ya están construidos — necesita sus calendarios completos.
            debt_payments_remaining: Vec::new(),
        }
    });

    let assets_scope = view.scope_where("");
    let assets_sql = format!(
        r#"SELECT id, name, current_value, purchase_price, is_liquid,
                  expected_annual_return_percent, annual_volatility_percent
           FROM assets
           WHERE {assets_scope}
           ORDER BY sort_index ASC, name ASC, id ASC"#
    );
    let assets_rows: Vec<AssetEngineRow> = view
        .bind_scope_as(sqlx::query_as(&assets_sql), iid, session_user_id)
        .fetch_all(pool)
        .await?;

    let alloc_scope = view.scope_where("");
    let alloc_sql = format!(
        r#"SELECT id, target_asset_id, kind, amount, cap_kind, cap_value
           FROM allocation_rules
           WHERE {alloc_scope} AND enabled = true
           ORDER BY priority ASC, id ASC"#
    );
    let alloc_rows: Vec<AllocationRuleEngineRow> = view
        .bind_scope_as(sqlx::query_as(&alloc_sql), iid, session_user_id)
        .fetch_all(pool)
        .await?;

    let plan_scope = view.scope_where("p");
    let plan_sql = format!(
        r#"SELECT c.scope AS scope, p.expected_amount, p.amount_basis, p.due_date,
                  p.window_start_date, p.window_end_date, p.title
           FROM planning_flows p
           JOIN categories c ON c.id = p.category_id
           WHERE {plan_scope}"#
    );
    let planning_rows: Vec<PlanningFlowProjRow> = view
        .bind_scope_as(sqlx::query_as(&plan_sql), iid, session_user_id)
        .fetch_all(pool)
        .await?;

    let flow_adj =
        planning_monthly_cash_adjustments_from_flows(today, horizon_months, &planning_rows);
    // Modo B/C con datos: el gasto ya no viene del presupuesto → los ajustes por end-date de partidas
    // de presupuesto se anulan (los `planning_flows` son ortogonales y se mantienen).
    let end_adj = if expense_from_avg {
        vec![Decimal::ZERO; horizon_months as usize]
    } else {
        expense_end_date_monthly_adjustments(today, horizon_months, &expense_end_entries)
    };
    let planning_monthly_cash_adjustment: Vec<Decimal> = flow_adj
        .iter()
        .zip(end_adj.iter())
        .map(|(a, b)| a + b)
        .collect();

    let mut asset_id_name: Vec<(Uuid, String)> = Vec::with_capacity(assets_rows.len());
    // Se rellena en el MISMO `map` que `assets`: la alineación con `input.assets` es entonces una
    // propiedad de construcción, no una invariante que alguien deba recordar más tarde.
    let mut asset_volatility_percent: Vec<Option<Decimal>> = Vec::with_capacity(assets_rows.len());
    let assets: Vec<SimAsset> = assets_rows
        .into_iter()
        .map(|r| {
            asset_id_name.push((r.id, r.name));
            asset_volatility_percent.push(r.annual_volatility_percent);
            SimAsset {
                id: r.id,
                value: r.current_value,
                purchase_price: r.purchase_price,
                is_liquid: r.is_liquid,
                expected_annual_return_percent: r.expected_annual_return_percent,
            }
        })
        .collect();

    // Build allocation rules in priority order; resolve target_asset_id → index in assets[].
    let asset_index_by_id: HashMap<Uuid, usize> = assets
        .iter()
        .enumerate()
        .map(|(i, a)| (a.id, i))
        .collect();
    let mut allocation_rule_ids: Vec<Uuid> = Vec::with_capacity(alloc_rows.len());
    let allocation_rules: Vec<AllocationRule> = alloc_rows
        .into_iter()
        .filter_map(|r| {
            let target_index = *asset_index_by_id.get(&r.target_asset_id)?;
            let kind = match r.kind.as_str() {
                "fixed" => AllocationKind::Fixed,
                "percent" => AllocationKind::Percent,
                "remainder" => AllocationKind::Remainder,
                _ => return None,
            };
            let amount = r.amount;
            let cap = match (r.cap_kind.as_deref(), r.cap_value) {
                (Some("amount"), Some(v)) => Some(AllocationCap::Amount(v.max(Decimal::ZERO))),
                (Some("months_expense"), Some(v)) => {
                    Some(AllocationCap::MonthsExpense(v.max(Decimal::ZERO)))
                }
                (Some("income_multiple"), Some(v)) => {
                    Some(AllocationCap::IncomeMultiple(v.max(Decimal::ZERO)))
                }
                _ => None,
            };
            // El push va aquí, DESPUÉS de todos los `?`/`return None`: así el vector de ids
            // recibe exactamente las reglas que sobreviven al filtro, en el mismo orden.
            allocation_rule_ids.push(r.id);
            Some(AllocationRule {
                target_index,
                kind,
                amount,
                cap,
            })
        })
        .collect();

    let mut liability_id_label: Vec<(Uuid, String)> = Vec::with_capacity(liabs.len());
    let liabilities: Vec<ProjectionLiabilityInput> = liabs
        .into_iter()
        .map(|r| {
            liability_id_label.push((r.id, r.label.clone()));
            ProjectionLiabilityInput {
            principal: r.principal.max(Decimal::ZERO),
            monthly_payment: liability_monthly_payment(r.payment_amount, r.payment_frequency.as_deref()),
            payment_end: r.payment_end_date,
            // El CHECK de la columna acota el dominio a los cuatro literales, así que este
            // `parse` no puede fallar con datos escritos por la API. Ante lo imposible (una fila
            // manipulada a mano) se cae al default histórico en vez de propagar un error: la
            // proyección es una LECTURA y un pasivo con un literal corrupto debe degradar a los
            // números pre-4.2.0, no tumbar el chart entero. La validación de verdad vive en
            // `liabilities.rs`, que es por donde se escribe.
            repayment_model: LiabRepaymentModel::parse(&r.repayment_model)
                .map(LiabRepaymentModel::to_engine)
                .unwrap_or(RepaymentModel::FixedPayments),
            apr_percent: r.apr_percent,
            min_payment_pct: r.min_payment_pct,
            min_payment_eur: r.min_payment_eur,
            // Ejes what-if de `simulate_projection`: el ensamblado REAL nunca los pone. Los
            // aplica el override post-build sobre el input clonado del escenario, igual que
            // `asset_return_overrides` — una proyección de verdad no simula amortizaciones que
            // el usuario no ha hecho.
            extra_principal_monthly: Decimal::ZERO,
            extra_principal_lump_sums: Vec::new(),
            early_repayment_fee_pct: None,
            early_repayment_effect: Default::default(),
            }
        })
        .collect();

    // #142: el objetivo debe cubrir, además de la perpetuidad del gasto, cada euro de cuota que
    // quede por pagar (la base del cruce son los LÍQUIDOS BRUTOS de #143, que no restan ningún
    // principal). Serie por sufijos sobre el calendario COMPLETO — no depende del horizonte.
    if let Some(ft) = fire_target.as_mut() {
        ft.debt_payments_remaining =
            futurefin_engine::debt_payments_remaining_series(&liabilities, today);
    }

    // -----------------------------------------------------------------------------------------
    // Estrategia del perfil → `PhasePlan` (5.0.0 WP5-2b, §C del plan de #207)
    // -----------------------------------------------------------------------------------------
    // El perfil ENTERO se traduce aquí: trigger, fase parcial, pensión con fecha, base del
    // objetivo, tasa de descuento del puente y regla de retirada. Es el único punto del API donde
    // una decisión del usuario se convierte en un mes del bucle — y por eso todas las edades se
    // convierten con `months_until_target_age`, la misma aritmética civil que publica
    // `jubilacion_age`: derivarlas con una resta de años × 12 haría que la respuesta dijera «te
    // jubilas a los 54» habiendo pedido 55.
    //
    // **Sin fecha de nacimiento se degradan las PARTES dependientes de la edad**, nunca la
    // lectura entera: el trigger vuelve al cruce, la pensión y la media jornada no entran al
    // plan, y viaja un `birth_date_missing`. Un 500 por un campo opcional del perfil sería el
    // peor cambio posible en una LECTURA (§A del plan).
    let mut warnings: Vec<String> = Vec::new();
    fn push_warning(warnings: &mut Vec<String>, w: &str) {
        if !warnings.iter().any(|x| x == w) {
            warnings.push(w.to_string());
        }
    }

    // -----------------------------------------------------------------------------------------
    // QUÉ BLOQUES DEL PERFIL USA LA ESTRATEGIA — la puerta, y está SOLO aquí
    // -----------------------------------------------------------------------------------------
    // El perfil es ACUMULATIVO por diseño: conserva todos los bloques que el usuario llegó a
    // rellenar (`partial_retirement`, `target_retirement_age`, `pension`…) para que cambiar de
    // estrategia y volver no pierda nada, y `GET /v1/auth/me/retirement-profile` los sigue
    // devolviendo enteros. **Un bloque guardado no es, por tanto, una declaración de que esa fase
    // se viva**: quien la declara es la ESTRATEGIA. Aquí se decide qué entra al `PhasePlan`; el
    // perfil almacenado no se toca.
    //
    // INCIDENTE (verificado en vivo sobre la demo, imagen construida de `debc52d`): un perfil con
    // `strategy: "asap"` que conservaba una media jornada de una prueba anterior (empieza a los
    // 40, 1.000 €/mes) se simulaba CON esa fase. La serie llegaba con
    // `warnings: ["partial_phase_capital_shrinking"]`, con un `partial_retirement_month_index` y
    // sin cruzar nunca el objetivo — una fase que la jubilación rediseñada ni siquiera enseña
    // para esa estrategia (U2: los campos que la estrategia no usa no se muestran), actuando en
    // silencio sobre los números (U12: nada puede actuar sin decirlo).
    //
    // Vive en UN solo sitio a propósito: `GET /v1/projection/series`, `GET /v1/projection/bands`
    // (`projection_bands.rs`), el bucle por miembro de `?view=household`
    // (`run_member_projection`) y el `profile_overrides` de `simulate_projection` construyen
    // todos su input con esta función, así que heredan la regla sin repetirla.
    //
    // Lo que NO pasa por esta puerta, y es deliberado:
    //   · `pension` — D15/R6: la pensión con fecha es un ingreso que esa persona cobrará gobierne
    //     quien gobierne el trigger, y quien decide si además DIMENSIONA el objetivo es
    //     `target_basis`, no la estrategia. Fuera de `partial`, `fraction_while_partial` queda
    //     moot sola: sin fase parcial el bucle nunca entra en `Phase::Partial`.
    //   · `target_retirement_age` — su puerta es `wants_age_trigger`, aquí abajo: solo es trigger
    //     en `retire_at_age`/`coast`, y en `partial` solo como edad OPCIONAL de jubilación total.
    //     Con `asap` o `pension_bridge` no se lee, y por eso no filtraba.
    let plan_uses_partial_phase =
        matches!(retirement_profile.strategy, RetirementStrategy::Partial);

    // `R`: mes del BUCLE (1-based) en que el miembro cumple `target_retirement_age`. Lo piden las
    // dos estrategias por edad y, opcionalmente, `partial` (fin de la media jornada).
    let wants_age_trigger = retirement_profile.strategy.requires_target_age()
        || (plan_uses_partial_phase && retirement_profile.target_retirement_age.is_some());
    let forced_retirement_month: Option<u32> = if wants_age_trigger {
        match (birth_date, retirement_profile.target_retirement_age) {
            (Some(b), Some(age)) => {
                // `+ 1`: `months_until_target_age` devuelve el mes de la REJILLA publicada
                // (0 = hoy) y `RetirementTrigger::AtMonth` habla en meses del BUCLE (1-based).
                // El primer mes jubilado del bucle es el que cierra sobre esa casilla de la
                // rejilla, así que la respuesta publica exactamente `m` como
                // `jubilacion_month_index` (ver `engine_month_to_grid`).
                Some(months_until_target_age(today, b, age).saturating_add(1))
            }
            (None, _) => {
                push_warning(&mut warnings, WARN_BIRTH_DATE_MISSING);
                None
            }
            (Some(_), None) => {
                push_warning(&mut warnings, WARN_TARGET_AGE_MISSING);
                None
            }
        }
    } else {
        None
    };

    // Pensión CON FECHA (D3/D8). `start_index` es **0-based**, la rejilla del objetivo, no la
    // 1-based del bucle: la asimetría la declara §B.3 y `PensionSchedule` la documenta.
    let pension: Option<PensionSchedule> = match retirement_profile.pension.as_ref() {
        None => None,
        Some(p) => match birth_date {
            Some(b) => Some(PensionSchedule {
                start_index: months_until_target_age(today, b, p.starts_at_age),
                monthly_today: p.monthly_amount_today,
                indexed: p.indexed,
                fraction_while_partial: p.fraction_while_partial,
            }),
            None => {
                push_warning(&mut warnings, WARN_BIRTH_DATE_MISSING);
                None
            }
        },
    };

    // Media jornada (P7/D10). `start_month` es del BUCLE (1-based), como el trigger. El
    // `.filter()` es la puerta de arriba: con cualquier otra estrategia el bloque guardado se
    // queda en el perfil y NO entra al motor.
    let partial: Option<PartialPhase> = match retirement_profile
        .partial_retirement
        .as_ref()
        .filter(|_| plan_uses_partial_phase)
    {
        None => None,
        Some(x) => match birth_date {
            Some(b) => Some(PartialPhase {
                start_month: months_until_target_age(today, b, x.starts_at_age).saturating_add(1),
                income_monthly: x.income_monthly_today,
                expense_basis: match x.expense_basis {
                    PartialExpenseBasis::Retirement => EngineExpenseBasis::Retirement,
                    PartialExpenseBasis::Regular => EngineExpenseBasis::Regular,
                },
            }),
            None => {
                push_warning(&mut warnings, WARN_BIRTH_DATE_MISSING);
                None
            }
        },
    };

    // Base del objetivo. `resolve_retirement_profile` ya la derivó (R6), así que aquí nunca es
    // `None`; el `unwrap_or_default` es la degradación de un perfil escrito fuera de la API.
    let target_basis = match retirement_profile.target_basis.unwrap_or_default() {
        ProfileTargetBasis::Perpetuity => EngineTargetBasis::Perpetuity,
        ProfileTargetBasis::BridgeToPension => EngineTargetBasis::BridgeToPension,
    };
    // El puente solo DESCUENTA cuando de verdad hay puente que descontar: base puente + pensión
    // con fecha resuelta. Sin las dos cosas la tasa no se lee, y publicarla sería invitar a
    // leerla (el mismo criterio que `debt_service_absent_reason`).
    let bridge_active = target_basis == EngineTargetBasis::BridgeToPension && pension.is_some();
    let bridge_discount_annual_pct = match retirement_profile.bridge_discount_basis {
        BridgeDiscountBasis::None => Decimal::ZERO,
        BridgeDiscountBasis::Swr => retirement_profile.swr_pct,
        BridgeDiscountBasis::ExpectedReturn => {
            match liquid_expected_return_pct(&assets) {
                // **Cota inferior 0** (pase de correcciones de la revisión adversarial). La
                // rentabilidad esperada por activo solo está acotada por `> −100`, así que una
                // cartera líquida con pérdidas esperadas produce un descuento negativo: el
                // objetivo puente se volvería MAYOR que la suma llana de sus flujos —lo contrario
                // de lo que descontar significa— y, pasado un umbral que depende de los meses
                // hasta la pensión, la tabla desborda `Decimal` y el motor devuelve
                // `BridgeDiscountOverflow`. Se sube a 0 (puente sin descuento, la lectura
                // conservadora) y se dice: `bridge_discount_clamped`.
                Some(r) if r < Decimal::ZERO => {
                    if bridge_active {
                        push_warning(&mut warnings, WARN_BRIDGE_DISCOUNT_CLAMPED);
                    }
                    Decimal::ZERO
                }
                Some(r) => r,
                None => {
                    if bridge_active {
                        push_warning(&mut warnings, WARN_BRIDGE_DISCOUNT_NO_LIQUID_ASSETS);
                    }
                    Decimal::ZERO
                }
            }
        }
    };

    let mut phase_plan = match forced_retirement_month {
        Some(r) => PhasePlan::forced_at(
            r,
            income_retirement,
            expense_retirement,
            // La retirada extra sigue siendo 0: el mecanismo de drenaje es la caída de ingresos,
            // no un importe suelto (el campo existe por el pin `P10_jubilacion_forzada`).
            Decimal::ZERO,
        ),
        None => PhasePlan::classic(income_retirement, expense_retirement),
    };
    phase_plan.withdrawal = withdrawal_rule_to_engine(
        &retirement_profile.withdrawal_rule,
        retirement_profile.swr_pct,
    );
    phase_plan.spend_mode = match retirement_profile.withdrawal_rule.spend_mode {
        ProfileSpendMode::Ceiling => EngineSpendMode::Ceiling,
        ProfileSpendMode::RuleIsSpend => EngineSpendMode::RuleIsSpend,
    };
    phase_plan.partial = partial;
    phase_plan.pension = pension;
    phase_plan.target_basis = target_basis;
    phase_plan.bridge_discount_annual_pct = bridge_discount_annual_pct;
    // **D17 — un solo trigger por simulación.** Con una estrategia por edad el objetivo SIGUE
    // entrando al motor (lo necesita el chart, el infra-financiado y los solves), pero el cruce
    // deja de jubilar: solo se anota como `liquid_crossing_month_index`. Hasta WP5-2a el handler
    // conseguía lo mismo pasando `fire_target: None` y recalculando el cruce por su cuenta con
    // `fire_target_at_month_index` — una segunda lectura del objetivo que, con base puente, NO
    // es el objetivo que el motor evalúa. Ese atajo muere aquí.
    phase_plan.crossing_is_reading_only = forced_retirement_month.is_some();
    let retirement_trigger = if forced_retirement_month.is_some() {
        RETIREMENT_TRIGGER_AGE
    } else {
        RETIREMENT_TRIGGER_CROSSING
    };
    // El objetivo que ENTRA al motor y el que se LEE son ya el MISMO objeto: `fire_target_reading`
    // sobrevive porque las lecturas (base de hoy, nominal del mes de jubilación, razón de
    // ausencia) lo consultan fuera del `ProjectionInput`, no porque difieran.
    let fire_target_reading = fire_target.clone();
    let engine_fire_target = fire_target;

    let input = ProjectionInput {
        ref_date: today,
        horizon_months,
        // #139: el MISMO supuesto efectivo que el target (sin clamp desde #146) — indexa el
        // gasto del bucle aunque no haya objetivo FIRE configurado.
        annual_inflation_percent: inflation_annual_percent,
        // #140 fase 1: la MISMA escala y el MISMO switch que el objetivo — el drenaje bruto y
        // el target se dimensionan con una sola fiscalidad. Sin fire_settings: sin impuesto.
        tax_brackets: fire_settings.map(|f| f.tax_brackets.clone()).unwrap_or_default(),
        taxes_enabled: fire_settings.map(|f| f.taxes_enabled).unwrap_or(false),
        taxable_gain_ratio: fire_settings
            .map(|f| f.taxable_gain_ratio)
            .unwrap_or(Decimal::ONE),
        income_regular_monthly: inputs.income,
        expense_regular_monthly: inputs.expense,
        assets,
        allocation_rules,
        liabilities,
        planning_monthly_cash_adjustment,
        phase_plan,
        // D17 — **un solo trigger por simulación**: con una estrategia por edad el motor NO
        // recibe objetivo, así que el cruce no puede dispararlo. El objetivo sigue existiendo
        // como LECTURA en `fire_target_reading` (línea del chart, `jubilacion_target_net_worth`,
        // `liquid_crossing_month_index`), calculada por el handler con `fire_crossover_month`.
        fire_target: engine_fire_target,
    };

    // **El colchón de caja** (V6/P2): se deriva del tope de la regla de ahorro salvo que el
    // perfil declare uno explícito. Va DESPUÉS de `input` porque necesita los activos y las
    // reglas ya resueltos a índices — o sea exactamente lo que la cascada ejecuta— y el gasto de
    // jubilación con el que se calcula su equivalente informativo en meses.
    let cash_buffer = crate::handlers::cash_buffer::resolve_cash_buffer(
        retirement_profile.cash_buffer_months,
        &input.assets,
        &asset_volatility_percent,
        &asset_id_name,
        &input.allocation_rules,
        &allocation_rule_ids,
        input.income_regular_monthly,
        input.expense_regular_monthly + debt_service_monthly,
        expense_retirement,
    );

    Ok(BuiltProjection {
        input,
        monthly_net_regular,
        allocation_rule_ids,
        asset_id_name,
        asset_volatility_percent,
        liability_id_label,
        planning_rows,
        effective_savings_source,
        savings_income_basis,
        savings_expense_basis,
        fire_target_absent_reason,
        debt_service_monthly,
        debt_service_absent_reason,
        fire_target_reading,
        retirement_trigger,
        warnings,
        forced_retirement_month,
        bridge_discount_annual_pct: bridge_active.then_some(bridge_discount_annual_pct),
        cash_buffer,
    })
}

/// Todo lo que `assets.rs` necesita del motor para una vista, con **un solo**
/// `build_installation_projection_input` (horizonte 1 mes: basta para el reparto del mes 1 y para
/// los escalares del mes 0).
pub(crate) struct AssetsProjectionContext {
    /// Asset id → € nominales encaminados en el mes 1 (fixed escalado + parte del remanente).
    /// **Incluye el tramo de planning flows del mes en curso** — desde #126 anclado al mes civil,
    /// así que es idéntico se consulte el día que se consulte (antes variaba día a día).
    pub nominals: HashMap<Uuid, Decimal>,
    /// Asset id → € que la MISMA cascada encamina sobre el neto **recurrente**
    /// (`income − expense − debt_service`, sin el tramo de planning). Estable y reproducible.
    pub recurring_nominals: HashMap<Uuid, Decimal>,
    /// Income mensual **efectivo** del mes 0 (presupuesto en modo A/fallback; promedio real en B).
    pub income_monthly: Decimal,
    /// Gasto mensual **efectivo** + servicio de deuda de los pasivos activos.
    pub expense_with_debt: Decimal,
}

/// Contexto de proyección para el listado/edición de activos: aportación del primer mes por activo
/// más los escalares que resuelven los caps de las reglas de asignación.
///
/// Los caps `months_expense` / `income_multiple` se resuelven con **los mismos escalares que usa el
/// engine** (fix B4): en modo B/C con datos el ahorro sale del promedio real 12m, así que antes —
/// cuando estos escalares venían siempre del presupuesto — el objetivo mostrado en Activos no
/// casaba ni con la aportación del mes 1 ni con la simulación. El resto de campos de `fire_settings`
/// (p.ej. `fire_target`) no altera ni las aportaciones del mes 1 ni los escalares.
pub(crate) async fn assets_projection_context(
    pool: &sqlx::PgPool,
    iid: Uuid,
    session_user_id: Uuid,
    view: LedgerView,
    today: NaiveDate,
) -> Result<AssetsProjectionContext, ApiError> {
    let fire_settings = load_fire_settings(pool, iid).await?;
    // Los caps y el reparto del mes 1 no miran el objetivo, pero el ensamblado sí necesita un
    // perfil: se pasa el del solicitante, que es de quien son las filas del scope `mine`.
    let retirement_profile =
        crate::handlers::retirement_profile::load_retirement_profile(pool, session_user_id).await?;
    // La DOB entra porque el plan de fases la necesita para las estrategias por edad: sin ella,
    // el mes 1 de un `retire_at_age` ya cumplido se repartiría como si el usuario siguiera
    // trabajando, y la aportación que muestra Activos no sería la que simula el chart.
    let birth_date: Option<NaiveDate> =
        sqlx::query_scalar(r#"SELECT birth_date FROM users WHERE id = $1"#)
            .bind(session_user_id)
            .fetch_one(pool)
            .await?;
    let built = build_installation_projection_input(
        pool,
        iid,
        session_user_id,
        view,
        today,
        1,
        Decimal::ZERO,
        Some(&fire_settings),
        &retirement_profile,
        birth_date,
        None,
    )
    .await?;
    let nominals_vec =
        first_month_per_asset_contribution_nominals(&built.input).map_err(map_engine_err)?;
    let nominals: HashMap<Uuid, Decimal> = built
        .input
        .assets
        .iter()
        .zip(nominals_vec.into_iter())
        .map(|(a, n)| (a.id, n))
        .collect();

    // Segunda pasada de la MISMA cascada sobre el neto recurrente: se anula el ajuste de planning
    // del mes en curso y se vuelve a repartir. Reutilizar el engine (en vez de aproximar el
    // reparto a mano) es lo que garantiza que los caps y la precedencia sean idénticos; el coste
    // son microsegundos de aritmética Decimal, sin ningún SELECT extra.
    let mut recurring_input = built.input.clone();
    if let Some(first) = recurring_input.planning_monthly_cash_adjustment.first_mut() {
        *first = Decimal::ZERO;
    }
    let recurring_vec = first_month_per_asset_contribution_nominals(&recurring_input)
        .map_err(map_engine_err)?;
    let recurring_nominals: HashMap<Uuid, Decimal> = built
        .input
        .assets
        .iter()
        .zip(recurring_vec.into_iter())
        .map(|(a, n)| (a.id, n))
        .collect();

    Ok(AssetsProjectionContext {
        nominals,
        recurring_nominals,
        income_monthly: built.input.income_regular_monthly,
        // Misma semántica que el helper anterior (`expense_reg + debt_service`), pero con la base de
        // gasto EFECTIVA en vez de la del presupuesto.
        expense_with_debt: built.input.expense_regular_monthly + built.debt_service_monthly,
    })
}

#[utoipa::path(
    get,
    path = "/v1/projection/series",
    tag = "projection",
    params(
        ("view" = Option<String>, Query, description = "`mine` (default desde 5.0.0: `view` omitido o vacío) = una simulación con el perfil, la fecha de nacimiento y las filas del solicitante. `household` = AGREGADO de una simulación por miembro (suma de series; `jubilacion_*` y `fire_target_series` vacíos, detalle por persona en `members[]`). Cualquier otro valor → 400 `invalid_view`."),
        ("months" = Option<u32>, Query, description = "Horizonte en meses (12–840; fuera de rango → 400 `months_out_of_range`); omitir = horizonte derivado (`lifespan_age` | `fallback_no_demographics`), ver `horizon_basis` + `horizon_lifespan_age` en la respuesta"),
        ("density" = Option<String>, Query, description = "`monthly` (default) | `hybrid` (mensual el primer año, anual después). Cualquier otro valor → 400 `invalid_density`"),
    ),
    responses(
        (status = 200, description = "Serie mensual motor dossier", body = ProjectionSeriesResponse),
        (status = 401, description = "No valid session"),
        (status = 403, description = "Not an installation member"),
    )
)]
pub async fn get_projection_series(
    Extension(state): Extension<Arc<AppState>>,
    jar: CookieJar,
    Query(q): Query<ProjectionSeriesQuery>,
) -> Result<Json<ProjectionSeriesResponse>, ApiError> {
    let user = require_session_user(&jar, &state.pool).await?;
    let (iid, _) = require_installation_member(&state.pool, user.id.0).await?;

    // Parseo compartido con el resto del ledger: este handler tenía su propia copia del `match`
    // y por eso se le pasó por alto el rechazo de `view` desconocido. Un solo parser, un solo sitio.
    let view = crate::handlers::person_view::LedgerViewQuery {
        view: q.view.clone(),
    }
    .resolve()?;
    let density = resolve_density(&q)?;
    let response =
        projection_series_cached(&state, user.id.0, iid, view, q.months, density).await?;
    Ok(Json(response))
}

/// Core sin HTTP con la política de cache intacta: lo comparten el handler GET y la
/// tool MCP `get_projection`. Cache hot path solo sin `months_override` (caso por
/// defecto y 99% del tráfico); con horizonte custom se recomputa sin cachear.
pub(crate) async fn projection_series_cached(
    state: &AppState,
    user_id: Uuid,
    iid: Uuid,
    view: LedgerView,
    months_override: Option<u32>,
    density: Density,
) -> Result<ProjectionSeriesResponse, ApiError> {
    if months_override.is_none() {
        // `owner_user_id` va también en `household`: la respuesta lleva demografía
        // del solicitante (`viewer_birth_date`, horizonte, `jubilacion_age`), así que
        // una entrada household compartida servía los datos de un miembro a otro.
        let key = ProjectionCacheKey {
            installation_id: iid,
            view,
            owner_user_id: Some(user_id),
            density,
        };
        if let Some(cached) = state.projection_cache_get(&key).await {
            tracing::info!(installation_id = %iid, view = ?view, density = ?density, "projection cache HIT");
            return Ok((*cached).clone());
        }
        tracing::info!(installation_id = %iid, view = ?view, density = ?density, "projection cache MISS, computing");
        let t0 = std::time::Instant::now();
        let response =
            compute_projection_series_response(state, user_id, iid, view, None, density).await?;
        tracing::info!(
            installation_id = %iid,
            density = ?density,
            ms = t0.elapsed().as_millis() as u64,
            "projection compute done, inserting in cache"
        );
        state
            .projection_cache_insert(key, Arc::new(response.clone()))
            .await;
        return Ok(response);
    }

    compute_projection_series_response(state, user_id, iid, view, months_override, density).await
}

/// Calcula la respuesta de proyección sin tocar el cache. Es la unidad de
/// recompute reusada por: (a) cache miss en el handler, (b) warm-up post-login,
/// (c) warm-up post-mutación. `density` solo afecta a la serialización (qué
/// puntos incluir en `points`/`fire_target_series`/`asset_series.values`);
/// el compute interno del engine siempre es el horizonte mensual completo.
/// Cotas del horizonte EXPLÍCITO (`GET /v1/projection/series?months=`, tool `get_projection.months`
/// y `simulate_projection.months`). Son las mismas que declara el JSON Schema de las tools.
pub(crate) const MIN_PROJECTION_MONTHS: u32 = 12;
pub(crate) const MAX_PROJECTION_MONTHS: u32 = 840;

/// Un `months` fuera de rango se **rechaza**, no se clampa (4.4.0).
///
/// Hasta 4.3.1 `resolve_projection_context` hacía `m.clamp(12, 840)` y devolvía 200 con
/// `horizon_basis: "months_override"` — o sea: la respuesta afirmaba «te he hecho caso» mientras
/// contestaba a una pregunta distinta de la que se hizo. Pedir 1.200 meses y recibir 840 rotulados
/// como override es indistinguible de un horizonte de 1.200 para quien lee la respuesta, y las dos
/// tools hermanas discrepaban sobre el MISMO valor: `simulate_projection` ya rechazaba (aquí
/// abajo) y `get_projection` clampaba. El esquema declara `range(min = 12, max = 840)`; esto es
/// simplemente cumplirlo.
///
/// **Es breaking**: un cliente que enviaba 1.200 pasa de 200 a 400. La SPA no manda nunca `months`
/// (`projectionSeriesUrl` en `apps/web/src/App.tsx` solo compone `view` y `density`), así que la
/// ruptura es real solo para clientes de API/MCP — y exactamente sobre los valores que el esquema
/// ya declaraba inválidos.
pub(crate) fn validate_months_override(months: Option<u32>) -> Result<(), ApiError> {
    if let Some(m) = months {
        if !(MIN_PROJECTION_MONTHS..=MAX_PROJECTION_MONTHS).contains(&m) {
            return Err(ApiError::BadRequest(format!(
                "months_out_of_range: months must be between {MIN_PROJECTION_MONTHS} and {MAX_PROJECTION_MONTHS}"
            )));
        }
    }
    Ok(())
}

/// Contexto resuelto de una proyección: `today` en la TZ de la instalación, inflación efectiva,
/// `show_age_mode`, `fire_settings` resueltos, DOB para demografía y la regla de horizonte.
/// Extraído de `compute_projection_series_response` para que la tool MCP `simulate_projection`
/// resuelva EXACTAMENTE el mismo contexto (mismas queries, misma regla de clamp) sin duplicarlo.
pub(crate) struct ProjectionContext {
    pub today: NaiveDate,
    pub inflation_annual_percent: Decimal,
    pub show_age_mode: String,
    pub fire_settings: FireSettings,
    /// Perfil de jubilación del usuario de la SESIÓN, ya resuelto (5.0.0, D13). De aquí salen
    /// el modo del objetivo, el importe manual, el SWR y la edad límite del horizonte — los
    /// cuatro ejes que hasta 4.15.x vivían en `fire_settings`.
    ///
    /// Semántica de UN miembro por ahora: también en `?view=household` es el perfil del
    /// solicitante, igual que la demografía. WP5 corre una simulación por miembro.
    pub retirement_profile: RetirementProfile,
    /// DOB de demografía: la del usuario de la sesión, o la del primer miembro del hogar.
    pub birth_date: Option<NaiveDate>,
    /// DOB **del usuario de la sesión**, sin el fallback demográfico de arriba. Es la única que
    /// puede convertir `target_retirement_age` en un mes: heredar la fecha de otra persona para
    /// decidir CUÁNDO se jubila el solicitante sería inventarle una edad. Sin ella, las
    /// estrategias por edad degradan a `asap` con `warnings: ["birth_date_missing"]`.
    pub session_birth_date: Option<NaiveDate>,
    pub months: u32,
    pub horizon_basis: String,
}

/// Resuelve el [`ProjectionContext`]. 1 query consolidada a `installation` (calendar_tz +
/// inflación + show_age_mode + fire_settings) y las DOB del usuario y del primer miembro del
/// hogar en paralelo con `try_join!`.
pub(crate) async fn resolve_projection_context(
    pool: &sqlx::PgPool,
    iid: Uuid,
    user_id: Uuid,
    months_override: Option<u32>,
) -> Result<ProjectionContext, ApiError> {
    type InstallationRow = (
        String, // calendar_tz
        Decimal,
        String,
        Option<sqlx::types::Json<FireSettings>>,
    );
    let inst_q = sqlx::query_as::<_, InstallationRow>(
        r#"SELECT calendar_tz,
                  annual_inflation_assumption_percent,
                  show_age_mode,
                  fire_settings
           FROM installation WHERE id = $1"#,
    )
    .bind(iid)
    .fetch_one(pool);
    // La DOB y el perfil salen de la MISMA fila: pedirlos por separado añadía un viaje para
    // leer dos columnas de la misma tabla por la misma clave.
    type SessionUserRow = (
        Option<NaiveDate>,
        Option<sqlx::types::Json<RetirementProfile>>,
    );
    let session_user_q = sqlx::query_as::<_, SessionUserRow>(
        r#"SELECT birth_date, retirement_profile FROM users WHERE id = $1"#,
    )
    .bind(user_id)
    .fetch_one(pool);
    let household_birth_q = sqlx::query_scalar::<_, NaiveDate>(
        r#"SELECT birth_date FROM persons
           WHERE installation_id = $1 AND birth_date IS NOT NULL
           ORDER BY is_primary DESC, sort_index ASC
           LIMIT 1"#,
    )
    .bind(iid)
    .fetch_optional(pool);

    let (inst_row, session_user_row, household_member_birth) =
        tokio::try_join!(inst_q, session_user_q, household_birth_q)?;
    let (session_birth, stored_profile) = session_user_row;
    let retirement_profile = resolve_retirement_profile(stored_profile.map(|j| j.0));

    let today = naive_date_in_calendar_tz(&inst_row.0)?;
    // Sin clamp desde 4.9.0 (#146): rango garantizado por la validación de escritura.
    let inflation_annual_percent = inst_row.1;
    let show_age_mode = inst_row.2;
    let fire_settings = resolve_fire_settings(inst_row.3.map(|j| j.0));

    let birth_date = session_birth.or(household_member_birth);
    let birth_dates: Vec<Option<NaiveDate>> = vec![birth_date];

    let (months, horizon_basis): (u32, String) = match months_override {
        // Rechazo, no clamp: ver `validate_months_override`. Se comprueba también aquí (y no solo
        // en los extractores) porque ESTA es la core compartida por HTTP, `get_projection` y
        // `simulate_projection`: una cota que viva en el borde vuelve a divergir entre superficies.
        Some(m) => {
            validate_months_override(Some(m))?;
            (m, "months_override".into())
        }
        None => {
            let (m, b) = projection_horizon_months(
                today,
                &birth_dates,
                retirement_profile.horizon_lifespan_age,
            );
            (m, b.into())
        }
    };

    Ok(ProjectionContext {
        today,
        inflation_annual_percent,
        show_age_mode,
        fire_settings,
        retirement_profile,
        birth_date,
        session_birth_date: session_birth,
        months,
        horizon_basis,
    })
}

/// Razones de ausencia ESTRUCTURAL de 5.0.0 — las que no dependen de los datos sino de la forma
/// de la pregunta. Literales cerrados, compartidos por la serie y por el MCP.
pub(crate) const ABSENT_HOUSEHOLD_AGGREGATE: &str = "household_aggregate";
pub(crate) const ABSENT_NO_FIRE_TARGET: &str = "no_fire_target";
pub(crate) const ABSENT_NO_RETIREMENT_TRIGGER: &str = "no_retirement_trigger";

/// Una simulación COMPLETA de un miembro: el ensamblado, la salida del motor y las dos lecturas
/// derivadas que hay que calcular junto a ella (amortización negativa y marcador de interés
/// compuesto). Es la unidad que `view=mine` produce una vez y `view=household` produce N veces.
/// **Los solves de §B.7 de UNA simulación**, calculados junto a la serie y guardados con ella en
/// la entrada de cache (M4): el primer GET tras una invalidación los paga, los HITs no pagan nada.
///
/// Cada solve es una bisección **sobre el motor entero** (`crates/engine/src/solve.rs`), o sea
/// hasta 26 proyecciones completas — el orden de magnitud es ~0,4 s a 600 meses. Por eso se
/// calculan **solo cuando la estrategia los necesita** y nunca en el camino de `simulate`, que es
/// cache-neutral por diseño (allí se piden con `solve: {…}`, explícitamente).
struct StrategySolves {
    /// `retire_at_age` y `partial` con edad de fin: la aportación mensual mínima que llega a
    /// `T(R−1)`, su techo de búsqueda, el rojo de D17 y la serie de capital necesario.
    required: Option<SolveResult>,
    /// `coast`: el mes a partir del cual se puede dejar de aportar, su número y su serie.
    coast: Option<CoastSolve>,
    /// **Sobrante del mes 1 (R5)**, clampado a ≥ 0: `recurring_net` del primer paso real del
    /// motor (`first_month_allocation`), es decir ingreso − gasto − servicio de deuda REALMENTE
    /// pagado. Es el mismo número que `solve.rs` usa como SUELO de su techo de búsqueda, y el
    /// que `coast` publica como margen disponible.
    monthly_headroom: Decimal,
}

struct MemberRun {
    user_id: Uuid,
    username: String,
    profile: RetirementProfile,
    birth_date: Option<NaiveDate>,
    /// Horizonte PROPIO del miembro (el que tendría en `view=mine`), no el común del hogar con
    /// el que se ha simulado. Se calcula donde se conoce su perfil y su DOB, y viaja hasta
    /// `members[].horizon_months`.
    own_horizon_months: u32,
    built: BuiltProjection,
    output: futurefin_engine::ProjectionOutput,
    negative_amortization: Vec<LiabilityNegativeAmortization>,
    compound_month: Option<u32>,
    solves: StrategySolves,
}

/// Fila de miembro del hogar tal y como sale de la BD (id, nombre, DOB y perfil sin resolver).
#[derive(Debug, FromRow)]
struct HouseholdMemberRow {
    user_id: Uuid,
    username: String,
    birth_date: Option<NaiveDate>,
    retirement_profile: Option<sqlx::types::Json<RetirementProfile>>,
}

/// Miembros de la instalación, cada uno con su fecha de nacimiento y su perfil de jubilación.
///
/// **Solo los que tienen fila en `installation_memberships`** (D9): un usuario registrado y
/// pendiente de aprobación no es del hogar, y sumar su patrimonio al agregado sería enseñar
/// dinero de alguien a quien todavía no se ha dado acceso. El orden es estable (owner primero,
/// luego por nombre) para que dos peticiones idénticas devuelvan `members[]` y `asset_series` en
/// el mismo orden.
async fn household_members(
    pool: &sqlx::PgPool,
    iid: Uuid,
) -> Result<Vec<HouseholdMemberRow>, ApiError> {
    let rows: Vec<HouseholdMemberRow> = sqlx::query_as(
        r#"SELECT m.user_id, u.username, u.birth_date, u.retirement_profile
           FROM installation_memberships m
           JOIN users u ON u.id = m.user_id
           WHERE m.installation_id = $1
           ORDER BY
               CASE m.role WHEN 'owner' THEN 0 WHEN 'member' THEN 1 ELSE 2 END,
               u.username, m.user_id"#,
    )
    .bind(iid)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

/// Simula a UN miembro con SUS filas (`LedgerView::Mine` atado a su id), SU perfil y SU fecha de
/// nacimiento.
///
/// `member_id` es deliberadamente independiente del usuario de la sesión: en `view=household` el
/// solicitante corre N simulaciones que no son la suya. Ese es todo el cambio de fondo de D9 —
/// el resto es aritmética de sumas.
#[allow(clippy::too_many_arguments)]
async fn run_member_projection(
    state: &AppState,
    iid: Uuid,
    member_id: Uuid,
    username: String,
    profile: RetirementProfile,
    birth_date: Option<NaiveDate>,
    today: NaiveDate,
    months: u32,
    own_horizon_months: u32,
    inflation_annual_percent: Decimal,
    fire_settings: &FireSettings,
) -> Result<MemberRun, ApiError> {
    let built = build_installation_projection_input(
        &state.pool,
        iid,
        member_id,
        LedgerView::Mine,
        today,
        months,
        inflation_annual_percent,
        Some(fire_settings),
        &profile,
        birth_date,
        None,
    )
    .await?;

    // Las dos simulaciones (principal + marker «compound supera ahorro») son CPU-bound y se
    // ejecutan en el pool blocking de Tokio. `tokio::join!` arranca ambas en paralelo, así que
    // un horizonte de 70 años con N activos no bloquea el reactor y aprovecha 2 cores.
    //
    // Desde la Fase 3 van bajo el techo de `heavy::run_projection_sim` (el tercer semáforo, junto
    // al KDF y al cripto del backup): sin él el límite efectivo eran los 512 hilos del pool de
    // blocking de Tokio, y N proyecciones concurrentes —trivial de provocar con `?months=`, que
    // salta la cache por diseño— dejaban sin CPU al reactor hasta tumbar `/v1/ready`. El permiso
    // envuelve SOLO la simulación: un HIT de cache no pasa por aquí y no espera a nadie.
    //
    // 5.0.0: en `household` esto se repite POR MIEMBRO, en serie. La concurrencia máxima sigue
    // siendo 2 permisos —la misma que en 4.15.x—; lo que crece es el tiempo total, y por eso el
    // agregado se cachea igual que cualquier otra entrada.
    let main_input = built.input.clone();
    let marker_input = built.input.clone();
    let assumption = built.monthly_net_regular;
    let (main_join, marker_join) = tokio::join!(
        crate::heavy::run_projection_sim("projection", move || {
            let output = project_net_worth_series(&main_input)?;
            // #119: amortización negativa por pasivo, con la ÚNICA recurrencia del crate
            // (liability_amortization_schedule) — nunca comparando cierres del bucle, que ya
            // llevan restada la amortización extra what-if. `Some((opening, final))` ⟺ algún
            // mes con `principal_repaid < 0`. Corre dentro del mismo permiso de CPU.
            let neg_am: Vec<Option<(Decimal, Decimal)>> = main_input
                .liabilities
                .iter()
                .map(|l| {
                    let s = futurefin_engine::liability_amortization_schedule(
                        l,
                        main_input.ref_date,
                        main_input.horizon_months,
                    );
                    s.months
                        .iter()
                        .any(|m| m.principal_repaid < Decimal::ZERO)
                        .then_some((s.opening_principal, s.final_principal))
                })
                .collect();
            Ok::<_, futurefin_engine::EngineError>((output, neg_am))
        }),
        crate::heavy::run_projection_sim("compound marker", move || {
            compound_outpaces_true_savings_month(&marker_input, assumption)
        }),
    );
    let (output, negative_amortization_flags) = main_join?.map_err(map_engine_err)?;
    let negative_amortization: Vec<LiabilityNegativeAmortization> = built
        .liability_id_label
        .iter()
        .zip(negative_amortization_flags.iter())
        .filter_map(|((id, label), flag)| {
            flag.map(|(opening, final_p)| LiabilityNegativeAmortization {
                liability_id: *id,
                label: label.clone(),
                opening_principal: money_out(opening),
                final_principal: money_out(final_p),
                horizon_months: months,
            })
        })
        .collect();
    let compound_month = marker_join?.map_err(map_engine_err)?;

    // §B.7 — los solves de la estrategia, en el MISMO permiso de CPU y DESPUÉS de la serie (no en
    // paralelo con ella: son hasta 26 proyecciones y competirían con el reactor por los dos
    // permisos del semáforo). Solo corren cuando la estrategia los necesita, así que `asap` y
    // `pension_bridge` —las dos por cruce— no pagan un céntimo.
    let solve_input = built.input.clone();
    let solve_month = built.forced_retirement_month;
    let strategy = profile.strategy;
    let solves = crate::heavy::run_projection_sim("strategy solves", move || {
        compute_strategy_solves(&solve_input, solve_month, strategy)
    })
    .await?
    .map_err(map_engine_err)?;

    Ok(MemberRun {
        user_id: member_id,
        username,
        profile,
        birth_date,
        own_horizon_months,
        built,
        output,
        negative_amortization,
        compound_month,
        solves,
    })
}

/// Suma posición a posición dos series del mismo largo. Los miembros comparten horizonte (§D:
/// `household_max_lifespan`), así que las longitudes coinciden por construcción; el `zip` deja
/// fuera cualquier cola sobrante en vez de indexar a ciegas.
fn add_series_into(acc: &mut Vec<Decimal>, add: &[Decimal]) {
    if acc.is_empty() {
        acc.extend_from_slice(add);
        return;
    }
    for (a, b) in acc.iter_mut().zip(add.iter()) {
        *a += *b;
    }
}

/// Serie de una salida del motor, con `0` donde el índice no existe (el mes 0 de las series de
/// retirada, que el motor sí rellena, o una cola más corta de la que se espera).
fn series_at(serie: &[Decimal], i: usize) -> Decimal {
    serie.get(i).copied().unwrap_or(Decimal::ZERO)
}

/// **Margen mensual disponible de un miembro** (D16/D31), con la base que corresponde a SU
/// estrategia. Vive en una función y no repetido en dos sitios porque el agregado del hogar
/// publica exactamente el mismo número por miembro que la vista `mine` publica para uno solo.
///
/// - Solve de aportación (`retire_at_age`, `partial` con edad): `techo − aportación`, clampado a
///   ≥ 0. El techo es el máximo sobrante mensual del horizonte, que es contra lo que el solve
///   buscó; restar el sobrante del mes 1 daría un margen negativo en cuanto la caja crezca.
/// - `coast`: el sobrante del mes 1 (R5) **si el mes coast ya llegó**, y 0 si no. Antes del mes
///   coast no hay margen: cada euro que se deje de aportar retrasa la fecha.
/// - Resto: `None` — la estrategia no tiene margen que publicar.
fn member_disposable_monthly(r: &MemberRun) -> Option<Decimal> {
    disposable_monthly_of(&r.solves)
}

/// **Los solves de §B.7 que la estrategia necesita**, y ninguno más.
///
/// Función libre (y no un método de `MemberRun`) porque tiene DOS llamantes con inputs distintos:
/// la proyección real, que la corre por miembro y guarda el resultado en la cache, y
/// `simulate_projection`, que la corre sobre el baseline y sobre el escenario para poder publicar
/// los deltas del plan. Si cada uno decidiera por su cuenta qué solve toca, `retire_at_age`
/// podría publicar un `required_contribution_monthly` en la serie y `null` en el what-if.
///
/// **Coste**: cada solve es una bisección sobre el motor entero — hasta 26 proyecciones
/// completas. Por eso las dos estrategias por cruce (`asap`, `pension_bridge`) no pagan nada, y
/// por eso el llamante corre esto DENTRO del semáforo de CPU.
fn compute_strategy_solves(
    input: &ProjectionInput,
    forced_retirement_month: Option<u32>,
    strategy: RetirementStrategy,
) -> Result<StrategySolves, EngineError> {
    let monthly_headroom = futurefin_engine::first_month_allocation(input)?
        .recurring_net
        .max(Decimal::ZERO);
    let mut out = StrategySolves {
        required: None,
        coast: None,
        monthly_headroom,
    };
    let Some(r) = forced_retirement_month else {
        return Ok(out);
    };
    match strategy {
        // `partial` con edad de fin comparte solve con `retire_at_age`: la pregunta es la misma
        // («¿cuánto tengo que aportar para llegar a R?») y el criterio también.
        RetirementStrategy::RetireAtAge | RetirementStrategy::Partial => {
            out.required = futurefin_engine::required_contribution_monthly(input, r)?;
        }
        RetirementStrategy::Coast => {
            out.coast = futurefin_engine::coast_fire_month_index(input, r)?;
        }
        // `asap` y `pension_bridge` se disparan por cruce: no hay `R` que resolver.
        RetirementStrategy::Asap | RetirementStrategy::PensionBridge => {}
    }
    Ok(out)
}

/// **Margen mensual disponible** a partir de los solves, con la base que corresponde a cada
/// estrategia (la misma cuenta para la serie y para el what-if — ver `member_disposable_monthly`).
fn disposable_monthly_of(solves: &StrategySolves) -> Option<Decimal> {
    if let Some(sol) = solves.required.as_ref() {
        return Some(money_out(
            (sol.search_ceiling - sol.contribution).max(Decimal::ZERO),
        ));
    }
    let coast = solves.coast.as_ref()?;
    // `coast_month_index` es un mes del BUCLE (1-based) y «hoy» es el mes 1: antes del mes coast
    // no hay margen —cada euro que se deje de aportar retrasa la fecha— y con `None` (coast no
    // alcanzable) tampoco lo hay nunca.
    let reached_today = coast.coast_month_index.is_some_and(|k| k <= 1);
    Some(money_out(if reached_today {
        solves.monthly_headroom
    } else {
        Decimal::ZERO
    }))
}

/// **Los avisos de UN miembro**, en un solo sitio: los del ensamblado (`birth_date_missing`,
/// `bridge_discount_no_liquid_assets`, `bridge_discount_clamped`), los del bucle (`retire_at_age_underfunded`,
/// `partial_phase_capital_shrinking`) y los del SOLVE (`coast_not_reachable`, que solo la
/// bisección puede saber).
///
/// **Deduplicado**: `retire_at_age_underfunded` lo emiten el bucle (se jubila por debajo del
/// objetivo) y el solve (ni aportándolo todo llega) por caminos distintos, y publicarlo dos veces
/// haría que un consumidor que cuenta avisos midiera gravedad donde solo hay repetición.
///
/// El literal público de cada aviso del motor lo pone `EngineWarning::code()` en el propio crate,
/// no un `match` aquí: un mapeo duplicado se queda atrás en cuanto el enum crece, y un aviso con
/// dos nombres es un aviso que nadie puede buscar.
fn merge_warnings(
    assembly: &[String],
    output: &futurefin_engine::ProjectionOutput,
    solves: &StrategySolves,
) -> Vec<String> {
    let mut out = assembly.to_vec();
    let engine = output
        .warnings
        .iter()
        .copied()
        .chain(
            solves
                .required
                .iter()
                .flat_map(|s| s.warnings.iter().copied()),
        )
        .chain(solves.coast.iter().flat_map(|c| c.warnings.iter().copied()));
    for w in engine {
        let code = w.code().to_string();
        if !out.contains(&code) {
            out.push(code);
        }
    }
    out
}

fn member_warnings(r: &MemberRun) -> Vec<String> {
    merge_warnings(&r.built.warnings, &r.output, &r.solves)
}

/// Decima una serie `Decimal` del motor a los mismos puntos que `points[]` y la serializa como
/// f64 (excepción chart-only D4). Se itera sobre `points` —no sobre `kept_indices`— por la misma
/// razón que `fire_target_series`: el paralelismo con los puntos tiene que ser estructural.
fn decimate_to_points(points: &[ProjectionPoint], serie: &[Decimal]) -> Vec<f64> {
    points
        .iter()
        .map(|p| {
            series_at(serie, p.month_index as usize)
                .to_f64()
                .unwrap_or(0.0)
        })
        .collect()
}

pub async fn compute_projection_series_response(
    state: &AppState,
    user_id: Uuid,
    iid: Uuid,
    view: LedgerView,
    months_override: Option<u32>,
    density: Density,
) -> Result<ProjectionSeriesResponse, ApiError> {
    let ProjectionContext {
        today,
        inflation_annual_percent,
        show_age_mode,
        fire_settings,
        retirement_profile,
        birth_date: resolved_birth_for_demographics,
        session_birth_date,
        months: ctx_months,
        horizon_basis: ctx_horizon_basis,
    } = resolve_projection_context(&state.pool, iid, user_id, months_override).await?;

    // -----------------------------------------------------------------------------------------
    // Qué se simula, y cuántas veces (D9)
    // -----------------------------------------------------------------------------------------
    // `mine`: UNA simulación, la del solicitante. `household`: UNA POR MIEMBRO, cada una con su
    // perfil, su fecha de nacimiento y sus filas — porque desde 5.0.0 la jubilación es una
    // estrategia por persona y no hay forma de correr dos estrategias en un solo bucle.
    let aggregated = matches!(view, LedgerView::Household);
    let member_rows: Vec<HouseholdMemberRow> = if aggregated {
        household_members(&state.pool, iid).await?
    } else {
        Vec::new()
    };

    // Horizonte COMÚN del hogar: el mayor de los horizontes individuales
    // (`horizon_basis: "household_max_lifespan"`). Cortar por el más corto jubilaría de golpe al
    // resto; alargar el propio de cada uno les haría vivir más de lo que declararon. Con
    // `?months=` explícito manda el llamante, como siempre.
    let (months, horizon_basis) = if months_override.is_some() || !aggregated {
        (ctx_months, ctx_horizon_basis)
    } else {
        let longest = member_rows
            .iter()
            .map(|m| {
                let p = resolve_retirement_profile(
                    m.retirement_profile.as_ref().map(|j| j.0.clone()),
                );
                projection_horizon_months(today, &[m.birth_date], p.horizon_lifespan_age).0
            })
            .max()
            .unwrap_or(ctx_months);
        (longest, "household_max_lifespan".to_string())
    };
    let horizon_years = months / 12;

    let runs: Vec<MemberRun> = if aggregated {
        let mut acc = Vec::with_capacity(member_rows.len());
        for m in member_rows {
            let profile =
                resolve_retirement_profile(m.retirement_profile.map(|j| j.0));
            // El horizonte PROPIO del miembro, con la MISMA regla que `view=mine` usaría para
            // él: es lo que `members[].horizon_months` publica para que el chart sepa hasta
            // dónde llega el plan de cada uno dentro de la rejilla común.
            let own = projection_horizon_months(today, &[m.birth_date], profile.horizon_lifespan_age).0;
            acc.push(
                run_member_projection(
                    state,
                    iid,
                    m.user_id,
                    m.username,
                    profile,
                    m.birth_date,
                    today,
                    months,
                    own,
                    inflation_annual_percent,
                    &fire_settings,
                )
                .await?,
            );
        }
        acc
    } else {
        vec![
            run_member_projection(
                state,
                iid,
                user_id,
                String::new(),
                retirement_profile.clone(),
                session_birth_date,
                today,
                months,
                // En `mine` el horizonte común ES el suyo (o el `?months=` que pidió).
                months,
                inflation_annual_percent,
                &fire_settings,
            )
            .await?,
        ]
    };

    // -----------------------------------------------------------------------------------------
    // Agregación (§D del plan, campo a campo). Con un solo run es la identidad.
    // -----------------------------------------------------------------------------------------
    let series_len = runs
        .first()
        .map(|r| r.output.net_worth.len())
        .unwrap_or((months + 1) as usize);
    let mut agg_net_worth: Vec<Decimal> = Vec::new();
    let mut agg_contributed: Vec<Decimal> = Vec::new();
    let mut agg_liquid: Vec<Decimal> = Vec::new();
    let mut agg_withdrawal: Vec<Decimal> = Vec::new();
    let mut agg_shortfall: Vec<Decimal> = Vec::new();
    let mut agg_excess: Vec<Decimal> = Vec::new();
    let mut agg_unmet: Vec<Decimal> = Vec::new();
    let mut uncovered_deficit_total = Decimal::ZERO;
    let mut unallocated_savings_total = Decimal::ZERO;
    let mut monthly_delta_assumption = Decimal::ZERO;
    let mut assets_depleted_month_index: Option<u32> = None;
    let mut asset_series_ids: Vec<(Uuid, String)> = Vec::new();
    let mut all_assets: Vec<futurefin_engine::SimAsset> = Vec::new();
    let mut all_planning_rows: Vec<PlanningFlowProjRow> = Vec::new();
    let mut liabilities_negative_amortization: Vec<LiabilityNegativeAmortization> = Vec::new();

    for r in &runs {
        add_series_into(&mut agg_net_worth, &r.output.net_worth);
        add_series_into(&mut agg_contributed, &r.output.contributed_capital);
        add_series_into(&mut agg_liquid, &r.output.liquid_worth);
        add_series_into(&mut agg_withdrawal, &r.output.withdrawal);
        add_series_into(&mut agg_shortfall, &r.output.withdrawal_shortfall);
        add_series_into(&mut agg_excess, &r.output.withdrawal_excess);
        // El motor ya clampa esta serie a 0 mes a mes; la suma de no-negativos lo sigue siendo,
        // así que el agregado del hogar no necesita un segundo clamp (a diferencia del TOTAL,
        // que conserva el operando literal de 4.15.0 y sí se clampa al publicar).
        add_series_into(&mut agg_unmet, &r.output.unmet_need);
        uncovered_deficit_total += r.output.uncovered_deficit_total;
        unallocated_savings_total += r.output.unallocated_savings_total;
        monthly_delta_assumption += r.built.monthly_net_regular;
        // MÍNIMO, no suma: el hogar se queda sin cartera cuando el PRIMERO de sus miembros se
        // queda sin la suya — el detalle de quién y cuándo vive en `members[]`. Se convierte a la
        // rejilla ANTES de minimizar (#210): la variable guarda meses publicables, no meses de
        // bucle, así que ningún camino puede escapársele sin convertir.
        if let Some(k) = engine_month_to_grid(r.output.assets_depleted_month_index) {
            assets_depleted_month_index =
                Some(assets_depleted_month_index.map_or(k, |acc: u32| acc.min(k)));
        }
        asset_series_ids.extend(r.built.asset_id_name.iter().cloned());
        all_assets.extend(r.built.input.assets.iter().cloned());
        all_planning_rows.extend(r.built.planning_rows.iter().cloned());
        liabilities_negative_amortization.extend(r.negative_amortization.iter().cloned());
    }
    if agg_net_worth.is_empty() {
        // Hogar sin ningún miembro simulable: serie plana de ceros en vez de un array vacío que
        // el chart leería como «no hay datos» y un agente como «patrimonio desconocido».
        agg_net_worth = vec![Decimal::ZERO; series_len];
        agg_contributed = vec![Decimal::ZERO; series_len];
        agg_liquid = vec![Decimal::ZERO; series_len];
        agg_withdrawal = vec![Decimal::ZERO; series_len];
        agg_shortfall = vec![Decimal::ZERO; series_len];
        agg_excess = vec![Decimal::ZERO; series_len];
        agg_unmet = vec![Decimal::ZERO; series_len];
    }

    let starting_net_worth = agg_net_worth.first().copied().unwrap_or(Decimal::ZERO);

    // Indices a serializar según la densidad solicitada. Para `Hybrid`
    // (mes 0..12 mensual + anual desde 24) el JSON pesa ~5× menos.
    let kept_indices = density_month_indices(density, agg_net_worth.len() as u32);

    let points: Vec<ProjectionPoint> = kept_indices
        .iter()
        .filter_map(|&i| {
            let idx = i as usize;
            let nw = agg_net_worth.get(idx)?;
            Some(ProjectionPoint {
                month_index: i,
                net_worth: *nw,
                contributed_capital: series_at(&agg_contributed, idx),
                net_worth_liquid: series_at(&agg_liquid, idx),
                // Deflactado por el `month_index` del punto, JAMÁS por su posición en el array:
                // con `density=hybrid` los puntos no son equidistantes y la versión ingenua
                // deflacta 70 años como si fueran 30 — el bug del chart de la v1.4.2, que aquí
                // no puede reproducirse porque el helper solo acepta un número de mes.
                //
                // En el agregado se deflacta DESPUÉS de sumar (§D): la inflación es de la
                // instalación, así que deflactar cada miembro y sumar da lo mismo — pero
                // hacerlo una vez deja una sola división por punto y una sola cifra que auditar.
                net_worth_real: *nw * deflator_at_month_index(inflation_annual_percent, i),
                withdrawal: series_at(&agg_withdrawal, idx),
                withdrawal_shortfall: series_at(&agg_shortfall, idx),
                withdrawal_excess: series_at(&agg_excess, idx),
                // **A la escala monetaria, y aquí sí** (a diferencia de las tres columnas de
                // arriba, que son geometría de chart y viajan tal cual). Esta es la única
                // columna del punto cuyo SIGNO se lee como un veredicto —«este mes no se
                // cubrió»—, y el acumulador del motor conserva el operando literal de 4.15.0:
                // `after_tax(gross_up(n))` devuelve `n` solo hasta el redondeo a 28 dígitos, así
                // que la serie llega con una polvareda de ±1e-25 € en meses de un hogar
                // perfectamente solvente (medido: 7 de 66 puntos del arnés de `mcp_simulate`).
                // Publicarla encendería meses en rojo al azar. El motor no puede clamparla —
                // movería el pin dorado—; quien publica, sí.
                unmet_need: money_out(series_at(&agg_unmet, idx)),
            })
        })
        .collect();

    // Milestones se computan sobre TODOS los meses (no sobre los serializados),
    // si no, con `density=hybrid` se perderían milestones que caen entre dos
    // puntos anuales.
    let points_full: Vec<NwPoint> = agg_net_worth
        .iter()
        .enumerate()
        .map(|(i, nw)| NwPoint {
            month_index: i as u32,
            net_worth: *nw,
        })
        .collect();

    // `asset_id_name` y `planning_rows` se reusan del ensamblado de cada miembro — antes el
    // handler hacía 2 SELECTs adicionales redundantes contra `assets` y `planning_flows`. En
    // `household` las series por activo se CONCATENAN: ya vienen identificadas por `asset_id`,
    // así que sumarlas sería destruir la única desagregación que el chart necesita.
    let mut asset_series: Vec<AssetSeries> = Vec::with_capacity(asset_series_ids.len());
    for r in &runs {
        for ((id, name), serie) in r.built.asset_id_name.iter().zip(r.output.per_asset_series.iter())
        {
            asset_series.push(AssetSeries {
                asset_id: *id,
                asset_name: name.clone(),
                values: kept_indices
                    .iter()
                    .filter_map(|&i| serie.get(i as usize))
                    .map(|v| v.to_f64().unwrap_or(0.0))
                    .collect(),
            });
        }
    }

    let milestone_baseline_adjustment =
        planning_upcoming_net_for_milestone_baseline(today, &all_planning_rows);
    // Eventos: mismo `planning_rows` ya cargado, misma regla fecha→mes que los ajustes de caja.
    // Ninguna query nueva. En `household` la lista es la UNIÓN de los Próximos de los miembros y
    // el tope de 100 se aplica al conjunto — un tope por miembro dejaría respuestas de tamaño
    // proporcional al hogar.
    let (events, events_truncated) = projection_events_from_flows(today, months, &all_planning_rows);
    let milestones = projection_unique_reached_milestones(
        &points_full,
        today,
        milestone_baseline_adjustment,
        PROJECTION_MILESTONE_LIMIT,
        PROJECTION_MILESTONE_SEARCH_COUNT,
    );

    // Milestones en euros de hoy: mismos umbrales sobre el patrimonio deflactado. Con inflación 0 el
    // deflactor es 1 y serían idénticos a `milestones`, así que dejamos el vector vacío y la web
    // reusa `milestones`. Con inflación NEGATIVA (#146) sí se computan: el deflactor es > 1 y los
    // hitos reales llegan ANTES que los nominales. Se computa sobre `points_full` (resolución mensual) por la misma razón
    // que `milestones`: no perder hitos que caen entre dos puntos anuales con densidad `hybrid`.
    let milestones_real = if !inflation_annual_percent.is_zero() {
        let points_full_real = deflate_points_to_today(&points_full, inflation_annual_percent);
        projection_unique_reached_milestones(
            &points_full_real,
            today,
            milestone_baseline_adjustment,
            PROJECTION_MILESTONE_LIMIT,
            PROJECTION_MILESTONE_SEARCH_COUNT,
        )
    } else {
        Vec::new()
    };

    // -----------------------------------------------------------------------------------------
    // Lecturas de jubilación: del run del solicitante en `mine`, ninguna en `household`
    // -----------------------------------------------------------------------------------------
    let solo = (!aggregated).then(|| &runs[0]);

    let fire_target_ref = solo
        .and_then(|r| r.built.fire_target_reading.as_ref())
        .filter(|ft| futurefin_engine::fire_target_base_at_month_index(ft, 0).is_some());

    // #142: el término de hoy (mes 0) para la vista previa del formulario.
    let fire_target_debt_component = fire_target_ref
        .map(|ft| {
            ft.debt_payments_remaining
                .first()
                .copied()
                .unwrap_or(Decimal::ZERO)
        })
        .map(crate::money::money_out);

    // R8 — `jubilacion_month_index` ES el mes EFECTIVO de jubilación que decidió el motor, no el
    // cruce derivado por el handler. Con `asap` los dos coinciden exactamente (el cruce ES el
    // trigger) y ni un pin de 4.15.x se mueve; con una estrategia por edad, esto dice cuándo te
    // jubilas de verdad y `liquid_crossing_month_index` cuándo habrías llegado.
    let jubilacion_month_index =
        solo.and_then(|r| engine_month_to_grid(r.output.retirement_month_index));
    // **El cruce lo lee el MOTOR desde WP5-2b.** Hasta WP5-2a lo recalculaba el handler
    // (`fire_crossover_month`) porque a las estrategias por edad se les pasaba `fire_target:
    // None` para que el cruce no las jubilara; con `crossing_is_reading_only` el objetivo entra
    // igual y el motor anota el cruce sin decidir nada. La diferencia no es de estilo: el motor
    // evalúa el objetivo CONSCIENTE DEL PLAN (puente incluido) y el handler evaluaba la
    // perpetuidad de 4.15.x — con `pension_bridge` eran dos cruces distintos para la misma línea.
    let liquid_crossing_month_index =
        solo.and_then(|r| engine_month_to_grid(r.output.liquid_crossing_month_index));
    // El `PhasePlan` del solicitante: es lo que hace que la línea del chart sea el objetivo del
    // PLAN (puente con pensión con fecha) y no la perpetuidad de 4.15.x.
    let solo_plan = solo.map(|r| &r.built.input.phase_plan);

    let (fire_target_series, jubilacion_target_net_worth, jubilacion_target_net_worth_nominal) =
        match (fire_target_ref, solo_plan) {
            (Some(ft), Some(plan)) => {
                // **El evaluador se construye UNA vez.** `fire_target_at_month_index_with_plan`
                // es la forma de conveniencia y rehace la tabla del puente —`O(P)` gross-ups—
                // en CADA llamada; con densidad mensual son ~841 llamadas y `P = 364`, o sea
                // 300.000 gross-ups por respuesta. Medido antes de arreglarlo: **1.943 ms** de
                // MISS en un hogar con pensión con fecha, contra 5 ms sin ella. `PlanFireTarget`
                // existe exactamente para esto: `O(P)` al construirlo, `O(1)` por consulta.
                let plan_target = futurefin_engine::PlanFireTarget::new(Some(ft), plan);
                // Se itera sobre `points`, NO sobre `kept_indices`: la serie es paralela a los
                // puntos y el consumidor la alinea por posición (no lleva `month_index` propio,
                // auditoría MCP §8), así que el paralelismo tiene que ser estructural. Antes `points`
                // usaba `filter_map` (descarta índices fuera de rango) y esta serie un `map` que
                // no descartaba nada: coincidían solo porque `density_month_indices` nunca emite
                // un índice > months-1. Un cambio ahí las habría desalineado en silencio.
                let series: Vec<f64> = points
                    .iter()
                    .map(|p| {
                        plan_target
                            .at(p.month_index)
                            .unwrap_or(Decimal::ZERO)
                            .to_f64()
                            .unwrap_or(0.0)
                    })
                    .collect();
                debug_assert_eq!(series.len(), points.len(), "fire_target_series ∥ points");
                // Target del mes de la jubilación EFECTIVA en euros NOMINALES, calculado EXACTO
                // con el helper del motor. Ni se interpola entre dos puntos de la serie ni se lee
                // de `fire_target_series[pos]`: con `density=hybrid` el mes puede no ser un punto
                // servido. Con una estrategia por edad esta cifra sigue siendo una LECTURA —
                // «esto es lo que habrías necesitado el mes en que te jubilas» —, y compararla
                // con el líquido de ese mes es exactamente lo que revela una infra-financiación.
                let nominal = jubilacion_month_index
                    .and_then(|k| plan_target.at(k))
                    .map(|v| money_out(v).to_string());
                // La BASE de hoy es el objetivo del PLAN en el mes 0 **menos su término de
                // deuda** — no `fire_target_base_at_month_index`, que solo conoce la perpetuidad.
                // Sin pensión con fecha las dos expresiones son la misma cifra por construcción
                // (`fire_target_at_month_index` = base + deuda), así que ningún hogar de 4.15.x
                // ve moverse este número; con base PUENTE, la de arriba es la única que cuadra
                // con `fire_target_series[0]`, que es la línea que el chart pinta.
                let debt_today = ft
                    .debt_payments_remaining
                    .first()
                    .copied()
                    .unwrap_or(Decimal::ZERO);
                let base_today = plan_target
                    .at(0)
                    .map(|t| (t - debt_today).max(Decimal::ZERO))
                    .unwrap_or(Decimal::ZERO);
                (series, Some(money_out(base_today).to_string()), nominal)
            }
            _ => (Vec::new(), None, None),
        };

    // Posición del mes de jubilación dentro de los arrays paralelos: el ÚLTIMO punto servido cuyo
    // `month_index` no pasa del mes de jubilación (convención documentada en el campo). `rposition`
    // sobre `points`, no aritmética sobre `kept_indices`: la posición es una propiedad del array
    // que se serializa, así que se deriva de él.
    let jubilacion_series_position = jubilacion_month_index.and_then(|k| {
        points
            .iter()
            .rposition(|p| p.month_index <= k)
            .map(|p| p as u32)
    });
    // Lectura civil del cruce, resuelta en servidor: el índice suelto obliga al consumidor a hacer
    // aritmética de calendario y de edad, que es donde se equivoca en silencio.
    let (jubilacion_date_ymd, jubilacion_age) = jubilacion_civil(
        today,
        resolved_birth_for_demographics,
        jubilacion_month_index,
    );

    let phase_transitions: Vec<PhaseTransition> = solo
        .map(|r| {
            r.output
                .phase_transitions
                .iter()
                .map(|(phase, k)| PhaseTransition {
                    phase: match phase {
                        Phase::Accumulating => "accumulating",
                        Phase::Partial => "partial",
                        Phase::Retired => "retired",
                    },
                    month_index: engine_month_to_grid(Some(*k)).unwrap_or(0),
                })
                .collect()
        })
        .unwrap_or_default();

    // Avisos del ensamblado + los del motor. El literal público de cada aviso del motor lo pone
    // `EngineWarning::code()` (en el propio crate), no un `match` aquí: un mapeo duplicado en el
    // handler se queda atrás en cuanto el enum crece, y un aviso con dos nombres es un aviso que
    // nadie puede buscar.
    let warnings: Vec<String> = solo.map(member_warnings).unwrap_or_default();

    let has_retirement_trigger = solo.is_some_and(|r| {
        r.built.retirement_trigger == RETIREMENT_TRIGGER_AGE || fire_target_ref.is_some()
    });

    let final_nominal = points_full.last().map(|p| p.net_worth).unwrap_or(Decimal::ZERO);
    let final_month_index = points_full.last().map(|p| p.month_index).unwrap_or(0);
    let final_net_worth_real =
        final_nominal * deflator_at_month_index(inflation_annual_percent, final_month_index);

    let members: Vec<HouseholdMemberProjection> = if aggregated {
        runs.iter()
            .map(|r| {
                let effective = engine_month_to_grid(r.output.retirement_month_index);
                let (_, age) = jubilacion_civil(today, r.birth_date, effective);
                HouseholdMemberProjection {
                    user_id: r.user_id,
                    username: r.username.clone(),
                    strategy: strategy_label(r.profile.strategy),
                    jubilacion_month_index: effective,
                    jubilacion_age: age,
                    // Su propio cruce, leído del motor (que evaluó SU objetivo con SU plan).
                    liquid_crossing_month_index: engine_month_to_grid(
                        r.output.liquid_crossing_month_index,
                    ),
                    retirement_month_index: effective,
                    coast_fire_month_index: r
                        .solves
                        .coast
                        .as_ref()
                        .and_then(|c| engine_month_to_grid(c.coast_month_index)),
                    underfunded: r.solves.required.as_ref().map(|s| s.underfunded),
                    required_contribution_monthly: r
                        .solves
                        .required
                        .as_ref()
                        .map(|s| money_out(s.contribution)),
                    disposable_monthly: member_disposable_monthly(r),
                    partial_retirement_month_index: engine_month_to_grid(
                        r.output.partial_retirement_month_index,
                    ),
                    pension_start_month_index: engine_month_to_grid(
                        r.output.pension_start_month_index,
                    ),
                    assets_depleted_month_index: engine_month_to_grid(
                        r.output.assets_depleted_month_index,
                    ),
                    warnings: member_warnings(r),
                    horizon_months: r.own_horizon_months,
                    // MISMOS `kept_indices` que `points[]`: la decimación es una decisión del
                    // servidor por respuesta, no por serie, y dos densidades distintas en el
                    // mismo JSON serían dos rejillas que el chart tendría que reconciliar.
                    // `filter_map` (y no `map`) por la misma razón que en `points`: si algún día
                    // una serie del motor viniera más corta, se cae un punto, no se inventa un 0.
                    series: kept_indices
                        .iter()
                        .filter_map(|&i| {
                            let idx = i as usize;
                            Some(MemberSeriesPoint {
                                month_index: i,
                                net_worth: *r.output.net_worth.get(idx)?,
                                net_worth_liquid: series_at(&r.output.liquid_worth, idx),
                            })
                        })
                        .collect(),
                }
            })
            .collect()
    } else {
        Vec::new()
    };

    // El eco de contexto del ahorro es el del SOLICITANTE también en `household`: la fuente y las
    // ventanas son de la instalación, pero el fallback («no hay meses reales») se decide sobre
    // las filas de cada scope, así que no existe UN basis del hogar. Se publica el suyo — el
    // mismo criterio que la demografía (`viewer_birth_date`, horizonte), que también es del
    // solicitante en las dos vistas.
    // -----------------------------------------------------------------------------------------
    // Solves y lecturas de estrategia (§B.3, §B.7). Todo vacío en `household`: el agregado suma N
    // planes y ninguna de estas cifras tiene versión «del hogar».
    // -----------------------------------------------------------------------------------------
    let required = solo.and_then(|r| r.solves.required.as_ref());
    let coast = solo.and_then(|r| r.solves.coast.as_ref());
    let liquid_full: &[Decimal] = solo.map(|r| r.output.liquid_worth.as_slice()).unwrap_or(&[]);

    let required_capital_path = required
        .map(|s| decimate_to_points(&points, &s.required_capital_path))
        .unwrap_or_default();
    let coast_path = coast
        .map(|c| decimate_to_points(&points, &c.coast_path))
        .unwrap_or_default();

    // `disposable_capital` mensual completo (se decima después, como el resto de series). **No se
    // clampa a ≥ 0**: con la cascada dirigiendo el sobrante a un activo NO líquido, aportar más
    // no sube el líquido y la diferencia puede quedar por debajo de cero — esconderlo con un
    // `max(0, ·)` publicaría un colchón que no existe.
    let disposable_capital_full: Vec<Decimal> = if let Some(sol) = required {
        liquid_full
            .iter()
            .enumerate()
            .map(|(i, l)| *l - series_at(&sol.required_capital_path, i))
            .collect()
    } else if let Some(c) = coast {
        // Antes del mes coast el margen es CERO por definición: hasta ahí hay que aportar. Desde
        // él, lo que el líquido real supera a la trayectoria «si dejas de aportar».
        let from = coast
            .and_then(|c| c.coast_month_index)
            .map(|k| k.saturating_sub(1) as usize)
            .unwrap_or(usize::MAX);
        liquid_full
            .iter()
            .enumerate()
            .map(|(i, l)| {
                if i >= from {
                    *l - series_at(&c.coast_path, i)
                } else {
                    Decimal::ZERO
                }
            })
            .collect()
    } else {
        Vec::new()
    };
    let disposable_capital = if disposable_capital_full.is_empty() {
        Vec::new()
    } else {
        decimate_to_points(&points, &disposable_capital_full)
    };
    // El margen en el mes EFECTIVO de jubilación: el nominal de ese mes y su lectura en euros de
    // hoy (mismo deflactor que `points[].net_worth_real`).
    let disposable_capital_at_retirement = (!disposable_capital_full.is_empty())
        .then_some(jubilacion_month_index)
        .flatten()
        .and_then(|k| disposable_capital_full.get(k as usize).copied())
        .map(money_out);
    let disposable_capital_today = disposable_capital_at_retirement.and_then(|v| {
        jubilacion_month_index
            .map(|k| money_out(v * deflator_at_month_index(inflation_annual_percent, k)))
    });

    let context_run = if aggregated {
        runs.iter().find(|r| r.user_id == user_id).or_else(|| runs.first())
    } else {
        runs.first()
    };
    let effective_savings_source = context_run
        .map(|r| r.built.effective_savings_source)
        .unwrap_or_default();
    let savings_income_basis = context_run
        .map(|r| r.built.savings_income_basis.clone())
        .unwrap_or_else(SavingsAvgBasis::budget);
    let savings_expense_basis = context_run
        .map(|r| r.built.savings_expense_basis.clone())
        .unwrap_or_else(SavingsAvgBasis::budget);

    Ok(ProjectionSeriesResponse {
        view: view.as_str(),
        points,
        months,
        horizon_years,
        horizon_basis,
        horizon_lifespan_age: retirement_profile.horizon_lifespan_age,
        final_net_worth_real: money_out(final_net_worth_real),
        starting_net_worth: money_out(starting_net_worth),
        // En modos B/C esto es `sum / meses reales`: sin `money_out` viajaba con ~25 decimales
        // mientras `simulate_projection` publicaba la misma cifra con cuatro.
        monthly_delta_assumption: money_out(monthly_delta_assumption),
        model_note: PROJECTION_MODEL_NOTE.into(),
        anchor_date_ymd: today.format("%Y-%m-%d").to_string(),
        show_age_mode: show_age_mode.clone(),
        use_age_on_x_axis: show_age_mode.trim() == "ages"
            && resolved_birth_for_demographics.is_some(),
        viewer_birth_date: resolved_birth_for_demographics
            .map(|d| d.format("%Y-%m-%d").to_string()),
        milestones,
        milestones_real,
        compound_outpaces_true_savings_month_index: solo.and_then(|r| r.compound_month),
        compound_outpaces_true_savings_absent_reason: aggregated
            .then_some(ABSENT_HOUSEHOLD_AGGREGATE),
        jubilacion_month_index,
        jubilacion_date_ymd,
        jubilacion_age,
        jubilacion_target_net_worth,
        fire_target_debt_component,
        jubilacion_series_position,
        jubilacion_target_net_worth_nominal,
        fire_target_series,
        asset_series,
        density: match density {
            Density::Monthly => "monthly".into(),
            Density::Hybrid => "hybrid".into(),
        },
        events,
        events_truncated,
        savings_source: effective_savings_source,
        savings_income_basis,
        savings_expense_basis,
        deflation_annual_inflation_percent: money_out(inflation_annual_percent),
        drawdown_gain_basis: drawdown_gain_basis_of(&all_assets),
        taxable_gain_ratio_today: taxable_gain_ratio_today_of(&all_assets),
        assets_depleted_month_index,
        // **Clamp a ≥ 0 en la PUBLICACIÓN, no en el motor.** El descubierto se acumula como
        // residuo de ventas brutas y puede salir con una cola de redondeo negativa del orden de
        // −5e-25 (declarado en `MonthSale::account`, `crates/engine/src/sim_core.rs`): eso no
        // es «−0,0000000000000000000000005 € descubiertos», es cero. Se corrige aquí porque el
        // motor debe seguir publicando su aritmética tal cual —el golden la hashea— y quien
        // redondea para un humano es la capa que serializa.
        uncovered_deficit_total: money_out(uncovered_deficit_total.max(Decimal::ZERO)),
        unallocated_savings_total: money_out(unallocated_savings_total),
        unallocated_savings_reason: unallocated_reason_of(&all_assets, unallocated_savings_total),
        liabilities_negative_amortization,
        fire_target_absent_reason: if aggregated {
            Some(ABSENT_HOUSEHOLD_AGGREGATE)
        } else {
            solo.and_then(|r| r.built.fire_target_absent_reason)
        },
        strategy: solo.map(|r| strategy_label(r.profile.strategy)),
        retirement_trigger: solo.map(|r| r.built.retirement_trigger),
        retirement_month_index: jubilacion_month_index,
        retirement_series_position: jubilacion_series_position,
        liquid_crossing_month_index,
        liquid_crossing_absent_reason: if aggregated {
            Some(ABSENT_HOUSEHOLD_AGGREGATE)
        } else if fire_target_ref.is_none() {
            Some(ABSENT_NO_FIRE_TARGET)
        } else {
            None
        },
        jubilacion_absent_reason: if aggregated {
            Some(ABSENT_HOUSEHOLD_AGGREGATE)
        } else if has_retirement_trigger {
            None
        } else {
            Some(ABSENT_NO_RETIREMENT_TRIGGER)
        },
        phase_transitions,
        pension_start_month_index: solo
            .and_then(|r| engine_month_to_grid(r.output.pension_start_month_index)),
        partial_retirement_month_index: solo
            .and_then(|r| engine_month_to_grid(r.output.partial_retirement_month_index)),
        warnings,
        members,
        bridge_discount_annual_pct: solo
            .and_then(|r| r.built.bridge_discount_annual_pct)
            .map(money_out),
        bridge_effective_withdrawal_pct: solo
            .and_then(|r| r.output.bridge_effective_withdrawal_pct)
            .map(money_out),
        pension_coverage_ratio: solo
            .and_then(|r| r.output.pension_coverage_ratio)
            .map(money_out),
        partial_gap_target: solo.and_then(|r| r.output.partial_gap_target).map(money_out),
        // `null` (no `false`) sin fase parcial: el motor publica un `bool` porque es una función
        // pura y debe definir el estado, pero «no hubo media jornada» y «hubo y menguó» no pueden
        // compartir valor en el wire.
        partial_phase_capital_growing: solo.and_then(|r| {
            r.output
                .partial_retirement_month_index
                .map(|_| r.output.partial_phase_capital_growing)
        }),
        required_contribution_monthly: required.map(|s| money_out(s.contribution)),
        required_contribution_search_ceiling: required.map(|s| money_out(s.search_ceiling)),
        underfunded: required.map(|s| s.underfunded),
        required_capital_path,
        disposable_monthly: solo.and_then(member_disposable_monthly),
        disposable_capital,
        disposable_capital_at_retirement,
        disposable_capital_today,
        coast_fire_month_index: coast.and_then(|c| engine_month_to_grid(c.coast_month_index)),
        coast_number: coast.and_then(|c| c.coast_number).map(money_out),
        coast_path,
    })
}

// ---------------------------------------------------------------------------
// simulate_projection (tool MCP, what-if puro sin persistir)
// ---------------------------------------------------------------------------

/// Overrides parseados de la tool `simulate_projection` (los strings decimales/UUID/fecha ya
/// convertidos por la capa MCP; las COTAS de dominio se validan aquí, en el core).
#[derive(Debug, Default)]
pub(crate) struct SimulationSpec {
    pub months: Option<u32>,
    /// Gasto puntual: importe > 0 + exactamente uno de (`month_index` 1..=horizonte, `date`).
    pub one_off_amount: Option<Decimal>,
    pub one_off_month_index: Option<u32>,
    pub one_off_date: Option<NaiveDate>,
    pub extra_monthly_expense: Option<Decimal>,
    pub extra_monthly_cash_adjustment: Option<Decimal>,
    pub extra_monthly_savings: Option<Decimal>,
    pub swr_pct: Option<Decimal>,
    pub annual_inflation_percent: Option<Decimal>,
    /// Ejes de `fire_settings` simulables sin persistir (auditoría de simulate_projection §3). Se aplican con el MISMO
    /// `FireSettingsPatch::apply_to` que el PATCH real, y `swr_pct` entra por aquí para que haya
    /// un solo camino.
    pub fire_settings_overrides: Option<crate::handlers::installation::FireSettingsPatch>,
    pub retirement_annual_expense: Option<Decimal>,
    pub asset_return_overrides: Vec<(Uuid, Decimal)>,
    /// Ejes what-if por PASIVO (4.4.0). Mismo molde que `asset_return_overrides`: se aplican
    /// post-build sobre el input clonado del escenario, porque ninguno de ellos mueve el target
    /// FIRE ni las bases de los caps (que dependen de `payment_amount`, que NO se toca).
    pub liability_overrides: Vec<LiabilityOverrideSpec>,
    /// **P11 — crecimiento REAL del ingreso, % anual** (5.0.0, D30; solo MCP). `[−10, 20]`, y
    /// `0` es un 400: un eje que no puede mover nada se rechaza, no se acepta en silencio.
    ///
    /// Entra como ajuste de CAJA mes a mes (`planning_monthly_cash_adjustment`), no como una
    /// subida de `income_regular_monthly`: el ingreso base es lo que ancla el objetivo FIRE en
    /// modo `current_income` y las bases de los caps, y una subida de sueldo no debe reescribir
    /// el objetivo del escenario por un camino que el usuario no pidió. La consecuencia está
    /// declarada: `income_monthly`, `net_recurring_monthly` y `savings_rate` NO se mueven — el
    /// que se mueve es `net_cash_monthly`, igual que con `extra_monthly_savings`.
    pub income_growth_real_pct_annual: Option<Decimal>,
    /// **P11 — escalones de ingreso** (≤ 24). Cada uno suma `delta_monthly` (con signo, ≠ 0) a
    /// la caja **desde su mes y hasta el final del horizonte**. A diferencia del crecimiento,
    /// NO se recortan en la jubilación: el usuario ha nombrado el mes, así que quitárselo sería
    /// simular otra cosa.
    pub income_steps: Vec<IncomeStepSpec>,
    /// **P5 y compañía — el PERFIL de jubilación entero como eje what-if** (5.0.0, §E/D30, solo
    /// MCP). Se aplica con el MISMO `RetirementProfilePatch::apply_to` que el PATCH real sobre un
    /// CLON del perfil RESUELTO del usuario, se valida con `validate_retirement_profile` y se
    /// vuelve a resolver: lo que se simula aquí es exactamente lo que pasaría al guardarlo.
    ///
    /// Por aquí vuelven `fire_number_mode` y `fire_number_manual_amount`, que WP4 quitó de
    /// `fire_settings_overrides` al mudarlos al perfil (D13) y que se quedaron sin eje what-if
    /// durante una ola.
    pub profile_overrides: Option<crate::handlers::retirement_profile::RetirementProfilePatch>,
    /// **P8.c — pausa de ingresos** («¿y si me cojo una excedencia de 12 meses?»). Mueve el
    /// escenario Y publica su retraso en `income_pause` de la respuesta.
    pub income_pause: Option<IncomePauseSpec>,
    /// **P8.b — «¿cuánto más puedo gastar sin mover la fecha?»**. Opt-in porque cuesta una
    /// bisección entera sobre el motor (hasta 26 proyecciones). `Some(false)` es un 400: pedir
    /// el bloque `solve` sin pedir ningún solve no puede devolver nada.
    pub solve_extra_monthly_expense_keeping_date: Option<bool>,
    /// **P3 — Monte Carlo sobre los DOS lados** (5.0.0, WP6b). Opt-in porque cuesta `2 · paths`
    /// simulaciones f64 (≈ 0,4 s a 1 000 caminos y 840 meses) y `simulate_projection` es
    /// cache-neutral por diseño: cada what-if paga sus caminos enteros.
    ///
    /// **No lleva anti-no-op, y es la única excepción declarada del bloque.** Los demás ejes
    /// (`income_growth`, `profile_overrides`, `solve`, los de pasivos) se rechazan cuando no
    /// pueden mover nada porque devolverían un escenario idéntico al baseline sin decir por qué.
    /// Este no cambia el escenario: **AÑADE información** sobre los dos lados que ya se iban a
    /// simular. `monte_carlo` con el resto del cuerpo vacío es la pregunta legítima «¿qué
    /// probabilidad de éxito tiene mi plan tal cual está?», y con `paths` a lo que sea siempre
    /// hay algo nuevo que contestar — incluso con toda la cartera a volatilidad cero, donde la
    /// respuesta es «éxito 1 o 0, sin dispersión» y `any_volatility_declared` lo dice.
    pub monte_carlo: Option<MonteCarloSpec>,
    pub include_series: bool,
}

/// El eje de Monte Carlo del what-if. `seed` es un override consciente: omitido, se usa la
/// semilla ESTABLE del usuario (D23), que es la misma con la que `GET /v1/projection/bands`
/// dibuja su fan chart — así el what-if y el gráfico comparan mercados idénticos y el delta de
/// probabilidad mide el CAMBIO DEL PLAN y no el ruido de dos muestras distintas.
#[derive(Debug, Clone)]
pub(crate) struct MonteCarloSpec {
    pub paths: u32,
    pub seed: Option<u64>,
}

/// Pausa de ingresos del what-if (P8.c). El mes es **1-based del ancla**, el mismo eje que
/// `one_off_expense.month_index` e `income_steps[].month_index`.
#[derive(Debug, Clone)]
pub(crate) struct IncomePauseSpec {
    pub from_month_index: Option<u32>,
    pub from_date: Option<NaiveDate>,
    /// Duración en meses (≥ 1). Ventana SEMIABIERTA: `from ≤ k < from + months`.
    pub months: u32,
    /// Multiplicador del ingreso GANADO durante la ventana, en `[0, 1)`. `0` = sin ingreso;
    /// `1` sería el baseline y se rechaza.
    pub income_fraction: Decimal,
}

/// Un escalón de ingreso del what-if: «desde el mes X, +/− N €/mes».
#[derive(Debug, Clone)]
pub(crate) struct IncomeStepSpec {
    /// Mes 1-based del ancla, exactamente el mismo eje que `one_off_expense.month_index` y
    /// `liability_overrides[].lump_sum_month_index`: el mes 1 es el mes civil de
    /// `anchor_date_ymd`. **No** es la rejilla 0-based de `points[].month_index`.
    pub month_index: Option<u32>,
    pub date: Option<NaiveDate>,
    /// Con signo y distinto de cero (un escalón de 0 es un no-op y se rechaza).
    pub delta_monthly: Decimal,
}

/// Un override what-if sobre un pasivo. Los cuatro ejes están **gateados contra el no-op
/// silencioso** en el core: un override que no puede hacer nada devuelve un 400 con su código,
/// nunca un escenario idéntico al baseline sin explicación.
#[derive(Debug, Clone)]
pub(crate) struct LiabilityOverrideSpec {
    pub liability_id: Uuid,
    /// Amortización extra mensual (≥ 0) mientras dure la deuda.
    pub extra_monthly_principal: Option<Decimal>,
    /// Amortización puntual: importe > 0 + exactamente uno de (`month_index`, `date`).
    pub lump_sum_amount: Option<Decimal>,
    pub lump_sum_month_index: Option<u32>,
    pub lump_sum_date: Option<NaiveDate>,
    /// TIN nominal anual (0..=100). Solo devenga en `french`/`interest_only`/`revolving`.
    pub apr_percent: Option<Decimal>,
    /// Modelo de amortización efectivo del escenario.
    ///
    /// No estaba en el mínimo pedido y se añade por una razón medible: hasta 4.7.0
    /// `fixed_payments` era el default de la columna, así que la mayoría de los pasivos
    /// guardados no devengaban — y sin este eje el override de TIN sería un no-op para casi
    /// todo el mundo, o un 400 que no deja hacer la pregunta. Con él, «mi hipoteca está
    /// guardada sin intereses; simúlala como francés al 3 %» es una sola llamada.
    pub repayment_model: Option<LiabRepaymentModel>,
    /// Compensación por reembolso anticipado (#151, Ley 5/2019 art. 23) en % del capital extra
    /// amortizado. Cota dura [0, 2] — el techo legal a tipo fijo. **Ausente ⇒ 2 %** (el default
    /// legal más alto: el what-if deja de ser optimista por defecto; opt-out explícito con "0").
    pub early_repayment_fee_pct: Option<Decimal>,
    /// Qué hace la amortización con el plan (#151): `reduce_term` (default, comportamiento
    /// 4.4.0 — el préstamo acaba antes) o `reduce_payment` (misma extinción, cuota menor).
    pub early_repayment_effect: Option<futurefin_engine::EarlyRepaymentEffect>,
}

#[derive(Debug, Serialize)]
pub(crate) struct SimKpis {
    /// Mes del cruce con el target FIRE (None = no se alcanza en el horizonte). Es la clave para
    /// indexar las series; la lectura humana son los dos campos siguientes.
    pub jubilacion_month_index: Option<u32>,
    /// Primer mes en que la cartera se vacía **y alguna venta posterior se queda sin fundar**
    /// (misma definición de dos condiciones y mismo motor que el campo homónimo de
    /// `/v1/projection/series`, #119). `null` = no se agota en el horizonte, y también el
    /// aterrizaje exacto sobre una pensión que cubre todo el gasto posterior: ahí la cartera se
    /// vacía sin que nadie se quede sin cobrar. Es la respuesta a «si gasto X más, ¿cuándo me
    /// quedo sin nada?» — la pregunta que más justifica un what-if. Desde 5.0.0 va en la rejilla 0-based como el resto de índices (#210); el
    /// delta `assets_depleted_months_delta` no se mueve (los dos lados se desplazan igual).
    pub assets_depleted_month_index: Option<u32>,
    /// Espejo de `/v1/projection/series` (4.12.1): ahorro que ninguna regla absorbió — fuera
    /// del balance, solo cuantificado. `"0.0000"` = caso normal.
    #[serde(with = "rust_decimal::serde::str")]
    pub unallocated_savings_total: Decimal,
    /// `null` | `"no_assets"` | `"no_sink"` — mismo vocabulario que la serie.
    pub unallocated_savings_reason: Option<&'static str>,
    /// Fecha civil del cruce (`YYYY-MM-DD`) = `anchor_date_ymd` + el índice, conservando el día
    /// del ancla con recorte a fin de mes. `null` ⟺ no hay cruce.
    /// Sin `skip_serializing_if`: su hermano `jubilacion_month_index` ya iba explícito y estos dos
    /// no, así que el mismo struct desaparecía unos campos y publicaba otros (auditoría MCP §8).
    pub jubilacion_date_ymd: Option<String>,
    /// Años cumplidos en esa fecha. `null` si no hay cruce o no hay fecha de nacimiento resuelta.
    pub jubilacion_age: Option<u32>,
    /// Patrimonio al final del horizonte, en euros **NOMINALES** de ese mes. Con el horizonte por
    /// defecto eso está a décadas vista, así que la cifra impresiona y no dice nada: para leerla,
    /// su hermano `final_net_worth_real`.
    #[serde(with = "rust_decimal::serde::str")]
    pub final_net_worth: Decimal,
    /// El mismo patrimonio final llevado a **euros de hoy** con la inflación efectiva de este lado.
    /// Con inflación ≤ 0 es exactamente el mismo valor (el deflactor es 1, no ~1).
    #[serde(with = "rust_decimal::serde::str")]
    pub final_net_worth_real: Decimal,
    /// Base del target FIRE (euros de hoy; el target servido crece con la inflación).
    #[serde(with = "rust_decimal::serde::str_option")]
    pub fire_target_base: Option<Decimal>,
    /// Runway de líquidos con la misma fórmula que `/v1/summary`, sobre los inputs del lado.
    #[serde(with = "rust_decimal::serde::str_option")]
    pub runway_months: Option<Decimal>,
    pub runway_is_indefinite: bool,
    /// Ingreso mensual efectivo del lado (`ProjectionInput::income_regular_monthly`): presupuesto
    /// en modos A y C, promedio real 12m en modo B.
    #[serde(with = "rust_decimal::serde::str")]
    pub income_monthly: Decimal,
    /// Gasto mensual **total** = gasto regular + servicio de deuda. Es la MISMA base que alimenta
    /// el runway y el target FIRE de este lado, y la que cuadra con
    /// `expense_total_monthly_equivalent` de `/v1/summary` en los tres modos. Ojo: no es
    /// `input.expense_regular_monthly` a secas — en modo A la cuota de pasivo vive deliberadamente
    /// fuera de esa base (`budget.rs`, para no contarla dos veces en la proyección) y entra aquí
    /// por `debt_service_monthly`.
    #[serde(with = "rust_decimal::serde::str")]
    pub expense_total_monthly: Decimal,
    /// Cuota mensual de los pasivos activos. Desde 4.8.0 (#142, opción 3 del owner) **viaja como
    /// número en los TRES modos**: en B/C la cuota declarada se RESTA del promedio real antes de
    /// alimentar el motor, así que la deuda vuelve a cobrarse por aquí y publicarla ya no es
    /// contarla dos veces. (Hasta 4.7.x en B/C era `null` con razón `included_in_real_expense` —
    /// aquel contrato de 3.4.0 murió con la anulación de la amortización que lo justificaba.)
    /// Un `0` significa «no hay pasivos con cuota activa».
    #[serde(with = "rust_decimal::serde::str_option")]
    pub debt_service_monthly: Option<Decimal>,
    /// Desde 4.8.0 siempre `null` (la cuota viaja en los tres modos — ver arriba). El campo se
    /// conserva por compatibilidad de forma; retirarlo es un breaking §5 aparte.
    pub debt_service_absent_reason: Option<&'static str>,
    /// El neto **recurrente** del lado — desde 4.8.0 (#127), el `recurring_net` del PRIMER PASO
    /// real del motor (`first_month_allocation`): ingreso − gasto − servicio de deuda REALMENTE
    /// pagado el mes 1 (`min(cuota, payoff)` + extra + comisión). En el caso común (principal
    /// holgado, sin extras) coincide con `income_monthly − expense_total_monthly` y la resta
    /// sigue siendo comprobable a mano; diverge a propósito cuando la cuota nominal ya no es lo
    /// que se paga (último mes de un plan, payoff parcial) — antes esas dos superficies
    /// publicaban dos «cajas del mes» distintas para la misma pregunta.
    ///
    /// Se llamaba `net_monthly` y era la trampa más cara de esta tool. `extra_monthly_savings` y
    /// `extra_monthly_cash_adjustment` no tocan el ingreso ni el gasto: entran como ajuste de caja
    /// mensual (el mismo mecanismo que un Próximo), así que este número sale **idéntico en
    /// baseline y escenario** y su delta es exactamente 0 — está dicho en el nombre (neto
    /// RECURRENTE) y el que se mueve es `net_cash_monthly`, ahí abajo.
    #[serde(with = "rust_decimal::serde::str")]
    pub net_recurring_monthly: Decimal,
    /// Ajuste de caja mensual **constante** que los overrides de este lado aplican a TODOS los
    /// meses del horizonte: `extra_monthly_savings − extra_monthly_cash_adjustment`.
    /// **Siempre `0` en el baseline** (el baseline no lleva overrides). No incluye el gasto
    /// puntual (`one_off_expense`, que afecta a un solo mes) ni los Próximos reales del hogar, que
    /// no son constantes y ya viven dentro de la simulación.
    #[serde(with = "rust_decimal::serde::str")]
    pub monthly_cash_adjustment: Decimal,
    /// La caja que la cascada reparte DE VERDAD el mes 1 — desde 4.8.0 (#127), el `base_cash`
    /// del motor (`first_month_allocation`): `net_recurring_monthly + planning_component`, donde
    /// el componente de planning lleva los Próximos del mes 1 Y el ajuste constante de los
    /// overrides; el extra de amortización y su comisión ya viven dentro del servicio de deuda
    /// real. **Es el campo que se mueve** cuando simulas ahorrar 200 € más al mes — o cuando
    /// amortizas deuda por encima de la cuota, que también deja de estar disponible para aportar.
    /// (Hasta 4.7.x se recalculaba aparte, sin Próximos y con la cuota nominal: 300 € de brecha
    /// con la resolución de la cascada en el escenario del issue #127.)
    ///
    /// Describe el MES 1, no todo el horizonte: el one-off cae en su mes, y el término de
    /// amortización extra se acaba con la deuda — desde ese mes la cuota liberada vuelve también
    /// a la cascada.
    #[serde(with = "rust_decimal::serde::str")]
    pub net_cash_monthly: Decimal,
    /// Suma de las amortizaciones extra MENSUALES de `liability_overrides` en este lado.
    /// **Siempre `0` en el baseline.** Va publicado aparte para que la resta de
    /// `net_cash_monthly` siga siendo comprobable a mano. No incluye los lump sums, que afectan a
    /// un solo mes (igual que `one_off_expense` queda fuera de `monthly_cash_adjustment`).
    #[serde(with = "rust_decimal::serde::str")]
    pub liability_extra_principal_monthly: Decimal,
    /// Comisión mensual por la amortización extra RECURRENTE (#151): Σ por pasivo de
    /// `extra_principal_monthly × early_repayment_fee_pct / 100`. Como su hermano de arriba, no
    /// incluye los lump sums (esos van en `liability_early_repayment_fee_total`). `0` en el
    /// baseline y con comisión 0.
    #[serde(with = "rust_decimal::serde::str")]
    pub liability_early_repayment_fee_monthly: Decimal,
    /// Comisión TOTAL por reembolso anticipado dentro del horizonte (#151), con el mismo
    /// calendario que el resto de agregados de deuda: Σ `total_early_repayment_fee` de los
    /// calendarios. Cubre lo recurrente Y los lump sums. Es el coste que hace que «¿me compensa
    /// amortizar?» tenga las dos columnas: interés ahorrado vs comisión pagada.
    #[serde(with = "rust_decimal::serde::str")]
    pub liability_early_repayment_fee_total: Decimal,
    /// Interés que los pasivos de este lado devengarán dentro del horizonte, con el mismo
    /// calendario que sirve `GET /v1/liabilities/{id}/schedule`. **Es la cifra que responde «¿me
    /// compensa amortizar antes?»**: su delta es el interés que el escenario NO paga. `0` cuando
    /// ningún pasivo devenga (todos `fixed_payments`, sin TIN, o sin plan de pago activo).
    #[serde(with = "rust_decimal::serde::str")]
    pub liability_total_interest: Decimal,
    /// Mes en que **todos** los pasivos de este lado quedan saldados (el máximo de sus meses de
    /// extinción). `0` = ya no hay deuda hoy. `null` ⟺ hay `liability_debt_free_absent_reason`.
    /// Es un número de MES desde `anchor_date_ymd`.
    pub liability_debt_free_month_index: Option<u32>,
    /// Por qué no hay mes libre de deuda: mismos códigos que el calendario
    /// (`no_payment_plan`, `payment_plan_ends_before_payoff`,
    /// `payment_does_not_reduce_principal`, `not_within_horizon`), del primer pasivo que no salda.
    /// `null` ⟺ hay `liability_debt_free_month_index`.
    pub liability_debt_free_absent_reason: Option<&'static str>,
    /// `net_recurring_monthly / income_monthly`, redondeado a 6 decimales igual que en
    /// `/v1/summary` (misma precisión en las dos superficies). `None` si no hay ingreso.
    /// Deliberadamente sobre el neto RECURRENTE: es la tasa de ahorro comparable con
    /// `financial_health.savings_rate`, que tampoco conoce ajustes de caja.
    #[serde(with = "rust_decimal::serde::str_option")]
    pub savings_rate: Option<Decimal>,

    // ---- Eco del contexto del lado (4.0.0, auditoría de simulate_projection §8) ------------------------------------
    // Todo lo de aquí abajo se calculaba ya dentro del ensamblado y se tiraba a la basura. Sin
    // ello, media respuesta se lee como un fallo: un `fire_target_base_delta: 0` es correcto en
    // `manual` e inexplicable sin saber el modo, y un override de `savings_source` que cae en el
    // fallback por falta de meses reales devuelve un escenario idéntico al baseline sin que nada
    // lo diga. Va **por lado** porque los overrides pueden hacer que difieran.
    /// Fuente del ahorro **efectiva** (tras el fallback: modo B/C sin meses reales → `budget`).
    /// Mismo tipo y mismos strings que `financial_health.savings_source` de `/v1/summary`.
    pub savings_source: SavingsSource,
    /// De dónde salió cada lado del ahorro y sobre cuántos meses reales. Con `basis: "budget"` el
    /// lado no promedió — es lo que distingue un escenario fundado de una extrapolación de un mes.
    pub savings_income_basis: SavingsAvgBasis,
    pub savings_expense_basis: SavingsAvgBasis,
    /// Modo con el que se calculó el target. Es lo que hace legible un `fire_target_base_delta`
    /// de 0: en `manual` el objetivo es un importe fijo y en `current_income` no mira el gasto,
    /// así que ningún override de gasto puede moverlo.
    pub fire_number_mode: FireNumberMode,
    /// Por qué no hay target, cuando no lo hay. `null` ⟺ sí lo hay.
    pub fire_target_absent_reason: Option<&'static str>,
    /// SWR efectivo del lado (%), tras el override. `0` anula el target entero.
    #[serde(with = "rust_decimal::serde::str")]
    pub swr_pct: Decimal,
    /// Inflación anual efectiva del lado (%), tras el override.
    #[serde(with = "rust_decimal::serde::str")]
    pub annual_inflation_percent: Decimal,
    /// Base de gasto regular que ha usado el modelo, **después** de overrides. No es
    /// `expense_total_monthly`: esta excluye el servicio de deuda. Es el número que revela qué
    /// recorte se aplicó de verdad.
    #[serde(with = "rust_decimal::serde::str")]
    pub expense_base_monthly: Decimal,
    /// Base de ingreso regular tras overrides (= `income_monthly`; se echa aparte para que el
    /// trío de bases se lea junto).
    #[serde(with = "rust_decimal::serde::str")]
    pub income_base_monthly: Decimal,
    /// Base de gasto **post-jubilación** tras overrides. En modo A es la que ancla el target FIRE.
    #[serde(with = "rust_decimal::serde::str")]
    pub expense_retirement_base_monthly: Decimal,
    /// Estrategia de jubilación con la que se simuló ESTE lado (5.0.0): `asap` | `retire_at_age`
    /// | `coast` | `partial` | `pension_bridge`. Va por lado porque el escenario puede llevar un
    /// perfil clonado y modificado, y sin el eco un `jubilacion_months_delta` de 0 no distingue
    /// «el eje no movió nada» de «la fecha la fija la edad, no el capital».
    pub strategy: String,
    /// Qué disparó la jubilación de este lado: `liquid_crossing` | `target_age` (D17). Con
    /// `target_age`, `jubilacion_month_index` es una edad cumplida y `fire_target_base` una
    /// referencia, no una meta alcanzada.
    pub retirement_trigger: &'static str,
    /// Mes en que el líquido alcanza el objetivo — LECTURA. Con `asap` coincide con
    /// `jubilacion_month_index`; con una estrategia por edad puede ser posterior (no llegas) o
    /// anterior (podrías haberte ido antes). `null` = no hay objetivo o no se cruza.
    pub liquid_crossing_month_index: Option<u32>,
    /// **Primer mes SIN el crecimiento de `income_growth_real_pct_annual`** (P11), en la rejilla
    /// de `points[].month_index`. `null` ⟺ el eje no se pidió (siempre en el baseline); cuando
    /// este lado no se jubila dentro del horizonte vale `horizon_months` —un índice una casilla
    /// más allá del último mes— y no `null`, para que «no se pidió» y «se aplicó entero» no
    /// compartan valor.
    ///
    /// Existe porque el corte NO es exacto y callarlo sería publicar una cifra sin base: se
    /// calcula con una PRIMERA pasada del escenario **sin** este eje, y el ingreso extra puede
    /// adelantar la jubilación respecto de ella. Si `jubilacion_month_index` acaba siendo menor
    /// que este número, los meses entre ambos llevan un sueldo que un jubilado no cobraría — la
    /// ventana es exactamente esa diferencia, y aquí está para poder medirla.
    pub income_growth_stops_at_month_index: Option<u32>,

    // ---- 5.0.0 WP5-2b — el PLAN de este lado (§B.3, §B.7) -----------------------------------
    // Van por lado porque `profile_overrides` puede cambiar la estrategia entera, y entonces las
    // dos columnas no describen el mismo plan. Los `null` NO son ceros: significan «esta
    // estrategia no responde a esa pregunta».
    /// Aportación mensual mínima para llegar al objetivo en la edad elegida. `null` con las
    /// estrategias por cruce (`asap`, `pension_bridge`), que no tienen edad contra la que
    /// resolver.
    #[serde(with = "rust_decimal::serde::str_option")]
    pub required_contribution_monthly: Option<Decimal>,
    /// Techo de la búsqueda del solve: el máximo sobrante mensual del horizonte. Es el
    /// denominador de «cuánto de mi margen se lleva el plan».
    #[serde(with = "rust_decimal::serde::str_option")]
    pub required_contribution_search_ceiling: Option<Decimal>,
    /// `true` ⟺ ni invirtiendo cada euro de sobrante se llega (D17, el rojo). **`null` = la
    /// pregunta no aplica**, nunca `false` para decir «no aplica».
    pub underfunded: Option<bool>,
    /// Margen mensual disponible (D16/D31), con la base de cada estrategia — la misma que
    /// declara `disposable_monthly` en `GET /v1/projection/series`.
    #[serde(with = "rust_decimal::serde::str_option")]
    pub disposable_monthly: Option<Decimal>,
    /// Mes a partir del cual se puede dejar de aportar y llegar igual (`coast`), en la rejilla.
    pub coast_fire_month_index: Option<u32>,
    /// El «número coast»: el líquido con el que se ENTRA en ese mes.
    #[serde(with = "rust_decimal::serde::str_option")]
    pub coast_number: Option<Decimal>,
    /// Capital que sostendría a perpetuidad el hueco de la media jornada. Informativo.
    #[serde(with = "rust_decimal::serde::str_option")]
    pub partial_gap_target: Option<Decimal>,
    /// `true` ⟺ hubo media jornada y el líquido no bajó ni un mes durante ella. `null` = no hubo.
    pub partial_phase_capital_growing: Option<bool>,
    /// Primer mes de media jornada, en la rejilla. `null` = no hay fase parcial.
    pub partial_retirement_month_index: Option<u32>,
    /// Primer mes con pensión CON FECHA, en la rejilla. `null` = no hay pensión con calendario.
    pub pension_start_month_index: Option<u32>,
    /// Qué FRACCIÓN del gasto cubre la pensión el mes en que empieza (`0.6` = 60 %).
    #[serde(with = "rust_decimal::serde::str_option")]
    pub pension_coverage_ratio: Option<Decimal>,
    /// Tasa de retirada efectiva del puente, en % ANUAL (`6.5` = 6,5 %).
    #[serde(with = "rust_decimal::serde::str_option")]
    pub bridge_effective_withdrawal_pct: Option<Decimal>,
    /// Tasa ANUAL (%) con la que el puente descontó sus flujos. `null` sin base puente.
    #[serde(with = "rust_decimal::serde::str_option")]
    pub bridge_discount_annual_pct: Option<Decimal>,
    /// Avisos de ESTE lado (mismos literales que `GET /v1/projection/series`), deduplicados:
    /// `birth_date_missing`, `target_retirement_age_missing`,
    /// `bridge_discount_no_liquid_assets`, `bridge_discount_clamped`,
    /// `retire_at_age_underfunded`, `coast_not_reachable`,
    /// `partial_phase_capital_shrinking`. Vacío = nada que advertir.
    pub warnings: Vec<String>,

    // ---- 5.0.0 WP6b — Monte Carlo de ESTE lado (P3, D22/D25/D28) ----------------------------
    // Todos son `null` cuando no se pidió el eje `monte_carlo` (`deltas.monte_carlo` presente ⟺
    // se pidió: es el campo que desambigua un `null` «no se preguntó» de un `null` con
    // significado), y van por lado porque el escenario puede llevar otra estrategia entera: la
    // probabilidad de éxito de dos planes distintos no se compara restando dos cifras que
    // describen cosas distintas — por eso el único delta es el de la probabilidad, y solo cuando
    // los dos lados la tienen.
    /// **Fracción de caminos en que el plan OCURRE y AGUANTA**: el hogar se jubila dentro del
    /// horizonte (o la estrategia es por EDAD, y entonces la jubilación es un dato y no un
    /// suceso) **Y** la cartera no se agota nunca. `null` ⟺ no se pidió `monte_carlo`.
    ///
    /// La definición cambió en el pase de correcciones de la revisión adversarial (hallazgo #7).
    /// Hasta entonces era solo «no se agota», y con un trigger por CRUCE eso premiaba al hogar
    /// que **no se jubila jamás**: quien trabaja hasta los 105 años sin llegar al objetivo nunca
    /// drena, así que nunca se agota. Medido sobre un hogar sintético: 0,960 publicados con el
    /// 33,1 % de los caminos sin jubilarse; hoy 0,629, con `never_retired_probability = 0,331` y
    /// `success_given_retired = 0,940` al lado.
    ///
    /// **No hay bandas aquí**: las series de percentil pesan (~19 KB a densidad hybrid) y un
    /// what-if devuelve DOS lados; el fan chart vive en `GET /v1/projection/bands`, que además
    /// lo cachea.
    #[serde(with = "rust_decimal::serde::str_option")]
    pub success_probability: Option<Decimal>,
    /// `green` | `amber` | `red` con el semáforo de D28, **de corte fijo desde 5.0.0** (V7):
    /// verde solo si NINGÚN camino agota la cartera.
    pub success_verdict: Option<&'static str>,
    /// **Fracción de caminos que NO se jubilan** dentro del horizonte. Con trigger por EDAD es
    /// `"0"` por construcción. Es el denominador escondido del éxito: un plan por cruce con una
    /// probabilidad alta y un tercio de caminos que no se jubilan nunca no es un buen plan, es un
    /// plan que no ocurre. `null` ⟺ no se pidió `monte_carlo`.
    #[serde(with = "rust_decimal::serde::str_option")]
    pub never_retired_probability: Option<Decimal>,
    /// **Éxito entre los caminos que SÍ se jubilan**: de los que llegan a la jubilación, cuántos
    /// no agotan la cartera. `null` cuando ningún camino se jubila —y también, como todos estos,
    /// cuando no se pidió el eje—. Junto a `success_probability` separa las dos preguntas que la
    /// definición vieja mezclaba: «¿ocurre el plan?» y «¿aguanta?».
    #[serde(with = "rust_decimal::serde::str_option")]
    pub success_given_retired: Option<Decimal>,
    /// Fracción de caminos que llegan a la edad objetivo por debajo del objetivo (D17). `null`
    /// también con el eje pedido si el plan de este lado no se jubila por edad.
    #[serde(with = "rust_decimal::serde::str_option")]
    pub underfunded_probability: Option<Decimal>,
    /// Mediana entre caminos del NÚMERO de meses jubilados en que el hogar no cubrió su gasto:
    /// cuenta el recorte de la regla (`withdrawal_shortfall`) **y** el gasto que la cartera no
    /// pudo financiar (`unmet_need`). `null` ⟺ no se pidió `monte_carlo`.
    ///
    /// Contar solo el recorte lo dejaba en 0 por construcción con `fixed_real` —la regla sin
    /// techo no recorta nunca— incluso en los caminos que se quedaban sin cartera: el mes sin
    /// dinero no aparecía en ninguna cifra.
    pub months_below_need_p50: Option<u32>,
    /// **Por qué NO se simuló el colchón de caja** en este lado. UN solo campo con motivos de
    /// dos capas, los mismos literales que `GET /v1/projection/bands`: del handler
    /// (`no_capped_rule`, `cap_is_zero`, `no_safe_liquid_asset`) y del motor (`no_volatility`).
    /// El `not_requested` del motor ya no se publica: desde que el colchón se DERIVA del tope de
    /// la regla de ahorro (V6), «no se pidió» no es un motivo. `null` = **sí se simuló** — o no
    /// se pidió el eje `monte_carlo`, que lo dice `deltas.monte_carlo`.
    pub buffer_inactive_reason: Option<&'static str>,
    /// **De dónde sale el colchón de ESTE lado**: `explicit` | `allocation_cap` | `none`. Va por
    /// lado y no en `MonteCarloKpis` porque `profile_overrides.cash_buffer_months` puede fijarlo
    /// solo en el escenario: publicar uno compartido describiría el colchón equivocado en la
    /// mitad de las simulaciones. `null` ⟺ no se pidió el eje `monte_carlo`.
    pub buffer_source: Option<&'static str>,
    /// El objetivo del colchón en euros **nominales**, sin indexar (P2). `null` salvo con
    /// `buffer_source: "allocation_cap"`.
    #[serde(with = "rust_decimal::serde::str_option")]
    pub buffer_target_amount: Option<Decimal>,
    /// Meses de gasto que cubre el colchón: los explícitos, o el equivalente **informativo**
    /// `floor(tope / gasto de jubilación)` cuando se deriva del tope.
    pub buffer_months_effective: Option<u32>,
    /// La regla de asignación cuyo tope fijó el objetivo. `null` salvo con `allocation_cap`.
    pub buffer_source_rule_id: Option<Uuid>,
    /// El activo que hace de colchón (el líquido σ = 0 de menor rentabilidad).
    pub buffer_source_asset_name: Option<String>,
}

#[derive(Debug, Serialize)]
pub(crate) struct SimDeltas {
    /// `scenario − baseline` en meses; None si alguno de los dos lados no alcanza el target.
    pub jubilacion_months_delta: Option<i64>,
    /// `scenario − baseline` del mes de agotamiento; None si alguno de los dos lados no se agota
    /// dentro del horizonte (misma regla que `jubilacion_months_delta`). Positivo = el escenario
    /// aguanta MÁS meses. (#119)
    pub assets_depleted_months_delta: Option<i64>,
    #[serde(with = "rust_decimal::serde::str")]
    pub final_net_worth_delta: Decimal,
    /// El mismo delta en euros de hoy — **solo cuando las dos inflaciones coinciden**. Cada lado se
    /// deflacta con la suya, así que en cuanto el eje simulado ES la inflación los dos deflactores
    /// dejan de ser el mismo y la resta no compara nada: simulando
    /// `annual_inflation_percent: "0"` sobre una instalación con inflación, este delta salía muy
    /// positivo mientras `final_net_worth_delta` salía muy negativo — misma magnitud, signos
    /// opuestos. Cualquier frase redactada con eso es basura, así que ahora es `null` y
    /// `real_delta_absent_reason` lo explica.
    #[serde(with = "rust_decimal::serde::str_option")]
    pub final_net_worth_real_delta: Option<Decimal>,
    /// `incomparable_deflators` ⟺ `final_net_worth_real_delta` es `null` (las inflaciones efectivas
    /// de baseline y escenario difieren). `null` ⟺ el delta real viaja. Mismo patrón que
    /// `fire_target_absent_reason` de `SimKpis`.
    pub real_delta_absent_reason: Option<&'static str>,
    #[serde(with = "rust_decimal::serde::str_option")]
    pub fire_target_base_delta: Option<Decimal>,
    #[serde(with = "rust_decimal::serde::str_option")]
    pub runway_months_delta: Option<Decimal>,
    #[serde(with = "rust_decimal::serde::str")]
    pub income_monthly_delta: Decimal,
    #[serde(with = "rust_decimal::serde::str")]
    pub expense_total_monthly_delta: Decimal,
    /// `scenario − baseline` del neto **recurrente**. Vale 0 para todo override que solo mueva
    /// caja (`extra_monthly_savings`, `extra_monthly_cash_adjustment`, `one_off_expense`): eso es
    /// correcto y ahora el nombre lo dice. El delta que responde a esos ejes es
    /// `net_cash_monthly_delta`.
    #[serde(with = "rust_decimal::serde::str")]
    pub net_recurring_monthly_delta: Decimal,
    /// `scenario − baseline` de la caja mensual estable. Con el baseline siempre a
    /// `monthly_cash_adjustment = 0`, esto es `net_recurring_monthly_delta + el ajuste del
    /// escenario`: el número que un agente debe citar cuando el usuario pregunta «¿y si ahorro X
    /// más al mes?».
    #[serde(with = "rust_decimal::serde::str")]
    pub net_cash_monthly_delta: Decimal,
    /// Diferencia de tasas calculada sobre los ratios **sin redondear** y redondeada al final:
    /// restar dos valores ya recortados a 6 dp propagaría el error de presentación al delta.
    #[serde(with = "rust_decimal::serde::str_option")]
    pub savings_rate_delta: Option<Decimal>,
    /// `scenario − baseline` de la amortización extra mensual. Con el baseline siempre a 0, es
    /// literalmente lo que pediste amortizar de más.
    #[serde(with = "rust_decimal::serde::str")]
    pub liability_extra_principal_monthly_delta: Decimal,
    /// `scenario − baseline` de la comisión total por reembolso anticipado (#151). Con el
    /// baseline siempre a 0, es la comisión del escenario: la columna de COSTE que se compara
    /// contra el interés ahorrado de abajo.
    #[serde(with = "rust_decimal::serde::str")]
    pub liability_early_repayment_fee_total_delta: Decimal,
    /// `scenario − baseline` del interés de los pasivos. **NEGATIVO = interés ahorrado**, que es
    /// la respuesta numérica a «¿me compensa amortizar antes?» — junto a la comisión de arriba:
    /// compensa si `|interés ahorrado| > comisión`.
    #[serde(with = "rust_decimal::serde::str")]
    pub liability_total_interest_delta: Decimal,
    /// `scenario − baseline` en meses hasta quedar libre de deuda. Negativo = te libras antes.
    /// `null` si alguno de los dos lados no llega a saldar dentro del horizonte.
    pub liability_debt_free_months_delta: Option<i64>,

    // ---- 5.0.0 WP5-2b — deltas del PLAN. `null` ⟺ alguno de los dos lados no publica la cifra
    // (la misma regla que `jubilacion_months_delta`): restar contra un «no aplica» inventaría un
    // número. Los `bool` (`underfunded`, `partial_phase_capital_growing`) NO tienen delta: se
    // leen comparando las dos columnas, y un «delta booleano» sería un tercer valor que
    // interpretar.
    /// `scenario − baseline` del ahorro mensual necesario. **Negativo = necesitas ahorrar
    /// menos**, que es la respuesta que el eje suele buscar.
    #[serde(with = "rust_decimal::serde::str_option")]
    pub required_contribution_monthly_delta: Option<Decimal>,
    /// `scenario − baseline` del margen mensual. Positivo = te sobra más cada mes.
    #[serde(with = "rust_decimal::serde::str_option")]
    pub disposable_monthly_delta: Option<Decimal>,
    /// `scenario − baseline` del mes coast. Negativo = puedes dejar de aportar antes.
    pub coast_fire_months_delta: Option<i64>,
    /// `scenario − baseline` del capital que sostiene el hueco de la media jornada.
    #[serde(with = "rust_decimal::serde::str_option")]
    pub partial_gap_target_delta: Option<Decimal>,
    /// `scenario − baseline` de la cobertura de la pensión, en FRACCIÓN.
    #[serde(with = "rust_decimal::serde::str_option")]
    pub pension_coverage_ratio_delta: Option<Decimal>,
    /// `scenario − baseline` de la tasa de retirada del puente, en PUNTOS PORCENTUALES.
    #[serde(with = "rust_decimal::serde::str_option")]
    pub bridge_effective_withdrawal_pct_delta: Option<Decimal>,
    /// **`scenario − baseline` de la probabilidad de éxito de Monte Carlo**, en FRACCIÓN
    /// (`0.12` = doce puntos porcentuales más de escenarios que aguantan). `null` ⟺ no se pidió
    /// el eje `monte_carlo`.
    ///
    /// Es comparable porque los dos lados sortean con la MISMA semilla: las realizaciones de
    /// mercado son idénticas y lo único que cambia entre las dos columnas es el plan, así que la
    /// diferencia mide el cambio y no el ruido de dos muestras. Es el único delta de este eje —
    /// el veredicto es un color (se lee comparando las dos columnas) y `months_below_need_p50` es
    /// una mediana de enteros cuya resta no significa nada estable.
    #[serde(with = "rust_decimal::serde::str_option")]
    pub success_probability_delta: Option<Decimal>,
}

/// **Lo que la pausa de ingresos le cuesta a la fecha de jubilación** (P8.c). Los dos meses viajan
/// al lado del delta a propósito: «la pausa te saca del horizonte» es una respuesta legítima, y
/// publicarla como un retraso enorme sería inventarse una cifra.
#[derive(Debug, Serialize)]
pub(crate) struct IncomePauseKpis {
    /// Mes de jubilación del escenario **sin** la pausa, en la rejilla. `null` = no se jubila
    /// dentro del horizonte.
    pub baseline_month_index: Option<u32>,
    /// Mes de jubilación del escenario **con** la pausa, en la rejilla. Coincide con
    /// `scenario.jubilacion_month_index`, que es el lado que la lleva aplicada.
    pub paused_month_index: Option<u32>,
    /// `paused − baseline` en meses. **`null` cuando alguno de los dos no se jubila dentro del
    /// horizonte.**
    pub retirement_delay_months: Option<i64>,
}

/// Series decimadas (hybrid) opt-in — números f64 como el resto de series de chart.
#[derive(Debug, Serialize)]
pub(crate) struct SimSeries {
    pub month_indices: Vec<u32>,
    pub baseline_net_worth: Vec<f64>,
    pub scenario_net_worth: Vec<f64>,
    /// **Gasto del mes que los activos NO pudieron financiar** en el lado baseline, neto, `≥ 0` y
    /// a la escala monetaria (el espejo de `points[].unmet_need` de `GET /v1/projection/series`,
    /// con su mismo redondeo de publicación). Viaja porque es la
    /// única columna que dice DÓNDE deja de cubrirse el plan: `assets_depleted_month_index` da un
    /// mes y `uncovered_deficit_total` un total, y entre los dos no se ve el perfil del hueco.
    ///
    /// **No es el recorte de la regla** (`withdrawal_shortfall`, que no viaja aquí): con
    /// `fixed_real` el recorte es cero por construcción y esta serie sigue siendo la que se llena.
    pub baseline_unmet_need: Vec<f64>,
    /// Lo mismo del lado escenario. Restarlas punto a punto es legítimo: los dos lados comparten
    /// rejilla, ancla y horizonte.
    pub scenario_unmet_need: Vec<f64>,
}

#[derive(Debug, Serialize)]
pub(crate) struct SimulateProjectionResponse {
    pub horizon_months: u32,
    /// De dónde sale el horizonte: `lifespan_age` (hasta `horizon_lifespan_age` años por fecha
    /// de nacimiento — hasta 4.8.0 el literal era `lifespan_90`), `fallback_no_demographics`
    /// (30 años, no hay ninguna) o `months_override` (lo pediste tú). Sin él, un horizonte de
    /// 360 meses no se distingue de uno elegido a ciegas.
    pub horizon_basis: String,
    /// Eco de la edad límite configurada (#149) — misma semántica que en la serie.
    pub horizon_lifespan_age: u32,
    /// Vista efectivamente aplicada: `household` | `mine`. Eco de `view`.
    pub view: &'static str,
    /// Mes 0 de la simulación (`YYYY-MM-DD`), en el calendario de la instalación. Va aquí para que
    /// la respuesta sea autocontenida: sin ancla, convertir un índice de mes obligaba a encadenar
    /// una llamada a `get_projection`.
    pub anchor_date_ymd: String,
    /// `dates` | `ages`: preferencia de la instalación para presentar el eje temporal.
    pub show_age_mode: String,
    /// Fecha de nacimiento con la que se resolvieron las edades. `null` si no hay ninguna.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub viewer_birth_date: Option<String>,
    /// Supuestos del modelo con los que hay que leer los deltas. Ver [`SIMULATE_MODEL_NOTE`].
    pub model_note: String,
    pub baseline: SimKpis,
    pub scenario: SimKpis,
    pub deltas: SimDeltas,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub series: Option<SimSeries>,
    /// Presente ⟺ se pidió el eje `income_pause` (P8.c). Ausente = no se preguntó.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub income_pause: Option<IncomePauseKpis>,
    /// **P8.b — el mayor gasto mensual extra CONSTANTE, en euros de hoy, que deja la fecha de
    /// jubilación donde está** (±1 mes). Presente ⟺ se pidió `solve.extra_monthly_expense_keeping_date`.
    ///
    /// Sube solo el gasto REGULAR (el de la fase de acumulación), no el de jubilación ni la
    /// necesidad que el objetivo capitaliza: la pregunta es «¿cuánto margen tengo AHORA?», no
    /// «¿cuánto puedo subir mi nivel de vida para siempre?». Cuando ni gastándose el sobrante
    /// entero se mueve la fecha —lo normal con un trigger por EDAD, que no depende del gasto—,
    /// la respuesta es el máximo sobrante mensual: un SUELO honesto («al menos esto»), no un
    /// infinito inventado. `null` ⟺ el escenario base no se jubila dentro del horizonte.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(with = "rust_decimal::serde::str_option")]
    pub max_extra_monthly_expense_keeping_date: Option<Decimal>,
    /// Presente ⟺ se pidió el eje `monte_carlo`. Ecoa lo que hace falta para REPETIR el sorteo y
    /// para leerlo; las cifras por lado viven en `baseline`/`scenario`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub monte_carlo: Option<MonteCarloKpis>,
}

/// El eco del sorteo. **La semilla es lo que convierte una probabilidad en un resultado**: sin
/// ella nadie puede repetir la ejecución, y dos llamadas con semillas distintas no son
/// comparables aunque sus deltas lo parezcan.
#[derive(Debug, Serialize)]
pub(crate) struct MonteCarloKpis {
    /// Caminos sorteados **por lado** (el coste total es el doble).
    pub paths: u32,
    /// Semilla efectiva, **como cadena de dígitos**: es un entero de 64 bits y `JSON.parse` lo
    /// redondea por encima de 2^53. Misma convención que `GET /v1/projection/bands`.
    pub seed: String,
    /// `false` ⟺ ningún activo declara volatilidad: entonces los dos lados sortean el camino
    /// determinista y la probabilidad de éxito solo puede ser `1` o `0`. Sin este campo, un
    /// `success_probability: "1"` se leería como «tu plan es seguro» cuando significa «no has
    /// declarado ninguna volatilidad».
    pub any_volatility_declared: bool,
}

fn require_non_negative(name: &str, v: Option<Decimal>) -> Result<Decimal, ApiError> {
    let v = v.unwrap_or(Decimal::ZERO);
    if v < Decimal::ZERO {
        return Err(ApiError::BadRequest(format!("amount_negative: {name} must be >= 0")));
    }
    Ok(v)
}

/// KPIs de un lado de la simulación (baseline o escenario).
/// `input` es el que se SIMULÓ de verdad (en el escenario, el clon ya mutado por los overrides
/// post-build); `built` es el ensamblado de ese mismo lado, del que salen el servicio de deuda y
/// el eco de contexto. Van separados a propósito: `built.input` es el de antes de mutar.
#[allow(clippy::too_many_arguments)]
fn sim_kpis(
    input: &ProjectionInput,
    output: &futurefin_engine::ProjectionOutput,
    built: &BuiltProjection,
    inflation_annual_percent: Decimal,
    fs: &FireSettings,
    profile: &RetirementProfile,
    today: NaiveDate,
    birth_date: Option<NaiveDate>,
    // `monthly_cash_adjustment`: ajuste de caja mensual constante de ESTE lado (0 en el baseline).
    // Se pasa desde el llamante, que es quien lo aplicó al `planning_monthly_cash_adjustment` del
    // input: derivarlo aquí a partir del array significaría adivinar qué parte de él es el
    // override y qué parte son los Próximos reales del hogar.
    monthly_cash_adjustment: Decimal,
    // Primer mes de la rejilla SIN el crecimiento de ingreso (P11). Igual que el anterior: lo
    // sabe el llamante, que es quien construyó el vector, y aquí solo se ecoa.
    income_growth_stops_at_month_index: Option<u32>,
    // Los solves de §B.7 de ESTE lado, ya calculados por el llamante (que es quien puede meterlos
    // en el semáforo de CPU): `sim_kpis` es una función de ensamblado, no un sitio donde lanzar
    // 26 proyecciones.
    solves: &StrategySolves,
) -> SimKpis {
    let debt_service_monthly = built.debt_service_monthly;
    // 5.0.0 (R8): el mes publicado es el EFECTIVO del motor, traducido a la rejilla — con `asap`
    // es exactamente el cruce de 4.15.x y ningún delta se mueve. El objetivo que se LEE sale de
    // `built.fire_target_reading` y no de `input.fire_target`: con una estrategia por edad el
    // input no lleva objetivo (D17) y el KPI se quedaría sin base que citar.
    let reading_target = built
        .fire_target_reading
        .as_ref()
        .filter(|ft| futurefin_engine::fire_target_base_at_month_index(ft, 0).is_some());
    let jubilacion_month_index = engine_month_to_grid(output.retirement_month_index);
    // El cruce lo lee el MOTOR (WP5-2b): evalúa el objetivo consciente del plan —puente
    // incluido—, que es el mismo contra el que se mide todo lo demás de este lado.
    let liquid_crossing_month_index = engine_month_to_grid(output.liquid_crossing_month_index);
    let (jubilacion_date_ymd, jubilacion_age) =
        jubilacion_civil(today, birth_date, jubilacion_month_index);
    let final_net_worth = output.net_worth.last().copied().unwrap_or(Decimal::ZERO);
    // El índice de mes del último punto es `len - 1` porque `output.net_worth` es la serie mensual
    // COMPLETA (la decimación a hybrid ocurre después, al serializar). Se toma explícitamente y no
    // como «la posición del último elemento»: si algún día esta cifra saliera de una serie ya
    // decimada, la posición dejaría de ser el mes y deflactaríamos 70 años como si fueran 30.
    let final_month_index = output.net_worth.len().saturating_sub(1) as u32;
    let final_net_worth_real =
        final_net_worth * deflator_at_month_index(inflation_annual_percent, final_month_index);

    // Runway: misma fórmula que `/v1/summary` (gasto total = regular + servicio de deuda;
    // infinito ⟺ SWR sobre el gasto anual grosseado Y rentabilidad esperada ponderada > 0,
    // drenaje secuencial en el caso finito — #128), evaluada sobre los inputs del lado.
    // #178: la base declarada (purchase_price) viaja al bucle finito — misma regla que el
    // summary: None = sin coste ⇒ escalar; el umbral SWR sigue con el escalar (perpetuidad).
    let liquid_rows: Vec<(Decimal, Option<Decimal>, Option<Decimal>)> = input
        .assets
        .iter()
        .filter(|a| a.is_liquid)
        .map(|a| {
            (
                a.value.max(Decimal::ZERO),
                a.expected_annual_return_percent,
                a.purchase_price,
            )
        })
        .collect();
    let monthly_expense = input.expense_regular_monthly + debt_service_monthly;
    // #140 fase 2: el umbral del runway pasa g — la misma venta y el mismo impuesto que el
    // objetivo; dejarlo a g=1 reabriría la asimetría en otra tarjeta.
    let annual_expense_gross = gross_up_net_annual_fire(
        monthly_expense * Decimal::from(12u32),
        &fs.tax_brackets,
        fs.taxes_enabled,
        fs.taxable_gain_ratio,
    );
    let (runway_months, runway_is_indefinite) = match futurefin_engine::liquid_runway_months(
        &liquid_rows,
        monthly_expense,
        inflation_annual_percent,
        profile.swr_pct,
        annual_expense_gross,
        &fs.tax_brackets,
        fs.taxes_enabled,
        fs.taxable_gain_ratio,
    ) {
        futurefin_engine::RunwayOutcome::Months(m) => (Some(m.round_dp(1)), false),
        futurefin_engine::RunwayOutcome::Indefinite => (None, true),
        futurefin_engine::RunwayOutcome::NoExpenseBase => (None, false),
    };

    let income_monthly = input.income_regular_monthly;
    // #127 (4.8.0): las dos cifras de «caja del mes» convergen al PRIMER PASO real del motor
    // (`first_month_allocation`): el servicio de deuda es el que se paga de verdad
    // (min(cuota, payoff) + extra + comisión, no la cuota nominal) y la caja del mes incluye el
    // tramo de Próximos del mes 1 — antes sim_kpis recalculaba con la cuota nominal y sin
    // planning flows, y las dos superficies publicaban dos números distintos para la misma
    // pregunta (300 € de brecha en el escenario del issue). Fallback a la fórmula nominal solo
    // si el primer paso fallara (misma validación que la serie, que ya pasó — no debería).
    let fma = futurefin_engine::first_month_allocation(input).ok();
    let net_recurring_monthly = fma
        .as_ref()
        .map(|f| f.recurring_net)
        .unwrap_or(income_monthly - monthly_expense);
    let savings_rate = (income_monthly > Decimal::ZERO)
        .then(|| (net_recurring_monthly / income_monthly).round_dp(SIM_RATIO_DP));

    // Agregados de deuda de ESTE lado, con el MISMO calendario que sirve
    // `GET /v1/liabilities/{id}/schedule` — cero matemática nueva, y por tanto imposible que la
    // tool del calendario y el what-if den meses de extinción distintos para los mismos datos.
    // Horizonte = el de la simulación: «libre de deuda» tiene que significar lo mismo aquí que en
    // la serie que se está pintando.
    let liability_extra_principal_monthly: Decimal = input
        .liabilities
        .iter()
        .map(|l| l.extra_principal_monthly.max(Decimal::ZERO))
        .sum();
    let liability_early_repayment_fee_monthly: Decimal = input
        .liabilities
        .iter()
        .map(|l| {
            l.extra_principal_monthly.max(Decimal::ZERO)
                * l.early_repayment_fee_pct.unwrap_or(Decimal::ZERO).max(Decimal::ZERO)
                / Decimal::from(100)
        })
        .sum();
    let mut liability_early_repayment_fee_total = Decimal::ZERO;
    let mut liability_total_interest = Decimal::ZERO;
    // `Some(0)` con cero pasivos: no deber nada es estar libre de deuda hoy, no «no se sabe».
    let mut liability_debt_free_month_index = Some(0u32);
    let mut liability_debt_free_absent_reason: Option<&'static str> = None;
    for l in &input.liabilities {
        let sch = futurefin_engine::liability_amortization_schedule(
            l,
            input.ref_date,
            input.horizon_months,
        );
        liability_total_interest += sch.total_interest;
        liability_early_repayment_fee_total += sch.total_early_repayment_fee;
        match (sch.payoff_month_index, sch.payoff_absent) {
            (Some(k), _) => {
                // El hogar queda libre de deuda cuando cae el ÚLTIMO pasivo.
                liability_debt_free_month_index =
                    liability_debt_free_month_index.map(|acc| acc.max(k));
            }
            (None, absent) => {
                liability_debt_free_month_index = None;
                if liability_debt_free_absent_reason.is_none() {
                    liability_debt_free_absent_reason = absent.map(payoff_absence_code);
                }
            }
        }
    }

    SimKpis {
        jubilacion_month_index,
        // #210 — misma rejilla 0-based que el resto de `*_month_index` desde 5.0.0.
        assets_depleted_month_index: engine_month_to_grid(output.assets_depleted_month_index),
        unallocated_savings_total: money_out(output.unallocated_savings_total),
        unallocated_savings_reason: unallocated_reason_of(
            &input.assets,
            output.unallocated_savings_total,
        ),
        jubilacion_date_ymd,
        jubilacion_age,
        final_net_worth: money_out(final_net_worth),
        final_net_worth_real: money_out(final_net_worth_real),
        // La base del objetivo del PLAN en el mes 0: `T(0) − deuda(0)`. Sin pensión con fecha es
        // exactamente `fire_target_base_at_month_index(ft, 0)` por construcción; con base PUENTE
        // es la única lectura que cuadra con la línea que el chart pinta.
        fire_target_base: reading_target
            .filter(|ft| futurefin_engine::fire_target_base_at_month_index(ft, 0).is_some())
            .and_then(|ft| {
                let debt0 = ft
                    .debt_payments_remaining
                    .first()
                    .copied()
                    .unwrap_or(Decimal::ZERO);
                fire_target_at_month_index_with_plan(Some(ft), &input.phase_plan, 0)
                    .map(|t| money_out((t - debt0).max(Decimal::ZERO)))
            }),
        runway_months,
        runway_is_indefinite,
        income_monthly: money_out(income_monthly),
        expense_total_monthly: money_out(monthly_expense),
        // `null` + razón cuando la cuota ya vive dentro del gasto real: la aritmética interna
        // (`monthly_expense`) sigue usando el `Decimal` — es solo la PUBLICACIÓN la que distingue
        // «cero euros de cuota» de «esta cifra no existe en este modo».
        debt_service_monthly: built
            .debt_service_absent_reason
            .is_none()
            .then(|| money_out(debt_service_monthly)),
        debt_service_absent_reason: built.debt_service_absent_reason,
        net_recurring_monthly: money_out(net_recurring_monthly),
        monthly_cash_adjustment: money_out(monthly_cash_adjustment),
        // #127: la caja que la cascada reparte DE VERDAD el mes 1 (base_cash del motor). La
        // identidad comprobable pasa a ser `net_cash = recurring_net + planning_component`
        // (flows del mes + ajuste del override − retirada), con el extra y la comisión ya
        // dentro del servicio de deuda real.
        net_cash_monthly: money_out(
            fma.as_ref().map(|f| f.base_cash).unwrap_or(
                net_recurring_monthly + monthly_cash_adjustment
                    - liability_extra_principal_monthly
                    - liability_early_repayment_fee_monthly,
            ),
        ),
        liability_extra_principal_monthly: money_out(liability_extra_principal_monthly),
        liability_early_repayment_fee_monthly: money_out(liability_early_repayment_fee_monthly),
        liability_early_repayment_fee_total: money_out(liability_early_repayment_fee_total),
        liability_total_interest: money_out(liability_total_interest),
        liability_debt_free_month_index,
        liability_debt_free_absent_reason,
        savings_rate,
        savings_source: built.effective_savings_source,
        savings_income_basis: built.savings_income_basis.clone(),
        savings_expense_basis: built.savings_expense_basis.clone(),
        fire_number_mode: profile.fire_number_mode,
        fire_target_absent_reason: built.fire_target_absent_reason,
        swr_pct: profile.swr_pct,
        annual_inflation_percent: inflation_annual_percent,
        // `money_out` obligatorio: en modo B estas bases salen de `suma / n` y arrastran la escala
        // que `rust_decimal` produce en una división (auditoría MCP §7).
        expense_base_monthly: money_out(input.expense_regular_monthly),
        income_base_monthly: money_out(income_monthly),
        expense_retirement_base_monthly: money_out(input.phase_plan.expense_retirement_monthly),
        strategy: strategy_label(profile.strategy),
        retirement_trigger: built.retirement_trigger,
        liquid_crossing_month_index,
        income_growth_stops_at_month_index,
        required_contribution_monthly: solves.required.as_ref().map(|s| money_out(s.contribution)),
        required_contribution_search_ceiling: solves
            .required
            .as_ref()
            .map(|s| money_out(s.search_ceiling)),
        underfunded: solves.required.as_ref().map(|s| s.underfunded),
        disposable_monthly: disposable_monthly_of(solves),
        coast_fire_month_index: solves
            .coast
            .as_ref()
            .and_then(|c| engine_month_to_grid(c.coast_month_index)),
        coast_number: solves.coast.as_ref().and_then(|c| c.coast_number).map(money_out),
        partial_gap_target: output.partial_gap_target.map(money_out),
        partial_phase_capital_growing: output
            .partial_retirement_month_index
            .map(|_| output.partial_phase_capital_growing),
        partial_retirement_month_index: engine_month_to_grid(
            output.partial_retirement_month_index,
        ),
        pension_start_month_index: engine_month_to_grid(output.pension_start_month_index),
        pension_coverage_ratio: output.pension_coverage_ratio.map(money_out),
        bridge_effective_withdrawal_pct: output.bridge_effective_withdrawal_pct.map(money_out),
        bridge_discount_annual_pct: built.bridge_discount_annual_pct.map(money_out),
        warnings: merge_warnings(&built.warnings, output, solves),
        // El eje `monte_carlo` los rellena DESPUÉS, en el core: `sim_kpis` es una función de
        // ensamblado y lanzar aquí 2·paths simulaciones la sacaría del semáforo de CPU.
        success_probability: None,
        success_verdict: None,
        never_retired_probability: None,
        success_given_retired: None,
        underfunded_probability: None,
        months_below_need_p50: None,
        buffer_inactive_reason: None,
        buffer_source: None,
        buffer_target_amount: None,
        buffer_months_effective: None,
        buffer_source_rule_id: None,
        buffer_source_asset_name: None,
    }
}

/// Nota de modelo de `simulate_projection`. Era la única tool de proyección sin ella, y la que
/// más la necesita: es la única que deja **mover los supuestos**, así que sus deltas se pueden
/// leer como una predicción cuando son la consecuencia mecánica de un cambio de hipótesis.
///
/// El caso que la motiva: simular `annual_inflation_percent: "0"` adelanta la jubilación años. No
/// porque el plan mejore, sino porque el motor capitaliza en NOMINAL y solo el objetivo FIRE crece
/// con la inflación — bajarla sube la rentabilidad real de todos los activos y congela el objetivo
/// a la vez, gratis, en el mismo movimiento. Lo mismo, en pequeño, con `swr_pct`.
const SIMULATE_MODEL_NOTE: &str = "What-if sin persistir: se simulan dos veces los MISMOS datos (baseline y escenario) con el mismo horizonte, ancla y calendario; los deltas son escenario − baseline. QUIÉN DECIDE QUÉ: la ESTRATEGIA del perfil elige el disparador de la jubilación (`retirement_trigger`: cruce del líquido con el objetivo, o la EDAD en `retire_at_age`/`coast` — llegue o no el capital; entonces el cruce es solo lectura, `liquid_crossing_month_index`) y la base del objetivo (`perpetuity` o `bridge_to_pension`). El SWR solo DIMENSIONA el objetivo (gasto anual grosseado / SWR), no gobierna lo que se retira: subirlo baja el objetivo por división, no por ahorrar más. Ya jubilado manda `withdrawal_rule`; por defecto `fixed_real` = la necesidad declarada, indexada y SIN techo. TRES magnitudes por mes y no dos: lo que se obtuvo (`withdrawal`), lo que la REGLA rechazó (`withdrawal_shortfall`: informativo, no resta patrimonio ni es un fracaso, y cero por construcción con `fixed_real`) y lo que la CARTERA no pudo dar (`unmet_need`, en `series` por lado); `withdrawal_excess` es lo que se retira de más en `rule_is_spend`. `assets_depleted_month_index` exige DOS condiciones — cartera a cero Y alguna venta posterior sin fundar—, así que un aterrizaje exacto sobre una pensión que cubre todo el gasto es `null`. Con el eje `monte_carlo`, ÉXITO = el plan OCURRE y AGUANTA (se jubila dentro del horizonte, o la estrategia es por edad, Y la cartera nunca se agota); `never_retired_probability` y `success_given_retired` separan las dos preguntas, y `months_below_need_p50` cuenta los meses jubilados sin cubrir el gasto — recorte de la regla MÁS descubierto de la cartera. EL PLAN ES UN EJE: `profile_overrides` cambia estrategia, edad objetivo, modo y objetivo manual del número FIRE, base del objetivo, regla de retirada, pensión con fecha y media jornada sobre un CLON del perfil (mismas cotas que guardarlo; nada se persiste), y por eso las lecturas del plan viajan POR LADO: `required_contribution_monthly` (el ahorro mínimo que llega a la edad), su techo de búsqueda, `underfunded` (ni ahorrándolo todo llegas — el rojo de D17), `disposable_monthly`, `coast_fire_month_index` y `coast_number`, `partial_gap_target`, `pension_coverage_ratio` (FRACCIÓN) y `bridge_effective_withdrawal_pct` (PORCENTAJE anual). Un `null` ahí NO es un cero: significa que esa estrategia no responde a esa pregunta, y por eso su delta también sale `null` en vez de restar contra un hueco. `income_pause` multiplica el ingreso GANADO durante su ventana —la pensión con fecha NO se pausa— y publica en `income_pause` los dos meses de jubilación y su diferencia; con «no se jubila dentro del horizonte» en cualquiera de los dos lados, `retirement_delay_months` es `null` en vez de un retraso inventado. `solve: {extra_monthly_expense_keeping_date: true}` responde «¿cuánto más puedo gastar sin mover la fecha?» subiendo solo el gasto REGULAR (no el de jubilación ni el objetivo), y con un trigger por EDAD —que no depende del gasto— devuelve el máximo sobrante mensual como SUELO honesto, no como infinito. `view=household` no se simula (400 `household_not_simulable`): el agregado del hogar es informativo y no es el plan de nadie. El motor capitaliza en euros NOMINALES; la inflación indexa el GASTO mes a mes (regular y de jubilación, eje (k−1)/12: el mes 1 cobra el gasto declarado tal cual) y el objetivo, y deja los INGRESOS planos a propósito. Bajar `annual_inflation_percent` abarata TODO el gasto futuro, sube la rentabilidad real de los activos Y frena el objetivo a la vez: puede adelantar la jubilación años sin que nada del plan haya mejorado — léelo como un cambio de supuesto, no como una mejora. Admite negativos hasta −2 (deflación sostenida: gasto y objetivo DECRECEN). Los ejes de CAJA (`extra_monthly_savings`, `extra_monthly_cash_adjustment`, `one_off_expense`, `income_growth_real_pct_annual`, `income_steps`) NO tocan ingreso ni gasto: mueven `net_cash_monthly`, y `net_recurring_monthly` y `savings_rate` salen con delta 0 EXACTO por diseño. `income_growth_real_pct_annual` añade `ingreso · ((1+g)^((k−1)/12) − 1)` al mes k y solo mientras el escenario NO está jubilado; el corte se calcula con una PRIMERA pasada del escenario sin el eje, así que es aproximado: si el sueldo extra adelanta la jubilación, los meses entre `scenario.jubilacion_month_index` y `scenario.income_growth_stops_at_month_index` llevan una nómina que un jubilado no cobraría — esa diferencia es la ventana, y los dos números viajan para poder medirla. Los `income_steps` NO se recortan en la jubilación: el mes lo nombra el llamante. `final_net_worth` está en euros nominales del último mes del horizonte; para comparar poder adquisitivo usa `final_net_worth_real`, y solo cuando `deltas.real_delta_absent_reason` es null. `liability_overrides` amortiza deuda: el importe sale de la caja del mes Y baja el principal a la vez, así que el efecto instantáneo sobre el patrimonio es CERO salvo por la compensación por reembolso anticipado (default 2 % del extra, techo legal a tipo fijo de la Ley 5/2019 art. 23; `early_repayment_fee_pct: \"0\"` la quita): esa comisión sale de la caja y NO amortiza. No se modela la caída al 1,5 % tras el año 10, ni los topes de tipo variable (0,25 %/0,15 %), ni el límite de la pérdida financiera del prestamista: si tu préstamo es variable o veterano, pasa el % que te aplique. `early_repayment_effect: \"reduce_payment\"` baja la cuota en vez de acortar el plazo — con una amortización PUNTUAL el mes de extinción se conserva EXACTAMENTE; con amortización extra RECURRENTE puede adelantarse algo (nunca atrasarse). Lo que se gana está en `liability_total_interest_delta` (negativo = interés que ya no se devenga; compara contra `liability_early_repayment_fee_total_delta`, el coste) y en que la cuota liberada vuelve sola a la cascada, no en un salto de patrimonio el día que amortizas. Si el pasivo no devenga intereses (`fixed_payments`, o sin TIN), amortizar antes no mejora nada y el escenario lo dirá con deltas a cero.";

/// Nota de modelo de `GET /v1/projection/series` (P6, 5.0.0).
///
/// Dice **quién decide qué**, que es lo que la versión de 4.15.x no decía: hasta 5.0.0 la
/// jubilación era un cruce y el SWR parecía gobernarlo todo. Ahora hay tres piezas distintas —la
/// estrategia elige el disparador y la base del objetivo, el SWR solo dimensiona ese objetivo, y
/// la regla de retirada gobierna lo que sale de la cartera— y confundirlas produce lecturas
/// plausibles y falsas. Es constante y no un `format!` para que sea la MISMA cadena en cada
/// respuesta (un `model_note` que cambia entre llamadas es ruido en la cache y en el diff).
const PROJECTION_MODEL_NOTE: &str = "Motor mensual en euros NOMINALES: presupuesto regular sin las cuotas derivadas de pasivos, servicio de deuda por mes, ajustes por Próximos y crecimiento compuesto por activo. El GASTO (regular y de jubilación) se indexa a la inflación de la instalación con el eje (k−1)/12 —el mes 1 cobra el gasto declarado tal cual— y los INGRESOS quedan planos a propósito. Quien decide la jubilación es la ESTRATEGIA del perfil, y decide DOS cosas: el disparador (`retirement_trigger`: el cruce del líquido con el objetivo, o la EDAD en `retire_at_age`/`coast`, llegue o no el capital — entonces el cruce pasa a ser lectura en `liquid_crossing_month_index`) y la base del objetivo (`perpetuity`, o `bridge_to_pension` = capital para llegar a la pensión más la perpetuidad sobre lo que la pensión no cubra). El SWR solo DIMENSIONA ese objetivo (gasto anual grosseado / SWR); no gobierna lo que se retira. Ya jubilado manda `withdrawal_rule`: por defecto `fixed_real`, es decir la necesidad declarada, indexada y SIN techo. TRES magnitudes por mes y no dos: `withdrawal` es lo que se obtuvo, `withdrawal_shortfall` lo que la REGLA rechazó (informativo: no resta patrimonio ni cuenta como fracaso; cero por construcción con `fixed_real`, que no tiene techo) y `unmet_need` lo que la CARTERA no pudo dar ese mes — su suma es la necesidad neta, y `uncovered_deficit_total` es la suma de la tercera en todo el horizonte. `withdrawal_excess` es lo que se retira de más en modo `rule_is_spend`. `assets_depleted_month_index` exige DOS condiciones: que la venta dejara la cartera a cero Y que alguna venta posterior se quedara sin fundar — un puente que se vacía EXACTAMENTE el mes en que entra una pensión que cubre todo el gasto es `null`, no una ruina. Con una estrategia por EDAD el objetivo entra igual al motor pero no dispara nada, y el plan publica además lo que cuesta llegar: `required_contribution_monthly` (el ahorro mínimo), `required_capital_path` (la serie líquida SIMULADA de esa aportación, no un descuento escalar), `disposable_monthly`/`disposable_capital` (el margen por encima de lo necesario) y `underfunded` cuando ni ahorrándolo todo se llega. Con `coast`, `coast_fire_month_index` y `coast_path` dicen desde cuándo se puede dejar de aportar. Con pensión CON FECHA, `pension_coverage_ratio` es una FRACCIÓN y `bridge_effective_withdrawal_pct` un PORCENTAJE anual. Con `view=household` la curva es la SUMA de una simulación independiente por miembro, cada una con su estrategia: una lectura INFORMATIVA del conjunto, no el plan de nadie, y por eso el hogar no publica jubilación propia (el hito de cada uno va en `members[]`).";

/// Cotas del eje P11 `income_growth_real_pct_annual` (% REAL anual, 5.0.0).
///
/// El techo es 20 y no 50 porque esto no es una rentabilidad: es la subida SOSTENIDA del sueldo
/// por encima de la inflación, año tras año y durante todo el horizonte. Un 20 % real anual
/// multiplica el ingreso por 38 en veinte años — ya es el límite de lo que se puede pedir sin
/// que la pregunta deje de describir una carrera profesional. El suelo negativo existe porque
/// «¿y si mi sueldo pierde poder adquisitivo un 2 % al año?» es exactamente la misma pregunta.
const MIN_INCOME_GROWTH_PCT: Decimal = Decimal::from_parts(10, 0, 0, true, 0);
const MAX_INCOME_GROWTH_PCT: Decimal = Decimal::from_parts(20, 0, 0, false, 0);

/// Tope de `income_steps`. Veinticuatro escalones cubren dos décadas de cambios anuales; por
/// encima, lo que el usuario está describiendo es una serie, y una serie se declara en el
/// presupuesto, no en un what-if.
const MAX_INCOME_STEPS: usize = 24;

/// Decimales de `SimKpis::savings_rate`. Debe seguir siendo el mismo que el `RATIO_DP` de
/// `handlers/summary.rs`: si divergen, se reabre la incoherencia de precisión entre superficies que
/// el redondeo de 3.8.0 vino a cerrar (le pasó a `runway_months` durante dos versiones).
const SIM_RATIO_DP: u32 = 6;

/// What-if de proyección/FIRE sin persistir nada. Ensambla el baseline y el escenario con el
/// MISMO contexto (`resolve_projection_context`: today, horizonte, inflación, fire_settings) y
/// re-simula ambos en `spawn_blocking` (patrón del marker de `compute_projection_series_response`).
///
/// **Cache-neutral por construcción**: jamás pasa por `projection_series_cached` (que insertaría
/// el what-if en la cache del hogar) — regresión pinneada en `mcp_simulate.rs`.
///
/// Dos ensamblados (una tanda de SELECTs por lado) a propósito: los overrides pre-target
/// (`SimOverrides`) se aplican DENTRO de `build_installation_projection_input`, en el mismo punto
/// donde se derivan target y caps, para que la semántica del escenario no pueda divergir de la de
/// una proyección real con esos valores guardados. El resto de overrides muta el input clonado.
pub(crate) async fn simulate_projection_core(
    pool: &sqlx::PgPool,
    iid: Uuid,
    user_id: Uuid,
    view: LedgerView,
    spec: SimulationSpec,
) -> Result<SimulateProjectionResponse, ApiError> {
    // ---- D9: el hogar NO se simula ----------------------------------------------------------
    // `view=household` dejó de ser «las mismas cuentas con más filas»: desde 5.0.0 es el AGREGADO
    // de N simulaciones, una por miembro y con la estrategia de cada uno. Un what-if sobre eso no
    // tiene una respuesta única —¿el `swr_pct` de quién?, ¿el gasto extra de quién?— y devolver
    // «algo» sería publicar un escenario que no describe el plan de nadie. Se rechaza en el CORE,
    // no en la capa MCP, para que HTTP y MCP no puedan discrepar si mañana hay ruta.
    if matches!(view, LedgerView::Household) {
        return Err(ApiError::BadRequest(
            "household_not_simulable: simulate_projection runs one member's plan; view=household is an aggregate of N independent simulations (one strategy per member) and has no single scenario to move — call it with view=mine (the default)".into(),
        ));
    }

    // ---- Cotas de dominio (mismas que sus ejes de settings reales) --------------------------
    // Mismo helper que el `?months=` de la proyección real: un solo literal, un solo código.
    // Redundante con `resolve_projection_context` (más abajo) a propósito — aquí fija la
    // PRECEDENCIA: `months` se valida antes que el resto de ejes del escenario.
    validate_months_override(spec.months)?;
    // Único eje con signo: es el que tiene semántica de GASTO (mueve runway, caps y —en
    // `annual_expense`— el objetivo), así que es el único donde un recorte no tiene sustituto.
    // Los dos ejes de caja siguen exigiendo `>= 0` porque entre ellos ya cubren ambos signos:
    // `extra_monthly_savings` ES el ajuste de caja negativo.
    let extra_expense = spec.extra_monthly_expense.unwrap_or(Decimal::ZERO);
    let extra_cash_adj = require_non_negative(
        "extra_monthly_cash_adjustment",
        spec.extra_monthly_cash_adjustment,
    )?;
    let extra_savings = require_non_negative("extra_monthly_savings", spec.extra_monthly_savings)?;
    if let Some(p) = spec.annual_inflation_percent {
        crate::handlers::installation::validate_annual_inflation_assumption(p)?;
    }
    if let Some(v) = spec.retirement_annual_expense {
        if v <= Decimal::ZERO {
            return Err(ApiError::BadRequest(
                "retirement_expense_not_positive: retirement_annual_expense must be > 0".into(),
            ));
        }
    }
    for (asset_id, pct) in &spec.asset_return_overrides {
        // −100 no tiene raíz 12ª real (el engine clamparía a pérdida total): se rechaza.
        if *pct <= Decimal::from(-100) {
            return Err(ApiError::BadRequest(format!(
                "return_percent_too_low: expected_annual_return_percent for asset {asset_id} must be greater than -100"
            )));
        }
    }
    // Ejes por pasivo: forma y cotas. Lo que depende del pasivo concreto (que exista en el
    // scope, que tenga plan de pago, que su modelo devengue) se comprueba abajo, cuando el
    // ensamblado ya ha leído la tabla.
    {
        let mut vistos: Vec<Uuid> = Vec::with_capacity(spec.liability_overrides.len());
        for ov in &spec.liability_overrides {
            if vistos.contains(&ov.liability_id) {
                return Err(ApiError::BadRequest(
                    "liability_override_duplicate: liability_overrides must not repeat the same liability_id".into(),
                ));
            }
            vistos.push(ov.liability_id);
            if ov.extra_monthly_principal.is_some_and(|v| v < Decimal::ZERO) {
                return Err(ApiError::BadRequest(
                    "liability_extra_principal_negative: liability_overrides[].extra_monthly_principal must be >= 0".into(),
                ));
            }
            if ov.apr_percent.is_some_and(|v| v < Decimal::ZERO) {
                return Err(ApiError::BadRequest(
                    "liability_apr_negative: liability_overrides[].apr_percent must be >= 0".into(),
                ));
            }
            if ov
                .early_repayment_fee_pct
                .is_some_and(|v| v < Decimal::ZERO || v > Decimal::from(2))
            {
                return Err(ApiError::BadRequest(
                    "early_repayment_fee_out_of_range: liability_overrides[].early_repayment_fee_pct must be between 0 and 2 (Ley 5/2019 art. 23 caps the fixed-rate compensation at 2 %)".into(),
                ));
            }
            match (
                ov.lump_sum_amount,
                ov.lump_sum_month_index,
                ov.lump_sum_date,
            ) {
                (None, None, None) => {}
                (Some(a), mi, d) => {
                    if a <= Decimal::ZERO {
                        return Err(ApiError::BadRequest(
                            "liability_lump_sum_not_positive: liability_overrides[].lump_sum.amount must be > 0".into(),
                        ));
                    }
                    if mi.is_some() == d.is_some() {
                        return Err(ApiError::BadRequest(
                            "liability_lump_sum_timing_ambiguous: liability_overrides[].lump_sum requires exactly one of month_index or date".into(),
                        ));
                    }
                }
                _ => {
                    return Err(ApiError::BadRequest(
                        "liability_lump_sum_amount_required: liability_overrides[].lump_sum.amount is required with month_index/date".into(),
                    ))
                }
            }
            // Un override que no pide NADA es un error de llamada, no una petición vacía: lo
            // silencioso sería aceptarlo y devolver un escenario idéntico al baseline.
            if ov.extra_monthly_principal.is_none()
                && ov.lump_sum_amount.is_none()
                && ov.apr_percent.is_none()
                && ov.repayment_model.is_none()
            {
                return Err(ApiError::BadRequest(
                    "liability_override_empty: each entry of liability_overrides must set at least one of extra_monthly_principal, lump_sum, apr_percent or repayment_model".into(),
                ));
            }
        }
    }

    // ---- P11: crecimiento del ingreso y escalones (5.0.0, D30 — solo MCP) ------------------
    // Cotas y anti-no-op, en el CORE como el resto: la capa MCP parsea strings, no decide
    // semántica. Un `0` no es «sin crecimiento», es una llamada que no puede mover nada.
    if let Some(g) = spec.income_growth_real_pct_annual {
        if g.is_zero() {
            return Err(ApiError::BadRequest(
                "income_growth_no_op: income_growth_real_pct_annual must not be 0 — a zero growth is exactly the baseline, so the scenario would come back identical with nothing to say why; omit the axis instead".into(),
            ));
        }
        if g < MIN_INCOME_GROWTH_PCT || g > MAX_INCOME_GROWTH_PCT {
            return Err(ApiError::BadRequest(format!(
                "income_growth_out_of_range: income_growth_real_pct_annual must be between {MIN_INCOME_GROWTH_PCT} and {MAX_INCOME_GROWTH_PCT} (percent per year)"
            )));
        }
    }
    if spec.income_steps.len() > MAX_INCOME_STEPS {
        return Err(ApiError::BadRequest(format!(
            "income_steps_too_many: income_steps accepts at most {MAX_INCOME_STEPS} entries"
        )));
    }
    for st in &spec.income_steps {
        if st.delta_monthly.is_zero() {
            return Err(ApiError::BadRequest(
                "income_step_delta_zero: income_steps[].delta_monthly must not be 0 — a zero step changes nothing and the scenario would equal the baseline".into(),
            ));
        }
        if st.month_index.is_some() == st.date.is_some() {
            return Err(ApiError::BadRequest(
                "income_step_timing_ambiguous: income_steps[] requires exactly one of month_index or date".into(),
            ));
        }
    }

    // ---- P8.c: pausa de ingresos. Forma y anti-no-op ---------------------------------------
    if let Some(pause) = &spec.income_pause {
        if pause.from_month_index.is_some() == pause.from_date.is_some() {
            return Err(ApiError::BadRequest(
                "income_pause_timing_ambiguous: income_pause requires exactly one of from_month_index or from_date".into(),
            ));
        }
        if pause.months == 0 {
            return Err(ApiError::BadRequest(
                "income_pause_months_zero: income_pause.months must be >= 1 — a zero-month pause is exactly the baseline".into(),
            ));
        }
        if pause.income_fraction < Decimal::ZERO || pause.income_fraction >= Decimal::ONE {
            return Err(ApiError::BadRequest(
                "income_pause_fraction_out_of_range: income_pause.income_fraction must be in [0, 1) — 0 is an unpaid leave and 1 is the baseline with nothing to say".into(),
            ));
        }
    }

    match (
        spec.one_off_amount,
        spec.one_off_month_index,
        spec.one_off_date,
    ) {
        (None, None, None) => {}
        (Some(a), mi, d) => {
            if a <= Decimal::ZERO {
                return Err(ApiError::BadRequest("one_off_amount_not_positive: one_off_expense.amount must be > 0".into()));
            }
            if mi.is_some() == d.is_some() {
                return Err(ApiError::BadRequest(
                    "one_off_timing_ambiguous: one_off_expense requires exactly one of month_index or date".into(),
                ));
            }
        }
        _ => {
            return Err(ApiError::BadRequest(
                "one_off_amount_required: one_off_expense.amount is required with month_index/date".into(),
            ))
        }
    }

    // ---- Contexto compartido (mismas queries y regla de horizonte que el GET) ----------------
    let ctx = resolve_projection_context(pool, iid, user_id, spec.months).await?;
    let months = ctx.months;

    // Settings efectivos del escenario, re-validados con las cotas del PATCH real.
    //
    // ESTE es el punto de aplicación, y el otro (mutar el `ProjectionInput` clonado, más abajo)
    // es el EQUIVOCADO para esto: `savings_source` y las ventanas del promedio los lee el
    // ensamblado para decidir si lanza siquiera la query de `transactions_avg`. Aplicados
    // después, el override no haría absolutamente nada, en silencio.
    let fs_patch = spec.fire_settings_overrides.unwrap_or_default();
    let fs_eff = fs_patch.apply_to(&ctx.fire_settings);
    crate::handlers::installation::validate_fire_settings(&fs_eff)?;
    // El eje `swr_pct` del what-if sobrevive a la mudanza de 5.0.0: el SWR pasó a ser del perfil
    // del usuario, así que el override se aplica sobre un CLON del perfil — se simula, no se
    // persiste. El baseline usa el perfil real; el escenario, este clon.
    // **P5 y el resto del perfil como eje what-if** (§E). El patchset se aplica con el MISMO
    // `apply_to` del PATCH real sobre un CLON del perfil RESUELTO del solicitante, se valida con
    // las cotas de escritura y se vuelve a resolver (clamps + derivación de `target_basis`): lo
    // que se simula es exactamente lo que pasaría al guardarlo. No se persiste nada.
    let mut profile_eff = match spec.profile_overrides.as_ref() {
        None => ctx.retirement_profile.clone(),
        Some(patch) => {
            // Un patchset vacío es una llamada que no puede mover nada: 400, no un escenario
            // idéntico al baseline sin explicación (la misma puerta que `liability_override_empty`).
            if patch.is_empty() {
                return Err(ApiError::BadRequest(
                    "profile_overrides_empty: profile_overrides must set at least one field — an empty patch is exactly the baseline".into(),
                ));
            }
            patch.apply_to(&ctx.retirement_profile)
        }
    };
    // El eje suelto `swr_pct` convive con el del perfil porque es el que lleva publicado desde
    // 1.x. Pedir los dos a la vez es una intención contradictoria y elegir uno sería adivinar.
    if let Some(swr) = spec.swr_pct {
        if spec
            .profile_overrides
            .as_ref()
            .is_some_and(|p| p.swr_pct.is_some())
        {
            return Err(ApiError::BadRequest(
                "swr_pct_set_twice: pass swr_pct either at the top level or inside profile_overrides, not both".into(),
            ));
        }
        profile_eff.swr_pct = swr;
    }
    crate::handlers::retirement_profile::validate_retirement_profile(&profile_eff)?;
    let profile_eff = resolve_retirement_profile(Some(profile_eff));
    if spec.profile_overrides.is_some() && profile_eff == ctx.retirement_profile {
        return Err(ApiError::BadRequest(
            "profile_overrides_no_op: profile_overrides resolves to the profile you already have, so the scenario would come back identical with nothing to say why".into(),
        ));
    }
    let inflation_eff = spec
        .annual_inflation_percent
        .unwrap_or(ctx.inflation_annual_percent);

    let sim_ov = SimOverrides {
        extra_monthly_expense: extra_expense,
        retirement_monthly_expense: spec
            .retirement_annual_expense
            .map(|v| v / Decimal::from(12u32)),
    };

    let baseline_built = build_installation_projection_input(
        pool,
        iid,
        user_id,
        view,
        ctx.today,
        months,
        ctx.inflation_annual_percent,
        Some(&ctx.fire_settings),
        &ctx.retirement_profile,
        ctx.session_birth_date,
        None,
    )
    .await?;
    let scenario_built = build_installation_projection_input(
        pool,
        iid,
        user_id,
        view,
        ctx.today,
        months,
        inflation_eff,
        Some(&fs_eff),
        &profile_eff,
        ctx.session_birth_date,
        Some(&sim_ov),
    )
    .await?;

    // ---- Overrides post-build sobre el input clonado del escenario ---------------------------
    let mut scenario_input = scenario_built.input.clone();

    // Ajustes de caja constantes: mecanismo planning adjustment (entran en el net_cash mensual y
    // pasan por la cascada real, sin mover target ni caps).
    let monthly_adj = extra_savings - extra_cash_adj;
    if !monthly_adj.is_zero() {
        for slot in scenario_input.planning_monthly_cash_adjustment.iter_mut() {
            *slot += monthly_adj;
        }
    }

    // Gasto puntual.
    if let Some(amount) = spec.one_off_amount {
        match (spec.one_off_month_index, spec.one_off_date) {
            (Some(k), None) => {
                if !(1..=months).contains(&k) {
                    return Err(ApiError::BadRequest(format!(
                        "one_off_month_out_of_range: one_off_expense.month_index must be between 1 and {months}"
                    )));
                }
                scenario_input.planning_monthly_cash_adjustment[(k - 1) as usize] -= amount;
            }
            (None, Some(date)) => {
                // Mismo mapeo fecha→mes que un planning flow real con due_date, con UNA
                // excepción deliberada: desde #126 el mapeo compartido carga lo vencido en el
                // mes 0, pero el what-if mantiene su contrato previo («nunca anterior al mes
                // ancla») — un escenario no modela deuda vencida, y el mes en curso ya es
                // expresable con month_index = 1. Antes de #126 este rechazo salía gratis del
                // check de todo-ceros; ahora es explícito, con el mismo código de wire.
                if date < proj_month_first(ctx.today) {
                    return Err(ApiError::BadRequest(
                        "one_off_date_out_of_horizon: one_off_expense.date is outside the projection horizon".into(),
                    ));
                }
                let synthetic = PlanningFlowProjRow::one_off("expense", amount, Some(date), "one_off_expense");
                let adj = planning_monthly_cash_adjustments_from_flows(
                    ctx.today,
                    months,
                    std::slice::from_ref(&synthetic),
                );
                if adj.iter().all(|v| v.is_zero()) {
                    return Err(ApiError::BadRequest(
                        "one_off_date_out_of_horizon: one_off_expense.date is outside the projection horizon".into(),
                    ));
                }
                for (slot, extra) in scenario_input
                    .planning_monthly_cash_adjustment
                    .iter_mut()
                    .zip(adj.iter())
                {
                    *slot += *extra;
                }
            }
            _ => unreachable!("validated above"),
        }
    }

    // Ejes por pasivo. Post-build como `asset_return_overrides`: ninguno mueve el target FIRE ni
    // las bases de los caps, que dependen de `payment_amount` — y `payment_amount` no se toca.
    if !spec.liability_overrides.is_empty() {
        // En modo real (B/C con datos de gasto) el ensamblado anula la cuota EN MEMORIA porque
        // las cuotas pagadas ya viven dentro del promedio: los pasivos no tocan la caja y su
        // principal es una resta constante. Un override de amortización aquí no haría nada —o
        // peor, contaría la cuota dos veces si alguien relajara el gate. Se rechaza en vez de
        // devolver un escenario idéntico al baseline sin decir por qué.
        if scenario_built.debt_service_absent_reason.is_some() {
            return Err(ApiError::BadRequest(
                "liability_overrides_unavailable_in_real_expense_mode: liability_overrides do not apply when the expense base comes from the real transactions average (savings_source transactions_avg or budget_income_real_expense) — in that mode the paid instalments already live inside the average, so liabilities have no cash flow and their principal is a constant subtraction".into(),
            ));
        }
        let anchor = proj_month_first(ctx.today);
        for ov in &spec.liability_overrides {
            let Some(idx) = scenario_built
                .liability_id_label
                .iter()
                .position(|(id, _)| *id == ov.liability_id)
            else {
                return Err(ApiError::BadRequest(format!(
                    "liability_not_in_scope: unknown liability_id {} (not in scope for this view, or its payment plan already ended)",
                    ov.liability_id
                )));
            };
            let target = &mut scenario_input.liabilities[idx];

            // Modelo efectivo del escenario: el override si lo hay, si no el guardado.
            let effective_model = ov
                .repayment_model
                .map(LiabRepaymentModel::to_engine)
                .unwrap_or(target.repayment_model);
            let effective_apr = ov.apr_percent.or(target.apr_percent);
            // Post-#144 `interest_only` devenga de verdad (la cuota ES el interés del período).
            let accrues = matches!(
                effective_model,
                RepaymentModel::French | RepaymentModel::Revolving | RepaymentModel::InterestOnly
            );

            // Tres puertas contra el no-op silencioso. Las tres describen configuraciones que el
            // motor acepta sin quejarse y que producen exactamente los mismos números que el
            // baseline — un 400 con su código dice qué falta; un escenario idéntico, no.
            if ov.apr_percent.is_some() && !accrues {
                return Err(ApiError::BadRequest(
                    "liability_apr_ignored_by_repayment_model: apr_percent only accrues with repayment_model french, interest_only or revolving — fixed_payments is an interest-free loan (0 %); set repayment_model in the same override if that is what you mean".into(),
                ));
            }
            if accrues && !effective_apr.is_some_and(|v| v > Decimal::ZERO) {
                return Err(ApiError::BadRequest(
                    "liability_repayment_model_needs_apr: repayment_model french, interest_only or revolving needs an apr_percent > 0 (stored on the liability or set in the same override) — without it the model degenerates into fixed_payments and the scenario would equal the baseline".into(),
                ));
            }
            let wants_amortization =
                ov.extra_monthly_principal.is_some() || ov.lump_sum_amount.is_some();
            if wants_amortization && target.monthly_payment <= Decimal::ZERO {
                return Err(ApiError::BadRequest(
                    "liability_override_needs_payment_plan: extra_monthly_principal and lump_sum require the liability to have an active payment plan (payment_amount > 0) — without one the projection freezes its principal and there is no instalment to bring forward".into(),
                ));
            }
            // Cuarta puerta (#151), mismo principio anti no-op: la comisión y el efecto solo
            // significan algo si en ESTE override hay amortización que comisionar/aplicar.
            if !wants_amortization
                && (ov.early_repayment_fee_pct.is_some() || ov.early_repayment_effect.is_some())
            {
                return Err(ApiError::BadRequest(
                    "liability_early_repayment_axis_needs_amortization: early_repayment_fee_pct and early_repayment_effect only apply to an override that amortizes (extra_monthly_principal or lump_sum) — without one they would silently do nothing".into(),
                ));
            }
            // Quinta puerta (verificación adversarial de la Ola 3): simular un pasivo como
            // `revolving` exige que la fila TENGA su cuota mínima — el brazo revolving cobra
            // max(pct·saldo, suelo), y con mínimos NULL eso es 0 €/mes: la deuda compondría
            // hasta el horizonte en silencio. No hay eje de mínimos en el override, así que la
            // única fuente es la fila guardada.
            if effective_model == RepaymentModel::Revolving
                && !(matches!(target.min_payment_pct, Some(p) if p > Decimal::ZERO)
                    || matches!(target.min_payment_eur, Some(e) if e > Decimal::ZERO))
            {
                return Err(ApiError::BadRequest(
                    "liability_override_revolving_needs_minimums: repayment_model revolving only simulates liabilities that carry min_payment_pct/min_payment_eur (the stored row has neither) — its instalment is max(pct × balance, floor), which would be 0 here and the debt would silently compound".into(),
                ));
            }
            // Sexta puerta: `reduce_payment` no hace nada sobre una revolving — su caja es la
            // cuota MÍNIMA (pct/suelo), no la declarada que la λ-escala reduce. Rechazar antes
            // que devolver un escenario bit-idéntico al de `reduce_term` prometiendo lo contrario.
            if ov.early_repayment_effect == Some(futurefin_engine::EarlyRepaymentEffect::ReducePayment)
                && effective_model == RepaymentModel::Revolving
            {
                return Err(ApiError::BadRequest(
                    "reduce_payment_ignored_by_repayment_model: early_repayment_effect reduce_payment has no effect on a revolving — its cash leg is the minimum instalment (min_payment_pct/min_payment_eur), not the declared payment that reduce_payment scales; use reduce_term or change the model in the same override".into(),
                ));
            }

            if let Some(m) = ov.repayment_model {
                target.repayment_model = m.to_engine();
            }
            if wants_amortization {
                // Default 2 % (#151): la única línea de la ola que cambia el resultado de un
                // caller ya existente de 4.4.0 — el what-if de amortizar deja de ser gratis por
                // defecto. Opt-out explícito con "0". Nota de migración en el CHANGELOG.
                target.early_repayment_fee_pct =
                    Some(ov.early_repayment_fee_pct.unwrap_or(Decimal::from(2)));
                target.early_repayment_effect = ov.early_repayment_effect.unwrap_or_default();
            }
            if let Some(apr) = ov.apr_percent {
                target.apr_percent = Some(apr);
            }
            if let Some(extra) = ov.extra_monthly_principal {
                target.extra_principal_monthly = extra;
            }
            if let Some(amount) = ov.lump_sum_amount {
                // Mes **1-based del ancla** (el mes 1 es el mes civil de `anchor_date_ymd`), el
                // mismo eje que `one_off_expense.month_index` e `income_steps[].month_index` —
                // **NO** la rejilla 0-based de `points[].month_index`, que es lo que este
                // comentario decía hasta 5.0.0. Se resuelve a un índice discreto (y no por el
                // reparto de un planning flow) porque una amortización es un acto puntual en un
                // mes concreto.
                let k = match (ov.lump_sum_month_index, ov.lump_sum_date) {
                    (Some(k), None) => k,
                    (None, Some(d)) => {
                        let diff = month_diff(anchor, proj_month_first(d));
                        if diff < 0 {
                            return Err(ApiError::BadRequest(
                                "liability_lump_sum_date_out_of_horizon: liability_overrides[].lump_sum.date is outside the projection horizon".into(),
                            ));
                        }
                        (diff as u32).saturating_add(1)
                    }
                    _ => unreachable!("validated above"),
                };
                if !(1..=months).contains(&k) {
                    return Err(ApiError::BadRequest(
                        "liability_lump_sum_month_out_of_range: liability_overrides[].lump_sum.month_index must be between 1 and the projection horizon in months".into(),
                    ));
                }
                target.extra_principal_lump_sums.push((k, amount));
            }
        }
    }

    // Tasas por activo (los negativos > −100 son válidos desde el fix de `monthly_multiplier`).
    for (asset_id, pct) in &spec.asset_return_overrides {
        let Some(idx) = scenario_built
            .asset_id_name
            .iter()
            .position(|(id, _)| id == asset_id)
        else {
            return Err(ApiError::BadRequest(format!(
                "asset_not_in_scope: unknown asset_id {asset_id} (not in scope for this view)"
            )));
        };
        scenario_input.assets[idx].expected_annual_return_percent = Some(*pct);
    }

    // #142: los overrides de pasivos (amortización extra, TIN, modelo, comisión) CAMBIAN el
    // interés/cuotas restantes y por tanto el objetivo del escenario. El término se recomputa
    // aquí, DESPUÉS de aplicar los overrides — construirlo solo en el build dejaría el objetivo
    // del escenario calculado con el calendario sin override (bug de ensamblado, no de
    // matemática; el spike de la ola lo señaló explícitamente).
    if let Some(ft) = scenario_input.fire_target.as_mut() {
        ft.debt_payments_remaining = futurefin_engine::debt_payments_remaining_series(
            &scenario_input.liabilities,
            scenario_input.ref_date,
        );
    }

    // ---- P11 (D30): crecimiento del ingreso y escalones, SOLO en el escenario ---------------
    //
    // Los dos entran por `planning_monthly_cash_adjustment`, el mismo mecanismo que un Próximo:
    // pasan por la cascada real y por el servicio de deuda del mes, y NO tocan
    // `income_regular_monthly`. Esa elección tiene consecuencia declarada — el objetivo FIRE en
    // modo `current_income` y las bases de los caps se derivan del ingreso base, así que una
    // subida de sueldo simulada aquí no reescribe el objetivo del escenario. Subirla por el otro
    // camino haría que «¿y si me suben el sueldo?» moviera la meta a la vez que el capital, y el
    // delta no significaría nada.
    let mut income_growth_stops_at: Option<u32> = None;
    {
        let anchor = proj_month_first(ctx.today);
        // Escalones: `delta` desde su mes HASTA EL FINAL. No se recortan en la jubilación —
        // el usuario nombró el mes, y quitárselo sería simular otra cosa.
        for st in &spec.income_steps {
            let k = match (st.month_index, st.date) {
                (Some(k), None) => k,
                (None, Some(d)) => {
                    let diff = month_diff(anchor, proj_month_first(d));
                    if diff < 0 {
                        return Err(ApiError::BadRequest(
                            "income_step_date_out_of_horizon: income_steps[].date is outside the projection horizon".into(),
                        ));
                    }
                    (diff as u32).saturating_add(1)
                }
                _ => unreachable!("validated above"),
            };
            if !(1..=months).contains(&k) {
                return Err(ApiError::BadRequest(format!(
                    "income_step_month_out_of_range: income_steps[].month_index must be between 1 and {months}"
                )));
            }
            for slot in scenario_input.planning_monthly_cash_adjustment[(k - 1) as usize..]
                .iter_mut()
            {
                *slot += st.delta_monthly;
            }
        }

        if let Some(g) = spec.income_growth_real_pct_annual {
            // PRIMERA PASADA (medida antes de elegirla: **+11,5 ms** por llamada en build de
            // DEBUG sobre el fixture de `mcp_simulate`, de 14,4 ms a 25,9 ms — dos simulaciones
            // pasan a tres; el listón que había que pasar era 50 ms y release va varias veces más
            // rápido, así que la pasada extra se queda). Se corre el
            // MISMO escenario sin este eje para saber en qué mes se jubila, y el crecimiento se
            // aplica solo a los meses de ACUMULACIÓN. Sin ella, el vector llevaría sueldo hasta
            // el final del horizonte y el escenario cobraría una nómina 40 años después de
            // jubilarse — un regalo silencioso justo en el eje que más se usa para decidir.
            //
            // El corte NO es exacto y por eso se PUBLICA (`income_growth_stops_at_month_index`):
            // el ingreso extra puede adelantar la jubilación respecto de esta pasada, y los meses
            // entre la nueva y la de la sonda siguen llevando sueldo. La ventana es exactamente
            // `income_growth_stops_at_month_index − jubilacion_month_index` del escenario.
            let probe_input = scenario_input.clone();
            let probe = crate::heavy::run_projection_sim("projection", move || {
                project_net_worth_series(&probe_input)
            })
            .await?
            .map_err(map_engine_err)?;
            // Índice `i` del vector ⟺ mes `i` de la rejilla ⟺ mes `i+1` del bucle, así que el
            // primer índice YA jubilado es exactamente `engine_month_to_grid(R)`.
            let stop = engine_month_to_grid(probe.retirement_month_index).unwrap_or(months);
            income_growth_stops_at = Some(stop);
            let base_income = scenario_input.income_regular_monthly;
            for (i, slot) in scenario_input
                .planning_monthly_cash_adjustment
                .iter_mut()
                .enumerate()
                .take(stop as usize)
            {
                // Mismo eje `(k−1)/12` que la indexación del gasto y del objetivo: el mes 1
                // (índice 0) cobra el sueldo declarado tal cual y el extra es exactamente 0.
                let factor = real_growth_factor_at_month_index(g, i as u32);
                *slot += base_income * (factor - Decimal::ONE);
            }
        }
    }

    // ---- P8.c: la pausa de ingresos, y lo que le cuesta a la fecha --------------------------
    //
    // El eje hace DOS cosas, y las dos son necesarias: aplica la pausa al escenario (para que las
    // KPIs y la serie que se publican sean las del hogar en excedencia) y publica el RETRASO
    // medido contra el mismo escenario sin la pausa. Medirlo contra el baseline de la instalación
    // mezclaría el efecto de la pausa con el de todos los demás overrides de la llamada.
    let mut income_pause_kpis: Option<IncomePauseKpis> = None;
    if let Some(pause) = &spec.income_pause {
        let anchor = proj_month_first(ctx.today);
        let from = match (pause.from_month_index, pause.from_date) {
            (Some(k), None) => k,
            (None, Some(d)) => {
                let diff = month_diff(anchor, proj_month_first(d));
                if diff < 0 {
                    return Err(ApiError::BadRequest(
                        "income_pause_date_out_of_horizon: income_pause.from_date is outside the projection horizon".into(),
                    ));
                }
                (diff as u32).saturating_add(1)
            }
            _ => unreachable!("validated above"),
        };
        if !(1..=months).contains(&from) {
            return Err(ApiError::BadRequest(format!(
                "income_pause_month_out_of_range: income_pause.from_month_index must be between 1 and {months}"
            )));
        }
        let engine_pause = futurefin_engine::IncomePause {
            from_month: from,
            months: pause.months,
            income_fraction: pause.income_fraction,
        };
        let probe = scenario_input.clone();
        let delay = crate::heavy::run_projection_sim("income pause delay", move || {
            futurefin_engine::retirement_delay_months(&probe, engine_pause)
        })
        .await?
        .map_err(map_engine_err)?;
        income_pause_kpis = Some(IncomePauseKpis {
            // Los dos meses van a la rejilla; el delta NO se recalcula desde ellos porque el
            // desplazamiento de −1 se cancela en la resta y el motor ya lo publica.
            baseline_month_index: engine_month_to_grid(delay.baseline_month_index),
            paused_month_index: engine_month_to_grid(delay.paused_month_index),
            retirement_delay_months: delay.delay_months,
        });
        scenario_input.phase_plan.income_pause = Some(engine_pause);
    }

    // ---- P8.b: «¿cuánto más puedo gastar sin mover la fecha?» -------------------------------
    let max_extra_monthly_expense_keeping_date = match spec.solve_extra_monthly_expense_keeping_date
    {
        None => None,
        Some(false) => {
            return Err(ApiError::BadRequest(
                "solve_no_op: solve.extra_monthly_expense_keeping_date must be true to ask for it — passing false requests a solve and then declines it".into(),
            ))
        }
        Some(true) => {
            let probe = scenario_input.clone();
            crate::heavy::run_projection_sim("max extra expense solve", move || {
                futurefin_engine::max_extra_monthly_expense_keeping_date(&probe)
            })
            .await?
            .map_err(map_engine_err)?
            .map(money_out)
        }
    };

    // ---- Doble simulación en el pool blocking (CPU-bound; patrón del marker) -----------------
    // Bajo el MISMO techo que la proyección real (`heavy::run_projection_sim`). Es el llamante
    // que más lo necesita: `simulate_projection` es cache-neutral por diseño, así que cada
    // what-if de un agente en bucle es dos simulaciones nuevas, sin excepción.
    let baseline_input = baseline_built.input.clone();
    let scenario_sim_input = scenario_input.clone();
    let (baseline_join, scenario_join) = tokio::join!(
        crate::heavy::run_projection_sim("projection", move || project_net_worth_series(
            &baseline_input
        )),
        crate::heavy::run_projection_sim("projection", move || project_net_worth_series(
            &scenario_sim_input
        )),
    );
    let baseline_out = baseline_join?.map_err(map_engine_err)?;
    let scenario_out = scenario_join?.map_err(map_engine_err)?;

    // ---- Solves del PLAN, uno por lado (§B.7) ------------------------------------------------
    // Van DESPUÉS de las dos series y en paralelo entre sí, bajo el mismo semáforo. Solo cuestan
    // algo con una estrategia por edad; `asap` y `pension_bridge` devuelven un `StrategySolves`
    // vacío tras una sola llamada a `first_month_allocation`.
    let baseline_solve_input = baseline_built.input.clone();
    let baseline_solve_month = baseline_built.forced_retirement_month;
    let baseline_strategy = ctx.retirement_profile.strategy;
    let scenario_solve_input = scenario_input.clone();
    let scenario_solve_month = scenario_built.forced_retirement_month;
    let scenario_strategy = profile_eff.strategy;
    let (baseline_solve_join, scenario_solve_join) = tokio::join!(
        crate::heavy::run_projection_sim("strategy solves", move || compute_strategy_solves(
            &baseline_solve_input,
            baseline_solve_month,
            baseline_strategy
        )),
        crate::heavy::run_projection_sim("strategy solves", move || compute_strategy_solves(
            &scenario_solve_input,
            scenario_solve_month,
            scenario_strategy
        )),
    );
    let baseline_solves = baseline_solve_join?.map_err(map_engine_err)?;
    let scenario_solves = scenario_solve_join?.map_err(map_engine_err)?;

    let mut baseline = sim_kpis(
        &baseline_built.input,
        &baseline_out,
        &baseline_built,
        ctx.inflation_annual_percent,
        &ctx.fire_settings,
        &ctx.retirement_profile,
        ctx.today,
        ctx.birth_date,
        // El baseline es la instalación tal cual: por definición no lleva ajuste de caja.
        Decimal::ZERO,
        // …ni crecimiento de ingreso: el eje es del escenario.
        None,
        &baseline_solves,
    );
    let mut scenario = sim_kpis(
        &scenario_input,
        &scenario_out,
        &scenario_built,
        inflation_eff,
        &fs_eff,
        &profile_eff,
        ctx.today,
        ctx.birth_date,
        // El MISMO `monthly_adj` que se sumó a `planning_monthly_cash_adjustment` arriba.
        monthly_adj,
        income_growth_stops_at,
        &scenario_solves,
    );

    // ---- P3: Monte Carlo sobre los DOS lados (WP6b) -----------------------------------------
    // La MISMA semilla para los dos: las realizaciones de mercado son idénticas y lo único que
    // cambia entre columnas es el plan, así que el delta de probabilidad mide el cambio y no el
    // ruido de dos muestras. Se corre DESPUÉS de las series y de los solves, en paralelo entre
    // sí, bajo el mismo semáforo de CPU que todo lo demás.
    let monte_carlo = match &spec.monte_carlo {
        None => None,
        Some(mc) => {
            use crate::handlers::projection_bands::{
                resolve_paths, resolve_seed, success_verdict, MCP_MAX_PATHS,
            };
            let paths = resolve_paths(Some(mc.paths), MCP_MAX_PATHS)?;
            let seed = resolve_seed(iid, user_id, mc.seed);
            // El colchón lo resolvió el ENSAMBLADO de cada lado (`resolve_cash_buffer`, V6/P2):
            // derivado del tope de la regla de ahorro, o explícito si el perfil —o el
            // `profile_overrides` de este lado— lo declara. Se lee de `built` y no del perfil
            // para que el what-if y la banda simulen exactamente el mismo colchón.
            let config = |built: &BuiltProjection| futurefin_engine_stochastic::McConfig {
                seed,
                paths,
                percentiles: crate::handlers::projection_bands::BANDS_PERCENTILES.to_vec(),
                cash_buffer: built.cash_buffer.spec,
            };
            // Las volatilidades salen del ensamblado de CADA lado, alineadas con sus activos:
            // el escenario puede haber movido tasas por activo, pero nunca el ORDEN, y aun así
            // cada lado usa su propio vector para que un cambio futuro no los descoloque.
            let b_vols = volatilities_f64(&baseline_built);
            let s_vols = volatilities_f64(&scenario_built);
            let b_input = baseline_built.input.clone();
            let s_input = scenario_input.clone();
            let b_cfg = config(&baseline_built);
            let s_cfg = config(&scenario_built);
            let (b_join, s_join) = tokio::join!(
                crate::heavy::run_projection_sim("monte carlo baseline", move || {
                    futurefin_engine_stochastic::project_percentile_bands(&b_input, &b_vols, &b_cfg)
                }),
                crate::heavy::run_projection_sim("monte carlo scenario", move || {
                    futurefin_engine_stochastic::project_percentile_bands(&s_input, &s_vols, &s_cfg)
                }),
            );
            let b_out = b_join?.map_err(map_mc_err)?;
            let s_out = s_join?.map_err(map_mc_err)?;
            let apply = |k: &mut SimKpis,
                         out: &futurefin_engine_stochastic::McOutcome,
                         built: &BuiltProjection| {
                k.success_probability = probability_out(out.success_probability);
                k.success_verdict = Some(success_verdict(out.success_probability));
                k.never_retired_probability = probability_out(out.never_retired_probability);
                k.success_given_retired = out.success_given_retired.and_then(probability_out);
                k.underfunded_probability =
                    out.underfunded_probability.and_then(probability_out);
                k.months_below_need_p50 = Some(out.months_below_need_p50);
                // El mismo merge que las bandas, y por el mismo motivo: UN campo, el
                // `not_requested` del motor sustituido por el motivo REAL de la derivación.
                k.buffer_inactive_reason =
                    crate::handlers::projection_bands::merge_buffer_inactive_reason(
                        out.buffer_inactive_reason,
                        &built.cash_buffer,
                    );
                k.buffer_source = Some(built.cash_buffer.source);
                k.buffer_target_amount = built.cash_buffer.target_amount.map(crate::money::money_out);
                k.buffer_months_effective = built.cash_buffer.months_effective;
                k.buffer_source_rule_id = built.cash_buffer.source_rule_id;
                k.buffer_source_asset_name = built.cash_buffer.source_asset_name.clone();
            };
            apply(&mut baseline, &b_out, &baseline_built);
            apply(&mut scenario, &s_out, &scenario_built);
            Some(MonteCarloKpis {
                paths,
                seed: seed.to_string(),
                // Los dos lados comparten activos, así que comparten la respuesta; se toma la del
                // baseline porque es el lado sin overrides.
                any_volatility_declared: b_out.any_volatility_declared,
            })
        }
    };

    // Deflactores comparables ⟺ las dos inflaciones EFECTIVAS coinciden. Se lee del eco de cada
    // lado (`SimKpis::annual_inflation_percent`) y no de `ctx`/`inflation_eff` sueltos: así la
    // condición mira exactamente los dos números que produjeron los `*_real` que se restan.
    let deflators_comparable =
        baseline.annual_inflation_percent == scenario.annual_inflation_percent;

    let deltas = SimDeltas {
        jubilacion_months_delta: match (baseline.jubilacion_month_index, scenario.jubilacion_month_index)
        {
            (Some(b), Some(s)) => Some(s as i64 - b as i64),
            _ => None,
        },
        assets_depleted_months_delta: match (
            baseline.assets_depleted_month_index,
            scenario.assets_depleted_month_index,
        ) {
            (Some(b), Some(s)) => Some(s as i64 - b as i64),
            _ => None,
        },
        final_net_worth_delta: money_out(scenario.final_net_worth - baseline.final_net_worth),
        final_net_worth_real_delta: deflators_comparable.then(|| {
            money_out(scenario.final_net_worth_real - baseline.final_net_worth_real)
        }),
        real_delta_absent_reason: (!deflators_comparable).then_some("incomparable_deflators"),
        fire_target_base_delta: match (baseline.fire_target_base, scenario.fire_target_base) {
            (Some(b), Some(s)) => Some(money_out(s - b)),
            _ => None,
        },
        runway_months_delta: match (baseline.runway_months, scenario.runway_months) {
            (Some(b), Some(s)) => Some(s - b),
            _ => None,
        },
        income_monthly_delta: money_out(scenario.income_monthly - baseline.income_monthly),
        expense_total_monthly_delta: money_out(
            scenario.expense_total_monthly - baseline.expense_total_monthly,
        ),
        net_recurring_monthly_delta: money_out(
            scenario.net_recurring_monthly - baseline.net_recurring_monthly,
        ),
        net_cash_monthly_delta: money_out(
            scenario.net_cash_monthly - baseline.net_cash_monthly,
        ),
        // Se recalcula desde `net`/`income` (que viajan EXACTOS) en vez de restar los dos
        // `savings_rate` ya redondeados.
        savings_rate_delta: {
            let raw = |k: &SimKpis| {
                (k.income_monthly > Decimal::ZERO)
                    .then(|| k.net_recurring_monthly / k.income_monthly)
            };
            match (raw(&baseline), raw(&scenario)) {
                (Some(b), Some(s)) => Some((s - b).round_dp(SIM_RATIO_DP)),
                _ => None,
            }
        },
        liability_extra_principal_monthly_delta: money_out(
            scenario.liability_extra_principal_monthly - baseline.liability_extra_principal_monthly,
        ),
        liability_early_repayment_fee_total_delta: money_out(
            scenario.liability_early_repayment_fee_total
                - baseline.liability_early_repayment_fee_total,
        ),
        liability_total_interest_delta: money_out(
            scenario.liability_total_interest - baseline.liability_total_interest,
        ),
        liability_debt_free_months_delta: match (
            baseline.liability_debt_free_month_index,
            scenario.liability_debt_free_month_index,
        ) {
            (Some(b), Some(s)) => Some(s as i64 - b as i64),
            _ => None,
        },
        required_contribution_monthly_delta: pair_delta(
            baseline.required_contribution_monthly,
            scenario.required_contribution_monthly,
        ),
        disposable_monthly_delta: pair_delta(
            baseline.disposable_monthly,
            scenario.disposable_monthly,
        ),
        coast_fire_months_delta: match (
            baseline.coast_fire_month_index,
            scenario.coast_fire_month_index,
        ) {
            (Some(b), Some(s)) => Some(s as i64 - b as i64),
            _ => None,
        },
        partial_gap_target_delta: pair_delta(
            baseline.partial_gap_target,
            scenario.partial_gap_target,
        ),
        pension_coverage_ratio_delta: pair_delta(
            baseline.pension_coverage_ratio,
            scenario.pension_coverage_ratio,
        ),
        bridge_effective_withdrawal_pct_delta: pair_delta(
            baseline.bridge_effective_withdrawal_pct,
            scenario.bridge_effective_withdrawal_pct,
        ),
        // Sin `money_out`: es una FRACCIÓN, no euros. Se resta sobre los dos valores ya
        // redondeados a 6 decimales porque los dos vienen del MISMO estimador (un cociente de
        // contadores con `paths ≤ 1000` en el denominador), así que el redondeo no puede mover
        // el delta más allá de su última cifra.
        success_probability_delta: match (baseline.success_probability, scenario.success_probability)
        {
            (Some(b), Some(sc)) => Some((sc - b).round_dp(SIM_RATIO_DP)),
            _ => None,
        },
    };

    let series = if spec.include_series {
        let kept = density_month_indices(Density::Hybrid, baseline_out.net_worth.len() as u32);
        let pick = |serie: &[Decimal]| -> Vec<f64> {
            kept.iter()
                .filter_map(|&i| serie.get(i as usize))
                .map(|v| v.to_f64().unwrap_or(0.0))
                .collect()
        };
        // El descubierto va a la escala monetaria antes de salir, por la misma razón que en
        // `GET /v1/projection/series`: es la única serie cuyo signo se lee como un veredicto y el
        // acumulador del motor arrastra una polvareda de ±1e-25 €.
        let pick_money = |serie: &[Decimal]| -> Vec<f64> {
            kept.iter()
                .filter_map(|&i| serie.get(i as usize))
                .map(|v| money_out(*v).to_f64().unwrap_or(0.0))
                .collect()
        };
        Some(SimSeries {
            baseline_net_worth: pick(&baseline_out.net_worth),
            scenario_net_worth: pick(&scenario_out.net_worth),
            baseline_unmet_need: pick_money(&baseline_out.unmet_need),
            scenario_unmet_need: pick_money(&scenario_out.unmet_need),
            month_indices: kept,
        })
    } else {
        None
    };

    Ok(SimulateProjectionResponse {
        horizon_months: months,
        horizon_basis: ctx.horizon_basis.clone(),
        horizon_lifespan_age: ctx.retirement_profile.horizon_lifespan_age,
        view: view.as_str(),
        anchor_date_ymd: ctx.today.format("%Y-%m-%d").to_string(),
        show_age_mode: ctx.show_age_mode.clone(),
        viewer_birth_date: ctx.birth_date.map(|d| d.format("%Y-%m-%d").to_string()),
        model_note: SIMULATE_MODEL_NOTE.into(),
        baseline,
        scenario,
        deltas,
        series,
        income_pause: income_pause_kpis,
        max_extra_monthly_expense_keeping_date,
        monte_carlo,
    })
}

/// `escenario − baseline` de dos cifras que pueden no existir. **`None` en cuanto falta una de
/// las dos**: restar contra un «esta estrategia no responde a esa pregunta» inventaría un número
/// con el signo de la existencia, no del cambio. Es la misma regla de `jubilacion_months_delta`.
fn pair_delta(baseline: Option<Decimal>, scenario: Option<Decimal>) -> Option<Decimal> {
    match (baseline, scenario) {
        (Some(b), Some(s)) => Some(money_out(s - b)),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Deflactado servido (4.4.0) — GET /v1/projection/deflate
// ---------------------------------------------------------------------------

/// Tope del mes a deflactar: el mismo horizonte máximo de proyección. Más allá, la respuesta
/// describiría un punto que ninguna otra superficie modela.
const MAX_DEFLATE_MONTH_INDEX: u32 = MAX_PROJECTION_MONTHS;

#[derive(Debug, Deserialize, utoipa::IntoParams)]
pub struct DeflateQuery {
    /// Importe a convertir, como string decimal.
    pub amount: String,
    /// Mes desde el ancla (0..=840). Exactamente uno de `month_index` o `date`.
    pub month_index: Option<u32>,
    /// Fecha civil `YYYY-MM-DD`. Exactamente uno de `month_index` o `date`.
    pub date: Option<String>,
}

/// Conversión entre euros nominales de un mes futuro y euros de hoy, en **las dos direcciones a
/// la vez**.
///
/// Se publican las dos porque `amount` por sí solo es ambiguo —¿está en euros de aquel mes o en
/// los de hoy?— y elegir una dirección por el llamante es exactamente cómo se cuela un error de
/// signo en una respuesta que parece razonable. Con las dos etiquetadas, no hay nada que adivinar.
///
/// **Capa de presentación.** Aplica el MISMO `deflator_at_month_index` que produce
/// `milestones_real` y `points[].net_worth_real`, sobre la misma asunción de inflación de la
/// instalación. No simula nada y no toca el motor, que sigue capitalizando en nominal (ver el
/// doc-comment de `ProjectionPoint::net_worth_real` para por qué eso importa).
#[derive(Debug, Serialize, ToSchema)]
pub struct DeflateResponse {
    /// Eco del importe recibido.
    #[serde(with = "rust_decimal::serde::str")]
    #[schema(value_type = String)]
    pub amount: Decimal,
    /// Mes efectivo (0 = hoy). Número de MES, no una posición de serie.
    pub month_index: u32,
    /// Primero del mes civil correspondiente (`YYYY-MM-DD`).
    pub month_ymd: String,
    /// Mes 0 (`YYYY-MM-DD`): el «hoy» civil de la instalación.
    pub anchor_date_ymd: String,
    /// Inflación anual (%) usada. Es la asunción de la instalación, clampada a ≥ 0.
    #[serde(with = "rust_decimal::serde::str")]
    #[schema(value_type = String)]
    pub annual_inflation_percent: Decimal,
    /// `1 / (1 + inflación%)^(month_index/12)`. Exactamente `1` con inflación ≤ 0 o mes 0.
    #[serde(with = "rust_decimal::serde::str")]
    #[schema(value_type = String)]
    pub deflator: Decimal,
    /// Si `amount` está en euros NOMINALES del mes `month_index`, esto es lo que vale HOY:
    /// `amount × deflator`. Es la conversión que hace `net_worth_real`.
    #[serde(with = "rust_decimal::serde::str")]
    #[schema(value_type = String)]
    pub amount_in_today_euros: Decimal,
    /// Si `amount` está en euros de HOY, esto es lo que costará lo mismo en el mes `month_index`:
    /// `amount / deflator`. Es la conversión inversa (inflar), la que responde «¿cuánto
    /// necesitaré entonces para comprar lo que hoy cuesta esto?».
    #[serde(with = "rust_decimal::serde::str")]
    #[schema(value_type = String)]
    pub amount_in_month_euros: Decimal,
    pub model_note: String,
}

const DEFLATE_MODEL_NOTE: &str = "Capa de PRESENTACIÓN: convierte un importe ya calculado, no simula nada. El motor de proyección capitaliza siempre en euros NOMINALES y solo el objetivo FIRE se ajusta por inflación; deflactar aquí no cambia ninguna proyección, ningún mes de jubilación y ninguna cascada. Usa la asunción `annual_inflation_assumption_percent` de la instalación, así que dos llamadas con inflaciones distintas NO son comparables. Di siempre en qué euros está la cifra que cites.";

/// Core compartida con la tool MCP `deflate_amount`.
pub(crate) async fn deflate_amount_core(
    pool: &sqlx::PgPool,
    iid: Uuid,
    amount: Decimal,
    month_index: Option<u32>,
    date: Option<NaiveDate>,
) -> Result<DeflateResponse, ApiError> {
    // Exactamente uno de los dos. Aceptar ambos obligaría a inventar una precedencia y a que un
    // llamante que se contradiga reciba una respuesta plausible en vez de un error.
    if month_index.is_some() == date.is_some() {
        return Err(ApiError::BadRequest(
            "deflate_timing_ambiguous: provide exactly one of month_index or date".into(),
        ));
    }

    let today = crate::handlers::installation::installation_naive_today(pool, iid).await?;
    let inflation_annual_percent: Decimal = sqlx::query_scalar(
        r#"SELECT annual_inflation_assumption_percent FROM installation WHERE id = $1"#,
    )
    .bind(iid)
    .fetch_one(pool)
    .await
    // Sin clamp desde 4.9.0 (#146); el map solo fija el tipo del scalar.
    .map(|v: Decimal| v)?;
    let anchor = proj_month_first(today);

    let k = match (month_index, date) {
        (Some(k), None) => k,
        (None, Some(d)) => {
            // Mismo mapeo fecha→mes que la serie: meses civiles completos desde el ancla.
            let months = month_diff(anchor, proj_month_first(d));
            if months < 0 {
                return Err(ApiError::BadRequest(
                    "deflate_date_in_past: date must not be before the anchor month".into(),
                ));
            }
            months as u32
        }
        _ => unreachable!("validated above"),
    };
    if k > MAX_DEFLATE_MONTH_INDEX {
        return Err(ApiError::BadRequest(
            "deflate_month_out_of_range: month_index must be between 0 and 840".into(),
        ));
    }

    let deflator = deflator_at_month_index(inflation_annual_percent, k);
    // `deflator` nunca es 0 (`1/(1+i)^y` con i ≥ 0), así que la división inversa es segura; el
    // `checked_div` está por disciplina, no por un caso conocido.
    let inflated = amount
        .checked_div(deflator)
        .unwrap_or(amount);

    Ok(DeflateResponse {
        amount: money_out(amount),
        month_index: k,
        month_ymd: proj_add_months(anchor, k).format("%Y-%m-%d").to_string(),
        anchor_date_ymd: today.format("%Y-%m-%d").to_string(),
        annual_inflation_percent: money_out(inflation_annual_percent),
        // El deflactor no es dinero: se publica con más resolución que `money_out` para que
        // multiplicarlo a mano reproduzca la cifra. 10 decimales sobre un factor de orden 1 son
        // ~1e−10 de error relativo, muy por debajo del céntimo en cualquier patrimonio real.
        deflator: deflator.round_dp(10),
        amount_in_today_euros: money_out(amount * deflator),
        amount_in_month_euros: money_out(inflated),
        model_note: DEFLATE_MODEL_NOTE.into(),
    })
}

/// Meses civiles completos entre dos primeros-de-mes (`b − a`). Negativo si `b` es anterior.
fn month_diff(a: NaiveDate, b: NaiveDate) -> i32 {
    (b.year() - a.year()) * 12 + (b.month() as i32 - a.month() as i32)
}

#[utoipa::path(
    get,
    path = "/v1/projection/deflate",
    tag = "projection",
    params(DeflateQuery),
    responses(
        (status = 200, description = "Importe convertido entre euros nominales y euros de hoy", body = DeflateResponse),
        (status = 400, description = "Parámetros inválidos"),
        (status = 401, description = "Unauthorized")
    )
)]
pub async fn get_projection_deflate(
    Extension(state): Extension<Arc<AppState>>,
    jar: CookieJar,
    Query(q): Query<DeflateQuery>,
) -> Result<Json<DeflateResponse>, ApiError> {
    let user = require_session_user(&jar, &state.pool).await?;
    let (iid, _role) = require_installation_member(&state.pool, user.id.0).await?;
    let amount = q.amount.trim().parse::<Decimal>().map_err(|_| {
        ApiError::BadRequest(
            "decimal_invalid: amount must be a decimal string — use '.' as the decimal separator, with no currency symbol and no thousands separator (\"1234.56\", not \"1.234,56 €\")".into(),
        )
    })?;
    let date = q
        .date
        .as_deref()
        .map(|raw| {
            raw.trim().parse::<NaiveDate>().map_err(|_| {
                ApiError::BadRequest(
                    "date_invalid: date must be a calendar date as \"YYYY-MM-DD\" (e.g. \"2026-03-01\"), never \"01/03/2026\" nor a month alone".into(),
                )
            })
        })
        .transpose()?;
    let res = deflate_amount_core(&state.pool, iid, amount, q.month_index, date).await?;
    Ok(Json(res))
}

pub fn projection_router() -> Router {
    Router::new()
        .route("/series", get(get_projection_series))
        .route("/deflate", get(get_projection_deflate))
        // `/bands` vive en su propio módulo (`projection_bands.rs`) pero cuelga de este router:
        // es la misma familia de rutas y comparte ancla, rejilla y ensamblado con `/series`.
        .merge(crate::handlers::projection_bands::projection_bands_router())
}

/// Recompute de la proyección **`view=mine`** del usuario que acaba de entrar (ambas densidades)
/// y guardado en cache. Pensado para `tokio::spawn` tras login. Si falla, no propaga el error:
/// solo deja el cache vacío para que el próximo GET haga el compute sincronamente.
///
/// **5.0.0 (R2): calienta `mine`, no `household`.** El warm-up solo sirve para algo si precalcula
/// lo que el siguiente GET va a pedir, y desde 5.0.0 la vista por defecto —de la SPA y del
/// parámetro ausente— es `mine`. Calentar `household` dejaría en la cache una entrada que nadie
/// consulta mientras el primer GET real paga el compute entero; y en 5.0.0 `household` cuesta N
/// simulaciones, no una, así que además sería el warm-up más caro para la vista menos usada.
pub async fn warm_up_mine_projection(
    state: Arc<AppState>,
    installation_id: Uuid,
    user_id: Uuid,
) {
    for density in [Density::Hybrid, Density::Monthly] {
        tracing::info!(installation_id = %installation_id, density = ?density, "warm-up mine projection start");
        let t0 = std::time::Instant::now();
        let key = ProjectionCacheKey {
            installation_id,
            view: LedgerView::Mine,
            owner_user_id: Some(user_id),
            density,
        };
        match compute_projection_series_response(
            &state,
            user_id,
            installation_id,
            LedgerView::Mine,
            None,
            density,
        )
        .await
        {
            Ok(response) => {
                state.projection_cache_insert(key, Arc::new(response)).await;
                tracing::info!(
                    installation_id = %installation_id,
                    density = ?density,
                    ms = t0.elapsed().as_millis() as u64,
                    "warm-up done"
                );
            }
            Err(e) => {
                tracing::warn!(installation_id = %installation_id, density = ?density, error = ?e, "warm-up failed");
            }
        }
    }
}

/// Helper para handlers de mutación. Invalida todas las entries del
/// installation. **No** dispara warm-up tras mutación para evitar una race
/// condition: dos mutaciones consecutivas (M1, M2) podrían generar dos
/// warm-ups concurrentes y el de M1 (con datos pre-M2) puede terminar
/// después del de M2, dejando el cache stale. El próximo GET (cache miss)
/// hace compute on-demand — paga ~500 ms una vez tras una mutación, luego
/// cache. El warm-up proactivo se mantiene solo en login (sin
/// invalidaciones concurrentes).
///
/// **Se espera, no se lanza en background** (antes era un `tokio::spawn`). Dos razones:
///
/// 1. **Cerraba una ventana de lectura obsoleta real**: con el spawn el orden era
///    `commit → responder → (algún momento después) invalidar`, así que un GET que cayera en
///    medio servía la proyección vieja. El usuario lo ve como «he editado y la cifra no cambia».
///    Esperando la invalidación, cuando la mutación responde el estado de la cache ya es final.
/// 2. El coste es un `retain` sobre un `HashMap` pequeño bajo un `RwLock` sin contención —
///    microsegundos. El spawn no compraba latencia apreciable y a cambio hacía el efecto
///    observable solo «en algún momento», lo que volvía no deterministas todos los tests de cache
///    (y obligaba a sembrarlos de sleeps que a su vez abrían la ventana que hacía fallar al de
///    al lado).
pub async fn refresh_projection_after_mutation(
    state: &AppState,
    installation_id: Uuid,
    _user_id: Uuid,
) {
    state
        .invalidate_projection_by_installation(installation_id)
        .await;
}

#[cfg(test)]
mod horizon_tests {
    use super::*;

    #[test]
    fn horizon_fallback_without_birth_dates() {
        let today = NaiveDate::from_ymd_opt(2026, 5, 2).unwrap();
        let (m, basis) = projection_horizon_months(today, &[None], 90);
        assert_eq!(m, 30 * 12);
        assert_eq!(basis, "fallback_no_demographics");
    }

    #[test]
    fn horizon_uses_lifespan_age_from_birth_date() {
        let today = NaiveDate::from_ymd_opt(2026, 5, 2).unwrap();
        let bd = vec![
            Some(NaiveDate::from_ymd_opt(1990, 1, 1).unwrap()), // age 36 → 54y to 90
            Some(NaiveDate::from_ymd_opt(1985, 1, 1).unwrap()), // age 41 → 49y to 90
        ];
        let (m, basis) = projection_horizon_months(today, &bd, 90);
        assert_eq!(m, 54 * 12); // max of 54 and 49, not clamped (54 < 70)
        assert_eq!(basis, "lifespan_age");
    }

    #[test]
    fn horizon_minimum_five_years_when_already_near_lifespan() {
        let today = NaiveDate::from_ymd_opt(2026, 5, 2).unwrap();
        let bd = vec![Some(NaiveDate::from_ymd_opt(1940, 1, 1).unwrap())]; // age 86 → 4y to 90, clamped to 5
        let (m, basis) = projection_horizon_months(today, &bd, 90);
        assert_eq!(basis, "lifespan_age");
        assert_eq!(m, 5 * 12);
    }

    /// #149: la edad configurable mueve el horizonte año a año (36 años, límite 100 → 64 años)
    /// y el techo de 70 sigue mandando (límite 105 sobre 30 años de edad → 840, el tope).
    #[test]
    fn horizon_follows_the_configured_lifespan_age() {
        let today = NaiveDate::from_ymd_opt(2026, 5, 2).unwrap();
        let bd = vec![Some(NaiveDate::from_ymd_opt(1990, 1, 1).unwrap())]; // age 36
        let (m, basis) = projection_horizon_months(today, &bd, 100);
        assert_eq!(m, 64 * 12);
        assert_eq!(basis, "lifespan_age");

        let joven = vec![Some(NaiveDate::from_ymd_opt(1996, 1, 1).unwrap())]; // age 30
        let (m, _) = projection_horizon_months(today, &joven, 105);
        assert_eq!(m, 70 * 12, "el clamp [5,70] no se toca: 75 años pedidos → 840 meses");
    }
}

#[cfg(test)]
mod planning_distribution_tests {
    use super::*;

    #[test]
    fn dated_planning_hits_single_calendar_month() {
        let ref_d = NaiveDate::from_ymd_opt(2026, 3, 20).unwrap();
        let flows = vec![PlanningFlowProjRow::one_off(
            "expense",
            Decimal::from(500),
            Some(NaiveDate::from_ymd_opt(2026, 5, 2).unwrap()),
            "Derrama",
        )];
        let adj = planning_monthly_cash_adjustments_from_flows(ref_d, 4, &flows);
        assert_eq!(adj[2], Decimal::from(-500));
        assert_eq!(adj[0] + adj[1] + adj[3], Decimal::ZERO);
    }

    /// INVERTIDO a propósito en #126 (antes: `dated_before_anchor_month_is_ignored`, que pineaba
    /// el descarte silencioso). Un Próximo vencido no desaparece: carga íntegro en el mes ancla,
    /// declarado con `overdue: true` en `events[]`. Nunca borrar este test.
    #[test]
    fn dated_before_anchor_month_loads_into_month_zero() {
        let ref_d = NaiveDate::from_ymd_opt(2026, 3, 10).unwrap();
        let flows = vec![PlanningFlowProjRow::one_off(
            "income",
            Decimal::from(9999),
            Some(NaiveDate::from_ymd_opt(2026, 2, 28).unwrap()),
            "Cobro pasado",
        )];
        let adj = planning_monthly_cash_adjustments_from_flows(ref_d, 3, &flows);
        assert_eq!(adj[0], Decimal::from(9999));
        assert_eq!(adj[1] + adj[2], Decimal::ZERO);
    }

    #[test]
    fn undated_splits_over_ninety_days_from_the_anchor_month_first() {
        // ref_d es día 1: los valores son idénticos a los del test pre-#126, que arrancaba la
        // ventana en el propio ref_date. Enero 31 + febrero 28 + marzo 31 = 90 días exactos.
        let ref_d = NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();
        let flows = vec![PlanningFlowProjRow::one_off("expense", Decimal::from(900), None, "Sin fecha")];
        let adj = planning_monthly_cash_adjustments_from_flows(ref_d, 3, &flows);
        assert_eq!(adj.iter().sum::<Decimal>(), Decimal::from(-900));
        assert_eq!(adj[0], Decimal::from(-310));
        assert_eq!(adj[1], Decimal::from(-280));
        assert_eq!(adj[2], Decimal::from(-310));
    }

    /// El pin del arreglo (B) de #126: el reparto de la rampa sin fecha ya no depende del día en
    /// que se consulta. Agosto de 2026 (31 días): 900/90 = 10 €/día → [−310, −300, −290] para
    /// CUALQUIER ref_date dentro del mes (antes: el mes 0 recibía solo los días restantes —
    /// −310 el día 1, −20 el día 30, −10 el día 31; rango completo 300 €, un 30 % de una
    /// aportación tipo de 1.000 €/mes).
    #[test]
    fn undated_ramp_is_identical_for_every_day_of_the_anchor_month() {
        let flows = vec![PlanningFlowProjRow::one_off("expense", Decimal::from(900), None, "Sin fecha")];
        let expected = vec![
            Decimal::from(-310),
            Decimal::from(-300),
            Decimal::from(-290),
        ];
        for day in [1u32, 15, 30, 31] {
            let ref_d = NaiveDate::from_ymd_opt(2026, 8, day).unwrap();
            let adj = planning_monthly_cash_adjustments_from_flows(ref_d, 3, &flows);
            assert_eq!(adj, expected, "ref_date = 2026-08-{day:02}");
        }
    }

    /// #148: un `per_month` carga su €/mes en cada mes civil que su ventana TOCA — mes completo,
    /// sin prorrateo por días en los meses frontera (la ventana arranca el 20/10 y el octubre
    /// entero cobra los 800).
    #[test]
    fn per_month_charges_every_calendar_month_its_window_touches() {
        let ref_d = NaiveDate::from_ymd_opt(2026, 8, 15).unwrap();
        let flows = vec![PlanningFlowProjRow {
            scope: "expense".into(),
            expected_amount: Decimal::from(800),
            amount_basis: "per_month".into(),
            due_date: None,
            window_start_date: Some(NaiveDate::from_ymd_opt(2026, 10, 20).unwrap()),
            window_end_date: Some(NaiveDate::from_ymd_opt(2027, 1, 5).unwrap()),
            title: "Alquiler".into(),
        }];
        let adj = planning_monthly_cash_adjustments_from_flows(ref_d, 8, &flows);
        // ago, sep fuera; oct (20/10 ≤ 31/10), nov, dic, ene (5/1 ≥ 1/1) dentro; feb, mar fuera.
        let expected: Vec<Decimal> = [0, 0, -800, -800, -800, -800, 0, 0]
            .iter()
            .map(|v| Decimal::from(*v))
            .collect();
        assert_eq!(adj, expected);
    }

    /// #148: `window_end_date` NULL = abierta — carga hasta el horizonte. Y una ventana que
    /// arrancó en el pasado carga desde el mes 0 sin mecánica de vencido: es renta corriente.
    #[test]
    fn per_month_with_open_window_reaches_the_horizon() {
        let ref_d = NaiveDate::from_ymd_opt(2026, 8, 15).unwrap();
        let flows = vec![PlanningFlowProjRow {
            scope: "income".into(),
            expected_amount: Decimal::from(500),
            amount_basis: "per_month".into(),
            due_date: None,
            window_start_date: Some(NaiveDate::from_ymd_opt(2026, 7, 1).unwrap()),
            window_end_date: None,
            title: "Alquiler cobrado".into(),
        }];
        let adj = planning_monthly_cash_adjustments_from_flows(ref_d, 6, &flows);
        assert_eq!(adj, vec![Decimal::from(500); 6]);
    }
}

#[cfg(test)]
mod milestone_tests {
    use super::*;
    use uuid::Uuid;

    /// REESCRITO en #126 (antes: `baseline_adjustment_uses_dated_ninety_days_and_all_undated`,
    /// que pineaba la tercera regla fecha→mes propia del baseline). Ahora el baseline deriva del
    /// MISMO mapeo que la caja del motor (tres meses ancla) y por tanto ya no depende del día de
    /// la consulta.
    #[test]
    fn baseline_adjustment_derives_from_the_shared_monthly_mapping() {
        let flows = vec![
            PlanningFlowProjRow::one_off(
                "income",
                Decimal::from(1200),
                Some(NaiveDate::from_ymd_opt(2026, 1, 15).unwrap()),
                "Paga extra",
            ),
            PlanningFlowProjRow::one_off(
                "expense",
                Decimal::from(300),
                Some(NaiveDate::from_ymd_opt(2026, 4, 5).unwrap()),
                "IRPF",
            ),
            // Múltiplo de 90 a propósito: la rampa divide entre 90 en `Decimal` y para importes
            // no múltiplos deja residuo ~1e-25 € — un assert_eq exacto sobre 100 € fallaría por
            // la 25.ª cifra decimal, no por el modelo.
            PlanningFlowProjRow::one_off("expense", Decimal::from(90), None, "Varios"),
        ];
        // +1200 (mes 0) − 90 (rampa íntegra en meses 0..2); el gasto de abril cae en el mes 3,
        // fuera de los tres meses del baseline. Con la regla antigua y ref 20/01 el resultado
        // habría sido −390 (la paga del 15/01 quedaba fuera de [hoy, hoy+89] y el IRPF dentro):
        // mismo escenario, cifra distinta según el día — eso es lo que muere aquí.
        for day in [1u32, 20] {
            let ref_d = NaiveDate::from_ymd_opt(2026, 1, day).unwrap();
            assert_eq!(
                planning_upcoming_net_for_milestone_baseline(ref_d, &flows),
                Decimal::from(1110),
                "ref_date = 2026-01-{day:02}"
            );
        }
    }

    #[test]
    fn milestones_deduplicate_by_year_keeping_highest_target() {
        let points = vec![
            NwPoint {
                month_index: 0,
                net_worth: Decimal::from(900),
            },
            NwPoint {
                month_index: 1,
                net_worth: Decimal::from(1200),
            },
            NwPoint {
                month_index: 3,
                net_worth: Decimal::from(2700),
            },
            NwPoint {
                month_index: 9,
                net_worth: Decimal::from(6000),
            },
            NwPoint {
                month_index: 15,
                net_worth: Decimal::from(11000),
            },
        ];
        let out = projection_unique_reached_milestones(
            &points,
            NaiveDate::from_ymd_opt(2026, 1, 10).unwrap(),
            Decimal::ZERO,
            3,
            16,
        );
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].target, Decimal::from(5000));
        assert_eq!(out[1].target, Decimal::from(10_000));
    }

    #[test]
    fn deflate_points_to_today_is_identity_with_zero_inflation() {
        let points = vec![
            NwPoint {
                month_index: 0,
                net_worth: Decimal::from(1000),
            },
            NwPoint {
                month_index: 12,
                net_worth: Decimal::from(2000),
            },
        ];
        let out = deflate_points_to_today(&points, Decimal::ZERO);
        assert_eq!(out[0].net_worth, Decimal::from(1000));
        assert_eq!(out[1].net_worth, Decimal::from(2000));
    }

    #[test]
    fn deflate_points_to_today_discounts_future_to_present() {
        // Con 10% anual, 1.100 € dentro de un año equivalen a 1.000 € de hoy.
        let points = vec![
            NwPoint {
                month_index: 0,
                net_worth: Decimal::from(1000),
            },
            NwPoint {
                month_index: 12,
                net_worth: Decimal::from(1100),
            },
        ];
        let out = deflate_points_to_today(&points, Decimal::from(10));
        assert_eq!(out[0].net_worth, Decimal::from(1000)); // mes 0 intacto
        let diff = (out[1].net_worth - Decimal::from(1000)).abs();
        assert!(
            diff < Decimal::new(1, 6),
            "expected ~1000 € de hoy, got {}",
            out[1].net_worth
        );
    }

    #[test]
    fn real_milestones_are_reached_later_than_nominal() {
        // Patrimonio nominal que alcanza 10.000 € al final del horizonte. Deflactado al 8% anual,
        // el mismo umbral en euros de hoy se cruza más tarde (o no se cruza dentro del horizonte).
        let points: Vec<NwPoint> = (0u32..=120)
            .map(|m| NwPoint {
                month_index: m,
                net_worth: Decimal::from(1000) + Decimal::from(75) * Decimal::from(m),
            })
            .collect();
        let anchor = NaiveDate::from_ymd_opt(2026, 1, 10).unwrap();
        let nominal = projection_unique_reached_milestones(&points, anchor, Decimal::ZERO, 3, 64);
        let real_points = deflate_points_to_today(&points, Decimal::from(8));
        let real = projection_unique_reached_milestones(&real_points, anchor, Decimal::ZERO, 3, 64);
        // Para cualquier umbral común, en euros de hoy se cruza más tarde (nunca antes); al menos uno
        // estrictamente posterior, porque el deflactor < 1 en cualquier mes > 0.
        let mut found_strictly_later = false;
        for n in &nominal {
            if let Some(r) = real.iter().find(|r| r.target == n.target) {
                assert!(
                    r.reached_month_index >= n.reached_month_index,
                    "umbral {} se cruzó antes en euros de hoy ({} < {})",
                    n.target,
                    r.reached_month_index,
                    n.reached_month_index
                );
                if r.reached_month_index > n.reached_month_index {
                    found_strictly_later = true;
                }
            }
        }
        assert!(
            found_strictly_later,
            "algún umbral común debería cruzarse más tarde en euros de hoy"
        );
    }

    #[test]
    fn compound_marker_ignores_planning_and_liability_payments() {
        let input = ProjectionInput {
            ref_date: NaiveDate::from_ymd_opt(2026, 1, 15).unwrap(),
            horizon_months: 24,
            annual_inflation_percent: Decimal::ZERO,
            tax_brackets: Vec::new(),
            taxes_enabled: false,
            taxable_gain_ratio: Decimal::ONE,
            income_regular_monthly: Decimal::from(3000),
            expense_regular_monthly: Decimal::from(2500),
            assets: vec![SimAsset {
                id: Uuid::from_u128(1),
                value: Decimal::from(10_000),
                purchase_price: Some(Decimal::from(10_000)),
                is_liquid: true,
                expected_annual_return_percent: Some(Decimal::from(6)),
            }],
            allocation_rules: vec![AllocationRule {
                target_index: 0,
                kind: AllocationKind::Remainder,
                amount: None,
                cap: None,
            }],
            liabilities: vec![ProjectionLiabilityInput {
                principal: Decimal::from(50_000),
                monthly_payment: Decimal::from(1200),
                payment_end: None,
                repayment_model: RepaymentModel::FixedPayments,
                apr_percent: None,
                min_payment_pct: None,
                min_payment_eur: None,
                extra_principal_monthly: Decimal::ZERO,
                extra_principal_lump_sums: Vec::new(),
                early_repayment_fee_pct: None,
                early_repayment_effect: Default::default(),
            }],
            planning_monthly_cash_adjustment: vec![Decimal::from(5_000); 24],
            phase_plan: PhasePlan::classic(Decimal::ZERO, Decimal::from(2500)),
            fire_target: None,
        };
        let month = compound_outpaces_true_savings_month(&input, Decimal::from(500)).unwrap();
        assert!(month.is_none());
    }

    #[test]
    fn compound_marker_requires_persistent_crossover() {
        let input = ProjectionInput {
            ref_date: NaiveDate::from_ymd_opt(2026, 1, 15).unwrap(),
            horizon_months: 24,
            annual_inflation_percent: Decimal::ZERO,
            tax_brackets: Vec::new(),
            taxes_enabled: false,
            taxable_gain_ratio: Decimal::ONE,
            income_regular_monthly: Decimal::from(1200),
            expense_regular_monthly: Decimal::from(1000),
            assets: vec![SimAsset {
                id: Uuid::from_u128(2),
                value: Decimal::from(50_000),
                purchase_price: Some(Decimal::from(50_000)),
                is_liquid: true,
                expected_annual_return_percent: Some(Decimal::from(18)),
            }],
            allocation_rules: vec![AllocationRule {
                target_index: 0,
                kind: AllocationKind::Remainder,
                amount: None,
                cap: None,
            }],
            liabilities: vec![],
            planning_monthly_cash_adjustment: vec![Decimal::ZERO; 24],
            phase_plan: PhasePlan::classic(Decimal::ZERO, Decimal::from(1000)),
            fire_target: None,
        };
        let month = compound_outpaces_true_savings_month(&input, Decimal::from(200)).unwrap();
        assert!(month.is_some());
        assert!(month.unwrap() >= 1);
    }
}


#[cfg(test)]
mod jubilacion_civil_tests {
    use super::*;

    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).unwrap()
    }

    #[test]
    fn sin_cruce_no_hay_ni_fecha_ni_edad() {
        let (date, age) = jubilacion_civil(d(2026, 8, 21), Some(d(1990, 5, 10)), None);
        assert_eq!(date, None);
        assert_eq!(age, None);
    }

    #[test]
    fn mes_cero_es_hoy_y_no_se_trata_como_no_alcanzado() {
        // Ya-FIRE hoy: `Some(0)` es un cruce válido, no un «no alcanzado».
        let (date, age) = jubilacion_civil(d(2026, 8, 21), Some(d(1990, 5, 10)), Some(0));
        assert_eq!(date.as_deref(), Some("2026-08-21"));
        assert_eq!(age, Some(36));
    }

    #[test]
    fn conserva_el_dia_del_ancla_igual_que_add_months_civil() {
        // 137 meses = 11 años y 5 meses.
        let (date, _) = jubilacion_civil(d(2026, 8, 21), None, Some(137));
        assert_eq!(date.as_deref(), Some("2038-01-21"));
    }

    #[test]
    fn recorta_a_fin_de_mes_cuando_el_dia_no_existe() {
        // Mismo clamp que `addMonthsCivil` (pinned en apps/web/src/lib/dates.test.ts):
        // 31 ene + 1 mes = 28 feb, y 2028 es bisiesto → 29 feb.
        let (date, _) = jubilacion_civil(d(2026, 1, 31), None, Some(1));
        assert_eq!(date.as_deref(), Some("2026-02-28"));
        let (date, _) = jubilacion_civil(d(2027, 8, 31), None, Some(6));
        assert_eq!(date.as_deref(), Some("2028-02-29"));
    }

    #[test]
    fn salta_de_ano_correctamente() {
        let (date, _) = jubilacion_civil(d(2026, 8, 21), None, Some(5));
        assert_eq!(date.as_deref(), Some("2027-01-21"));
        let (date, _) = jubilacion_civil(d(2026, 12, 15), None, Some(13));
        assert_eq!(date.as_deref(), Some("2028-01-15"));
    }

    #[test]
    fn sin_fecha_de_nacimiento_hay_fecha_pero_no_edad() {
        let (date, age) = jubilacion_civil(d(2026, 8, 21), None, Some(12));
        assert_eq!(date.as_deref(), Some("2027-08-21"));
        assert_eq!(age, None);
    }

    #[test]
    fn la_edad_son_anos_cumplidos_en_la_fecha_del_cruce() {
        let birth = d(1990, 5, 10);
        // Cruce el 2038-01-21: el cumpleaños de mayo aún no ha llegado ese año → 47, no 48.
        let (_, age) = jubilacion_civil(d(2026, 8, 21), Some(birth), Some(137));
        assert_eq!(age, Some(47));
        // Cruce el 2038-06-21: cumpleaños ya pasado → 48.
        let (_, age) = jubilacion_civil(d(2026, 8, 21), Some(birth), Some(142));
        assert_eq!(age, Some(48));
    }

    #[test]
    fn anclar_al_dia_1_restaria_un_ano_en_el_mes_del_cumpleanos() {
        // La razón de conservar el día del ancla. Cruce en mayo, nacimiento el día 10:
        // con el día 21 del ancla el cumpleaños YA pasó; con día 1 aún no.
        let birth = d(1990, 5, 10);
        let today = d(2026, 8, 21);
        let (_, age) = jubilacion_civil(today, Some(birth), Some(129)); // 2037-05-21
        assert_eq!(age, Some(47), "con el día del ancla, el cumpleaños ya pasó");
        let dia_1 = age_completed_years(proj_add_months(proj_month_first(today), 129), birth);
        assert_eq!(dia_1, 46, "anclado al día 1 saldría un año menos");
    }
}

#[cfg(test)]
mod density_tail_tests {
    use super::*;

    /// REGRESIÓN — `hybrid` no puede perder la cola del horizonte.
    ///
    /// El bucle anual solo emitía múltiplos de 12, así que un horizonte que no lo fuera se
    /// cortaba en silencio: `?months=100` daba como último punto el mes 96 y los meses 97–100
    /// no existían en ninguna de las tres series. Se pierde justo el punto que se lee como
    /// «patrimonio al final», y la tool MCP `get_projection` fuerza `hybrid`, así que era el
    /// camino por defecto para un consumidor conversacional.
    #[test]
    fn hybrid_density_always_includes_the_last_month_of_the_horizon() {
        for horizonte in [12u32, 18, 24, 25, 100, 119, 120, 121, 840] {
            let v = density_month_indices(Density::Hybrid, horizonte + 1);
            assert_eq!(
                v.last(),
                Some(&horizonte),
                "horizonte {horizonte}: el último índice debe ser el propio horizonte, no {:?}",
                v.last()
            );
            // Y sigue siendo estrictamente creciente y sin duplicados.
            assert!(
                v.windows(2).all(|w| w[0] < w[1]),
                "horizonte {horizonte}: índices no crecientes: {v:?}"
            );
            // Los trece primeros meses siguen siendo mensuales.
            let esperados: Vec<u32> = (0..=12u32.min(horizonte)).collect();
            assert_eq!(v[..esperados.len()], esperados[..], "horizonte {horizonte}");
        }
    }
}

#[cfg(test)]
mod engine_error_mapping_tests {
    use super::*;

    /// **`BridgeDiscountOverflow` es un 422 con nombre, no un 500 ni un 400 genérico.**
    ///
    /// El motor devuelve este error cuando la tabla del puente desborda `Decimal` porque el
    /// descuento derivado es demasiado negativo para los meses que faltan hasta la pensión. Hasta
    /// el pase de correcciones de la revisión adversarial esto **panicaba** dentro de `powd` y
    /// salía como un 500 opaco de `GET /v1/projection/series`.
    ///
    /// Este test es UNITARIO a propósito: aguas arriba está el clamp de
    /// `build_installation_projection_input` (un descuento negativo se sube a 0 con el aviso
    /// `bridge_discount_clamped`), así que por la API **ya no se puede alcanzar** — y una rama de
    /// mapeo inalcanzable por HTTP sigue necesitando que alguien pinee su código y su status, o
    /// el día que el clamp se relaje el 500 vuelve sin que nada avise.
    #[test]
    fn el_desbordamiento_del_puente_es_un_422_con_codigo_propio() {
        let e = map_engine_err(EngineError::BridgeDiscountOverflow);
        assert_eq!(
            e.status(),
            axum::http::StatusCode::UNPROCESSABLE_ENTITY,
            "la petición está bien formada y los datos son válidos uno a uno: lo que no cabe es \
             el número que sale de combinarlos"
        );
        let msg = e.sanitised_message();
        assert!(
            msg.starts_with("bridge_discount_out_of_range: "),
            "el código estable viaja en el prefijo `snake_code:`: {msg}"
        );
        // El mensaje nombra las DOS magnitudes que hay que mirar para arreglarlo: el descuento y
        // el mes de la pensión. Sin ellas, el 422 dice «algo no cabe» y manda a leer el código.
        assert!(
            msg.contains("bridge_discount_annual_pct") && msg.contains("pension_start_month_index"),
            "el mensaje debe nombrar el descuento y el índice de la pensión: {msg}"
        );
    }

    /// Las dos variantes de «capacidad que aún no existe» siguen siendo 400
    /// `engine_feature_unavailable`, y el resto 400 `engine_rejected_input`: el 422 nuevo no se
    /// come ninguna rama vecina.
    #[test]
    fn el_resto_del_mapeo_no_se_mueve() {
        for e in [
            EngineError::UnsupportedWithdrawalRule,
            EngineError::UnsupportedPhase,
        ] {
            let mapped = map_engine_err(e);
            assert_eq!(mapped.status(), axum::http::StatusCode::BAD_REQUEST);
            assert!(
                mapped
                    .sanitised_message()
                    .starts_with("engine_feature_unavailable: "),
                "{}",
                mapped.sanitised_message()
            );
        }
        let mapped = map_engine_err(EngineError::InvalidHorizon);
        assert_eq!(mapped.status(), axum::http::StatusCode::BAD_REQUEST);
        assert!(
            mapped
                .sanitised_message()
                .starts_with("engine_rejected_input: "),
            "{}",
            mapped.sanitised_message()
        );
    }
}
