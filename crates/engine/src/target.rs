//! **Objetivo de jubilación consciente del PLAN** (WP3 de 5.0.0, §B.3 del plan de la issue #207).
//!
//! Hasta 4.15.0 el objetivo era una función de dos argumentos —`fire_target_at_month_index(ft, i)`—
//! porque solo había una forma de dimensionarlo: capitalizar la necesidad de HOY a perpetuidad al
//! SWR. Con la **pensión con fecha** (P2) eso deja de bastar: la necesidad ya no es la misma antes
//! y después de `P`, y hay dos maneras legítimas de leerla, ninguna de las cuales se asume (D15).
//!
//! # Las tres unidades, y por qué se declaran
//!
//! El hallazgo **B1** de la revisión adversarial fue exactamente este: mezclar euros/mes con
//! euros/año dentro de la misma suma hace que el puente salga 12 veces mal sin que nada falle. Así
//! que aquí **cada término declara su unidad y nunca se anualiza dos veces**:
//!
//! | símbolo | unidad | definición |
//! |---|---|---|
//! | `f(i)` | — | `inflation_factor_at_month_index(ft.annual_inflation_percent, i)` |
//! | `E·f(i)` | €/mes | gasto del mes `i` (ver [`PlanFireTarget::expense_monthly_at`]) |
//! | `I_persist` | €/mes | ingreso PLANO que persiste tras jubilarse (la pensión sin fecha de 4.15.0) |
//! | `P_m(i)` | €/mes | pensión CON fecha en el índice `i` (0 antes de `P`) |
//! | `need_full_m(i)` | €/mes | `max(0, E·f(i) − I_persist)` |
//! | `need_net_m(i)` | €/mes | `max(0, E·f(i) − I_persist − P_m(i))` |
//! | `T(i)` | € | el objetivo — un STOCK, no un flujo |
//!
//! **La rejilla es 0-based** (la de `fire_target_at_month_index`): el bucle evalúa su mes `k`
//! contra el índice `i = k−1`, y `P = pension.start_index` vive en esa misma rejilla.
//!
//! # Las dos bases
//!
//! - [`TargetBasis::Perpetuity`] — `T(i) = gross_up(12·need(i))/SWR + deuda(i)`, con
//!   `need = need_full_m` mientras `i < P` (la pensión todavía no existe: no se cuenta con ella,
//!   R6) y `need_net_m` desde `P`. Si `need_net_m(i) ≤ 0` la pensión cubre el gasto entero y
//!   `T(i) = deuda(i)` — **nunca `None`**: el hallazgo B3 de la revisión fue que un objetivo
//!   ausente ahí significaba «no se jubila jamás» cuando la verdad es «se jubila ya».
//! - [`TargetBasis::BridgeToPension`] — para `i < P`, el valor presente de los meses que faltan
//!   más la perpetuidad de lo que la pensión no cubra:
//!
//!   ```text
//!   T(i) = Σ_{m=i}^{P−1} gross_up_monthly(need_full_m(m)) · (1+d)^{−(m−i)/12}
//!        + [gross_up(12·need_net_m(P)) / SWR] · (1+d)^{−(P−i)/12}
//!        + deuda(i)
//!   ```
//!
//!   Desde `P` coincide, término a término, con la perpetuidad neta. **Los dos escenarios de D15
//!   caen solos**: si la pensión cubre el 100 % del gasto el término perpetuo es 0 y el objetivo
//!   es solo el puente; si cubre una parte, queda la perpetuidad sobre el resto. Lo decide el
//!   importe declarado frente al gasto, no un supuesto.
//!
//! # Cómo se computa el puente, y por qué no es la suma llana
//!
//! La suma directa es `O(P−i)` por evaluación y el bucle la pide una vez por mes: `O(P²)`, con un
//! `gross_up` y una potencia por término. Medido a 840 meses son cientos de miles de gross-ups —
//! un orden de magnitud por encima del coste de la proyección entera.
//!
//! La identidad que lo arregla es exacta en los reales y usa **el mismo factor** que todo lo demás
//! del motor: con `q(j) = inflation_factor_at_month_index(d, j) = (1+d)^{j/12}`,
//!
//! ```text
//!   (1+d)^{−(m−i)/12} = q(i)/q(m)      ⇒     Σ_m G(m)·q(i)/q(m) = q(i) · Σ_m G(m)/q(m)
//! ```
//!
//! y `Σ_{m=i}^{P−1} G(m)/q(m)` es una **suma sufijo**: `O(P)` UNA vez por simulación, `O(1)` por
//! evaluación. Es la forma implementada y por tanto **la definición**: en `i = 0` (donde `q(0) = 1`
//! exacto) coincide término a término con la suma directa, y para `i > 0` difiere de ella en el
//! redondeo de `powd`, no en el valor.
//!
//! Nunca por producto acumulado (`q(j+1) = q(j)·q(1)`): la casa ya tiene fichado que `powd` enruta
//! los exponentes enteros por `checked_powu` y un producto acumulado los desviaría a `exp`/`ln`.
//!
//! # Bit-identidad con 4.15.0
//!
//! **Sin pensión con fecha, [`PlanTargetG::at`] LLAMA al objetivo de 4.15.0**
//! ([`crate::sim_core::fire_target_at_index_g`]) en vez de reproducir su fórmula. No es
//! elegancia: es la única forma de que el pin dorado no pueda moverse por un paréntesis. La misma
//! disciplina rige la rama `i < P` de la perpetuidad, que es literalmente el objetivo de 4.15.0
//! evaluado en `i`.
//!
//! # 5.0.0 WP5.5 — dos capas
//!
//! El evaluador de verdad es [`PlanTargetG`], genérico sobre [`MoneyOps`] y **sin lifetime**: el
//! objetivo se le pasa como [`FireTargetView`] en cada llamada, que es `Copy` y presta los dos
//! vectores en vez de copiarlos. [`PlanFireTarget`] es su cara pública en `Decimal`, con la misma
//! firma que WP3 publicó.

