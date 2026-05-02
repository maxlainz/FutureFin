//! MVP ZIP export de CSV (sin cifrado). Solo propietario — ver `docs/spec/BACKUP_AND_CSV_SPEC.md`.

use crate::error::ApiError;
use crate::handlers::installation::require_installation_member;
use crate::handlers::membership::MembershipRole;
use crate::handlers::session::require_session_user;
use crate::state::AppState;
use axum::extract::Extension;
use axum::response::IntoResponse;
use axum_extra::extract::cookie::CookieJar;
use chrono::NaiveDate;
use http::header::{CONTENT_DISPOSITION, CONTENT_TYPE};
use rust_decimal::Decimal;
use sqlx::PgPool;
use std::io::Write;
use std::sync::Arc;
use uuid::Uuid;
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipWriter};

fn csv_cell(raw: &str) -> String {
    let needs_quote = raw.contains(',')
        || raw.contains('"')
        || raw.contains('\n')
        || raw.contains('\r');
    if needs_quote {
        format!("\"{}\"", raw.replace('"', "\"\""))
    } else {
        raw.to_string()
    }
}

fn csv_line<I: IntoIterator<Item = String>>(cells: I) -> String {
    cells.into_iter().collect::<Vec<_>>().join(",") + "\n"
}

fn opt_uuid(o: Option<Uuid>) -> String {
    o.map(|u| u.to_string()).unwrap_or_default()
}

fn opt_date(o: Option<NaiveDate>) -> String {
    o.map(|d| d.to_string()).unwrap_or_default()
}

fn opt_dec(o: Option<Decimal>) -> String {
    o.map(|d| d.to_string()).unwrap_or_default()
}

fn zip_add_utf8(
    zip: &mut ZipWriter<std::io::Cursor<&mut Vec<u8>>>,
    name: &str,
    body: String,
) -> Result<(), ApiError> {
    let opts = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
    zip.start_file(name, opts).map_err(zip_err)?;
    zip.write_all(body.as_bytes()).map_err(zip_io_err)?;
    Ok(())
}

fn zip_io_err(e: std::io::Error) -> ApiError {
    tracing::error!(?e, "zip export io");
    ApiError::Unavailable
}

fn zip_err(e: zip::result::ZipError) -> ApiError {
    tracing::error!(?e, "zip export");
    ApiError::Unavailable
}

#[utoipa::path(
    get,
    path = "/v1/backup/export.zip",
    tag = "backup",
    responses(
        (status = 200, description = "ZIP de CSV", content_type = "application/zip"),
        (status = 401, description = "No valid session"),
        (status = 403, description = "Not installation owner"),
    )
)]
pub async fn export_backup_zip(
    Extension(state): Extension<Arc<AppState>>,
    jar: CookieJar,
) -> Result<impl IntoResponse, ApiError> {
    let user = require_session_user(&jar, &state.pool).await?;
    let (iid, role) = require_installation_member(&state.pool, user.id.0).await?;
    if role != MembershipRole::Owner {
        return Err(ApiError::Forbidden);
    }

    let bytes = build_csv_zip(&state.pool, iid).await?;

    let mut res = axum::response::Response::new(axum::body::Body::from(bytes));
    res.headers_mut().insert(
        CONTENT_TYPE,
        http::HeaderValue::from_static("application/zip"),
    );
    res.headers_mut().insert(
        CONTENT_DISPOSITION,
        http::HeaderValue::from_static("attachment; filename=\"futurefin-export.zip\""),
    );
    Ok(res)
}

async fn build_csv_zip(pool: &PgPool, iid: Uuid) -> Result<Vec<u8>, ApiError> {
    let mut buf = Vec::new();

    let household_csv = build_summary_household(pool, iid).await?;
    let people_csv = build_summary_people(pool, iid).await?;
    let categories_csv = build_categories(pool, iid).await?;
    let assets_csv = build_assets(pool, iid).await?;
    let liabilities_csv = build_liabilities(pool, iid).await?;
    let budget_csv = build_budget(pool, iid).await?;
    let planning_csv = build_planning(pool, iid).await?;

    {
        let mut zip = ZipWriter::new(std::io::Cursor::new(&mut buf));
        zip_add_utf8(&mut zip, "summary_household.csv", household_csv)?;
        zip_add_utf8(&mut zip, "summary_people.csv", people_csv)?;
        zip_add_utf8(&mut zip, "categories.csv", categories_csv)?;
        zip_add_utf8(&mut zip, "assets.csv", assets_csv)?;
        zip_add_utf8(&mut zip, "liabilities.csv", liabilities_csv)?;
        zip_add_utf8(&mut zip, "budget.csv", budget_csv)?;
        zip_add_utf8(&mut zip, "planning.csv", planning_csv)?;
        zip.finish().map_err(zip_err)?;
    }

    Ok(buf)
}

