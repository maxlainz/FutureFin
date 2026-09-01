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
    /// `true` = regla construida en memoria desde los `pending_assignments` del preview
    /// (no persistida: su `id` es sintético y NO debe publicarse como `matched_rule_id`).
    pub ephemeral: bool,
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
            ephemeral: false,
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
    let (out, _total) = list_categorization_rules_core(&state.pool, iid, user.id.0, None, 0).await?;
    Ok(Json(out))
}

/// Core sin HTTP: lo comparten el handler GET y la tool MCP `list_categorization_rules`.
/// Siempre own-user (el endpoint no acepta `?view` — no inventarlo en la tool).
///
/// Paginación con el mismo contrato que `list_transactions_core`: con `limit = None` (el handler
/// HTTP) no se emite `LIMIT`/`OFFSET` ni la query de `COUNT`, así que el conjunto entero y el
/// contrato REST quedan intactos; con `limit = Some(n)` (la tool MCP) la paginación baja a SQL y
/// `total_count` sale de un `COUNT(*)`.
///
/// Hace falta aquí y no en los otros listados porque **éste es el único que crece con el uso
/// normal**: `learn_rule` inserta una regla por concepto distinto en cada import con
/// `learn_rules = true`. Una instalación con dos años de extractos devolvía ~100 reglas y ~11 KB
/// de una tacada — una porción notable de la ventana de contexto de un agente (auditoría MCP §9).
pub(crate) async fn list_categorization_rules_core(
    pool: &sqlx::PgPool,
    iid: Uuid,
    user_id: Uuid,
    limit: Option<i64>,
    offset: i64,
) -> Result<(Vec<RuleResponse>, i64), ApiError> {
    let mut sql = format!(
        "{RULE_SELECT} WHERE r.installation_id = $1 AND r.owner_user_id = $2 \
         ORDER BY r.updated_at DESC, r.id ASC"
    );
    if limit.is_some() {
        sql.push_str(" LIMIT $3 OFFSET $4");
    }
    let mut q = sqlx::query_as(&sql).bind(iid).bind(user_id);
    if let Some(n) = limit {
        q = q.bind(n).bind(offset);
    }
    let rows: Vec<RuleRow> = q.fetch_all(pool).await?;
    let page: Vec<RuleResponse> = rows.into_iter().map(row_to_response).collect();

    // Sin `limit` el total ES la página: nos ahorramos el COUNT y el camino HTTP no cambia.
    let total = match limit {
        None => page.len() as i64,
        Some(_) => sqlx::query_scalar(
            "SELECT COUNT(*) FROM categorization_rules \
             WHERE installation_id = $1 AND owner_user_id = $2",
        )
        .bind(iid)
        .bind(user_id)
        .fetch_one(pool)
        .await?,
    };
    Ok((page, total))
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

    // Duplicado → 409, que es lo que el contrato promete desde siempre («Ya existe una regla con
    // ese (source, pattern)») y lo que hasta 4.3.1 NO pasaba con las reglas agnósticas: la
    // constraint UNIQUE no atrapa `source IS NULL` porque en SQL `NULL <> NULL`, así que dos
    // llamadas idénticas sin `source` creaban dos reglas y devolvían 200 las dos veces. Es el caso
    // por defecto (el campo es opcional) y el caso del reintento tras un timeout.
    //
    // La comparación es `COALESCE(source,'')` —`normalize_source` ya convierte `""` en NULL, así
    // que no hay colisión espuria— y NO mira `match_kind`, para que la promesa se cumpla igual con
    // `source` y sin él. El índice parcial de 20260828120000 es el respaldo en carrera; es más
    // estrecho a propósito (ver la cabecera de esa migración).
    let dup: Option<Uuid> = sqlx::query_scalar(
        r#"SELECT id FROM categorization_rules
           WHERE installation_id = $1 AND owner_user_id = $2
             AND COALESCE(source, '') = COALESCE($3, '')
             AND pattern = $4
           LIMIT 1"#,
    )
    .bind(iid)
    .bind(user_id)
    .bind(source.as_deref())
    .bind(&pattern)
    .fetch_optional(pool)
    .await?;
    if let Some(existing) = dup {
        return Err(ApiError::ConflictWith(format!(
            "rule_duplicate: a rule for source {} and pattern '{}' already exists ({existing}); patch it instead of creating a second one",
            source.as_deref().unwrap_or("(any bank)"),
            pattern
        )));
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
    /// Con `assigns_nothing = true` los ejemplos son de los movimientos **ensombrecidos**, no de
    /// los que cambiarían (no cambia ninguno).
    pub sample: Vec<String>,
    /// `true` ⟺ la regla no tiene `assign_kind`, así que **no asigna nada**. Alcanzable desde el
    /// propio catálogo (`clear_assign_kind`). Solo puede salir `true` en `dry_run`: aplicarla de
    /// verdad sigue siendo `rule_not_applicable` (400), porque no hay nada que escribir.
    pub assigns_nothing: bool,
    /// Movimientos donde ESTA regla gana la precedencia sin asignar nada Y otra regla se la
    /// habría llevado. Es la huella real de una regla que no asigna: no categoriza, **tapa**.
    /// Retirarla deja que esas otras reglas actúen en los imports futuros. `0` salvo con
    /// `assigns_nothing = true` (no se calcula en el caso normal: costaría un pase extra de
    /// precedencia por fila para un dato que allí no significa nada).
    pub shadowed_transactions: i64,
    /// Explicación en prosa cuando la huella necesita una, hoy solo el caso `assigns_nothing`.
    /// `null` en el caso normal — los contadores ya se explican solos.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
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
    // Una regla SIN `assign_kind` no asigna nada (y por el invariante de `patch_rule_core`,
    // tampoco puede llevar categoría). Aplicarla de verdad sigue siendo un 400: no hay nada que
    // escribir. **Previsualizarla, en cambio, es una pregunta legítima y hasta 4.3.1 reventaba**:
    // el preview de `delete_categorization_rule` entra por aquí con `dry_run = true`, así que
    // borrar la regla a ciegas (`confirm: true`) funcionaba y previsualizarla fallaba con
    // «rule_not_applicable», un mensaje que en ese contexto no quiere decir nada. El peor patrón
    // posible: lo destructivo pasa y lo seguro no.
    //
    // ¿Qué es la huella de una regla que no asigna nada? No es «cero movimientos afectados»: la
    // regla SÍ participa en la precedencia de `match_rule`, así que puede **tapar** a otra que sí
    // asignaría (`suggest_kind_category` cae al default por signo cuando gana una regla sin
    // `assign_kind`). Eso es lo que cuenta `shadowed_transactions` y lo que dice `note`.
    let assign_kind = target.assign_kind.clone();
    match &assign_kind {
        // La categoría pudo cambiar de scope desde que se creó la regla: revalidar una vez, no por fila.
        Some(k) => super::assert_transaction_category(pool, iid, k, target.assign_category_id).await?,
        None if !dry_run => {
            return Err(ApiError::BadRequest(
                "rule_not_applicable: rule has no assign_kind to apply".into(),
            ))
        }
        None => {}
    }

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

    // Conjunto de reglas SIN esta, para responder «¿quién ganaría si no existiera?». Solo hace
    // falta en el caso `assigns_nothing`; en el normal ni se materializa.
    let others: Vec<LoadedRule> = if assign_kind.is_none() {
        rules.iter().filter(|r| r.id != rule_id).cloned().collect()
    } else {
        Vec::new()
    };

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
        let Some(assign_kind) = assign_kind.as_deref() else {
            // Gana la precedencia y no asigna nada: el movimiento se queda como está. Solo cuenta
            // como ensombrecido si OTRA regla se lo habría llevado — si no la hay, esta regla no
            // le está tapando nada a nadie y retirarla no cambiaría el import.
            if match_rule(&others, &r.source, &r.concept).is_some() {
                out.shadowed_transactions += 1;
                if out.sample.len() < 10 {
                    out.sample.push(format!(
                        "{} · {} · {} ({})",
                        r.op_date,
                        r.concept,
                        r.amount,
                        r.kind.as_deref().unwrap_or("-")
                    ));
                }
            }
            continue;
        };
        if r.kind.as_deref() == Some(assign_kind)
            && r.category_id == target.assign_category_id
        {
            out.already_correct += 1;
            continue;
        }
        if r.kind.as_deref() != Some(assign_kind) {
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

    if assign_kind.is_none() {
        // `dry_run` garantizado: sin él ya se devolvió `rule_not_applicable` arriba.
        out.assigns_nothing = true;
        out.note = Some(format!(
            "Esta regla no asigna nada (sin `assign_kind`), así que no categoriza ningún movimiento: \
             su huella de cambio es cero por definición, no porque no haya trabajo. Lo que sí hace es \
             ganar la precedencia y TAPAR a otras reglas en {n} movimiento(s) ({muestra}); retirarla \
             dejaría que esas reglas se apliquen en los imports futuros.",
            n = out.shadowed_transactions,
            muestra = if out.sample.is_empty() { "sin ejemplos" } else { "ver `sample`" },
        ));
    }

    if dry_run || to_update.is_empty() {
        return Ok(out);
    }

    sqlx::query(
        r#"UPDATE transactions SET kind = $1, category_id = $2, updated_at = now()
           WHERE id = ANY($3)"#,
    )
    .bind(assign_kind.as_deref())
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
            assigns_nothing: false,
            shadowed_transactions: 0,
            note: None,
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
    let out = patch_rule_core(&state.pool, iid, user.id.0, id, body).await?;
    Ok(Json(out))
}

/// Core sin HTTP: la comparten el handler PATCH y la tool MCP `update_categorization_rule`.
///
/// **Cache: NONE.** Editar una regla no recategoriza nada retroactivamente — solo cambia lo que se
/// aplicará a imports futuros —, así que el conjunto de transacciones no se mueve y la proyección
/// no puede cambiar en ningún modo. Por eso toma `pool` y no `&Arc<AppState>`: pasar el state
/// sugeriría que hay invalidación que hacer. Para reescribir el pasado está
/// `apply_categorization_rule_core`, que sí es COND.
pub(crate) async fn patch_rule_core(
    pool: &sqlx::PgPool,
    iid: Uuid,
    user_id: Uuid,
    id: Uuid,
    body: PatchRuleBody,
) -> Result<RuleResponse, ApiError> {
    // Destructuring EXHAUSTIVO y **sin `..`**: añadir un campo al body deja de compilar hasta que
    // alguien decida si cuenta como «algo que actualizar» y si colisiona con algún `clear_*`. Es la
    // red que le faltó a `cap_value` en `update_allocation_rule`, donde el campo existía, nadie lo
    // leía, y la llamada devolvía 200 sin hacer nada (auditoría MCP §5).
    let PatchRuleBody {
        match_kind,
        pattern,
        source,
        clear_source,
        assign_kind,
        clear_assign_kind,
        assign_category_id,
        clear_assign_category,
    } = &body;
    let set = |b: &Option<bool>| *b == Some(true);

    // Una sola tabla alimenta la guardia de patch vacío Y el texto del error, así que no pueden
    // desincronizarse.
    let provided: [(&str, bool); 8] = [
        ("match_kind", match_kind.is_some()),
        ("pattern", pattern.is_some()),
        ("source", source.is_some()),
        ("clear_source", set(clear_source)),
        ("assign_kind", assign_kind.is_some()),
        ("clear_assign_kind", set(clear_assign_kind)),
        ("assign_category_id", assign_category_id.is_some()),
        ("clear_assign_category", set(clear_assign_category)),
    ];
    if !provided.iter().any(|(_, present)| *present) {
        let campos: Vec<&str> = provided.iter().map(|(name, _)| *name).collect();
        return Err(ApiError::BadRequest(format!(
            "rule_patch_empty: provide at least one of {}",
            campos.join(", ")
        )));
    }

    // Poner y borrar el mismo campo a la vez: error, no «gana el clear». Hasta 4.0.0 el `clear`
    // ganaba en silencio, que es la misma clase de fallo que `cap_value` — un 200 y no lo que
    // pediste. El propio auditoría MCP elogia que `due_date` + `clear_due_date` juntos den error.
    // El nombre del FLAG va aparte del nombre del campo: componerlo como `clear_{campo}` daba
    // `clear_assign_category_id` para `assign_category_id`, y ese parámetro NO EXISTE (el real es
    // `clear_assign_category`). Los mensajes de error son documentación de facto: si nombran un
    // campo inventado, dirigen mal el reintento del cliente hacia algo que nunca va a funcionar.
    for (campo, flag, puesto, borrado) in [
        ("source", "clear_source", source.is_some(), set(clear_source)),
        (
            "assign_kind",
            "clear_assign_kind",
            assign_kind.is_some(),
            set(clear_assign_kind),
        ),
        (
            "assign_category_id",
            "clear_assign_category",
            assign_category_id.is_some(),
            set(clear_assign_category),
        ),
    ] {
        if puesto && borrado {
            return Err(ApiError::BadRequest(format!(
                "rule_patch_conflict: {campo} and {flag} are mutually exclusive"
            )));
        }
    }

    // Guardia id + installation + owner → 404 si no es tuyo.
    let current: Option<RuleRow> = {
        let sql = format!("{RULE_SELECT} WHERE r.id = $1 AND r.installation_id = $2 AND r.owner_user_id = $3");
        sqlx::query_as(&sql)
            .bind(id)
            .bind(iid)
            .bind(user_id)
            .fetch_optional(pool)
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
        validate_rule_assignment(pool, iid, k, new_assign_category).await?;
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
    .bind(user_id)
    .execute(pool)
    .await?;

    let row = load_rule_row(pool, id).await?;
    Ok(row_to_response(row))
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
    delete_rule_core(&state.pool, iid, user.id.0, id).await?;
    Ok(axum::http::StatusCode::NO_CONTENT)
}

/// Core sin HTTP: la comparten el handler DELETE y la tool MCP `delete_categorization_rule`.
///
/// **Cache: NONE**, por el mismo motivo que `patch_rule_core`. Y borrar la regla **no descategoriza
/// nada**: los movimientos que ya llevan categoría la conservan; la regla simplemente deja de
/// aplicarse a los imports futuros.
pub(crate) async fn delete_rule_core(
    pool: &sqlx::PgPool,
    iid: Uuid,
    user_id: Uuid,
    id: Uuid,
) -> Result<(), ApiError> {
    let res = sqlx::query(
        r#"DELETE FROM categorization_rules
           WHERE id = $1 AND installation_id = $2 AND owner_user_id = $3"#,
    )
    .bind(id)
    .bind(iid)
    .bind(user_id)
    .execute(pool)
    .await?;
    if res.rows_affected() == 0 {
        return Err(ApiError::NotFound);
    }
    Ok(())
}
