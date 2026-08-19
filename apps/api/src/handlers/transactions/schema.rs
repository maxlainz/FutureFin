//! DTOs (Decimal siempre como string) y helpers puros del módulo `transactions`:
//! normalización de concepto, huella de dedup, derivación de patrón de regla y validación
//! de `kind`. Sin I/O: los validadores que tocan la BD viven en `mod.rs`.

use crate::error::ApiError;
use chrono::{DateTime, NaiveDate, Utc};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

/// `source` de un movimiento metido a mano (efectivo): no viene de ningún CSV.
pub const SOURCE_MANUAL: &str = "manual";

/// Separador de campos de la huella: ASCII Unit Separator (`\u{1f}`), imposible en un concepto.
const FINGERPRINT_SEP: char = '\u{1f}';

// ---------------------------------------------------------------------------
// Helpers puros
// ---------------------------------------------------------------------------

/// Normaliza un concepto para matching de reglas y para la huella de dedup:
/// trim + colapsar runs de whitespace a un único espacio + ASCII-uppercase. Los acentos
/// (no-ASCII) quedan intactos (`Café` → `CAFÉ`, no `CAFE`).
pub fn normalize_concept(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut last_was_space = false;
    for ch in raw.trim().chars() {
        if ch.is_whitespace() {
            if !last_was_space {
                out.push(' ');
                last_was_space = true;
            }
        } else {
            // ASCII-uppercase; los caracteres acentuados (no-ASCII) quedan intactos (é → é).
            out.push(ch.to_ascii_uppercase());
            last_was_space = false;
        }
    }
    out
}

/// Fold de diacríticos del español a ASCII en mayúscula (ÁÉÍÓÚÜÑ → AEIOUUN, y sus minúsculas).
/// Se aplica **solo en comparaciones** (hint de savings en `csv_presets`, matching de reglas
/// aprendidas en `rules`): NUNCA se usa en `normalize_concept` ni en la huella de dedup (esas deben
/// conservar los acentos para no cambiar los fingerprints ni los patrones ya almacenados). Como
/// `normalize_concept` solo hace ASCII-uppercase, las vocales acentuadas llegan en su caja original,
/// por eso se contemplan mayúsculas y minúsculas.
pub fn fold_diacritics_upper(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            'Á' | 'À' | 'Ä' | 'Â' | 'á' | 'à' | 'ä' | 'â' => 'A',
            'É' | 'È' | 'Ë' | 'Ê' | 'é' | 'è' | 'ë' | 'ê' => 'E',
            'Í' | 'Ì' | 'Ï' | 'Î' | 'í' | 'ì' | 'ï' | 'î' => 'I',
            'Ó' | 'Ò' | 'Ö' | 'Ô' | 'ó' | 'ò' | 'ö' | 'ô' => 'O',
            'Ú' | 'Ù' | 'Ü' | 'Û' | 'ú' | 'ù' | 'ü' | 'û' => 'U',
            'Ñ' | 'ñ' => 'N',
            other => other,
        })
        .collect()
}

/// Forma canónica del importe a 4 decimales fijos, independiente del formato Display:
/// `-26.000000000` → `-26.0000`, `-3` → `-3.0000`, `-3.00` → `-3.0000`. Garantiza que
/// `-3` y `-3.00` produzcan la MISMA huella.
pub fn canonical_amount(amount: Decimal) -> String {
    let mut a = amount.round_dp(4);
    a.rescale(4);
    a.to_string()
}

/// Huella de dedup: `source · op_date ISO · importe canónico 4dp · normalize_concept(concept)`
/// unidos con `\u{1f}`. Computada en Rust (nunca almacenada en el CSV/backup).
pub fn compute_fingerprint(
    source: &str,
    op_date: NaiveDate,
    amount: Decimal,
    concept: &str,
) -> String {
    format!(
        "{source}{sep}{date}{sep}{amount}{sep}{concept}",
        sep = FINGERPRINT_SEP,
        date = op_date.format("%Y-%m-%d"),
        amount = canonical_amount(amount),
        concept = normalize_concept(concept),
    )
}

