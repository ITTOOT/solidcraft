//! OPC UA link for the ITTOOT bridge.
//!
//! An S7-1500 keeps its optimised data blocks readable only by name, and OPC UA is how to read by
//! name. A bridge key `Gantry_01_Live.driveX.cmd.start` becomes the node
//! `ns=<siemens>;s="Gantry_01_Live"."driveX"."cmd"."start"`, and `rows[3].enabled` becomes
//! `"rows"[3]."enabled"`. The namespace is looked up by its URI on connect, not assumed to be 3.
//!
//! The bridge's `PlcDriver` is synchronous, so the link owns a small tokio runtime and blocks on it.

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

use ittoot_bridge::connection::LinkState;
use ittoot_bridge::driver::{PlcDriver, Value};
use ittoot_bridge::{Error, Result};
use opcua::client::{ClientBuilder, IdentityToken, Session};
use opcua::crypto::SecurityPolicy;
use opcua::types::{AttributeId, MessageSecurityMode, NodeId, ReadValueId, StatusCode, TimestampsToReturn, UserTokenPolicy, Variant, WriteValue};
use std::sync::Arc;
use std::time::Duration;

/// The namespace Siemens publishes a PLC's data blocks under.
pub const SIEMENS_NAMESPACE_URI: &str = "http://www.siemens.com/simatic-s7-opcua";

#[derive(Clone, Debug)]
pub struct OpcUaConfig {
    /// `opc.tcp://192.168.0.1:4840`
    pub endpoint_url: String,
    pub username: Option<String>,
    pub password: Option<String>,
    /// Where data blocks live; the Siemens one unless the server is something else.
    pub namespace_uri: String,
    pub timeout: Duration,
    /// Folder for the client's certificate and the servers it trusts.
    pub pki_dir: std::path::PathBuf,
}

impl OpcUaConfig {
    pub fn new(endpoint_url: impl Into<String>, pki_dir: impl Into<std::path::PathBuf>) -> Self {
        OpcUaConfig {
            endpoint_url: endpoint_url.into(),
            username: None,
            password: None,
            namespace_uri: SIEMENS_NAMESPACE_URI.to_string(),
            timeout: Duration::from_secs(5),
            pki_dir: pki_dir.into(),
        }
    }
}

/// `Db.rows[3].enabled` -> `"Db"."rows"[3]."enabled"` (the part after `s=` in a Siemens node id).
pub fn symbolic_node_name(key: &str) -> String {
    let mut out = String::new();
    for (n, segment) in key.split('.').enumerate() {
        if n > 0 {
            out.push('.');
        }
        let (name, indexes) = match segment.find('[') {
            Some(at) => (segment.get(..at).unwrap_or(segment), segment.get(at..).unwrap_or("")),
            None => (segment, ""),
        };
        out.push('"');
        out.push_str(&name.replace('"', "\"\""));
        out.push('"');
        out.push_str(indexes);
    }
    out
}

pub fn node_id_for(key: &str, namespace: u16) -> NodeId {
    NodeId::new(namespace, symbolic_node_name(key))
}

fn to_value(v: &Variant) -> Option<Value> {
    Some(match v {
        Variant::Boolean(b) => Value::Bool(*b),
        Variant::SByte(n) => Value::Int(i64::from(*n)),
        Variant::Byte(n) => Value::Int(i64::from(*n)),
        Variant::Int16(n) => Value::Int(i64::from(*n)),
        Variant::UInt16(n) => Value::Int(i64::from(*n)),
        Variant::Int32(n) => Value::Int(i64::from(*n)),
        Variant::UInt32(n) => Value::Int(i64::from(*n)),
        Variant::Int64(n) => Value::Int(*n),
        Variant::UInt64(n) => Value::Int(i64::try_from(*n).ok()?),
        Variant::Float(f) => Value::Real(f64::from(*f)),
        Variant::Double(f) => Value::Real(*f),
        Variant::String(s) => Value::Text(s.value().clone().unwrap_or_default()),
        _ => return None,
    })
}

