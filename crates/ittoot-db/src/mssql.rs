use crate::{page_sql, remote, require_listed, DbConfig, Engine, Grid, SqlSource, TableInfo};
use ittoot_bridge::{Error, Result};
use tiberius::{AuthMethod, Client, ColumnData, Config, EncryptionLevel};
use tokio::net::TcpStream;
use tokio_util::compat::{Compat, TokioAsyncWriteCompatExt};

pub struct Ms {
    rt: tokio::runtime::Runtime,
    client: Client<Compat<TcpStream>>,
    listed: Vec<TableInfo>,
    timeout: std::time::Duration,
}

impl Ms {
    pub fn connect(cfg: &DbConfig) -> Result<Ms> {
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().map_err(|e| remote("SQL Server", e))?;
        let mut c = Config::new();
        c.host(&cfg.host);
        c.port(cfg.port());
        if !cfg.database.is_empty() {
            c.database(&cfg.database);
        }
        if cfg.windows_auth {
            #[cfg(windows)]
            c.authentication(AuthMethod::Integrated);
            #[cfg(not(windows))]
            return Err(Error::Invalid("Windows sign-in only works when the bridge runs on Windows".into()));
        } else {
            c.authentication(AuthMethod::sql_server(&cfg.user, &cfg.password));
        }
        if cfg.encrypt {
            c.encryption(EncryptionLevel::Required);
            c.trust_cert();
        } else {
            c.encryption(EncryptionLevel::NotSupported);
        }
        let timeout = cfg.timeout;
        let client = rt
            .block_on(async {
                let tcp = tokio::time::timeout(timeout, TcpStream::connect(c.get_addr())).await.map_err(|_| "no answer from the server".to_string())?.map_err(|e| e.to_string())?;
                tcp.set_nodelay(true).map_err(|e| e.to_string())?;
                tokio::time::timeout(timeout, Client::connect(c, tcp.compat_write())).await.map_err(|_| "sign-in timed out".to_string())?.map_err(|e| e.to_string())
            })
            .map_err(|e| remote("SQL Server", e))?;
        Ok(Ms { rt, client, listed: Vec::new(), timeout })
    }

    fn rows(&mut self, sql: &str) -> Result<(Vec<String>, Vec<Vec<String>>)> {
        let timeout = self.timeout;
        let client = &mut self.client;
        self.rt
            .block_on(async {
                let stream = tokio::time::timeout(timeout, client.simple_query(sql)).await.map_err(|_| "query timed out".to_string())?.map_err(|e| e.to_string())?;
                let rows = stream.into_first_result().await.map_err(|e| e.to_string())?;
                let columns: Vec<String> = rows.first().map(|r| r.columns().iter().map(|c| c.name().to_string()).collect()).unwrap_or_default();
                let data = rows.iter().map(|r| r.cells().map(|(_, v)| show(v)).collect()).collect();
                Ok::<_, String>((columns, data))
            })
            .map_err(|e| remote("SQL Server", e))
    }
}

fn show(v: &ColumnData<'_>) -> String {
    match v {
        ColumnData::Bit(Some(b)) => if *b { "1" } else { "0" }.to_string(),
        ColumnData::U8(Some(n)) => n.to_string(),
        ColumnData::I16(Some(n)) => n.to_string(),
        ColumnData::I32(Some(n)) => n.to_string(),
        ColumnData::I64(Some(n)) => n.to_string(),
        ColumnData::F32(Some(n)) => n.to_string(),
        ColumnData::F64(Some(n)) => n.to_string(),
        ColumnData::String(Some(s)) => s.to_string(),
        ColumnData::Guid(Some(g)) => g.to_string(),
        ColumnData::Numeric(Some(n)) => n.to_string(),
        ColumnData::Binary(Some(b)) => format!("0x{}", b.iter().map(|x| format!("{x:02X}")).collect::<String>()),
        ColumnData::Bit(None) | ColumnData::U8(None) | ColumnData::I16(None) | ColumnData::I32(None) | ColumnData::I64(None) | ColumnData::F32(None)
        | ColumnData::F64(None) | ColumnData::String(None) | ColumnData::Guid(None) | ColumnData::Numeric(None) | ColumnData::Binary(None) => String::new(),
        other => format!("{other:?}"),
    }
}

impl SqlSource for Ms {
    fn server_version(&mut self) -> Result<String> {
        let (_, rows) = self.rows("SELECT CAST(SERVERPROPERTY('ProductVersion') AS nvarchar(64))")?;
        Ok(format!("SQL Server {}", rows.first().and_then(|r| r.first()).cloned().unwrap_or_default()))
    }

    fn tables(&mut self) -> Result<Vec<TableInfo>> {
        let (_, rows) = self.rows("SELECT TABLE_SCHEMA, TABLE_NAME FROM INFORMATION_SCHEMA.TABLES WHERE TABLE_TYPE = 'BASE TABLE' ORDER BY 1, 2")?;
        self.listed = rows.into_iter().filter_map(|mut r| if r.len() == 2 { let name = r.pop()?; let schema = r.pop()?; Some(TableInfo { schema, name }) } else { None }).collect();
        Ok(self.listed.clone())
    }

    fn read(&mut self, table: &TableInfo, limit: usize, offset: usize) -> Result<Grid> {
        require_listed(&self.listed, table).map_err(|e| Error::Missing(e.to_string()))?;
        let (columns, rows) = self.rows(&page_sql(Engine::SqlServer, table, limit, offset))?;
        Ok(Grid { columns, rows })
    }
}
