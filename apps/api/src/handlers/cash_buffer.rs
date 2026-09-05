//! **De dónde sale el colchón de caja** (5.0.0, decisión V6 del owner + P2 del plan).
//!
//! Hasta 5.0.0 el colchón era un campo del perfil (`cash_buffer_months`) que la SPA pedía en un
//! formulario. La decisión V6 lo retira de la pantalla y lo **deriva del tope de la regla de
//! ahorro**: si tu cascada dice «la cuenta corriente hasta 6.000 €, el resto al fondo», ese 6.000
//! ES tu colchón —lo es mientras ahorras y lo sigue siendo jubilado—, y no hace falta que lo
//! declares dos veces con dos números que pueden discrepar.
//!
//! # Por qué el colchón derivado es un IMPORTE y no unos meses (P2)
//!
//! El tope `amount` de una regla es un importe **nominal fijo** que la cascada persigue sin
//! indexar nunca; el colchón en MESES se dimensiona contra el gasto **ya indexado** de cada mes.
//! Convertir un tope de 6.000 € a «≈ 2,4 meses» a mes 0 y dejar que se indexe daría, a 35 años
//! con un 2,5 %, un objetivo de ~14.200 € nominales: 2,4× lo que el usuario escribió. Por eso el
//! motor tiene desde 5.0.0 la variante `CashBufferTarget::Amount` y esta derivación la usa: **la
//! misma regla gobierna las dos fases**.
//!
//! # Por qué vive AQUÍ y no en `resolve_retirement_profile`
//!
//! Esa función es pura, síncrona y **sin ledger por contrato** (D25): resuelve defaults y clamps
//! del perfil almacenado y nada más, y `GET /v1/auth/me/retirement-profile` depende de que siga
//! así. La derivación necesita activos, reglas de asignación y el gasto de jubilación — o sea el
//! ensamblado entero de la proyección. Por eso se resuelve al final de
//! `build_installation_projection_input` y viaja en `BuiltProjection`.
//!
//! # Qué se publica
//!
//! En la respuesta de **bandas** y en los KPIs del what-if, nunca en el GET del perfil (P3): el
//! perfil sigue siendo ledger-free. La familia `buffer_*` ya existía; esta derivación añade
//! `buffer_source`, `buffer_target_amount`, `buffer_months_effective`, `buffer_source_rule_id` y
//! `buffer_source_asset_name`, y **reutiliza** `buffer_inactive_reason`, que sigue siendo UN solo
//! campo: los motivos del handler sustituyen al `not_requested` del motor, y los del motor
//! (`no_volatility`) pasan tal cual.

use rust_decimal::prelude::ToPrimitive;
use rust_decimal::Decimal;
use uuid::Uuid;

use futurefin_engine::{resolve_cap_ceiling, safe_cash_buffer_index, AllocationRule, SimAsset, SimAssetG};
use futurefin_engine_stochastic::CashBufferSpec;

/// El colchón lo pidió el usuario a mano (perfil o `profile_overrides`), en meses.
pub(crate) const BUFFER_SOURCE_EXPLICIT: &str = "explicit";
/// El colchón sale del tope de una regla de ahorro. Es el caso por defecto desde 5.0.0.
pub(crate) const BUFFER_SOURCE_ALLOCATION_CAP: &str = "allocation_cap";
/// No hay colchón. `buffer_inactive_reason` dice por qué.
pub(crate) const BUFFER_SOURCE_NONE: &str = "none";

/// No hay ninguna regla **habilitada y con tope** apuntando al activo que haría de colchón. Es el
/// caso común de la pauta «todo al fondo»: el líquido σ=0 es el sumidero sin tope (invariante I1)
/// y no hay ningún importe que perseguir.
pub(crate) const BUFFER_INACTIVE_NO_CAPPED_RULE: &str = "no_capped_rule";
/// Hay regla con tope, pero su techo resuelto en euros es 0 o negativo (`Amount(0)`, o un
/// `months_expense` sobre un gasto nulo). Un colchón de 0 € no es un colchón.
pub(crate) const BUFFER_INACTIVE_CAP_IS_ZERO: &str = "cap_is_zero";
/// No hay ningún activo **líquido con σ = 0** donde alojarlo. MISMO literal que el del motor
/// (`BufferInactiveReason::NoSafeLiquidAsset::code`) porque es el mismo hecho: el handler solo
/// llega antes.
pub(crate) const BUFFER_INACTIVE_NO_SAFE_LIQUID_ASSET: &str = "no_safe_liquid_asset";

