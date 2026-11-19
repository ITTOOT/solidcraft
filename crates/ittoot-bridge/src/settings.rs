//! Everything the user typed on the Settings and Connections pages, saved as one JSON file.

use crate::area::{AreaRole, DataArea, Origin};
use crate::connection::Connection;
use crate::error::{Error, Result};
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BridgeSettings {
    /// Settings file version, for upgrades.
    pub version: u32,
    /// The controls projects the user gave (a TIA `.ap18`, a folder, an export). Any number.
    #[serde(default)]
    pub projects: Vec<String>,
    /// Folders the intake scans for drawings, FDS, photos ...
    #[serde(default)]
    pub document_folders: Vec<String>,
    #[serde(default)]
    pub areas: Vec<DataArea>,
    #[serde(default)]
    pub connections: Vec<Connection>,
    /// Sizes the user set over the inferred ones, by key (`axis0.length`).
    #[serde(default)]
    pub overrides: std::collections::BTreeMap<String, f64>,
    /// The connection the tables read from; `None` means the start values in the sources.
    #[serde(default)]
    pub active_link: Option<String>,
    /// The user's choice of PLC signal for a joint, by joint name; an empty key unbinds the joint.
    #[serde(default)]
    pub bindings: std::collections::BTreeMap<String, String>,
    /// The Solidcraft instances the bridge builds models in, one per machine or site. Add as many as needed.
    #[serde(default)]
    pub models: Vec<crate::target::ModelTarget>,
    /// The model the Model page works on.
    #[serde(default)]
    pub active_model: Option<String>,
}

pub const CURRENT_VERSION: u32 = 1;

impl Default for BridgeSettings {
    fn default() -> Self {
        BridgeSettings { version: CURRENT_VERSION, projects: Vec::new(), document_folders: Vec::new(), areas: Vec::new(), connections: Vec::new(), overrides: Default::default(), active_link: None, bindings: Default::default(), models: Vec::new(), active_model: None }
    }
}

impl BridgeSettings {
    pub fn load(path: &Path) -> Result<Self> {
        match std::fs::read_to_string(path) {
            Ok(text) => {
                let s: BridgeSettings = serde_json::from_str(&text).map_err(|e| Error::Parse { path: path.display().to_string(), message: e.to_string() })?;
                if s.version > CURRENT_VERSION {
                    return Err(Error::Parse { path: path.display().to_string(), message: format!("saved by a newer version ({})", s.version) });
                }
                Ok(s)
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(BridgeSettings::default()),
            Err(e) => Err(Error::io(path, e)),
        }
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        let text = serde_json::to_string_pretty(self).map_err(|e| Error::Invalid(e.to_string()))?;
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| Error::io(dir, e))?;
        }
        // Write beside, then rename: a crash mid-save leaves the old file whole.
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, text).map_err(|e| Error::io(&tmp, e))?;
        std::fs::rename(&tmp, path).map_err(|e| Error::io(path, e))
    }

    /// The areas shown on one tab.
    pub fn areas_of(&self, role: AreaRole) -> impl Iterator<Item = &DataArea> {
        self.areas.iter().filter(move |a| a.role == role)
    }

    /// The "+" button: a new empty area on a tab. Returns its id.
    pub fn add_area(&mut self, role: AreaRole) -> String {
        let id = self.fresh_id("area");
        self.areas.push(DataArea::new(id.clone(), role, crate::area::AreaKind::DataBlock, ""));
        id
    }

    pub fn remove_area(&mut self, id: &str) -> bool {
        let before = self.areas.len();
        self.areas.retain(|a| a.id != id);
        self.areas.len() != before
    }

    fn fresh_id(&self, prefix: &str) -> String {
        let mut n = self.areas.len() + self.connections.len() + 1;
        loop {
            let id = format!("{prefix}{n}");
            if !self.areas.iter().any(|a| a.id == id) && !self.connections.iter().any(|c| c.id == id) {
                return id;
            }
            n += 1;
        }
    }

    pub fn add_connection(&mut self, c: Connection) -> Result<()> {
        if self.connections.iter().any(|x| x.id == c.id) {
            return Err(Error::Invalid(format!("connection id {} is already used", c.id)));
        }
        self.connections.push(c);
        Ok(())
    }

    /// Merge auto-config results: new areas are added, areas the user made or edited are never touched,
    /// auto areas that are gone from disk are dropped. Returns (added, removed).
    pub fn merge_auto(&mut self, found: Vec<DataArea>) -> (usize, usize) {
        let keep: Vec<DataArea> = self.areas.iter().filter(|a| a.origin == Origin::User).cloned().collect();
        let old_auto: Vec<DataArea> = self.areas.iter().filter(|a| a.origin == Origin::Auto).cloned().collect();
        let mut added = 0;
        let mut next = keep;
        for mut f in found {
            if next.iter().any(|a| a.source == f.source && a.role == f.role) {
                continue; // the user already has it
            }
            f.origin = Origin::Auto;
            if !old_auto.iter().any(|a| a.source == f.source) {
                added += 1;
            }
            next.push(f);
        }
        let removed = old_auto.iter().filter(|a| !next.iter().any(|n| n.source == a.source)).count();
        // ids must stay unique after the merge
        let mut seen = std::collections::HashSet::new();
        for a in &mut next {
            let mut id = a.id.clone();
            let mut n = 1;
            while !seen.insert(id.clone()) {
                n += 1;
                id = format!("{}-{n}", a.id);
            }
            a.id = id;
        }
        self.areas = next;
        (added, removed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::area::AreaKind;

    #[test]
    fn plus_button_adds_unique_areas_and_round_trips() {
        let mut s = BridgeSettings::default();
        let a = s.add_area(AreaRole::Controls);
        let b = s.add_area(AreaRole::Management);
        assert_ne!(a, b);
        assert_eq!(s.areas_of(AreaRole::Controls).count(), 1);
        let dir = std::env::temp_dir().join(format!("ittoot-bridge-settings-{}", std::process::id()));
        let file = dir.join("settings.json");
        s.save(&file).unwrap();
        assert_eq!(BridgeSettings::load(&file).unwrap(), s);
        assert!(s.remove_area(&a));
        assert!(!s.remove_area(&a));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn newer_file_is_refused() {
        let dir = std::env::temp_dir().join(format!("ittoot-bridge-newer-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("s.json");
        std::fs::write(&file, r#"{"version": 99}"#).unwrap();
        assert!(BridgeSettings::load(&file).is_err());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn auto_merge_keeps_user_areas_and_drops_stale_auto() {
        let mut s = BridgeSettings::default();
        s.areas.push(DataArea::new("u", AreaRole::Controls, AreaKind::DataBlock, "A.scl"));
        let mut stale = DataArea::new("old", AreaRole::Controls, AreaKind::DataBlock, "Gone.scl");
        stale.origin = Origin::Auto;
        s.areas.push(stale);
        let found = vec![DataArea::new("n1", AreaRole::Controls, AreaKind::DataBlock, "A.scl"), DataArea::new("n2", AreaRole::Controls, AreaKind::DataBlock, "B.scl")];
        let (added, removed) = s.merge_auto(found);
        assert_eq!((added, removed), (1, 1));
        assert_eq!(s.areas.len(), 2);
        assert!(s.areas.iter().any(|a| a.source == "B.scl" && a.origin == Origin::Auto));
        assert!(s.areas.iter().any(|a| a.id == "u" && a.origin == Origin::User));
    }
}
