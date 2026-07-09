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
    fold_diacritics_upper, normalize_concept, normalize_kind, CreateRuleBody, PatchRuleBody,
    RuleResponse,
};
use crate::state::AppState;
use axum::extract::{Extension, Path};
use axum::Json;
use axum_extra::extract::cookie::CookieJar;
use chrono::{DateTime, Utc};
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
            "match_kind must be 'substring', 'prefix' or 'exact'".into(),
        )),
    }
}

fn normalize_pattern(raw: &str) -> Result<String, ApiError> {
    let n = normalize_concept(raw);
    if n.trim().is_empty() {
        return Err(ApiError::BadRequest("pattern must not be empty".into()));
    }
    if n.chars().count() > 500 {
        return Err(ApiError::BadRequest(
            "pattern must be at most 500 characters".into(),
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
    let sql = format!(
        "{RULE_SELECT} WHERE r.installation_id = $1 AND r.owner_user_id = $2 \
         ORDER BY r.updated_at DESC, r.id ASC"
    );
    let rows: Vec<RuleRow> = sqlx::query_as(&sql)
        .bind(iid)
        .bind(user.id.0)
        .fetch_all(&state.pool)
        .await?;
    Ok(Json(rows.into_iter().map(row_to_response).collect()))
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

    let match_kind = normalize_match_kind(body.match_kind.as_deref().unwrap_or("substring"))?;
    let pattern = normalize_pattern(&body.pattern)?;
    let source = normalize_source(&body.source);
    let assign_kind = match &body.assign_kind {
        Some(k) => Some(normalize_kind(k)?),
        None => {
            return Err(ApiError::BadRequest(
                "assign_kind is required (expense, income or savings)".into(),
            ))
        }
    };
    if let Some(k) = &assign_kind {
        validate_rule_assignment(&state.pool, iid, k, body.assign_category_id).await?;
    }

    let id: Uuid = sqlx::query_scalar(
        r#"INSERT INTO categorization_rules
               (installation_id, owner_user_id, match_kind, pattern, source,
                assign_kind, assign_category_id)
           VALUES ($1, $2, $3, $4, $5, $6, $7)
           RETURNING id"#,
    )
    .bind(iid)
    .bind(user.id.0)
    .bind(&match_kind)
    .bind(&pattern)
    .bind(source.as_deref())
    .bind(assign_kind.as_deref())
    .bind(body.assign_category_id)
    .fetch_one(&state.pool)
    .await?;

    let row = load_rule_row(&state.pool, id).await?;
    Ok((axum::http::StatusCode::CREATED, Json(row_to_response(row))))
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
            "assign_category_id requires an assign_kind".into(),
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
