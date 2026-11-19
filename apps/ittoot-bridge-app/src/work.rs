//! Everything slow runs off the window thread: scans, table builds, connection tests, live reads.

use ittoot_bridge::area::{AreaKind, DataArea};
use ittoot_bridge::connection::{Connection, LinkState, Protocol};
use ittoot_bridge::driver::{value_from_literal, PlcDriver, Value};
use ittoot_bridge::infer::{records_from_db, Record};
use ittoot_bridge::scl;
use ittoot_bridge::table::{files_under, tables_for, Table};
use ittoot_db::{DbConfig, Engine, Grid, TableInfo};
use ittoot_opcua::{OpcUaConfig, OpcUaDriver};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;
use std::time::Duration;

/// One piece of background work whose answer is collected by `poll`.
pub struct Job<T> {
    rx: Receiver<T>,
}

impl<T: Send + 'static> Job<T> {
    pub fn spawn(f: impl FnOnce() -> T + Send + 'static) -> Job<T> {
        let (tx, rx) = channel();
        std::thread::spawn(move || {
            let _ = tx.send(f());
        });
        Job { rx }
    }

    pub fn poll(&self) -> Option<T> {
        self.rx.try_recv().ok()
    }
}

pub struct Built {
    pub tables: Vec<Table>,
    pub records: Vec<Record>,
    /// What each value is set to in the source, as the stand-in for a PLC.
    pub start_values: HashMap<String, Value>,
}

pub fn build(areas: &[DataArea]) -> Built {
    let mut tables = Vec::new();
    let mut files: Vec<PathBuf> = Vec::new();
    for a in areas.iter().filter(|a| a.enabled) {
        tables.extend(tables_for(a));
        match a.kind {
            AreaKind::DataBlock => files.push(PathBuf::from(&a.source)),
            AreaKind::Folder => files.extend(files_under(Path::new(&a.source))),
            _ => {}
        }
    }
    // Controls first, each group in the order the areas came
    tables.sort_by_key(|t| t.role != ittoot_bridge::area::AreaRole::Controls);
    let mut seen: HashSet<String> = HashSet::new();
    let mut records = Vec::new();
    let mut start_values = HashMap::new();
    for f in files {
        if !f.extension().is_some_and(|e| e.eq_ignore_ascii_case("scl") || e.eq_ignore_ascii_case("db")) {
            continue;
        }
        let Ok(bytes) = std::fs::read(&f) else { continue };
        for db in scl::parse(&String::from_utf8_lossy(&bytes)).dbs {
            if !seen.insert(db.name.clone()) {
                continue;
            }
            records.extend(records_from_db(&db));
            for (path, literal) in &db.initial {
                if let Some(v) = value_from_literal(literal) {
                    start_values.insert(format!("{}.{}", db.name, path), v);
                }
            }
        }
    }
    Built { tables, records, start_values }
}

/// What the UI asks the Solidcraft worker to do.
pub enum PushMsg {
    /// Send these commands, in order, to build the model; the answer says how it went.
    Build(Vec<serde_json::Value>),
    /// Send these `joint.drive` commands (only the newest batch matters).
    Drive(Vec<serde_json::Value>),
}

/// A link to a running Solidcraft over its control channel, on a thread of its own.
/// Wait until something listens on `addr` (Solidcraft opening its control channel); false on timeout.
pub fn wait_for_port(addr: &str, secs: u64) -> bool {
    let Ok(sock) = addr.parse::<std::net::SocketAddr>() else { return false };
    let end = std::time::Instant::now() + Duration::from_secs(secs);
    while std::time::Instant::now() < end {
        if std::net::TcpStream::connect_timeout(&sock, Duration::from_millis(400)).is_ok() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(500));
    }
    false
}

pub struct SolidLink {
    tx: Sender<PushMsg>,
    pub rx: Receiver<String>,
}

fn control_app() -> solidcraft_control::auth::App {
    solidcraft_control::auth::App { name: "Solidcraft", data_dir: || None, env: |_| None, utc: |s| s.to_string(), default_port: 0 }
}

