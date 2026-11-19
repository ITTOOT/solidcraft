//! A model target: one Solidcraft the bridge builds a model in. There can be many (one per machine,
//! one per site), each with its own address, port, token and design file.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ModelTarget {
    pub id: String,
    pub name: String,
    /// `127.0.0.1` for a Solidcraft this PC starts; another address for one already running elsewhere.
    pub host: String,
    pub port: u16,
    /// Path of `solidcraft.exe`; empty means look for it (see [`find_exe`]).
    #[serde(default)]
    pub exe: String,
    /// A design (`.sc`) to open when the bridge starts Solidcraft. Optional.
    #[serde(default)]
    pub design: String,
    /// Software rendering, for a PC whose graphics card cannot be read back.
    #[serde(default)]
    pub cpu_render: bool,
    #[serde(default)]
    pub notes: String,
}

pub const DEFAULT_PORT: u16 = 38400;

impl ModelTarget {
    pub fn new(id: impl Into<String>, name: impl Into<String>, port: u16) -> Self {
        ModelTarget { id: id.into(), name: name.into(), host: "127.0.0.1".into(), port, exe: String::new(), design: String::new(), cpu_render: false, notes: String::new() }
    }

    /// True when this PC can start it: the address is this PC.
    pub fn is_local(&self) -> bool {
        matches!(self.host.trim().to_ascii_lowercase().as_str(), "" | "127.0.0.1" | "localhost" | "::1")
    }

    /// Where the control channel is.
    pub fn address(&self) -> String {
        let h = if self.is_local() { "127.0.0.1" } else { self.host.trim() };
        format!("{h}:{}", self.port)
    }

    /// Arguments for `solidcraft.exe`: its own control channel on this target's port, its own window.
    pub fn launch_args(&self) -> Vec<String> {
        let mut a = vec!["--control".to_string(), self.port.to_string(), "--new-instance".to_string()];
        if !self.design.trim().is_empty() {
            a.push(self.design.trim().to_string());
        }
        a
    }

    /// Environment for the launch: the token the bridge will present, and the render mode.
    pub fn launch_env(&self, token: &str) -> Vec<(String, String)> {
        let mut e = vec![("SOLIDCRAFT_CONTROL_TOKEN".to_string(), token.to_string())];
        if self.cpu_render {
            e.push(("SOLIDCRAFT_GPU".to_string(), "cpu".to_string()));
        }
        e
    }

    pub fn problems(&self) -> Vec<String> {
        let mut p = Vec::new();
        if self.name.trim().is_empty() {
            p.push("the model needs a name".to_string());
        }
        if self.port == 0 {
            p.push("the control port must be 1 to 65535".to_string());
        }
        if self.host.trim().contains(char::is_whitespace) {
            p.push("the address has a space in it".to_string());
        }
        p
    }
}

/// A port no other model uses, counting up from the default.
pub fn next_free_port(models: &[ModelTarget]) -> u16 {
    let mut port = DEFAULT_PORT;
    while models.iter().any(|m| m.port == port) && port < u16::MAX {
        port += 1;
    }
    port
}

/// An id no other model uses.
pub fn next_id(models: &[ModelTarget]) -> String {
    let mut n = models.len() + 1;
    loop {
        let id = format!("model{n}");
        if !models.iter().any(|m| m.id == id) {
            return id;
        }
        n += 1;
    }
}

/// Find `solidcraft.exe`: the path given, else `ITTOOT_SOLIDCRAFT_EXE`, else beside the bridge, else
/// in the workspace `target` folders, else the usual install folder.
pub fn find_exe(hint: &str) -> Option<PathBuf> {
    let mut tries: Vec<PathBuf> = Vec::new();
    if !hint.trim().is_empty() {
        tries.push(PathBuf::from(hint.trim()));
    }
    if let Some(v) = std::env::var_os("ITTOOT_SOLIDCRAFT_EXE") {
        tries.push(PathBuf::from(v));
    }
    if let Ok(me) = std::env::current_exe()
        && let Some(dir) = me.parent()
    {
        tries.push(dir.join("solidcraft.exe"));
        tries.push(dir.join("solidcraft"));
    }
    if let Some(v) = std::env::var_os("LOCALAPPDATA") {
        tries.push(PathBuf::from(v).join("Programs").join("SolidCraft").join("solidcraft.exe"));
    }
    tries.into_iter().find(|p| p.is_file())
}

/// A token for one launch: 64 hex digits from the system's random seeds. The control channel is
/// loopback-only and the token is never written to the settings file.
pub fn new_token() -> String {
    use std::hash::{BuildHasher, Hasher};
    let mut out = String::with_capacity(64);
    for i in 0..4u64 {
        let mut h = std::collections::hash_map::RandomState::new().build_hasher();
        h.write_u64(i);
        h.write_u128(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0));
        h.write_u32(std::process::id());
        out.push_str(&format!("{:016x}", h.finish()));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn launch_uses_its_own_port_token_and_design() {
        let mut m = ModelTarget::new("model1", "Gantry", 38401);
        m.design = "C:/x/gantry.sc".into();
        m.cpu_render = true;
        assert_eq!(m.launch_args(), ["--control", "38401", "--new-instance", "C:/x/gantry.sc"]);
        let env = m.launch_env("t".repeat(40).as_str());
        assert!(env.iter().any(|(k, v)| k == "SOLIDCRAFT_CONTROL_TOKEN" && v.len() == 40));
        assert!(env.iter().any(|(k, v)| k == "SOLIDCRAFT_GPU" && v == "cpu"));
        assert_eq!(m.address(), "127.0.0.1:38401");
        assert!(m.is_local());
    }

    #[test]
    fn remote_models_are_not_started_here_and_ports_do_not_clash() {
        let mut a = ModelTarget::new("model1", "A", DEFAULT_PORT);
        a.host = "10.0.0.5".into();
        assert!(!a.is_local());
        assert_eq!(a.address(), "10.0.0.5:38400");
        let b = ModelTarget::new(next_id(std::slice::from_ref(&a)), "B", next_free_port(std::slice::from_ref(&a)));
        assert_eq!((b.id.as_str(), b.port), ("model2", 38401));
        assert!(ModelTarget::new("x", "", 0).problems().len() == 2);
    }

    #[test]
    fn tokens_are_long_and_differ() {
        let (a, b) = (new_token(), new_token());
        assert!(a.len() >= 32 && a.chars().all(|c| c.is_ascii_hexdigit()));
        assert_ne!(a, b);
    }
}
