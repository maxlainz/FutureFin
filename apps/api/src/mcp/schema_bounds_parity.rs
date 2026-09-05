//! PIN C6 — las cotas que anuncia el schema MCP son las mismas que valida el runtime.
//!
//! `#[schemars(range(min = .., max = ..))]` **exige literales**: no admite una constante, ni
//! `MAX_SCHEDULE_WINDOW_MONTHS`, ni un `const fn`. El literal del schema y la constante que
//! valida en el handler son por tanto dos copias del mismo número, y nada en el compilador las
//! ata. La única red posible es un test que las enfrente — esto.
//!
//! **Por qué importa**: el schema es el texto que LEE EL MODELO. Si la constante sube a 600 y el
//! literal se queda en 480, el modelo cree que 600 es ilegal y ni siquiera lo intenta; si baja a
//! 240 y el literal se queda en 480, el modelo manda 480, la tool devuelve un 400 y el usuario ve
//! «la herramienta está rota». Es exactamente el incidente que la norma de CLAUDE.md ya registra:
//! «el schema de una tool MCP anunciaba "default 15" cuando el real era 30, y topaba en 60 donde
//! la core acepta 365; el mismo parámetro funcionaba por HTTP y fallaba por MCP».
//!
//! **De dónde sale el literal del schema.** No se re-deriva aquí: se lee del fixture congelado
//! `tests/fixtures/mcp-catalog.json`, que `mcp_http.rs::tools_list_freezes_the_input_contract_of_every_tool`
//! genera desde el `inputSchema` real (`UPDATE_MCP_CATALOG=1 cargo test -p futurefin-api --test
//! mcp_http -- tools_list_freezes_the_input_contract`). Las dos guardias se cubren mutuamente:
//!
//! - tocas la macro y NO regeneras el fixture → falla `tools_list_freezes_…` (fixture ≠ schema);
//! - tocas la macro y SÍ regeneras → falla este test (schema ≠ constante);
//! - tocas la constante y nada más → falla este test (constante ≠ schema).
//!
//! Solo pasa el cambio coordinado, que es el punto.
//!
//! **Cómo crece esta tabla**: al añadir un `#[schemars(range(...))]` cuyo tope también viva en una
//! constante de runtime, añade su fila a [`PINNED_BOUNDS`]. Un `range` sobre un valor que no tiene
//! contraparte en runtime (p. ej. `month_index` de `deflate_amount`, acotado solo por el horizonte
//! del engine) no va aquí: esto pinea *duplicaciones*, no cotas.

use crate::handlers::history::MAX_HISTORY_WINDOW_MONTHS;
use crate::handlers::installation::{MAX_HORIZON_LIFESPAN_AGE, MIN_HORIZON_LIFESPAN_AGE};
use crate::handlers::liabilities::{MAX_SCHEDULE_WINDOW_MONTHS, SCHEDULE_HORIZON_MONTHS};
use crate::handlers::retirement_profile::{
    MAX_CASH_BUFFER_MONTHS, MIN_PENSION_AGE, MIN_PROFILE_AGE,
};

const CATALOG_JSON: &str =
    include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/mcp-catalog.json"));

/// Una duplicación literal↔constante que este test vigila.
struct PinnedBound {
    /// Nombre de la tool en `tools/list`.
    tool: &'static str,
    /// Posición dentro de `constraints` del fixture (`$.<propiedad>`).
    pointer: &'static str,
    /// `minimum` esperado en el schema, cuando el runtime también lo fija.
    expected_min: Option<i64>,
    /// `maximum` esperado en el schema = el valor de la constante de runtime.
    expected_max: i64,
    /// Ruta `fichero: CONSTANTE` de la que sale `expected_max`.
    runtime_const: &'static str,
    /// Sitios que hay que actualizar A LA VEZ si este número cambia.
    also_update: &'static str,
}

