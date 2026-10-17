//! **Déficit por el sumidero y ventas explícitas** (6.0.0 F1c, RFC «Planes» D-9, D-10, D-21, D-29).
//!
//! Dos piezas, las dos ADITIVAS detrás de un modo para que la API de 5.0 siga funcionando hasta
//! que F2 la cambie:
//!
//! 1. **Política de déficit** ([`DeficitPolicy`]). `LegacyDrain` (default) es el comportamiento de
//!    siempre, bit a bit: el déficit del mes se vende de TODOS los activos en el orden de
//!    `drain_order_g` (líquidos primero, menor rentabilidad primero) y el fallo solo cuenta en
//!    fase jubilada o parcial. `SinkOnly` es D-29: el déficit sale **solo del sumidero** de las
//!    reglas de reparto —el activo que recibe el sobrante— tributando su plusvalía con el MISMO
//!    gross-up mensual del motor; lo que el sumidero no cubre es un **mes sin cubrir**, y eso es
//!    fallo (`PathFailure::UncoveredMonth`) en CUALQUIER fase (D-9). Nada más se vende solo
//!    (D-10): ni un líquido que no sea el sumidero, ni un ilíquido.
//!
//! 2. **Ventas explícitas** ([`AssetSaleSpec`] → [`build_asset_sale_orders`] → [`AssetSales`]).
//!    El plan dice «vendo X €/mes netos del activo A de s a e», «vendo X € una vez en s» o «vendo
//!    A entero en s»; cada venta lleva su fiscalidad ([`SaleTaxMode`], D-21). El bucle las ejecuta
//!    ANTES de decidir el déficit del mes (son caja planeada) y en las DOS políticas.
//!
//! # Identificación del sumidero
//!
//! [`remainder_sink_index`]: el destino de la **primera regla `Remainder` sin tope** en el orden de
//! la cascada — la que se queda con todo lo que queda, que es exactamente lo que la cascada hace
//! con el sobrante. La API mantiene el invariante I1 (como mucho un `remainder` sin tope por scope,
//! y el último), y la simulación de `view=household` es la SUMA de una simulación por miembro, así
//! que cada `SimInput` tiene un solo scope y un solo sumidero. Si una entrada trajera varios (el
//! motor es una función pura y lo admite), manda el PRIMERO: es el único que la cascada llega a
//! alimentar sin tope. Sin regla así, no hay sumidero y todo déficit queda sin cubrir.
//!
//! **Un sumidero ILÍQUIDO no se vende nunca** para cubrir un déficit (decisión del owner, D-10:
//! nada ilíquido se vende sin un cambio explícito): el bucle lo trata como «sin sumidero» y el
//! déficit del mes queda sin cubrir.
//!
//! **Fallo en `SinkOnly`**: SOLO el mes sin cubrir (`PathFailure::UncoveredMonth`), en cualquier
//! fase. La puerta de tasa inicial (F2) no firma en este modo (decisión del owner).
//!
//! # Fiscalidad de las ventas explícitas
//!
//! Con `g` la plusvalía relativa del activo en ese momento (`1 − base/valor` si la base es un dato,
//! el escalar `taxable_gain_ratio` si no — la MISMA regla que la venta del mes):
//!
//! | modo | bruto `G` para un neto `N` | impuesto `T` |
//! |---|---|---|
//! | `Auto`, mensual | `gross_up_monthly(N, tramos, g)` (anualización M1, como el déficit) | `G − N` |
//! | `Auto`, puntual/entera | `gross_up_net_annual_fire(N, tramos, g)` (la venta es UN hecho del año) | `G − N` |
//! | `Exempt` | `N` | `0` |
//! | `ManualAmount { tax }` | `N + tax` | `tax` (nominal, por mes en una venta mensual) |
//! | `ManualPct { pct }` | `N / (1 − pct/100)` | `G · pct/100` |
//!
//! Si `G` supera el valor del activo, se vende el activo entero (`G = v`), el impuesto se recalcula
//! sobre `v` con la misma regla (`Auto` mensual: `tax(12·g·v)/12`; `Auto` puntual: `tax(g·v)`;
//! `ManualAmount`: `min(tax, v)`; `ManualPct`: `v·pct/100`) y la orden se marca como **agotada**
//! (la venta se detiene: las órdenes posteriores sobre ese activo encuentran 0). Un `pct ≥ 100`
//! no puede netear nada: la orden no vende y se marca agotada. La base de coste del resto baja en
//! proporción al valor vendido, `b' = b·v_post/v_pre` (#120), igual que en la venta del mes.

