//! **El número FIRE clásico** — una LECTURA informativa, no una decisión.
//!
//! # Qué queda aquí, y por qué queda tan poco
//!
//! WP3 de 5.0.0 construyó en este módulo un objetivo «consciente del plan»: dos bases
//! (`Perpetuity` y `BridgeToPension`), una tabla de puente descontada mes a mes hasta la pensión,
//! y el objetivo restando la pensión con fecha desde `P`. **El modelo v2 (decisión M4 del owner,
//! 2026-09-06) lo retiró entero**: la pensión es un FLUJO DE CAJA que el bucle cobra mes a mes, no
//! un descuento sobre un stock, y la fecha de jubilación ya no la decide ningún objetivo — la
//! decide el umbral de éxito sobre miles de caminos (`crates/engine-stochastic`).
//!
//! Lo que sobrevive es **el número FIRE clásico, con UNA sola base**:
//!
//! ```text
//! T(i) = gross_up(12 · E·f(i) − I_persist·12) / (SWR/100) + deuda(i)
//! ```
//!
//! es decir, exactamente [`fire_target_at_month_index`](crate::fire_target_at_month_index), el
//! objetivo de 4.15.0 que el pin dorado hashea. `E·f(i)` es el gasto de jubilación indexado,
//! `I_persist` el ingreso PLANO que persiste (la pensión sin fecha de 4.15.0, que viaja dentro de
//! [`crate::FireNeed`]) y `deuda(i)` el término finito de #142.
//!
//! # Lo que este módulo YA NO hace, y qué se rompía
//!
//! **Ya no resta la pensión CON FECHA.** «25 veces tu gasto» es la lectura que la literatura FIRE
//! publica y la que el usuario reconoce; «25 veces tu gasto menos tu pensión» es otra cosa, y
//! además tenía un **acantilado de construcción**: con una pensión que cubriera el gasto entero,
//! `need_net(i) ≤ 0` desde `P` dejaba `T(i) = deuda(i)` — cero euros en un caso sin deuda. El
//! objetivo caía de 600.000 € a 0 € entre dos meses consecutivos, y con él la lectura que el chart
//! pinta y el tile publica. Hoy eso **no puede pasar**: `T` no mira la pensión con fecha, así que
//! no hay escalón en `P` que dar. Lo verifica
//! `the_classic_fire_number_ignores_the_dated_pension_and_never_drops_to_zero`.
//!
//! **Ya no hay puente ni descuento.** El puente dejó de ser una forma de dimensionar un objetivo y
//! pasó a ser lo que la corrección C2 del panel adversarial dice que es: un TOPE DE TASA INICIAL
//! con fecha límite ([`crate::BridgeCap`], evaluado por el bucle en el mes `R`). Con él se fue la
//! tabla sufijo `O(P)`, la constante `MAX_BRIDGE_MONTHS` y su violación de contrato LATENTE (más
//! allá de los 1.200 meses la degradación podía bajar el objetivo un 77 %), y el error tipado
//! `BridgeDiscountOverflow`, que solo existía porque un descuento muy negativo desbordaba la
//! tabla.
//!
//! # El objetivo NO gobierna la jubilación
//!
//! El cruce `líquido(k−1) ≥ T(k−1)` sigue vivo en el bucle y sigue siendo el default de
//! [`PhasePlan::classic`] — los pines P1–P13 de `pins-4.15.json` lo hashean—, pero es una
//! **lectura** (`liquid_crossing_month_index`) y la fecha que la app publica la resuelve el
//! umbral de éxito. Este módulo publica el número; no decide nada con él.

use rust_decimal::Decimal;

use crate::phases::PhasePlan;
use crate::projection::FireTarget;
use crate::sim::{FireNeedG, FireTargetView, TaxBracketG};
use crate::sim_core::fire_target_at_index_g;

/// El objetivo de jubilación de UN plan, evaluable en `O(1)` en cualquier índice.
///
/// **Desde E4 de 5.0.0 el plan no entra en el número.** El tipo se conserva porque es la cara con
/// la que `apps/api` recorre la serie entera —construirlo una vez y consultarlo `at(i)` mes a
/// mes— y porque presta la serie de `debt_payments_remaining`, que puede tener 841 números y no
/// se copia en ninguna evaluación. Lo que ya no hace es tabular nada: la escala de tramos se
/// convierte una vez (5 elementos) y el resto es la evaluación de 4.15.0.
#[derive(Debug, Clone)]
pub struct PlanFireTarget<'a> {
    target: Option<&'a FireTarget>,
    brackets: Vec<TaxBracketG<Decimal>>,
}