/// **La lista, no el número.** Nada de «hay N cotas duplicadas»: el test recorre esta tabla.
const PINNED_BOUNDS: &[PinnedBound] = &[
    PinnedBound {
        tool: "get_liability_schedule",
        pointer: "$.months",
        expected_min: Some(1),
        expected_max: MAX_SCHEDULE_WINDOW_MONTHS as i64,
        runtime_const: "apps/api/src/handlers/liabilities.rs: MAX_SCHEDULE_WINDOW_MONTHS",
        also_update: "(1) la const `MAX_SCHEDULE_WINDOW_MONTHS`; \
                      (2) el literal `#[schemars(range(min = 1, max = 480))]` sobre \
                      `LiabilityScheduleParams::months` en apps/api/src/mcp/server.rs; \
                      (3) el literal del mensaje de error \
                      `\"schedule_window_out_of_range: months must be between 1 and 480\"` en \
                      `liability_schedule_core` (apps/api/src/handlers/liabilities.rs) — ese \
                      mensaje es la tercera copia del mismo número y NO lo cubre ningún test; \
                      (4) la descripción `params(\"months\" …)` del `#[utoipa::path]` de \
                      `GET /v1/liabilities/{id}/schedule`; \
                      (5) regenera el fixture: UPDATE_MCP_CATALOG=1 cargo test -p futurefin-api \
                      --test mcp_http -- tools_list_freezes_the_input_contract",
    },
    PinnedBound {
        tool: "get_liability_schedule",
        pointer: "$.from_month_index",
        // El runtime solo rechaza `from < 1` (y `from` es `u32`, así que el suelo es implícito);
        // el techo del schema es el horizonte que SIEMPRE se simula, porque pedir un primer mes
        // más allá del calendario entero no puede devolver nada.
        expected_min: Some(1),
        expected_max: SCHEDULE_HORIZON_MONTHS as i64,
        runtime_const: "apps/api/src/handlers/liabilities.rs: SCHEDULE_HORIZON_MONTHS \
                        (= futurefin_engine::MAX_LIABILITY_SCHEDULE_MONTHS)",
        also_update: "(1) `MAX_LIABILITY_SCHEDULE_MONTHS` en crates/engine/src/projection.rs; \
                      (2) el literal `#[schemars(range(min = 1, max = 840))]` sobre \
                      `LiabilityScheduleParams::from_month_index` en apps/api/src/mcp/server.rs; \
                      (3) regenera el fixture (ver la fila de `$.months`)",
    },
    PinnedBound {
        tool: "get_history",
        pointer: "$.window_months",
        expected_min: Some(1),
        expected_max: MAX_HISTORY_WINDOW_MONTHS,
        runtime_const: "apps/api/src/handlers/history.rs: MAX_HISTORY_WINDOW_MONTHS",
        also_update: "(1) la const `MAX_HISTORY_WINDOW_MONTHS`; \
                      (2) el literal `#[schemars(range(min = 1, max = 1200))]` sobre \
                      `HistoryParams::window_months` en apps/api/src/mcp/server.rs; \
                      (3) el texto de la descripción de ese mismo campo, que dice «(1–1200)»; \
                      (4) la descripción `params(\"window_months\" …)` del `#[utoipa::path]` de \
                      `GET /v1/history/series`; \
                      (5) regenera el fixture (ver la fila de `$.months`)",
    },
    // ---- Perfil de jubilación por usuario (5.0.0, D13) --------------------------------------
    // Cinco cotas duplicadas entre el schema y `handlers/retirement_profile.rs`. Las tres de EDAD
    // topan en `MAX_HORIZON_LIFESPAN_AGE` y no en el `horizon_lifespan_age` de cada perfil, que es
    // el techo REAL: el schema no puede expresar una cota que depende de otro campo del mismo
    // objeto, así que publica el techo absoluto y `validate_retirement_profile` aprieta el resto
    // (`retirement_age_out_of_range`, `pension_age_out_of_range`, `partial_age_out_of_range`).
    PinnedBound {
        tool: "update_retirement_profile",
        pointer: "$.horizon_lifespan_age",
        expected_min: Some(MIN_HORIZON_LIFESPAN_AGE as i64),
        expected_max: MAX_HORIZON_LIFESPAN_AGE as i64,
        runtime_const: "apps/api/src/handlers/installation.rs: MIN/MAX_HORIZON_LIFESPAN_AGE",
        also_update: "(1) las dos consts; (2) el literal \
                      `#[schemars(range(min = 85, max = 105))]` sobre \
                      `UpdateRetirementProfileParams::horizon_lifespan_age`; (3) el mensaje \
                      `horizon_lifespan_age_out_of_range` (lo compone `format!` con las consts, \
                      así que no es una cuarta copia); (4) regenera el fixture (ver la fila de \
                      `$.months`)",
    },
    PinnedBound {
        tool: "update_retirement_profile",
        pointer: "$.target_retirement_age",
        expected_min: Some(MIN_PROFILE_AGE as i64),
        expected_max: MAX_HORIZON_LIFESPAN_AGE as i64,
        runtime_const: "apps/api/src/handlers/retirement_profile.rs: MIN_PROFILE_AGE \
                        (techo: MAX_HORIZON_LIFESPAN_AGE)",
        also_update: "(1) `MIN_PROFILE_AGE`; (2) el literal \
                      `#[schemars(range(min = 18, max = 105))]` sobre \
                      `UpdateRetirementProfileParams::target_retirement_age`; (3) regenera el \
                      fixture (ver la fila de `$.months`)",
    },
    PinnedBound {
        tool: "update_retirement_profile",
        pointer: "$defs.PensionParam.starts_at_age",
        expected_min: Some(MIN_PENSION_AGE as i64),
        expected_max: MAX_HORIZON_LIFESPAN_AGE as i64,
        runtime_const: "apps/api/src/handlers/retirement_profile.rs: MIN_PENSION_AGE \
                        (techo: MAX_HORIZON_LIFESPAN_AGE)",
        also_update: "(1) `MIN_PENSION_AGE`; (2) el literal \
                      `#[schemars(range(min = 50, max = 105))]` sobre \
                      `PensionParam::starts_at_age`; (3) regenera el fixture",
    },
    PinnedBound {
        tool: "update_retirement_profile",
        pointer: "$.cash_buffer_months",
        expected_min: Some(0),
        expected_max: MAX_CASH_BUFFER_MONTHS as i64,
        runtime_const: "apps/api/src/handlers/retirement_profile.rs: MAX_CASH_BUFFER_MONTHS",
        also_update: "(1) la const; (2) el literal `#[schemars(range(min = 0, max = 60))]` sobre \
                      `UpdateRetirementProfileParams::cash_buffer_months`; (3) regenera el fixture",
    },
    // `$.success_threshold_pct` **ya no tiene fila** (5.0.0, V7): el parámetro sigue en el schema
    // de las dos tools —son `deny_unknown_fields` y borrarlo convertiría en 400 lo que hoy
    // funciona— pero está DEPRECADO e ignorado, y ya no tiene cotas que sujetar. Pinear una
    // cota de un parámetro que no se lee sería congelar una promesa que el runtime no cumple.
];

