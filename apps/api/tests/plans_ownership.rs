//! 6.0.0 F2-1 — quién puede leer y escribir un plan.
//!
//! El contrato es el del ledger (D21/D23): un plan es de UNA persona y solo ella lo escribe —ni
//! siquiera el `owner` de la instalación (403 `not_row_owner`)—; un viewer no escribe nada (403
//! por ROL); `view=household` lee los planes de todos y es de solo lectura. Y lo nuevo de 6.0:
//! los planes de HOGAR (`owner_user_id` NULL) existen en el esquema pero la API no los crea ni
//! los escribe hasta F3 (400 `household_plans_not_available`).

mod common;

use common::{LoggedInOwner, TestApp};
use serde_json::{json, Value};

async fn create_plan(app: &TestApp, who: &LoggedInOwner, name: &str) -> String {
    let r = app
        .post_json_with_cookie("/v1/plans", json!({ "name": name }), &who.cookie)
        .await;
    assert_eq!(r.status, http::StatusCode::CREATED, "{r:?}");
    r.json()["id"].as_str().unwrap().to_string()
}

fn ids(list: &Value) -> Vec<String> {
    list["plans"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["id"].as_str().unwrap().to_string())
        .collect()
}

#[tokio::test]
async fn nobody_writes_another_members_plan_not_even_the_owner() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("ana").await;
    let member = app
        .register_and_approve_member(&owner, "bea", "member")
        .await;
    let theirs = create_plan(&app, &member, "De Bea").await;
    let block = app
        .get_with_cookie(&format!("/v1/plans/{theirs}?view=household"), &owner.cookie)
        .await
        .json()["blocks"][0]["id"]
        .as_str()
        .unwrap()
        .to_string();
    let change = app
        .post_json_with_cookie(
            &format!("/v1/plans/{theirs}/changes"),
            json!({ "kind": "expense", "start_date": "2030-01-01", "amount": "50" }),
            &member.cookie,
        )
        .await;
    assert_eq!(change.status, http::StatusCode::CREATED, "{change:?}");
    let change_id = change.json()["id"].as_str().unwrap().to_string();

    let attempts = [
        app.patch_json_with_cookie(
            &format!("/v1/plans/{theirs}"),
            json!({ "name": "Mío" }),
            &owner.cookie,
        )
        .await,
        app.delete_with_cookie(&format!("/v1/plans/{theirs}"), &owner.cookie)
            .await,
        app.post_json_with_cookie(
            &format!("/v1/plans/{theirs}/activate"),
            json!({}),
            &owner.cookie,
        )
        .await,
        app.post_json_with_cookie(
            &format!("/v1/plans/{theirs}/blocks"),
            json!({ "name": "Intruso" }),
            &owner.cookie,
        )
        .await,
        app.patch_json_with_cookie(
            &format!("/v1/plans/{theirs}/blocks/{block}"),
            json!({ "enabled": false }),
            &owner.cookie,
        )
        .await,
        app.delete_with_cookie(&format!("/v1/plans/{theirs}/blocks/{block}"), &owner.cookie)
            .await,
        app.post_json_with_cookie(
            &format!("/v1/plans/{theirs}/changes"),
            json!({ "kind": "income", "start_date": "2030-01-01", "amount": "1" }),
            &owner.cookie,
        )
        .await,
        app.patch_json_with_cookie(
            &format!("/v1/plans/{theirs}/changes/{change_id}"),
            json!({ "amount": "1" }),
            &owner.cookie,
        )
        .await,
        app.delete_with_cookie(
            &format!("/v1/plans/{theirs}/changes/{change_id}"),
            &owner.cookie,
        )
        .await,
    ];
    for r in attempts {
        assert_eq!(r.status, http::StatusCode::FORBIDDEN, "{r:?}");
        assert_eq!(r.json()["code"], "not_row_owner", "{r:?}");
    }

    let plan = app
        .get_with_cookie(&format!("/v1/plans/{theirs}"), &member.cookie)
        .await
        .json();
    assert_eq!(plan["name"], "De Bea");
    assert_eq!(
        plan["revision"], 2,
        "ningún intento ajeno subió la revisión"
    );
    assert_eq!(plan["is_active"], false);
    assert_eq!(plan["blocks"].as_array().unwrap().len(), 1);

    // Un id que no existe sigue siendo 404, no 403: el cliente no pierde la señal de «ya no está».
    let ghost = app
        .patch_json_with_cookie(
            "/v1/plans/00000000-0000-0000-0000-000000000001",
            json!({ "name": "X" }),
            &member.cookie,
        )
        .await;
    assert_eq!(ghost.status, http::StatusCode::NOT_FOUND);
    assert_eq!(ghost.json()["code"], "plan_not_found");
}

#[tokio::test]
async fn a_viewer_cannot_write_plans() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("ana").await;
    let viewer = app
        .register_and_approve_member(&owner, "vic", "viewer")
        .await;
    let r = app
        .post_json_with_cookie("/v1/plans", json!({ "name": "X" }), &viewer.cookie)
        .await;
    assert_eq!(r.status, http::StatusCode::FORBIDDEN, "{r:?}");
    assert_eq!(r.json()["code"], "forbidden", "motivo: el ROL, no la fila");
    let off = app
        .post_json_with_cookie("/v1/plans/deactivate", json!({}), &viewer.cookie)
        .await;
    assert_eq!(off.status, http::StatusCode::FORBIDDEN);
    // Leer, sí.
    let list = app
        .get_with_cookie("/v1/plans?view=household", &viewer.cookie)
        .await;
    assert_eq!(list.status, http::StatusCode::OK);
}

