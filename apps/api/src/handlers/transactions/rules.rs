//! Reglas de categorización (A4): matching en preview, aprendizaje en confirm, y CRUD.
//!
//! Precedencia de matching: source-específica > agnóstica → exact > prefix > substring →
//! patrón más largo → `updated_at` más reciente. Sin regla → el caller aplica el default por
//! signo (negativo=expense, positivo=income), categoría NULL.

use crate::error::ApiError;
use crate::handlers::installation::require_installation_member;
use crate::handlers::membership::role_can_write;
use crate::handlers::session::require_session_user;
use crate::handlers::transactions::schema::{
    fold_diacritics_upper, normalize_concept, normalize_kind, ApplyRuleBody, CreateRuleBody,
    PatchRuleBody, RuleResponse,
};
use crate::state::AppState;
use axum::extract::{Extension, Path};
use axum::Json;
use axum_extra::extract::cookie::CookieJar;
use chrono::{DateTime, Utc};
use serde::Serialize;
use utoipa::ToSchema;
use sqlx::{FromRow, PgConnection};
use std::sync::Arc;
use uuid::Uuid;

// ---------------------------------------------------------------------------
// Matching (usado por el preview de import)
// ---------------------------------------------------------------------------

/// Regla cargada en memoria para el matching de un batch de preview.
#[derive(Debug, Clone)]
pub struct LoadedRule {
    pub id: Uuid,
    pub match_kind: String,
    pub pattern: String,
    pub source: Option<String>,
    pub assign_kind: Option<String>,
    pub assign_category_id: Option<Uuid>,
    pub updated_at: DateTime<Utc>,
}

/// Carga todas las reglas del usuario para el matching del preview.
pub async fn load_rules(
    pool: &sqlx::PgPool,
    iid: Uuid,
    owner: Uuid,
) -> Result<Vec<LoadedRule>, ApiError> {
    #[derive(FromRow)]
    struct Row {
        id: Uuid,
        match_kind: String,
        pattern: String,
        source: Option<String>,
        assign_kind: Option<String>,
        assign_category_id: Option<Uuid>,
        updated_at: DateTime<Utc>,
    }
    let rows: Vec<Row> = sqlx::query_as(
        r#"SELECT id, match_kind, pattern, source, assign_kind, assign_category_id, updated_at
           FROM categorization_rules
           WHERE installation_id = $1 AND owner_user_id = $2"#,
    )
    .bind(iid)
    .bind(owner)
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| LoadedRule {
            id: r.id,
            match_kind: r.match_kind,
            pattern: r.pattern,
            source: r.source,
            assign_kind: r.assign_kind,
            assign_category_id: r.assign_category_id,
            updated_at: r.updated_at,
        })
        .collect())
}

fn rule_matches(r: &LoadedRule, norm_concept: &str) -> bool {
    // Fold de diacríticos en AMBOS lados solo para comparar: un patrón acentuado («APORTACIóN»)
    // matchea un concepto sin tilde («APORTACION») y viceversa. Los patrones almacenados y
    // `normalize_concept`/fingerprints conservan sus acentos intactos.
    let concept = fold_diacritics_upper(norm_concept);
    let pattern = fold_diacritics_upper(&r.pattern);
    match r.match_kind.as_str() {
        "exact" => concept == pattern,
        "prefix" => concept.starts_with(&pattern),
        _ => concept.contains(&pattern),
    }
}

fn match_kind_rank(k: &str) -> u8 {
    match k {
        "exact" => 3,
        "prefix" => 2,
        _ => 1,
    }
}

/// Mejor regla aplicable a `(source, concept)`, o `None`. Reglas cuyo `source` es `Some` y
/// distinto de `source` no aplican; `source = None` (agnóstica) aplica a cualquier banco.
pub fn match_rule<'a>(rules: &'a [LoadedRule], source: &str, concept: &str) -> Option<&'a LoadedRule> {
    let norm = normalize_concept(concept);
    let mut best: Option<&LoadedRule> = None;
    // Clave de precedencia (mayor gana): (source_específica, rank(match_kind), len(pattern), updated_at).
    let mut best_key: Option<(bool, u8, usize, DateTime<Utc>)> = None;
    for r in rules {
        // Una regla source-específica de otro banco no aplica.
        if let Some(rs) = r.source.as_deref() {
            if rs != source {
                continue;
            }
        }
        if !rule_matches(r, &norm) {
            continue;
        }
        let key = (
            r.source.is_some(),
            match_kind_rank(&r.match_kind),
            r.pattern.chars().count(),
            r.updated_at,
        );
        if best_key.map_or(true, |bk| key > bk) {
            best_key = Some(key);
            best = Some(r);
        }
    }
    best
}

