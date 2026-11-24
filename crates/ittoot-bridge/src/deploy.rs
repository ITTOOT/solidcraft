//! The deployment record: everything the bridge decided and everything it did to Solidcraft, written
//! down so the same model can be built again on any machine without anyone remembering the steps.
//!
//! A deployment folder holds
//! - `deployment.json`: the sizes with their evidence, the document the part names came from, the
//!   parts, joints and bindings, the Solidcraft it needs, and the command script;
//! - `model-script.json`: the command script alone, in the form `solidcraft-cli run` takes;
//! - `journal.jsonl`: one line per command sent to a running Solidcraft, with its answer;
//! - `REBUILD.txt`: the lines to run to build it again, and what to check afterwards.
//!
//! Nothing is done to Solidcraft by hand: whatever the bridge does, it can say, and `verify` checks
//! that a record only asks Solidcraft for commands the bridge knows how to explain.

use crate::error::{Error, Result};
use crate::infer::Inference;
use crate::model::{ModelBinding, ModelPlan};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value as Json};
use std::io::Write;
use std::path::{Path, PathBuf};

pub const RECORD_VERSION: u32 = 1;

/// The Solidcraft commands the bridge sends. A record that needs any other is refused by `verify`.
pub const KNOWN_COMMANDS: &[&str] = &["solid.box", "component.create", "component.move_bodies", "occurrence.ground", "joint.create", "joint.motion_link", "joint.drive", "appearance.assign"];