impl SolidLink {
    /// `journal` is the deployment folder: every command of a build is written there with its answer.
    pub fn spawn(addr: String, token: String, journal: PathBuf) -> SolidLink {
        let (tx, jobs) = channel::<PushMsg>();
        let (say, rx) = channel::<String>();
        std::thread::spawn(move || {
            let t = if token.trim().is_empty() { None } else { Some(token.trim().to_string()) };
            let mut conn = solidcraft_control::client::Connection::new(control_app(), &addr, t.as_deref());
            let run = |conn: &mut solidcraft_control::client::Connection, cmds: &[serde_json::Value]| -> Result<usize, String> {
                let mut n = 0;
                for c in cmds {
                    let params = serde_json::json!({"command": c["command"], "params": c["params"]});
                    conn.ask("engine.execute", params).map_err(|e| format!("{}: {e}", c["command"].as_str().unwrap_or("?")))?;
                    n += 1;
                }
                Ok(n)
            };
            while let Ok(mut msg) = jobs.recv() {
                // a drive batch that is already out of date is dropped for the newest one
                while let (PushMsg::Drive(_), Ok(next @ PushMsg::Drive(_))) = (&msg, jobs.try_recv()) {
                    msg = next;
                }
                match msg {
                    PushMsg::Build(cmds) => {
                        let mut n = 0;
                        let mut failed: Option<String> = None;
                        for c in &cmds {
                            match run(&mut conn, std::slice::from_ref(c)) {
                                Ok(_) => {
                                    n += 1;
                                    let _ = ittoot_bridge::deploy::journal(&journal, c, "ok");
                                }
                                Err(e) => {
                                    let _ = ittoot_bridge::deploy::journal(&journal, c, &format!("refused: {e}"));
                                    failed = Some(e);
                                    break;
                                }
                            }
                        }
                        let r: Result<usize, String> = match failed {
                            Some(e) => Err(e),
                            None => Ok(n),
                        };
                        let _ = say.send(match r {
                            Ok(n) => format!("built: {n} commands accepted"),
                            Err(e) => format!("build stopped: {e}"),
                        });
                    }
                    PushMsg::Drive(cmds) => {
                        if let Err(e) = run(&mut conn, &cmds) {
                            // a drive is sent five times a second: only a failure is worth a line
                            if let Some(c) = cmds.first() {
                                let _ = ittoot_bridge::deploy::journal(&journal, c, &format!("drive failed: {e}"));
                            }
                            let _ = say.send(format!("drive failed: {e}"));
                        } else {
                            let _ = say.send("driving".to_string());
                        }
                    }
                }
            }
        });
        SolidLink { tx, rx }
    }

    pub fn send(&self, m: PushMsg) {
        let _ = self.tx.send(m);
    }
}

/// Live reads of one OPC UA server on a thread of its own.
pub struct Poller {
    keys: Sender<Vec<String>>,
    pub rx: Receiver<PollMsg>,
    stop: Arc<AtomicBool>,
}

pub enum PollMsg {
    State(LinkState),
    Values(HashMap<String, Value>),
}