use crate::money::MoneyOps;
use crate::sim::TaxBracketG;
use crate::sim_core::inflation_factor_at_index_g;
use rust_decimal::Decimal;

/// Qué cubre el déficit de un mes. Ver el módulo.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DeficitPolicy {
    /// 5.0, bit a bit: venta automática de todos los activos en el orden de drenaje; fallo solo
    /// en fase jubilada/parcial.
    #[default]
    LegacyDrain,
    /// D-29: el déficit sale solo del sumidero (con su fiscalidad); lo no cubierto es fallo en
    /// cualquier fase.
    SinkOnly,
}

/// Fiscalidad de una venta explícita (D-21). Ver la tabla del módulo.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SaleTaxMode<M> {
    /// Tramos del ahorro sobre la plusvalía realizada, con el gross-up del motor.
    Auto,
    /// Sin impuesto (vivienda habitual reinvertida, mayores de 65…; la app no valida la ley).
    Exempt,
    /// Un impuesto NOMINAL fijo por evento de venta (en una venta mensual, cada mes).
    ManualAmount { tax: M },
    /// Un porcentaje del BRUTO vendido.
    ManualPct { pct: M },
}

/// Qué vende una venta explícita.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum AssetSaleKind<M> {
    /// Un neto mensual, cada mes de `[start, end]` (o hasta agotar el activo).
    MonthlyNet { amount_today: M },
    /// Un neto una sola vez, en el mes de inicio (`end_month` se ignora).
    OneOffNet { amount_today: M },
    /// El activo entero, en el mes de inicio (`end_month` y `keeps_purchasing_power` se ignoran).
    Whole,
}

/// Una venta explícita de un plan, en la rejilla del bucle (la misma de `start_month` de F1a).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AssetSaleSpec<M> {
    /// Índice del activo en `ProjectionInput::assets`.
    pub asset_index: usize,
    /// Primer mes (1-based). `0` se trata como `1`; posterior al horizonte ⇒ sin efecto.
    pub start_month: u32,
    /// Último mes, INCLUSIVO, de una venta mensual. `None` ⇒ hasta el horizonte (o hasta agotar).
    pub end_month: Option<u32>,
    pub kind: AssetSaleKind<M>,
    /// `true` ⇒ el importe está en euros de hoy y se indexa con el factor del bucle
    /// `f(k) = inflation_factor_at_index_g(inflación, k−1)`; `false` ⇒ nominal.
    pub keeps_purchasing_power: bool,
    pub tax_mode: SaleTaxMode<M>,
}

/// Convención fiscal de una orden: mensual (anualización M1, «este mes repetido doce veces») o
/// puntual (la venta es un hecho aislado del año fiscal).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SaleCadence {
    Monthly,
    OneOff,
}

/// Cuánto vende una orden.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SaleAmount<M> {
    /// Un neto NOMINAL (ya indexado por el constructor).
    Net(M),
    /// El activo entero.
    All,
}

/// Una orden de venta ya resuelta a un mes concreto: lo que consume el bucle.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SaleOrderG<M> {
    /// Mes del bucle (1-based), `1 ..= horizon_months`.
    pub month: u32,
    pub asset_index: usize,
    pub amount: SaleAmount<M>,
    pub cadence: SaleCadence,
    pub tax_mode: SaleTaxMode<M>,
}

/// Las ventas explícitas de un plan, como las consume el bucle: órdenes en orden NO decreciente
/// de mes (dentro del mes, en el orden de las especificaciones). Vacío (el default) ⇒ el bucle no
/// ejecuta ni una operación de más.
#[derive(Debug, Clone, PartialEq)]
pub struct AssetSalesG<M> {
    pub orders: Vec<SaleOrderG<M>>,
}

impl<M> Default for AssetSalesG<M> {
    fn default() -> Self {
        AssetSalesG { orders: Vec::new() }
    }
}

/// Las ventas en el dinero del dominio: lo que lleva [`crate::ProjectionInput`].
pub type AssetSales = AssetSalesG<Decimal>;

