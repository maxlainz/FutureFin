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
//! Reglas de RESOLUCIÓN que no son defaults sino derivaciones, y que por eso viven en
//! `resolve_retirement_profile` y no en el `Default`:
//!
//! * **U4 — el porcentaje de retirada es ÚNICO.** `swr_pct` dimensiona el objetivo FIRE **y** es
//!   el porcentaje de las reglas de retirada basadas en saldo: `withdrawal_rule.pct`
//!   (`percent_of_balance`, `guardrails`) y `withdrawal_rule.start_pct` (`hybrid`) son
//!   **opcionales** y, ausentes, se resuelven a `swr_pct`. El perfil publicado dice de dónde sale
//!   el número (`withdrawal_rule.pct_source`: `swr` | `explicit`). Un porcentaje explícito se
//!   sigue honrando y gana. El resolvedor es uno solo —[`resolve_withdrawal_rule`]— porque con
//!   dos, «único» valdría en el formulario y no en el chart.
//! * **C7 — el puente NO es una estrategia, es un ajuste de la pensión.** `pension.bridge_enabled`
//!   está disponible con CUALQUIER estrategia y viene apagado. Al encenderlo sin números, el
//!   resolvedor rellena `bridge_max_pct = max(5, swr + 1)` y `bridge_max_years = 7`: el puente sin
//!   tope no es un puente, es una retirada sin límite.
//!
//! # El modelo v2 (5.0.0, decisiones M2/M4/M5/C3/C5/C7 del owner)
//!
//! Lo que cambia respecto de la primera vuelta de 5.0.0, y qué se llevó por delante:
//!
//! * **El éxito define la fecha** ⇒ `success_threshold_pct` vuelve al perfil como RESTRICCIÓN
//!   (80–100, default 95, C3). Desde V7 y hasta aquí era «se acepta y se ignora»; ahora es
//!   load-bearing y la migración `20260906091500_drop_stored_success_threshold.sql` borra los 95
//!   que aquella promesa dejó almacenados (un valor guardado que nadie leía no puede resucitar
//!   como elección del usuario).
//! * **La pensión es un FLUJO DE CAJA** (M4): no hay objetivo descontado, así que mueren
//!   `target_basis` (con su derivación R6 y su `target_basis_stored`) y `bridge_discount_basis`.
//! * **El colchón de caja desaparece** (M6): `cash_buffer_months` fuera. La caja es un activo y
//!   la regla de ahorro decide cuánto se guarda.
//! * **`pension_bridge` deja de ser una estrategia** (C7). El literal se sigue ACEPTANDO en la
//!   deserialización como alias de `asap`; el perfil resuelto enciende el puente con sus defaults
//!   y el ensamblado avisa (`strategy_pension_bridge_migrated`). **Nunca se re-emite**: la primera
//!   escritura deja `"strategy":"asap"` en el JSONB.
//! * **Dos modos nuevos**: `coast_mode` (`fixed_retirement_age` | `fixed_stop_age`, M10) y
//!   `partial_retirement.mode` (`at_age` | `asap`, M11) — por eso `partial_retirement.starts_at_age`
//!   pasa a ser opcional y por eso [`requires_target_age`] es una función libre de dos argumentos:
//!   con `coast` la edad de jubilación solo es obligatoria en el modo A.
//!
//! La columna es `users.retirement_profile jsonb NULL` (`NULL` = defaults). La migración
//! `20260902200000_users_retirement_profile.sql` la crea y **copia** los cuatro ejes movidos
//! desde `installation.fire_settings` al perfil de cada usuario, para que el upgrade no mueva
//! un número; `20260906091500_drop_stored_success_threshold.sql` limpia las cuatro claves que v2
//! retira. Ninguna de las dos es necesaria para LEER un perfil viejo: el struct no lleva
//! `deny_unknown_fields`, así que un JSONB con `target_basis`, `bridge_discount_basis` o
//! `cash_buffer_months` sigue cargando y esas claves se ignoran solas.

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
/// Techo del SWR (%). Era 4 desde `FireSettings`; **sube a 6 en 5.0.0** (decisión de integración
/// del plan v2, bajo M5): el SWR deja de dimensionar un objetivo y pasa a ser «el máximo que se
/// vende en cualquier año», con la fecha decidida por el éxito. Un 5–6 % es un régimen agresivo
/// pero describible —y el sorteo lo castiga solo—; con el tope en 4 no se podía ni escribir.
pub(crate) const MAX_SWR_PCT: Decimal = Decimal::from_parts(6, 0, 0, false, 0);

/// Umbral de éxito mínimo aceptable (%). Por debajo del 80 el plan ya no describe una jubilación:
/// describe una apuesta, y el veredicto verde dejaría de significar nada.
pub(crate) const MIN_SUCCESS_THRESHOLD_PCT: u32 = 80;
/// Umbral máximo (%). `100` es un literal con semántica propia (C3): **cero fallos de N caminos**,
/// evaluado sobre el estimador puntual, no sobre la cota de Wilson.
pub(crate) const MAX_SUCCESS_THRESHOLD_PCT: u32 = 100;
/// Umbral por defecto (%). C3: el 100 hacía que la fecha fuera el mínimo muestral (±10 años según
/// la semilla, sin converger al subir N). 95 sobre el límite inferior de Wilson es estable.
pub(crate) const DEFAULT_SUCCESS_THRESHOLD_PCT: u32 = 95;

/// Techo de la tasa inicial del puente (%). Es el MISMO techo que el de cualquier retirada
/// ([`MAX_WITHDRAWAL_PCT`]) y se nombra aparte porque se lee en otro sitio: el puente es una tasa
/// inicial mayor con fecha límite (C2), no una regla distinta.
pub(crate) const MAX_BRIDGE_PCT: Decimal = MAX_WITHDRAWAL_PCT;
/// Años máximos del puente: mínimo 1 (menos de un año no es un puente, es un mes de caja).
pub(crate) const MIN_BRIDGE_YEARS: u32 = 1;
/// Años máximos del puente: 20. Más allá, «puente hasta la pensión» describe la jubilación entera
/// y el tope deja de ser una restricción.
pub(crate) const MAX_BRIDGE_YEARS: u32 = 20;
/// Años del puente al ENCENDERLO sin decir cuántos (C7).
pub(crate) const DEFAULT_BRIDGE_YEARS: u32 = 7;

/// Tasa inicial del puente al ENCENDERLO sin decir cuál (C7): `max(5, swr + 1)` %.
///
/// El puente solo sirve para algo si permite vender MÁS que el régimen ordinario, así que el
/// default tiene que quedar por encima del SWR sea cual sea el SWR — de ahí el `swr + 1`. El
/// suelo de 5 % es el número que el owner fijó para el caso normal (SWR 3–3,5): sin él, un perfil
/// conservador estrenaría el puente con un 4 % que apenas mueve la fecha.
pub(crate) fn default_bridge_pct(swr: Decimal) -> Decimal {
    let five = Decimal::from(5u32);
    let lifted = swr + Decimal::ONE;
    let v = if lifted > five { lifted } else { five };
    // Un SWR pegado al techo dejaría el default por encima de la cota del propio puente.
    v.min(MAX_BRIDGE_PCT)
}

// ---------------------------------------------------------------------------
// Enumerados del perfil
// ---------------------------------------------------------------------------

/// Las CUATRO estrategias de jubilación (D15, C7). **Una por usuario**: la estrategia decide el
/// trigger de la jubilación y qué lecturas tienen sentido.
///
/// El `Deserialize` es manual —como los de `FireSettings`— para que un literal desconocido dé
/// un error con la lista de variantes en vez de un `unknown variant` genérico, y para que la
/// superficie MCP pueda reusar EXACTAMENTE esta lista (`parse_enum_param`).
///
/// **`pension_bridge` ya no es una estrategia** (C7): el puente es un ajuste de la tarjeta
/// Pensión, disponible con cualquier estrategia. El literal se sigue aceptando como ALIAS de
/// `asap` —ver [`RetirementProfile::migrated_from_pension_bridge`]— y **no aparece en la lista de
/// variantes válidas del error**: quien escribe hoy una estrategia nueva no debe aprender un
/// nombre que ya no existe.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ToSchema, Default)]
#[serde(rename_all = "snake_case")]
pub enum RetirementStrategy {
    /// «Cuanto antes (FIRE clásico)»: se jubila el primer mes válido. Es la conducta de 4.15.x y
    /// por eso es el default.
    #[default]
    Asap,
    /// «A una edad fija»: la edad manda (D17), llegue o no el capital.
    RetireAtAge,
    /// «Ahorrar ahora y dejar crecer (Coast FIRE)».
    Coast,
    /// «Media jornada».
    Partial,
}

/// El literal retirado que se sigue aceptando como alias de [`RetirementStrategy::Asap`].
pub(crate) const PENSION_BRIDGE_ALIAS: &str = "pension_bridge";

/// Las variantes VÁLIDAS, en el orden en que se enseñan. El alias no está: es compatibilidad de
/// entrada, no una opción que ofrecer. La superficie MCP reusa esta lista (`parse_enum_param`).
pub(crate) const RETIREMENT_STRATEGY_VARIANTS: &[&str] =
    &["asap", "retire_at_age", "coast", "partial"];

