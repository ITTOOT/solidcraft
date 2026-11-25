//! Contracts and result types for the `physics.*` commands in `cmd/physics.rs`: the digital-twin physics that runs inside the engine.

use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Start a physics run on a design from the design agent (`geometry.parts`, `joints`, belts, loads, vessels).
#[derive(Debug, Deserialize, JsonSchema)]
#[schemars(deny_unknown_fields)]
pub struct PhysicsLoadParams {
    /// The design: the JSON the design agent writes (`meta`, `spec`, `geometry`).
    pub design: Value,
    /// Settings to start from, as the Physics page keeps them (default: a plating jig in air at 20 C). Anything left out keeps its default.
    #[serde(default)]
    pub settings: Option<Value>,
    /// The joints of the gantry's X, Y and Z axes (a hoist design); the PLC's commanded values for them go through the physics.
    #[serde(default)]
    pub axes: Option<Vec<String>>,
}

/// Change settings of the run in place; the world is not rebuilt unless a setting that shapes it changed.
#[derive(Debug, Deserialize, JsonSchema)]
#[schemars(deny_unknown_fields)]
pub struct PhysicsSettingsParams {
    /// Only the settings to change, by their names on the Physics page's file (`gravity`, `sway`, `plc`, `loads`, ...).
    pub settings: Value,
}

/// Advance the physics by `dt` seconds and drive the model's joints with what it finds.
#[derive(Debug, Deserialize, JsonSchema)]
#[schemars(deny_unknown_fields)]
pub struct PhysicsStepParams {
    /// Seconds since the last step, 0 to 60 (anything above half a second is taken as half a second).
    pub dt: f64,
    /// The PLC's commanded value for each joint it drives (the gantry's X, Y, Z in mm).
    #[serde(default)]
    pub commanded: BTreeMap<String, f64>,
    /// PLC signals the physics reads, by the key the PLC links name: a bool is 0 or 1, a valve above 1 is a percentage.
    #[serde(default)]
    pub signals: BTreeMap<String, f64>,
    /// With `false` the joints are not driven; the reply still says what would be.
    #[serde(default)]
    pub apply: Option<bool>,
    /// Keys of writes from earlier steps that did not reach the PLC: they are offered again.
    #[serde(default)]
    pub unsent: Vec<String>,
}

/// Forget the swing, the levels, the cartons: start again from rest.
#[derive(Debug, Deserialize, JsonSchema)]
#[schemars(deny_unknown_fields)]
pub struct PhysicsResetParams {}

/// Take the solids' sizes and places from the document as drawn now.
#[derive(Debug, Deserialize, JsonSchema)]
#[schemars(deny_unknown_fields)]
pub struct PhysicsSyncParams {}

/// What `physics.sync` returns.
#[derive(Debug, Serialize, JsonSchema)]
pub struct PhysicsSynced {
    /// Solid bodies of the design that were found in the document and measured again.
    pub updated: usize,
    /// Solid bodies of the design that were moved or resized by the edit.
    pub changed: usize,
    /// Names of solid bodies the document does not have (the design's numbers are kept for these).
    pub missing: Vec<String>,
}

/// What `physics.load` returns.
#[derive(Debug, Serialize, JsonSchema)]
pub struct PhysicsLoaded {
    /// The design's application: plating or hoist, conveyor_line, robot_cell, process.
    pub application: String,
    /// The joints the run will drive if the model has them.
    pub axes: Vec<String>,
}

/// What `physics.settings` returns.
#[derive(Debug, Serialize, JsonSchema)]
pub struct PhysicsSettings {
    /// Every setting after the change.
    pub settings: Value,
}

/// A joint value the physics drives.
#[derive(Debug, Serialize, JsonSchema)]
pub struct PhysicsDrive {
    pub joint: String,
    pub value: f64,
    /// Whether the model has the joint and took the value.
    pub applied: bool,
}

/// A signal the PLC should be given.
#[derive(Debug, Serialize, JsonSchema)]
pub struct PhysicsWrite {
    pub key: String,
    /// The value as a number (a bool is 0 or 1).
    pub value: f64,
    /// Whether the signal is a bool.
    pub bool: bool,
}

/// What `physics.step` returns.
#[derive(Debug, Serialize, JsonSchema)]
pub struct PhysicsStep {
    pub drives: Vec<PhysicsDrive>,
    /// Signals for the PLC; only those that changed since the last step.
    pub writes: Vec<PhysicsWrite>,
    /// What the model lacks for the physics to show everything (empty when nothing).
    pub message: String,
    /// A hoist: whether X, Y and Z are held back by a solid.
    pub blocked: Vec<bool>,
    /// A hoist: times the load has met a solid, and the hardest hit so far (N).
    pub hits: u32,
    pub peak_impact_n: f64,
    /// Conveyor line or robot cell: cartons on belts or carried, fallen, released so far.
    pub on_belts: usize,
    pub fallen: usize,
    pub released: usize,
    /// Process plant: each vessel's fill in percent.
    pub levels: BTreeMap<String, f64>,
}

/// What `physics.reset` returns.
#[derive(Debug, Serialize, JsonSchema)]
pub struct PhysicsReset {
    pub reset: bool,
}