// ---------------------------------------------------------------------------
// Aprendizaje (usado por confirm)
// ---------------------------------------------------------------------------

/// Upsert de una regla aprendida (source concreto → la constraint UNIQUE con source no-NULL
/// dispara el ON CONFLICT). `pattern` ya viene normalizado (via `derive_rule_pattern`).
pub async fn learn_rule(
    conn: &mut PgConnection,
    iid: Uuid,
    owner: Uuid,
    source: &str,
    pattern: &str,
    assign_kind: &str,
    assign_category_id: Option<Uuid>,
) -> Result<(), ApiError> {
    sqlx::query(
        r#"INSERT INTO categorization_rules
               (installation_id, owner_user_id, match_kind, pattern, source,
                assign_kind, assign_category_id)
           VALUES ($1, $2, 'substring', $3, $4, $5, $6)
           ON CONFLICT ON CONSTRAINT categorization_rules_unique
           DO UPDATE SET assign_kind = EXCLUDED.assign_kind,
                         assign_category_id = EXCLUDED.assign_category_id,
                         updated_at = now()"#,
    )
    .bind(iid)
    .bind(owner)
    .bind(pattern)
    .bind(source)
    .bind(assign_kind)
    .bind(assign_category_id)
    .execute(&mut *conn)
    .await?;
    Ok(())
}

// ---------------------------------------------------------------------------
// CRUD handlers
// ---------------------------------------------------------------------------

#[derive(Debug, FromRow)]
struct RuleRow {
    id: Uuid,
    match_kind: String,
    pattern: String,
    source: Option<String>,
    assign_kind: Option<String>,
    assign_category_id: Option<Uuid>,
    assign_category_name: Option<String>,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

fn row_to_response(r: RuleRow) -> RuleResponse {
    RuleResponse {
        id: r.id,
        match_kind: r.match_kind,
        pattern: r.pattern,
        source: r.source,
        assign_kind: r.assign_kind,
        assign_category_id: r.assign_category_id,
        assign_category_name: r.assign_category_name,
        created_at: r.created_at,
        updated_at: r.updated_at,
    }
}

const RULE_SELECT: &str = r#"SELECT r.id, r.match_kind, r.pattern, r.source, r.assign_kind,
              r.assign_category_id, c.name AS assign_category_name, r.created_at, r.updated_at
       FROM categorization_rules r
       LEFT JOIN categories c ON c.id = r.assign_category_id"#;

fn normalize_match_kind(raw: &str) -> Result<String, ApiError> {
    match raw.trim() {
        "substring" | "" => Ok("substring".into()),
        "prefix" => Ok("prefix".into()),
        "exact" => Ok("exact".into()),
        _ => Err(ApiError::BadRequest(
            "rule_match_kind_invalid: match_kind must be 'substring', 'prefix' or 'exact'".into(),
        )),
    }
}

fn normalize_pattern(raw: &str) -> Result<String, ApiError> {
    let n = normalize_concept(raw);
    if n.trim().is_empty() {
        return Err(ApiError::BadRequest("rule_pattern_empty: pattern must not be empty".into()));
    }
    if n.chars().count() > 500 {
        return Err(ApiError::BadRequest(
            "rule_pattern_too_long: pattern must be at most 500 characters".into(),
        ));
    }
    Ok(n)
}

fn normalize_source(raw: &Option<String>) -> Option<String> {
    raw.as_ref()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

#[utoipa::path(
    get,
    path = "/v1/transactions/rules",
    tag = "transactions",
    responses(
        (status = 200, description = "Reglas de categorización del usuario", body = [RuleResponse]),
        (status = 401, description = "No valid session"),
        (status = 403, description = "Not an installation member"),
        (status = 404, description = "Installation missing"),
    )
)]
pub async fn list_rules(
    Extension(state): Extension<Arc<AppState>>,
    jar: CookieJar,
) -> Result<Json<Vec<RuleResponse>>, ApiError> {
    let user = require_session_user(&jar, &state.pool).await?;
    let (iid, _role) = require_installation_member(&state.pool, user.id.0).await?;
    let out = list_categorization_rules_core(&state.pool, iid, user.id.0).await?;
    Ok(Json(out))
}

