//! Optional query `view=mine` scopes ledger reads to rows attributed to the signed-in user.
//! Omitting `view` or any other value means **household** (full installation).

use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct LedgerViewQuery {
    #[serde(default)]
    pub view: Option<String>,
}

#[derive(Clone, Copy)]
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
