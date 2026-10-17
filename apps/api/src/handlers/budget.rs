use crate::error::ApiError;
use crate::handlers::installation::{installation_naive_today, require_installation_member};
use crate::handlers::liabilities::PaymentFrequency;
use crate::handlers::membership::role_can_write;
use crate::handlers::person_view::{require_row_owner, LedgerView, LedgerViewQuery};
use crate::handlers::projection::refresh_projection_after_mutation;
use crate::handlers::session::require_session_user;
use crate::money::money_out;
use crate::state::AppState;
use axum::extract::{Extension, Path, Query};
use axum::routing::{get, patch};
use axum::{Json, Router};
use axum_extra::extract::cookie::CookieJar;
use chrono::{Datelike, NaiveDate};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use std::sync::Arc;
use utoipa::ToSchema;
use uuid::Uuid;

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum BudgetCategoryScope {
    Income,
    Expense,
}

/// Procedencia de una línea del presupuesto.
///
/// Hasta la 3.6.0 las cuotas de pasivo vivían en un bloque aparte (`derived_from_liabilities`)
/// que se sumaba por debajo del presupuesto; desde la 3.7.0 son una partida más de `entries`,
/// atribuida a la categoría de gasto que declara el pasivo (`expense_category_id`), y este
/// campo es lo que distingue una fila editable de una derivada.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum BudgetEntrySource {
    /// Fila persistida en `budget_entries`: la escribe el usuario y admite PATCH/DELETE.
    Manual,
    /// Cuota derivada del plan de pago de un pasivo activo. **Solo lectura**: no existe en
    /// `budget_entries`, así que `PATCH`/`DELETE /v1/budget/entries/{id}` responden 404. Se
    /// edita en `/v1/liabilities`.
    Liability,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct BudgetEntryResponse {
    /// Id de la fila de `budget_entries` (`source = manual`) o del pasivo que genera la cuota
    /// (`source = liability`). Único en ambos casos — los UUID no colisionan entre tablas.
    #[schema(value_type = String, format = "uuid")]
    pub id: Uuid,
    /// Categoría de la partida. Ausente **solo** en cuotas de pasivos sin `expense_category_id`
    /// asignada (pasivos anteriores a la 3.4.0 y backups viejos importados): siguen contando en
    /// los totales, pero no tienen categoría donde sentarse hasta que se les asigne una.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>, format = "uuid")]
    pub category_id: Option<Uuid>,
    pub scope: BudgetCategoryScope,
    /// Procedencia: `manual` (editable) o `liability` (cuota derivada, solo lectura).
    pub source: BudgetEntrySource,
    /// Pasivo que genera la cuota; presente ⟺ `source = liability` (mismo valor que `id`).
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>, format = "uuid")]
    pub liability_id: Option<Uuid>,
    /// Etiqueta del pasivo (p. ej. «Hipoteca»); presente ⟺ `source = liability`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// Importe mensual (el presupuesto persistido es solo mensual). En una cuota derivada es el
    /// **equivalente mensual** del plan (`weekly` → ×52/12); el importe y la frecuencia crudos
    /// viven en `/v1/liabilities`.
    #[serde(with = "rust_decimal::serde::str")]
    #[schema(value_type = String)]
    pub amount: Decimal,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
    pub sort_index: i32,
    /// Whether this income entry continues contributing after the retirement start month.
    pub persists_after_retirement: bool,
    /// Whether this expense entry stops at the retirement start month (always `false` for income).
    pub ends_at_retirement: bool,
    /// The date on which this expense entry stops counting (exclusive of the month that starts after this date).
    /// `null` when the expense has no explicit end date. Always `null` for income entries.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expense_end_date: Option<NaiveDate>,
}

/// Valor único de `BudgetTotalsResponse::basis`. Constante y no una cadena suelta: el mismo
/// literal lo consumen el test de contrato y la descripción de la tool MCP `get_budget`.
pub(crate) const BUDGET_TOTALS_BASIS: &str = "plan";