/// Lee `constraints["<pointer>"]` de la tool y extrae `minimum` / `maximum`.
///
/// El fixture serializa cada restricción como una línea de `clave=valor` ordenada, p. ej.
/// `format="uint32" maximum=480 minimum=1 type=["integer","null"]`. No hace falta un parser: los
/// dos números que importan son tokens `maximum=<n>` / `minimum=<n>`.
fn numeric_bound(constraint: &str, key: &str) -> Option<i64> {
    let needle = format!("{key}=");
    constraint.split_whitespace().find_map(|tok| {
        tok.strip_prefix(&needle)
            .and_then(|v| v.parse::<i64>().ok())
    })
}

#[test]
fn mcp_schema_ranges_match_the_runtime_constants_they_duplicate() {
    let catalog: serde_json::Value =
        serde_json::from_str(CATALOG_JSON).expect("mcp-catalog.json es JSON válido");
    let tools = catalog["tools"]
        .as_array()
        .expect("mcp-catalog.json tiene un array `tools`");

    let mut failures: Vec<String> = Vec::new();

    for pin in PINNED_BOUNDS {
        let tool = tools.iter().find(|t| t["name"] == pin.tool);
        let Some(tool) = tool else {
            failures.push(format!(
                "la tool `{}` no está en mcp-catalog.json. O se renombró/retiró (y esta fila de \
                 PINNED_BOUNDS sobra) o el fixture está desactualizado.",
                pin.tool
            ));
            continue;
        };
        let constraint = tool["constraints"][pin.pointer].as_str();
        let Some(constraint) = constraint else {
            failures.push(format!(
                "`{}` ya no declara restricciones en `{}`. Si el parámetro se retiró, borra esta \
                 fila de PINNED_BOUNDS; si perdió su `#[schemars(range(...))]`, el schema dejó de \
                 acotar lo que el runtime sigue acotando ({}) y el modelo mandará valores que la \
                 tool rechazará con un 400.",
                pin.tool, pin.pointer, pin.runtime_const
            ));
            continue;
        };

        let actual_max = numeric_bound(constraint, "maximum");
        if actual_max != Some(pin.expected_max) {
            failures.push(format!(
                "DIVERGENCIA en `{tool}{pointer}`: el schema MCP anuncia maximum={actual:?} y el \
                 runtime valida {expected} ({runtime_const}).\n    \
                 El schema es el texto que LEE EL MODELO: divergir significa que el MCP rechaza \
                 lo que HTTP acepta (o que el modelo manda algo que la tool devuelve como 400 y \
                 el usuario lee como «la herramienta está rota»).\n    \
                 Actualiza A LA VEZ: {also_update}",
                tool = pin.tool,
                pointer = pin.pointer,
                actual = actual_max,
                expected = pin.expected_max,
                runtime_const = pin.runtime_const,
                also_update = pin.also_update,
            ));
        }

        if let Some(expected_min) = pin.expected_min {
            let actual_min = numeric_bound(constraint, "minimum");
            if actual_min != Some(expected_min) {
                failures.push(format!(
                    "DIVERGENCIA en `{tool}{pointer}`: el schema MCP anuncia minimum={actual:?} y \
                     el runtime exige {expected}.\n    \
                     Mismo coste que el techo: un suelo que no coincide hace que el modelo \
                     descarte valores legales, o que mande ilegales.\n    \
                     Actualiza A LA VEZ: {also_update}",
                    tool = pin.tool,
                    pointer = pin.pointer,
                    actual = actual_min,
                    expected = expected_min,
                    also_update = pin.also_update,
                ));
            }
        }
    }

    assert!(
        failures.is_empty(),
        "El schema MCP y el runtime dejaron de coincidir en {} sitio(s):\n\n- {}\n",
        failures.len(),
        failures.join("\n\n- ")
    );
}

