//! Fase 3 — escritura segura: idempotencia del alta manual, techo de concurrencia de la
//! proyección, preview honesto de `delete_liability` y error tipado al confundir una cuota
//! derivada con una partida de presupuesto.
//!
//! Los cuatro fallos que cubre comparten una firma: **el servidor respondía «ok» (o «no existe»)
//! sobre una situación que no era esa**, y quien llamaba —una SPA, o un agente— no tenía forma de
//! enterarse. En los modos que usan transacciones, un movimiento duplicado por un reintento mueve
//! el ahorro mensual del motor y con él la fecha de jubilación proyectada.

mod common;

use common::{LoggedInOwner, TestApp};
use serde_json::{json, Value};

fn dec(v: &Value) -> f64 {
    v.as_str()
        .unwrap_or_else(|| panic!("se esperaba un decimal como string, llegó {v:?}"))
        .parse()
        .expect("decimal")
}

async fn setup(app: &TestApp) -> (LoggedInOwner, String) {
    let owner = app.register_and_login_owner("alice").await;
    let cat = app.create_category(&owner, "expense", "Ocio").await;
    (owner, cat)
}

fn expense_body(cat: &str, key: Option<&str>) -> Value {
    let mut b = json!({
        "op_date": "2026-06-10", "concept": "Cine", "amount": "-12.50",
        "kind": "expense", "category_id": cat,
    });
    if let Some(k) = key {
        b["idempotency_key"] = json!(k);
    }
    b
}

// ---------------------------------------------------------------------------
// TAREA 1 — idempotencia opt-in del alta manual
// ---------------------------------------------------------------------------

/// El contrato histórico NO cambia: sin clave, dos POST idénticos son dos movimientos. Los
/// duplicados legítimos existen y el `fingerprint_ordinal` está para eso.
#[tokio::test]
async fn sin_clave_el_reenvio_sigue_creando_otro_movimiento() {
    let app = TestApp::spawn().await;
    let (owner, cat) = setup(&app).await;

    let a = app
        .post_json_with_cookie("/v1/transactions", expense_body(&cat, None), &owner.cookie)
        .await;
    let b = app
        .post_json_with_cookie("/v1/transactions", expense_body(&cat, None), &owner.cookie)
        .await;
    assert_eq!(a.status, http::StatusCode::CREATED, "{a:?}");
    assert_eq!(b.status, http::StatusCode::CREATED, "{b:?}");
    assert_ne!(a.json()["id"], b.json()["id"], "sin clave son dos altas");
    assert_eq!(app.count_rows("transactions").await, 2);
    assert_eq!(
        app.count_rows("transaction_idempotency_keys").await,
        0,
        "sin clave no se toca la tabla nueva"
    );
}

/// Con la misma clave y el mismo cuerpo: la fila original, sin crear nada. La respuesta es la
/// misma, id incluido — esa igualdad ES la idempotencia.
#[tokio::test]
async fn misma_clave_y_mismo_cuerpo_devuelve_el_movimiento_original() {
    let app = TestApp::spawn().await;
    let (owner, cat) = setup(&app).await;

    let a = app
        .post_json_with_cookie(
            "/v1/transactions",
            expense_body(&cat, Some("k-cine-1")),
            &owner.cookie,
        )
        .await;
    assert_eq!(a.status, http::StatusCode::CREATED, "{a:?}");

    let b = app
        .post_json_with_cookie(
            "/v1/transactions",
            expense_body(&cat, Some("k-cine-1")),
            &owner.cookie,
        )
        .await;
    assert_eq!(b.status, http::StatusCode::CREATED, "el replay no es un error: {b:?}");
    assert_eq!(a.json()["id"], b.json()["id"], "debe volver la MISMA fila");
    assert_eq!(a.json(), b.json(), "la respuesta del replay es la original");
    assert_eq!(app.count_rows("transactions").await, 1, "no se creó una segunda fila");
}

/// La misma clave con OTRO cuerpo es un 409: gana el primero. Devolver la fila original diría
/// «tu segundo movimiento se creó» —y ese gasto faltaría—; crear otra anularía la clave.
#[tokio::test]
async fn misma_clave_con_cuerpo_distinto_es_conflicto_y_gana_el_primero() {
    let app = TestApp::spawn().await;
    let (owner, cat) = setup(&app).await;

    let a = app
        .post_json_with_cookie(
            "/v1/transactions",
            expense_body(&cat, Some("k-1")),
            &owner.cookie,
        )
        .await;
    assert_eq!(a.status, http::StatusCode::CREATED, "{a:?}");

    let mut otro = expense_body(&cat, Some("k-1"));
    otro["amount"] = json!("-99.00");
    let b = app
        .post_json_with_cookie("/v1/transactions", otro, &owner.cookie)
        .await;
    assert_eq!(b.status, http::StatusCode::CONFLICT, "{b:?}");
    assert_eq!(b.json()["code"], "idempotency_key_conflict");
    assert!(
        b.json()["message"]
            .as_str()
            .unwrap()
            .contains(a.json()["id"].as_str().unwrap()),
        "el mensaje debe nombrar la fila que ocupa la clave: {:?}",
        b.json()
    );
    assert_eq!(app.count_rows("transactions").await, 1, "el primero sobrevive intacto");
}