/// Patrón de regla aprendida: concepto normalizado SIN los sufijos de referencia numérica
/// (`WWW.AMAZON* NR40Q0424` → `WWW.AMAZON*`, `PERIODO 27/05/2026 27/06/2026` → `PERIODO`).
/// Descarta tokens finales que contengan algún dígito; conserva siempre ≥1 token.
pub fn derive_rule_pattern(concept: &str) -> String {
    let norm = normalize_concept(concept);
    let tokens: Vec<&str> = norm.split(' ').filter(|t| !t.is_empty()).collect();
    if tokens.is_empty() {
        return norm;
    }
    let mut end = tokens.len();
    while end > 1 && tokens[end - 1].chars().any(|c| c.is_ascii_digit()) {
        end -= 1;
    }
    tokens[..end].join(" ")
}

/// Valida y normaliza un `kind` de transacción.
pub fn normalize_kind(raw: &str) -> Result<String, ApiError> {
    match raw.trim() {
        "expense" => Ok("expense".into()),
        "income" => Ok("income".into()),
        "savings" => Ok("savings".into()),
        _ => Err(ApiError::BadRequest(
            "invalid_kind: kind must be 'expense', 'income' or 'savings'".into(),
        )),
    }
}

/// Normaliza `notes`: trim, vacío → None, ≤4000 chars.
pub fn normalize_notes(raw: &Option<String>) -> Result<Option<String>, ApiError> {
    match raw {
        None => Ok(None),
        Some(s) => {
            let t = s.trim();
            if t.is_empty() {
                return Ok(None);
            }
            if t.chars().count() > 4000 {
                return Err(ApiError::BadRequest(
                    "notes must be at most 4000 characters".into(),
                ));
            }
            Ok(Some(t.into()))
        }
    }
}

/// Normaliza y valida un concepto (para altas manuales): trim, no vacío, ≤500 chars.
pub fn normalize_concept_field(raw: &str) -> Result<String, ApiError> {
    let t = raw.trim();
    if t.is_empty() {
        return Err(ApiError::BadRequest("concept must not be empty".into()));
    }
    if t.chars().count() > 500 {
        return Err(ApiError::BadRequest(
            "concept must be at most 500 characters".into(),
        ));
    }
    Ok(t.into())
}

// ---------------------------------------------------------------------------
// Response DTOs
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize, ToSchema)]
pub struct TransactionResponse {
    #[schema(value_type = String, format = "uuid")]
    pub id: Uuid,
    /// `null` para movimientos manuales (efectivo).
    #[schema(value_type = Option<String>, format = "uuid")]
    pub import_id: Option<Uuid>,
    /// `myinvestor` | `n26` | `manual` | …
    pub source: String,
    pub op_date: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value_date: Option<String>,
    pub concept: String,
    #[serde(with = "rust_decimal::serde::str")]
    #[schema(value_type = String)]
    pub amount: Decimal,
    pub currency: String,
    /// `expense` | `income` | `savings` (nullable en BD; en la práctica siempre presente).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>, format = "uuid")]
    pub category_id: Option<Uuid>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub category_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>, format = "uuid")]
    pub linked_asset_id: Option<Uuid>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>, format = "uuid")]
    pub linked_liability_id: Option<Uuid>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
    /// Regla recurrente que generó este movimiento (o de la que es instancia de origen); `null`
    /// para movimientos sueltos. `ON DELETE SET NULL` → sobrevive al borrado de la regla.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>, format = "uuid")]
    pub recurring_rule_id: Option<Uuid>,
    /// Contrapartida de la conciliación de transferencia. Presente ⇒ el movimiento está
    /// CONCILIADO: sigue visible en Movimientos pero excluido de TODOS los agregados de flujo
    /// (totales del mes, comparativa por categoría, promedio real 12m del engine en modos B/C,
    /// serie por categoría y `months[]` de `/v1/history/cashflow`; NO de la curva fina — un
    /// traspaso mueve saldo real entre cuentas propias). `ON DELETE SET NULL` → borrar una pata
    /// desconcilia la otra.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>, format = "uuid")]
    pub transfer_counterpart_id: Option<Uuid>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>, format = "date-time")]
    pub transfer_reconciled_at: Option<DateTime<Utc>>,
    /// `auto` (pase del matcher) | `manual` (par conciliado a mano).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub transfer_reconciled_source: Option<String>,
    /// Concepto de la contrapartida (denormalizado: puede estar en otro mes, evita un round-trip).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub transfer_counterpart_concept: Option<String>,
    /// `YYYY-MM-DD` de la contrapartida (denormalizado).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub transfer_counterpart_op_date: Option<String>,
    #[schema(value_type = String, format = "date-time")]
    pub created_at: DateTime<Utc>,
    #[schema(value_type = String, format = "date-time")]
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct MonthEntry {
    /// `YYYY-MM`.
    pub month: String,
    /// `false` si es el mes civil en curso de la instalación.
    pub is_complete: bool,
    pub txn_count: i64,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ImportBatchResponse {
    #[schema(value_type = String, format = "uuid")]
    pub id: Uuid,
    pub source: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>, format = "uuid")]
    pub account_asset_id: Option<Uuid>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub account_asset_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub original_filename: Option<String>,
    #[schema(value_type = String, format = "date-time")]
    pub created_at: DateTime<Utc>,
    pub txn_count: i64,
}