#[derive(Debug, Serialize, ToSchema)]
pub struct BudgetTotalsResponse {
    /// **Siempre `"plan"`.** Declara que estos totales son el PRESUPUESTO —lo que el usuario dijo
    /// que ingresaría y gastaría— y nunca lo realmente medido, en ningún modo de `savings_source`.
    ///
    /// No es decoración. Cuatro de estos campos —`income_monthly_equivalent`,
    /// `expense_regular_monthly_equivalent`, `expense_total_monthly_equivalent` y
    /// `net_monthly_equivalent`— se llaman **exactamente igual** que cuatro de
    /// `GET /v1/summary` → `financial_health`, y allí siguen el modo: en `budget` (modo A) valen lo
    /// mismo que aquí, y en `transactions_avg` / `budget_income_real_expense` (modos B y C) valen
    /// el promedio real. Dos cifras distintas con el mismo nombre en el mismo catálogo es
    /// exactamente el incidente de las tres cifras de ahorro de 3.9.0. Los nombres no se renombran
    /// (rompería la SPA y el `.ffbackup` de nadie mejoraría por ello): lo que faltaba era que cada
    /// lado **declarase su base**, que es lo que este campo y su gemelo
    /// `financial_health.basis` hacen. Regla de lectura: si `financial_health.basis` no dice
    /// `"plan"`, esas cuatro cifras y éstas NO son comparables uno a uno.
    pub basis: &'static str,
    #[serde(with = "rust_decimal::serde::str")]
    #[schema(value_type = String)]
    pub income_monthly_equivalent: Decimal,
    /// Sum of income entries with `persists_after_retirement = true`.
    #[serde(with = "rust_decimal::serde::str")]
    #[schema(value_type = String)]
    pub income_retirement_monthly_equivalent: Decimal,
    /// Gasto mensual del plan: partidas persistidas **+ cuotas de pasivos activos**. Desde la
    /// 3.7.0 las cuotas son una partida más de `entries`, así que este total es exactamente la
    /// suma de los `entries` de scope `expense` — antes excluía las cuotas, que se sumaban
    /// aparte en el desaparecido `expense_derived_monthly_equivalent`.
    #[serde(with = "rust_decimal::serde::str")]
    #[schema(value_type = String)]
    pub expense_regular_monthly_equivalent: Decimal,
    /// Suma de las partidas de gasto que continúan tras la jubilación (`ends_at_retirement =
    /// false`). Cuenta **solo partidas persistidas**: una cuota de pasivo tiene fecha de fin de
    /// plan, así que por definición no es gasto post-jubilación. Es el campo que consume el
    /// cálculo FIRE (`RetirementView`) — no lo confundas con `expense_regular_*` (incidente de
    /// la divergencia 2–3× en la previa del formulario, v1.3.0).
    #[serde(with = "rust_decimal::serde::str")]
    #[schema(value_type = String)]
    pub expense_retirement_monthly_equivalent: Decimal,
    /// Idéntico a `expense_regular_monthly_equivalent` desde la 3.7.0 (ya no hay una componente
    /// derivada que sumarle). Se mantiene por compatibilidad de contrato.
    #[serde(with = "rust_decimal::serde::str")]
    #[schema(value_type = String)]
    pub expense_total_monthly_equivalent: Decimal,
    #[serde(with = "rust_decimal::serde::str")]
    #[schema(value_type = String)]
    pub net_monthly_equivalent: Decimal,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct BudgetSnapshotResponse {
    /// Vista efectivamente aplicada: `household` | `mine`. Eco de `?view` — ver
    /// `SummaryResponse::view` para el porqué (en una instalación de un solo usuario las dos
    /// respuestas eran byte a byte idénticas).
    pub view: &'static str,
    /// Partidas del presupuesto: las persistidas (`source = manual`) **y** las cuotas de los
    /// pasivos activos (`source = liability`, solo lectura, atribuidas a su categoría de gasto).
    pub entries: Vec<BudgetEntryResponse>,
    pub totals: BudgetTotalsResponse,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateBudgetEntryBody {
    #[schema(value_type = String, format = "uuid")]
    pub category_id: Uuid,
    #[serde(with = "rust_decimal::serde::str")]
    #[schema(value_type = String)]
    pub amount: Decimal,
    #[serde(default)]
    pub notes: Option<String>,
    #[serde(default)]
    pub sort_index: Option<i32>,
    #[serde(default)]
    pub persists_after_retirement: bool,
    #[serde(default)]
    pub ends_at_retirement: bool,
    #[serde(default)]
    pub expense_end_date: Option<NaiveDate>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct PatchBudgetEntryBody {
    #[serde(default)]
    #[schema(value_type = Option<String>, format = "uuid")]
    pub category_id: Option<Uuid>,
    #[serde(default)]
    #[serde(with = "rust_decimal::serde::str_option")]
    #[schema(value_type = Option<String>)]
    pub amount: Option<Decimal>,
    #[serde(default)]
    pub notes: Option<String>,
    #[serde(default)]
    pub sort_index: Option<i32>,
    #[serde(default)]
    pub persists_after_retirement: Option<bool>,
    #[serde(default)]
    pub ends_at_retirement: Option<bool>,
    #[serde(default)]
    pub expense_end_date: Option<NaiveDate>,
    /// Set to `true` to explicitly clear `expense_end_date` to NULL. Mutually exclusive with
    /// `expense_end_date`: enviar los dos es 400 `expense_end_set_and_clear`, no «gana el clear».
    #[serde(default)]
    pub clear_expense_end_date: Option<bool>,
}

#[derive(Debug, FromRow)]
pub(crate) struct BudgetEntryJoinRow {
    id: Uuid,
    category_id: Uuid,
    scope: String,
    amount: Decimal,
    notes: Option<String>,
    sort_index: i32,
    persists_after_retirement: bool,
    ends_at_retirement: bool,
    expense_end_date: Option<NaiveDate>,
    /// `NOT NULL` desde la migración 5.0.0 (D14). Lo lee la puerta de D21 del PATCH.
    owner_user_id: Uuid,
}

#[derive(Debug, Clone, FromRow)]
pub(crate) struct LiabilityDerivedRow {
    id: Uuid,
    expense_category_id: Option<Uuid>,
    label: String,
    payment_amount: Decimal,
    payment_frequency: String,
    payment_end_date: Option<NaiveDate>,
}

pub(crate) fn monthly_equivalent(amount: Decimal, frequency: &str) -> Decimal {
    match frequency.trim() {
        "weekly" => (amount * Decimal::from(52u32)) / Decimal::from(12u32),
        _ => amount,
    }
}

fn scope_to_budget_enum(scope: &str) -> Result<BudgetCategoryScope, ApiError> {
    match scope {
        "income" => Ok(BudgetCategoryScope::Income),
        "expense" => Ok(BudgetCategoryScope::Expense),
        _ => Err(ApiError::BadRequest(
            "budget_entry_category_scope_unsupported: budget entry category must be income or expense scope".into(),
        )),
    }
}

fn row_to_entry_response(r: BudgetEntryJoinRow) -> Result<BudgetEntryResponse, ApiError> {
    let scope = scope_to_budget_enum(&r.scope)?;
    Ok(BudgetEntryResponse {
        id: r.id,
        category_id: Some(r.category_id),
        scope,
        source: BudgetEntrySource::Manual,
        liability_id: None,
        label: None,
        amount: r.amount,
        notes: r.notes,
        sort_index: r.sort_index,
        persists_after_retirement: r.persists_after_retirement,
        ends_at_retirement: r.ends_at_retirement,
        expense_end_date: r.expense_end_date,
    })
}

/// Cuota de un pasivo activo publicada como partida de gasto del presupuesto (3.7.0).
///
/// `category_id` es la **categoría de gasto** que declara el pasivo (`expense_category_id`), la
/// misma con la que la comparativa de Movimientos empareja los recibos reales — por eso la cuota
/// cae junto a las partidas manuales de esa categoría en vez de en un bloque aparte. Queda `None`
/// en pasivos anteriores a la 3.4.0 (y en los importados de backups viejos): esas cuotas siguen
/// sumando en los totales, y la UI las marca «sin categoría de cuota» hasta que se les asigne una
/// — nunca se descartan, porque hacerlo bajaría el gasto presupuestado en silencio.
///
/// `sort_index = 0` y las banderas de jubilación a `false` son valores fijos: una cuota no es
/// ordenable ni marcable por el usuario, y su fin lo fija el plan de pago (`expense_end_date`).
fn liability_row_to_entry(d: LiabilityDerivedRow) -> Result<BudgetEntryResponse, ApiError> {
    // Misma validación que antes de la fusión: una frecuencia desconocida es un 400 explícito,
    // no una fila muda que descuadra el total.
    PaymentFrequency::parse(&d.payment_frequency)?;
    Ok(BudgetEntryResponse {
        id: d.id,
        category_id: d.expense_category_id,
        scope: BudgetCategoryScope::Expense,
        source: BudgetEntrySource::Liability,
        liability_id: Some(d.id),
        label: Some(d.label),
        amount: monthly_equivalent(d.payment_amount, &d.payment_frequency),
        notes: None,
        sort_index: 0,
        persists_after_retirement: false,
        ends_at_retirement: false,
        expense_end_date: d.payment_end_date,
    })
}

fn normalize_notes(raw: &Option<String>) -> Result<Option<String>, ApiError> {
    match raw {
        None => Ok(None),
        Some(s) => {
            let t = s.trim();
            if t.is_empty() {
                return Ok(None);
            }
            if t.len() > 4000 {
                return Err(ApiError::BadRequest(
                    "notes_too_long: notes must be at most 4000 characters".into(),
                ));
            }
            Ok(Some(t.into()))
        }
    }
}

pub(crate) async fn assert_budget_category(
    pool: &sqlx::PgPool,
    installation_id: Uuid,
    category_id: Uuid,
) -> Result<String, ApiError> {
    let scope: Option<String> = sqlx::query_scalar(
        r#"SELECT scope FROM categories
           WHERE id = $1 AND installation_id = $2"#,
    )
    .bind(category_id)
    .bind(installation_id)
    .fetch_optional(pool)
    .await?;

    let Some(s) = scope else {
        return Err(ApiError::BadRequest(
            "category_not_found: category_id must reference a category in this installation".into(),
        ));
    };

    if !matches!(s.as_str(), "income" | "expense") {
        return Err(ApiError::BadRequest(
            "category_wrong_scope: budget entries must use a category with scope income or expense".into(),
        ));
    }

    Ok(s)
}

pub(crate) async fn fetch_budget_rows_and_derived_liabilities(
    pool: &sqlx::PgPool,
    iid: Uuid,
    session_user_id: Uuid,
    view: LedgerView,
    today: NaiveDate,
) -> Result<(Vec<BudgetEntryJoinRow>, Vec<LiabilityDerivedRow>), ApiError> {
    let entries_scope = view.scope_where("b");
    let entries_sql = format!(
        r#"SELECT b.id, b.category_id, c.scope AS scope, b.amount,
                  b.notes, b.sort_index, b.persists_after_retirement,
                  b.ends_at_retirement, b.expense_end_date, b.owner_user_id
           FROM budget_entries b
           JOIN categories c ON c.id = b.category_id
           WHERE {entries_scope}
           ORDER BY b.sort_index ASC, b.id ASC"#
    );
    let rows: Vec<BudgetEntryJoinRow> = view
        .bind_scope_as(sqlx::query_as(&entries_sql), iid, session_user_id)
        .fetch_all(pool)
        .await?;

    // Predicado de «pasivo activo» unificado con el resto del sistema (liabilities.rs, summary.rs,
    // projection.rs, history.rs): fecha fin NULL = plan indefinido, sigue activo; borde `>=` (el
    // día exacto de fin aún cuenta). Hasta la 3.4.0 esta query era el único outlier (exigía
    // NOT NULL y `>` estricto): un pasivo sin fecha fin no generaba línea derivada aunque el
    // engine sí cobrara su cuota — el modo A no lo contaba «una vez» de forma consistente y el
    // KPI «real vs esperado» del Resumen descuadraba exactamente en esa cuota.
    let derived_scope = view.scope_where("");
    let today_ph = view.next_arg_index();
    let derived_sql = format!(
        r#"SELECT id, expense_category_id, label, payment_amount, payment_frequency,
                  payment_end_date
           FROM liabilities
           WHERE {derived_scope}
             AND payment_amount IS NOT NULL
             AND payment_frequency IS NOT NULL
             AND (payment_end_date IS NULL OR payment_end_date >= ${today_ph})"#
    );
    let derived_raw: Vec<LiabilityDerivedRow> = view
        .bind_scope_as(sqlx::query_as(&derived_sql), iid, session_user_id)
        .bind(today)
        .fetch_all(pool)
        .await?;

    Ok((rows, derived_raw))
}

pub(crate) fn ledger_budget_totals_from_parts(
    rows: &[BudgetEntryJoinRow],
    derived_raw: &[LiabilityDerivedRow],
    today: NaiveDate,
) -> Result<BudgetTotalsResponse, ApiError> {
    let mut income_m = Decimal::ZERO;
    let mut income_retirement_m = Decimal::ZERO;
    let mut expense_reg = Decimal::ZERO;
    let mut expense_retirement_m = Decimal::ZERO;

    let month_first_today =
        NaiveDate::from_ymd_opt(today.year(), today.month(), 1).unwrap_or(today);
    for r in rows {
        let me = r.amount;
        match r.scope.as_str() {
            "income" => {
                income_m += me;
                if r.persists_after_retirement {
                    income_retirement_m += me;
                }
            }
            "expense" => {
                // #124: la partida vencida no se paga — fuera de los totales (misma regla y
                // misma granularidad de mes que el motor y que el resolver de la proyección).
                // La FILA sigue visible en `entries` con su `expense_end_date` para que el
                // usuario la vea, edite o borre; solo los agregados dejan de sumarla.
                if matches!(r.expense_end_date, Some(end) if month_first_today > end) {
                    continue;
                }
                expense_reg += me;
                if !r.ends_at_retirement {
                    expense_retirement_m += me;
                }
            }
            _ => {}
        }
    }

    // Las cuotas de los pasivos activos son gasto del plan como cualquier otra partida: se suman
    // DENTRO de `expense_reg`, no como una componente aparte. `expense_retirement_m` no las
    // recibe a propósito (una cuota termina con su plan; ver el doc del campo).
    for d in derived_raw {
        PaymentFrequency::parse(&d.payment_frequency)?;
        expense_reg += monthly_equivalent(d.payment_amount, &d.payment_frequency);
    }

    let net = income_m - expense_reg;

    Ok(BudgetTotalsResponse {
        basis: BUDGET_TOTALS_BASIS,
        income_monthly_equivalent: income_m,
        income_retirement_monthly_equivalent: income_retirement_m,
        expense_regular_monthly_equivalent: expense_reg,
        expense_retirement_monthly_equivalent: expense_retirement_m,
        expense_total_monthly_equivalent: expense_reg,
        net_monthly_equivalent: net,
    })
}

/// La partida de presupuesto que DESAPARECE al borrar un pasivo, con el efecto exacto sobre los
/// totales del hogar.
///
/// Existe porque el preview de `delete_liability` engañaba por omisión: contaba los movimientos
/// que quedan desvinculados y callaba que, tras confirmar, la cuota derivada se va de
/// `GET /v1/budget` y el gasto mensual presupuestado baja. En una hipoteca son cientos de euros
/// al mes; quien confirma el borrado tiene derecho a verlos ANTES.
#[derive(Debug, serde::Serialize)]
pub struct BudgetLineRemoved {
    /// Categoría de gasto donde se sentaba la cuota (`None` en pasivos sin `expense_category_id`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub category_id: Option<Uuid>,
    /// Etiqueta del pasivo, tal y como aparece hoy en el presupuesto.
    pub label: String,
    /// Equivalente MENSUAL de la cuota (una cuota semanal ya viene convertida ×52/12).
    #[serde(with = "rust_decimal::serde::str")]
    pub monthly_amount: Decimal,
    #[serde(with = "rust_decimal::serde::str")]
    pub expense_monthly_before: Decimal,
    #[serde(with = "rust_decimal::serde::str")]
    pub expense_monthly_after: Decimal,
    #[serde(with = "rust_decimal::serde::str")]
    pub net_monthly_before: Decimal,
    #[serde(with = "rust_decimal::serde::str")]
    pub net_monthly_after: Decimal,
}

/// Calcula [`BudgetLineRemoved`] para un pasivo. `None` cuando el pasivo **no genera** cuota
/// derivada (sin plan de pago, o con `payment_end_date` ya pasada): entonces borrarlo no mueve
/// ni un euro del presupuesto, y decir «desaparece una partida» sería mentir en la otra dirección.
///
/// Los totales de después se **recomputan** con la misma función que los de antes, quitando la
/// fila derivada del conjunto — no se restan a mano. Restar daría hoy el mismo número, pero
/// dejaría el preview mintiendo el día que una cuota contribuya a algún total de otra manera.
///
/// Vista **household** a propósito: borrar un pasivo lo borra para todo el hogar, así que el
/// efecto que hay que enseñar es el del hogar, no el del solicitante.
pub(crate) async fn budget_line_removed_with_liability(
    pool: &sqlx::PgPool,
    iid: Uuid,
    liability_id: Uuid,
) -> Result<Option<BudgetLineRemoved>, ApiError> {
    let today = installation_naive_today(pool, iid).await?;
    // `LedgerView::Household` ignora el `session_user_id` en `bind_scope_as` (ver person_view.rs),
    // así que el nil no llega a ningún bind: no hay identidad que pasar.
    let (rows, derived) = fetch_budget_rows_and_derived_liabilities(
        pool,
        iid,
        Uuid::nil(),
        LedgerView::Household,
        today,
    )
    .await?;

    let Some(target) = derived.iter().find(|d| d.id == liability_id).cloned() else {
        return Ok(None);
    };
    let before = ledger_budget_totals_from_parts(&rows, &derived, today)?;
    let after_derived: Vec<LiabilityDerivedRow> = derived
        .into_iter()
        .filter(|d| d.id != liability_id)
        .collect();
    let after = ledger_budget_totals_from_parts(&rows, &after_derived, today)?;

    Ok(Some(BudgetLineRemoved {
        category_id: target.expense_category_id,
        label: target.label,
        monthly_amount: money_out(monthly_equivalent(
            target.payment_amount,
            &target.payment_frequency,
        )),
        expense_monthly_before: money_out(before.expense_regular_monthly_equivalent),
        expense_monthly_after: money_out(after.expense_regular_monthly_equivalent),
        net_monthly_before: money_out(before.net_monthly_equivalent),
        net_monthly_after: money_out(after.net_monthly_equivalent),
    }))
}

/// ¿Este id es un **pasivo** de la instalación en vez de una partida de presupuesto?
///
/// Las cuotas derivadas se publican en `GET /v1/budget` con el **UUID de su pasivo**
/// (`source = "liability"`), así que un id recogido de ahí y pasado a
/// `PATCH`/`DELETE /v1/budget/entries/{id}` apunta a un pasivo. Hasta ahora eso era un 404 pelado
/// —«no existe»— justo sobre un id que el llamante acababa de leer en una respuesta nuestra: el
/// error decía lo contrario de la verdad y no señalaba a dónde ir.
async fn id_is_liability(pool: &sqlx::PgPool, iid: Uuid, id: Uuid) -> Result<bool, ApiError> {
    Ok(sqlx::query_scalar(
        r#"SELECT EXISTS(SELECT 1 FROM liabilities WHERE id = $1 AND installation_id = $2)"#,
    )
    .bind(id)
    .bind(iid)
    .fetch_one(pool)
    .await?)
}

/// Traduce «no hay partida con ese id» a un error que distinga las dos causas: un id inventado
/// (404) o el id de un pasivo cuya cuota derivada NO se edita aquí (422, con el sitio al que ir).
///
/// Solo se llama en el camino de error, así que el coste de la consulta extra es cero en la ruta
/// buena. Mensaje literal COMPLETO: `error_codes_parity` extrae el código del literal del fuente.
async fn missing_budget_entry_error(pool: &sqlx::PgPool, iid: Uuid, id: Uuid) -> ApiError {
    match id_is_liability(pool, iid, id).await {
        Ok(true) => ApiError::Unprocessable(
            "budget_entry_is_liability_derived: this id is a liability, not a budget entry — its budget line is derived from the liability's payment plan; edit or remove it with update_liability / delete_liability (PATCH or DELETE /v1/liabilities/{id})".into(),
        ),
        // Un fallo del SELECT no debe inventar un 422: se cae al 404 de siempre.
        Ok(false) | Err(_) => ApiError::NotFound,
    }
}

/// Los mismos totales que `GET /v1/budget` (partidas persistidas + cuotas de pasivos activos,
/// ya fundidas dentro de `expense_regular_monthly_equivalent`).
pub(crate) async fn ledger_budget_totals_for_summary(
    pool: &sqlx::PgPool,
    iid: Uuid,
    session_user_id: Uuid,
    view: LedgerView,
    today: NaiveDate,
) -> Result<BudgetTotalsResponse, ApiError> {
    let (rows, derived_raw) =
        fetch_budget_rows_and_derived_liabilities(pool, iid, session_user_id, view, today).await?;
    ledger_budget_totals_from_parts(&rows, &derived_raw, today)
}

/// Persisted budget rows only (no liability-derived lines), for projection / FIRE expense bases.
///
/// **No fundas aquí las cuotas de pasivo.** La fusión de la 3.7.0 es de presentación y de totales
/// de `/v1/budget`; el engine cobra la cuota por su lado desde `ProjectionLiabilityInput`
/// (`monthly_payment`, con amortización y fecha de fin de plan), así que sumarla también a esta
/// base la contaría dos veces en toda la proyección del modo A — en silencio, que es el modo de
/// fallo caro de este repo. Clavado por `apps/api/tests/budget_derived.rs`
/// (`liability_quota_stays_out_of_the_engine_expense_base`).
///
/// Returns `(income_reg, income_retirement, expense_reg, expense_retirement, expense_end_entries)`:
/// - `income_retirement`: sum of income entries with `persists_after_retirement = true`.
/// - `expense_retirement`: sum of expense entries with `ends_at_retirement = false` (continue after retirement).
/// - `expense_end_entries`: `(amount, end_date)` pairs for expense entries with an explicit `expense_end_date`.
pub(crate) async fn ledger_regular_monthly_income_and_expense(
    pool: &sqlx::PgPool,
    iid: Uuid,
    session_user_id: Uuid,
    view: LedgerView,
    today: NaiveDate,
) -> Result<(Decimal, Decimal, Decimal, Decimal, Vec<(Decimal, NaiveDate)>), ApiError> {
    let (rows, _) =
        fetch_budget_rows_and_derived_liabilities(pool, iid, session_user_id, view, today).await?;
    let mut income_m = Decimal::ZERO;
    let mut income_retirement_m = Decimal::ZERO;
    let mut expense_reg = Decimal::ZERO;
    let mut expense_retirement_m = Decimal::ZERO;
    let mut expense_end_entries: Vec<(Decimal, NaiveDate)> = Vec::new();
    for r in rows {
        let me = r.amount;
        match r.scope.as_str() {
            "income" => {
                income_m += me;
                if r.persists_after_retirement {
                    income_retirement_m += me;
                }
            }
            "expense" => {
                // #124 (4.8.0): una partida cuyo `expense_end_date` ya venció NO se paga — no
                // entra en el gasto regular, ni en el de jubilación, ni (crucial) en las
                // entradas de cancelación del motor. El predicado es el MISMO del motor
                // (`expense_end_date_monthly_adjustments`: cancela cuando `m_first > end`),
                // a granularidad de MES: filtrar solo del sumatorio y dejar la entrada
                // compensadora habría creado caja fantasma de +importe/mes desde el mes 0.
                let month_first_today =
                    NaiveDate::from_ymd_opt(today.year(), today.month(), 1).unwrap_or(today);
                let already_ended =
                    matches!(r.expense_end_date, Some(end) if month_first_today > end);
                if already_ended {
                    continue;
                }
                expense_reg += me;
                if !r.ends_at_retirement {
                    expense_retirement_m += me;
                }
                if let Some(end_date) = r.expense_end_date {
                    expense_end_entries.push((me, end_date));
                }
            }
            _ => {}
        }
    }
    Ok((income_m, income_retirement_m, expense_reg, expense_retirement_m, expense_end_entries))
}

#[utoipa::path(
    get,
    path = "/v1/budget",
    tag = "budget",
    params(
        ("view" = Option<String>, Query, description = "`mine` (default: `view` omitido o vacío) = filas atribuidas al usuario de la sesión; `household` = hogar completo, y hay que pedirlo EXPLÍCITAMENTE desde 5.0.0. Cualquier otro valor → 400 `invalid_view`."),
    ),
    responses(
        (status = 200, description = "Presupuesto: partidas persistidas y cuotas de pasivos activos en una sola lista (`entries`, discriminadas por `source`), con los totales normalizados a equivalente mensual.", body = BudgetSnapshotResponse),
        (status = 401, description = "No valid session"),
        (status = 403, description = "Not an installation member"),
        (status = 404, description = "Installation missing"),
    )
)]
pub async fn get_budget_snapshot(
    Extension(state): Extension<Arc<AppState>>,
    jar: CookieJar,
    Query(q): Query<LedgerViewQuery>,
) -> Result<Json<BudgetSnapshotResponse>, ApiError> {
    let user = require_session_user(&jar, &state.pool).await?;
    let (iid, _) = require_installation_member(&state.pool, user.id.0).await?;
    let out = budget_snapshot_core(&state.pool, iid, user.id.0, q.resolve()?).await?;
    Ok(Json(out))
}

/// Core sin HTTP: lo comparten el handler GET y la tool MCP `get_budget`.
pub(crate) async fn budget_snapshot_core(
    pool: &sqlx::PgPool,
    iid: Uuid,
    user_id: Uuid,
    view: LedgerView,
) -> Result<BudgetSnapshotResponse, ApiError> {
    let today = installation_naive_today(pool, iid).await?;

    let (rows, derived_raw) = fetch_budget_rows_and_derived_liabilities(
        pool,
        iid,
        user_id,
        view,
        today,
    )
    .await?;

    let totals = ledger_budget_totals_from_parts(&rows, &derived_raw, today)?;

    let mut entries = Vec::with_capacity(rows.len() + derived_raw.len());
    for r in rows {
        entries.push(row_to_entry_response(r)?);
    }

    for d in derived_raw {
        entries.push(liability_row_to_entry(d)?);
    }

    Ok(BudgetSnapshotResponse {
        view: view.as_str(),
        entries,
        totals,
    })
}

#[utoipa::path(
    post,
    path = "/v1/budget/entries",
    tag = "budget",
    request_body = CreateBudgetEntryBody,
    responses(
        (status = 201, description = "Created", body = BudgetEntryResponse),
        (status = 400, description = "Validation error"),
        (status = 401, description = "No valid session"),
        (status = 403, description = "Viewer or not a member"),
        (status = 404, description = "Installation missing"),
    )
)]
pub async fn create_budget_entry(
    Extension(state): Extension<Arc<AppState>>,
    jar: CookieJar,
    Json(body): Json<CreateBudgetEntryBody>,
) -> Result<(axum::http::StatusCode, Json<BudgetEntryResponse>), ApiError> {
    let user = require_session_user(&jar, &state.pool).await?;
    let (iid, role) = require_installation_member(&state.pool, user.id.0).await?;
    if !role_can_write(role.as_str()) {
        return Err(ApiError::Forbidden);
    }
    let resp = create_budget_entry_core(&state, iid, user.id.0, body).await?;
    Ok((axum::http::StatusCode::CREATED, Json(resp)))
}

