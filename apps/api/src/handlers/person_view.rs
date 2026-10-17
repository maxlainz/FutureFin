//! Optional query `view` scopes ledger reads. **Desde 5.0.0 el default es `mine`** (R2): omitir
//! `view` —o mandarlo vacío— filtra a las filas del usuario de la sesión; `household` hay que
//! pedirlo EXPLÍCITAMENTE, y cualquier otro valor es un error — ver [`LedgerViewQuery::resolve`].

use crate::error::ApiError;
use serde::Deserialize;
use sqlx::postgres::PgArguments;
use sqlx::query::{Query, QueryAs, QueryScalar};
use sqlx::{Postgres, Type};
use uuid::Uuid;

/// **D21 (5.0.0)** — toda mutación del ledger exige que la fila sea del usuario de la sesión.
///
/// `?view` NUNCA fue una frontera de autorización (D2) y sigue sin serlo: es un filtro de
/// LECTURA. Lo que cambia en 5.0.0 es la ESCRITURA. Con proyecciones independientes por miembro
/// (D9) cada fila del ledger pertenece a la simulación de UNA persona, así que editar la de otro
/// miembro no es «colaborar»: es mover su plan sin que se entere. El rol `owner` **tampoco**
/// salta la regla — ser dueño de la instalación no es ser dueño de la fila.
///
/// Es 403 y no 404 a propósito: la fila existe, el hogar la ve en su listado (`view=household`)
/// y ocultarla al editar produciría un «no existe» que el usuario puede desmentir en la pantalla
/// de al lado. El 404 se reserva para lo que de verdad no está.
pub fn not_row_owner() -> ApiError {
    ApiError::ForbiddenWith(
        "not_row_owner: this row belongs to another household member; only its owner can change it"
            .into(),
    )
}

/// Puerta de D21: compara el dueño de la fila con el usuario de la sesión.
///
/// Punto ÚNICO — los cinco módulos del ledger la llaman en vez de escribir el `if`, que es lo
/// que evita que uno de ellos se quede atrás en la próxima refactorización (el patrón del
/// dual-branch drift que ya mordió dos veces en el MCP).
pub fn require_row_owner(row_owner: Uuid, session_user_id: Uuid) -> Result<(), ApiError> {
    if row_owner == session_user_id {
        Ok(())
    } else {
        Err(not_row_owner())
    }
}

#[derive(Debug, Deserialize)]
pub struct LedgerViewQuery {
    #[serde(default)]
    pub view: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum LedgerView {
    Household,
    Mine,
}

impl LedgerViewQuery {
    /// `household` → Household; ausente, vacío o `mine` → **Mine**. **Cualquier otro valor es un
    /// error**, no un household silencioso.
    ///
    /// **BREAKING 5.0.0 (R2): el default cambió de `household` a `mine`.** Con la jubilación
    /// convertida en una estrategia POR USUARIO (D9/D13), la simulación por defecto es la del
    /// solicitante: su perfil, su fecha de nacimiento, sus filas. Servir el hogar entero por
    /// omisión mezclaba las filas de dos personas bajo el perfil de una sola —un patrimonio
    /// plausible con la estrategia equivocada—, y `household` pasa a ser un AGREGADO explícito
    /// de N simulaciones independientes (§D del plan de #207). Pedirlo sigue siendo una línea:
    /// `?view=household`.
    ///
    /// Hasta 4.0.0 el brazo comodín se comía el valor desconocido y devolvía el hogar entero. Con
    /// la SPA como único cliente eso nunca se notó — nunca manda otra cosa que `mine` o nada —,
    /// pero un agente MCP que escriba `"MINE"` o `"self"` recibía los datos de todo el hogar
    /// creyendo haber pedido solo los suyos, y respondería sobre ellos sin ninguna señal de que
    /// se le ignoró el filtro (auditoría MCP). No es una frontera de autorización — el mismo token
    /// podía pedir `household` a la cara (D2) —, pero sí una respuesta sobre otra población que
    /// la pedida, que es peor que un error. Ese brazo no se toca: lo único que cambia es a dónde
    /// cae la AUSENCIA del parámetro.
    pub fn resolve(&self) -> Result<LedgerView, ApiError> {
        match self.view.as_deref().map(str::trim) {
            None | Some("") | Some("mine") => Ok(LedgerView::Mine),
            Some("household") => Ok(LedgerView::Household),
            Some(_) => Err(ApiError::BadRequest(
                "invalid_view: view must be 'mine' or 'household'".into(),
            )),
        }
    }
}

impl LedgerView {
    /// Fragmento SQL del WHERE para esta vista: `installation_id = $1` (Household) o
    /// `installation_id = $1 AND owner_user_id = $2` (Mine). Pensado para concatenar tras un
    /// `WHERE` propio del handler — el caller añade el resto de condiciones con placeholders
    /// que arrancan en el índice devuelto por [`next_arg_index`].
    pub fn scope_where(&self, table_alias: &str) -> String {
        let prefix = if table_alias.is_empty() {
            String::new()
        } else {
            format!("{table_alias}.")
        };
        match self {
            LedgerView::Household => format!("{prefix}installation_id = $1"),
            LedgerView::Mine => format!(
                "{prefix}installation_id = $1 AND {prefix}owner_user_id = $2"
            ),
        }
    }