impl<'a> PlanFireTarget<'a> {
    /// El `_plan` ya no se lee: el número FIRE clásico no depende de las fases (E4). El parámetro
    /// se conserva en la firma porque la lectura sigue siendo «el objetivo de ESTE plan» para
    /// quien la consume, y borrarlo obligaría a tocar todos los llamantes sin cambiar un dígito.
    pub fn new(target: Option<&'a FireTarget>, _plan: &PhasePlan) -> Self {
        let brackets = target
            .map(|ft| TaxBracketG::<Decimal>::from_decimal_slice(&ft.tax_brackets))
            .unwrap_or_default();
        Self { target, brackets }
    }

    fn view(&self) -> Option<FireTargetView<'_, Decimal>> {
        decimal_view(self.target, &self.brackets)
    }

    /// El objetivo en el índice **0-based** `month_index`. `None` = no hay objetivo (sin objetivo
    /// declarado, sin SWR positivo o sin necesidad HOY) — **nunca** «cero».
    pub fn at(&self, month_index: u32) -> Option<Decimal> {
        fire_target_at_index_g(self.view(), month_index)
    }
}

/// La vista prestada de un objetivo público, con la escala de tramos ya convertida.
fn decimal_view<'b>(
    target: Option<&'b FireTarget>,
    brackets: &'b [TaxBracketG<Decimal>],
) -> Option<FireTargetView<'b, Decimal>> {
    let ft = target?;
    Some(FireTargetView {
        need: FireNeedG::from(&ft.need),
        swr_pct: ft.swr_pct,
        tax_brackets: brackets,
        taxes_enabled: ft.taxes_enabled,
        taxable_gain_ratio: ft.taxable_gain_ratio,
        annual_inflation_percent: ft.annual_inflation_percent,
        debt_payments_remaining: &ft.debt_payments_remaining,
    })
}