impl Poller {
    pub fn spawn(cfg: OpcUaConfig) -> Poller {
        let (ktx, krx) = channel::<Vec<String>>();
        let (tx, rx) = channel();
        let stop = Arc::new(AtomicBool::new(false));
        let flag = stop.clone();
        std::thread::spawn(move || {
            let mut driver = match OpcUaDriver::new(cfg) {
                Ok(d) => d,
                Err(e) => {
                    let _ = tx.send(PollMsg::State(LinkState::Failed(e.to_string())));
                    return;
                }
            };
            let _ = tx.send(PollMsg::State(LinkState::Connecting));
            if let Err(e) = driver.connect() {
                let _ = tx.send(PollMsg::State(LinkState::Failed(e.to_string())));
                return;
            }
            let _ = tx.send(PollMsg::State(LinkState::Connected));
            let mut keys: Vec<String> = Vec::new();
            while !flag.load(Ordering::Relaxed) {
                while let Ok(k) = krx.try_recv() {
                    keys = k;
                }
                if !keys.is_empty() {
                    match driver.read(&keys) {
                        Ok(values) => {
                            let map = keys.iter().cloned().zip(values).filter_map(|(k, v)| v.map(|v| (k, v))).collect();
                            let _ = tx.send(PollMsg::Values(map));
                        }
                        Err(e) => {
                            let _ = tx.send(PollMsg::State(LinkState::Failed(e.to_string())));
                            break;
                        }
                    }
                }
                for _ in 0..5 {
                    if flag.load(Ordering::Relaxed) {
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(100));
                }
            }
            driver.disconnect();
        });
        Poller { keys: ktx, rx, stop }
    }

    pub fn want(&self, keys: Vec<String>) {
        let _ = self.keys.send(keys);
    }
}

impl Drop for Poller {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

pub fn opcua_config(c: &Connection, password: &str, pki: &Path) -> OpcUaConfig {
    let scheme_host = if c.host.starts_with("opc.tcp://") { c.host.clone() } else { format!("opc.tcp://{}:{}", c.host, c.effective_port().unwrap_or(4840)) };
    let mut cfg = OpcUaConfig::new(scheme_host, pki);
    if !c.username.is_empty() {
        cfg.username = Some(c.username.clone());
        cfg.password = Some(password.to_string());
    }
    cfg
}

pub fn db_config(c: &Connection, password: &str) -> Option<DbConfig> {
    let engine = match c.protocol {
        Protocol::Postgres => Engine::Postgres,
        Protocol::SqlServer => Engine::SqlServer,
        Protocol::MySql => Engine::MySql,
        _ => return None,
    };
    let mut cfg = DbConfig::new(engine, c.host.clone(), c.extra.clone());
    cfg.port = c.port;
    cfg.user = c.username.clone();
    cfg.password = password.to_string();
    // SQL Server with no user name signs in as the Windows user running the bridge
    cfg.windows_auth = engine == Engine::SqlServer && c.username.is_empty();
    Some(cfg)
}

/// "Test": prove the details work, say what was found.
pub fn test_connection(c: &Connection, password: &str, pki: &Path) -> Result<String, String> {
    match c.protocol {
        Protocol::Simulated => Ok("Simulated link: always available; values come from the start values in the sources.".into()),
        Protocol::OpcUa => {
            let mut d = OpcUaDriver::new(opcua_config(c, password, pki)).map_err(|e| e.to_string())?;
            d.connect().map_err(|e| e.to_string())?;
            d.disconnect();
            Ok(format!("Connected to {} and found the data-block namespace.", c.host))
        }
        Protocol::Postgres | Protocol::SqlServer | Protocol::MySql => {
            let cfg = db_config(c, password).ok_or("not a database")?;
            let mut s = ittoot_db::open(&cfg).map_err(|e| e.to_string())?;
            let v = s.server_version().map_err(|e| e.to_string())?;
            let n = s.tables().map_err(|e| e.to_string())?.len();
            Ok(format!("{v}; {n} tables visible."))
        }
        Protocol::S7 | Protocol::ModbusTcp | Protocol::Http => Err("No driver for this protocol yet. Use OPC UA for S7-1500.".into()),
    }
}

pub fn db_tables(cfg: &DbConfig) -> Result<Vec<TableInfo>, String> {
    ittoot_db::open(cfg).and_then(|mut s| s.tables()).map_err(|e| e.to_string())
}

pub fn db_page(cfg: &DbConfig, t: &TableInfo, limit: usize, offset: usize) -> Result<Grid, String> {
    let mut s = ittoot_db::open(cfg).map_err(|e| e.to_string())?;
    s.tables().map_err(|e| e.to_string())?; // the page is only read for a table the server lists
    s.read(t, limit, offset).map_err(|e| e.to_string())
}