impl AssetSales {
    pub(crate) fn to_generic<M: MoneyOps>(&self) -> AssetSalesG<M> {
        let tax = |t: SaleTaxMode<Decimal>| match t {
            SaleTaxMode::Auto => SaleTaxMode::Auto,
            SaleTaxMode::Exempt => SaleTaxMode::Exempt,
            SaleTaxMode::ManualAmount { tax } => SaleTaxMode::ManualAmount {
                tax: M::from_decimal(tax),
            },
            SaleTaxMode::ManualPct { pct } => SaleTaxMode::ManualPct {
                pct: M::from_decimal(pct),
            },
        };
        AssetSalesG {
            orders: self
                .orders
                .iter()
                .map(|o| SaleOrderG {
                    month: o.month,
                    asset_index: o.asset_index,
                    amount: match o.amount {
                        SaleAmount::Net(n) => SaleAmount::Net(M::from_decimal(n)),
                        SaleAmount::All => SaleAmount::All,
                    },
                    cadence: o.cadence,
                    tax_mode: tax(o.tax_mode),
                })
                .collect(),
        }
    }
}

impl<M: MoneyOps> AssetSalesG<M> {
    /// Cada orden con activo en rango, mes en `1..=horizon` y meses no decrecientes.
    pub(crate) fn orders_ok(&self, n_assets: usize, horizon: u32) -> bool {
        let mut prev = 0u32;
        for o in &self.orders {
            if o.asset_index >= n_assets || o.month == 0 || o.month > horizon || o.month < prev {
                return false;
            }
            prev = o.month;
        }
        true
    }
}

/// **Construye las órdenes del bucle a partir de las ventas de un plan** (función pura, como
/// [`crate::build_change_vectors`]): resuelve euros de hoy → nominal con el factor ÚNICO del bucle.
///
/// - `MonthlyNet`: una orden `Net(a·f(k))` (o `Net(a)`) por cada mes `k` de `[s, e]`.
/// - `OneOffNet`: una orden `Net(a·f(s))` (o `Net(a)`) en `s`.
/// - `Whole`: una orden `All` en `s`.
///
/// Un importe ≤ 0 no genera orden (no hay «venta negativa»; comprar es un cambio de otro tipo).
/// Las órdenes salen ordenadas por mes con orden ESTABLE: dentro de un mes, en el orden de
/// `specs` — que es el orden en que el bucle las ejecuta.
pub fn build_asset_sale_orders<M: MoneyOps>(
    specs: &[AssetSaleSpec<M>],
    horizon_months: u32,
    annual_inflation_percent: M,
) -> AssetSalesG<M> {
    let indexed = |amount: M, keeps: bool, k: u32| -> M {
        if keeps {
            amount * inflation_factor_at_index_g(annual_inflation_percent, k - 1)
        } else {
            amount
        }
    };
    let mut orders: Vec<SaleOrderG<M>> = Vec::new();
    for s in specs {
        let start = s.start_month.max(1);
        if start > horizon_months {
            continue;
        }
        match s.kind {
            AssetSaleKind::MonthlyNet { amount_today } => {
                if amount_today <= M::zero() {
                    continue;
                }
                let end = s.end_month.unwrap_or(horizon_months).min(horizon_months);
                for k in start..=end {
                    orders.push(SaleOrderG {
                        month: k,
                        asset_index: s.asset_index,
                        amount: SaleAmount::Net(indexed(amount_today, s.keeps_purchasing_power, k)),
                        cadence: SaleCadence::Monthly,
                        tax_mode: s.tax_mode,
                    });
                }
            }
            AssetSaleKind::OneOffNet { amount_today } => {
                if amount_today <= M::zero() {
                    continue;
                }
                orders.push(SaleOrderG {
                    month: start,
                    asset_index: s.asset_index,
                    amount: SaleAmount::Net(indexed(amount_today, s.keeps_purchasing_power, start)),
                    cadence: SaleCadence::OneOff,
                    tax_mode: s.tax_mode,
                });
            }
            AssetSaleKind::Whole => orders.push(SaleOrderG {
                month: start,
                asset_index: s.asset_index,
                amount: SaleAmount::All,
                cadence: SaleCadence::OneOff,
                tax_mode: s.tax_mode,
            }),
        }
    }
    orders.sort_by_key(|o| o.month);
    AssetSalesG { orders }
}

/// El sumidero de la cascada: destino de la PRIMERA regla `Remainder` sin tope. Ver el módulo.
pub fn remainder_sink_index<M>(rules: &[crate::sim::AllocationRuleG<M>]) -> Option<usize> {
    rules
        .iter()
        .find(|r| r.kind == crate::projection::AllocationKind::Remainder && r.cap.is_none())
        .map(|r| r.target_index)
}