async fn build_summary_household(pool: &PgPool, iid: Uuid) -> Result<String, ApiError> {
    let row: Option<(Uuid, String, String, bool, Option<i16>, String)> = sqlx::query_as(
        r#"SELECT id, base_currency, calendar_tz, projection_includes_inflation,
                  projection_target_age, show_age_mode
           FROM installation WHERE id = $1"#,
    )
    .bind(iid)
    .fetch_optional(pool)
    .await?;

    let mut out = String::new();
    out.push_str(&csv_line([
        "id".into(),
        "base_currency".into(),
        "calendar_tz".into(),
        "projection_includes_inflation".into(),
        "projection_target_age".into(),
        "show_age_mode".into(),
    ]));
    if let Some((id, cur, tz, infl, age, mode)) = row {
        out.push_str(&csv_line([
            csv_cell(&id.to_string()),
            csv_cell(&cur),
            csv_cell(&tz),
            csv_cell(&infl.to_string()),
            csv_cell(&age.map(|a| a.to_string()).unwrap_or_default()),
            csv_cell(&mode),
        ]));
    }
    Ok(out)
}

async fn build_summary_people(pool: &PgPool, iid: Uuid) -> Result<String, ApiError> {
    let rows: Vec<(Uuid, String, bool, Option<NaiveDate>, i32)> = sqlx::query_as(
        r#"SELECT id, display_name, is_primary, birth_date, sort_index
           FROM persons WHERE installation_id = $1
           ORDER BY sort_index ASC, display_name ASC"#,
    )
    .bind(iid)
    .fetch_all(pool)
    .await?;

    let mut out = String::new();
    out.push_str(&csv_line([
        "id".into(),
        "installation_id".into(),
        "display_name".into(),
        "is_primary".into(),
        "birth_date".into(),
        "sort_index".into(),
    ]));
    for (id, name, prim, bd, si) in rows {
        out.push_str(&csv_line([
            csv_cell(&id.to_string()),
            csv_cell(&iid.to_string()),
            csv_cell(&name),
            csv_cell(&prim.to_string()),
            csv_cell(&opt_date(bd)),
            csv_cell(&si.to_string()),
        ]));
    }
    Ok(out)
}

async fn build_categories(pool: &PgPool, iid: Uuid) -> Result<String, ApiError> {
    let rows: Vec<(Uuid, String, String, i32)> = sqlx::query_as(
        r#"SELECT id, scope, name, sort_index FROM categories
           WHERE installation_id = $1 ORDER BY scope ASC, sort_index ASC, name ASC"#,
    )
    .bind(iid)
    .fetch_all(pool)
    .await?;

    let mut out = String::new();
    out.push_str(&csv_line([
        "id".into(),
        "installation_id".into(),
        "scope".into(),
        "name".into(),
        "sort_index".into(),
    ]));
    for (id, scope, name, si) in rows {
        out.push_str(&csv_line([
            csv_cell(&id.to_string()),
            csv_cell(&iid.to_string()),
            csv_cell(&scope),
            csv_cell(&name),
            csv_cell(&si.to_string()),
        ]));
    }
    Ok(out)
}

async fn build_assets(pool: &PgPool, iid: Uuid) -> Result<String, ApiError> {
    let rows: Vec<(
        Uuid,
        Uuid,
        String,
        Decimal,
        Option<Decimal>,
        bool,
        Option<Decimal>,
        Decimal,
        String,
        Decimal,
        Option<String>,
        i32,
        Option<Uuid>,
    )> = sqlx::query_as(
        r#"SELECT id, category_id, name, current_value, purchase_price, is_liquid,
                  expected_annual_return_percent,
                  monthly_contribution_fixed, contribution_frequency, contribution_remainder_weight,
                  notes, sort_index, owner_user_id
           FROM assets WHERE installation_id = $1
           ORDER BY sort_index ASC, name ASC"#,
    )
    .bind(iid)
    .fetch_all(pool)
    .await?;

    let mut out = String::new();
    out.push_str(&csv_line([
        "id".into(),
        "installation_id".into(),
        "category_id".into(),
        "name".into(),
        "current_value".into(),
        "purchase_price".into(),
        "is_liquid".into(),
        "expected_annual_return_percent".into(),
        "monthly_contribution_fixed".into(),
        "contribution_frequency".into(),
        "contribution_remainder_weight".into(),
        "notes".into(),
        "sort_index".into(),
        "owner_user_id".into(),
    ]));
    for (id, cid, name, cv, pp, liq, ear, mcf, cfreq, crw, notes, si, owner) in rows {
        out.push_str(&csv_line([
            csv_cell(&id.to_string()),
            csv_cell(&iid.to_string()),
            csv_cell(&cid.to_string()),
            csv_cell(&name),
            csv_cell(&cv.to_string()),
            csv_cell(&opt_dec(pp)),
            csv_cell(&liq.to_string()),
            csv_cell(&opt_dec(ear)),
            csv_cell(&mcf.to_string()),
            csv_cell(&cfreq),
            csv_cell(&crw.to_string()),
            csv_cell(&notes.unwrap_or_default()),
            csv_cell(&si.to_string()),
            csv_cell(&opt_uuid(owner)),
        ]));
    }
    Ok(out)
}