use rust_decimal::Decimal;

use crate::money::MoneyOps;
use crate::phases::{PhasePlan, TargetBasis};
use crate::projection::FireTarget;
use crate::sim::{
    FireNeedG, FireTargetView, PensionScheduleG, PhasePlanG, TaxBracketG,
};
use crate::sim_core::{debt_term_at_index_g, fire_target_at_index_g, inflation_factor_at_index_g};
use crate::tax::gross_up_net_annual_fire_g;

/// Tope del puente, en meses: 100 años. Una pensión declarada MÁS ALLÁ de este índice no cae
/// dentro de ningún horizonte que este motor simule (el máximo publicado son 840 meses), así que
/// dimensionar un puente hasta ella sería tabular cien mil `gross_up` para nada.
///
/// La degradación es la PRUDENTE y está declarada: el objetivo pasa a ser la perpetuidad sobre la
/// necesidad ÍNTEGRA — o sea, MÁS grande, nunca menor. Truncar el puente iría en la dirección
/// contraria (objetivo pequeño ⇒ cruce temprano ⇒ jubilación falsa), y esa es la clase de número
/// que esta casa no publica.
pub const MAX_BRIDGE_MONTHS: u32 = 1_200;

/// Tabla del puente, calculada UNA vez por simulación.
#[derive(Debug, Clone)]
struct BridgeTable<M> {
    /// `P`, el índice 0-based en que empieza la pensión.
    p: u32,
    /// `q(j) = (1+d)^{j/12}` para `j ∈ [0, P]`.
    disc: Vec<M>,
    /// `T(i) = Σ_{m=i}^{P−1} G(m)/q(m)` para `i ∈ [0, P]` (`T(P) = 0`), con
    /// `G(m) = gross_up(12·need_full_m(m))/12` en €/mes.
    suffix: Vec<M>,
    /// `gross_up(12·need_net_m(P))/SWR`, en €. **0 exacto** cuando la pensión cubre el gasto
    /// entero — el escenario «la pensión llega para todo» de D15.
    perp_at_p: M,
}

/// El objetivo de jubilación de UN plan, listo para evaluarse en cualquier índice en `O(1)`.
///
/// Se construye una vez por simulación y el bucle lo consulta mes a mes. Es una FUNCIÓN pura de
/// sus entradas: no guarda estado mutable ni depende del orden de las consultas.
///
/// **El objetivo viaja como argumento, no como campo**, y a propósito: así el tipo no tiene
/// lifetime y la cara pública en `Decimal` puede poseer la escala de tramos convertida sin
/// convertirse en una estructura autorreferencial. El contrato es que se le pase SIEMPRE el mismo
/// objetivo con el que se construyó — la tabla del puente se tabuló con él.
#[derive(Debug, Clone)]
pub(crate) struct PlanTargetG<M: MoneyOps> {
    pension: Option<PensionScheduleG<M>>,
    basis: TargetBasis,
    /// `Some` solo con base puente, pensión con fecha `P ∈ [1, MAX_BRIDGE_MONTHS]` y un objetivo
    /// que pasa la puerta de `i = 0`. En cualquier otro caso el puente degrada a perpetuidad.
    bridge: Option<BridgeTable<M>>,
}

impl<M: MoneyOps> PlanTargetG<M> {
    /// Construye el evaluador. Coste: `O(1)` sin pensión con fecha o sin base puente; `O(P)`
    /// (una potencia y un gross-up por mes hasta la pensión) con el puente activo.
    pub(crate) fn new(target: Option<FireTargetView<'_, M>>, plan: &PhasePlanG<M>) -> Self {
        let mut out = Self {
            pension: plan.pension,
            basis: plan.target_basis,
            bridge: None,
        };
        let (Some(ft), Some(pen)) = (target, plan.pension) else {
            return out;
        };
        if out.basis != TargetBasis::BridgeToPension {
            return out;
        }
        // La MISMA puerta que `fire_target_base_at_index_g`: sin SWR positivo o sin necesidad HOY
        // no hay objetivo en ningún mes, y tabular un puente para un objetivo que no existe sería
        // trabajo para tirar.
        if ft.swr_pct <= M::zero() || ft.need.annual_net_at(M::one()) <= M::zero() {
            return out;
        }
        let p = pen.start_index;
        if p == 0 || p > MAX_BRIDGE_MONTHS {
            // `p == 0`: la pensión ya está cobrándose, no hay puente que cruzar. `p > MAX`:
            // degradación declarada (ver la constante).
            return out;
        }
        out.bridge = Some(build_bridge_table(
            ft,
            pen,
            plan.bridge_discount_annual_pct,
            p,
        ));
        out
    }

