//! Fase 4.6 — Paridad cliente↔servidor sobre el cálculo FIRE.
//!
//! Carga los casos canónicos desde `tests/fixtures/fire-parity.json` y para cada uno:
//!   1. configura una instalación con los `fire_settings` del caso — **repartidos en dos
//!      superficies desde 5.0.0**: lo compartido por el hogar sigue en `PATCH /v1/installation`,
//!      y los cuatro ejes personales (`fire_number_mode`, `fire_number_manual_amount`, `swr_pct`,
//!      `horizon_lifespan_age`) van al perfil de jubilación del usuario (D13). El FIXTURE no
//!      cambia: sigue describiendo UN cálculo FIRE, que es lo que comparte con el frontend; lo
//!      que cambió es dónde vive cada mitad,
//!   2. inserta un asset en estado inicial (mes 0),
//!   3. inserta budget entries que reproducen los `monthly` del caso (income/expense),
//!   4. llama `GET /v1/projection/series`,
//!   5. compara **`fire_number_classic_today`** con `expected_target_nw` ± 1 €.
//!
//! El mismo JSON lo consume el test del frontend (`apps/web/src/lib/fire.test.ts`). Si alguien
//! cambia los tramos fiscales o la fórmula en un lado y no en el otro, uno de los dos suites
//! falla. La fuente de verdad es el JSON.
//!
//! # El campo cambió de nombre en 5.0.0; el NÚMERO no (WP A12)
//!
//! Hasta 4.15.x el campo se llamaba `jubilacion_target_net_worth` y era el objetivo que **disparaba
//! la jubilación**: el motor cruzaba la línea contra él. El modelo v2 retiró ese cruce entero —la
//! fecha la decide el éxito— y lo que queda es el **número FIRE CLÁSICO**, informativo:
//! `fire_number_classic_today`.
//!
//! **El fixture NO se ha regenerado, y no debía**: lo que pinea es la ARITMÉTICA
//! `gross_up(necesidad anual) / SWR` con los tramos españoles, que es exactamente la misma en las
//! dos versiones. El handler la evalúa en el índice 0 de la rejilla —donde el factor de inflación
//! es 1 exacto, o sea euros de HOY— sobre el mismo `FireTarget` que construía el objetivo de
//! 4.15.x (`projection.rs`: `PlanFireTarget::new(Some(ft), &phase_plan).at(0)`), sin término de
//! deuda porque estos casos no tienen pasivos. Lo único que cambió es qué DECIDE ese número: antes
//! una fecha, ahora nada. Un fixture regenerado aquí habría convertido un renombrado en una
//! licencia para mover diecisiete cifras sin derivarlas.

mod common;

use common::TestApp;
use serde_json::Value;
use std::fs;

#[derive(Debug)]
struct FireCase {
    name: String,
    fire_settings: Value,
    income_monthly: String,
    income_retirement_monthly: String,
    expense_retirement_monthly: String,
    expected_target_nw: Option<f64>,
}

fn load_cases() -> Vec<FireCase> {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/fire-parity.json");
    let raw = fs::read_to_string(path).expect("fire-parity.json missing");
    let v: Value = serde_json::from_str(&raw).expect("fire-parity.json malformed");
    let cases = v["cases"].as_array().expect("cases array");
    cases
        .iter()
        .map(|c| FireCase {
            name: c["name"].as_str().unwrap().to_string(),
            fire_settings: c["fire_settings"].clone(),
            income_monthly: c["monthly"]["income"].as_str().unwrap().to_string(),
            income_retirement_monthly: c["monthly"]["income_retirement"]
                .as_str()
                .unwrap()
                .to_string(),
            expense_retirement_monthly: c["monthly"]["expense_retirement"]
                .as_str()
                .unwrap()
                .to_string(),
            expected_target_nw: c["expected_target_nw"].as_f64(),
        })
        .collect()
}