/// Mismo movimiento escrito de otra forma (`-12.50` vs `-12.5`, concepto con espacios de sobra)
/// es el MISMO reintento: la huella se calcula sobre los valores ya normalizados.
#[tokio::test]
async fn la_huella_ignora_la_forma_y_solo_mira_el_movimiento() {
    let app = TestApp::spawn().await;
    let (owner, cat) = setup(&app).await;

    let a = app
        .post_json_with_cookie(
            "/v1/transactions",
            expense_body(&cat, Some("k-forma")),
            &owner.cookie,
        )
        .await;
    assert_eq!(a.status, http::StatusCode::CREATED, "{a:?}");

    let b = app
        .post_json_with_cookie(
            "/v1/transactions",
            json!({
                "op_date": "2026-06-10", "concept": "  Cine  ", "amount": "-12.5",
                "kind": "expense", "category_id": cat, "idempotency_key": "k-forma",
            }),
            &owner.cookie,
        )
        .await;
    assert_eq!(b.status, http::StatusCode::CREATED, "debe reproducir, no chocar: {b:?}");
    assert_eq!(a.json()["id"], b.json()["id"]);
    assert_eq!(app.count_rows("transactions").await, 1);
}

/// La clave es POR USUARIO. Dos miembros pueden elegir la misma cadena sin verse: con ámbito de
/// instalación, la clave de Bob le habría devuelto el movimiento de Alice.
#[tokio::test]
async fn la_clave_no_cruza_entre_miembros() {
    let app = TestApp::spawn().await;
    let (owner, cat) = setup(&app).await;
    let bob = app.register_and_approve_member(&owner, "bob", "member").await;

    let a = app
        .post_json_with_cookie(
            "/v1/transactions",
            expense_body(&cat, Some("misma-clave")),
            &owner.cookie,
        )
        .await;
    let b = app
        .post_json_with_cookie(
            "/v1/transactions",
            expense_body(&cat, Some("misma-clave")),
            &bob.cookie,
        )
        .await;
    assert_eq!(a.status, http::StatusCode::CREATED, "{a:?}");
    assert_eq!(b.status, http::StatusCode::CREATED, "{b:?}");
    assert_ne!(a.json()["id"], b.json()["id"], "cada uno crea el suyo");
    assert_eq!(app.count_rows("transactions").await, 2);
}

/// Una clave en blanco es un error, no un «desactívala en silencio»: quien manda `" "` cree que
/// está protegido.
#[tokio::test]
async fn una_clave_vacia_o_gigante_es_un_400_explicito() {
    let app = TestApp::spawn().await;
    let (owner, cat) = setup(&app).await;

    for k in ["   ", &"x".repeat(201)] {
        let r = app
            .post_json_with_cookie("/v1/transactions", expense_body(&cat, Some(k)), &owner.cookie)
            .await;
        assert_eq!(r.status, http::StatusCode::BAD_REQUEST, "{r:?}");
        assert_eq!(r.json()["code"], "idempotency_key_invalid");
    }
    assert_eq!(app.count_rows("transactions").await, 0);
}

/// El lote RECHAZA la clave en vez de ignorarla: un lote es todo-o-nada, y aceptar el campo para
/// tirarlo dejaría al llamante creyéndose protegido.
#[tokio::test]
async fn el_lote_rechaza_la_clave_en_vez_de_tragarsela() {
    let app = TestApp::spawn().await;
    let (owner, cat) = setup(&app).await;

    let r = app
        .post_json_with_cookie(
            "/v1/transactions/batch",
            json!({ "transactions": [expense_body(&cat, Some("k"))] }),
            &owner.cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::BAD_REQUEST, "{r:?}");
    assert_eq!(r.json()["code"], "idempotency_key_batch_unsupported");
    assert_eq!(app.count_rows("transactions").await, 0);

    // …y sin clave el lote sigue funcionando exactamente igual que antes.
    let ok = app
        .post_json_with_cookie(
            "/v1/transactions/batch",
            json!({ "transactions": [expense_body(&cat, None)] }),
            &owner.cookie,
        )
        .await;
    assert_eq!(ok.status, http::StatusCode::CREATED, "{ok:?}");
    assert_eq!(app.count_rows("transactions").await, 1);
}