/// Core sin HTTP: lo comparten el handler GET y la tool MCP `list_categorization_rules`.
/// Siempre own-user (el endpoint no acepta `?view` — no inventarlo en la tool).
pub(crate) async fn list_categorization_rules_core(
    pool: &sqlx::PgPool,
    iid: Uuid,
    user_id: Uuid,
) -> Result<Vec<RuleResponse>, ApiError> {
    let sql = format!(
        "{RULE_SELECT} WHERE r.installation_id = $1 AND r.owner_user_id = $2 \
         ORDER BY r.updated_at DESC, r.id ASC"
    );
    let rows: Vec<RuleRow> = sqlx::query_as(&sql)
        .bind(iid)
        .bind(user_id)
        .fetch_all(pool)
        .await?;
    Ok(rows.into_iter().map(row_to_response).collect())
}

/// Valida `(assign_kind, assign_category_id)`: savings exige categoría NULL; expense/income con
/// categoría exigen scope acorde.
async fn validate_rule_assignment(
    pool: &sqlx::PgPool,
    iid: Uuid,
    assign_kind: &str,
    assign_category_id: Option<Uuid>,
) -> Result<(), ApiError> {
    super::assert_transaction_category(pool, iid, assign_kind, assign_category_id).await
}

#[utoipa::path(
    post,
    path = "/v1/transactions/rules",
    tag = "transactions",
    request_body = CreateRuleBody,
    responses(
        (status = 201, description = "Regla creada", body = RuleResponse),
        (status = 400, description = "Validación"),
        (status = 401, description = "No valid session"),
        (status = 403, description = "Viewer or not a member"),
        (status = 409, description = "Ya existe una regla con ese (source, pattern)"),
    )
)]
pub async fn create_rule(
    Extension(state): Extension<Arc<AppState>>,
    jar: CookieJar,
    Json(body): Json<CreateRuleBody>,
) -> Result<(axum::http::StatusCode, Json<RuleResponse>), ApiError> {
    let user = require_session_user(&jar, &state.pool).await?;
    let (iid, role) = require_installation_member(&state.pool, user.id.0).await?;
    if !role_can_write(role.as_str()) {
        return Err(ApiError::Forbidden);
    }
    let scope = ApplyScope::parse(body.apply_to_existing.as_deref().unwrap_or("none"))?;
    let confirm = body.confirm.unwrap_or(false);
    if scope != ApplyScope::None && !confirm {
        return Err(ApiError::BadRequest(
            "confirm_required: confirm must be true to apply a new rule to existing transactions".into(),
        ));
    }
    let from_month = body.from_month.clone();
    let resp = create_categorization_rule_core(&state.pool, iid, user.id.0, body).await?;
    // El backfill va DESPUÉS del INSERT y por su propia core, que es quien decide la invalidación:
    // crear la regla sigue siendo NONE, aplicarla es COND. Dos rutas, dos contratos de cache.
    if scope != ApplyScope::None {
        apply_categorization_rule_core(
            &state,
            iid,
            user.id.0,
            resp.id,
            scope,
            from_month.as_deref(),
            false,
        )
        .await?;
    }
    Ok((axum::http::StatusCode::CREATED, Json(resp)))
}