    /// Etiqueta pública de la vista: `"household"` | `"mine"`. Es la cadena que las respuestas
    /// **ecoan** en su campo `view`, y la que `LedgerViewQuery::resolve` acepta de vuelta:
    /// `resolve(as_str(v)) == v` para las dos variantes (test `as_str_round_trips_through_resolve`).
    ///
    /// Existe para que el eco no se escriba a mano. Antes vivía como
    /// `if view == LedgerView::Mine { "mine" } else { "household" }` copiado en cuatro handlers, y
    /// el brazo `else` convertía cualquier variante nueva en `"household"` sin avisar — la misma
    /// forma del comodín silencioso que `resolve` eliminó en 4.0.0.
    pub fn as_str(&self) -> &'static str {
        match self {
            LedgerView::Household => "household",
            LedgerView::Mine => "mine",
        }
    }

    /// Índice del siguiente placeholder libre tras los binds del scope: 2 para Household, 3 para Mine.
    pub fn next_arg_index(&self) -> usize {
        match self {
            LedgerView::Household => 2,
            LedgerView::Mine => 3,
        }
    }

    /// Añade los binds del scope (`iid` y, si es `Mine`, `session_user_id`) a una `QueryAs`.
    pub fn bind_scope_as<'q, T>(
        &self,
        q: QueryAs<'q, Postgres, T, PgArguments>,
        iid: Uuid,
        session_user_id: Uuid,
    ) -> QueryAs<'q, Postgres, T, PgArguments> {
        match self {
            LedgerView::Household => q.bind(iid),
            LedgerView::Mine => q.bind(iid).bind(session_user_id),
        }
    }

    /// Igual que [`bind_scope_as`] pero para `Query` (queries sin `query_as`).
    #[allow(dead_code)]
    pub fn bind_scope_query<'q>(
        &self,
        q: Query<'q, Postgres, PgArguments>,
        iid: Uuid,
        session_user_id: Uuid,
    ) -> Query<'q, Postgres, PgArguments> {
        match self {
            LedgerView::Household => q.bind(iid),
            LedgerView::Mine => q.bind(iid).bind(session_user_id),
        }
    }

    /// Igual que [`bind_scope_as`] pero para `QueryScalar`.
    pub fn bind_scope_scalar<'q, O>(
        &self,
        q: QueryScalar<'q, Postgres, O, PgArguments>,
        iid: Uuid,
        session_user_id: Uuid,
    ) -> QueryScalar<'q, Postgres, O, PgArguments>
    where
        O: Send + Unpin,
        (O,): for<'r> sqlx::FromRow<'r, sqlx::postgres::PgRow>,
        O: Type<Postgres>,
    {
        match self {
            LedgerView::Household => q.bind(iid),
            LedgerView::Mine => q.bind(iid).bind(session_user_id),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scope_where_with_table_alias() {
        assert_eq!(LedgerView::Household.scope_where("l"), "l.installation_id = $1");
        assert_eq!(
            LedgerView::Mine.scope_where("a"),
            "a.installation_id = $1 AND a.owner_user_id = $2"
        );
    }

    #[test]
    fn scope_where_without_table_alias() {
        assert_eq!(LedgerView::Household.scope_where(""), "installation_id = $1");
        assert_eq!(
            LedgerView::Mine.scope_where(""),
            "installation_id = $1 AND owner_user_id = $2"
        );
    }

    /// Los tres valores aceptados y el rechazo del resto. El brazo `Some(_)` es el que cierra la
    /// clase entera: antes de 4.0.0 cualquier cadena desconocida caía a Household en silencio.
    ///
    /// **El default vive AQUÍ** (5.0.0, R2): ausente y vacío son `mine`. Si algún día alguien lo
    /// mueve, este test es lo que se lo dice.
    #[test]
    fn resolve_accepts_only_mine_household_and_absence() {
        let v = |s: Option<&str>| LedgerViewQuery { view: s.map(str::to_string) }.resolve();

        assert_eq!(v(None).unwrap(), LedgerView::Mine, "5.0.0: el default es mine");
        assert_eq!(v(Some("")).unwrap(), LedgerView::Mine);
        assert_eq!(v(Some("  ")).unwrap(), LedgerView::Mine);
        assert_eq!(v(Some("household")).unwrap(), LedgerView::Household);
        assert_eq!(v(Some(" mine ")).unwrap(), LedgerView::Mine);

        // Mayúsculas incluidas: `"MINE"` era el caso exacto del issue — devolvía el hogar entero.
        for bad in ["MINE", "Mine", "HOUSEHOLD", "self", "no-existe-esta-vista", "mía"] {
            let err = v(Some(bad)).expect_err(bad);
            assert!(
                matches!(&err, ApiError::BadRequest(m) if m.starts_with("invalid_view: ")),
                "`{bad}` debería dar invalid_view, dio {err:?}"
            );
        }
    }

    /// El eco (`as_str`) y el parser (`resolve`) son inversos: una respuesta que diga
    /// `view: "mine"` describe exactamente la vista que se obtiene reenviando ese valor.
    #[test]
    fn as_str_round_trips_through_resolve() {
        for v in [LedgerView::Household, LedgerView::Mine] {
            let echoed = v.as_str();
            let back = LedgerViewQuery { view: Some(echoed.to_string()) }
                .resolve()
                .expect("el eco debe ser un valor aceptado");
            assert_eq!(back, v, "round-trip roto para {echoed}");
        }
        assert_eq!(LedgerView::Household.as_str(), "household");
        assert_eq!(LedgerView::Mine.as_str(), "mine");
    }

    #[test]
    fn next_arg_index_matches_scope_placeholder_count() {
        assert_eq!(LedgerView::Household.next_arg_index(), 2);
        assert_eq!(LedgerView::Mine.next_arg_index(), 3);
    }
}