/// Para cada caso del fixture, monta el estado y verifica `fire_number_classic_today`.
#[tokio::test]
async fn the_classic_fire_number_matches_the_canonical_fixtures() {
    let cases = load_cases();
    assert!(!cases.is_empty(), "fixtures vacíos");

    for case in cases {
        let app = TestApp::spawn().await;
        let owner = app.register_and_login_owner("alice").await;

        // Reparto del objeto del fixture entre sus dos dueños de 5.0.0. Se hace con una
        // allowlist explícita y no descartando claves «que suenen a perfil»: si el fixture gana
        // un eje nuevo, quien lo añada tiene que decidir de quién es.
        const PROFILE_KEYS: [&str; 4] = [
            "fire_number_mode",
            "fire_number_manual_amount",
            "swr_pct",
            "horizon_lifespan_age",
        ];
        let obj = case
            .fire_settings
            .as_object()
            .expect("fire_settings del fixture es un objeto");
        let household: serde_json::Map<String, Value> = obj
            .iter()
            .filter(|(k, _)| !PROFILE_KEYS.contains(&k.as_str()))
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        let profile: serde_json::Map<String, Value> = obj
            .iter()
            .filter(|(k, _)| PROFILE_KEYS.contains(&k.as_str()))
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        let patch = app
            .patch_json_with_cookie(
                "/v1/installation",
                serde_json::json!({ "fire_settings": Value::Object(household) }),
                &owner.cookie,
            )
            .await;
        assert_eq!(
            patch.status,
            http::StatusCode::OK,
            "[{}] patch fire_settings failed: {patch:?}",
            case.name
        );
        if !profile.is_empty() {
            let patch = app
                .patch_json_with_cookie(
                    "/v1/auth/me/retirement-profile",
                    Value::Object(profile),
                    &owner.cookie,
                )
                .await;
            assert_eq!(
                patch.status,
                http::StatusCode::OK,
                "[{}] patch retirement-profile failed: {patch:?}",
                case.name
            );
        }

        // Seed: una categoría income, una expense, un asset para tener algo en proyección.
        let asset_cat = app.create_category(&owner, "asset", "Cash").await;
        let income_cat = app.create_category(&owner, "income", "Nómina").await;
        let expense_cat = app.create_category(&owner, "expense", "Alquiler").await;

        let asset = app
            .post_json_with_cookie(
                "/v1/assets",
                serde_json::json!({
                    "category_id": asset_cat,
                    "name": "EUR",
                    "current_value": "10000",
                    "is_liquid": true,
                }),
                &owner.cookie,
            )
            .await;
        assert_eq!(asset.status, http::StatusCode::CREATED);

        // income normal + posible income post-jubilación (persists_after_retirement=true).
        let income_normal: f64 = case.income_monthly.parse().unwrap();
        let income_ret: f64 = case.income_retirement_monthly.parse().unwrap();
        if income_normal > income_ret {
            let pre = app
                .post_json_with_cookie(
                    "/v1/budget/entries",
                    serde_json::json!({
                        "category_id": income_cat,
                        "amount": format!("{:.2}", income_normal - income_ret),
                        "persists_after_retirement": false,
                    }),
                    &owner.cookie,
                )
                .await;
            assert_eq!(pre.status, http::StatusCode::CREATED, "{pre:?}");
        }
        if income_ret > 0.0 {
            let post = app
                .post_json_with_cookie(
                    "/v1/budget/entries",
                    serde_json::json!({
                        "category_id": income_cat,
                        "amount": format!("{:.2}", income_ret),
                        "persists_after_retirement": true,
                    }),
                    &owner.cookie,
                )
                .await;
            assert_eq!(post.status, http::StatusCode::CREATED, "{post:?}");
        }
        // Expense: ends_at_retirement=false → cuenta como expense_retirement (el que usa el servidor).
        let expense_ret: f64 = case.expense_retirement_monthly.parse().unwrap();
        if expense_ret > 0.0 {
            let exp = app
                .post_json_with_cookie(
                    "/v1/budget/entries",
                    serde_json::json!({
                        "category_id": expense_cat,
                        "amount": format!("{:.2}", expense_ret),
                        "ends_at_retirement": false,
                    }),
                    &owner.cookie,
                )
                .await;
            assert_eq!(exp.status, http::StatusCode::CREATED, "{exp:?}");
        }

        // GET projection y comparar.
        let series = app
            .get_with_cookie("/v1/projection/series", &owner.cookie)
            .await;
        assert_eq!(
            series.status,
            http::StatusCode::OK,
            "[{}] series failed: {series:?}",
            case.name
        );
        let body = series.json();
        let target = body["fire_number_classic_today"].as_str();
        // El campo viejo no puede volver por la puerta de atrás: si reapareciera, habría DOS
        // definiciones del mismo número en la misma respuesta y este test estaría mirando la que
        // no decide nada.
        assert!(
            body.get("jubilacion_target_net_worth").is_none(),
            "[{}] `jubilacion_target_net_worth` se retiró con el cruce (modelo v2): {body}",
            case.name
        );

        match (case.expected_target_nw, target) {
            (None, None) => {
                // **Ausencia CON su razón**: un `null` mudo aquí sería indistinguible de un campo
                // que el servidor se dejó sin rellenar. El caso del fixture es el ingreso de
                // jubilación que cubre el gasto entero, así que no hay necesidad que capitalizar.
                assert!(
                    !body["fire_number_classic_absent_reason"].is_null(),
                    "[{}] sin número FIRE tiene que viajar su razón: {body}",
                    case.name
                );
            }
            (None, Some(_)) => panic!(
                "[{}] esperado sin número FIRE clásico, servidor devolvió uno",
                case.name
            ),
            (Some(_), None) => panic!(
                "[{}] esperado número FIRE clásico, servidor devolvió null (razón: {})",
                case.name, body["fire_number_classic_absent_reason"]
            ),
            (Some(expected), Some(s)) => {
                let actual: f64 = s.parse().expect("decimal");
                let diff = (actual - expected).abs();
                assert!(
                    diff <= 1.0,
                    "[{}] número FIRE divergente: esperado ≈ {expected}, servidor {actual} \
                     (diff {diff})",
                    case.name
                );
                assert!(
                    body["fire_number_classic_absent_reason"].is_null(),
                    "[{}] con número no puede haber razón de ausencia: {body}",
                    case.name
                );
            }
        }
    }
}