/// El colchón resuelto para una simulación, listo para viajar al Monte Carlo **y** a la
/// respuesta.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ResolvedCashBuffer {
    /// Lo que se le pasa a `McConfig::cash_buffer`. `None` = no se pide colchón, y entonces el
    /// motor contestará `not_requested` — que aquí se sustituye por el motivo REAL.
    pub spec: Option<CashBufferSpec>,
    /// `explicit` | `allocation_cap` | `none`.
    pub source: &'static str,
    /// El objetivo en **euros nominales**. Solo con `allocation_cap`: con `explicit` el objetivo
    /// se indexa mes a mes y publicar un escalar sería publicar una de sus infinitas caras.
    pub target_amount: Option<Decimal>,
    /// Meses. Con `explicit`, los que el usuario escribió. Con `allocation_cap`, el equivalente
    /// **informativo** `floor(techo / gasto de jubilación)` — para el copy, no para el motor.
    /// `None` cuando no hay colchón, o cuando el gasto de jubilación no es positivo (dividir por
    /// él daría un número sin lectura).
    pub months_effective: Option<u32>,
    /// La regla cuyo tope fijó el objetivo. Solo con `allocation_cap`.
    pub source_rule_id: Option<Uuid>,
    /// El activo que hará de colchón (el líquido σ=0 de menor rentabilidad, el mismo que elige el
    /// motor). Se publica también con `explicit`: saber DÓNDE se aloja el colchón es la mitad de
    /// entenderlo.
    pub source_asset_name: Option<String>,
    /// Por qué no hay colchón, **decidido aquí**. `None` = hay colchón, o el motivo lo pondrá el
    /// motor (`no_volatility`).
    pub inactive_reason: Option<&'static str>,
}

impl ResolvedCashBuffer {
    /// El «no hay colchón, y este es el motivo».
    fn none(reason: &'static str, asset_name: Option<String>) -> Self {
        ResolvedCashBuffer {
            spec: None,
            source: BUFFER_SOURCE_NONE,
            target_amount: None,
            months_effective: None,
            source_rule_id: None,
            source_asset_name: asset_name,
            inactive_reason: Some(reason),
        }
    }
}

/// **¿Qué activos cuentan como libres de riesgo?** — la MISMA política que
/// `futurefin_engine_stochastic::mc::monthly_sigma`, que es privada: una volatilidad ausente,
/// nula, negativa o no finita ⇒ σ = 0 ⇒ el activo es determinista.
///
/// Se escribe una vez y se usa aquí y en el test de igualdad de índice. Si el handler y el motor
/// eligieran activos distintos, la respuesta nombraría una cuenta y el sorteo rellenaría otra —
/// un fallo perfectamente silencioso (riesgo R6 del plan).
pub(crate) fn risk_free_flags(asset_volatility_percent: &[Option<Decimal>]) -> Vec<bool> {
    asset_volatility_percent
        .iter()
        .map(|v| {
            !matches!(
                v.and_then(|d| d.to_f64()),
                Some(x) if x.is_finite() && x > 0.0
            )
        })
        .collect()
}

