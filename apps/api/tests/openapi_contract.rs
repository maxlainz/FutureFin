//! Gate del contrato OpenAPI — el documento que consume cualquiera que integre contra la API.
//!
//! El repo congela el catálogo MCP (`tools_list_returns_exactly_the_v1_catalog`) y los códigos
//! de error (`error_codes_parity.rs`), pero la superficie OpenAPI **no tenía ninguna red**, y
//! por eso pudo acumular en silencio: dos structs distintos compartiendo nombre de componente
//! (`ImportPreviewResponse` de CSV y de `.ffbackup` — utoipa nombra por el último segmento del
//! tipo y `Components.schemas` es un mapa, así que uno machacaba al otro y los dos endpoints de
//! preview acababan apuntando al mismo `$ref`), un path con plantilla sin declarar su
//! parámetro (documento formalmente inválido), un parámetro de query vivo y sin declarar
//! (`?density`), y **ni un solo `securityScheme`**, con lo que 81 operaciones autenticadas se
//! presentaban como públicas y un cliente generado nacía sin enviar credencial.
//!
//! Ninguno rompía un test porque ninguno afectaba al runtime. Este fichero los convierte en
//! rojo. No requiere base de datos: opera sobre el documento generado.

use futurefin_api::openapi::ApiDoc;
use std::collections::BTreeSet;
use utoipa::OpenApi;

/// Endpoints públicos por diseño: los dos de salud y las tres vías de acceso.
///
/// `/v1/auth/sso` no lleva cookie porque su credencial no es una cookie: es la palabra de un
/// proxy de confianza (cabecera `X-Remote-User-Id` desde una IP autorizada), y esa política no
/// se puede expresar como `securityScheme` de OpenAPI. Está descrita en la propia operación.
///
/// Las dos de `/v1/auth/ha/*` («Entrar con Home Assistant») tampoco: su credencial es el
/// round-trip del navegador por Home Assistant más una cookie de estado de un solo uso, que se
/// crea DENTRO del propio flujo. Tampoco es expresable como `securityScheme`.
const PUBLIC_OPERATIONS: &[(&str, &str)] = &[
    ("/v1/health", "get"),
    ("/v1/ready", "get"),
    ("/v1/auth/register", "post"),
    ("/v1/auth/login", "post"),
    ("/v1/auth/sso", "post"),
    ("/v1/auth/ha/start", "get"),
    ("/v1/auth/ha/callback", "get"),
];

fn doc() -> serde_json::Value {
    serde_json::to_value(ApiDoc::openapi()).expect("el documento OpenAPI serializa")
}

/// Toda expresión `{var}` de un path tiene su parameter object. OpenAPI 3.1 lo exige: sin él
/// los validadores rechazan el documento y los generadores producen un método sin forma de
/// pasar el id.
#[test]
fn every_templated_path_declares_its_path_parameters() {
    let doc = doc();
    let paths = doc["paths"].as_object().expect("paths");
    let mut faltan: Vec<String> = Vec::new();

    for (path, item) in paths {
        let esperados: BTreeSet<&str> = path
            .split('/')
            .filter_map(|seg| seg.strip_prefix('{')?.strip_suffix('}'))
            .collect();
        if esperados.is_empty() {
            continue;
        }
        for (method, op) in item.as_object().expect("path item") {
            if !matches!(
                method.as_str(),
                "get" | "put" | "post" | "delete" | "patch" | "head" | "options" | "trace"
            ) {
                continue;
            }
            let declarados: BTreeSet<&str> = op["parameters"]
                .as_array()
                .map(|ps| {
                    ps.iter()
                        .filter(|p| p["in"] == "path")
                        .filter_map(|p| p["name"].as_str())
                        .collect()
                })
                .unwrap_or_default();
            for e in esperados.difference(&declarados) {
                faltan.push(format!("{method} {path} → falta el parámetro `{e}`"));
            }
        }
    }

    assert!(faltan.is_empty(), "paths con plantilla sin parámetro:\n{}", faltan.join("\n"));
}

/// Los dos `preview` tienen cuerpos completamente distintos y deben tener componentes
/// distintos. Si alguien vuelve a dejar dos tipos con el mismo último segmento, uno desaparece
/// del documento sin ruido y su endpoint pasa a documentar el cuerpo del otro.
#[test]
fn the_two_import_previews_do_not_share_a_schema() {
    let doc = doc();
    let schemas = doc["components"]["schemas"].as_object().expect("schemas");
    assert!(
        schemas.contains_key("TransactionImportPreviewResponse"),
        "falta el componente del preview de CSV; ¿se le ha quitado el `#[schema(as = …)]`?"
    );
    assert!(
        schemas.contains_key("ImportPreviewResponse"),
        "falta el componente del preview de .ffbackup"
    );

    let csv = doc["paths"]["/v1/transactions/import/preview"]["post"]["responses"]["200"]
        ["content"]["application/json"]["schema"]["$ref"]
        .as_str()
        .expect("$ref del preview de CSV");
    let backup = doc["paths"]["/v1/backup/user-import/preview"]["post"]["responses"]["200"]
        ["content"]["application/json"]["schema"]["$ref"]
        .as_str()
        .expect("$ref del preview de .ffbackup");
    assert_ne!(
        csv, backup,
        "los dos endpoints de preview apuntan al MISMO schema: uno documenta el cuerpo del otro"
    );
}

