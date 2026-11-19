//! SQL sources for the bridge: PostgreSQL, SQL Server and MySQL.
//!
//! Read-only on purpose. The bridge lists the tables of a database and tabulates them the way the
//! OT Analyser does; it never writes to a plant database. Every value comes back as text, because a
//! table page shows values and does not compute with them. Identifiers are quoted per engine, and a
//! table is only read if it came from the server's own listing.

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

mod mssql;
mod mysql_src;
mod postgres_src;

use ittoot_bridge::{Error, Result};
use std::time::Duration;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Engine {
    Postgres,
    SqlServer,
    MySql,
}

impl Engine {
    pub fn default_port(self) -> u16 {
        match self {
            Engine::Postgres => 5432,
            Engine::SqlServer => 1433,
            Engine::MySql => 3306,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Engine::Postgres => "PostgreSQL",
            Engine::SqlServer => "SQL Server",
            Engine::MySql => "MySQL",
        }
    }
}

#[derive(Clone, Debug)]
pub struct DbConfig {
    pub engine: Engine,
    pub host: String,
    pub port: Option<u16>,
    pub database: String,
    pub user: String,
    pub password: String,
    /// SQL Server only: sign in with the Windows account that runs the bridge.
    pub windows_auth: bool,
    /// SQL Server: encrypt the link (and accept the server's own certificate).
    pub encrypt: bool,
    pub timeout: Duration,
}

impl DbConfig {
    pub fn new(engine: Engine, host: impl Into<String>, database: impl Into<String>) -> Self {
        DbConfig {
            engine,
            host: host.into(),
            port: None,
            database: database.into(),
            user: String::new(),
            password: String::new(),
            windows_auth: false,
            encrypt: false,
            timeout: Duration::from_secs(8),
        }
    }

    pub fn port(&self) -> u16 {
        self.port.unwrap_or_else(|| self.engine.default_port())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TableInfo {
    pub schema: String,
    pub name: String,
}

impl TableInfo {
    pub fn display(&self) -> String {
        if self.schema.is_empty() { self.name.clone() } else { format!("{}.{}", self.schema, self.name) }
    }
}

/// One page of a table, as text.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Grid {
    pub columns: Vec<String>,
    pub rows: Vec<Vec<String>>,
}

pub trait SqlSource {
    /// Server name and version, proving the sign-in worked.
    fn server_version(&mut self) -> Result<String>;
    fn tables(&mut self) -> Result<Vec<TableInfo>>;
    /// `limit` rows from row `offset`, in the table's own order.
    fn read(&mut self, table: &TableInfo, limit: usize, offset: usize) -> Result<Grid>;
}

pub fn open(cfg: &DbConfig) -> Result<Box<dyn SqlSource>> {
    if cfg.host.trim().is_empty() {
        return Err(Error::Invalid("the address is empty".into()));
    }
    Ok(match cfg.engine {
        Engine::Postgres => Box::new(postgres_src::Pg::connect(cfg)?),
        Engine::MySql => Box::new(mysql_src::My::connect(cfg)?),
        Engine::SqlServer => Box::new(mssql::Ms::connect(cfg)?),
    })
}

/// Quote a name for the engine so it cannot end the statement it sits in.
pub fn quote_ident(engine: Engine, name: &str) -> String {
    match engine {
        Engine::Postgres => format!("\"{}\"", name.replace('"', "\"\"")),
        Engine::MySql => format!("`{}`", name.replace('`', "``")),
        Engine::SqlServer => format!("[{}]", name.replace(']', "]]")),
    }
}

pub(crate) fn qualified(engine: Engine, t: &TableInfo) -> String {
    if t.schema.is_empty() { quote_ident(engine, &t.name) } else { format!("{}.{}", quote_ident(engine, &t.schema), quote_ident(engine, &t.name)) }
}

/// The page query for engines that select every column as it is.
pub(crate) fn page_sql(engine: Engine, t: &TableInfo, limit: usize, offset: usize) -> String {
    let from = qualified(engine, t);
    match engine {
        Engine::SqlServer => format!("SELECT * FROM {from} ORDER BY (SELECT NULL) OFFSET {offset} ROWS FETCH NEXT {limit} ROWS ONLY"),
        Engine::Postgres | Engine::MySql => format!("SELECT * FROM {from} LIMIT {limit} OFFSET {offset}"),
    }
}

/// Only tables the server listed may be read.
pub(crate) fn require_listed(list: &[TableInfo], t: &TableInfo) -> Result<()> {
    if list.contains(t) { Ok(()) } else { Err(Error::Missing(format!("table {} is not in the server's list", t.display()))) }
}

pub(crate) fn remote(what: &str, e: impl std::fmt::Display) -> Error {
    Error::Remote(format!("{what}: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identifiers_are_quoted_per_engine() {
        assert_eq!(quote_ident(Engine::Postgres, "a\"b"), "\"a\"\"b\"");
        assert_eq!(quote_ident(Engine::MySql, "a`b"), "`a``b`");
        assert_eq!(quote_ident(Engine::SqlServer, "a]b; DROP TABLE x"), "[a]]b; DROP TABLE x]");
    }

    #[test]
    fn page_queries() {
        let t = TableInfo { schema: "dbo".into(), name: "Alarms".into() };
        assert_eq!(page_sql(Engine::SqlServer, &t, 100, 200), "SELECT * FROM [dbo].[Alarms] ORDER BY (SELECT NULL) OFFSET 200 ROWS FETCH NEXT 100 ROWS ONLY");
        assert_eq!(page_sql(Engine::Postgres, &t, 10, 0), "SELECT * FROM \"dbo\".\"Alarms\" LIMIT 10 OFFSET 0");
        assert_eq!(page_sql(Engine::MySql, &TableInfo { schema: String::new(), name: "t".into() }, 1, 1), "SELECT * FROM `t` LIMIT 1 OFFSET 1");
    }

    #[test]
    fn only_listed_tables_and_ports() {
        let t = TableInfo { schema: "s".into(), name: "n".into() };
        assert!(require_listed(std::slice::from_ref(&t), &t).is_ok());
        assert!(require_listed(&[], &t).is_err());
        let mut c = DbConfig::new(Engine::SqlServer, "h", "d");
        assert_eq!(c.port(), 1433);
        c.port = Some(1500);
        assert_eq!(c.port(), 1500);
        assert!(open(&DbConfig::new(Engine::MySql, " ", "d")).is_err());
    }

    #[test]
    fn nothing_listening_is_an_error_not_a_hang() {
        for e in [Engine::Postgres, Engine::MySql, Engine::SqlServer] {
            let mut c = DbConfig::new(e, "127.0.0.1", "d");
            c.port = Some(1);
            c.timeout = Duration::from_secs(2);
            assert!(open(&c).is_err(), "{e:?}");
        }
    }
}