/// **La derivación entera**, en cinco pasos y con un motivo por cada salida.
///
/// 1. **Explícito gana** (patrón `pct_source`): `cash_buffer_months` almacenado —o el de
///    `profile_overrides` en el what-if— es una elección, y una elección no se deriva. Volver a
///    derivado ya tiene camino: `PATCH {"cash_buffer_months": null}`.
/// 2. **El activo es el que usará el MOTOR**: `safe_cash_buffer_index` con el mismo vector
///    `risk_free`. Sin líquido σ=0 no hay colchón posible → `no_safe_liquid_asset`.
/// 3. **Reglas habilitadas y con tope que apuntan a ese activo.** El ensamblado ya filtró
///    `enabled = true` y resolvió el activo destino a un índice, así que la lista de reglas que
///    llega aquí es exactamente la que la cascada ejecuta. Ninguna → `no_capped_rule`.
/// 4. **El techo es el MÁXIMO**, no la suma ni el mínimo: la cascada evalúa `cap_room` contra el
///    valor VIVO del activo, compartido por todas las reglas que apuntan a él, así que el estado
///    alcanzable lo fija el techo mayor. Sumar prometería un saldo que la cascada nunca alcanza;
///    el mínimo infraprometería. Techo ≤ 0 → `cap_is_zero`.
/// 5. `Amount(techo)` al motor, y `floor(techo / gasto de jubilación)` como equivalente
///    **informativo** en meses.
///
/// `allocation_rules` y `allocation_rule_ids` van alineados posición a posición, igual que en
/// `BuiltProjection`; `assets` y `asset_id_name` también.
#[allow(clippy::too_many_arguments)]
pub(crate) fn resolve_cash_buffer(
    explicit_months: Option<u32>,
    assets: &[SimAsset],
    asset_volatility_percent: &[Option<Decimal>],
    asset_id_name: &[(Uuid, String)],
    allocation_rules: &[AllocationRule],
    allocation_rule_ids: &[Uuid],
    income_monthly: Decimal,
    expense_with_debt: Decimal,
    expense_retirement_monthly: Decimal,
) -> ResolvedCashBuffer {
    // (2) El activo, ANTES que nada: se publica igual con colchón explícito, porque «dónde se
    //     aloja» es parte de entenderlo. Y con `explicit` NO se usa para decidir: quien lo pidió
    //     a mano recibe el motivo del MOTOR si no hay dónde alojarlo (`no_safe_liquid_asset`),
    //     que es exactamente el mismo literal.
    let sim_assets: Vec<SimAssetG<Decimal>> = assets.iter().map(SimAssetG::from).collect();
    let risk_free = risk_free_flags(asset_volatility_percent);
    let buffer_index = safe_cash_buffer_index(&sim_assets, &risk_free);
    let buffer_asset_name = buffer_index
        .and_then(|i| asset_id_name.get(i))
        .map(|(_, name)| name.clone());

    // (1) Explícito gana.
    if let Some(months) = explicit_months {
        return ResolvedCashBuffer {
            spec: Some(CashBufferSpec::Months(months)),
            source: BUFFER_SOURCE_EXPLICIT,
            target_amount: None,
            months_effective: Some(months),
            source_rule_id: None,
            source_asset_name: buffer_asset_name,
            inactive_reason: None,
        };
    }

    let Some(buffer_index) = buffer_index else {
        return ResolvedCashBuffer::none(BUFFER_INACTIVE_NO_SAFE_LIQUID_ASSET, None);
    };

    // (3) + (4) El techo mayor entre las reglas con tope que apuntan a ese activo.
    let mut best: Option<(Decimal, Uuid)> = None;
    for (i, rule) in allocation_rules.iter().enumerate() {
        if rule.target_index != buffer_index {
            continue;
        }
        // `resolve_cap_ceiling` es el ÚNICO resolutor de techos de la casa (el mismo que
        // `resolve_cap_ceiling_eur` de `allocation_rules.rs` envuelve para las superficies que
        // solo tienen el par `(cap_kind, cap_value)` en texto). Aquí el tope ya viene tipado del
        // ensamblado, así que se llama directo: sin adaptador que pueda quedarse atrás.
        let Some(ceiling) = resolve_cap_ceiling(rule.cap, expense_with_debt, income_monthly)
        else {
            continue;
        };
        let Some(rule_id) = allocation_rule_ids.get(i).copied() else {
            continue;
        };
        if best.map_or(true, |(b, _)| ceiling > b) {
            best = Some((ceiling, rule_id));
        }
    }

    let Some((ceiling, rule_id)) = best else {
        return ResolvedCashBuffer::none(BUFFER_INACTIVE_NO_CAPPED_RULE, buffer_asset_name);
    };
    if ceiling <= Decimal::ZERO {
        return ResolvedCashBuffer::none(BUFFER_INACTIVE_CAP_IS_ZERO, buffer_asset_name);
    }

    // (5) El equivalente en meses es INFORMATIVO: euros de hoy sobre el gasto de jubilación de
    //     hoy. No entra al motor y no manda sobre nada — el objetivo es el importe.
    let months_effective = (expense_retirement_monthly > Decimal::ZERO)
        .then(|| (ceiling / expense_retirement_monthly).floor().to_u32())
        .flatten();

    ResolvedCashBuffer {
        spec: Some(CashBufferSpec::Amount(ceiling)),
        source: BUFFER_SOURCE_ALLOCATION_CAP,
        target_amount: Some(ceiling),
        months_effective,
        source_rule_id: Some(rule_id),
        source_asset_name: buffer_asset_name,
        inactive_reason: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futurefin_engine::{AllocationCap, AllocationKind};

    fn dec(n: i64) -> Decimal {
        Decimal::from(n)
    }

    fn asset(n: u128, liquid: bool, ret: Option<i64>) -> SimAsset {
        SimAsset {
            id: Uuid::from_u128(n),
            value: dec(10_000),
            purchase_price: None,
            is_liquid: liquid,
            expected_annual_return_percent: ret.map(dec),
        }
    }

    fn names(n: &[(u128, &str)]) -> Vec<(Uuid, String)> {
        n.iter()
            .map(|(id, name)| (Uuid::from_u128(*id), (*name).to_string()))
            .collect()
    }

    fn rule(target_index: usize, cap: Option<AllocationCap>) -> AllocationRule {
        AllocationRule {
            target_index,
            kind: if cap.is_some() {
                AllocationKind::Fixed
            } else {
                AllocationKind::Remainder
            },
            amount: cap.is_some().then(|| dec(500)),
            cap,
        }
    }

    /// La cartera de la pauta «cuenta hasta X, resto al fondo»: cuenta corriente líquida al 0 %
    /// (σ = 0) y fondo líquido al 6 % (σ = 15 %).
    fn portfolio() -> (Vec<SimAsset>, Vec<Option<Decimal>>, Vec<(Uuid, String)>) {
        (
            vec![asset(1, true, Some(0)), asset(2, true, Some(6))],
            vec![None, Some(dec(15))],
            names(&[(1, "Cuenta corriente"), (2, "Fondo indexado global")]),
        )
    }

    fn resolve(
        explicit: Option<u32>,
        rules: &[AllocationRule],
        ids: &[Uuid],
    ) -> ResolvedCashBuffer {
        let (assets, vols, id_name) = portfolio();
        resolve_cash_buffer(
            explicit,
            &assets,
            &vols,
            &id_name,
            rules,
            ids,
            dec(3_000),
            dec(2_000),
            dec(2_000),
        )
    }

    /// **Explícito gana** (patrón `pct_source`): un colchón declarado a mano es una elección, y
    /// una elección no se deriva por mucho que exista un tope. Se publica el activo donde se
    /// alojará, pero NI el importe (se indexa cada mes: no hay un escalar honesto) NI la regla.
    #[test]
    fn explicit_months_win_over_the_cap() {
        let ids = vec![Uuid::from_u128(10)];
        let r = resolve(Some(6), &[rule(0, Some(AllocationCap::Amount(dec(6_000))))], &ids);
        assert_eq!(r.source, BUFFER_SOURCE_EXPLICIT);
        assert_eq!(r.spec, Some(CashBufferSpec::Months(6)));
        assert_eq!(r.months_effective, Some(6));
        assert_eq!(r.target_amount, None);
        assert_eq!(r.source_rule_id, None);
        assert_eq!(r.source_asset_name.as_deref(), Some("Cuenta corriente"));
        assert_eq!(r.inactive_reason, None);
    }

    /// **Sin líquido σ = 0 no hay dónde alojarlo.** Un colchón sobre un activo volátil no es un
    /// colchón: es la misma cartera con más impuestos (corrección P4 de la revisión adversarial).
    #[test]
    fn without_a_risk_free_liquid_there_is_no_buffer() {
        let assets = vec![asset(1, true, Some(6)), asset(2, false, Some(1))];
        let vols = vec![Some(dec(15)), None];
        let id_name = names(&[(1, "Fondo"), (2, "Vivienda")]);
        let ids = vec![Uuid::from_u128(10)];
        let r = resolve_cash_buffer(
            None,
            &assets,
            &vols,
            &id_name,
            &[rule(0, Some(AllocationCap::Amount(dec(6_000))))],
            &ids,
            dec(3_000),
            dec(2_000),
            dec(2_000),
        );
        assert_eq!(r.source, BUFFER_SOURCE_NONE);
        assert_eq!(r.spec, None);
        assert_eq!(r.inactive_reason, Some(BUFFER_INACTIVE_NO_SAFE_LIQUID_ASSET));
        // Ni siquiera hay activo que nombrar: la vivienda no es líquida y el fondo tiene σ.
        assert_eq!(r.source_asset_name, None);
    }

    /// **El caso común de hoy**: el líquido σ = 0 es el SUMIDERO sin tope (invariante I1) y no
    /// hay ningún importe que perseguir. No es un error — es «pon un tope a tu cuenta».
    #[test]
    fn the_uncapped_sink_yields_no_capped_rule() {
        let ids = vec![Uuid::from_u128(10)];
        let r = resolve(None, &[rule(0, None)], &ids);
        assert_eq!(r.source, BUFFER_SOURCE_NONE);
        assert_eq!(r.inactive_reason, Some(BUFFER_INACTIVE_NO_CAPPED_RULE));
        // El activo SÍ se nombra: existe, lo que falta es el tope.
        assert_eq!(r.source_asset_name.as_deref(), Some("Cuenta corriente"));
    }

    /// Una regla con tope que apunta a OTRO activo tampoco cuenta: el colchón es el que el motor
    /// va a rellenar, no cualquier tope de la cascada.
    #[test]
    fn a_cap_on_another_asset_is_not_a_buffer() {
        let ids = vec![Uuid::from_u128(10), Uuid::from_u128(11)];
        let r = resolve(
            None,
            &[
                rule(1, Some(AllocationCap::Amount(dec(50_000)))),
                rule(0, None),
            ],
            &ids,
        );
        assert_eq!(r.inactive_reason, Some(BUFFER_INACTIVE_NO_CAPPED_RULE));
    }

    /// **El techo es el MÁXIMO, no la suma.** Dos reglas hacia el mismo activo comparten el valor
    /// vivo contra el que la cascada evalúa `cap_room`, así que el saldo alcanzable lo fija el
    /// techo mayor: sumar (9.000) prometería un colchón que la cascada nunca llena; el mínimo
    /// (3.000) infraprometería.
    #[test]
    fn two_rules_on_the_same_asset_take_the_max_not_the_sum() {
        let ids = vec![Uuid::from_u128(10), Uuid::from_u128(11)];
        let r = resolve(
            None,
            &[
                rule(0, Some(AllocationCap::Amount(dec(3_000)))),
                rule(0, Some(AllocationCap::Amount(dec(6_000)))),
            ],
            &ids,
        );
        assert_eq!(r.source, BUFFER_SOURCE_ALLOCATION_CAP);
        assert_eq!(r.target_amount, Some(dec(6_000)));
        assert_eq!(r.spec, Some(CashBufferSpec::Amount(dec(6_000))));
        assert_eq!(r.source_rule_id, Some(Uuid::from_u128(11)));
    }

    /// **Los tres tipos de tope se resuelven a euros con el MISMO resolutor del motor**
    /// (`resolve_cap_ceiling`, el que `resolve_cap_ceiling_eur` envuelve para las superficies que
    /// solo tienen el par en texto). Sus bases son `gasto + deuda` (2.000) e `ingreso` (3.000) —
    /// distintas de la del equivalente en meses, que es el gasto de JUBILACIÓN.
    #[test]
    fn the_three_cap_kinds_resolve_to_euros() {
        let ids = vec![Uuid::from_u128(10)];
        for (cap, expected) in [
            (AllocationCap::Amount(dec(6_000)), dec(6_000)),
            (AllocationCap::MonthsExpense(dec(4)), dec(8_000)),
            (AllocationCap::IncomeMultiple(dec(2)), dec(6_000)),
        ] {
            let r = resolve(None, &[rule(0, Some(cap))], &ids);
            assert_eq!(r.target_amount, Some(expected), "tope {cap:?}");
            assert_eq!(r.source, BUFFER_SOURCE_ALLOCATION_CAP);
        }
    }

    /// Un techo de 0 € no es un colchón, y decirlo es distinto de decir «no hay regla».
    #[test]
    fn a_zero_ceiling_is_not_a_buffer() {
        let ids = vec![Uuid::from_u128(10)];
        let r = resolve(None, &[rule(0, Some(AllocationCap::Amount(Decimal::ZERO)))], &ids);
        assert_eq!(r.source, BUFFER_SOURCE_NONE);
        assert_eq!(r.spec, None);
        assert_eq!(r.inactive_reason, Some(BUFFER_INACTIVE_CAP_IS_ZERO));
        assert_eq!(r.source_asset_name.as_deref(), Some("Cuenta corriente"));
    }

    /// El equivalente en meses es `floor(techo / gasto de jubilación)` y **solo informativo**: el
    /// motor persigue el IMPORTE. Con gasto de jubilación 0 no hay división honesta → `null`,
    /// nunca 0 (que se leería como «cero meses de colchón» teniendo 6.000 € ahí).
    #[test]
    fn the_months_equivalent_floors_and_disappears_without_expense() {
        let (assets, vols, id_name) = portfolio();
        let ids = vec![Uuid::from_u128(10)];
        let rules = [rule(0, Some(AllocationCap::Amount(dec(6_000))))];
        let with = resolve_cash_buffer(
            None, &assets, &vols, &id_name, &rules, &ids,
            dec(3_000), dec(2_000),
            // 6.000 / 1.750 = 3,42… → 3
            Decimal::from(1_750),
        );
        assert_eq!(with.months_effective, Some(3));
        let without = resolve_cash_buffer(
            None, &assets, &vols, &id_name, &rules, &ids,
            dec(3_000), dec(2_000), Decimal::ZERO,
        );
        assert_eq!(without.months_effective, None);
        assert_eq!(without.target_amount, Some(dec(6_000)));
        assert_eq!(without.source, BUFFER_SOURCE_ALLOCATION_CAP);
    }

    /// **R6: el handler y el motor tienen que elegir el MISMO activo.** Si no, la respuesta
    /// nombraría una cuenta y el sorteo rellenaría otra — un fallo silencioso perfecto. El
    /// `risk_free` de aquí reproduce la política de `monthly_sigma` (privada en el crate
    /// estocástico): ausente, nula, negativa o no finita ⇒ σ = 0.
    #[test]
    fn the_handler_picks_the_same_asset_as_the_engine() {
        let assets = vec![
            // Líquido σ = 0 pero con MÁS rentabilidad: el orden de drenaje no lo elige primero.
            asset(1, true, Some(3)),
            // Líquido, la menor rentabilidad de todas… pero con volatilidad: no vale de colchón.
            asset(2, true, Some(1)),
            // Líquido σ = 0 al 2 %: éste es el colchón.
            asset(3, true, Some(2)),
            // Ilíquido y sin volatilidad declarada: nunca es colchón.
            asset(4, false, Some(0)),
        ];
        let vols = vec![None, Some(dec(15)), Some(Decimal::ZERO), None];
        let id_name = names(&[(1, "Cuenta A"), (2, "Fondo"), (3, "Cuenta B"), (4, "Piso")]);
        let ids = vec![Uuid::from_u128(10)];
        let r = resolve_cash_buffer(
            None,
            &assets,
            &vols,
            &id_name,
            &[rule(2, Some(AllocationCap::Amount(dec(9_000))))],
            &ids,
            dec(3_000),
            dec(2_000),
            dec(2_000),
        );
        assert_eq!(r.source_asset_name.as_deref(), Some("Cuenta B"));
        assert_eq!(r.target_amount, Some(dec(9_000)));

        // Y el índice ES el del motor, calculado con el mismo vector.
        let sim: Vec<SimAssetG<Decimal>> = assets.iter().map(SimAssetG::from).collect();
        let risk_free = risk_free_flags(&vols);
        assert_eq!(risk_free, vec![true, false, true, true]);
        assert_eq!(safe_cash_buffer_index(&sim, &risk_free), Some(2));
    }

    /// Sin activos no hay colchón, y el motivo es el del activo — no el del tope.
    #[test]
    fn an_empty_portfolio_has_no_buffer() {
        let r = resolve_cash_buffer(
            None,
            &[],
            &[],
            &[],
            &[],
            &[],
            Decimal::ZERO,
            Decimal::ZERO,
            Decimal::ZERO,
        );
        assert_eq!(r.inactive_reason, Some(BUFFER_INACTIVE_NO_SAFE_LIQUID_ASSET));
        assert_eq!(r.source, BUFFER_SOURCE_NONE);
    }
}