#[tokio::test]
async fn mine_is_the_default_and_household_reads_everyone() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("ana").await;
    let member = app
        .register_and_approve_member(&owner, "bea", "member")
        .await;
    let ana = create_plan(&app, &owner, "De Ana").await;
    let bea = create_plan(&app, &member, "De Bea").await;

    let mine = app.get_with_cookie("/v1/plans", &owner.cookie).await.json();
    assert_eq!(mine["view"], "mine");
    assert_eq!(ids(&mine), vec![ana.clone()]);

    let house = app
        .get_with_cookie("/v1/plans?view=household", &owner.cookie)
        .await
        .json();
    assert_eq!(house["view"], "household");
    let mut all = ids(&house);
    all.sort();
    let mut expected = vec![ana.clone(), bea.clone()];
    expected.sort();
    assert_eq!(all, expected);

    let bad = app
        .get_with_cookie("/v1/plans?view=MINE", &owner.cookie)
        .await;
    assert_eq!(bad.status, http::StatusCode::BAD_REQUEST);
    assert_eq!(bad.json()["code"], "invalid_view");

    let not_mine = app
        .get_with_cookie(&format!("/v1/plans/{bea}"), &owner.cookie)
        .await;
    assert_eq!(not_mine.status, http::StatusCode::NOT_FOUND);
    assert_eq!(not_mine.json()["code"], "plan_not_in_scope");
    let read = app
        .get_with_cookie(&format!("/v1/plans/{bea}?view=household"), &owner.cookie)
        .await;
    assert_eq!(read.status, http::StatusCode::OK);
    assert_eq!(read.json()["owner_user_id"], member.user_id.to_string());
}

#[tokio::test]
async fn one_active_plan_per_person_not_per_household() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("ana").await;
    let member = app
        .register_and_approve_member(&owner, "bea", "member")
        .await;
    let ana = create_plan(&app, &owner, "De Ana").await;
    let bea = create_plan(&app, &member, "De Bea").await;
    for (who, id) in [(&owner, &ana), (&member, &bea)] {
        let r = app
            .post_json_with_cookie(&format!("/v1/plans/{id}/activate"), json!({}), &who.cookie)
            .await;
        assert_eq!(r.status, http::StatusCode::OK, "{r:?}");
        assert_eq!(r.json()["deactivated_plan_ids"], json!([]));
    }
    // Desactivar es de la persona que lo pide: el plan de Bea sigue activo.
    let off = app
        .post_json_with_cookie("/v1/plans/deactivate", json!({}), &owner.cookie)
        .await;
    assert_eq!(off.json()["deactivated_plan_ids"], json!([ana]));
    let active: Vec<String> =
        sqlx::query_scalar::<_, uuid::Uuid>("SELECT id FROM plans WHERE is_active")
            .fetch_all(&app.pool)
            .await
            .unwrap()
            .into_iter()
            .map(|u| u.to_string())
            .collect();
    assert_eq!(active, vec![bea]);
}

#[tokio::test]
async fn household_plans_are_not_available_until_f3() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("ana").await;
    let r = app
        .post_json_with_cookie(
            "/v1/plans",
            json!({ "name": "Del hogar", "household": true }),
            &owner.cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::BAD_REQUEST, "{r:?}");
    assert_eq!(r.json()["code"], "household_plans_not_available");

    // Uno sembrado por debajo de la API (el esquema ya lo admite): se lee en el hogar, no en
    // «mine», y no se escribe.
    let iid = app.installation_id().await;
    let hh: uuid::Uuid = sqlx::query_scalar(
        "INSERT INTO plans (installation_id, owner_user_id, name) VALUES ($1, NULL, 'Hogar') RETURNING id",
    )
    .bind(iid)
    .fetch_one(&app.pool)
    .await
    .unwrap();
    let mine = app.get_with_cookie("/v1/plans", &owner.cookie).await.json();
    assert!(ids(&mine).is_empty());
    let house = app
        .get_with_cookie("/v1/plans?view=household", &owner.cookie)
        .await
        .json();
    assert_eq!(ids(&house), vec![hh.to_string()]);
    assert_eq!(house["plans"][0]["owner_user_id"], Value::Null);

    for r in [
        app.patch_json_with_cookie(
            &format!("/v1/plans/{hh}"),
            json!({ "name": "X" }),
            &owner.cookie,
        )
        .await,
        app.post_json_with_cookie(
            &format!("/v1/plans/{hh}/activate"),
            json!({}),
            &owner.cookie,
        )
        .await,
        app.delete_with_cookie(&format!("/v1/plans/{hh}"), &owner.cookie)
            .await,
    ] {
        assert_eq!(r.status, http::StatusCode::BAD_REQUEST, "{r:?}");
        assert_eq!(r.json()["code"], "household_plans_not_available");
    }
}

#[tokio::test]
async fn a_plan_cannot_sell_another_members_asset() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("ana").await;
    let member = app
        .register_and_approve_member(&owner, "bea", "member")
        .await;
    let cat = app.create_category(&owner, "asset", "Inversión").await;
    let theirs = app
        .post_json_with_cookie(
            "/v1/assets",
            json!({ "category_id": cat, "name": "Fondo de Bea", "current_value": "1000" }),
            &member.cookie,
        )
        .await;
    assert_eq!(theirs.status, http::StatusCode::CREATED, "{theirs:?}");
    let plan = create_plan(&app, &owner, "Vender lo ajeno").await;
    let r = app
        .post_json_with_cookie(
            &format!("/v1/plans/{plan}/changes"),
            json!({ "kind": "asset_sale", "start_date": "2030-01-01",
                    "asset_id": theirs.json()["id"] }),
            &owner.cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::BAD_REQUEST, "{r:?}");
    assert_eq!(r.json()["code"], "asset_not_in_scope");
}
