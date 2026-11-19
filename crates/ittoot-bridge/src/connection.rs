//! The devices the bridge talks to: PLCs, PCs, HMIs, SCADA servers, databases.
//!
//! Details live in the settings; secrets (passwords, keys) live in a separate file so the settings
//! can be shared, committed or sent without them.

use crate::error::{Error, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeviceKind {
    Plc,
    Pc,
    Hmi,
    Scada,
    Database,
    Other,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Protocol {
    /// Symbolic access to optimised DBs. The first choice for S7-1500.
    OpcUa,
    /// S7 PUT/GET: absolute addressing, so non-optimised DBs only.
    S7,
    ModbusTcp,
    Postgres,
    SqlServer,
    MySql,
    Http,
    /// No link: values come from the simulated driver.
    Simulated,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Connection {
    pub id: String,
    pub name: String,
    pub kind: DeviceKind,
    pub protocol: Protocol,
    pub host: String,
    #[serde(default)]
    pub port: Option<u16>,
    /// S7: rack and slot.
    #[serde(default)]
    pub rack: Option<u8>,
    #[serde(default)]
    pub slot: Option<u8>,
    #[serde(default)]
    pub username: String,
    /// OPC UA endpoint path, database name, or any extra the protocol needs.
    #[serde(default)]
    pub extra: String,
    #[serde(default = "yes")]
    pub enabled: bool,
    #[serde(default)]
    pub notes: String,
}

fn yes() -> bool {
    true
}

impl Connection {
    pub fn new(id: impl Into<String>, name: impl Into<String>, kind: DeviceKind, protocol: Protocol, host: impl Into<String>) -> Self {
        Connection {
            id: id.into(),
            name: name.into(),
            kind,
            protocol,
            host: host.into(),
            port: None,
            rack: None,
            slot: None,
            username: String::new(),
            extra: String::new(),
            enabled: true,
            notes: String::new(),
        }
    }

    /// The port in use: the one set, else the protocol's usual one.
    pub fn effective_port(&self) -> Option<u16> {
        self.port.or(match self.protocol {
            Protocol::OpcUa => Some(4840),
            Protocol::S7 => Some(102),
            Protocol::ModbusTcp => Some(502),
            Protocol::Http => Some(80),
            Protocol::Postgres => Some(5432),
            Protocol::SqlServer => Some(1433),
            Protocol::MySql => Some(3306),
            Protocol::Simulated => None,
        })
    }

    pub fn is_database(&self) -> bool {
        matches!(self.protocol, Protocol::Postgres | Protocol::SqlServer | Protocol::MySql)
    }

    /// What is wrong with the details as typed, for the Connections page to show. Empty when fine.
    pub fn problems(&self) -> Vec<String> {
        let mut p = Vec::new();
        if self.name.trim().is_empty() {
            p.push("Name is empty".to_string());
        }
        if self.protocol != Protocol::Simulated && self.host.trim().is_empty() {
            p.push("Address is empty".to_string());
        }
        if self.protocol == Protocol::S7 && (self.rack.is_none() || self.slot.is_none()) {
            p.push("S7 needs a rack and a slot (S7-1500: 0 and 1)".to_string());
        }
        if self.is_database() && self.extra.trim().is_empty() {
            p.push("Database name is empty (put it in the extra box)".to_string());
        }
        if self.host.contains(char::is_whitespace) {
            p.push("Address contains a space".to_string());
        }
        p
    }
}

/// Where a link stands. Set by the driver, shown on the Connections page.
#[derive(Clone, Debug, PartialEq, Default)]
pub enum LinkState {
    #[default]
    Disconnected,
    Connecting,
    Connected,
    Failed(String),
}

/// Passwords and keys by connection id.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Secrets {
    #[serde(default)]
    pub by_connection: BTreeMap<String, String>,
}

impl Secrets {
    pub fn load(path: &Path) -> Result<Self> {
        match std::fs::read_to_string(path) {
            Ok(text) => serde_json::from_str(&text).map_err(|e| Error::Parse { path: path.display().to_string(), message: e.to_string() }),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Secrets::default()),
            Err(e) => Err(Error::io(path, e)),
        }
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        let text = serde_json::to_string_pretty(self).map_err(|e| Error::Invalid(e.to_string()))?;
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| Error::io(dir, e))?;
        }
        std::fs::write(path, text).map_err(|e| Error::io(path, e))
    }

    pub fn get(&self, connection: &str) -> Option<&str> {
        self.by_connection.get(connection).map(String::as_str)
    }

    /// An empty secret removes the entry.
    pub fn set(&mut self, connection: &str, secret: &str) {
        if secret.is_empty() {
            self.by_connection.remove(connection);
        } else {
            self.by_connection.insert(connection.to_string(), secret.to_string());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_ports_and_problems() {
        let mut c = Connection::new("plc1", "Hoist PLC", DeviceKind::Plc, Protocol::S7, "192.168.0.1");
        assert_eq!(c.effective_port(), Some(102));
        assert_eq!(c.problems(), ["S7 needs a rack and a slot (S7-1500: 0 and 1)"]);
        c.rack = Some(0);
        c.slot = Some(1);
        assert!(c.problems().is_empty());
        c.host = "bad host".into();
        assert_eq!(c.problems().len(), 1);
    }

    #[test]
    fn secrets_stay_out_of_the_connection_and_round_trip() {
        let c = Connection::new("a", "A", DeviceKind::Plc, Protocol::OpcUa, "10.0.0.2");
        assert!(!serde_json::to_string(&c).unwrap_or_default().contains("password"));
        let dir = std::env::temp_dir().join(format!("ittoot-bridge-secrets-{}", std::process::id()));
        let file = dir.join("secrets.json");
        let mut s = Secrets::default();
        s.set("a", "pw");
        s.save(&file).unwrap();
        assert_eq!(Secrets::load(&file).unwrap().get("a"), Some("pw"));
        s.set("a", "");
        assert_eq!(s.get("a"), None);
        assert_eq!(Secrets::load(&dir.join("none.json")).unwrap(), Secrets::default());
        let _ = std::fs::remove_dir_all(dir);
    }
}
