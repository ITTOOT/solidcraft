//! A data area: somewhere in the PLC (or a file that describes it) the bridge reads signals from.

use serde::{Deserialize, Serialize};

/// Which settings tab an area lives on.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AreaRole {
    /// What runs the machine now: live control and status, I/O, drives. Drives the model.
    Controls,
    /// What configures and supervises it: settings, modes, messages, logs, heartbeat.
    Management,
}

/// What kind of thing `source` names.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AreaKind {
    /// One data block: a source file (.scl / .db / .xml) or the DB's symbolic name.
    DataBlock,
    /// Inputs / outputs: an I/O list (.csv) or a tag-table export.
    Io,
    /// A whole folder; every area found inside is used.
    Folder,
    /// Any other single file.
    File,
    /// A database (SQL Server, PostgreSQL, SQLite) given by connection id.
    Database,
}

/// Where an area came from. The user may edit both; auto-config never overwrites a `User` area.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Origin {
    #[default]
    User,
    Auto,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DataArea {
    /// Stable within the settings; never shown.
    pub id: String,
    pub role: AreaRole,
    pub kind: AreaKind,
    /// Shown name; defaults to the file or DB name.
    pub label: String,
    /// A path, or a symbolic name (`"Gantry_01_Live"`), or a connection id for `Database`.
    pub source: String,
    /// The connection (PLC) this area is read from, if more than one is set up.
    #[serde(default)]
    pub connection: Option<String>,
    #[serde(default = "yes")]
    pub enabled: bool,
    #[serde(default)]
    pub origin: Origin,
    #[serde(default)]
    pub notes: String,
}

fn yes() -> bool {
    true
}

impl DataArea {
    pub fn new(id: impl Into<String>, role: AreaRole, kind: AreaKind, source: impl Into<String>) -> Self {
        let source = source.into();
        let label = label_of(&source);
        DataArea { id: id.into(), role, kind, label, source, connection: None, enabled: true, origin: Origin::User, notes: String::new() }
    }
}

/// The last path segment without its extension: `C:\x\Limits_01.scl` becomes `Limits_01`.
pub fn label_of(source: &str) -> String {
    let last = source.rsplit(['\\', '/']).next().unwrap_or(source);
    match last.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() && ext.len() <= 5 => stem.to_string(),
        _ => last.trim_matches('"').to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn label_drops_folder_and_extension() {
        assert_eq!(label_of(r"C:\a b\Limits_01.scl"), "Limits_01");
        assert_eq!(label_of("/x/io_list.csv"), "io_list");
        assert_eq!(label_of("\"Gantry_01_Live\""), "Gantry_01_Live");
        assert_eq!(label_of(""), "");
    }
}
