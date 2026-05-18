//! Optional query `view=mine` scopes ledger reads to rows attributed to the signed-in user.
//! Omitting `view` or any other value means **household** (full installation).

use serde::Deserialize;
use sqlx::postgres::PgArguments;
use sqlx::query::{Query, QueryAs, QueryScalar};
use sqlx::{Postgres, Type};
use uuid::Uuid;

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
    pub fn resolve(&self) -> LedgerView {
        match self.view.as_deref().map(str::trim) {
            Some("mine") => LedgerView::Mine,
            _ => LedgerView::Household,
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

    #[test]
    fn next_arg_index_matches_scope_placeholder_count() {
        assert_eq!(LedgerView::Household.next_arg_index(), 2);
        assert_eq!(LedgerView::Mine.next_arg_index(), 3);
    }
}
