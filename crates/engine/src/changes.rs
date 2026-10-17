//! **Cambios de ingreso y gasto con fecha** (6.0.0 F1b, RFC «Planes» D-4/D-5/D-6/D-8, §3–§5).
//!
//! En 6.0 un plan es una lista de cambios con fecha sobre la realidad de hoy, y los cambios de
//! ingreso y gasto se expresan como **delta relativo** a las cifras actuales («desde 03/2028 mi
//! gasto baja 900 €/mes», «desde 03/2051 mi ingreso baja un 100 %», «desde 01/2063 mi ingreso sube
//! 1.400 €/mes»). El motor los recibe ya resueltos a **cuatro vectores mensuales** y una
//! indexación por lado:
//!
//! ```text
//! income_k  = base_income_k  · f_inc(k) · income_scale[k−1]  + income_delta[k−1]
//! expense_k = base_expense_k · f_exp(k) · expense_scale[k−1] + expense_delta[k−1]
//! ```
//!
//! - `base_*_k` es la base de la FASE del mes (regular / media jornada / jubilado) — los cambios
//!   se aplican en todas las fases, encima de la base que la fase use; la pausa de ingresos
//!   multiplica la base ANTES (es parte de la base). La pensión con fecha se suma DESPUÉS y no la
//!   tocan ni la escala ni el delta.
//! - `f(k) = inflation_factor_at_index_g(inflación, k−1)`: el factor ÚNICO del bucle (#139).
//!   `f_exp` es `f` si `expense_follows_inflation` (default `true`, el comportamiento de siempre)
//!   y `1` si no; `f_inc` es `f` si `income_follows_inflation` (default `false`, legacy) y `1` si
//!   no.
//! - Las escalas son multiplicativas sobre la base (1 = sin cambio, 0 = «−100 %»); los deltas son
//!   NOMINALES (ya convertidos desde euros de hoy por [`build_change_vectors`]) y la escala no los
//!   toca.
//! - **Suelo en 0**: si `income_k` o `expense_k` salen negativos se publican como 0 y el motor
//!   emite una vez [`crate::EngineWarning::CashFlowClampedAtZero`]. Un gasto negativo sería un
//!   ingreso fantasma (y un ingreso negativo un gasto fantasma) que ningún cambio declaró: lo
//!   honesto es no inventarlo y avisar, no propagarlo en silencio.
//!
//! Vectores vacíos = sin cambios: el bucle no ejecuta ni una operación de más y la salida es
//! bit-idéntica a la de 5.0 (pines dorados sin regenerar).

use crate::money::MoneyOps;
use crate::sim_core::inflation_factor_at_index_g;
use rust_decimal::Decimal;

/// Los cuatro vectores mensuales de cambios. Cada uno está **vacío** (sin cambios en ese
/// componente, cero operaciones) o tiene **exactamente `horizon_months`** elementos, índice
/// `k−1` para el mes `k` (rejilla 1-based del bucle, la misma de `start_month` en F1a). Otro
/// largo es `EngineError::InvalidIncomeExpenseChanges`.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ChangeVectorsG<M> {
    /// Factor multiplicativo sobre la base del ingreso del mes.
    pub income_scale_monthly: Vec<M>,
    /// Factor multiplicativo sobre la base del gasto del mes.
    pub expense_scale_monthly: Vec<M>,
    /// Delta NOMINAL del ingreso del mes, con signo.
    pub income_delta_monthly: Vec<M>,
    /// Delta NOMINAL del gasto del mes, con signo.
    pub expense_delta_monthly: Vec<M>,
}

/// Los cambios de ingreso/gasto de un plan, tal como los consume el bucle. El `Default` es el
/// comportamiento de 5.0: gasto indexado, ingreso plano y ningún vector.
#[derive(Debug, Clone, PartialEq)]
pub struct IncomeExpenseChangesG<M> {
    /// `true` ⇒ la base del ingreso se multiplica por el mismo factor de inflación que el gasto
    /// (`f(k)`, eje `(k−1)/12`). Default `false` (#139: ingresos planos).
    pub income_follows_inflation: bool,
    /// `false` ⇒ la base del gasto NO se indexa. Default `true` (el comportamiento desde #139).
    /// La pensión con fecha indexada sigue usando `f(k)` en cualquier caso.
    pub expense_follows_inflation: bool,
    pub vectors: ChangeVectorsG<M>,
}