    /// El objetivo en el índice **0-based** `month_index`. `None` = no hay objetivo (sin
    /// objetivo, sin SWR positivo o sin necesidad hoy) — **nunca** «cero».
    pub(crate) fn at(&self, target: Option<FireTargetView<'_, M>>, month_index: u32) -> Option<M> {
        let ft = target?;
        // Sin pensión con fecha, el objetivo es EL DE 4.15.0, llamado tal cual: bit-identidad por
        // construcción, no por revisión.
        let Some(pen) = self.pension else {
            return fire_target_at_index_g(Some(ft), month_index);
        };
        if ft.swr_pct <= M::zero() || ft.need.annual_net_at(M::one()) <= M::zero() {
            return None;
        }

        if month_index < pen.start_index {
            return match &self.bridge {
                // PUENTE: `q(i)·T(i)` + perpetuidad descontada + deuda.
                Some(b) => {
                    let i = month_index as usize;
                    let q_i = b.disc[i];
                    let bridge_sum = q_i * b.suffix[i];
                    let perp = if b.perp_at_p.is_zero() {
                        M::zero()
                    } else {
                        let q_p = b.disc[b.p as usize];
                        // `q(i)/q(P)` = `(1+d)^{−(P−i)/12}`. Un `q(P)` degenerado se lee como
                        // «sin descuento» en vez de panicar (el motor es una función pura).
                        let ratio = if q_p > M::zero() {
                            q_i.checked_div(q_p).unwrap_or(M::one())
                        } else {
                            M::one()
                        };
                        b.perp_at_p * ratio
                    };
                    Some(
                        bridge_sum
                            + perp
                            + debt_term_at_index_g(ft.debt_payments_remaining, month_index),
                    )
                }
                // PERPETUIDAD antes de `P` (y puente degradado): la pensión aún no existe, así
                // que la necesidad es la ÍNTEGRA — que es exactamente el objetivo de 4.15.0.
                None => fire_target_at_index_g(Some(ft), month_index),
            };
        }

        // `i ≥ P`: perpetuidad sobre la necesidad NETA de pensión, en las dos bases.
        let debt = debt_term_at_index_g(ft.debt_payments_remaining, month_index);
        let annual_net = annual_net_need_at(ft, pen, month_index);
        if annual_net <= M::zero() {
            // La pensión cubre el gasto entero: no hace falta capital para vivir, solo para la
            // deuda que quede. Cruce inmediato, y jamás `None` (B3 de la revisión).
            return Some(debt);
        }
        Some(perpetuity_from_annual_net(ft, annual_net) + debt)
    }

    /// `12·need_full_m(i)` — la necesidad ÍNTEGRA del índice `i` en €/AÑO, **antes** de restar la
    /// pensión con fecha. `None` sin objetivo.
    pub(crate) fn need_full_annual_at(
        &self,
        target: Option<FireTargetView<'_, M>>,
        month_index: u32,
    ) -> Option<M> {
        let ft = target?;
        let f = inflation_factor_at_index_g(ft.annual_inflation_percent, month_index);
        Some(ft.need.annual_net_at(f))
    }

    /// `P_m(i)` en €/mes, con la inflación DEL OBJETIVO. `ZERO` sin pensión con fecha o antes de
    /// `P` (cero euros, no «no aplica»: la pensión existe y vale cero ese mes).
    pub(crate) fn pension_monthly_at(
        &self,
        target: Option<FireTargetView<'_, M>>,
        month_index: u32,
    ) -> M {
        let (Some(ft), Some(pen)) = (target, self.pension) else {
            return M::zero();
        };
        let f = inflation_factor_at_index_g(ft.annual_inflation_percent, month_index);
        pen.monthly_at(month_index, f)
    }

    /// `E·f(i)` en €/mes: el GASTO del índice `i`, sin restarle nada.
    ///
    /// No es la necesidad: `FireNeed::ExpenseMinusPension` le resta después el ingreso que
    /// persiste. Se publica aparte porque `pension_coverage_ratio` mide la pensión contra el
    /// GASTO («qué parte de lo que gasto me la paga la pensión»), no contra el hueco.
    pub(crate) fn expense_monthly_at(
        &self,
        target: Option<FireTargetView<'_, M>>,
        month_index: u32,
    ) -> Option<M> {
        let ft = target?;
        let f = inflation_factor_at_index_g(ft.annual_inflation_percent, month_index);
        Some(match ft.need {
            FireNeedG::Indexed { annual_net_today } => {
                annual_net_today * f / M::from_u32(12)
            }
            FireNeedG::ExpenseMinusPension {
                expense_monthly, ..
            } => expense_monthly * f,
        })
    }