/// Write `value` as the same OPC UA type the node already holds (the PLC refuses a different one).
fn like(existing: &Variant, value: &Value) -> Option<Variant> {
    Some(match (existing, value) {
        (Variant::Boolean(_), Value::Bool(b)) => Variant::Boolean(*b),
        (Variant::SByte(_), Value::Int(n)) => Variant::SByte(i8::try_from(*n).ok()?),
        (Variant::Byte(_), Value::Int(n)) => Variant::Byte(u8::try_from(*n).ok()?),
        (Variant::Int16(_), Value::Int(n)) => Variant::Int16(i16::try_from(*n).ok()?),
        (Variant::UInt16(_), Value::Int(n)) => Variant::UInt16(u16::try_from(*n).ok()?),
        (Variant::Int32(_), Value::Int(n)) => Variant::Int32(i32::try_from(*n).ok()?),
        (Variant::UInt32(_), Value::Int(n)) => Variant::UInt32(u32::try_from(*n).ok()?),
        (Variant::Int64(_), Value::Int(n)) => Variant::Int64(*n),
        (Variant::UInt64(_), Value::Int(n)) => Variant::UInt64(u64::try_from(*n).ok()?),
        (Variant::Float(_), Value::Real(f)) => Variant::Float(*f as f32),
        (Variant::Float(_), Value::Int(n)) => Variant::Float(*n as f32),
        (Variant::Double(_), Value::Real(f)) => Variant::Double(*f),
        (Variant::Double(_), Value::Int(n)) => Variant::Double(*n as f64),
        (Variant::String(_), Value::Text(t)) => Variant::from(t.as_str()),
        _ => return None,
    })
}

struct Link {
    session: Arc<Session>,
    namespace: u16,
    event_loop: tokio::task::JoinHandle<StatusCode>,
}

pub struct OpcUaDriver {
    cfg: OpcUaConfig,
    rt: tokio::runtime::Runtime,
    link: Option<Link>,
    state: LinkState,
}

impl OpcUaDriver {
    pub fn new(cfg: OpcUaConfig) -> Result<Self> {
        // One worker thread: it runs the session's event loop while the caller is not inside block_on.
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .enable_all()
            .build()
            .map_err(|e| Error::Invalid(format!("cannot start the OPC UA runtime: {e}")))?;
        Ok(OpcUaDriver { cfg, rt, link: None, state: LinkState::Disconnected })
    }

    fn fail<T>(&mut self, message: String) -> Result<T> {
        self.state = LinkState::Failed(message.clone());
        Err(Error::NotConnected(message))
    }
}

impl PlcDriver for OpcUaDriver {
    fn state(&self) -> LinkState {
        self.state.clone()
    }

    fn connect(&mut self) -> Result<()> {
        if self.link.is_some() {
            return Ok(());
        }
        self.state = LinkState::Connecting;
        let cfg = self.cfg.clone();
        let outcome = self.rt.block_on(async move {
            let mut client = ClientBuilder::new()
                .application_name("ITTOOT Bridge")
                .application_uri("urn:ittoot:bridge")
                .product_uri("urn:ittoot:bridge")
                .pki_dir(cfg.pki_dir.clone())
                .create_sample_keypair(true)
                .trust_server_certs(true)
                .session_retry_limit(1)
                .request_timeout(cfg.timeout)
                .client()
                .map_err(|e| e.join("; "))?;
            let identity = match (&cfg.username, &cfg.password) {
                (Some(u), Some(p)) => IdentityToken::new_user_name(u.clone(), p.clone()),
                _ => IdentityToken::Anonymous,
            };
            let endpoint = (cfg.endpoint_url.as_str(), SecurityPolicy::None.to_str(), MessageSecurityMode::None, UserTokenPolicy::anonymous());
            let (session, event_loop) = client.connect_to_matching_endpoint(endpoint, identity).await.map_err(|e| e.to_string())?;
            let handle = event_loop.spawn();
            let connected = tokio::time::timeout(cfg.timeout, session.wait_for_connection()).await.unwrap_or(false);
            if !connected {
                handle.abort();
                return Err(format!("no answer from {} within {:?}", cfg.endpoint_url, cfg.timeout));
            }
            let namespace = session.get_namespace_index(&cfg.namespace_uri).await.map_err(|e| format!("namespace {} not found: {e}", cfg.namespace_uri))?;
            Ok::<Link, String>(Link { session, namespace, event_loop: handle })
        });
        match outcome {
            Ok(link) => {
                self.link = Some(link);
                self.state = LinkState::Connected;
                Ok(())
            }
            Err(message) => self.fail(message),
        }
    }

    fn disconnect(&mut self) {
        if let Some(link) = self.link.take() {
            self.rt.block_on(async {
                let _ = tokio::time::timeout(Duration::from_secs(2), link.session.disconnect()).await;
            });
            link.event_loop.abort();
        }
        self.state = LinkState::Disconnected;
    }

