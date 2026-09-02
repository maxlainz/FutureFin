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
use crate::handlers::retirement_profile::{resolve_retirement_profile, RetirementProfile};
use crate::handlers::person_view::LedgerView;
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
    fire_target_at_month_index, first_month_per_asset_contribution_nominals,
    project_net_worth_series, AllocationCap, AllocationKind, AllocationRule, EngineError,
    FireTarget, ProjectionInput, ProjectionLiabilityInput, RepaymentModel, SimAsset,
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
fn density_month_indices(density: Density, months: u32) -> Vec<u32> {
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
    /// Primer mes cuya venta bruta necesaria iguala o supera TODO lo drenable (todos los
    /// activos): la cartera se vacía ese mes y desde el siguiente el descubierto se acumula
    /// restando del patrimonio. Número de MES (misma base que `points[].month_index`), nunca una
    /// posición de array. `null` explícito = no se agota dentro del horizonte — no «no
    /// calculado». (#119)
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
    /// Objetivo FIRE base **en euros de HOY** (gross-up de impuestos aplicado). Sirve como
    /// referencia y como anclaje del target móvil — el target en cada mes es este valor ×
    /// `(1 + inflación%)^(meses/12)`, así que el objetivo nominal el mes de la jubilación es
    /// bastante mayor que esta cifra. `null` cuando no hay configuración FIRE válida.
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
    /// **exacto** con la misma `fire_target_at_month_index` del motor sobre el mes del cruce
    /// (`base × (1 + inflación%)^(mes/12)`) — no se interpola entre puntos de la serie ni se lee de
    /// `fire_target_series`, que con `density=hybrid` puede no tener el mes del cruce.
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

#[derive(Debug, FromRow)]
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
/// v1.4.2 arregló en el chart. Un único helper con dos callers, por el mismo motivo por el que
/// existe `fire_target_at_month_index`.
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
fn jubilacion_civil(
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

pub(crate) fn map_engine_err(e: EngineError) -> ApiError {
    ApiError::BadRequest(format!("engine_rejected_input: {e}"))
}

/// Primer mes cuyo patrimonio cruza el target FIRE (inflado mes a mes). Se evalúa sobre TODOS
/// los meses de la serie — nunca sobre puntos decimados — para no perder precisión. Compartido
/// por `compute_projection_series_response` y `simulate_projection_core`.
/// Primer mes cuyo patrimonio LÍQUIDO (#143) alcanza el objetivo. Escaneo LINEAL a propósito:
/// con el término finito de deuda (#142) el objetivo ya NO es monótono, así que una búsqueda
/// binaria o una salida temprana serían incorrectas en silencio.
fn fire_crossover_month(
    ft: Option<&futurefin_engine::FireTarget>,
    liquid_worth: &[Decimal],
) -> Option<u32> {
    // La puerta (necesidad positiva HOY, SWR > 0) vive en el engine: base None ⇒ sin cruce.
    let ft = ft.filter(|f| futurefin_engine::fire_target_base_at_month_index(f, 0).is_some())?;
    for (i, lw) in liquid_worth.iter().enumerate() {
        let target = fire_target_at_month_index(Some(ft), i as u32).unwrap_or(Decimal::ZERO);
        if target > Decimal::ZERO && *lw >= target {
            return Some(i as u32);
        }
    }
    None
}

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
                  expected_annual_return_percent
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
    let assets: Vec<SimAsset> = assets_rows
        .into_iter()
        .map(|r| {
            asset_id_name.push((r.id, r.name));
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
        retirement_start_month: None,
        income_retirement_monthly: income_retirement,
        expense_retirement_monthly: expense_retirement,
        retirement_monthly_withdrawal: Decimal::ZERO,
        fire_target,
    };

    Ok(BuiltProjection {
        input,
        monthly_net_regular,
        allocation_rule_ids,
        asset_id_name,
        liability_id_label,
        planning_rows,
        effective_savings_source,
        savings_income_basis,
        savings_expense_basis,
        fire_target_absent_reason,
        debt_service_monthly,
        debt_service_absent_reason,
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
        ("view" = Option<String>, Query, description = "`mine` | `household`; ausente = household. Cualquier otro valor → 400 `invalid_view`"),
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
        months,
        horizon_basis,
    })
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
        months,
        horizon_basis,
    } = resolve_projection_context(&state.pool, iid, user_id, months_override).await?;

    let horizon_years = months / 12;

    let built = build_installation_projection_input(
        &state.pool,
        iid,
        user_id,
        view,
        today,
        months,
        inflation_annual_percent,
        Some(&fire_settings),
        &retirement_profile,
        None,
    )
    .await?;
    let BuiltProjection {
        input: projection_input,
        monthly_net_regular: monthly_delta_assumption,
        allocation_rule_ids: _,
        asset_id_name,
        liability_id_label,
        planning_rows,
        effective_savings_source,
        savings_income_basis,
        savings_expense_basis,
        fire_target_absent_reason,
        debt_service_monthly: _,
        debt_service_absent_reason: _,
    } = built;

    // Las dos simulaciones (principal + marker «compound supera ahorro») son CPU-bound y se
    // ejecutan en el pool blocking de Tokio. `tokio::join!` arranca ambas en paralelo, así que
    // un horizonte de 70 años con N activos no bloquea el reactor y aprovecha 2 cores.
    //
    // Desde la Fase 3 van bajo el techo de `heavy::run_projection_sim` (el tercer semáforo, junto
    // al KDF y al cripto del backup): sin él el límite efectivo eran los 512 hilos del pool de
    // blocking de Tokio, y N proyecciones concurrentes —trivial de provocar con `?months=`, que
    // salta la cache por diseño— dejaban sin CPU al reactor hasta tumbar `/v1/ready`. El permiso
    // envuelve SOLO la simulación: un HIT de cache no pasa por aquí y no espera a nadie.
    let main_input = projection_input.clone();
    let marker_input = projection_input.clone();
    let assumption = monthly_delta_assumption;
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
    let liabilities_negative_amortization: Vec<LiabilityNegativeAmortization> = liability_id_label
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
    let compound_outpaces_true_savings_month_index = marker_join?.map_err(map_engine_err)?;

    let starting_net_worth = output
        .net_worth
        .first()
        .copied()
        .unwrap_or(Decimal::ZERO);

    // Indices a serializar según la densidad solicitada. Para `Hybrid`
    // (mes 0..12 mensual + anual desde 24) el JSON pesa ~5× menos.
    let kept_indices = density_month_indices(density, output.net_worth.len() as u32);

    let points: Vec<ProjectionPoint> = kept_indices
        .iter()
        .filter_map(|&i| {
            let idx = i as usize;
            let nw = output.net_worth.get(idx)?;
            let cc = output.contributed_capital.get(idx)?;
            Some(ProjectionPoint {
                month_index: i,
                net_worth: *nw,
                contributed_capital: *cc,
                net_worth_liquid: output
                    .liquid_worth
                    .get(idx)
                    .copied()
                    .unwrap_or(Decimal::ZERO),
                // Deflactado por el `month_index` del punto, JAMÁS por su posición en el array:
                // con `density=hybrid` los puntos no son equidistantes y la versión ingenua
                // deflacta 70 años como si fueran 30 — el bug del chart de la v1.4.2, que aquí
                // no puede reproducirse porque el helper solo acepta un número de mes.
                net_worth_real: *nw
                    * deflator_at_month_index(inflation_annual_percent, i),
            })
        })
        .collect();

    // Milestones se computan sobre TODOS los meses (no sobre los serializados),
    // si no, con `density=hybrid` se perderían milestones que caen entre dos
    // puntos anuales.
    let points_full: Vec<NwPoint> = output
        .net_worth
        .iter()
        .enumerate()
        .map(|(i, nw)| NwPoint {
            month_index: i as u32,
            net_worth: *nw,
        })
        .collect();

    // `asset_id_name` y `planning_rows` se reusan de `build_installation_projection_input` —
    // antes el handler hacía 2 SELECTs adicionales redundantes contra `assets` y `planning_flows`.
    let asset_series: Vec<AssetSeries> = asset_id_name
        .iter()
        .zip(output.per_asset_series.iter())
        .map(|((id, name), series)| AssetSeries {
            asset_id: *id,
            asset_name: name.clone(),
            values: kept_indices
                .iter()
                .filter_map(|&i| series.get(i as usize))
                .map(|v| v.to_f64().unwrap_or(0.0))
                .collect(),
        })
        .collect();

    let milestone_baseline_adjustment =
        planning_upcoming_net_for_milestone_baseline(today, &planning_rows);
    // Eventos: mismo `planning_rows` ya cargado, misma regla fecha→mes que los ajustes de caja.
    // Ninguna query nueva.
    let (events, events_truncated) = projection_events_from_flows(today, months, &planning_rows);
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

    let fire_target_ref = projection_input.fire_target.as_ref();
    // #142: el término de hoy (mes 0) para la vista previa del formulario.
    let fire_target_debt_component = fire_target_ref
        .filter(|ft| futurefin_engine::fire_target_base_at_month_index(ft, 0).is_some())
        .map(|ft| {
            ft.debt_payments_remaining
                .first()
                .copied()
                .unwrap_or(Decimal::ZERO)
        })
        .map(crate::money::money_out);
    let (
        fire_target_series,
        jubilacion_month_index,
        jubilacion_target_net_worth,
        jubilacion_target_net_worth_nominal,
    ) = match fire_target_ref {
        Some(ft) if futurefin_engine::fire_target_base_at_month_index(ft, 0).is_some() => {
            let crossed_at = fire_crossover_month(Some(ft), &output.liquid_worth);
            // Se itera sobre `points`, NO sobre `kept_indices`: la serie es paralela a los
            // puntos y el consumidor la alinea por posición (no lleva `month_index` propio,
            // auditoría MCP §8), así que el paralelismo tiene que ser estructural. Antes `points`
            // usaba `filter_map` (descarta índices fuera de rango) y esta serie un `map` que
            // no descartaba nada: coincidían solo porque `density_month_indices` nunca emite
            // un índice > months-1. Un cambio ahí las habría desalineado en silencio.
            let series: Vec<f64> = points
                .iter()
                .map(|p| {
                    fire_target_at_month_index(Some(ft), p.month_index)
                        .unwrap_or(Decimal::ZERO)
                        .to_f64()
                        .unwrap_or(0.0)
                })
                .collect();
            debug_assert_eq!(series.len(), points.len(), "fire_target_series ∥ points");
            // Target del mes del cruce en euros NOMINALES, calculado EXACTO con el helper del
            // motor sobre `crossed_at`. Ni se interpola entre dos puntos de la serie ni se lee
            // de `fire_target_series[pos]`: con `density=hybrid` el mes del cruce puede no ser
            // un punto servido, y `base_amount` sin redondear es el mismo que usó el motor para
            // decidir el cruce.
            let nominal = crossed_at
                .and_then(|k| fire_target_at_month_index(Some(ft), k))
                .map(|v| money_out(v).to_string());
            (
                series,
                crossed_at,
                Some(
                    money_out(
                        futurefin_engine::fire_target_base_at_month_index(ft, 0)
                            .unwrap_or(Decimal::ZERO),
                    )
                    .to_string(),
                ),
                nominal,
            )
        }
        _ => (Vec::new(), None, None, None),
    };
    // Posición del mes del cruce dentro de los arrays paralelos: el ÚLTIMO punto servido cuyo
    // `month_index` no pasa del mes del cruce (convención documentada en el campo). `rposition`
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

    let final_nominal = points_full.last().map(|p| p.net_worth).unwrap_or(Decimal::ZERO);
    let final_month_index = points_full.last().map(|p| p.month_index).unwrap_or(0).max(0) as u32;
    let final_net_worth_real =
        final_nominal * deflator_at_month_index(inflation_annual_percent, final_month_index);

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
        model_note:
            "Motor mensual: presupuesto regular sin cuotas derivadas de pasivos, servicio de deuda activo por mes, ajustes por Próximos, crecimiento compuesto por activo en términos nominales. El GASTO (regular y de jubilación) se indexa mes a mes a la inflación de la instalación; los INGRESOS quedan planos a propósito (las subidas se pelean, no se regalan), y el target FIRE se evalúa mes a mes ajustado por inflación para preservar el poder adquisitivo del usuario."
                .into(),
        anchor_date_ymd: today.format("%Y-%m-%d").to_string(),
        show_age_mode: show_age_mode.clone(),
        use_age_on_x_axis: show_age_mode.trim() == "ages"
            && resolved_birth_for_demographics.is_some(),
        viewer_birth_date: resolved_birth_for_demographics
            .map(|d| d.format("%Y-%m-%d").to_string()),
        milestones,
        milestones_real,
        compound_outpaces_true_savings_month_index,
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
        drawdown_gain_basis: drawdown_gain_basis_of(&projection_input.assets),
        taxable_gain_ratio_today: taxable_gain_ratio_today_of(&projection_input.assets),
        assets_depleted_month_index: output.assets_depleted_month_index,
        uncovered_deficit_total: money_out(output.uncovered_deficit_total),
        unallocated_savings_total: money_out(output.unallocated_savings_total),
        unallocated_savings_reason: unallocated_reason_of(
            &projection_input.assets,
            output.unallocated_savings_total,
        ),
        liabilities_negative_amortization,
        fire_target_absent_reason,
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
    pub include_series: bool,
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
    /// Primer mes en que la cartera se vacía del todo (misma definición y motor que el campo
    /// homónimo de `/v1/projection/series`, #119). `null` = no se agota en el horizonte. Es la
    /// respuesta a «si gasto X más, ¿cuándo me quedo sin nada?» — la pregunta que más justifica
    /// un what-if.
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
}

/// Series decimadas (hybrid) opt-in — números f64 como el resto de series de chart.
#[derive(Debug, Serialize)]
pub(crate) struct SimSeries {
    pub month_indices: Vec<u32>,
    pub baseline_net_worth: Vec<f64>,
    pub scenario_net_worth: Vec<f64>,
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
) -> SimKpis {
    let debt_service_monthly = built.debt_service_monthly;
    let jubilacion_month_index = fire_crossover_month(input.fire_target.as_ref(), &output.liquid_worth);
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
        assets_depleted_month_index: output.assets_depleted_month_index,
        unallocated_savings_total: money_out(output.unallocated_savings_total),
        unallocated_savings_reason: unallocated_reason_of(
            &input.assets,
            output.unallocated_savings_total,
        ),
        jubilacion_date_ymd,
        jubilacion_age,
        final_net_worth: money_out(final_net_worth),
        final_net_worth_real: money_out(final_net_worth_real),
        fire_target_base: input
            .fire_target
            .as_ref()
            .and_then(|ft| futurefin_engine::fire_target_base_at_month_index(ft, 0))
            .map(money_out),
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
        expense_retirement_base_monthly: money_out(input.expense_retirement_monthly),
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
const SIMULATE_MODEL_NOTE: &str = "What-if sin persistir: se simulan dos veces los MISMOS datos del hogar (baseline y escenario) con el mismo horizonte, ancla y calendario; los deltas son escenario − baseline. El motor capitaliza en euros NOMINALES; la inflación indexa el GASTO mes a mes (regular y de jubilación, eje (k−1)/12: el mes 1 cobra el gasto declarado tal cual) y el objetivo FIRE, y deja los INGRESOS planos a propósito (decisión del owner: las subidas se pelean). Bajar `annual_inflation_percent` abarata TODO el gasto futuro, sube la rentabilidad real de los activos Y frena el objetivo a la vez: puede adelantar la jubilación años sin que nada del plan haya mejorado — léelo como un cambio de supuesto, no como una mejora. Admite negativos hasta −2 (deflación sostenida: gasto y objetivo DECRECEN). Igual con `swr_pct`: subirlo baja el objetivo por división, no por ahorrar más. Los ejes de caja (`extra_monthly_savings`, `extra_monthly_cash_adjustment`, `one_off_expense`) NO tocan ingreso ni gasto: mueven `net_cash_monthly`, no `net_recurring_monthly` ni `savings_rate`. `final_net_worth` está en euros nominales del último mes del horizonte; para comparar poder adquisitivo usa `final_net_worth_real`, y solo cuando `deltas.real_delta_absent_reason` es null. `liability_overrides` amortiza deuda: el importe sale de la caja del mes Y baja el principal a la vez, así que el efecto instantáneo sobre el patrimonio es CERO salvo por la compensación por reembolso anticipado (default 2 % del extra, techo legal a tipo fijo de la Ley 5/2019 art. 23; `early_repayment_fee_pct: \"0\"` la quita): esa comisión sale de la caja y NO amortiza — el what-if deja de ser gratis por defecto. No se modela la caída al 1,5 % tras el año 10, ni los topes de tipo variable (0,25 %/0,15 %), ni el límite de la pérdida financiera del prestamista: si tu préstamo es variable o veterano, pasa el % que te aplique. `early_repayment_effect: \"reduce_payment\"` baja la cuota en vez de acortar el plazo — con una amortización PUNTUAL (lump) el mes de extinción se conserva EXACTAMENTE; con amortización extra RECURRENTE puede adelantarse algo (nunca atrasarse), porque el importe fijo cancela antes cerca del final. Lo que se gana está en `liability_total_interest_delta` (negativo = interés que ya no se devenga; compara contra `liability_early_repayment_fee_total_delta`, el coste) y en que la cuota liberada vuelve sola a la cascada, no en un salto de patrimonio el día que amortizas. Si el pasivo no devenga intereses (`fixed_payments`, o sin TIN), amortizar antes no mejora nada y el escenario lo dirá con deltas a cero.";

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
    let mut profile_eff = ctx.retirement_profile.clone();
    if let Some(swr) = spec.swr_pct {
        profile_eff.swr_pct = swr;
    }
    crate::handlers::retirement_profile::validate_retirement_profile(&profile_eff)?;
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
                // Mes 1-based, misma rejilla que `points[].month_index`: el mes 1 es el mes civil
                // del ancla. Se resuelve a un índice discreto (y no por el reparto de un planning
                // flow) porque una amortización es un acto puntual en un mes concreto.
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

    let baseline = sim_kpis(
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
    );
    let scenario = sim_kpis(
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
    );

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
    };

    let series = if spec.include_series {
        let kept = density_month_indices(Density::Hybrid, baseline_out.net_worth.len() as u32);
        let pick = |serie: &[Decimal]| -> Vec<f64> {
            kept.iter()
                .filter_map(|&i| serie.get(i as usize))
                .map(|v| v.to_f64().unwrap_or(0.0))
                .collect()
        };
        Some(SimSeries {
            baseline_net_worth: pick(&baseline_out.net_worth),
            scenario_net_worth: pick(&scenario_out.net_worth),
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
    })
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
}

/// Recompute de la proyección `view=household` (ambas densidades) y guardado
/// en cache. Pensado para `tokio::spawn` tras login. Si falla, no propaga el
/// error: solo deja el cache vacío para que el próximo GET haga el compute
/// sincronamente.
pub async fn warm_up_household_projection(
    state: Arc<AppState>,
    installation_id: Uuid,
    user_id: Uuid,
) {
    for density in [Density::Hybrid, Density::Monthly] {
        tracing::info!(installation_id = %installation_id, density = ?density, "warm-up household projection start");
        let t0 = std::time::Instant::now();
        let key = ProjectionCacheKey {
            installation_id,
            view: LedgerView::Household,
            owner_user_id: Some(user_id),
            density,
        };
        match compute_projection_series_response(
            &state,
            user_id,
            installation_id,
            LedgerView::Household,
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
            retirement_start_month: None,
            income_retirement_monthly: Decimal::ZERO,
            expense_retirement_monthly: Decimal::from(2500),
            retirement_monthly_withdrawal: Decimal::ZERO,
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
            retirement_start_month: None,
            income_retirement_monthly: Decimal::ZERO,
            expense_retirement_monthly: Decimal::from(1000),
            retirement_monthly_withdrawal: Decimal::ZERO,
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