    /// `P_m(P)/(E·f(P))` — qué FRACCIÓN del gasto cubre la pensión el mes en que empieza (D15:
    /// el modelo la lee, no la supone). `None` sin pensión con fecha, sin objetivo, o con un
    /// gasto no positivo en `P` (no hay base contra la que medir — jamás un 0 inventado).
    pub(crate) fn pension_coverage_ratio(&self, target: Option<FireTargetView<'_, M>>) -> Option<M> {
        let pen = self.pension?;
        let expense = self.expense_monthly_at(target, pen.start_index)?;
        if expense <= M::zero() {
            return None;
        }
        self.pension_monthly_at(target, pen.start_index)
            .checked_div(expense)
    }

    /// `gross_up(12·gap_m)/SWR` con el hueco que la media jornada deja abierto (§B.3):
    ///
    /// ```text
    /// gap_m(X) = max(0, E_basis·f(X−1) − income_partial − P_m(X−1)·fraction)
    /// ```
    ///
    /// `X` es el mes 1-based en que arranca la fase parcial, así que su índice es `X−1` — la
    /// misma asimetría que el resto del bucle. Es una lectura INFORMATIVA («cuánto capital haría
    /// falta para sostener este hueco a perpetuidad»), no un objetivo que dispare nada.
    ///
    /// `Some(0)` = el hueco es cero (la media jornada se paga sola): cero euros, no «no aplica».
    /// `None` = no hay fase parcial, no hay objetivo, o el SWR no es positivo.
    pub(crate) fn partial_gap_target(
        &self,
        target: Option<FireTargetView<'_, M>>,
        plan: &PhasePlanG<M>,
        expense_regular: M,
    ) -> Option<M> {
        let ft = target?;
        if ft.swr_pct <= M::zero() {
            return None;
        }
        let partial = plan.partial?;
        let basis_monthly = plan.partial_expense_basis_monthly(expense_regular)?;
        // `X.max(1) − 1`: un `start_month` de 0 arranca en el mes 1 del bucle, cuyo índice es 0.
        let i = partial.start_month.max(1) - 1;
        let f = inflation_factor_at_index_g(ft.annual_inflation_percent, i);
        let pension_share = match self.pension {
            Some(pen) => pen.monthly_at(i, f) * pen.partial_fraction(),
            None => M::zero(),
        };
        let gap = (basis_monthly * f - partial.income_monthly - pension_share).max(M::zero());
        if gap <= M::zero() {
            return Some(M::zero());
        }
        Some(perpetuity_from_annual_net(ft, gap * M::from_u32(12)))
    }
}

/// `max(0, 12·(E·f(i) − I_persist) − 12·P_m(i))`, en €/AÑO. Privada porque el signo de esta
/// magnitud ya viaja en `at`.
fn annual_net_need_at<M: MoneyOps>(
    ft: FireTargetView<'_, M>,
    pen: PensionScheduleG<M>,
    i: u32,
) -> M {
    let f = inflation_factor_at_index_g(ft.annual_inflation_percent, i);
    let annual_full = ft.need.annual_net_at(f);
    (annual_full - pen.monthly_at(i, f) * M::from_u32(12)).max(M::zero())
}

/// `gross_up(net_annual)/(SWR/100)` — la perpetuidad, con la MISMA escala y el MISMO switch que
/// el drenaje (#140). El llamante garantiza `swr_pct > 0` y `net_annual > 0`.
fn perpetuity_from_annual_net<M: MoneyOps>(ft: FireTargetView<'_, M>, net_annual: M) -> M {
    let gross = gross_up_net_annual_fire_g(
        net_annual,
        ft.tax_brackets,
        ft.taxes_enabled,
        ft.taxable_gain_ratio,
    );
    gross / (ft.swr_pct / M::from_u32(100))
}