/// El objetivo de un PLAN en el índice 0-based `month_index` — la versión de un solo disparo de
/// [`PlanFireTarget`].
///
/// **Contrato de bit-identidad**: devuelve EXACTAMENTE lo que devuelve
/// [`fire_target_at_month_index`](crate::fire_target_at_month_index), porque ejecuta la misma
/// función del núcleo. Desde E4 eso vale para CUALQUIER plan, no solo para los que no tienen
/// pensión con fecha. Los dos pines dorados dependen de ello.
pub fn fire_target_at_month_index_with_plan(
    target: Option<&FireTarget>,
    plan: &PhasePlan,
    month_index: u32,
) -> Option<Decimal> {
    PlanFireTarget::new(target, plan).at(month_index)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::phases::PensionSchedule;
    use crate::projection::{fire_target_at_month_index, FireNeed};
    use crate::tax::TaxBracket;

    fn dec(s: &str) -> Decimal {
        s.parse().expect("literal decimal válido")
    }

    /// Objetivo sin impuestos, sin inflación y sin deuda: la aritmética queda a la vista.
    fn ft(need: FireNeed, swr: Decimal) -> FireTarget {
        FireTarget {
            need,
            swr_pct: swr,
            tax_brackets: Vec::new(),
            taxes_enabled: false,
            taxable_gain_ratio: Decimal::ONE,
            annual_inflation_percent: Decimal::ZERO,
            debt_payments_remaining: Vec::new(),
        }
    }

    fn expense_need(expense: u32, persistent: u32) -> FireNeed {
        FireNeed::ExpenseMinusPension {
            expense_monthly: Decimal::from(expense),
            pension_monthly: Decimal::from(persistent),
        }
    }

    fn plan_with_pension(start_index: u32, monthly: u32, indexed: bool) -> PhasePlan {
        let mut p = PhasePlan::classic(Decimal::ZERO, Decimal::from(2_000u32));
        p.pension = Some(PensionSchedule {
            start_index,
            monthly_today: Decimal::from(monthly),
            indexed,
            fraction_while_partial: Decimal::ZERO,
        });
        p
    }

    /// **El objetivo del plan NO se mueve NI UN DÍGITO respecto al de 4.15.0.** Es el contrato del
    /// que cuelgan los dos pines dorados; se comprueba sobre una configuración con impuestos,
    /// inflación y deuda —donde hay dígitos de sobra que mover— y en toda la rejilla.
    #[test]
    fn the_plan_aware_target_is_the_4_15_one() {
        let mut target = ft(expense_need(2_000, 300), dec("3.5"));
        target.annual_inflation_percent = dec("2.5");
        target.taxes_enabled = true;
        target.tax_brackets = vec![
            TaxBracket {
                up_to: Some(Decimal::from(6_000u32)),
                pct: Decimal::from(19u32),
            },
            TaxBracket {
                up_to: None,
                pct: Decimal::from(21u32),
            },
        ];
        target.debt_payments_remaining = (0..40)
            .map(|m| Decimal::from(40_000u32 - m * 1_000))
            .collect();

        // Y con pensión CON FECHA también: desde E4 el plan no entra en el número.
        for plan in [
            PhasePlan::classic(Decimal::ZERO, Decimal::from(2_000u32)),
            plan_with_pension(24, 1_500, true),
        ] {
            for i in 0..60u32 {
                assert_eq!(
                    fire_target_at_month_index_with_plan(Some(&target), &plan, i),
                    fire_target_at_month_index(Some(&target), i),
                    "índice {i}"
                );
            }
        }
    }

    /// **El número FIRE clásico ignora la pensión con fecha y NUNCA cae a cero** (E4, decisión M4).
    ///
    /// El caso es el del acantilado: una pensión de 2.060 €/mes contra un gasto de 2.000 €/mes
    /// cubre el **103 %** del gasto desde el índice `P = 240`. Con la base retirada, `need_net`
    /// era ≤ 0 desde `P` y el objetivo se desplomaba a `deuda(P)` = **0 €** — un escalón de
    /// 600.000 € a 0 € entre el mes 239 y el 240.
    ///
    /// Predicho a mano (gasto 2.000 €/mes, sin ingreso persistente, SWR 4 %, sin impuestos, sin
    /// inflación, sin deuda): `T(i) = 24.000/0,04 = 600.000 €` **en todos los índices**, antes y
    /// después de `P`, con pensión y sin ella.
    #[test]
    fn the_classic_fire_number_ignores_the_dated_pension_and_never_drops_to_zero() {
        let target = ft(expense_need(2_000, 0), Decimal::from(4u32));
        let covering = plan_with_pension(240, 2_060, false);
        let at = |i| fire_target_at_month_index_with_plan(Some(&target), &covering, i);

        // La pensión cubre el 103 % del gasto (2.060/2.000) y el objetivo no se entera.
        for i in [0u32, 239, 240, 241, 600] {
            assert_eq!(
                at(i),
                Some(Decimal::from(600_000u32)),
                "índice {i}: la perpetuidad del gasto ÍNTEGRO, sin restar la pensión"
            );
        }
        assert_eq!(
            at(239).unwrap() - at(240).unwrap(),
            Decimal::ZERO,
            "no hay escalón en P: ese acantilado era el bug"
        );

        // Y es exactamente el mismo número que sin pensión: el plan no entra.
        let no_pension = PhasePlan::classic(Decimal::ZERO, Decimal::from(2_000u32));
        for i in [0u32, 239, 240, 241, 600] {
            assert_eq!(
                at(i),
                fire_target_at_month_index_with_plan(Some(&target), &no_pension, i),
                "índice {i}"
            );
        }
    }

    /// Con inflación el objetivo CRECE monótonamente (sin deuda): el acantilado no vuelve por la
    /// puerta de atrás de la indexación, y la pensión indexada tampoco lo abre.
    #[test]
    fn with_inflation_the_number_only_grows_and_the_pension_never_bends_it() {
        let mut target = ft(expense_need(2_000, 0), Decimal::from(4u32));
        target.annual_inflation_percent = Decimal::from(2u32);
        let plan = plan_with_pension(120, 3_000, true);
        let evaluator = PlanFireTarget::new(Some(&target), &plan);
        let mut prev = evaluator.at(0).expect("hay objetivo en el índice 0");
        assert_eq!(prev, Decimal::from(600_000u32));
        for i in 1..240u32 {
            let now = evaluator.at(i).expect("hay objetivo");
            assert!(now >= prev, "índice {i}: {now} < {prev}");
            prev = now;
        }
    }

    /// El evaluador precomputado y la función de un disparo **son la misma función**.
    #[test]
    fn the_precomputed_evaluator_matches_the_one_shot_function() {
        let mut target = ft(expense_need(2_100, 250), dec("3.5"));
        target.annual_inflation_percent = dec("2.2");
        let plan = plan_with_pension(48, 900, true);
        let evaluator = PlanFireTarget::new(Some(&target), &plan);
        for i in 0..72u32 {
            assert_eq!(
                evaluator.at(i),
                fire_target_at_month_index_with_plan(Some(&target), &plan, i),
                "índice {i}"
            );
        }
    }

    /// La puerta de `i = 0` sigue mandando: sin necesidad HOY no hay objetivo en ningún mes, y eso
    /// es `None` — no un 0 que dispararía un cruce falso.
    #[test]
    fn no_need_today_still_means_no_target_at_all() {
        let target = ft(expense_need(1_000, 1_500), Decimal::from(4u32));
        let plan = plan_with_pension(24, 500, true);
        for i in [0u32, 23, 24, 60] {
            assert_eq!(
                fire_target_at_month_index_with_plan(Some(&target), &plan, i),
                None,
                "índice {i}"
            );
        }
    }
}