/// Un replay NO debe crear una segunda regla recurrente: el camino de réplica sale antes de
/// tocar nada.
#[tokio::test]
async fn el_replay_no_duplica_la_regla_recurrente() {
    let app = TestApp::spawn().await;
    let (owner, cat) = setup(&app).await;

    let mut body = expense_body(&cat, Some("k-rec"));
    body["recurrence"] = json!({});
    let a = app
        .post_json_with_cookie("/v1/transactions", body.clone(), &owner.cookie)
        .await;
    assert_eq!(a.status, http::StatusCode::CREATED, "{a:?}");
    let b = app
        .post_json_with_cookie("/v1/transactions", body, &owner.cookie)
        .await;
    assert_eq!(b.status, http::StatusCode::CREATED, "{b:?}");
    assert_eq!(a.json()["id"], b.json()["id"]);
    assert_eq!(app.count_rows("recurring_transaction_rules").await, 1);
}

/// Borrar el movimiento se lleva su clave (`ON DELETE CASCADE`): borrar es una intención
/// posterior y explícita, no un reintento, así que después se puede volver a crear.
#[tokio::test]
async fn borrar_el_movimiento_libera_su_clave() {
    let app = TestApp::spawn().await;
    let (owner, cat) = setup(&app).await;

    let a = app
        .post_json_with_cookie(
            "/v1/transactions",
            expense_body(&cat, Some("k-borrado")),
            &owner.cookie,
        )
        .await;
    let id = a.json()["id"].as_str().unwrap().to_string();
    assert_eq!(app.count_rows("transaction_idempotency_keys").await, 1);

    let d = app
        .delete_with_cookie(&format!("/v1/transactions/{id}"), &owner.cookie)
        .await;
    assert_eq!(d.status, http::StatusCode::NO_CONTENT, "{d:?}");
    assert_eq!(
        app.count_rows("transaction_idempotency_keys").await,
        0,
        "la clave cae con su movimiento"
    );

    let again = app
        .post_json_with_cookie(
            "/v1/transactions",
            expense_body(&cat, Some("k-borrado")),
            &owner.cookie,
        )
        .await;
    assert_eq!(again.status, http::StatusCode::CREATED, "{again:?}");
    assert_ne!(again.json()["id"].as_str().unwrap(), id);
}

/// La poda es perezosa y vive en la escritura (D5: nunca en un GET). Una clave envejecida a mano
/// desaparece en el siguiente POST con clave y deja de proteger.
#[tokio::test]
async fn las_claves_caducan_y_las_poda_el_propio_post() {
    let app = TestApp::spawn().await;
    let (owner, cat) = setup(&app).await;

    let a = app
        .post_json_with_cookie(
            "/v1/transactions",
            expense_body(&cat, Some("k-vieja")),
            &owner.cookie,
        )
        .await;
    assert_eq!(a.status, http::StatusCode::CREATED, "{a:?}");

    sqlx::query("UPDATE transaction_idempotency_keys SET created_at = now() - interval '48 hours'")
        .execute(&app.pool)
        .await
        .expect("envejecer la clave");

    let b = app
        .post_json_with_cookie(
            "/v1/transactions",
            expense_body(&cat, Some("k-vieja")),
            &owner.cookie,
        )
        .await;
    assert_eq!(b.status, http::StatusCode::CREATED, "{b:?}");
    assert_ne!(
        a.json()["id"], b.json()["id"],
        "pasada la retención la clave ya no protege: se crea un movimiento nuevo"
    );
    assert_eq!(app.count_rows("transactions").await, 2);
    assert_eq!(
        app.count_rows("transaction_idempotency_keys").await,
        1,
        "la caducada se podó y quedó la nueva"
    );
}

// ---------------------------------------------------------------------------
// TAREA 3 — el preview de delete_liability cuenta la partida que desaparece
// ---------------------------------------------------------------------------

async fn liability_with_quota(app: &TestApp, owner: &LoggedInOwner, amount: &str) -> String {
    let cat = app.create_category(owner, "liability", "Préstamo").await;
    let exp_cat = app.create_category(owner, "expense", "Hogar").await;
    let r = app
        .post_json_with_cookie(
            "/v1/liabilities",
            json!({
                "category_id": cat, "expense_category_id": exp_cat, "label": "Hipoteca",
                "principal": "150000", "payment_amount": amount, "payment_frequency": "monthly",
            }),
            &owner.cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::CREATED, "{r:?}");
    r.json()["id"].as_str().unwrap().to_string()
}