impl<M> Default for IncomeExpenseChangesG<M> {
    fn default() -> Self {
        IncomeExpenseChangesG {
            income_follows_inflation: false,
            expense_follows_inflation: true,
            vectors: ChangeVectorsG {
                income_scale_monthly: Vec::new(),
                expense_scale_monthly: Vec::new(),
                income_delta_monthly: Vec::new(),
                expense_delta_monthly: Vec::new(),
            },
        }
    }
}

/// Los cambios en el dinero del dominio: lo que lleva [`crate::ProjectionInput`].
pub type IncomeExpenseChanges = IncomeExpenseChangesG<Decimal>;
/// Los vectores en el dinero del dominio: lo que devuelve [`build_change_vectors`] con `Decimal`.
pub type ChangeVectors = ChangeVectorsG<Decimal>;

impl IncomeExpenseChanges {
    pub(crate) fn to_generic<M: MoneyOps>(&self) -> IncomeExpenseChangesG<M> {
        let conv = |v: &[Decimal]| v.iter().copied().map(M::from_decimal).collect::<Vec<M>>();
        IncomeExpenseChangesG {
            income_follows_inflation: self.income_follows_inflation,
            expense_follows_inflation: self.expense_follows_inflation,
            vectors: ChangeVectorsG {
                income_scale_monthly: conv(&self.vectors.income_scale_monthly),
                expense_scale_monthly: conv(&self.vectors.expense_scale_monthly),
                income_delta_monthly: conv(&self.vectors.income_delta_monthly),
                expense_delta_monthly: conv(&self.vectors.expense_delta_monthly),
            },
        }
    }
}

impl<M: MoneyOps> IncomeExpenseChangesG<M> {
    /// ¿Hay algo que aplicar? `false` en el default: el bucle sigue el camino de 5.0 sin tocar
    /// una sola operación.
    pub(crate) fn is_active(&self) -> bool {
        self.income_follows_inflation
            || !self.expense_follows_inflation
            || !self.vectors.income_scale_monthly.is_empty()
            || !self.vectors.expense_scale_monthly.is_empty()
            || !self.vectors.income_delta_monthly.is_empty()
            || !self.vectors.expense_delta_monthly.is_empty()
    }

    /// Cada vector vacío o de largo exacto `horizon`.
    pub(crate) fn lengths_ok(&self, horizon: u32) -> bool {
        let ok = |v: &[M]| v.is_empty() || v.len() == horizon as usize;
        ok(&self.vectors.income_scale_monthly)
            && ok(&self.vectors.expense_scale_monthly)
            && ok(&self.vectors.income_delta_monthly)
            && ok(&self.vectors.expense_delta_monthly)
    }
}

/// Aplica escala y delta del índice `i` (= `k−1`) sobre un importe ya indexado, con suelo en 0.
/// Devuelve `(importe, se_clampó)`. El booleano es un veredicto PUBLICADO (dispara un aviso), así
/// que lo decide el tipo (`strictly_below`): un `−1e−13` de coma flotante donde `Decimal` da 0
/// exacto no puede avisar por un camino y no por el otro.
pub(crate) fn apply_scale_delta_g<M: MoneyOps>(
    amount: M,
    scale: &[M],
    delta: &[M],
    i: usize,
) -> (M, bool) {
    let mut x = amount;
    if let Some(s) = scale.get(i) {
        x = x * *s;
    }
    if let Some(d) = delta.get(i) {
        x = x + *d;
    }
    let clamped = M::strictly_below(x, M::zero());
    (x.max(M::zero()), clamped)
}

// =============================================================================================
// Constructor de vectores desde especificaciones de cambio
// =============================================================================================

/// A qué lado del flujo de caja va un cambio.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChangeTarget {
    Income,
    Expense,
}

/// Qué hace un cambio. Los importes llevan SIGNO: «el gasto baja 900 €/mes» es
/// `DeltaMonthly { amount_today: −900 }`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ChangeKind<M> {
    /// Un importe mensual, cada mes de `[start, end]`.
    DeltaMonthly { amount_today: M },
    /// Un importe una sola vez, en el mes de inicio (`end_month` se ignora).
    OneOff { amount_today: M },
    /// Un porcentaje sobre la BASE del mes: factor `1 + pct/100` en `[start, end]` (−100 ⇒ 0).
    /// Varios porcentajes solapados se componen multiplicando. `keeps_purchasing_power` no
    /// aplica: el porcentaje sigue a la base, que lleva su propia indexación.
    Percent { pct: M },
}

