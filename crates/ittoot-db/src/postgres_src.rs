use crate::{qualified, require_listed, DbConfig, Engine, Grid, SqlSource, TableInfo};
use ittoot_bridge::Result;

pub struct Pg {
    client: postgres::Client,
    listed: Vec<TableInfo>,
}

impl Pg {
    pub fn connect(cfg: &DbConfig) -> Result<Pg> {
        let mut c = postgres::Config::new();
        c.host(&cfg.host).port(cfg.port()).user(&cfg.user).password(&cfg.password).connect_timeout(cfg.timeout);
        if !cfg.database.is_empty() {
            c.dbname(&cfg.database);
        }
        let client = c.connect(postgres::NoTls).map_err(|e| pg_err(&e))?;
        Ok(Pg { client, listed: Vec::new() })
    }
}

impl SqlSource for Pg {
    fn server_version(&mut self) -> Result<String> {
        let row = self.client.query_one("SHOW server_version", &[]).map_err(|e| pg_err(&e))?;
        Ok(format!("PostgreSQL {}", row.get::<_, String>(0)))
    }

    fn tables(&mut self) -> Result<Vec<TableInfo>> {
        let rows = self
            .client
            .query(
                "SELECT table_schema::text, table_name::text FROM information_schema.tables \
                 WHERE table_type = 'BASE TABLE' AND table_schema NOT IN ('pg_catalog', 'information_schema') ORDER BY 1, 2",
                &[],
            )
            .map_err(|e| pg_err(&e))?;
        self.listed = rows.iter().map(|r| TableInfo { schema: r.get(0), name: r.get(1) }).collect();
        Ok(self.listed.clone())
    }

    fn read(&mut self, table: &TableInfo, limit: usize, offset: usize) -> Result<Grid> {
        require_listed(&self.listed, table)?;
        let cols = self
            .client
            .query(
                "SELECT column_name::text FROM information_schema.columns WHERE table_schema = $1 AND table_name = $2 ORDER BY ordinal_position",
                &[&table.schema, &table.name],
            )
            .map_err(|e| pg_err(&e))?;
        let names: Vec<String> = cols.iter().map(|r| r.get::<_, String>(0)).collect();
        // every column cast to text: a table page shows values, whatever their type
        let select: Vec<String> = names.iter().map(|n| format!("{}::text", crate::quote_ident(Engine::Postgres, n))).collect();
        let sql = format!("SELECT {} FROM {} LIMIT {limit} OFFSET {offset}", select.join(", "), qualified(Engine::Postgres, table));
        let rows = self.client.query(sql.as_str(), &[]).map_err(|e| pg_err(&e))?;
        let data = rows.iter().map(|r| (0..names.len()).map(|i| r.get::<_, Option<String>>(i).unwrap_or_default()).collect()).collect();
        Ok(Grid { columns: names, rows: data })
    }
}

/// The server's own words (`password authentication failed for user ...`), not the client's bare "db error".
fn pg_err(e: &postgres::Error) -> crate::Error {
    if let Some(db) = e.as_db_error() {
        return crate::Error::Remote(format!("PostgreSQL: {} ({})", db.message(), db.code().code()));
    }
    let mut text = e.to_string();
    let mut source = std::error::Error::source(e);
    while let Some(s) = source {
        text = format!("{text}: {s}");
        source = s.source();
    }
    crate::Error::Remote(format!("PostgreSQL: {text}"))
}