/// Core sin HTTP: lo comparten el handler POST y la tool MCP `create_categorization_rule`.
/// **Solo hace INSERT**: no recategoriza nada, así que NUNCA invalida la cache (pinneado por
/// `creating_a_categorization_rule_never_invalidates_projection_cache`, en los tres modos). El
/// backfill retroactivo es `apply_categorization_rule_core`, otra ruta y otra clase de cache.
/// 409 en duplicado `(source, pattern)` vía el mapeo global de sqlx.
pub(crate) async fn create_categorization_rule_core(
    pool: &sqlx::PgPool,
    iid: Uuid,
    user_id: Uuid,
    body: CreateRuleBody,
) -> Result<RuleResponse, ApiError> {
    let match_kind = normalize_match_kind(body.match_kind.as_deref().unwrap_or("substring"))?;
    let pattern = normalize_pattern(&body.pattern)?;
    let source = normalize_source(&body.source);
    let assign_kind = match &body.assign_kind {
        Some(k) => Some(normalize_kind(k)?),
        None => {
            return Err(ApiError::BadRequest(
                "rule_assign_kind_required: assign_kind is required (expense, income or savings)".into(),
            ))
        }
    };
    if let Some(k) = &assign_kind {
        validate_rule_assignment(pool, iid, k, body.assign_category_id).await?;
    }

    let id: Uuid = sqlx::query_scalar(
        r#"INSERT INTO categorization_rules
               (installation_id, owner_user_id, match_kind, pattern, source,
                assign_kind, assign_category_id)
           VALUES ($1, $2, $3, $4, $5, $6, $7)
           RETURNING id"#,
    )
    .bind(iid)
    .bind(user_id)
    .bind(&match_kind)
    .bind(&pattern)
    .bind(source.as_deref())
    .bind(assign_kind.as_deref())
    .bind(body.assign_category_id)
    .fetch_one(pool)
    .await?;

    let row = load_rule_row(pool, id).await?;
    Ok(row_to_response(row))
}

// ---------------------------------------------------------------------------
// POST /v1/transactions/rules/{id}/apply — backfill retroactivo
// ---------------------------------------------------------------------------

#[utoipa::path(
    post,
    path = "/v1/transactions/rules/{id}/apply",
    tag = "transactions",
    params(("id" = Uuid, Path, description = "Id de la regla")),
    request_body = ApplyRuleBody,
    responses(
        (status = 200, description = "Backfill aplicado (o preview si falta `confirm`)", body = ApplyRuleOutcome),
        (status = 400, description = "Validación / falta confirm"),
        (status = 401, description = "No valid session"),
        (status = 403, description = "Viewer or not a member"),
        (status = 404, description = "Regla inexistente o de otro usuario"),
    )
)]
pub async fn apply_rule(
    Extension(state): Extension<Arc<AppState>>,
    jar: CookieJar,
    Path(id): Path<Uuid>,
    Json(body): Json<ApplyRuleBody>,
) -> Result<Json<ApplyRuleOutcome>, ApiError> {
    let user = require_session_user(&jar, &state.pool).await?;
    let (iid, role) = require_installation_member(&state.pool, user.id.0).await?;
    if !role_can_write(role.as_str()) {
        return Err(ApiError::Forbidden);
    }
    let scope = ApplyScope::parse(body.apply_to_existing.as_deref().unwrap_or("uncategorized"))?;
    let confirm = body.confirm.unwrap_or(false);
    if scope != ApplyScope::None && !confirm {
        // Por HTTP el preview se pide explícitamente con `apply_to_existing` + sin confirm es un
        // 400: el formulario de la SPA ya enseña el impacto antes de llamar. La tool MCP, en
        // cambio, devuelve el preview (patrón de la casa para las destructivas).
        return Err(ApiError::BadRequest(
            "confirm_required: confirm must be true to apply a rule to existing transactions".into(),
        ));
    }
    let out = apply_categorization_rule_core(
        &state,
        iid,
        user.id.0,
        id,
        scope,
        body.from_month.as_deref(),
        false,
    )
    .await?;
    Ok(Json(out))
}


/// Alcance del backfill de una regla sobre los movimientos ya existentes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ApplyScope {
    /// No toca nada (default de `create_categorization_rule`: contrato histórico intacto).
    None,
    /// Solo movimientos sin categoría.
    Uncategorized,
    /// También reasigna los ya categorizados — el caso «desglosar una categoría cajón».
    All,
}

impl ApplyScope {
    pub(crate) fn parse(raw: &str) -> Result<Self, ApiError> {
        match raw.trim() {
            "none" => Ok(Self::None),
            "uncategorized" => Ok(Self::Uncategorized),
            "all" => Ok(Self::All),
            other => Err(ApiError::BadRequest(format!(
                "apply_to_existing_invalid: apply_to_existing must be none, uncategorized or all (got {other})"
            ))),
        }
    }
}