// ---------------------------------------------------------------------------
// Request DTOs — altas manuales / PATCH
// ---------------------------------------------------------------------------

/// Convierte una alta manual en una regla recurrente: se crea la regla-plantilla y la transacción
/// que la origina queda enlazada a ella. Marcador sin campos: las reglas tienen resolución
/// MENSUAL (la instancia del mes M se fecha en su último día y solo se materializa con M ya
/// cerrado). El antiguo `day_of_month` (≤3.1.0) se ignora si un cliente viejo lo envía.
#[derive(Debug, Deserialize, ToSchema)]
pub struct RecurrenceSpec {}

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateTransactionBody {
    #[schema(value_type = String, format = "date")]
    pub op_date: NaiveDate,
    #[serde(default)]
    #[schema(value_type = Option<String>, format = "date")]
    pub value_date: Option<NaiveDate>,
    pub concept: String,
    #[serde(with = "rust_decimal::serde::str")]
    #[schema(value_type = String)]
    pub amount: Decimal,
    /// `expense` | `income` | `savings`.
    pub kind: String,
    #[serde(default)]
    #[schema(value_type = Option<String>, format = "uuid")]
    pub category_id: Option<Uuid>,
    #[serde(default)]
    #[schema(value_type = Option<String>, format = "uuid")]
    pub linked_asset_id: Option<Uuid>,
    #[serde(default)]
    #[schema(value_type = Option<String>, format = "uuid")]
    pub linked_liability_id: Option<Uuid>,
    #[serde(default)]
    pub notes: Option<String>,
    /// Si viene, además de la transacción se crea una regla recurrente enlazada.
    #[serde(default)]
    pub recurrence: Option<RecurrenceSpec>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct BatchCreateBody {
    pub transactions: Vec<CreateTransactionBody>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct PatchTransactionBody {
    /// Editable en manuales e importadas. En manuales recomputa la huella; en importadas la huella
    /// queda anclada a la del CSV original (el dedup del re-import sigue funcionando).
    #[serde(default)]
    #[schema(value_type = Option<String>, format = "date")]
    pub op_date: Option<NaiveDate>,
    #[serde(default)]
    #[schema(value_type = Option<String>, format = "date")]
    pub value_date: Option<NaiveDate>,
    #[serde(default)]
    pub clear_value_date: Option<bool>,
    /// Editable en manuales e importadas (huella anclada al CSV en importadas).
    #[serde(default)]
    pub concept: Option<String>,
    /// Editable en manuales e importadas (huella anclada al CSV en importadas).
    #[serde(default)]
    #[serde(with = "rust_decimal::serde::str_option")]
    #[schema(value_type = Option<String>)]
    pub amount: Option<Decimal>,
    #[serde(default)]
    pub kind: Option<String>,
    #[serde(default)]
    #[schema(value_type = Option<String>, format = "uuid")]
    pub category_id: Option<Uuid>,
    #[serde(default)]
    pub clear_category: Option<bool>,
    #[serde(default)]
    #[schema(value_type = Option<String>, format = "uuid")]
    pub linked_asset_id: Option<Uuid>,
    #[serde(default)]
    pub clear_linked_asset: Option<bool>,
    #[serde(default)]
    #[schema(value_type = Option<String>, format = "uuid")]
    pub linked_liability_id: Option<Uuid>,
    #[serde(default)]
    pub clear_linked_liability: Option<bool>,
    #[serde(default)]
    pub notes: Option<String>,
    #[serde(default)]
    pub clear_notes: Option<bool>,
}

// ---------------------------------------------------------------------------
// Request/response DTOs — import preview / confirm
// ---------------------------------------------------------------------------

fn default_source_auto() -> String {
    "auto".into()
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct ImportPreviewBody {
    /// `auto` | `myinvestor` | `n26`.
    #[serde(default = "default_source_auto")]
    pub source: String,
    /// Bytes del CSV, base64.
    pub file_b64: String,
    #[serde(default)]
    #[schema(value_type = Option<String>, format = "uuid")]
    pub account_asset_id: Option<Uuid>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct PreviewRow {
    pub index: u32,
    pub op_date: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value_date: Option<String>,
    pub concept: String,
    #[serde(with = "rust_decimal::serde::str")]
    #[schema(value_type = String)]
    pub amount: Decimal,
    pub currency: String,
    /// `new` | `already_imported`.
    pub status: String,
    /// `expense` | `income` | `savings`.
    pub suggested_kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>, format = "uuid")]
    pub suggested_category_id: Option<Uuid>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub suggested_category_name: Option<String>,
    /// Heurística: probable transferencia interna (sugerida como descarte).
    pub suggested_transfer: bool,
    /// `true` si la divisa del movimiento no es EUR (no importable en fase 1).
    pub currency_warning: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>, format = "uuid")]
    pub matched_rule_id: Option<Uuid>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ImportPreviewResponse {
    /// Preset detectado: `myinvestor` | `n26`.
    pub source: String,
    /// SHA-256 hex de los bytes del CSV. Debe reenviarse en confirm (anti file-swap).
    pub file_sha256: String,
    pub row_count: u32,
    pub new_count: u32,
    pub already_imported_count: u32,
    pub suggested_transfer_count: u32,
    /// Filas con kind+categoría pre-asignados por una regla.
    pub precategorized_count: u32,
    pub currency_warning_count: u32,
    pub rows: Vec<PreviewRow>,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct ImportDecision {
    #[serde(default)]
    pub discard: bool,
    /// Fuerza el alta aunque la fila sea `already_imported` (nueva ocurrencia/ordinal).
    #[serde(default)]
    pub force: bool,
    /// `expense` | `income` | `savings`.
    pub kind: String,
    #[serde(default)]
    #[schema(value_type = Option<String>, format = "uuid")]
    pub category_id: Option<Uuid>,
    #[serde(default)]
    #[schema(value_type = Option<String>, format = "uuid")]
    pub linked_asset_id: Option<Uuid>,
    #[serde(default)]
    #[schema(value_type = Option<String>, format = "uuid")]
    pub linked_liability_id: Option<Uuid>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct ImportConfirmBody {
    /// `auto` | `myinvestor` | `n26`.
    #[serde(default = "default_source_auto")]
    pub source: String,
    pub file_b64: String,
    /// Debe coincidir con el `file_sha256` del preview (mismo archivo) → si no, 400.
    pub file_sha256: String,
    /// Paralelo por índice a las filas parseadas (mismo número → si no, 400).
    pub decisions: Vec<ImportDecision>,
    #[serde(default = "default_true")]
    pub learn_rules: bool,
    #[serde(default)]
    #[schema(value_type = Option<String>, format = "uuid")]
    pub account_asset_id: Option<Uuid>,
    #[serde(default)]
    pub original_filename: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ImportConfirmResponse {
    /// `null` si no se importó ninguna fila (todo descartado/omitido).
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>, format = "uuid")]
    pub import_id: Option<Uuid>,
    pub imported: u32,
    pub skipped_already_imported: u32,
    pub discarded: u32,
    pub rules_learned: u32,
    /// Pares conciliados por el pase automático post-commit (sobre TODO el dataset del owner,
    /// no solo este lote — la contrapartida puede venir de un import anterior). `0` también si
    /// el pase best-effort falló (la mutación ya está persistida; el siguiente pase lo recoge).
    pub reconciled_pairs: u32,
}

// ---------------------------------------------------------------------------
// Reconcile DTOs (conciliación de transferencias, 3.5.0)
// ---------------------------------------------------------------------------

/// Resultado de un pase de auto-conciliación (`POST /v1/transactions/reconcile`).
#[derive(Debug, Serialize, ToSchema)]
pub struct ReconcileRunResponse {
    /// Pares nuevos enlazados en este pase. Idempotente: un segundo pase inmediato devuelve 0.
    pub pairs_created: u32,
    /// `pairs_created × 2` (las dos patas de cada par).
    pub transactions_reconciled: u32,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct ReconcilePairBody {
    /// La otra pata del par. Debe ser del mismo usuario, con importe exactamente opuesto y misma
    /// divisa; la ventana de ±5 días NO aplica a la conciliación manual.
    #[schema(value_type = String, format = "uuid")]
    pub counterpart_id: Uuid,
}

/// Las dos patas tras conciliar o desconciliar un par.
#[derive(Debug, Serialize, ToSchema)]
pub struct ReconcilePairResponse {
    pub transaction: TransactionResponse,
    pub counterpart: TransactionResponse,
}

// ---------------------------------------------------------------------------
// Rules DTOs
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize, ToSchema)]
pub struct RuleResponse {
    #[schema(value_type = String, format = "uuid")]
    pub id: Uuid,
    /// `substring` | `prefix` | `exact`.
    pub match_kind: String,
    pub pattern: String,
    /// `null` = agnóstica (cualquier banco).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    /// `expense` | `income` | `savings`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub assign_kind: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>, format = "uuid")]
    pub assign_category_id: Option<Uuid>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub assign_category_name: Option<String>,
    #[schema(value_type = String, format = "date-time")]
    pub created_at: DateTime<Utc>,
    #[schema(value_type = String, format = "date-time")]
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateRuleBody {
    /// `substring` (default) | `prefix` | `exact`.
    #[serde(default)]
    pub match_kind: Option<String>,
    pub pattern: String,
    #[serde(default)]
    pub source: Option<String>,
    #[serde(default)]
    pub assign_kind: Option<String>,
    #[serde(default)]
    #[schema(value_type = Option<String>, format = "uuid")]
    pub assign_category_id: Option<Uuid>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct PatchRuleBody {
    #[serde(default)]
    pub match_kind: Option<String>,
    #[serde(default)]
    pub pattern: Option<String>,
    #[serde(default)]
    pub source: Option<String>,
    #[serde(default)]
    pub clear_source: Option<bool>,
    #[serde(default)]
    pub assign_kind: Option<String>,
    #[serde(default)]
    pub clear_assign_kind: Option<bool>,
    #[serde(default)]
    #[schema(value_type = Option<String>, format = "uuid")]
    pub assign_category_id: Option<Uuid>,
    #[serde(default)]
    pub clear_assign_category: Option<bool>,
}

// ---------------------------------------------------------------------------
// Summary DTOs (§A5)
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize, ToSchema)]
pub struct CategoryComparisonLine {
    /// `null` = "Sin categoría".
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>, format = "uuid")]
    pub category_id: Option<Uuid>,
    pub category_name: String,
    /// Magnitud positiva del mes (gastos e income como cifras ≥ 0; ver doc del handler).
    #[serde(with = "rust_decimal::serde::str")]
    #[schema(value_type = String)]
    pub actual: Decimal,
    #[serde(with = "rust_decimal::serde::str")]
    #[schema(value_type = String)]
    pub budget: Decimal,
    #[serde(with = "rust_decimal::serde::str")]
    #[schema(value_type = String)]
    pub avg: Decimal,
    #[serde(with = "rust_decimal::serde::str")]
    #[schema(value_type = String)]
    pub delta_vs_budget: Decimal,
    #[serde(with = "rust_decimal::serde::str")]
    #[schema(value_type = String)]
    pub delta_vs_avg: Decimal,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct BlockActualAvg {
    #[serde(with = "rust_decimal::serde::str")]
    #[schema(value_type = String)]
    pub actual: Decimal,
    #[serde(with = "rust_decimal::serde::str")]
    #[schema(value_type = String)]
    pub avg: Decimal,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct SummaryTotals {
    #[serde(with = "rust_decimal::serde::str")]
    #[schema(value_type = String)]
    pub expense_actual: Decimal,
    /// Σ budget de categorías de gasto (sin línea derivada de cuotas de pasivo).
    #[serde(with = "rust_decimal::serde::str")]
    #[schema(value_type = String)]
    pub expense_budget: Decimal,
    #[serde(with = "rust_decimal::serde::str")]
    #[schema(value_type = String)]
    pub expense_avg: Decimal,
    #[serde(with = "rust_decimal::serde::str")]
    #[schema(value_type = String)]
    pub income_actual: Decimal,
    #[serde(with = "rust_decimal::serde::str")]
    #[schema(value_type = String)]
    pub income_budget: Decimal,
    #[serde(with = "rust_decimal::serde::str")]
    #[schema(value_type = String)]
    pub income_avg: Decimal,
    #[serde(with = "rust_decimal::serde::str")]
    #[schema(value_type = String)]
    pub savings_actual: Decimal,
    #[serde(with = "rust_decimal::serde::str")]
    #[schema(value_type = String)]
    pub savings_avg: Decimal,
    /// `income_actual − expense_actual` (savings excluido del consumo).
    #[serde(with = "rust_decimal::serde::str")]
    #[schema(value_type = String)]
    pub net_actual: Decimal,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct TransactionsSummaryResponse {
    pub year: i32,
    pub month: u32,
    /// `true` si el mes seleccionado es el mes civil en curso de la instalación.
    pub is_partial: bool,
    /// Ventana efectiva del promedio: `"3"` | `"6"` | `"12"` (nº de meses) | `"ytd"` | `"all"`.
    pub avg_window: String,
    /// Nº de meses civiles del tramo `[window_start, selected)` (Months(n)=n; Ytd=month−1;
    /// All=amplitud; puede ser 0).
    pub window_months: u32,
    /// Nº de meses del tramo con ≥1 transacción de cualquier kind/categoría. Es el denominador
    /// del promedio ponderado (`max(months_with_data, 1)`).
    pub months_with_data: u32,
    /// `household` | `mine`.
    pub view: String,
    pub expense_categories: Vec<CategoryComparisonLine>,
    pub income_categories: Vec<CategoryComparisonLine>,
    pub savings: BlockActualAvg,
    pub income: BlockActualAvg,
    pub totals: SummaryTotals,
}

// ---------------------------------------------------------------------------
// Serie mensual por categoría (`GET /v1/transactions/category-series` + tool MCP)
// ---------------------------------------------------------------------------

/// Un mes de la serie de una categoría. Magnitudes ≥ 0 (misma convención de signos que la
/// comparativa: gasto = `−Σ(amount)`, ingreso = `+Σ(amount)`); un reembolso puede dejarla
/// negativa.
#[derive(Debug, Serialize, ToSchema)]
pub struct CategoryMonthPoint {
    /// `YYYY-MM`.
    pub month: String,
    #[serde(with = "rust_decimal::serde::str")]
    #[schema(value_type = String)]
    pub total: Decimal,
}

/// Serie de una categoría: un punto por cada mes de la ventana (cero-relleno — los meses sin
/// movimientos emiten `"0"` para que todas las series estén alineadas).
#[derive(Debug, Serialize, ToSchema)]
pub struct CategoryMonthlySeriesEntry {
    /// `null` = movimientos sin categoría.
    #[schema(value_type = Option<String>, format = "uuid")]
    pub category_id: Option<Uuid>,
    pub category_name: Option<String>,
    pub months: Vec<CategoryMonthPoint>,
}

/// Respuesta de la serie mensual por categoría. El último mes de la ventana es el mes civil en
/// curso (parcial).
#[derive(Debug, Serialize, ToSchema)]
pub struct CategoryMonthlySeriesResponse {
    /// `household` | `mine`.
    pub view: String,
    /// `expense` | `income`.
    pub kind: String,
    /// Amplitud efectiva de la ventana (1..=60).
    pub window_months: u32,
    /// Solo categorías con ≥1 movimiento del `kind` en la ventana (orden: nombre ASC, la
    /// pseudo-categoría `null` al final).
    pub series: Vec<CategoryMonthlySeriesEntry>,
}

// ---------------------------------------------------------------------------
// Recurring-rule DTOs (§B3)
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize, ToSchema)]
pub struct RecurringRuleResponse {
    #[schema(value_type = String, format = "uuid")]
    pub id: Uuid,
    pub concept: String,
    /// Firmado (negativo = cargo).
    #[serde(with = "rust_decimal::serde::str")]
    #[schema(value_type = String)]
    pub amount: Decimal,
    /// `expense` | `income` | `savings`.
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>, format = "uuid")]
    pub category_id: Option<Uuid>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub category_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>, format = "uuid")]
    pub linked_asset_id: Option<Uuid>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>, format = "uuid")]
    pub linked_liability_id: Option<Uuid>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
    /// Cursor de idempotencia: primer día del mes más reciente ya materializado (`YYYY-MM-DD`).
    pub last_materialized_month: String,
    #[schema(value_type = String, format = "date-time")]
    pub created_at: DateTime<Utc>,
    #[schema(value_type = String, format = "date-time")]
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct MaterializeResponse {
    /// Reglas del usuario examinadas.
    pub rules_processed: u32,
    /// Transacciones nuevas generadas.
    pub materialized: u32,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    #[test]
    fn normalize_concept_uppercases_ascii_keeps_accents_collapses_ws() {
        // ASCII-uppercase only: the accented chars stay verbatim (é not É).
        assert_eq!(normalize_concept("  Café   Módena  "), "CAFé MóDENA");
        assert_eq!(
            normalize_concept("WWW.AMAZON* NR40Q0424"),
            "WWW.AMAZON* NR40Q0424"
        );
        assert_eq!(normalize_concept("estalvi"), "ESTALVI");
        assert_eq!(normalize_concept("\tSopar\n entrada "), "SOPAR ENTRADA");
    }

    #[test]
    fn canonical_amount_is_stable_across_scales() {
        assert_eq!(
            canonical_amount(Decimal::from_str("-26.000000000").unwrap()),
            "-26.0000"
        );
        assert_eq!(
            canonical_amount(Decimal::from_str("-3").unwrap()),
            "-3.0000"
        );
        assert_eq!(
            canonical_amount(Decimal::from_str("-3.00").unwrap()),
            "-3.0000"
        );
        assert_eq!(
            canonical_amount(Decimal::from_str("1761.34").unwrap()),
            "1761.3400"
        );
        // -3 and -3.00 collapse to the same canonical string → same fingerprint.
        assert_eq!(
            canonical_amount(Decimal::from_str("-3").unwrap()),
            canonical_amount(Decimal::from_str("-3.00").unwrap())
        );
    }

    #[test]
    fn fingerprint_is_deterministic_and_dedups_scale_variants() {
        let d = NaiveDate::from_ymd_opt(2026, 6, 2).unwrap();
        let a = compute_fingerprint(
            "n26",
            d,
            Decimal::from_str("-26.000000000").unwrap(),
            " Sopar  entrada ",
        );
        let b = compute_fingerprint("n26", d, Decimal::from_str("-26").unwrap(), "Sopar entrada");
        assert_eq!(a, b, "scale + whitespace variants must share a fingerprint");
        // Different source, date, amount or concept must diverge.
        assert_ne!(
            a,
            compute_fingerprint(
                "myinvestor",
                d,
                Decimal::from_str("-26").unwrap(),
                "Sopar entrada"
            )
        );
        assert_ne!(
            a,
            compute_fingerprint("n26", d, Decimal::from_str("-27").unwrap(), "Sopar entrada")
        );
    }

    #[test]
    fn derive_rule_pattern_strips_numeric_reference_suffixes() {
        assert_eq!(derive_rule_pattern("WWW.AMAZON* NR40Q0424"), "WWW.AMAZON*");
        assert_eq!(
            derive_rule_pattern("PERIODO 27/05/2026 27/06/2026"),
            "PERIODO"
        );
        assert_eq!(derive_rule_pattern("Nomina Juny"), "NOMINA JUNY");
        assert_eq!(
            derive_rule_pattern("Aportacion automatica cartera"),
            "APORTACION AUTOMATICA CARTERA"
        );
        // Never strip the only token even if it has digits.
        assert_eq!(derive_rule_pattern("365 MALLORCA"), "365 MALLORCA");
        assert_eq!(derive_rule_pattern("A1B2"), "A1B2");
    }
}