/// La API se autentica, y el documento tiene que decirlo.
#[test]
fn authentication_is_declared_and_applies_to_every_private_operation() {
    let doc = doc();
    let schemes = doc["components"]["securitySchemes"]
        .as_object()
        .expect("securitySchemes: sin esto la spec presenta la API entera como pública");
    assert!(schemes.contains_key("ff_session"), "falta el esquema de la cookie de sesión");
    assert!(schemes.contains_key("bearer_token"), "falta el esquema Bearer");

    let global = doc["security"].as_array().expect("security global");
    assert!(
        global.iter().any(|s| s.get("ff_session").is_some()),
        "el `security` global debe exigir la cookie: {global:?}"
    );

    // Y los públicos por diseño lo anulan explícitamente — ni más ni menos que esos cuatro.
    let paths = doc["paths"].as_object().expect("paths");
    let mut anulan: Vec<(String, String)> = Vec::new();
    for (path, item) in paths {
        for (method, op) in item.as_object().expect("path item") {
            if op.get("security").is_some() {
                anulan.push((path.clone(), method.clone()));
            }
        }
    }
    anulan.sort();
    let esperados: Vec<(String, String)> = {
        let mut v: Vec<(String, String)> = PUBLIC_OPERATIONS
            .iter()
            .map(|(p, m)| (p.to_string(), m.to_string()))
            .collect();
        v.sort();
        v
    };
    assert_eq!(
        anulan, esperados,
        "la lista de operaciones sin autenticación cambió. Si es deliberado, actualiza \
         PUBLIC_OPERATIONS y explica por qué en el CHANGELOG"
    );
}

/// Todo tipo referenciado desde un `$ref` existe en `components.schemas`. Un `$ref` colgante
/// rompe cualquier generador de clientes.
#[test]
fn no_dangling_schema_references() {
    let doc = doc();
    let schemas = doc["components"]["schemas"].as_object().expect("schemas");
    let mut colgantes: BTreeSet<String> = BTreeSet::new();

    fn walk(v: &serde_json::Value, out: &mut BTreeSet<String>) {
        match v {
            serde_json::Value::Object(map) => {
                if let Some(r) = map.get("$ref").and_then(|r| r.as_str()) {
                    if let Some(name) = r.strip_prefix("#/components/schemas/") {
                        out.insert(name.to_string());
                    }
                }
                for (_, child) in map {
                    walk(child, out);
                }
            }
            serde_json::Value::Array(items) => items.iter().for_each(|i| walk(i, out)),
            _ => {}
        }
    }

    let mut referenciados = BTreeSet::new();
    walk(&doc, &mut referenciados);
    for name in &referenciados {
        if !schemas.contains_key(name) {
            colgantes.insert(name.clone());
        }
    }
    assert!(colgantes.is_empty(), "$ref sin componente: {colgantes:?}");
}

/// `PatchRetirementProfileBody.target_basis` debe anunciar el ENUM, no un string libre.
///
/// El fallo que cierra: el campo llevaba `#[schema(value_type = Option<String>)]` heredado del
/// molde de los otros tri-estado, así que el documento decía «cualquier string» sobre un
/// `Deserialize` que solo acepta dos literales. Un cliente generado a partir de él nace pudiendo
/// mandar `"perpetuidad"` y descubre la lista con un 400 — y el `$ref` al componente `TargetBasis`
/// existía, publicado y sin que nada apuntara a él.
#[test]
fn the_retirement_profile_patch_advertises_the_target_basis_enum() {
    let doc = doc();
    let field = &doc["components"]["schemas"]["PatchRetirementProfileBody"]["properties"]
        ["target_basis"];
    assert!(!field.is_null(), "el campo existe en el componente: {doc}");
    // utoipa envuelve el `Option<T>` nullable; la prueba es que en algún punto del subárbol del
    // campo se nombre el componente `TargetBasis` y en ninguno se declare `type: string` suelto.
    let rendered = field.to_string();
    assert!(
        rendered.contains("TargetBasis"),
        "target_basis debe referirse al enum TargetBasis, no a un string libre: {rendered}"
    );

    // …y el componente referido enumera exactamente las dos variantes que el Deserialize acepta.
    let variants = doc["components"]["schemas"]["TargetBasis"]["enum"]
        .as_array()
        .expect("TargetBasis publica su lista de variantes");
    let mut got: Vec<&str> = variants.iter().filter_map(|v| v.as_str()).collect();
    got.sort_unstable();
    assert_eq!(got, vec!["bridge_to_pension", "perpetuity"], "{variants:?}");
}

