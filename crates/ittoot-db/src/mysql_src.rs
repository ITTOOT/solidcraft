use crate::{page_sql, remote, require_listed, DbConfig, Engine, Grid, SqlSource, TableInfo};
use ittoot_bridge::Result;
use mysql::prelude::Queryable;
use mysql::Value;

pub struct My {
    conn: mysql::Conn,
    listed: Vec<TableInfo>,
}

impl My {
    pub fn connect(cfg: &DbConfig) -> Result<My> {
        let opts = mysql::OptsBuilder::new()
            .ip_or_hostname(Some(cfg.host.clone()))
            .tcp_port(cfg.port())
            .user(Some(cfg.user.clone()))
            .pass(Some(cfg.password.clone()))
            .db_name(if cfg.database.is_empty() { None } else { Some(cfg.database.clone()) })
            .tcp_connect_timeout(Some(cfg.timeout))
            .read_timeout(Some(cfg.timeout));
        let conn = mysql::Conn::new(opts).map_err(|e| remote("MySQL", e))?;
        Ok(My { conn, listed: Vec::new() })
    }
}

pub(crate) fn show(v: &Value) -> String {
    match v {
        Value::NULL => String::new(),
        Value::Bytes(b) => String::from_utf8_lossy(b).into_owned(),
        Value::Int(n) => n.to_string(),
        Value::UInt(n) => n.to_string(),
        Value::Float(f) => f.to_string(),
        Value::Double(f) => f.to_string(),
        Value::Date(y, mo, d, h, mi, s, us) => {
            if *h == 0 && *mi == 0 && *s == 0 && *us == 0 {
                format!("{y:04}-{mo:02}-{d:02}")
            } else if *us == 0 {
                format!("{y:04}-{mo:02}-{d:02} {h:02}:{mi:02}:{s:02}")
            } else {
                format!("{y:04}-{mo:02}-{d:02} {h:02}:{mi:02}:{s:02}.{us:06}")
            }
        }
        Value::Time(neg, days, h, m, s, us) => {
            let hours = u64::from(*days) * 24 + u64::from(*h);
            let sign = if *neg { "-" } else { "" };
            if *us == 0 { format!("{sign}{hours:02}:{m:02}:{s:02}") } else { format!("{sign}{hours:02}:{m:02}:{s:02}.{us:06}") }
        }
    }
}

impl SqlSource for My {
    fn server_version(&mut self) -> Result<String> {
        let v: Option<String> = self.conn.query_first("SELECT VERSION()").map_err(|e| remote("MySQL", e))?;
        Ok(format!("MySQL {}", v.unwrap_or_default()))
    }

    fn tables(&mut self) -> Result<Vec<TableInfo>> {
        let rows: Vec<(String, String)> = self
            .conn
            .query(
                "SELECT table_schema, table_name FROM information_schema.tables \
                 WHERE table_type = 'BASE TABLE' AND table_schema NOT IN ('mysql', 'information_schema', 'performance_schema', 'sys') ORDER BY 1, 2",
            )
            .map_err(|e| remote("MySQL", e))?;
        self.listed = rows.into_iter().map(|(schema, name)| TableInfo { schema, name }).collect();
        Ok(self.listed.clone())
    }

    fn read(&mut self, table: &TableInfo, limit: usize, offset: usize) -> Result<Grid> {
        require_listed(&self.listed, table)?;
        let sql = page_sql(Engine::MySql, table, limit, offset);
        let mut result = self.conn.query_iter(sql).map_err(|e| remote("MySQL", e))?;
        let columns: Vec<String> = result.columns().as_ref().iter().map(|c| c.name_str().into_owned()).collect();
        let mut rows = Vec::new();
        for row in result.by_ref() {
            let row = row.map_err(|e| remote("MySQL", e))?;
            rows.push(row.unwrap().iter().map(show).collect());
        }
        Ok(Grid { columns, rows })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn values_as_text() {
        assert_eq!(show(&Value::NULL), "");
        assert_eq!(show(&Value::Bytes(b"abc".to_vec())), "abc");
        assert_eq!(show(&Value::Date(2026, 11, 18, 0, 0, 0, 0)), "2026-11-18");
        assert_eq!(show(&Value::Date(2026, 11, 18, 9, 5, 7, 0)), "2026-11-18 09:05:07");
        assert_eq!(show(&Value::Time(true, 1, 2, 3, 4, 0)), "-26:03:04");
        assert_eq!(show(&Value::Double(1.5)), "1.5");
    }
}