/// El preview debe contar la cuota que se va del presupuesto y su efecto en los totales — el
/// efecto que callaba, y el único que se mide en cientos de euros al mes.
#[tokio::test]
async fn el_preview_del_borrado_de_pasivo_cuenta_la_cuota_del_presupuesto() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let liab = liability_with_quota(&app, &owner, "600").await;
    let liab_uuid = uuid::Uuid::parse_str(&liab).unwrap();
    let iid = app.installation_id().await;

    let antes = app.get_with_cookie("/v1/budget", &owner.cookie).await.json();
    let gasto_antes = dec(&antes["totals"]["expense_regular_monthly_equivalent"]);

    let effects = futurefin_api::handlers::liabilities::liability_delete_effects(
        &app.pool,
        iid,
        owner.user_id,
        liab_uuid,
    )
    .await
    .expect("efectos del borrado");
    let v = serde_json::to_value(&effects).unwrap();

    let removed = &v["budget_entry_removed"];
    assert!(!removed.is_null(), "la cuota derivada debe aparecer: {v}");
    assert_eq!(removed["label"], "Hipoteca");
    assert!((dec(&removed["monthly_amount"]) - 600.0).abs() < 0.01, "{v}");
    assert!(
        (dec(&removed["expense_monthly_before"]) - gasto_antes).abs() < 0.01,
        "el «antes» debe coincidir con lo que sirve /v1/budget: {v}"
    );
    assert!(
        (dec(&removed["expense_monthly_before"]) - dec(&removed["expense_monthly_after"]) - 600.0)
            .abs()
            < 0.01,
        "el gasto debe bajar exactamente la cuota: {v}"
    );
    assert!(
        (dec(&removed["net_monthly_after"]) - dec(&removed["net_monthly_before"]) - 600.0).abs()
            < 0.01,
        "el neto sube justo lo que deja de pagarse: {v}"
    );

    // …y el «después» del preview es el presupuesto real tras confirmar.
    let d = app
        .delete_with_cookie(&format!("/v1/liabilities/{liab}"), &owner.cookie)
        .await;
    assert_eq!(d.status, http::StatusCode::NO_CONTENT, "{d:?}");
    let despues = app.get_with_cookie("/v1/budget", &owner.cookie).await.json();
    assert!(
        (dec(&despues["totals"]["expense_regular_monthly_equivalent"])
            - dec(&removed["expense_monthly_after"]))
        .abs()
            < 0.01,
        "el preview prometió un total que el borrado no cumplió"
    );
}

/// Un pasivo SIN plan de pago no genera cuota: decir «desaparece una partida» sería mentir en la
/// otra dirección, así que el campo queda ausente.
#[tokio::test]
async fn un_pasivo_sin_plan_de_pago_no_promete_ninguna_partida() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let cat = app.create_category(&owner, "liability", "Préstamo").await;
    let exp_cat = app.create_category(&owner, "expense", "Hogar").await;
    let r = app
        .post_json_with_cookie(
            "/v1/liabilities",
            json!({ "category_id": cat, "expense_category_id": exp_cat,
                    "label": "Deuda suelta", "principal": "1000" }),
            &owner.cookie,
        )
        .await;
    assert_eq!(r.status, http::StatusCode::CREATED, "{r:?}");
    let id = uuid::Uuid::parse_str(r.json()["id"].as_str().unwrap()).unwrap();

    let effects = futurefin_api::handlers::liabilities::liability_delete_effects(
        &app.pool,
        app.installation_id().await,
        owner.user_id,
        id,
    )
    .await
    .expect("efectos");
    let v = serde_json::to_value(&effects).unwrap();
    assert!(v.get("budget_entry_removed").is_none(), "{v}");
    assert_eq!(v["transactions_unlinked"], 0);
}

// ---------------------------------------------------------------------------
// TAREA 4 — el id de una cuota derivada NO es una partida de presupuesto
// ---------------------------------------------------------------------------