fn build_bridge_table<M: MoneyOps>(
    ft: FireTargetView<'_, M>,
    pen: PensionScheduleG<M>,
    bridge_discount_annual_pct: M,
    p: u32,
) -> BridgeTable<M> {
    // Un descuento ≤ −100 % dejaría la base `1 + d/100` en cero o negativa y `powd` sin raíz real.
    // Se lee como «sin descuento», que es la lectura conservadora (el puente sale MÁS caro).
    let d_pct = if bridge_discount_annual_pct <= -M::from_u32(100) {
        M::zero()
    } else {
        bridge_discount_annual_pct
    };

    let n = p as usize;
    let mut disc = Vec::with_capacity(n + 1);
    for j in 0..=p {
        disc.push(inflation_factor_at_index_g(d_pct, j));
    }

    // Suma SUFIJO, de `P−1` hacia 0: `T(i) = G(i)/q(i) + T(i+1)`, `T(P) = 0`.
    let mut suffix = vec![M::zero(); n + 1];
    for m in (0..n).rev() {
        let f = inflation_factor_at_index_g(ft.annual_inflation_percent, m as u32);
        // `G(m) = gross_up_monthly(need_full_m(m))` escrito SIN el viaje de ida y vuelta por 12:
        // `gross_up_monthly(x)` es por definición `gross_up_annual(12x)/12`, y aquí el anual ya lo
        // tenemos. Dividir por 12 lo que acabamos de multiplicar por 12 no cambiaría el valor pero
        // sí los dígitos, y este término se suma cientos de veces.
        let annual_full = ft.need.annual_net_at(f);
        let gross_monthly = gross_up_net_annual_fire_g(
            annual_full,
            ft.tax_brackets,
            ft.taxes_enabled,
            ft.taxable_gain_ratio,
        ) / M::from_u32(12);
        // `q(m) > 0` para cualquier `d > −100`; el fallback «sin descuento» solo protege de un
        // valor degenerado, y NO puede quedarse corto (dividir por 1 da el término entero).
        let discounted = gross_monthly.checked_div(disc[m]).unwrap_or(gross_monthly);
        suffix[m] = discounted + suffix[m + 1];
    }

    // Perpetuidad en `P` sobre lo que la pensión NO cubre. `0` exacto si la cubre entera.
    let f_p = inflation_factor_at_index_g(ft.annual_inflation_percent, p);
    let annual_net_p =
        (ft.need.annual_net_at(f_p) - pen.monthly_at(p, f_p) * M::from_u32(12)).max(M::zero());
    let perp_at_p = if annual_net_p <= M::zero() {
        M::zero()
    } else {
        perpetuity_from_annual_net(ft, annual_net_p)
    };

    BridgeTable {
        p,
        disc,
        suffix,
        perp_at_p,
    }
}

/// El objetivo del plan en el índice `i`, de un solo disparo y **en el tipo del núcleo** — lo que
/// consume `first_month_allocation`.
pub(crate) fn plan_target_at<M: MoneyOps>(
    target: Option<FireTargetView<'_, M>>,
    plan: &PhasePlanG<M>,
    month_index: u32,
) -> Option<M> {
    PlanTargetG::new(target, plan).at(target, month_index)
}

// =============================================================================================
// Cara pública, en `Decimal`
// =============================================================================================

/// El objetivo de jubilación de UN plan, evaluable en `O(1)` en cualquier índice.
///
/// Cara pública de [`PlanTargetG`]: posee la escala de tramos ya convertida al tipo del núcleo
/// (5 elementos, una vez) y presta el resto — en particular la serie de `debt_payments_remaining`,
/// que puede tener 841 números y **no se copia en ninguna evaluación**.
#[derive(Debug, Clone)]
pub struct PlanFireTarget<'a> {
    target: Option<&'a FireTarget>,
    brackets: Vec<TaxBracketG<Decimal>>,
    plan: PhasePlanG<Decimal>,
    inner: PlanTargetG<Decimal>,
}

impl<'a> PlanFireTarget<'a> {
    pub fn new(target: Option<&'a FireTarget>, plan: &PhasePlan) -> Self {
        let brackets = target
            .map(|ft| TaxBracketG::<Decimal>::from_decimal_slice(&ft.tax_brackets))
            .unwrap_or_default();
        let plan_g = PhasePlanG::<Decimal>::from(plan);
        let inner = PlanTargetG::new(decimal_view(target, &brackets), &plan_g);
        Self {
            target,
            brackets,
            plan: plan_g,
            inner,
        }
    }