async fn build_liabilities(pool: &PgPool, iid: Uuid) -> Result<String, ApiError> {
    let rows: Vec<(
        Uuid,
        Uuid,
        String,
        Option<String>,
        Decimal,
        bool,
        Option<Decimal>,
        Option<Decimal>,
        Option<String>,
        Option<NaiveDate>,
        Option<String>,
        i32,
        Option<Uuid>,
    )> = sqlx::query_as(
        r#"SELECT id, category_id, label, type_tag, principal, principal_derived_from_plan,
                  apr_percent, payment_amount, payment_frequency, payment_end_date, notes,
                  sort_index, owner_user_id
           FROM liabilities WHERE installation_id = $1
           ORDER BY sort_index ASC, label ASC"#,
    )
    .bind(iid)
    .fetch_all(pool)
    .await?;

    let mut out = String::new();
    out.push_str(&csv_line([
        "id".into(),
        "installation_id".into(),
        "category_id".into(),
        "label".into(),
        "type_tag".into(),
        "principal".into(),
        "principal_derived_from_plan".into(),
        "apr_percent".into(),
        "payment_amount".into(),
        "payment_frequency".into(),
        "payment_end_date".into(),
        "notes".into(),
        "sort_index".into(),
        "owner_user_id".into(),
    ]));
    for (
        id,
        cid,
        label,
        tt,
        pr,
        derive,
        apr,
        pamt,
        pfreq,
        pend,
        notes,
        si,
        owner,
    ) in rows
    {
        out.push_str(&csv_line([
            csv_cell(&id.to_string()),
            csv_cell(&iid.to_string()),
            csv_cell(&cid.to_string()),
            csv_cell(&label),
            csv_cell(&tt.unwrap_or_default()),
            csv_cell(&pr.to_string()),
            csv_cell(&derive.to_string()),
            csv_cell(&opt_dec(apr)),
            csv_cell(&opt_dec(pamt)),
            csv_cell(&pfreq.unwrap_or_default()),
            csv_cell(&opt_date(pend)),
            csv_cell(&notes.unwrap_or_default()),
            csv_cell(&si.to_string()),
            csv_cell(&opt_uuid(owner)),
        ]));
    }
    Ok(out)
}

async fn build_budget(pool: &PgPool, iid: Uuid) -> Result<String, ApiError> {
    let rows: Vec<(Uuid, Uuid, String, Decimal, Option<String>, i32, Option<Uuid>)> =
        sqlx::query_as(
            r#"SELECT be.id, be.category_id, c.scope, be.amount,
                      be.notes, be.sort_index, be.owner_user_id
               FROM budget_entries be
               JOIN categories c ON c.id = be.category_id
               WHERE be.installation_id = $1
               ORDER BY be.sort_index ASC"#,
        )
        .bind(iid)
        .fetch_all(pool)
        .await?;

    let mut out = String::new();
    out.push_str(&csv_line([
        "id".into(),
        "installation_id".into(),
        "category_id".into(),
        "category_scope".into(),
        "amount_monthly".into(),
        "notes".into(),
        "sort_index".into(),
        "owner_user_id".into(),
    ]));
    for (id, cid, scope, amt, notes, si, owner) in rows {
        out.push_str(&csv_line([
            csv_cell(&id.to_string()),
            csv_cell(&iid.to_string()),
            csv_cell(&cid.to_string()),
            csv_cell(&scope),
            csv_cell(&amt.to_string()),
            csv_cell(&notes.unwrap_or_default()),
            csv_cell(&si.to_string()),
            csv_cell(&opt_uuid(owner)),
        ]));
    }
    Ok(out)
}

async fn build_planning(pool: &PgPool, iid: Uuid) -> Result<String, ApiError> {
    let rows: Vec<(Uuid, Uuid, String, String, Decimal, Option<NaiveDate>, Option<String>, i32, Option<Uuid>)> =
        sqlx::query_as(
            r#"SELECT pf.id, pf.category_id, c.scope, pf.title, pf.expected_amount,
                      pf.due_date, pf.notes, pf.sort_index, pf.owner_user_id
               FROM planning_flows pf
               JOIN categories c ON c.id = pf.category_id
               WHERE pf.installation_id = $1
               ORDER BY pf.sort_index ASC, pf.title ASC"#,
        )
        .bind(iid)
        .fetch_all(pool)
        .await?;

    let mut out = String::new();
    out.push_str(&csv_line([
        "id".into(),
        "installation_id".into(),
        "category_id".into(),
        "direction".into(),
        "title".into(),
        "expected_amount".into(),
        "due_date".into(),
        "notes".into(),
        "sort_index".into(),
        "owner_user_id".into(),
    ]));
    for (id, cid, scope, title, amt, due, notes, si, owner) in rows {
        let dir = match scope.as_str() {
            "income" => "inflow",
            _ => "outflow",
        };
        out.push_str(&csv_line([
            csv_cell(&id.to_string()),
            csv_cell(&iid.to_string()),
            csv_cell(&cid.to_string()),
            csv_cell(dir),
            csv_cell(&title),
            csv_cell(&amt.to_string()),
            csv_cell(&opt_date(due)),
            csv_cell(&notes.unwrap_or_default()),
            csv_cell(&si.to_string()),
            csv_cell(&opt_uuid(owner)),
        ]));
    }
    Ok(out)
}