/// What the bridge needs of Solidcraft.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SolidcraftNeeds {
    /// The Solidcraft the record was made and checked with (its workspace version).
    pub version: String,
    /// The commands the script uses, sorted.
    pub commands: Vec<String>,
    /// How the bridge reaches a running Solidcraft: the control channel, `engine.execute`.
    pub channel: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SizeRecord {
    pub key: String,
    pub subject: String,
    pub quantity: String,
    pub value: f64,
    pub unit: String,
    pub confidence: String,
    pub method: String,
    pub evidence: Vec<String>,
    pub set_by_user: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BindingRecord {
    pub joint: String,
    pub signal: String,
    pub scale: f64,
    pub offset: f64,
    pub set_by_user: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Deployment {
    pub record_version: u32,
    /// The job title (type, number, optional name) the record was made under.
    #[serde(default)]
    pub job: String,
    /// Seconds since 1970; the record does not depend on a calendar crate.
    pub created_unix: u64,
    pub bridge_version: String,
    pub solidcraft: SolidcraftNeeds,
    pub projects: Vec<String>,
    pub document_folders: Vec<String>,
    /// The document the part names came from, empty when none.
    pub roles_from: String,
    pub roles: Vec<(String, String)>,
    pub sizes: Vec<SizeRecord>,
    pub warnings: Vec<String>,
    pub notes: Vec<String>,
    pub parts: Vec<String>,
    pub joints: Vec<String>,
    pub bindings: Vec<BindingRecord>,
    /// The command script, exactly as sent.
    pub commands: Vec<Json>,
}

/// Everything the record is made from.
pub struct Inputs<'a> {
    pub projects: &'a [String],
    pub document_folders: &'a [String],
    pub roles_from: &'a str,
    pub roles: &'a [(String, Vec<char>)],
    pub inference: &'a Inference,
    pub plan: &'a ModelPlan,
    pub bindings: &'a [ModelBinding],
    /// The Solidcraft version in use (the bridge is built from the same workspace).
    pub solidcraft_version: &'a str,
    /// The job title (number and optional name) the record is made under.
    pub job: &'a str,
}

pub fn now_unix() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

impl Deployment {
    pub fn from_state(i: &Inputs<'_>) -> Deployment {
        let commands = i.plan.commands();
        let mut used: Vec<String> = commands.iter().filter_map(|c| c["command"].as_str().map(str::to_string)).collect();
        used.push("joint.drive".into()); // sent live once bound
        used.sort();
        used.dedup();
        Deployment {
            record_version: RECORD_VERSION,
            created_unix: now_unix(),
            job: i.job.to_string(),
            bridge_version: env!("CARGO_PKG_VERSION").to_string(),
            solidcraft: SolidcraftNeeds { version: i.solidcraft_version.to_string(), commands: used, channel: "control channel: engine.execute".into() },
            projects: i.projects.to_vec(),
            document_folders: i.document_folders.to_vec(),
            roles_from: i.roles_from.to_string(),
            roles: i.roles.iter().map(|(r, l)| (r.clone(), l.iter().collect::<String>())).collect(),
            sizes: i
                .inference
                .items
                .iter()
                .map(|s| SizeRecord {
                    key: s.key.clone(),
                    subject: s.subject.clone(),
                    quantity: s.quantity.clone(),
                    value: s.value,
                    unit: s.unit.to_string(),
                    confidence: format!("{:?}", s.confidence).to_lowercase(),
                    method: s.method.clone(),
                    evidence: s.evidence.iter().map(|e| format!("{}: {}", e.source, e.detail)).collect(),
                    set_by_user: s.overridden,
                })
                .collect(),
            warnings: i.inference.warnings.clone(),
            notes: i.plan.notes.clone(),
            parts: i.plan.parts.iter().map(|p| p.name.clone()).collect(),
            joints: i.plan.joints.iter().map(|j| j.name.clone()).collect(),
            bindings: i.bindings.iter().map(|b| BindingRecord { joint: b.joint.clone(), signal: b.key.clone(), scale: b.scale, offset: b.offset, set_by_user: !b.auto }).collect(),
            commands,
        }
    }

    /// The record is only trusted when it asks for commands the bridge knows, and every joint it
    /// binds exists.
    pub fn verify(&self) -> Result<()> {
        if self.record_version > RECORD_VERSION {
            return Err(Error::Invalid(format!("the record is version {}, newer than this bridge reads ({RECORD_VERSION})", self.record_version)));
        }
        for c in &self.commands {
            let name = c["command"].as_str().unwrap_or("");
            if !KNOWN_COMMANDS.contains(&name) {
                return Err(Error::Invalid(format!("the script uses a command the bridge does not know: {name:?}")));
            }
        }
        for b in &self.bindings {
            if !self.joints.contains(&b.joint) {
                return Err(Error::Invalid(format!("a binding names joint {:?}, which the model does not have", b.joint)));
            }
        }
        Ok(())
    }

    /// The text that says how to build it again.
    pub fn rebuild_text(&self, folder: &Path) -> String {
        let script = folder.join("model-script.json");
        let mut t = String::new();
        t.push_str(&format!("{}\n\n", if self.job.is_empty() { "Rebuild this model".to_string() } else { format!("{}: rebuild this model", self.job) }));
        t.push_str(&format!("Made by bridge {} for Solidcraft {}.\n\n", self.bridge_version, self.solidcraft.version));
        t.push_str("1. Headless, to a STEP file and a picture:\n\n");
        t.push_str(&format!("   solidcraft-cli run \"{}\" --out model.step --save model.sc\n", script.display()));
        t.push_str(&format!("   solidcraft-cli snapshot \"{}\" --out model.png --view iso\n\n", script.display()));
        t.push_str("2. Into a running Solidcraft: start it with `--control PORT`, then in the bridge's Model page\n   press Connect and Build the model in Solidcraft. Every command is written to journal.jsonl.\n\n");
        t.push_str("3. Check afterwards: `solidcraft-cli exec joint.list` on the saved design lists these joints:\n");
        for j in &self.joints {
            t.push_str(&format!("   - {j}\n"));
        }
        t.push_str("\nSignals that move them:\n");
        for b in &self.bindings {
            t.push_str(&format!("   - {} <- {} ({})\n", b.joint, b.signal, if b.set_by_user { "set by the user" } else { "found by the rules" }));
        }
        t.push_str(&format!("\nCommands used: {}.\n", self.solidcraft.commands.join(", ")));
        if !self.warnings.is_empty() {
            t.push_str("\nWarnings when it was made:\n");
            for w in &self.warnings {
                t.push_str(&format!("   - {w}\n"));
            }
        }
        t
    }

    /// Write the record into `folder` (created if needed). Returns the folder.
    pub fn save(&self, folder: &Path) -> Result<PathBuf> {
        std::fs::create_dir_all(folder).map_err(|e| Error::io(folder, e))?;
        let write = |name: &str, text: String| -> Result<()> {
            let p = folder.join(name);
            std::fs::write(&p, text).map_err(|e| Error::io(&p, e))
        };
        write("deployment.json", serde_json::to_string_pretty(self).map_err(|e| Error::Invalid(e.to_string()))?)?;
        write("model-script.json", serde_json::to_string_pretty(&self.commands).map_err(|e| Error::Invalid(e.to_string()))?)?;
        write("REBUILD.txt", self.rebuild_text(folder))?;
        Ok(folder.to_path_buf())
    }

    pub fn load(folder: &Path) -> Result<Deployment> {
        let p = folder.join("deployment.json");
        let text = std::fs::read_to_string(&p).map_err(|e| Error::io(&p, e))?;
        let d: Deployment = serde_json::from_str(&text).map_err(|e| Error::Parse { path: p.display().to_string(), message: e.to_string() })?;
        d.verify()?;
        Ok(d)
    }
}

/// Add one line to the journal: what was sent to Solidcraft and what came back.
pub fn journal(folder: &Path, command: &Json, outcome: &str) -> Result<()> {
    std::fs::create_dir_all(folder).map_err(|e| Error::io(folder, e))?;
    let p = folder.join("journal.jsonl");
    let mut f = std::fs::OpenOptions::new().create(true).append(true).open(&p).map_err(|e| Error::io(&p, e))?;
    let line = json!({"unix": now_unix(), "command": command["command"], "params": command["params"], "outcome": outcome});
    writeln!(f, "{line}").map_err(|e| Error::io(&p, e))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infer::{infer, station_points, Record};
    use crate::model::{auto_bind, plan};
    use std::collections::BTreeMap;

    fn rec(array: &str, index: usize, f: &[(&str, &str)]) -> Record {
        Record { db: "D".into(), array: array.into(), index, fields: f.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect() }
    }

    fn state() -> (Inference, ModelPlan, Vec<ModelBinding>) {
        let mut recs = Vec::new();
        for (axis, hi) in [(0, 11900.0), (1, 5900.0), (2, 2900.0)] {
            for (i, (n, p)) in [("LIM_LOW_LOW", 20.0), ("LIM_HIGH_HIGH", hi + 80.0)].into_iter().enumerate() {
                recs.push(rec("rows", axis * 2 + i, &[("axis", &axis.to_string()), ("sensor", &format!("'{n}'")), ("triggerPosition", &p.to_string())]));
            }
        }
        let inf = infer(&recs, &BTreeMap::new());
        let p = plan(&inf, &station_points(&recs), &[]).unwrap();
        let b = auto_bind(&p, &["T.axes[0].drive.status.actualPosition", "T.axes[1].drive.status.actualPosition", "T.axes[2].drive.status.actualPosition"]);
        (inf, p, b)
    }

    #[test]
    fn a_record_saves_loads_and_says_how_to_rebuild() {
        let (inf, p, b) = state();
        let d = Deployment::from_state(&Inputs { projects: &["x.ap18".into()], document_folders: &[], roles_from: "", roles: &[], inference: &inf, plan: &p, bindings: &b, solidcraft_version: "0.44.0", job: "Plating Line J-1" });
        let dir = std::env::temp_dir().join(format!("ittoot-bridge-deploy-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        d.save(&dir).unwrap();
        let back = Deployment::load(&dir).unwrap();
        assert_eq!(back, d);
        assert_eq!(back.bindings.len(), 3);
        assert!(back.solidcraft.commands.contains(&"joint.drive".to_string()));
        let text = std::fs::read_to_string(dir.join("REBUILD.txt")).unwrap();
        assert!(text.contains("solidcraft-cli run") && text.contains("Axis X <- T.axes[0]"), "{text}");
        journal(&dir, &d.commands[0], "ok").unwrap();
        journal(&dir, &d.commands[1], "refused: x").unwrap();
        assert_eq!(std::fs::read_to_string(dir.join("journal.jsonl")).unwrap().lines().count(), 2);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_record_with_an_unknown_command_or_joint_is_refused() {
        let (inf, p, b) = state();
        let mut d = Deployment::from_state(&Inputs { projects: &[], document_folders: &[], roles_from: "", roles: &[], inference: &inf, plan: &p, bindings: &b, solidcraft_version: "0.44.0", job: "Plating Line J-1" });
        assert!(d.verify().is_ok());
        d.commands.push(json!({"command":"file.delete","params":{}}));
        assert!(d.verify().unwrap_err().to_string().contains("file.delete"));
        d.commands.pop();
        d.bindings[0].joint = "Ghost".into();
        assert!(d.verify().is_err());
        d.bindings[0].joint = d.joints[0].clone();
        d.record_version = 99;
        assert!(d.verify().is_err());
    }
}