/// Core sin HTTP: lo comparten el handler POST y la tool MCP `create_budget_entry`. En modo A
/// el budget es la fuente del ahorro proyectado → invalidación FULL dentro.
pub(crate) async fn create_budget_entry_core(
    state: &Arc<AppState>,
    iid: Uuid,
    user_id: Uuid,
    body: CreateBudgetEntryBody,
) -> Result<BudgetEntryResponse, ApiError> {
    assert_budget_category(&state.pool, iid, body.category_id).await?;

    if body.amount <= Decimal::ZERO {
        return Err(ApiError::BadRequest(
            "amount_not_positive: amount must be greater than zero".into(),
        ));
    }

    if body.ends_at_retirement && body.expense_end_date.is_some() {
        return Err(ApiError::BadRequest(
            "end_condition_conflict: ends_at_retirement and expense_end_date are mutually exclusive".into(),
        ));
    }

    let notes = normalize_notes(&body.notes)?;
    let sort_index = body.sort_index.unwrap_or(0);

    let id: Uuid = sqlx::query_scalar(
        r#"INSERT INTO budget_entries (
               installation_id, category_id, amount, notes, sort_index,
               owner_user_id, persists_after_retirement,
               ends_at_retirement, expense_end_date
           )
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
           RETURNING id"#,
    )
    .bind(iid)
    .bind(body.category_id)
    .bind(body.amount)
    .bind(&notes)
    .bind(sort_index)
    .bind(user_id)
    .bind(body.persists_after_retirement)
    .bind(body.ends_at_retirement)
    .bind(body.expense_end_date)
    .fetch_one(&state.pool)
    .await?;

    let row: BudgetEntryJoinRow = sqlx::query_as(
        r#"SELECT b.id, b.category_id, c.scope AS scope, b.amount,
                  b.notes, b.sort_index, b.persists_after_retirement,
                  b.ends_at_retirement, b.expense_end_date, b.owner_user_id
           FROM budget_entries b
           JOIN categories c ON c.id = b.category_id
           WHERE b.id = $1"#,
    )
    .bind(id)
    .fetch_one(&state.pool)
    .await?;

    refresh_projection_after_mutation(&state, iid, user_id).await;
    row_to_entry_response(row)
}