/// Resultado del backfill. En `dry_run` los contadores describen lo que PASARÍA y no se escribe
/// nada; con `dry_run = false`, `updated` son las filas realmente modificadas.
#[derive(Debug, Serialize, ToSchema)]
pub struct ApplyRuleOutcome {
    /// Filas que cambiarían (o han cambiado).
    pub matched: i64,
    /// Filas donde la regla gana pero la asignación ya es la correcta: no se tocan.
    pub already_correct: i64,
    /// De las que cambiarían, cuántas cambian de `kind`. **No es decorativo**: el `kind` decide
    /// qué suma el promedio real 12m, así que un valor > 0 significa que la proyección se mueve
    /// en los modos B y C.
    pub would_change_kind: i64,
    /// Filas donde el patrón de ESTA regla casa pero su `source` no coincide con el del
    /// movimiento, así que no aplica (misma semántica que en el import). Sin este contador un
    /// `matched: 0` se lee como «no hay nada que hacer», que es justo lo que no es.
    pub skipped_by_source: i64,
    /// Filas donde esta regla casa pero PIERDE la precedencia frente a otra regla.
    pub matched_by_other_rule: i64,
    /// Patas de transferencia conciliadas: se excluyen (están fuera de todos los agregados de
    /// flujo, recategorizarlas no significa nada).
    pub skipped_reconciled: i64,
    /// Desglose de las filas que cambiarían por su categoría ACTUAL.
    pub by_current_category: Vec<ApplyRuleCategoryCount>,
    /// Hasta 10 `resumen` de ejemplo, para verificar que se tocaría lo correcto sin releer nada.
    pub sample: Vec<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ApplyRuleCategoryCount {
    pub category_id: Option<Uuid>,
    pub category_name: Option<String>,
    pub count: i64,
}

/// Aplica una regla de categorización a los movimientos YA EXISTENTES del propio usuario.
///
/// **Usa la precedencia completa** (`match_rule` sobre el conjunto entero de reglas), no la regla
/// suelta: el pasado queda como habría quedado importando hoy. Una fila donde otra regla gana no
/// se toca, y se cuenta en `matched_by_other_rule` para que el llamante no lo lea como un fallo.
///
/// **Invalidación COND dentro de la core**, y solo si se escribió algo: cambiar el `kind` de filas
/// históricas cambia `transactions_avg`, que es input del engine en los modos B y C. Crear la
/// regla sigue sin invalidar (`create_categorization_rule_core`) — son dos rutas distintas.
pub(crate) async fn apply_categorization_rule_core(
    state: &Arc<AppState>,
    iid: Uuid,
    user_id: Uuid,
    rule_id: Uuid,
    scope: ApplyScope,
    from_month: Option<&str>,
    dry_run: bool,
) -> Result<ApplyRuleOutcome, ApiError> {
    if scope == ApplyScope::None {
        return Ok(ApplyRuleOutcome::empty());
    }
    let pool = &state.pool;

    // Owner-guard: una regla de otro usuario es 404, nunca 403 (no se filtra su existencia).
    let target: LoadedRule = {
        let rules = load_rules(pool, iid, user_id).await?;
        rules
            .into_iter()
            .find(|r| r.id == rule_id)
            .ok_or(ApiError::NotFound)?
    };
    let assign_kind = target
        .assign_kind
        .clone()
        .ok_or_else(|| ApiError::BadRequest("rule_not_applicable: rule has no assign_kind to apply".into()))?;
    // La categoría pudo cambiar de scope desde que se creó la regla: revalidar una vez, no por fila.
    super::assert_transaction_category(pool, iid, &assign_kind, target.assign_category_id).await?;

    let rules = load_rules(pool, iid, user_id).await?;

    let from_date = match from_month {
        Some(m) => Some(super::crud::parse_month_start(m)?),
        None => None,
    };

    #[derive(sqlx::FromRow)]
    struct Row {
        id: Uuid,
        source: String,
        concept: String,
        op_date: chrono::NaiveDate,
        amount: rust_decimal::Decimal,
        kind: Option<String>,
        category_id: Option<Uuid>,
        category_name: Option<String>,
        transfer_counterpart_id: Option<Uuid>,
    }
    let mut sql = String::from(
        "SELECT t.id, t.source, t.concept, t.op_date, t.amount, t.kind, t.category_id, \
         c.name AS category_name, t.transfer_counterpart_id \
         FROM transactions t LEFT JOIN categories c ON c.id = t.category_id \
         WHERE t.installation_id = $1 AND t.owner_user_id = $2",
    );
    if scope == ApplyScope::Uncategorized {
        sql.push_str(" AND t.category_id IS NULL");
    }
    if from_date.is_some() {
        sql.push_str(" AND t.op_date >= $3");
    }
    sql.push_str(" ORDER BY t.op_date DESC, t.id DESC");
    let mut q = sqlx::query_as::<_, Row>(&sql).bind(iid).bind(user_id);
    if let Some(d) = from_date {
        q = q.bind(d);
    }
    let rows: Vec<Row> = q.fetch_all(pool).await?;

    let mut out = ApplyRuleOutcome::empty();
    let mut to_update: Vec<Uuid> = Vec::new();
    let mut by_cat: Vec<(Option<Uuid>, Option<String>, i64)> = Vec::new();

    for r in &rows {
        if r.transfer_counterpart_id.is_some() {
            out.skipped_reconciled += 1;
            continue;
        }
        let winner = match_rule(&rules, &r.source, &r.concept);
        let wins = winner.map(|w| w.id) == Some(rule_id);
        if !wins {
            // ¿Habría casado esta regla si no fuera por el `source`, o por la precedencia?
            let text_matches = rule_matches(&target, &normalize_concept(&r.concept));
            if text_matches {
                let source_blocks = target.source.as_deref().is_some_and(|rs| rs != r.source);
                if source_blocks {
                    out.skipped_by_source += 1;
                } else if winner.is_some() {
                    out.matched_by_other_rule += 1;
                }
            }
            continue;
        }
        if r.kind.as_deref() == Some(assign_kind.as_str())
            && r.category_id == target.assign_category_id
        {
            out.already_correct += 1;
            continue;
        }
        if r.kind.as_deref() != Some(assign_kind.as_str()) {
            out.would_change_kind += 1;
        }
        match by_cat.iter_mut().find(|(id, _, _)| *id == r.category_id) {
            Some((_, _, n)) => *n += 1,
            None => by_cat.push((r.category_id, r.category_name.clone(), 1)),
        }
        if out.sample.len() < 10 {
            out.sample.push(format!(
                "{} · {} · {} ({})",
                r.op_date,
                r.concept,
                r.amount,
                r.kind.as_deref().unwrap_or("-")
            ));
        }
        to_update.push(r.id);
    }
    out.matched = to_update.len() as i64;
    out.by_current_category = by_cat
        .into_iter()
        .map(|(category_id, category_name, count)| ApplyRuleCategoryCount {
            category_id,
            category_name,
            count,
        })
        .collect();

    if dry_run || to_update.is_empty() {
        return Ok(out);
    }

    sqlx::query(
        r#"UPDATE transactions SET kind = $1, category_id = $2, updated_at = now()
           WHERE id = ANY($3)"#,
    )
    .bind(&assign_kind)
    .bind(target.assign_category_id)
    .bind(&to_update)
    .execute(pool)
    .await?;

    // El conjunto cambió de atribución → COND. No se toca la conciliación: ni `amount` ni
    // `op_date` se han movido, así que ningún par puede haberse abierto ni cerrado.
    super::invalidate_projection_if_savings_uses_transactions(state, iid, user_id).await;
    Ok(out)
}

impl ApplyRuleOutcome {
    fn empty() -> Self {
        Self {
            matched: 0,
            already_correct: 0,
            would_change_kind: 0,
            skipped_by_source: 0,
            matched_by_other_rule: 0,
            skipped_reconciled: 0,
            by_current_category: Vec::new(),
            sample: Vec::new(),
        }
    }
}

async fn load_rule_row(pool: &sqlx::PgPool, id: Uuid) -> Result<RuleRow, ApiError> {
    let sql = format!("{RULE_SELECT} WHERE r.id = $1");
    sqlx::query_as(&sql)
        .bind(id)
        .fetch_one(pool)
        .await
        .map_err(Into::into)
}

#[utoipa::path(
    patch,
    path = "/v1/transactions/rules/{id}",
    tag = "transactions",
    request_body = PatchRuleBody,
    params(("id" = Uuid, Path, description = "Rule id")),
    responses(
        (status = 200, description = "Regla actualizada", body = RuleResponse),
        (status = 400, description = "Validación"),
        (status = 401, description = "No valid session"),
        (status = 403, description = "Viewer or not a member"),
        (status = 404, description = "Regla inexistente o de otro usuario"),
        (status = 409, description = "Colisión de (source, pattern)"),
    )
)]
pub async fn patch_rule(
    Extension(state): Extension<Arc<AppState>>,
    jar: CookieJar,
    Path(id): Path<Uuid>,
    Json(body): Json<PatchRuleBody>,
) -> Result<Json<RuleResponse>, ApiError> {
    let user = require_session_user(&jar, &state.pool).await?;
    let (iid, role) = require_installation_member(&state.pool, user.id.0).await?;
    if !role_can_write(role.as_str()) {
        return Err(ApiError::Forbidden);
    }

    // Guardia id + installation + owner → 404 si no es tuyo.
    let current: Option<RuleRow> = {
        let sql = format!("{RULE_SELECT} WHERE r.id = $1 AND r.installation_id = $2 AND r.owner_user_id = $3");
        sqlx::query_as(&sql)
            .bind(id)
            .bind(iid)
            .bind(user.id.0)
            .fetch_optional(&state.pool)
            .await?
    };
    let Some(current) = current else {
        return Err(ApiError::NotFound);
    };

    let new_match_kind = match &body.match_kind {
        Some(m) => normalize_match_kind(m)?,
        None => current.match_kind.clone(),
    };
    let new_pattern = match &body.pattern {
        Some(p) => normalize_pattern(p)?,
        None => current.pattern.clone(),
    };
    let new_source = if body.clear_source == Some(true) {
        None
    } else {
        match &body.source {
            Some(_) => normalize_source(&body.source),
            None => current.source.clone(),
        }
    };
    let new_assign_kind = if body.clear_assign_kind == Some(true) {
        None
    } else {
        match &body.assign_kind {
            Some(k) => Some(normalize_kind(k)?),
            None => current.assign_kind.clone(),
        }
    };
    let new_assign_category = if body.clear_assign_category == Some(true) {
        None
    } else {
        body.assign_category_id.or(current.assign_category_id)
    };
    if let Some(k) = &new_assign_kind {
        validate_rule_assignment(&state.pool, iid, k, new_assign_category).await?;
    } else if new_assign_category.is_some() {
        return Err(ApiError::BadRequest(
            "rule_assign_category_requires_kind: assign_category_id requires an assign_kind".into(),
        ));
    }

    sqlx::query(
        r#"UPDATE categorization_rules
           SET match_kind = $1, pattern = $2, source = $3,
               assign_kind = $4, assign_category_id = $5, updated_at = now()
           WHERE id = $6 AND installation_id = $7 AND owner_user_id = $8"#,
    )
    .bind(&new_match_kind)
    .bind(&new_pattern)
    .bind(new_source.as_deref())
    .bind(new_assign_kind.as_deref())
    .bind(new_assign_category)
    .bind(id)
    .bind(iid)
    .bind(user.id.0)
    .execute(&state.pool)
    .await?;

    let row = load_rule_row(&state.pool, id).await?;
    Ok(Json(row_to_response(row)))
}