    fn read(&mut self, keys: &[String]) -> Result<Vec<Option<Value>>> {
        let Some(link) = &self.link else { return Err(Error::NotConnected("OPC UA".into())) };
        let nodes: Vec<ReadValueId> = keys
            .iter()
            .map(|k| ReadValueId { node_id: node_id_for(k, link.namespace), attribute_id: AttributeId::Value as u32, ..Default::default() })
            .collect();
        let session = link.session.clone();
        let timeout = self.cfg.timeout;
        let answer = self.rt.block_on(async move {
            // Servers cap how many nodes one request may hold; ask in pieces.
            let mut out: Vec<Option<Value>> = Vec::with_capacity(nodes.len());
            for chunk in nodes.chunks(500) {
                let got = tokio::time::timeout(timeout, session.read(chunk, TimestampsToReturn::Neither, 0.0))
                    .await
                    .map_err(|_| "read timed out".to_string())?
                    .map_err(|e| e.to_string())?;
                for dv in got {
                    let good = dv.status.map(|s| s.is_good()).unwrap_or(true);
                    out.push(if good { dv.value.as_ref().and_then(to_value) } else { None });
                }
            }
            Ok::<_, String>(out)
        });
        match answer {
            Ok(v) if v.len() == keys.len() => Ok(v),
            Ok(_) => self.fail("the server answered with a different number of values than asked for".into()),
            Err(message) => self.fail(message),
        }
    }

    fn write(&mut self, key: &str, value: Value) -> Result<()> {
        let Some(link) = &self.link else { return Err(Error::NotConnected("OPC UA".into())) };
        let node = node_id_for(key, link.namespace);
        let session = link.session.clone();
        let timeout = self.cfg.timeout;
        let outcome = self.rt.block_on(async move {
            let current = session
                .read(&[ReadValueId { node_id: node.clone(), attribute_id: AttributeId::Value as u32, ..Default::default() }], TimestampsToReturn::Neither, 0.0)
                .await
                .map_err(|e| e.to_string())?;
            let existing = current.first().and_then(|d| d.value.clone()).ok_or_else(|| "the node has no value to match the type of".to_string())?;
            let variant = like(&existing, &value).ok_or_else(|| format!("{value:?} does not fit the type {existing:?} the PLC holds"))?;
            let w = WriteValue { node_id: node, attribute_id: AttributeId::Value as u32, value: variant.into(), ..Default::default() };
            let codes = tokio::time::timeout(timeout, session.write(&[w])).await.map_err(|_| "write timed out".to_string())?.map_err(|e| e.to_string())?;
            match codes.first() {
                Some(c) if c.is_good() => Ok(()),
                Some(c) => Err(format!("the PLC refused the write: {c}")),
                None => Err("no answer to the write".to_string()),
            }
        });
        outcome.map_err(Error::Invalid)
    }
}

impl Drop for OpcUaDriver {
    fn drop(&mut self) {
        self.disconnect();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn siemens_names() {
        assert_eq!(symbolic_node_name("Gantry_01_Live.driveX.cmd.start"), r#""Gantry_01_Live"."driveX"."cmd"."start""#);
        assert_eq!(symbolic_node_name("Limits_01.rows[3].enabled"), r#""Limits_01"."rows"[3]."enabled""#);
        assert_eq!(symbolic_node_name("D.a[1][2]"), r#""D"."a"[1][2]"#);
    }

    #[test]
    fn values_both_ways() {
        assert_eq!(to_value(&Variant::Int16(-4)), Some(Value::Int(-4)));
        assert_eq!(to_value(&Variant::Float(1.5)), Some(Value::Real(1.5)));
        assert!(like(&Variant::Byte(0), &Value::Int(300)).is_none());
        assert_eq!(like(&Variant::Double(0.0), &Value::Int(2)), Some(Variant::Double(2.0)));
        assert!(like(&Variant::Boolean(false), &Value::Real(1.0)).is_none());
    }

    #[test]
    fn nothing_listening_is_a_failed_link_not_a_hang() {
        let dir = std::env::temp_dir().join(format!("ittoot-opcua-pki-{}", std::process::id()));
        let mut cfg = OpcUaConfig::new("opc.tcp://127.0.0.1:1", &dir);
        cfg.timeout = Duration::from_secs(2);
        let mut d = OpcUaDriver::new(cfg).unwrap();
        assert!(d.connect().is_err());
        assert!(matches!(d.state(), LinkState::Failed(_)));
        assert!(d.read(&["a.b".into()]).is_err());
        let _ = std::fs::remove_dir_all(dir);
    }
}