#[utoipa::path(
    patch,
    path = "/v1/budget/entries/{id}",
    tag = "budget",
    request_body = PatchBudgetEntryBody,
    params(
        ("id" = Uuid, Path, description = "Budget entry id"),
    ),
    responses(
        (status = 200, description = "Updated", body = BudgetEntryResponse),
        (status = 400, description = "Validation error"),
        (status = 401, description = "No valid session"),
        (status = 403, description = "Viewer or not a member"),
        (status = 404, description = "Entry missing"),
        (status = 422, description = "El id es un pasivo: su partida es derivada y se edita en /v1/liabilities (`budget_entry_is_liability_derived`)"),
    )
)]
pub async fn patch_budget_entry(
    Extension(state): Extension<Arc<AppState>>,
    jar: CookieJar,
    Path(id): Path<Uuid>,
    Json(body): Json<PatchBudgetEntryBody>,
) -> Result<Json<BudgetEntryResponse>, ApiError> {
    let user = require_session_user(&jar, &state.pool).await?;
    let (iid, role) = require_installation_member(&state.pool, user.id.0).await?;
    if !role_can_write(role.as_str()) {
        return Err(ApiError::Forbidden);
    }
    let resp = patch_budget_entry_core(&state, iid, user.id.0, id, body).await?;
    Ok(Json(resp))
}