    fn view(&self) -> Option<FireTargetView<'_, Decimal>> {
        decimal_view(self.target, &self.brackets)
    }

    /// El objetivo en el índice **0-based** `month_index`. `None` = no hay objetivo, nunca «cero».
    pub fn at(&self, month_index: u32) -> Option<Decimal> {
        self.inner.at(self.view(), month_index)
    }

    /// `12·need_full_m(i)`, €/año.
    pub fn need_full_annual_at(&self, month_index: u32) -> Option<Decimal> {
        self.inner.need_full_annual_at(self.view(), month_index)
    }

    /// `P_m(i)`, €/mes.
    pub fn pension_monthly_at(&self, month_index: u32) -> Decimal {
        self.inner.pension_monthly_at(self.view(), month_index)
    }

    /// `E·f(i)`, €/mes (sin restar nada).
    pub fn expense_monthly_at(&self, month_index: u32) -> Option<Decimal> {
        self.inner.expense_monthly_at(self.view(), month_index)
    }

    /// `P_m(P)/(E·f(P))`, FRACCIÓN.
    pub fn pension_coverage_ratio(&self) -> Option<Decimal> {
        self.inner.pension_coverage_ratio(self.view())
    }

    /// El capital que sostendría a perpetuidad el hueco de la media jornada. El `plan` que se
    /// pasa aquí es el mismo con el que se construyó el evaluador.
    pub fn partial_gap_target(
        &self,
        plan: &PhasePlan,
        expense_regular: Decimal,
    ) -> Option<Decimal> {
        let _ = &self.plan;
        let plan_g = PhasePlanG::<Decimal>::from(plan);
        self.inner
            .partial_gap_target(self.view(), &plan_g, expense_regular)
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
/// **Contrato de bit-identidad**: con `plan.pension == None` devuelve EXACTAMENTE lo que devuelve
/// [`fire_target_at_month_index`](crate::fire_target_at_month_index), porque ejecuta la misma
/// función del núcleo. Los dos pines dorados dependen de ello.
///
/// Coste: `O(P)` por llamada con el puente activo (reconstruye la tabla). Para recorrer una serie
/// entera —el bucle, el chart— construye un [`PlanFireTarget`] UNA vez y consúltalo con
/// [`PlanFireTarget::at`], que es `O(1)`.
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
    use crate::phases::{ExpenseBasis, PartialPhase, PensionSchedule};
    use crate::projection::{fire_target_at_month_index, inflation_factor_at_month_index, FireNeed};
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

    fn plan_with_pension(
        start_index: u32,
        monthly: u32,
        indexed: bool,
        basis: TargetBasis,
        discount_pct: Decimal,
    ) -> PhasePlan {
        let mut p = PhasePlan::classic(Decimal::ZERO, Decimal::from(2_000u32));
        p.pension = Some(PensionSchedule {
            start_index,
            monthly_today: Decimal::from(monthly),
            indexed,
            fraction_while_partial: Decimal::ZERO,
        });
        p.target_basis = basis;
        p.bridge_discount_annual_pct = discount_pct;
        p
    }

    /// **Sin pensión con fecha el objetivo no se mueve NI UN DÍGITO.** Es el contrato del que
    /// cuelgan los dos pines dorados; se comprueba sobre una configuración con impuestos,
    /// inflación y deuda —donde hay dígitos de sobra que mover— y en toda la rejilla.
    #[test]
    fn without_a_dated_pension_the_plan_aware_target_is_the_4_15_one() {
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
        target.debt_payments_remaining = (0..40).map(|m| Decimal::from(40_000u32 - m * 1_000)).collect();

        let plan = PhasePlan::classic(Decimal::ZERO, Decimal::from(2_000u32));
        for i in 0..60u32 {
            assert_eq!(
                fire_target_at_month_index_with_plan(Some(&target), &plan, i),
                fire_target_at_month_index(Some(&target), i),
                "índice {i}"
            );
        }
    }

    /// **Perpetuidad con pensión: el escalón en `P` baja EXACTAMENTE la perpetuidad de la pensión,
    /// y nunca a `None`.**
    ///
    /// Predicho a mano (gasto 2.000 €/mes, sin ingreso persistente, SWR 4 %, sin impuestos, sin
    /// inflación, sin deuda; pensión plana de 1.200 €/mes desde el índice 240):
    ///
    /// - `i < 240`: `need_full = 2.000` ⇒ `24.000/0,04` = **600.000 €**.
    /// - `i ≥ 240`: `need_net = 800` ⇒ `9.600/0,04` = **240.000 €**.
    /// - El salto es `360.000 €` = `1.200·12/0,04`, la perpetuidad de la pensión. Ni un euro más.
    #[test]
    fn perpetuity_with_a_pension_steps_down_at_p_by_exactly_the_pensions_perpetuity() {
        let target = ft(expense_need(2_000, 0), Decimal::from(4u32));
        let plan = plan_with_pension(240, 1_200, false, TargetBasis::Perpetuity, Decimal::ZERO);
        let at = |i| fire_target_at_month_index_with_plan(Some(&target), &plan, i);

        assert_eq!(at(0), Some(Decimal::from(600_000u32)));
        assert_eq!(at(239), Some(Decimal::from(600_000u32)));
        assert_eq!(at(240), Some(Decimal::from(240_000u32)));
        assert_eq!(at(600), Some(Decimal::from(240_000u32)));
        assert_eq!(
            at(239).unwrap() - at(240).unwrap(),
            Decimal::from(360_000u32),
            "el escalón es la perpetuidad de la pensión: 1.200·12/0,04"
        );
    }

    /// **La pensión cubre el gasto entero ⇒ `target = deuda`, jamás `None`** (hallazgo B3).
    ///
    /// Un `None` ahí significaría «no se jubila nunca» cuando la verdad es «se jubila ya»: la
    /// necesidad neta es cero y solo hace falta capital para la deuda que quede.
    #[test]
    fn a_pension_that_covers_everything_leaves_only_the_debt_term() {
        let mut target = ft(expense_need(2_000, 0), Decimal::from(4u32));
        target.debt_payments_remaining = vec![Decimal::from(12_000u32); 300];
        let plan = plan_with_pension(240, 2_500, false, TargetBasis::Perpetuity, Decimal::ZERO);
        let at = |i| fire_target_at_month_index_with_plan(Some(&target), &plan, i);

        assert_eq!(
            at(239),
            Some(Decimal::from(612_000u32)),
            "antes de P la pensión no existe: 600.000 + 12.000 de deuda"
        );
        assert_eq!(
            at(240),
            Some(Decimal::from(12_000u32)),
            "desde P solo queda la deuda"
        );
        assert!(at(240).is_some(), "nunca None: el cruce es INMEDIATO, no imposible");
    }

    /// **Puente de 3 meses sin descuento**, predicho a mano.
    ///
    /// Gasto 2.000 €/mes, sin impuestos, sin inflación, sin deuda, SWR 4 %; pensión plana de
    /// 1.200 €/mes desde el índice 3; `d = 0`.
    ///
    /// - `G(m) = gross_up_monthly(2.000) = 2.000` (sin impuestos, la identidad).
    /// - `need_net_m(3) = 800` ⇒ perpetuidad `9.600/0,04` = **240.000**.
    /// - `T(0) = 2.000·3 + 240.000` = **246.000**
    /// - `T(1) = 2.000·2 + 240.000` = **244.000**
    /// - `T(2) = 2.000·1 + 240.000` = **242.000**
    /// - `T(3) = 240.000` (desde `P` es la perpetuidad neta y NADA más)
    #[test]
    fn a_three_month_bridge_without_discount_is_the_plain_sum() {
        let target = ft(expense_need(2_000, 0), Decimal::from(4u32));
        let plan = plan_with_pension(3, 1_200, false, TargetBasis::BridgeToPension, Decimal::ZERO);
        let at = |i| fire_target_at_month_index_with_plan(Some(&target), &plan, i);

        assert_eq!(at(0), Some(Decimal::from(246_000u32)));
        assert_eq!(at(1), Some(Decimal::from(244_000u32)));
        assert_eq!(at(2), Some(Decimal::from(242_000u32)));
        assert_eq!(at(3), Some(Decimal::from(240_000u32)));
        assert_eq!(at(4), Some(Decimal::from(240_000u32)));
    }

    /// **El mismo puente al 12 % anual**: cada término descontado por `(1+d)^{−m/12}`.
    ///
    /// Predicho con aritmética decimal de 40 dígitos:
    ///
    /// ```text
    /// T(0) = 2.000/q(0) + 2.000/q(1) + 2.000/q(2) + 240.000/q(3),  q(j) = 1,12^{j/12}
    ///      = 2.000,00 + 1.981,20 + 1.962,58 + 233.295,70
    ///      = 239.239,48 €
    /// ```
    ///
    /// El test comprueba DOS cosas: el número (a menos de un céntimo) y —en `i = 0`, donde
    /// `q(0) = 1` es exacto— la **igualdad exacta** con la suma directa término a término, que es
    /// lo que ata la forma-cociente implementada a la fórmula del plan.
    #[test]
    fn a_three_month_bridge_discounts_every_term_at_twelve_percent() {
        let target = ft(expense_need(2_000, 0), Decimal::from(4u32));
        let plan = plan_with_pension(
            3,
            1_200,
            false,
            TargetBasis::BridgeToPension,
            Decimal::from(12u32),
        );
        let got = fire_target_at_month_index_with_plan(Some(&target), &plan, 0).unwrap();

        let expected = dec("239239.48");
        assert!(
            (got - expected).abs() < dec("0.01"),
            "T(0) predicho 239.239,48 €, obtenido {got}"
        );

        // La suma DIRECTA, con las mismas llamadas a `inflation_factor_at_month_index`.
        let q = |j: u32| inflation_factor_at_month_index(Decimal::from(12u32), j);
        let direct = Decimal::from(2_000u32) / q(0)
            + Decimal::from(2_000u32) / q(1)
            + Decimal::from(2_000u32) / q(2)
            + Decimal::from(240_000u32) / q(3);
        assert_eq!(
            got, direct,
            "en i = 0 la forma-cociente ES la suma directa: q(0) = 1 exacto"
        );
    }

    /// El puente **coincide con la perpetuidad neta desde `P`** — no hay discontinuidad al llegar.
    #[test]
    fn from_p_onwards_bridge_and_perpetuity_agree() {
        let target = ft(expense_need(2_000, 0), Decimal::from(4u32));
        let bridge = plan_with_pension(
            36,
            1_200,
            true,
            TargetBasis::BridgeToPension,
            Decimal::from(5u32),
        );
        let perpetuity =
            plan_with_pension(36, 1_200, true, TargetBasis::Perpetuity, Decimal::ZERO);
        for i in 36..60u32 {
            assert_eq!(
                fire_target_at_month_index_with_plan(Some(&target), &bridge, i),
                fire_target_at_month_index_with_plan(Some(&target), &perpetuity, i),
                "índice {i}"
            );
        }
    }

    /// Con la pensión cubriendo el 100 % del gasto el término perpetuo del puente es **0 exacto**
    /// y el objetivo es SOLO el puente (D15, escenario «llega para todo»).
    #[test]
    fn a_bridge_to_a_full_pension_is_only_the_bridge() {
        let target = ft(expense_need(2_000, 0), Decimal::from(4u32));
        let plan = plan_with_pension(3, 2_500, false, TargetBasis::BridgeToPension, Decimal::ZERO);
        assert_eq!(
            fire_target_at_month_index_with_plan(Some(&target), &plan, 0),
            Some(Decimal::from(6_000u32)),
            "tres meses de 2.000 € y ni un euro de perpetuidad"
        );
        assert_eq!(
            fire_target_at_month_index_with_plan(Some(&target), &plan, 3),
            Some(Decimal::ZERO),
            "desde P la pensión lo paga todo y no hay deuda"
        );
    }

    /// El evaluador precomputado y la función de un disparo **son la misma función**.
    #[test]
    fn the_precomputed_evaluator_matches_the_one_shot_function() {
        let mut target = ft(expense_need(2_100, 250), dec("3.5"));
        target.annual_inflation_percent = dec("2.2");
        let plan = plan_with_pension(
            48,
            900,
            true,
            TargetBasis::BridgeToPension,
            dec("4.5"),
        );
        let evaluator = PlanFireTarget::new(Some(&target), &plan);
        for i in 0..72u32 {
            assert_eq!(
                evaluator.at(i),
                fire_target_at_month_index_with_plan(Some(&target), &plan, i),
                "índice {i}"
            );
        }
    }

    /// La cobertura de la pensión se mide contra el GASTO, no contra el hueco.
    #[test]
    fn pension_coverage_ratio_measures_against_the_expense() {
        let target = ft(expense_need(2_000, 500), Decimal::from(4u32));
        let plan = plan_with_pension(12, 1_200, false, TargetBasis::Perpetuity, Decimal::ZERO);
        let evaluator = PlanFireTarget::new(Some(&target), &plan);
        assert_eq!(
            evaluator.pension_coverage_ratio(),
            Some(dec("0.6")),
            "1.200/2.000 — el ingreso persistente de 500 no entra en el denominador"
        );

        let no_pension = PhasePlan::classic(Decimal::ZERO, Decimal::from(2_000u32));
        assert_eq!(
            PlanFireTarget::new(Some(&target), &no_pension).pension_coverage_ratio(),
            None,
            "sin pensión con fecha no hay cobertura que medir — jamás un 0"
        );
    }

    /// El hueco de la media jornada del ejemplo del issue: 2.000 de gasto, 1.100 de ingreso,
    /// hueco 900 ⇒ `900·12/0,04` = **270.000 €**.
    #[test]
    fn the_partial_gap_target_is_the_issues_270k() {
        let target = ft(expense_need(2_000, 0), Decimal::from(4u32));
        let mut plan = PhasePlan::classic(Decimal::ZERO, Decimal::from(2_000u32));
        plan.partial = Some(PartialPhase {
            start_month: 60,
            income_monthly: Decimal::from(1_100u32),
            expense_basis: ExpenseBasis::Retirement,
        });
        let evaluator = PlanFireTarget::new(Some(&target), &plan);
        assert_eq!(
            evaluator.partial_gap_target(&plan, Decimal::from(3_000u32)),
            Some(Decimal::from(270_000u32))
        );

        // Un ingreso de media jornada que cubre el gasto: hueco 0, y CERO euros — no «no aplica».
        plan.partial = Some(PartialPhase {
            start_month: 60,
            income_monthly: Decimal::from(2_500u32),
            expense_basis: ExpenseBasis::Retirement,
        });
        assert_eq!(
            PlanFireTarget::new(Some(&target), &plan).partial_gap_target(&plan, Decimal::from(3_000u32)),
            Some(Decimal::ZERO)
        );
    }

    /// Una pensión declarada más allá del tope degrada a la perpetuidad ÍNTEGRA — la dirección
    /// prudente (objetivo MÁS grande), nunca a un puente truncado.
    #[test]
    fn a_pension_beyond_the_bridge_cap_degrades_upwards() {
        let target = ft(expense_need(2_000, 0), Decimal::from(4u32));
        let plan = plan_with_pension(
            MAX_BRIDGE_MONTHS + 1,
            1_200,
            false,
            TargetBasis::BridgeToPension,
            Decimal::from(5u32),
        );
        assert_eq!(
            fire_target_at_month_index_with_plan(Some(&target), &plan, 0),
            Some(Decimal::from(600_000u32)),
            "la perpetuidad sobre la necesidad íntegra, no un puente recortado"
        );
    }

    /// La puerta de `i = 0` sigue mandando también con pensión: sin necesidad HOY no hay objetivo
    /// en ningún mes, y eso es `None` — no un 0 que dispararía un cruce falso.
    #[test]
    fn no_need_today_still_means_no_target_at_all() {
        let target = ft(expense_need(1_000, 1_500), Decimal::from(4u32));
        for basis in [TargetBasis::Perpetuity, TargetBasis::BridgeToPension] {
            let plan = plan_with_pension(24, 500, true, basis, Decimal::from(5u32));
            for i in [0u32, 23, 24, 60] {
                assert_eq!(
                    fire_target_at_month_index_with_plan(Some(&target), &plan, i),
                    None,
                    "{basis:?} en {i}"
                );
            }
        }
    }
}