#[test]
fn the_pinned_bounds_table_actually_reads_the_fixture() {
    // Anti-«grep que ya no encuentra nada» (norma de CLAUDE.md): si el formato de `constraints`
    // cambia y `numeric_bound` deja de extraer nada, el test de arriba fallaría — pero si alguien
    // «lo arregla» relajándolo a `actual.is_none() → ok`, el pin se apagaría en silencio. Esta es
    // la prueba de vida del parser sobre el fixture REAL.
    let catalog: serde_json::Value = serde_json::from_str(CATALOG_JSON).unwrap();
    let tools = catalog["tools"].as_array().unwrap();

    assert!(
        !PINNED_BOUNDS.is_empty(),
        "PINNED_BOUNDS está vacía: el pin C6 no vigila nada."
    );

    for pin in PINNED_BOUNDS {
        let tool = tools
            .iter()
            .find(|t| t["name"] == pin.tool)
            .unwrap_or_else(|| panic!("tool `{}` ausente del fixture", pin.tool));
        let constraint = tool["constraints"][pin.pointer]
            .as_str()
            .unwrap_or_else(|| panic!("`{}{}` sin constraints", pin.tool, pin.pointer));
        assert!(
            numeric_bound(constraint, "maximum").is_some(),
            "`numeric_bound` no extrajo `maximum` de {constraint:?}. El formato de `constraints` \
             en mcp-catalog.json cambió: arregla el parser, NO relajes la aserción."
        );
    }
}