/// Core sin HTTP: lo comparten el handler PATCH y la tool MCP `update_budget_entry`.
/// Exclusión mutua `ends_at_retirement` ⊕ `expense_end_date` re-verificada tras el merge, y
/// exclusión mutua `expense_end_date` ⊕ `clear_expense_end_date` sobre el body (4.3.1+).
pub(crate) async fn patch_budget_entry_core(
    state: &Arc<AppState>,
    iid: Uuid,
    user_id: Uuid,
    id: Uuid,
    body: PatchBudgetEntryBody,
) -> Result<BudgetEntryResponse, ApiError> {
    if body.category_id.is_none()
        && body.amount.is_none()
        && body.notes.is_none()
        && body.sort_index.is_none()
        && body.persists_after_retirement.is_none()
        && body.ends_at_retirement.is_none()
        && body.expense_end_date.is_none()
        && body.clear_expense_end_date.is_none()
    {
        return Err(ApiError::BadRequest(
            "patch_empty: provide at least one field to update".into(),
        ));
    }

    // Poner y borrar el MISMO campo en la MISMA llamada: error, no «gana el clear». Hasta 4.3.1
    // esta core dejaba ganar al `clear` en silencio (200 con `expense_end_date` a NULL) y la
    // guardia vivía SOLO en la tool MCP `update_budget_entry`: la superficie **derivada** era más
    // estricta que la fuente, que es exactamente lo contrario del contrato D14. Por HTTP/SPA un
    // patch armado desde plantilla (fecha + flag) borraba la fecha fin de la partida sin avisar, y
    // esa fecha es la que corta el gasto en la proyección: los totales siguen cuadrando y el
    // horizonte del gasto miente. La guardia va AQUÍ para que HTTP y MCP compartan la misma
    // respuesta, y la tool ya no necesita la suya.
    //
    // Va junto al `patch_empty` (antes del SELECT) porque este módulo NO tiene owner-guard: aquí
    // un 404 no oculta nada, así que validar la forma del body primero es lo coherente con el
    // orden que el propio fichero ya usaba. (`patch_transaction_core` la pone tras el SELECT
    // porque allí el SELECT ES el owner-guard y el 404 debe ganar.)
    //
    // Mensaje literal COMPLETO, jamás compuesto con `format!`: `error_codes_parity.rs` extrae los
    // códigos estables de los literales del fuente, y un código ensamblado en trozos queda
    // invisible para el catálogo y degrada al mensaje genérico de la SPA. Misma nota que en
    // `patch_transaction_core`; costó descubrirlo una vez.
    if body.expense_end_date.is_some() && body.clear_expense_end_date == Some(true) {
        return Err(ApiError::BadRequest(
            "expense_end_set_and_clear: expense_end_date and clear_expense_end_date are mutually exclusive".into(),
        ));
    }

    let row: Option<BudgetEntryJoinRow> = sqlx::query_as(
        r#"SELECT b.id, b.category_id, c.scope AS scope, b.amount,
                  b.notes, b.sort_index, b.persists_after_retirement,
                  b.ends_at_retirement, b.expense_end_date, b.owner_user_id
           FROM budget_entries b
           JOIN categories c ON c.id = b.category_id
           WHERE b.id = $1 AND b.installation_id = $2"#,
    )
    .bind(id)
    .bind(iid)
    .fetch_optional(&state.pool)
    .await?;

    let Some(current) = row else {
        return Err(missing_budget_entry_error(&state.pool, iid, id).await);
    };
    require_row_owner(current.owner_user_id, user_id)?;

    let new_cat = body.category_id.unwrap_or(current.category_id);
    if new_cat != current.category_id {
        assert_budget_category(&state.pool, iid, new_cat).await?;
    }

    let new_amount = match body.amount {
        Some(a) => {
            if a <= Decimal::ZERO {
                return Err(ApiError::BadRequest(
                    "amount_not_positive: amount must be greater than zero".into(),
                ));
            }
            a
        }
        None => current.amount,
    };

    let new_notes = match &body.notes {
        Some(_) => normalize_notes(&body.notes)?,
        None => current.notes.clone(),
    };

    let new_sort = body.sort_index.unwrap_or(current.sort_index);
    let new_persists = body.persists_after_retirement.unwrap_or(current.persists_after_retirement);
    let new_ends = body.ends_at_retirement.unwrap_or(current.ends_at_retirement);
    let new_expense_end_date = if body.clear_expense_end_date == Some(true) {
        None
    } else {
        body.expense_end_date.or(current.expense_end_date)
    };

    if new_ends && new_expense_end_date.is_some() {
        return Err(ApiError::BadRequest(
            "end_condition_conflict: ends_at_retirement and expense_end_date are mutually exclusive".into(),
        ));
    }

    let updated: BudgetEntryJoinRow = sqlx::query_as(
        r#"UPDATE budget_entries
           SET category_id = $1,
               amount = $2,
               notes = $3,
               sort_index = $4,
               persists_after_retirement = $5,
               ends_at_retirement = $6,
               expense_end_date = $7,
               updated_at = now()
           WHERE id = $8 AND installation_id = $9 AND owner_user_id = $10
           RETURNING budget_entries.id,
                     budget_entries.category_id,
                     (
                         SELECT c.scope
                         FROM categories c
                         WHERE c.id = budget_entries.category_id
                     ) AS scope,
                     budget_entries.amount,
                     budget_entries.notes,
                     budget_entries.sort_index,
                     budget_entries.persists_after_retirement,
                     budget_entries.ends_at_retirement,
                     budget_entries.expense_end_date,
                     budget_entries.owner_user_id"#,
    )
    .bind(new_cat)
    .bind(new_amount)
    .bind(&new_notes)
    .bind(new_sort)
    .bind(new_persists)
    .bind(new_ends)
    .bind(new_expense_end_date)
    .bind(id)
    .bind(iid)
    .bind(user_id)
    .fetch_one(&state.pool)
    .await?;

    refresh_projection_after_mutation(&state, iid, user_id).await;
    row_to_entry_response(updated)
}

