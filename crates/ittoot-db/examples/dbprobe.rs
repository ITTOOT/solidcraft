//! `cargo run -p ittoot-db --example dbprobe -- postgres|mysql|sqlserver HOST PORT DATABASE USER PASSWORD [windows]`:
//! sign in, print the server version and the first tables. Read only.

use ittoot_db::{open, DbConfig, Engine};

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    if a.len() < 6 {
        eprintln!("usage: dbprobe postgres|mysql|sqlserver HOST PORT DATABASE USER PASSWORD [windows]");
        return;
    }
    let engine = match a[0].as_str() {
        "postgres" => Engine::Postgres,
        "mysql" => Engine::MySql,
        _ => Engine::SqlServer,
    };
    let mut cfg = DbConfig::new(engine, a[1].clone(), a[3].clone());
    cfg.port = a[2].parse().ok();
    cfg.user = a[4].clone();
    cfg.password = a[5].clone();
    cfg.windows_auth = a.get(6).is_some_and(|w| w == "windows");
    match open(&cfg) {
        Err(e) => println!("open failed: {e}"),
        Ok(mut src) => {
            println!("version: {:?}", src.server_version());
            match src.tables() {
                Ok(t) => println!("{} tables; first: {:?}", t.len(), t.iter().take(5).map(|x| format!("{}.{}", x.schema, x.name)).collect::<Vec<_>>()),
                Err(e) => println!("tables failed: {e}"),
            }
        }
    }
}