/// Un cambio de ingreso o gasto con fecha, en la rejilla del bucle.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct IncomeExpenseChange<M> {
    pub target: ChangeTarget,
    /// Primer mes (1-based, la misma rejilla que `start_month` de F1a). `0` se trata como `1`.
    /// Un mes posterior al horizonte deja el cambio sin efecto.
    pub start_month: u32,
    /// Último mes, INCLUSIVO. `None` ⇒ hasta el horizonte. Si es anterior al inicio, el cambio
    /// no tiene efecto.
    pub end_month: Option<u32>,
    pub kind: ChangeKind<M>,
    /// `true` ⇒ el importe está en euros de hoy y se indexa mes a mes con el MISMO factor del
    /// bucle (`f(k) = inflation_factor_at_index_g(inflación, k−1)`); `false` ⇒ el importe es
    /// nominal tal cual, fijo desde el mes de inicio.
    pub keeps_purchasing_power: bool,
}

/// **Construye los vectores del motor a partir de los cambios de un plan** (función pura, sin
/// E/S), para que la API no duplique la conversión euros-de-hoy → nominal.
///
/// - `DeltaMonthly`: en cada mes `k` de `[s, e]`, `delta[k−1] += a·f(k)` si se indexa, `+= a` si no.
/// - `OneOff`: `delta[s−1] += a·f(s)` (o `a`).
/// - `Percent`: `scale[k−1] *= 1 + p/100` en cada mes de `[s, e]`.
///
/// con `f(k) = inflation_factor_at_index_g(annual_inflation_percent, k−1)`. Un vector al que no
/// apunta ningún cambio queda VACÍO (cero operaciones en el bucle). No se valida el signo ni el
/// rango del porcentaje: si la combinación deja un mes negativo, el bucle aplica el suelo en 0 y
/// avisa (ver el módulo).
pub fn build_change_vectors<M: MoneyOps>(
    changes: &[IncomeExpenseChange<M>],
    horizon_months: u32,
    annual_inflation_percent: M,
) -> ChangeVectorsG<M> {
    let h = horizon_months as usize;
    let mut out = ChangeVectorsG {
        income_scale_monthly: Vec::new(),
        expense_scale_monthly: Vec::new(),
        income_delta_monthly: Vec::new(),
        expense_delta_monthly: Vec::new(),
    };
    let indexed = |amount: M, keeps: bool, k: u32| -> M {
        if keeps {
            amount * inflation_factor_at_index_g(annual_inflation_percent, k - 1)
        } else {
            amount
        }
    };
    for c in changes {
        let start = c.start_month.max(1);
        if start > horizon_months {
            continue;
        }
        let end = c.end_month.unwrap_or(horizon_months).min(horizon_months);
        match c.kind {
            ChangeKind::Percent { pct } => {
                if end < start {
                    continue;
                }
                let v = match c.target {
                    ChangeTarget::Income => &mut out.income_scale_monthly,
                    ChangeTarget::Expense => &mut out.expense_scale_monthly,
                };
                if v.is_empty() {
                    *v = vec![M::one(); h];
                }
                let factor = M::one() + pct / M::from_u32(100);
                for k in start..=end {
                    v[(k - 1) as usize] = v[(k - 1) as usize] * factor;
                }
            }
            ChangeKind::DeltaMonthly { amount_today } => {
                if end < start {
                    continue;
                }
                let v = match c.target {
                    ChangeTarget::Income => &mut out.income_delta_monthly,
                    ChangeTarget::Expense => &mut out.expense_delta_monthly,
                };
                if v.is_empty() {
                    *v = vec![M::zero(); h];
                }
                for k in start..=end {
                    let i = (k - 1) as usize;
                    v[i] = v[i] + indexed(amount_today, c.keeps_purchasing_power, k);
                }
            }
            ChangeKind::OneOff { amount_today } => {
                let v = match c.target {
                    ChangeTarget::Income => &mut out.income_delta_monthly,
                    ChangeTarget::Expense => &mut out.expense_delta_monthly,
                };
                if v.is_empty() {
                    *v = vec![M::zero(); h];
                }
                let i = (start - 1) as usize;
                v[i] = v[i] + indexed(amount_today, c.keeps_purchasing_power, start);
            }
        }
    }
    out
}