/// **El único sitio donde un literal se convierte en estrategia.** Devuelve además si llegó por el
/// alias retirado `pension_bridge` — el dato que [`RetirementProfile`] necesita para encender el
/// puente y avisar. Con dos parsers, el alias valdría en una superficie y no en la otra.
pub(crate) fn parse_retirement_strategy(s: &str) -> Option<(RetirementStrategy, bool)> {
    match s {
        "asap" => Some((RetirementStrategy::Asap, false)),
        "retire_at_age" => Some((RetirementStrategy::RetireAtAge, false)),
        "coast" => Some((RetirementStrategy::Coast, false)),
        "partial" => Some((RetirementStrategy::Partial, false)),
        PENSION_BRIDGE_ALIAS => Some((RetirementStrategy::Asap, true)),
        _ => None,
    }
}

impl<'de> Deserialize<'de> for RetirementStrategy {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Ok(StrategyChoice::deserialize(deserializer)?.strategy)
    }
}

/// La estrategia MÁS de dónde vino: `migrated` = llegó como `pension_bridge` (C7).
///
/// Es un tipo propio y no un `deserialize_with` porque el dato tiene que sobrevivir hasta el
/// struct del perfil, y un `deserialize_with` de un campo no puede escribir en otro campo.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) struct StrategyChoice {
    pub strategy: RetirementStrategy,
    pub migrated: bool,
}

impl<'de> Deserialize<'de> for StrategyChoice {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        use serde::de::Error;
        let s = String::deserialize(deserializer)?;
        match parse_retirement_strategy(&s) {
            Some((strategy, migrated)) => Ok(StrategyChoice { strategy, migrated }),
            None => Err(D::Error::unknown_variant(&s, RETIREMENT_STRATEGY_VARIANTS)),
        }
    }
}

/// Los dos modos de Coast FIRE (M10, C8).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ToSchema, Default)]
#[serde(rename_all = "snake_case")]
pub enum CoastMode {
    /// **Modo A — fijo la edad de jubilación**: el solver busca el PRIMER mes en que se puede
    /// dejar de aportar y aun así llegar a esa edad con el plan en pie. Exige
    /// `target_retirement_age`; `coast_stop_age` es una LECTURA, no un dato.
    #[default]
    FixedRetirementAge,
    /// **Modo B — fijo cuándo dejo de aportar** (`coast_stop_age`): la fecha de jubilación es la
    /// que salga del umbral. Aquí `target_retirement_age` NO es obligatoria.
    FixedStopAge,
}

impl<'de> Deserialize<'de> for CoastMode {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        use serde::de::Error;
        let s = String::deserialize(deserializer)?;
        match s.as_str() {
            "fixed_retirement_age" => Ok(Self::FixedRetirementAge),
            "fixed_stop_age" => Ok(Self::FixedStopAge),
            _ => Err(D::Error::unknown_variant(
                &s,
                &["fixed_retirement_age", "fixed_stop_age"],
            )),
        }
    }
}

/// Los dos modos de arranque de la media jornada (M11).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ToSchema, Default)]
#[serde(rename_all = "snake_case")]
pub enum PartialStartMode {
    /// **Modo A — a una edad fija**: exige `partial_retirement.starts_at_age`.
    #[default]
    AtAge,
    /// **Modo B — «en cuanto pueda»**: el solver busca el primer mes en que bajar a media jornada
    /// deja el plan en pie. La edad de inicio pasa a ser una LECTURA y puede faltar.
    #[serde(rename = "asap")]
    AsSoonAsPossible,
}

impl<'de> Deserialize<'de> for PartialStartMode {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        use serde::de::Error;
        let s = String::deserialize(deserializer)?;
        match s.as_str() {
            "at_age" => Ok(Self::AtAge),
            "asap" => Ok(Self::AsSoonAsPossible),
            _ => Err(D::Error::unknown_variant(&s, &["at_age", "asap"])),
        }
    }
}

/// `true` cuando el plan EXIGE `target_retirement_age`.
///
/// Es una función LIBRE y de dos argumentos —y no un método de [`RetirementStrategy`]— porque
/// desde v2 la respuesta depende también del modo de coast (M10): con `fixed_stop_age` la edad de
/// jubilación no se impone, la calcula el umbral.
///
/// **`partial` no está en la lista, y es deliberado**: la media jornada USA
/// `target_retirement_age` cuando la hay (es el fin OPCIONAL de la fase), pero no la exige — con
/// la fase declarada, la jubilación total la decide el éxito.
pub(crate) fn requires_target_age(strategy: RetirementStrategy, coast_mode: CoastMode) -> bool {
    match strategy {
        RetirementStrategy::RetireAtAge => true,
        RetirementStrategy::Coast => coast_mode == CoastMode::FixedRetirementAge,
        RetirementStrategy::Asap | RetirementStrategy::Partial => false,
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

/// Procedencia del porcentaje de retirada de una regla (U4, 5.0.0). Se PUBLICA en el perfil
/// resuelto; **no se acepta como entrada** (`skip_deserializing`) ni se persiste en el JSONB:
/// es una derivación, no un dato del usuario.
///
/// Apunta al porcentaje que dimensiona la retirada de cada `kind`: `pct` en
/// `percent_of_balance` y `guardrails`, `start_pct` en `hybrid`. En `fixed_real` **no viaja**
/// (ausente, ni siquiera `null`): esa regla no tiene porcentaje, y publicar uno sugeriría que
/// hay un % en juego que nadie usa.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum PctSource {
    /// El porcentaje se HEREDÓ de `swr_pct` porque el usuario no escribió ninguno.
    Swr,
    /// El usuario escribió ese porcentaje a mano y manda sobre el SWR.
    Explicit,
}

/// Regla de retirada + su modo de gasto. Los `pct` son BRUTOS de impuestos (R9), igual que el
/// SWR: lo que se vende de la cartera antes de pasar por el gross-up.
///
/// **U4 — el porcentaje de retirada es ÚNICO** (decisión del owner, 5.0.0): `swr_pct` dimensiona
/// el objetivo FIRE **y** es el porcentaje de las reglas basadas en saldo. Por eso `pct`
/// (`percent_of_balance`, `guardrails`) y `start_pct` (`hybrid`) son **opcionales**: ausentes se
/// resuelven a `swr_pct` (mismo `Decimal`, mismos clamps) en `resolve_withdrawal_rule`, y el
/// perfil publicado dice de dónde salió el número en [`WithdrawalRule::pct_source`]. Un valor
/// explícito se sigue honrando —el wire de 4.15.x y las tools MCP no se rompen— y gana sobre el
/// SWR.
///
/// **Cómo se SUELTA un porcentaje explícito**: el PATCH sustituye `withdrawal_rule` ENTERA (ver
/// [`RetirementProfilePatch`]), así que mandar el objeto sin la clave `pct` ya la borra y el
/// porcentaje vuelve a heredarse. No hace falta —ni existe— un `clear_pct`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(default)]
pub struct WithdrawalRule {
    pub kind: WithdrawalRuleKind,
    /// `percent_of_balance` y `guardrails`: % anual del líquido. **Opcional**: ausente hereda
    /// `swr_pct` (U4).
    #[serde(with = "rust_decimal::serde::str_option")]
    #[schema(value_type = Option<String>)]
    pub pct: Option<Decimal>,
    /// `hybrid`: % de partida. **Opcional**: ausente hereda `swr_pct` (U4).
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
    /// **Solo salida** (U4): de dónde sale el porcentaje que dimensiona esta regla —`swr`
    /// (heredado de `swr_pct`) o `explicit` (escrito a mano)—. Lo rellena
    /// `resolve_withdrawal_rule`; se ignora en la entrada y no se guarda en el JSONB, así que
    /// **ausente** significa «esta regla no tiene porcentaje» (`fixed_real`) o «este objeto no
    /// ha pasado por el resolvedor».
    #[serde(default, skip_deserializing, skip_serializing_if = "Option::is_none")]
    pub pct_source: Option<PctSource>,
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
            pct_source: None,
        }
    }
}

/// Pensión pública (u otra renta vitalicia) **con fecha** (D3/D8, M4). Desde v2 es un FLUJO DE
/// CAJA y nada más: no descuenta ningún objetivo, no dimensiona ningún capital. Lo único que
/// cambia por tener fecha es **cuándo** entra el dinero — y, si el puente está encendido, hasta
/// cuándo se permite vender por encima del SWR.
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
    ///
    /// **Default `0` porque por defecto no se supone que cobres pensión mientras trabajas a
    /// jornada reducida; súbelo si tu régimen te la paga.** No es una regla legal —lo era en el
    /// texto anterior, y era falso: hay regímenes que compatibilizan pensión y trabajo a tiempo
    /// parcial—, es el supuesto CONSERVADOR: contar una pensión que no cobras adelanta la fecha
    /// de jubilación con dinero que no existe.
    #[serde(default, with = "rust_decimal::serde::str")]
    #[schema(value_type = String)]
    pub fraction_while_partial: Decimal,

    // ---- El PUENTE (C2/C7): un ajuste de la pensión, no una estrategia ----------------------
    /// **Puente hasta la pensión, apagado por defecto.** Encendido, el tope de la tasa inicial en
    /// el mes de jubilación pasa a ser [`Self::bridge_max_pct`] —en vez del SWR— siempre que la
    /// pensión llegue dentro de [`Self::bridge_max_years`], y la fecha válida nunca es anterior a
    /// `pensión − años máximos`.
    ///
    /// Qué modela: **jubilación anticipada — sin sueldo no hay aportaciones; se vende hasta la
    /// pensión; el tope del puente es la tasa inicial máxima si la pensión llega dentro de los
    /// años máximos.** Fuera de esa ventana manda el SWR de siempre. Está disponible con
    /// CUALQUIER estrategia (C7).
    #[serde(default)]
    pub bridge_enabled: bool,
    /// Tasa inicial máxima del puente (% anual, BRUTO igual que el SWR). Estrictamente mayor que
    /// `swr_pct` —si no, el puente no permite nada que el régimen ordinario no permitiera ya— y
    /// como mucho [`MAX_BRIDGE_PCT`]. Ausente con el puente encendido:
    /// [`default_bridge_pct`].
    #[serde(default, with = "rust_decimal::serde::str_option")]
    #[schema(value_type = Option<String>)]
    pub bridge_max_pct: Option<Decimal>,
    /// Años máximos entre la jubilación y la pensión para que el puente aplique. `[1, 20]`;
    /// ausente con el puente encendido: [`DEFAULT_BRIDGE_YEARS`].
    #[serde(default)]
    pub bridge_max_years: Option<u32>,
}