#[utoipa::path(
    delete,
    path = "/v1/transactions/rules/{id}",
    tag = "transactions",
    params(("id" = Uuid, Path, description = "Rule id")),
    responses(
        (status = 204, description = "Regla borrada"),
        (status = 401, description = "No valid session"),
        (status = 403, description = "Viewer or not a member"),
        (status = 404, description = "Regla inexistente o de otro usuario"),
    )
)]
pub async fn delete_rule(
    Extension(state): Extension<Arc<AppState>>,
    jar: CookieJar,
    Path(id): Path<Uuid>,
) -> Result<axum::http::StatusCode, ApiError> {
    let user = require_session_user(&jar, &state.pool).await?;
    let (iid, role) = require_installation_member(&state.pool, user.id.0).await?;
    if !role_can_write(role.as_str()) {
        return Err(ApiError::Forbidden);
    }
    let res = sqlx::query(
        r#"DELETE FROM categorization_rules
           WHERE id = $1 AND installation_id = $2 AND owner_user_id = $3"#,
    )
    .bind(id)
    .bind(iid)
    .bind(user.id.0)
    .execute(&state.pool)
    .await?;
    if res.rows_affected() == 0 {
        return Err(ApiError::NotFound);
    }
    Ok(axum::http::StatusCode::NO_CONTENT)
}
