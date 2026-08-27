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
const PUBLIC_OPERATIONS: &[(&str, &str)] = &[
    ("/v1/health", "get"),
    ("/v1/ready", "get"),
    ("/v1/auth/register", "post"),
    ("/v1/auth/login", "post"),
    ("/v1/auth/sso", "post"),
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