#[utoipa::path(
    delete,
    path = "/v1/budget/entries/{id}",
    tag = "budget",
    params(
        ("id" = Uuid, Path, description = "Budget entry id"),
    ),
    responses(
        (status = 204, description = "Deleted"),
        (status = 401, description = "No valid session"),
        (status = 403, description = "Viewer or not a member"),
        (status = 404, description = "Entry missing"),
        (status = 422, description = "El id es un pasivo: su partida es derivada y se edita en /v1/liabilities (`budget_entry_is_liability_derived`)"),
    )
)]
pub async fn delete_budget_entry(
    Extension(state): Extension<Arc<AppState>>,
    jar: CookieJar,
    Path(id): Path<Uuid>,
) -> Result<axum::http::StatusCode, ApiError> {
    let user = require_session_user(&jar, &state.pool).await?;
    let (iid, role) = require_installation_member(&state.pool, user.id.0).await?;
    if !role_can_write(role.as_str()) {
        return Err(ApiError::Forbidden);
    }

    delete_budget_entry_core(&state, iid, user.id.0, id).await?;
    Ok(axum::http::StatusCode::NO_CONTENT)
}

/// Core sin HTTP: lo comparten el handler DELETE y la tool MCP `delete_budget_entry`.
pub(crate) async fn delete_budget_entry_core(
    state: &Arc<AppState>,
    iid: Uuid,
    user_id: Uuid,
    id: Uuid,
) -> Result<(), ApiError> {
    // D21: el «no existe» de este módulo NO es un 404 pelado —puede ser el 422 que manda a
    // /v1/liabilities—, así que la puerta de dueño se resuelve con un SELECT propio y deja
    // intacto `missing_budget_entry_error` para el caso de verdad ausente.
    let owner: Option<Uuid> = sqlx::query_scalar(
        r#"SELECT owner_user_id FROM budget_entries WHERE id = $1 AND installation_id = $2"#,
    )
    .bind(id)
    .bind(iid)
    .fetch_optional(&state.pool)
    .await?;
    match owner {
        None => return Err(missing_budget_entry_error(&state.pool, iid, id).await),
        Some(o) => require_row_owner(o, user_id)?,
    }

    let res = sqlx::query(
        r#"DELETE FROM budget_entries
           WHERE id = $1 AND installation_id = $2 AND owner_user_id = $3"#,
    )
    .bind(id)
    .bind(iid)
    .bind(user_id)
    .execute(&state.pool)
    .await?;

    if res.rows_affected() == 0 {
        return Err(missing_budget_entry_error(&state.pool, iid, id).await);
    }

    refresh_projection_after_mutation(&state, iid, user_id).await;
    Ok(())
}

pub fn budget_router() -> Router {
    Router::new()
        .route("/", get(get_budget_snapshot))
        .route("/entries", axum::routing::post(create_budget_entry))
        .route(
            "/entries/{id}",
            patch(patch_budget_entry).delete(delete_budget_entry),
        )
}