/// Lo que una orden ejecutó: bruto que sale del activo (`≤ valor`), neto que entra en caja,
/// impuesto, y si la orden NO se pudo llenar entera (activo agotado).
#[derive(Debug, Clone, Copy)]
pub(crate) struct SaleExecution<M> {
    pub gross: M,
    pub net: M,
    pub tax: M,
    pub short: bool,
}

/// Impuesto de vender `gross` con este modo. Ver la tabla del módulo.
fn tax_for_gross_g<M: MoneyOps>(
    gross: M,
    gain: M,
    cadence: SaleCadence,
    mode: SaleTaxMode<M>,
    brackets: &[TaxBracketG<M>],
    taxes_enabled: bool,
) -> M {
    if gross <= M::zero() {
        return M::zero();
    }
    match mode {
        SaleTaxMode::Auto => {
            if !taxes_enabled {
                return M::zero();
            }
            match cadence {
                SaleCadence::Monthly => {
                    gross - crate::tax::after_tax_monthly_g(gross, brackets, true, gain)
                }
                SaleCadence::OneOff => {
                    crate::tax::tax_on_gross_capital_annual_g(gross * gain, brackets)
                }
            }
        }
        SaleTaxMode::Exempt => M::zero(),
        SaleTaxMode::ManualAmount { tax } => tax.max(M::zero()).min(gross),
        SaleTaxMode::ManualPct { pct } => {
            gross * pct.clamp(M::zero(), M::from_u32(100)) / M::from_u32(100)
        }
    }
}

/// Bruto que netea `net` con este modo; `None` si ningún bruto puede (pct ≥ 100).
fn gross_for_net_g<M: MoneyOps>(
    net: M,
    gain: M,
    cadence: SaleCadence,
    mode: SaleTaxMode<M>,
    brackets: &[TaxBracketG<M>],
    taxes_enabled: bool,
) -> Option<M> {
    match mode {
        SaleTaxMode::Auto => Some(match cadence {
            SaleCadence::Monthly => {
                crate::tax::gross_up_monthly_g(net, brackets, taxes_enabled, gain)
            }
            SaleCadence::OneOff => {
                crate::tax::gross_up_net_annual_fire_g(net, brackets, taxes_enabled, gain)
            }
        }),
        SaleTaxMode::Exempt => Some(net),
        SaleTaxMode::ManualAmount { tax } => Some(net + tax.max(M::zero())),
        SaleTaxMode::ManualPct { pct } => {
            let keep = M::one() - pct.max(M::zero()) / M::from_u32(100);
            if keep <= M::zero() {
                None
            } else {
                Some(net / keep)
            }
        }
    }
}