/// `GET /v1/budget` publica la cuota con el UUID de su PASIVO. Pasar ese id a
/// `DELETE /v1/budget/entries/{id}` daba un 404 pelado —«no existe»— sobre un id que el llamante
/// acababa de leer en una respuesta nuestra. Ahora es un 422 que dice dónde ir.
#[tokio::test]
async fn borrar_una_cuota_derivada_como_partida_remite_al_pasivo() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let liab = liability_with_quota(&app, &owner, "600").await;

    let budget = app.get_with_cookie("/v1/budget", &owner.cookie).await.json();
    let cuota = budget["entries"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["source"] == "liability")
        .expect("la cuota derivada está en el presupuesto");
    assert_eq!(cuota["id"].as_str().unwrap(), liab, "comparte UUID con su pasivo");

    let d = app
        .delete_with_cookie(&format!("/v1/budget/entries/{liab}"), &owner.cookie)
        .await;
    assert_eq!(d.status, http::StatusCode::UNPROCESSABLE_ENTITY, "{d:?}");
    assert_eq!(d.json()["code"], "budget_entry_is_liability_derived");
    assert!(
        d.json()["message"].as_str().unwrap().contains("update_liability"),
        "el error debe remitir a la tool correcta: {:?}",
        d.json()
    );

    // El PATCH cae en la misma trampa y recibe el mismo diagnóstico.
    let p = app
        .patch_json_with_cookie(
            &format!("/v1/budget/entries/{liab}"),
            json!({ "amount": "10" }),
            &owner.cookie,
        )
        .await;
    assert_eq!(p.status, http::StatusCode::UNPROCESSABLE_ENTITY, "{p:?}");
    assert_eq!(p.json()["code"], "budget_entry_is_liability_derived");

    // …y el pasivo sigue ahí: el error no borró nada por el camino.
    let despues = app.get_with_cookie("/v1/budget", &owner.cookie).await.json();
    assert_eq!(
        despues["entries"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|e| e["source"] == "liability")
            .count(),
        1
    );
}

/// Un id inventado sigue siendo un 404: el 422 nuevo describe una situación concreta y no puede
/// tragarse el «no existe» de siempre.
#[tokio::test]
async fn un_id_inventado_sigue_siendo_404() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;
    let fake = uuid::Uuid::new_v4();

    let d = app
        .delete_with_cookie(&format!("/v1/budget/entries/{fake}"), &owner.cookie)
        .await;
    assert_eq!(d.status, http::StatusCode::NOT_FOUND, "{d:?}");
    let p = app
        .patch_json_with_cookie(
            &format!("/v1/budget/entries/{fake}"),
            json!({ "amount": "10" }),
            &owner.cookie,
        )
        .await;
    assert_eq!(p.status, http::StatusCode::NOT_FOUND, "{p:?}");
}

// ---------------------------------------------------------------------------
// TAREA 2 — el techo de concurrencia no puede tocar el resultado
// ---------------------------------------------------------------------------

/// El semáforo acota CUÁNTAS simulaciones corren a la vez, jamás QUÉ devuelven. N proyecciones
/// concurrentes (con `?months=`, que salta la cache por diseño y por tanto simula de verdad cada
/// vez) deben terminar todas y coincidir hasta el último dígito.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn el_techo_de_concurrencia_no_cambia_ni_un_numero() {
    let app = TestApp::spawn().await;
    let owner = app.register_and_login_owner("alice").await;

    let esperado = app
        .get_with_cookie("/v1/projection/series?months=120", &owner.cookie)
        .await;
    assert_eq!(esperado.status, http::StatusCode::OK, "{esperado:?}");
    let esperado = esperado.json();

    // Ocho en vuelo DE VERDAD (`tokio::join!` las poliniza a la vez): el semáforo tiene entre 2 y
    // 8 permisos y cada petición pide dos, así que aquí hay cola garantizada. Ninguna puede
    // quedarse colgada ni devolver otra cosa.
    let u = "/v1/projection/series?months=120";
    let (r1, r2, r3, r4, r5, r6, r7, r8) = tokio::join!(
        app.get_with_cookie(u, &owner.cookie),
        app.get_with_cookie(u, &owner.cookie),
        app.get_with_cookie(u, &owner.cookie),
        app.get_with_cookie(u, &owner.cookie),
        app.get_with_cookie(u, &owner.cookie),
        app.get_with_cookie(u, &owner.cookie),
        app.get_with_cookie(u, &owner.cookie),
        app.get_with_cookie(u, &owner.cookie),
    );
    for r in [r1, r2, r3, r4, r5, r6, r7, r8] {
        assert_eq!(r.status, http::StatusCode::OK, "{r:?}");
        assert_eq!(r.json(), esperado, "el techo no debe alterar la serie");
    }

    // …y la proyección CACHEADA (sin `?months=`) sigue respondiendo mientras tanto: el permiso
    // envuelve la simulación, no el handler, así que un HIT no espera a nadie.
    let cacheada = app.get_with_cookie("/v1/projection/series", &owner.cookie).await;
    assert_eq!(cacheada.status, http::StatusCode::OK, "{cacheada:?}");
}