fn default_true() -> bool {
    true
}

/// Fase de media jornada (P7, M11). No lleva `ends_at_age` a propósito: termina en la jubilación
/// total, que ya tiene su propio trigger — dos fines chocarían.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct PartialRetirement {
    /// Edad a la que se baja a media jornada. **Opcional desde v2**: con
    /// [`PartialStartMode::AsSoonAsPossible`] la calcula el solver y aquí no hay dato que dar. Con
    /// el modo `at_age` es obligatoria (`partial_start_age_required`).
    #[serde(default)]
    pub starts_at_age: Option<u32>,
    /// Ingreso MENSUAL en euros de HOY durante la fase (>= 0; `0` = año sabático).
    #[serde(with = "rust_decimal::serde::str")]
    #[schema(value_type = String)]
    pub income_monthly_today: Decimal,
    #[serde(default)]
    pub expense_basis: PartialExpenseBasis,
    /// `at_age` (default, conducta de 5.0.0-WP3) | `asap`.
    #[serde(default)]
    pub mode: PartialStartMode,
}

// ---------------------------------------------------------------------------
// El perfil
// ---------------------------------------------------------------------------

/// Perfil de jubilación de UN usuario. Todas las claves son opcionales en el wire: un JSONB
/// `{}` —o `NULL`— es el perfil por defecto.
///
/// **Sin `deny_unknown_fields`, y eso es contrato**: un JSONB escrito por 5.0.0-WP5 con
/// `target_basis`, `bridge_discount_basis` o `cash_buffer_months` sigue cargando y esas tres
/// claves se ignoran solas. La migración las borra para dejar el almacén limpio, pero LEER nunca
/// dependió de que la migración hubiera corrido.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema)]
pub struct RetirementProfile {
    pub strategy: RetirementStrategy,
    /// Edad de jubilación total. OBLIGATORIA con `retire_at_age` y con `coast` en modo
    /// `fixed_retirement_age` ([`requires_target_age`]); opcional en `partial` (fin de la fase
    /// parcial); ignorada por `asap`, que se jubila en la primera fecha válida.
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

    /// **Umbral de éxito, en % de caminos que llegan al horizonte sin volver a trabajar** (M2/C3).
    /// `[80, 100]`, default 95. Es una RESTRICCIÓN sobre la fecha, no un adorno del veredicto: la
    /// fecha válida es el primer mes que lo cumple. `100` significa cero fallos de N.
    pub success_threshold_pct: u32,
    /// Modo de coast (M10). Con `fixed_stop_age` la edad de jubilación deja de ser obligatoria.
    pub coast_mode: CoastMode,
    /// Edad a la que se deja de aportar. **Dato en el modo `fixed_stop_age`, lectura en el modo
    /// `fixed_retirement_age`** (allí lo resuelve el solver). Cota: `[18, edad de jubilación o
    /// horizonte]`.
    pub coast_stop_age: Option<u32>,

    pub withdrawal_rule: WithdrawalRule,
    pub pension: Option<PensionPlan>,
    pub partial_retirement: Option<PartialRetirement>,

    /// **El perfil llegó con el literal retirado `strategy: "pension_bridge"`** (C7). No es un
    /// campo del wire —no se deserializa desde ninguna clave, no se serializa, no se persiste—:
    /// lo pone el [`Deserialize`] de este struct al ver el alias, y lo consumen dos sitios:
    ///
    /// 1. [`resolve_retirement_profile`], que enciende el puente y le pone sus defaults;
    /// 2. el ensamblado de la proyección, que emite el aviso `strategy_pension_bridge_migrated`.
    ///
    /// Como no viaja, la primera escritura del perfil deja `"strategy":"asap"` en el JSONB y el
    /// flag se apaga solo para siempre. **El perfil NUNCA re-emite `pension_bridge`.**
    #[serde(skip)]
    pub migrated_from_pension_bridge: bool,
}

impl Default for RetirementProfile {
    fn default() -> Self {
        default_retirement_profile()
    }
}

/// Gemelo DERIVADO de [`RetirementProfile`] usado solo para deserializar.
///
/// Existe por una razón concreta: el alias `pension_bridge` es información del WIRE que hay que
/// llevarse a un campo del perfil (`migrated_from_pension_bridge`), y un `deserialize_with` de un
/// campo no puede escribir en otro. Las alternativas eran un thread-local —que se rompe en cuanto
/// la carga cruza un `.await` y tokio mueve la tarea de hilo, encendiendo el puente del perfil
/// equivocado— o un visitor a mano de doce campos con sus decimales-string, que es exactamente el
/// sitio donde se pierde un default en silencio.
///
/// **La duplicación es segura porque la conversión de abajo construye el struct SIN `..`**: añadir
/// un campo a `RetirementProfile` y olvidarlo aquí no compila.
#[derive(Deserialize)]
#[serde(default)]
struct RetirementProfileWire {
    /// Lleva el alias consigo (ver [`StrategyChoice`]). **Una sola clave `strategy`**: dos campos
    /// serde con el mismo nombre dejarían el segundo sin rellenar en silencio.
    strategy: StrategyChoice,
    target_retirement_age: Option<u32>,
    fire_number_mode: FireNumberMode,
    #[serde(with = "rust_decimal::serde::str_option")]
    fire_number_manual_amount: Option<Decimal>,
    #[serde(with = "rust_decimal::serde::str")]
    swr_pct: Decimal,
    horizon_lifespan_age: u32,
    success_threshold_pct: u32,
    coast_mode: CoastMode,
    coast_stop_age: Option<u32>,
    withdrawal_rule: WithdrawalRule,
    pension: Option<PensionPlan>,
    partial_retirement: Option<PartialRetirement>,
}

impl Default for RetirementProfileWire {
    fn default() -> Self {
        let d = default_retirement_profile();
        RetirementProfileWire {
            strategy: StrategyChoice {
                strategy: d.strategy,
                migrated: false,
            },
            target_retirement_age: d.target_retirement_age,
            fire_number_mode: d.fire_number_mode,
            fire_number_manual_amount: d.fire_number_manual_amount,
            swr_pct: d.swr_pct,
            horizon_lifespan_age: d.horizon_lifespan_age,
            success_threshold_pct: d.success_threshold_pct,
            coast_mode: d.coast_mode,
            coast_stop_age: d.coast_stop_age,
            withdrawal_rule: d.withdrawal_rule,
            pension: d.pension,
            partial_retirement: d.partial_retirement,
        }
    }
}

impl<'de> Deserialize<'de> for RetirementProfile {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let w = RetirementProfileWire::deserialize(deserializer)?;
        let RetirementProfileWire {
            strategy,
            target_retirement_age,
            fire_number_mode,
            fire_number_manual_amount,
            swr_pct,
            horizon_lifespan_age,
            success_threshold_pct,
            coast_mode,
            coast_stop_age,
            withdrawal_rule,
            pension,
            partial_retirement,
        } = w;
        Ok(RetirementProfile {
            strategy: strategy.strategy,
            target_retirement_age,
            fire_number_mode,
            fire_number_manual_amount,
            swr_pct,
            horizon_lifespan_age,
            success_threshold_pct,
            coast_mode,
            coast_stop_age,
            withdrawal_rule,
            pension,
            partial_retirement,
            migrated_from_pension_bridge: strategy.migrated,
        })
    }
}