/// Ejecuta UNA orden sobre un activo de valor `value` y plusvalía relativa `gain`. No muta nada:
/// el bucle resta `gross` del valor y encoge la base.
pub(crate) fn execute_sale_order_g<M: MoneyOps>(
    value: M,
    gain: M,
    order: &SaleOrderG<M>,
    brackets: &[TaxBracketG<M>],
    taxes_enabled: bool,
) -> SaleExecution<M> {
    let v = value.max(M::zero());
    let full = |gross: M, short: bool| {
        let tax = tax_for_gross_g(
            gross,
            gain,
            order.cadence,
            order.tax_mode,
            brackets,
            taxes_enabled,
        );
        SaleExecution {
            gross,
            net: gross - tax,
            tax,
            short,
        }
    };
    match order.amount {
        SaleAmount::All => full(v, v <= M::zero()),
        SaleAmount::Net(n) => {
            if n <= M::zero() {
                return SaleExecution {
                    gross: M::zero(),
                    net: M::zero(),
                    tax: M::zero(),
                    short: false,
                };
            }
            match gross_for_net_g(
                n,
                gain,
                order.cadence,
                order.tax_mode,
                brackets,
                taxes_enabled,
            ) {
                Some(g) if g <= v => SaleExecution {
                    gross: g,
                    net: n,
                    tax: g - n,
                    short: false,
                },
                Some(_) => full(v, true),
                None => SaleExecution {
                    gross: M::zero(),
                    net: M::zero(),
                    tax: M::zero(),
                    short: true,
                },
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(n: i64) -> Decimal {
        Decimal::from(n)
    }

    #[test]
    fn builder_resolves_today_euros_and_orders_by_month() {
        // Predicción: inflación 0 ⇒ importes tal cual; la mensual de 3 a 5 da 3 órdenes, la
        // puntual en 4 cae entre ellas (orden estable dentro del mes 4: la mensual va primero
        // porque su spec va primero).
        let specs = [
            AssetSaleSpec {
                asset_index: 0,
                start_month: 3,
                end_month: Some(5),
                kind: AssetSaleKind::MonthlyNet {
                    amount_today: d(100),
                },
                keeps_purchasing_power: true,
                tax_mode: SaleTaxMode::Auto,
            },
            AssetSaleSpec {
                asset_index: 1,
                start_month: 4,
                end_month: None,
                kind: AssetSaleKind::Whole,
                keeps_purchasing_power: false,
                tax_mode: SaleTaxMode::Exempt,
            },
        ];
        let out = build_asset_sale_orders(&specs, 12, Decimal::ZERO);
        let months: Vec<(u32, usize)> = out
            .orders
            .iter()
            .map(|o| (o.month, o.asset_index))
            .collect();
        assert_eq!(months, vec![(3, 0), (4, 0), (4, 1), (5, 0)]);
        assert_eq!(out.orders[0].amount, SaleAmount::Net(d(100)));
        assert_eq!(out.orders[2].amount, SaleAmount::All);
        assert!(out.orders_ok(2, 12));
        assert!(!out.orders_ok(1, 12), "activo fuera de rango");
    }

    #[test]
    fn indexed_amount_uses_the_loop_factor() {
        // Predicción: 2 % anual, mes 13 ⇒ f = 1,02^(12/12) = 1,02 exacto ⇒ 1.000 · 1,02 = 1.020.
        let specs = [AssetSaleSpec {
            asset_index: 0,
            start_month: 13,
            end_month: None,
            kind: AssetSaleKind::OneOffNet {
                amount_today: d(1_000),
            },
            keeps_purchasing_power: true,
            tax_mode: SaleTaxMode::Auto,
        }];
        let out = build_asset_sale_orders(&specs, 24, d(2));
        assert_eq!(out.orders.len(), 1);
        assert_eq!(out.orders[0].amount, SaleAmount::Net(d(1_020)));
    }

    #[test]
    fn manual_modes_compute_their_tax() {
        // ManualPct 10 %: N = 900 ⇒ G = 1.000, T = 100. ManualAmount 50: N = 900 ⇒ G = 950.
        // Exempt: G = N. pct 100 ⇒ no vende, agotada.
        let order = |mode| SaleOrderG {
            month: 1,
            asset_index: 0,
            amount: SaleAmount::Net(d(900)),
            cadence: SaleCadence::Monthly,
            tax_mode: mode,
        };
        let e = execute_sale_order_g(
            d(10_000),
            Decimal::ONE,
            &order(SaleTaxMode::ManualPct { pct: d(10) }),
            &[],
            true,
        );
        assert_eq!(
            (e.gross, e.net, e.tax, e.short),
            (d(1_000), d(900), d(100), false)
        );
        let e = execute_sale_order_g(
            d(10_000),
            Decimal::ONE,
            &order(SaleTaxMode::ManualAmount { tax: d(50) }),
            &[],
            true,
        );
        assert_eq!((e.gross, e.net, e.tax), (d(950), d(900), d(50)));
        let e = execute_sale_order_g(
            d(10_000),
            Decimal::ONE,
            &order(SaleTaxMode::Exempt),
            &[],
            true,
        );
        assert_eq!((e.gross, e.net, e.tax), (d(900), d(900), d(0)));
        let e = execute_sale_order_g(
            d(10_000),
            Decimal::ONE,
            &order(SaleTaxMode::ManualPct { pct: d(100) }),
            &[],
            true,
        );
        assert_eq!((e.gross, e.net, e.short), (d(0), d(0), true));
        // Activo corto: vende 500, ManualPct 10 % ⇒ T = 50, N = 450, agotada.
        let e = execute_sale_order_g(
            d(500),
            Decimal::ONE,
            &order(SaleTaxMode::ManualPct { pct: d(10) }),
            &[],
            true,
        );
        assert_eq!(
            (e.gross, e.net, e.tax, e.short),
            (d(500), d(450), d(50), true)
        );
    }
}