/// **El bloque `plan` del Resumen y los solves de la proyección están DECLARADOS** (5.0.0
/// WP5-2b). Un campo que la API sirve y el documento no describe es un cliente generado que no
/// lo tiene, y aquí la mitad son cifras de dinero: `null` y `0` significan cosas distintas y el
/// contrato tiene que poder decirlo.
#[test]
fn the_plan_and_the_strategy_solves_are_declared_in_the_document() {
    let doc = doc();

    // `/v1/summary` → `plan`, con su razón de ausencia.
    let plan_ref = doc["components"]["schemas"]["SummaryResponse"]["properties"]["plan"].to_string();
    assert!(
        plan_ref.contains("SummaryPlan"),
        "SummaryResponse.plan debe referirse al componente SummaryPlan: {plan_ref}"
    );
    let plan = &doc["components"]["schemas"]["SummaryPlan"]["properties"];
    for k in [
        "strategy",
        "retirement_trigger",
        "jubilacion_month_index",
        "required_savings_monthly",
        "disposable_monthly",
        "underfunded",
        "absent_reason",
        // 5.0.0 WP6b — el KPI «Éxito del plan» (D28) y su razón de ausencia propia.
        "success_probability",
        "success_threshold_pct",
        "success_verdict",
        "success_absent_reason",
    ] {
        assert!(!plan[k].is_null(), "SummaryPlan.{k} no está declarado: {plan}");
    }

    // `/v1/projection/series` → los solves y las lecturas de pensión/puente/media jornada.
    let serie = &doc["components"]["schemas"]["ProjectionSeriesResponse"]["properties"];
    for k in [
        "bridge_discount_annual_pct",
        "bridge_effective_withdrawal_pct",
        "pension_coverage_ratio",
        "partial_gap_target",
        "partial_phase_capital_growing",
        "required_contribution_monthly",
        "required_contribution_search_ceiling",
        "underfunded",
        "required_capital_path",
        "disposable_monthly",
        "disposable_capital",
        "disposable_capital_at_retirement",
        "disposable_capital_today",
        "coast_fire_month_index",
        "coast_number",
        "coast_path",
    ] {
        assert!(
            !serie[k].is_null(),
            "ProjectionSeriesResponse.{k} no está declarado"
        );
    }

    // `/v1/projection/bands` → el contrato entero de Monte Carlo. Se declara aquí y no solo en el
    // handler porque es la superficie que un cliente lee ANTES de llamar: un campo que existe en
    // el JSON y no en la spec es un campo que nadie consume.
    let bandas = &doc["components"]["schemas"]["ProjectionBandsResponse"]["properties"];
    for k in [
        "view",
        "months",
        "horizon_basis",
        "anchor_date_ymd",
        "paths",
        "seed",
        "percentiles",
        "points",
        "success_probability",
        "success_threshold_pct",
        "success_verdict",
        "depletion_probability_by_age",
        "retirement_month_index_percentiles",
        "underfunded_probability",
        "months_below_need_p50",
        "withdrawal_to_need_ratio_p50",
        "any_volatility_declared",
        "buffer_active",
        "buffer_refills_p50",
        "buffer_refill_net_total_p50",
        "strategy",
        "retirement_trigger",
        "computed_in_ms",
        "model_note",
    ] {
        assert!(
            !bandas[k].is_null(),
            "ProjectionBandsResponse.{k} no está declarado"
        );
    }
    // La semilla es un **string** en la spec: un `u64` como número JSON pierde precisión por
    // encima de 2^53, y una semilla que cambia al ida-y-vuelta no reproduce nada.
    assert_eq!(
        bandas["seed"]["type"], "string",
        "la semilla debe declararse como string: {bandas}"
    );
    let punto = &doc["components"]["schemas"]["ProjectionBandPoint"]["properties"];
    for k in [
        "month_index",
        "net_worth_p10",
        "net_worth_p50",
        "net_worth_p90",
        "net_worth_liquid_p10",
        "net_worth_liquid_p50",
        "net_worth_liquid_p90",
    ] {
        assert!(
            !punto[k].is_null(),
            "ProjectionBandPoint.{k} no está declarado"
        );
    }

    // Y por miembro del hogar, las cuatro que explican de quién es cada marcador.
    let miembro = &doc["components"]["schemas"]["HouseholdMemberProjection"]["properties"];
    for k in [
        "coast_fire_month_index",
        "underfunded",
        "required_contribution_monthly",
        "disposable_monthly",
    ] {
        assert!(
            !miembro[k].is_null(),
            "HouseholdMemberProjection.{k} no está declarado"
        );
    }
}
