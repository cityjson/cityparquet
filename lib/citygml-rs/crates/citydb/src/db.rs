//! The one connection a reader holds, inside one read-only transaction.

use citygml_core::{Error, Result};
use postgres::types::ToSql;
use postgres::{Client, Row};

/// A connection inside a `REPEATABLE READ READ ONLY` transaction on one
/// 3DCityDB schema, so that every query sees one snapshot of it.
pub(crate) struct Session {
    client: Client,
    /// The schema's name, quoted as an identifier.
    schema: String,
}

impl Session {
    pub(crate) fn begin(mut client: Client, schema: &str) -> Result<Self> {
        // The connection idles inside the transaction while a caller
        // encodes what it read, which a server-wide idle timeout would cut.
        client
            .batch_execute(
                "SET idle_in_transaction_session_timeout = 0; \
                 BEGIN ISOLATION LEVEL REPEATABLE READ READ ONLY",
            )
            .map_err(|e| db_error("cannot open a read-only transaction", e))?;
        Ok(Self {
            client,
            schema: quote_ident(schema),
        })
    }

    /// Run `sql`, each `{s}` in it standing for the schema.
    pub(crate) fn query(&mut self, sql: &str, params: &[&(dyn ToSql + Sync)]) -> Result<Vec<Row>> {
        let sql = sql.replace("{s}", &self.schema);
        self.client
            .query(sql.as_str(), params)
            .map_err(|e| db_error("query failed", e))
    }

    /// Like [`Self::query`], for a query returning at most one row.
    pub(crate) fn query_opt(
        &mut self,
        sql: &str,
        params: &[&(dyn ToSql + Sync)],
    ) -> Result<Option<Row>> {
        let sql = sql.replace("{s}", &self.schema);
        self.client
            .query_opt(sql.as_str(), params)
            .map_err(|e| db_error("query failed", e))
    }
}

/// A PostgreSQL error, with what was being done.
pub(crate) fn db_error(context: &str, e: postgres::Error) -> Error {
    match e.as_db_error() {
        Some(db) => Error::invalid(format!("{context}: {}", db.message())),
        None => Error::invalid(format!("{context}: {e}")),
    }
}

/// `name` as an SQL identifier.
fn quote_ident(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}