/// El perfil de quien no ha tocado nada: se jubila en la primera fecha que cumple el umbral, con
/// el SWR de siempre como tope de venta anual y el gasto declarado como necesidad.
pub(crate) fn default_retirement_profile() -> RetirementProfile {
    RetirementProfile {
        strategy: RetirementStrategy::Asap,
        target_retirement_age: None,
        fire_number_mode: FireNumberMode::AnnualExpense,
        fire_number_manual_amount: None,
        swr_pct: Decimal::new(35, 1),
        horizon_lifespan_age: 90,
        success_threshold_pct: DEFAULT_SUCCESS_THRESHOLD_PCT,
        coast_mode: CoastMode::FixedRetirementAge,
        coast_stop_age: None,
        withdrawal_rule: WithdrawalRule::default(),
        pension: None,
        partial_retirement: None,
        migrated_from_pension_bridge: false,
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

/// **El resolvedor ÚNICO del porcentaje de retirada (U4).** Todo consumidor de una
/// `WithdrawalRule` —el perfil que publican `GET`/`PATCH`, el `PhasePlan` que arma
/// `handlers/projection.rs`, las bandas de Monte Carlo y el `profile_overrides` del what-if—
/// pasa por aquí, directamente o vía [`resolve_retirement_profile`]. Que sea uno solo es el
/// punto: con dos, «el porcentaje único» sería único en un sitio y otra cosa en el otro, y la
/// diferencia solo se vería en el chart.
///
/// Qué hace: rellena con `swr_pct` el porcentaje que la regla necesita y que el usuario no
/// escribió, y anota en `pct_source` de dónde salió.
///
/// * `percent_of_balance` / `guardrails` → `pct`.
/// * `hybrid` → `start_pct` (`end_pct` sigue siendo obligatorio: es el suelo del latch, no un
///   porcentaje que el SWR pueda dimensionar).
/// * `fixed_real` → nada: no tiene porcentaje, y `pct_source` se queda ausente.
///
/// **No clampa.** Los clamps de lectura viven en [`resolve_retirement_profile`] y corren ANTES,
/// así que el valor heredado es el `swr_pct` ya acotado; y un explícito fuera de rango que
/// llegue por una vía de escritura debe ser **rechazado** por `validate_*`, no reescrito.
///
/// **Es idempotente, y por una razón operativa**: el what-if aplica su patchset sobre el perfil
/// ya RESUELTO, así que este resolvedor vuelve a correr sobre un `pct` que él mismo materializó.
/// Un porcentaje marcado `swr` se RE-hereda (si el escenario cambió `swr_pct`, el % de la regla
/// se mueve con él); uno `explicit` se respeta. Sin esa relectura, un `profile_overrides` que
/// solo tocara el SWR dejaría congelado el porcentaje heredado del SWR anterior — el modo de
/// fallo silencioso exacto que U4 existe para eliminar.
pub(crate) fn resolve_withdrawal_rule(rule: &WithdrawalRule, swr_pct: Decimal) -> WithdrawalRule {
    let mut r = rule.clone();
    // `inherit` decide por el par (valor, procedencia) y no solo por el valor: ver la nota de
    // idempotencia de arriba.
    let inherit = |value: &mut Option<Decimal>, source: &mut Option<PctSource>| {
        if value.is_none() || *source == Some(PctSource::Swr) {
            *value = Some(swr_pct);
            *source = Some(PctSource::Swr);
        } else {
            *source = Some(PctSource::Explicit);
        }
    };
    match r.kind {
        // Sin porcentaje que dimensionar: se retira la necesidad declarada, indexada y sin techo.
        WithdrawalRuleKind::FixedReal => r.pct_source = None,
        WithdrawalRuleKind::PercentOfBalance | WithdrawalRuleKind::Guardrails => {
            let mut source = r.pct_source;
            inherit(&mut r.pct, &mut source);
            r.pct_source = source;
        }
        WithdrawalRuleKind::Hybrid => {
            let mut source = r.pct_source;
            inherit(&mut r.start_pct, &mut source);
            r.pct_source = source;
        }
    }
    r
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
    p.target_retirement_age = p
        .target_retirement_age
        .map(|a| a.clamp(MIN_PROFILE_AGE, p.horizon_lifespan_age));
    // Dejar de aportar DESPUÉS de jubilarse no describe nada, así que el techo es la edad de
    // jubilación cuando la hay (ya clampada, así que nunca es menor que el suelo) y el horizonte
    // cuando no.
    let coast_ceiling = p
        .target_retirement_age
        .unwrap_or(p.horizon_lifespan_age)
        .max(MIN_PROFILE_AGE);
    p.coast_stop_age = p
        .coast_stop_age
        .map(|a| a.clamp(MIN_PROFILE_AGE, coast_ceiling));

    p.withdrawal_rule.pct = clamp_pct(p.withdrawal_rule.pct, MAX_WITHDRAWAL_PCT);
    p.withdrawal_rule.start_pct = clamp_pct(p.withdrawal_rule.start_pct, MAX_WITHDRAWAL_PCT);
    p.withdrawal_rule.end_pct = clamp_pct(p.withdrawal_rule.end_pct, MAX_WITHDRAWAL_PCT);
    p.withdrawal_rule.band_pct = clamp_pct(p.withdrawal_rule.band_pct, MAX_GUARDRAIL_PCT);
    p.withdrawal_rule.adjust_pct = clamp_pct(p.withdrawal_rule.adjust_pct, MAX_GUARDRAIL_PCT);
    // U4 — DESPUÉS de los clamps, para que lo que se hereda sea el `swr_pct` ya acotado.
    p.withdrawal_rule = resolve_withdrawal_rule(&p.withdrawal_rule, p.swr_pct);

    let horizon = p.horizon_lifespan_age;
    let swr = p.swr_pct;
    // **C7 — el alias `pension_bridge` enciende el puente.** El perfil guardado decía «mi
    // estrategia ES el puente»; en v2 eso se dice con `pension.bridge_enabled`, así que apagarlo
    // al migrar cambiaría el plan de esa persona sin que tocara nada.
    if p.migrated_from_pension_bridge {
        if let Some(pen) = p.pension.as_mut() {
            pen.bridge_enabled = true;
        }
    }
    if let Some(pen) = p.pension.as_mut() {
        pen.monthly_amount_today = pen.monthly_amount_today.max(Decimal::ZERO);
        pen.starts_at_age = pen.starts_at_age.clamp(MIN_PENSION_AGE.min(horizon), horizon);
        pen.fraction_while_partial = pen.fraction_while_partial.clamp(Decimal::ZERO, Decimal::ONE);
        if pen.bridge_enabled {
            // Encender el puente sin números no es «puente sin tope»: es un puente con los
            // defaults del owner (C7). Sin esto, `bridge_max_pct = None` con el puente encendido
            // llegaría al motor como una puerta sin cota — una retirada inicial libre.
            pen.bridge_max_pct = Some(pen.bridge_max_pct.unwrap_or_else(|| default_bridge_pct(swr)));
            pen.bridge_max_years = Some(pen.bridge_max_years.unwrap_or(DEFAULT_BRIDGE_YEARS));
        }
        // Clamps de lectura. El suelo del % es el SWR y NO «el SWR + un épsilon»: la cota de
        // ESCRITURA es abierta —`bridge_max_pct_not_above_swr`— y aquí solo hay que impedir un
        // valor imposible. Un puente igual al SWR es un puente que no concede nada: una lectura
        // honesta y acotada. Inventar un número que el usuario no escribió sería peor.
        pen.bridge_max_pct = pen
            .bridge_max_pct
            .map(|v| v.clamp(swr.min(MAX_BRIDGE_PCT), MAX_BRIDGE_PCT));
        pen.bridge_max_years = pen
            .bridge_max_years
            .map(|y| y.clamp(MIN_BRIDGE_YEARS, MAX_BRIDGE_YEARS));
    }
    if let Some(par) = p.partial_retirement.as_mut() {
        par.income_monthly_today = par.income_monthly_today.max(Decimal::ZERO);
        par.starts_at_age = par
            .starts_at_age
            .map(|a| a.clamp(MIN_PROFILE_AGE, horizon));
    }

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
        return Err(ApiError::BadRequest(format!(
            "swr_out_of_range: la tasa de retirada tiene que estar entre 0 y {MAX_SWR_PCT} %"
        )));
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

    // ---- Umbral de éxito (M2/C3) -----------------------------------------------------------
    if !(MIN_SUCCESS_THRESHOLD_PCT..=MAX_SUCCESS_THRESHOLD_PCT).contains(&p.success_threshold_pct) {
        return Err(ApiError::BadRequest(format!(
            "success_threshold_out_of_range: la probabilidad de éxito exigida tiene que estar entre {MIN_SUCCESS_THRESHOLD_PCT} y {MAX_SUCCESS_THRESHOLD_PCT} %"
        )));
    }

    // ---- Estrategia ------------------------------------------------------------------------
    if requires_target_age(p.strategy, p.coast_mode) && p.target_retirement_age.is_none() {
        return Err(ApiError::BadRequest(
            "target_retirement_age_required: falta la edad a la que quieres jubilarte".into(),
        ));
    }
    // Espejo exacto de la regla de arriba: una estrategia que nombra una fase exige el bloque que
    // la define. Sin él, `partial` no tenía fase parcial que simular y se comportaba como `asap`
    // en silencio — la UI enseñaba «Media jornada» sobre una proyección que no la tenía.
    if p.strategy == RetirementStrategy::Partial && p.partial_retirement.is_none() {
        return Err(ApiError::BadRequest(
            "partial_retirement_required: elige a partir de cuándo trabajas a media jornada".into(),
        ));
    }
    // Coast modo B: la edad de parada ES el dato del plan. Sin ella no hay nada que resolver, y
    // callarlo devolvería la conducta del modo A sin decirlo.
    if p.strategy == RetirementStrategy::Coast
        && p.coast_mode == CoastMode::FixedStopAge
        && p.coast_stop_age.is_none()
    {
        return Err(ApiError::BadRequest(
            "coast_stop_age_required: falta la edad a la que dejas de aportar".into(),
        ));
    }
    // Media jornada modo A: la edad de inicio ES el dato. Con `asap` la calcula el solver.
    if p.strategy == RetirementStrategy::Partial {
        if let Some(par) = &p.partial_retirement {
            if par.mode == PartialStartMode::AtAge && par.starts_at_age.is_none() {
                return Err(ApiError::BadRequest(
                    "partial_start_age_required: falta la edad a la que empiezas la media jornada"
                        .into(),
                ));
            }
        }
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
        // ---- El puente (C2/C7) -------------------------------------------------------------
        // Se valida SIEMPRE que haya números, esté encendido o no: guardar un puente imposible
        // «porque ahora está apagado» es dejar el error para el día que se encienda.
        if let Some(pct) = pen.bridge_max_pct {
            if pct <= Decimal::ZERO || pct > MAX_BRIDGE_PCT {
                return Err(ApiError::BadRequest(format!(
                    "bridge_max_pct_out_of_range: la tasa máxima del puente tiene que ser mayor que 0 y como mucho {MAX_BRIDGE_PCT} %"
                )));
            }
            // El puente es «una tasa inicial MAYOR con fecha límite» (C2): igual o menor que el
            // SWR no concede nada y el usuario creería estar adelantando su jubilación.
            if pct <= p.swr_pct {
                let swr = p.swr_pct.normalize();
                return Err(ApiError::BadRequest(format!(
                    "bridge_max_pct_not_above_swr: la tasa máxima del puente tiene que ser mayor que tu tasa de retirada ({swr} %)"
                )));
            }
        }
        if let Some(years) = pen.bridge_max_years {
            if !(MIN_BRIDGE_YEARS..=MAX_BRIDGE_YEARS).contains(&years) {
                return Err(ApiError::BadRequest(format!(
                    "bridge_max_years_out_of_range: los años máximos del puente tienen que estar entre {MIN_BRIDGE_YEARS} y {MAX_BRIDGE_YEARS}"
                )));
            }
        }
    }
    if let Some(age) = p.coast_stop_age {
        let ceiling = p.target_retirement_age.unwrap_or(horizon);
        if !(MIN_PROFILE_AGE..=ceiling).contains(&age) {
            return Err(ApiError::BadRequest(format!(
                "coast_stop_age_out_of_range: la edad a la que dejas de aportar tiene que estar entre {MIN_PROFILE_AGE} y {ceiling}"
            )));
        }
    }
    if let Some(par) = &p.partial_retirement {
        if let Some(age) = par.starts_at_age {
            if !(MIN_PROFILE_AGE..=horizon).contains(&age) {
                return Err(ApiError::BadRequest(format!(
                    "partial_age_out_of_range: partial_retirement.starts_at_age must be between {MIN_PROFILE_AGE} and horizon_lifespan_age ({horizon})"
                )));
            }
        }
        if par.income_monthly_today < Decimal::ZERO {
            return Err(ApiError::BadRequest(
                "partial_income_negative: partial_retirement.income_monthly_today must be >= 0"
                    .into(),
            ));
        }
        // La fase parcial termina en la jubilación total: empezar después (o el mismo mes) la
        // dejaría vacía, y una fase vacía que la UI dibuja es peor que un error.
        if let (Some(start), Some(total)) = (par.starts_at_age, p.target_retirement_age) {
            if start >= total {
                return Err(ApiError::BadRequest(
                    "partial_not_before_retirement: partial_retirement.starts_at_age must be lower than target_retirement_age".into(),
                ));
            }
        }
    }

    // U4 — se valida el porcentaje EFECTIVO, no el escrito: `pct`/`start_pct` ausentes heredan
    // `swr_pct` (ya comprobado arriba contra `MAX_SWR_PCT`), así que lo que hay que acotar es lo
    // que de verdad va a retirar el motor. Consecuencia declarada: con `swr_pct = 0` una regla
    // basada en saldo y sin `pct` propio es `withdrawal_pct_out_of_range` — un plan que retira 0 %
    // no es un plan, y callarlo devolvería una simulación que no vende nada sin decir por qué.
    validate_withdrawal_rule(&resolve_withdrawal_rule(&p.withdrawal_rule, p.swr_pct))
}

/// Cada `kind` exige SUS campos y no los de otro. Corre sobre la regla YA resuelta
/// (`resolve_withdrawal_rule`), así que `pct` y `start_pct` nunca llegan aquí ausentes para los
/// `kind` que los usan: lo que sigue vivo de `withdrawal_pct_required` son `end_pct` del `hybrid`
/// y la banda/ajuste de `guardrails`, que **no** heredan nada (no son porcentajes de retirada:
/// son el suelo del latch y la reacción de la regla).
fn validate_withdrawal_rule(r: &WithdrawalRule) -> Result<(), ApiError> {
    let need_pct = |label: &str, v: Option<Decimal>, max: Decimal| -> Result<Decimal, ApiError> {
        let Some(v) = v else {
            return Err(ApiError::BadRequest(format!(
                "withdrawal_pct_required: withdrawal_rule.{label} is required for this rule kind"
            )));
        };
        // B8 — el mensaje dice la COTA REAL, no «out of range» a secas. Sin el número, quien
        // escribe un 25 no sabe si el techo es 5, 20 o 100, y la SPA no puede decírselo: el
        // catálogo de `errorMessages.ts` traduce el código, no interpola cotas.
        if v <= Decimal::ZERO || v > max {
            let max = max.normalize();
            return Err(ApiError::BadRequest(format!(
                "withdrawal_pct_out_of_range: {label} tiene que ser mayor que 0 y como mucho {max} %"
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
                // B8 — el arranque puede ser HEREDADO del SWR (U4), así que el mensaje nombra el
                // número contra el que se ha comparado de verdad: sin él, «menor que start_pct»
                // señala a un campo que el usuario ha dejado vacío a propósito.
                let start = start.normalize();
                return Err(ApiError::BadRequest(format!(
                    "hybrid_end_pct_not_below_start: el porcentaje final tiene que ser menor que tu tasa de retirada ({start} %)"
                )));
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
///
/// **Corolario de U4, y la respuesta a «¿cómo suelto un `pct` explícito?»**: como el objeto se
/// reemplaza entero, mandar `withdrawal_rule` SIN la clave `pct` (o sin `start_pct`) ya la borra,
/// y el porcentaje vuelve a heredarse de `swr_pct`. Por eso no hay —ni debe haber— un
/// `clear_pct`: sería un segundo mecanismo para lo que el reemplazo ya hace.
#[derive(Debug, Default)]
pub(crate) struct RetirementProfilePatch {
    pub strategy: Option<RetirementStrategy>,
    pub target_retirement_age: Option<Option<u32>>,
    pub fire_number_mode: Option<FireNumberMode>,
    pub fire_number_manual_amount: Option<Option<Decimal>>,
    pub swr_pct: Option<Decimal>,
    pub horizon_lifespan_age: Option<u32>,
    /// **Load-bearing desde el modelo v2** (M2/C3). Entre V7 y v2 se aceptaba y se descartaba;
    /// ahora es la restricción que decide la fecha.
    pub success_threshold_pct: Option<u32>,
    pub coast_mode: Option<CoastMode>,
    pub coast_stop_age: Option<Option<u32>>,
    pub withdrawal_rule: Option<WithdrawalRule>,
    pub pension: Option<Option<PensionPlan>>,
    pub partial_retirement: Option<Option<PartialRetirement>>,
}

impl RetirementProfilePatch {
    /// Aplica el patchset sobre una base y devuelve el resultado, **sin validar ni persistir**.
    /// Lo comparten el PATCH HTTP y la tool MCP — dos aplicadores se separan sin que ningún
    /// test lo note (la lección de `FireSettingsPatch::apply_to`).
    pub(crate) fn apply_to(&self, base: &RetirementProfile) -> RetirementProfile {
        let mut after = base.clone();
        // **C7 — el alias `pension_bridge` se MATERIALIZA en la primera escritura.** El flag no
        // se serializa (es información del wire de entrada), así que sin esto un PATCH de
        // cualquier otro campo persistiría `strategy: "asap"` con `bridge_enabled: false` y el
        // puente de esa persona desaparecería sin que nada lo dijera. Se materializa SOLO el
        // encendido —que es la elección que el usuario expresó con el vocabulario viejo—: el
        // tope y los años se siguen derivando en lectura, para que muevan con el SWR.
        //
        // Va ANTES del patchset a propósito: un `pension` explícito en el mismo PATCH gana, y
        // quien manda un bloque de pensión entero está eligiendo, no arrastrando.
        if after.migrated_from_pension_bridge {
            if let Some(pen) = after.pension.as_mut() {
                pen.bridge_enabled = true;
            }
        }
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
        if let Some(v) = self.success_threshold_pct {
            after.success_threshold_pct = v;
        }
        if let Some(v) = self.coast_mode {
            after.coast_mode = v;
        }
        if let Some(v) = self.coast_stop_age {
            after.coast_stop_age = v;
        }
        if let Some(v) = self.withdrawal_rule.clone() {
            after.withdrawal_rule = v;
        }
        if let Some(v) = self.pension.clone() {
            after.pension = v;
            // **S4 murió con `target_basis` (M4).** Quitar la pensión ya no tiene que soltar
            // ninguna base derivada: en v2 la pensión es un flujo de caja y no dimensiona nada.
            // El puente, que sí colgaba de ella, se va con el bloque — vive DENTRO de `pension`.
        }
        if let Some(v) = self.partial_retirement.clone() {
            after.partial_retirement = v;
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
            success_threshold_pct,
            coast_mode,
            coast_stop_age,
            withdrawal_rule,
            pension,
            partial_retirement,
        } = self;
        strategy.is_none()
            && target_retirement_age.is_none()
            && fire_number_mode.is_none()
            && fire_number_manual_amount.is_none()
            && swr_pct.is_none()
            && horizon_lifespan_age.is_none()
            && success_threshold_pct.is_none()
            && coast_mode.is_none()
            && coast_stop_age.is_none()
            && withdrawal_rule.is_none()
            && pension.is_none()
            && partial_retirement.is_none()
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
///
/// **Sin `target_basis_stored` desde el modelo v2** (M4): ese campo existía para que un cliente
/// distinguiera la base del objetivo ELEGIDA de la DERIVADA, y en v2 no hay base del objetivo —
/// la pensión es un flujo de caja. Todo lo que el perfil publica es lo que el usuario escribió,
/// con sus defaults y sus clamps aplicados.
#[derive(Debug, Serialize, ToSchema)]
pub struct RetirementProfileResponse {
    pub profile: RetirementProfile,
    #[schema(value_type = Option<String>, format = "date")]
    pub birth_date: Option<NaiveDate>,
}

/// Cuerpo del PATCH. Tri-estado en todo lo opcional: **omitir = no cambiar**, `null` = borrar.
#[derive(Debug, Deserialize, ToSchema)]
pub struct PatchRetirementProfileBody {
    /// `asap` | `retire_at_age` | `coast` | `partial`.
    ///
    /// **El literal retirado `pension_bridge` se sigue aceptando y aterriza en `asap`** (C7), sin
    /// tocar el puente: encenderlo es `pension.bridge_enabled`. No se rechaza —un 400 rompería a
    /// quien reenvíe un perfil que leyó antes de v2— y no es silencioso: la respuesta devuelve
    /// `strategy: "asap"` y el `bridge_enabled` que haya. Lo que sí migra solo es el perfil ya
    /// ALMACENADO con ese literal (ver `RetirementProfilePatch::apply_to`).
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
    /// **Umbral de éxito exigido (%), `[80, 100]` — LOAD-BEARING desde el modelo v2** (M2/C3).
    ///
    /// Entre la decisión V7 y v2 este campo se «aceptaba y se ignoraba»: no estaba en el perfil,
    /// no se validaba y no salía por ninguna respuesta. Vuelve a mandar, y ahora decide la fecha:
    /// la jubilación válida es el primer mes en que al menos este porcentaje de los caminos llega
    /// al horizonte sin volver a trabajar. Fuera de rango es **400
    /// `success_threshold_out_of_range`** — donde antes era un 200 silencioso.
    ///
    /// La migración `20260906091500_drop_stored_success_threshold.sql` borra los valores que la
    /// promesa anterior dejó almacenados: quien quiera un umbral distinto del 95 lo vuelve a
    /// escribir, y nadie hereda como elección un número que nunca eligió.
    #[serde(default)]
    pub success_threshold_pct: Option<u32>,
    /// `fixed_retirement_age` (default) | `fixed_stop_age` (M10). En el modo B la edad de
    /// jubilación deja de ser obligatoria y `coast_stop_age` pasa a serlo.
    #[serde(default)]
    pub coast_mode: Option<CoastMode>,
    /// Edad a la que dejas de aportar. Tri-estado: omitir no la toca, un valor la fija, `null` la
    /// borra (y con el modo B, un perfil sin ella no valida).
    #[serde(default, deserialize_with = "crate::handlers::deserialize_double_option_typed")]
    #[schema(value_type = Option<u32>, nullable = true)]
    pub coast_stop_age: Option<Option<u32>>,
    /// Regla de retirada COMPLETA: **sustituye a la actual**, no se mergea campo a campo. `pct`
    /// y `start_pct` son opcionales (U4): omitidos heredan `swr_pct`, y omitirlos es justamente
    /// cómo se suelta un porcentaje que antes era explícito.
    #[serde(default)]
    pub withdrawal_rule: Option<WithdrawalRule>,
    /// Pensión COMPLETA (el puente vive dentro, C7): **sustituye a la actual**. `null` la borra —
    /// y con ella el puente, que sin pensión no tiene destino.
    #[serde(default, deserialize_with = "crate::handlers::deserialize_double_option_typed")]
    #[schema(value_type = Option<PensionPlan>, nullable = true)]
    pub pension: Option<Option<PensionPlan>>,
    #[serde(default, deserialize_with = "crate::handlers::deserialize_double_option_typed")]
    #[schema(value_type = Option<PartialRetirement>, nullable = true)]
    pub partial_retirement: Option<Option<PartialRetirement>>,
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
            success_threshold_pct: self.success_threshold_pct,
            coast_mode: self.coast_mode,
            coast_stop_age: self.coast_stop_age,
            withdrawal_rule: self.withdrawal_rule.clone(),
            pension: self.pension.clone(),
            partial_retirement: self.partial_retirement.clone(),
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
    let stored = stored.map(|j| j.0);
    Ok(RetirementProfileResponse {
        profile: resolve_retirement_profile(stored),
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
    // elegido» y «he elegido esto». En v2 ya no hay `target_basis` que derivar, pero la regla se
    // queda: los defaults del puente (C7) los pone `resolve_*` al leer, y mergear sobre el
    // resuelto los persistiría como si el usuario los hubiera escrito — con lo que mover el SWR
    // dejaría de mover el tope del puente que nadie eligió. Vale para cualquier campo derivado
    // futuro.
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

    fn pension(amount: u32, age: u32) -> PensionPlan {
        PensionPlan {
            monthly_amount_today: Decimal::from(amount),
            starts_at_age: age,
            indexed: true,
            fraction_while_partial: Decimal::ZERO,
            bridge_enabled: false,
            bridge_max_pct: None,
            bridge_max_years: None,
        }
    }

    #[test]
    fn an_absent_profile_is_the_v2_default() {
        let p = resolve_retirement_profile(None);
        assert_eq!(p.strategy, RetirementStrategy::Asap);
        assert_eq!(p.swr_pct, Decimal::new(35, 1));
        assert_eq!(p.horizon_lifespan_age, 90);
        assert_eq!(p.fire_number_mode, FireNumberMode::AnnualExpense);
        assert_eq!(p.withdrawal_rule.kind, WithdrawalRuleKind::FixedReal);
        assert_eq!(p.withdrawal_rule.spend_mode, SpendMode::Ceiling);
        // v2: el umbral vuelve al perfil (M2/C3) y el coast estrena modo (M10).
        assert_eq!(p.success_threshold_pct, DEFAULT_SUCCESS_THRESHOLD_PCT);
        assert_eq!(p.coast_mode, CoastMode::FixedRetirementAge);
        assert_eq!(p.coast_stop_age, None);
        assert!(!p.migrated_from_pension_bridge);
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

    /// **Un JSONB de 5.0.0-WP5 sigue cargando y sus tres claves retiradas se ignoran solas.**
    /// El struct no lleva `deny_unknown_fields` a propósito: si lo llevara, el perfil de todo el
    /// que hubiera tocado la pantalla antes de v2 dejaría de deserializar — y un perfil que no
    /// carga es un 500 en la pantalla de jubilación, no un default.
    #[test]
    fn a_stored_v1_profile_ignores_the_three_retired_keys() {
        let stored: RetirementProfile = serde_json::from_str(
            r#"{"strategy":"asap","swr_pct":"3.5","target_basis":"bridge_to_pension",
                "bridge_discount_basis":"swr","cash_buffer_months":24}"#,
        )
        .expect("un perfil v1 tiene que seguir cargando");
        let p = resolve_retirement_profile(Some(stored));
        assert_eq!(p, resolve_retirement_profile(None));
        // Y no vuelven por la salida: lo que se publica es el perfil v2 y nada más.
        let json = serde_json::to_value(&p).expect("serializa");
        for dead in ["target_basis", "bridge_discount_basis", "cash_buffer_months"] {
            assert!(json.get(dead).is_none(), "{dead} no debe re-emitirse: {json}");
        }
    }

    /// **C7 — el literal `pension_bridge` es un ALIAS de `asap` que enciende el puente.**
    #[test]
    fn the_pension_bridge_literal_migrates_to_asap_with_the_bridge_on() {
        let stored: RetirementProfile = serde_json::from_str(
            r#"{"strategy":"pension_bridge","swr_pct":"3.5",
                "pension":{"monthly_amount_today":"1200","starts_at_age":67}}"#,
        )
        .expect("el alias tiene que seguir deserializando");
        assert_eq!(stored.strategy, RetirementStrategy::Asap);
        assert!(stored.migrated_from_pension_bridge, "el alias debe quedar registrado");

        let p = resolve_retirement_profile(Some(stored));
        let pen = p.pension.as_ref().expect("la pensión sigue ahí");
        assert!(pen.bridge_enabled, "el alias enciende el puente");
        // Defaults del owner: `max(5, swr + 1)` % y 7 años.
        assert_eq!(pen.bridge_max_pct, Some(Decimal::from(5u32)));
        assert_eq!(pen.bridge_max_years, Some(DEFAULT_BRIDGE_YEARS));

        // Y NUNCA se re-emite: lo que se guarda es `asap`.
        let json = serde_json::to_value(&p).expect("serializa");
        assert_eq!(json["strategy"], "asap", "{json}");
        assert!(
            json.get("migrated_from_pension_bridge").is_none(),
            "el flag no viaja por el wire: {json}"
        );

        // Un perfil `asap` normal no enciende nada.
        let plain: RetirementProfile = serde_json::from_str(
            r#"{"strategy":"asap","pension":{"monthly_amount_today":"1200","starts_at_age":67}}"#,
        )
        .expect("asap");
        assert!(!plain.migrated_from_pension_bridge);
        assert!(!resolve_retirement_profile(Some(plain)).pension.unwrap().bridge_enabled);
    }

    /// Una estrategia desconocida trae la lista de las CUATRO vivas — el alias no se ofrece.
    #[test]
    fn an_unknown_strategy_lists_the_four_live_variants() {
        let err = serde_json::from_str::<RetirementProfile>(r#"{"strategy":"no_existe"}"#)
            .expect_err("literal desconocido");
        let msg = err.to_string();
        for v in RETIREMENT_STRATEGY_VARIANTS {
            assert!(msg.contains(v), "el error debe listar `{v}`: {msg}");
        }
        assert!(
            !msg.contains(PENSION_BRIDGE_ALIAS),
            "el alias no es una opción que ofrecer: {msg}"
        );
    }

    /// El default del puente es `max(5, swr + 1)`: nunca por debajo del 5, y siempre por encima
    /// del SWR (un puente que no levanta la tasa no es un puente).
    #[test]
    fn the_default_bridge_pct_is_five_or_the_swr_plus_one() {
        assert_eq!(default_bridge_pct(Decimal::new(35, 1)), Decimal::from(5u32));
        assert_eq!(default_bridge_pct(Decimal::from(3u32)), Decimal::from(5u32));
        assert_eq!(default_bridge_pct(Decimal::from(5u32)), Decimal::from(6u32));
        assert_eq!(default_bridge_pct(Decimal::new(55, 1)), Decimal::new(65, 1));
        for swr in [Decimal::ZERO, Decimal::from(4u32), MAX_SWR_PCT] {
            assert!(default_bridge_pct(swr) > swr, "swr = {swr}");
            assert!(default_bridge_pct(swr) <= MAX_BRIDGE_PCT);
        }
    }

    /// Encender el puente sin números lo deja con los defaults — con CUALQUIER estrategia (C7).
    #[test]
    fn enabling_the_bridge_without_numbers_fills_the_defaults() {
        let mut p = default_retirement_profile();
        p.strategy = RetirementStrategy::Coast;
        p.target_retirement_age = Some(60);
        p.swr_pct = Decimal::from(5u32);
        p.pension = Some(PensionPlan {
            bridge_enabled: true,
            ..pension(1200, 67)
        });
        let r = resolve_retirement_profile(Some(p));
        let pen = r.pension.expect("pensión");
        assert_eq!(pen.bridge_max_pct, Some(Decimal::from(6u32)));
        assert_eq!(pen.bridge_max_years, Some(7));
    }

    #[test]
    fn out_of_range_values_are_clamped_on_read_never_rejected() {
        let mut p = default_retirement_profile();
        p.swr_pct = Decimal::from(99u32);
        p.horizon_lifespan_age = 200;
        p.target_retirement_age = Some(3);
        p.success_threshold_pct = 500;
        p.coast_stop_age = Some(2);
        let r = resolve_retirement_profile(Some(p.clone()));
        assert_eq!(r.swr_pct, MAX_SWR_PCT);
        assert_eq!(r.horizon_lifespan_age, MAX_HORIZON_LIFESPAN_AGE);
        assert_eq!(r.target_retirement_age, Some(MIN_PROFILE_AGE));
        assert_eq!(r.success_threshold_pct, MAX_SUCCESS_THRESHOLD_PCT);
        assert_eq!(r.coast_stop_age, Some(MIN_PROFILE_AGE));

        // Y por abajo.
        p.success_threshold_pct = 0;
        p.target_retirement_age = Some(60);
        p.coast_stop_age = Some(200);
        let r = resolve_retirement_profile(Some(p));
        assert_eq!(r.success_threshold_pct, MIN_SUCCESS_THRESHOLD_PCT);
        // El techo de «dejo de aportar» es la edad de jubilación cuando la hay.
        assert_eq!(r.coast_stop_age, Some(60));
    }

    /// Los números del puente se ACOTAN al leer (la vía no validada: restore, edición directa).
    #[test]
    fn the_bridge_numbers_are_clamped_on_read() {
        let mut p = default_retirement_profile();
        p.swr_pct = Decimal::from(4u32);
        p.pension = Some(PensionPlan {
            bridge_enabled: true,
            bridge_max_pct: Some(Decimal::from(99u32)),
            bridge_max_years: Some(999),
            ..pension(1000, 65)
        });
        let pen = resolve_retirement_profile(Some(p.clone())).pension.expect("pensión");
        assert_eq!(pen.bridge_max_pct, Some(MAX_BRIDGE_PCT));
        assert_eq!(pen.bridge_max_years, Some(MAX_BRIDGE_YEARS));

        // Por debajo del SWR el suelo es el SWR: un puente que no concede nada, pero acotado y
        // sin inventar un número que el usuario no escribió.
        p.pension = Some(PensionPlan {
            bridge_enabled: true,
            bridge_max_pct: Some(Decimal::from(1u32)),
            bridge_max_years: Some(0),
            ..pension(1000, 65)
        });
        let pen = resolve_retirement_profile(Some(p)).pension.expect("pensión");
        assert_eq!(pen.bridge_max_pct, Some(Decimal::from(4u32)));
        assert_eq!(pen.bridge_max_years, Some(MIN_BRIDGE_YEARS));
    }

    /// [`requires_target_age`] depende del MODO de coast (M10), no solo de la estrategia.
    #[test]
    fn only_the_age_driven_plans_require_the_retirement_age() {
        assert!(requires_target_age(
            RetirementStrategy::RetireAtAge,
            CoastMode::FixedRetirementAge
        ));
        assert!(requires_target_age(
            RetirementStrategy::RetireAtAge,
            CoastMode::FixedStopAge
        ));
        assert!(requires_target_age(
            RetirementStrategy::Coast,
            CoastMode::FixedRetirementAge
        ));
        assert!(!requires_target_age(
            RetirementStrategy::Coast,
            CoastMode::FixedStopAge
        ));
        for s in [RetirementStrategy::Asap, RetirementStrategy::Partial] {
            assert!(!requires_target_age(s, CoastMode::FixedRetirementAge));
            assert!(!requires_target_age(s, CoastMode::FixedStopAge));
        }
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
        // …salvo coast en modo B, donde el dato es la edad de PARADA.
        let mut p = default_retirement_profile();
        p.strategy = RetirementStrategy::Coast;
        p.coast_mode = CoastMode::FixedStopAge;
        let err = validate_retirement_profile(&p).expect_err("sin edad de parada debe fallar");
        assert!(
            matches!(&err, ApiError::BadRequest(m) if m.starts_with("coast_stop_age_required: ")),
            "{err:?}"
        );
        p.coast_stop_age = Some(50);
        validate_retirement_profile(&p).expect("coast B con edad de parada y sin edad de jubilación");
    }

    /// Un puente con tasa igual o menor que el SWR se RECHAZA al escribir.
    #[test]
    fn a_bridge_pct_at_or_below_the_swr_is_rejected() {
        let mut p = default_retirement_profile();
        p.swr_pct = Decimal::from(4u32);
        for pct in [Decimal::from(3u32), Decimal::from(4u32)] {
            p.pension = Some(PensionPlan {
                bridge_enabled: true,
                bridge_max_pct: Some(pct),
                ..pension(1000, 65)
            });
            let err = validate_retirement_profile(&p).expect_err("una tasa de puente <= swr debe fallar");
            assert!(
                matches!(&err, ApiError::BadRequest(m) if m.starts_with("bridge_max_pct_not_above_swr: ")),
                "{err:?}"
            );
            // El mensaje dice la cota REAL (B8), no «out of range» a secas.
            let ApiError::BadRequest(m) = &err else { unreachable!() };
            assert!(m.contains('4'), "el mensaje debe nombrar el SWR: {m}");
        }
        p.pension = Some(PensionPlan {
            bridge_enabled: true,
            bridge_max_pct: Some(Decimal::from(5u32)),
            ..pension(1000, 65)
        });
        validate_retirement_profile(&p).expect("por encima del SWR entra");

        // Y las cotas duras del puente.
        p.pension = Some(PensionPlan {
            bridge_enabled: true,
            bridge_max_pct: Some(Decimal::from(25u32)),
            ..pension(1000, 65)
        });
        let err = validate_retirement_profile(&p).expect_err("por encima del techo");
        assert!(
            matches!(&err, ApiError::BadRequest(m) if m.starts_with("bridge_max_pct_out_of_range: ")),
            "{err:?}"
        );
        p.pension = Some(PensionPlan {
            bridge_enabled: true,
            bridge_max_pct: Some(Decimal::from(5u32)),
            bridge_max_years: Some(50),
            ..pension(1000, 65)
        });
        let err = validate_retirement_profile(&p).expect_err("demasiados años");
        assert!(
            matches!(&err, ApiError::BadRequest(m) if m.starts_with("bridge_max_years_out_of_range: ")),
            "{err:?}"
        );
    }

    #[test]
    fn the_success_threshold_is_bounded_on_write() {
        let mut p = default_retirement_profile();
        for bad in [0u32, 79, 101, 1_000] {
            p.success_threshold_pct = bad;
            let err = validate_retirement_profile(&p).expect_err("umbral fuera de rango debe fallar");
            assert!(
                matches!(&err, ApiError::BadRequest(m) if m.starts_with("success_threshold_out_of_range: ")),
                "{err:?}"
            );
        }
        for ok in [MIN_SUCCESS_THRESHOLD_PCT, 95, MAX_SUCCESS_THRESHOLD_PCT] {
            p.success_threshold_pct = ok;
            validate_retirement_profile(&p).expect("dentro de rango");
        }
    }

    #[test]
    fn each_withdrawal_kind_demands_its_own_fields() {
        let mut p = default_retirement_profile();

        // U4: `percent_of_balance` SIN `pct` ya no es un error — hereda el SWR.
        p.withdrawal_rule = WithdrawalRule {
            kind: WithdrawalRuleKind::PercentOfBalance,
            ..WithdrawalRule::default()
        };
        validate_retirement_profile(&p).expect("percent sin pct hereda el SWR");

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
        // B8 — el mensaje nombra el arranque REAL contra el que se comparó.
        let ApiError::BadRequest(m) = &err else { unreachable!() };
        assert!(m.contains('3'), "{m}");

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

        // Lo que SIGUE siendo obligatorio tras U4: el `end_pct` del hybrid y la banda/ajuste de
        // guardrails. No son porcentajes de retirada, así que no heredan nada.
        p.withdrawal_rule = WithdrawalRule {
            kind: WithdrawalRuleKind::Hybrid,
            ..WithdrawalRule::default()
        };
        let err = validate_retirement_profile(&p).expect_err("hybrid sin end_pct");
        assert!(
            matches!(&err, ApiError::BadRequest(m) if m.starts_with("withdrawal_pct_required: ")),
            "{err:?}"
        );
    }

    /// U4 — el porcentaje de retirada es ÚNICO: `swr_pct` dimensiona el objetivo Y es el % de la
    /// regla basada en saldo cuando el usuario no escribe uno propio.
    #[test]
    fn a_missing_withdrawal_pct_inherits_the_swr_and_says_so() {
        let mut stored = default_retirement_profile();
        stored.swr_pct = Decimal::new(30, 1); // 3,0 %
        stored.withdrawal_rule = WithdrawalRule {
            kind: WithdrawalRuleKind::PercentOfBalance,
            ..WithdrawalRule::default()
        };
        let r = resolve_retirement_profile(Some(stored.clone()));
        assert_eq!(r.withdrawal_rule.pct, Some(Decimal::new(30, 1)));
        assert_eq!(r.withdrawal_rule.pct_source, Some(PctSource::Swr));

        // Explícito: se honra y se declara como tal.
        stored.withdrawal_rule.pct = Some(Decimal::from(2u32));
        let r = resolve_retirement_profile(Some(stored.clone()));
        assert_eq!(r.withdrawal_rule.pct, Some(Decimal::from(2u32)));
        assert_eq!(r.withdrawal_rule.pct_source, Some(PctSource::Explicit));

        // `hybrid` hereda por `start_pct`; `end_pct` no hereda nada.
        stored.withdrawal_rule = WithdrawalRule {
            kind: WithdrawalRuleKind::Hybrid,
            end_pct: Some(Decimal::from(2u32)),
            ..WithdrawalRule::default()
        };
        let r = resolve_retirement_profile(Some(stored.clone()));
        assert_eq!(r.withdrawal_rule.start_pct, Some(Decimal::new(30, 1)));
        assert_eq!(r.withdrawal_rule.end_pct, Some(Decimal::from(2u32)));
        assert_eq!(r.withdrawal_rule.pct_source, Some(PctSource::Swr));

        // `fixed_real` no tiene porcentaje: el campo NO viaja (ni siquiera como `null`).
        stored.withdrawal_rule = WithdrawalRule::default();
        let r = resolve_retirement_profile(Some(stored));
        assert_eq!(r.withdrawal_rule.pct_source, None);
        let json = serde_json::to_value(&r.withdrawal_rule).expect("serializa");
        assert!(
            json.get("pct_source").is_none(),
            "fixed_real no debe publicar pct_source: {json}"
        );
    }

    /// El resolvedor corre otra vez sobre lo que él mismo resolvió (el what-if aplica su patch
    /// sobre el perfil RESUELTO). Un % heredado se RE-hereda del SWR nuevo; uno explícito no se
    /// mueve. Sin esto, un `profile_overrides` que solo tocara `swr_pct` dejaría congelado el
    /// porcentaje del SWR anterior — silenciosamente.
    #[test]
    fn re_resolving_re_inherits_the_swr_but_never_touches_an_explicit_pct() {
        let mut p = default_retirement_profile();
        p.withdrawal_rule = WithdrawalRule {
            kind: WithdrawalRuleKind::PercentOfBalance,
            ..WithdrawalRule::default()
        };
        let resolved = resolve_retirement_profile(Some(p));
        assert_eq!(resolved.withdrawal_rule.pct, Some(Decimal::new(35, 1)));

        let mut moved = resolved.clone();
        moved.swr_pct = Decimal::from(2u32);
        let again = resolve_retirement_profile(Some(moved));
        assert_eq!(again.withdrawal_rule.pct, Some(Decimal::from(2u32)));
        assert_eq!(again.withdrawal_rule.pct_source, Some(PctSource::Swr));

        let mut explicit = resolved;
        explicit.withdrawal_rule.pct = Some(Decimal::from(1u32));
        explicit.withdrawal_rule.pct_source = Some(PctSource::Explicit);
        explicit.swr_pct = Decimal::from(2u32);
        let again = resolve_retirement_profile(Some(explicit));
        assert_eq!(again.withdrawal_rule.pct, Some(Decimal::from(1u32)));
        assert_eq!(again.withdrawal_rule.pct_source, Some(PctSource::Explicit));
    }

    /// La media jornada en modo `asap` no necesita edad de inicio; en modo `at_age`, sí.
    #[test]
    fn the_partial_start_age_is_required_only_in_at_age_mode() {
        let mut p = default_retirement_profile();
        p.strategy = RetirementStrategy::Partial;
        p.partial_retirement = Some(PartialRetirement {
            starts_at_age: None,
            income_monthly_today: Decimal::from(900u32),
            expense_basis: PartialExpenseBasis::Retirement,
            mode: PartialStartMode::AtAge,
        });
        let err = validate_retirement_profile(&p).expect_err("modo A sin edad");
        assert!(
            matches!(&err, ApiError::BadRequest(m) if m.starts_with("partial_start_age_required: ")),
            "{err:?}"
        );
        p.partial_retirement.as_mut().unwrap().mode = PartialStartMode::AsSoonAsPossible;
        validate_retirement_profile(&p).expect("modo B sin edad");
    }

    #[test]
    fn the_partial_phase_must_start_before_the_full_retirement() {
        let mut p = default_retirement_profile();
        p.strategy = RetirementStrategy::Partial;
        p.target_retirement_age = Some(60);
        p.partial_retirement = Some(PartialRetirement {
            starts_at_age: Some(60),
            income_monthly_today: Decimal::from(1000u32),
            expense_basis: PartialExpenseBasis::Retirement,
            mode: PartialStartMode::AtAge,
        });
        let err = validate_retirement_profile(&p).expect_err("parcial no anterior");
        assert!(
            matches!(&err, ApiError::BadRequest(m) if m.starts_with("partial_not_before_retirement: ")),
            "{err:?}"
        );
        p.partial_retirement.as_mut().unwrap().starts_at_age = Some(55);
        validate_retirement_profile(&p).expect("parcial antes de la total");
    }

    #[test]
    fn the_patch_only_touches_what_it_names() {
        let base = RetirementProfile {
            swr_pct: Decimal::new(30, 1),
            pension: Some(pension(1100, 67)),
            ..default_retirement_profile()
        };
        let patch = RetirementProfilePatch {
            swr_pct: Some(Decimal::new(35, 1)),
            ..RetirementProfilePatch::default()
        };
        let after = patch.apply_to(&base);
        assert_eq!(after.swr_pct, Decimal::new(35, 1));
        assert_eq!(after.pension, base.pension, "la pensión NO se resetea");
        assert_eq!(after.success_threshold_pct, base.success_threshold_pct);

        // `null` explícito sí borra — y el puente se va con la pensión, porque vive dentro.
        let clear = RetirementProfilePatch {
            pension: Some(None),
            ..RetirementProfilePatch::default()
        };
        assert_eq!(clear.apply_to(&base).pension, None);
        assert!(RetirementProfilePatch::default().is_empty());
        assert!(!clear.is_empty());

        // El umbral es un campo más del patchset (ya no un descarte).
        let threshold = RetirementProfilePatch {
            success_threshold_pct: Some(90),
            ..RetirementProfilePatch::default()
        };
        assert!(!threshold.is_empty());
        assert_eq!(threshold.apply_to(&base).success_threshold_pct, 90);

        // `coast_stop_age` es tri-estado: omitir ≠ `null`.
        let base_coast = RetirementProfile {
            coast_stop_age: Some(50),
            ..default_retirement_profile()
        };
        assert_eq!(
            RetirementProfilePatch {
                coast_mode: Some(CoastMode::FixedStopAge),
                ..RetirementProfilePatch::default()
            }
            .apply_to(&base_coast)
            .coast_stop_age,
            Some(50),
            "omitir no toca"
        );
        assert_eq!(
            RetirementProfilePatch {
                coast_stop_age: Some(None),
                ..RetirementProfilePatch::default()
            }
            .apply_to(&base_coast)
            .coast_stop_age,
            None,
            "`null` borra"
        );
    }
}
