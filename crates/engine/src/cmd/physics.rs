//! Digital-twin physics inside the engine (`physics.load`, `physics.settings`, `physics.step`, `physics.reset`).
//!
//! The physics is the `ittoot-physics` crate: a hoist swinging on its rope and held by the walls it meets, baths rocking on their mounts, liquid
//! levels, cartons on belts, vessels filled by pumps. A run is started with a design from the design agent; each `physics.step` takes the PLC's
//! commanded joint values and signals, works out what the machine does, and drives this document's own joints with it, so a controller needs one call
//! per tick and no round trip for every joint. What the PLC should be told comes back in the reply. The run is not part of the document: it is not
//! saved, not undone, and a new document starts without one.

use std::sync::{Arc, Mutex, MutexGuard};

use ittoot_physics::physics::Physics;
use ittoot_physics::runner::{Inputs, Runner, WriteVal};
use serde_json::{Value, json};

use super::CommandSpec;
use crate::params::bad;
use crate::schema::physics::{
    PhysicsDrive, PhysicsLoadParams, PhysicsLoaded, PhysicsReset, PhysicsResetParams, PhysicsSettings, PhysicsSettingsParams, PhysicsStep, PhysicsStepParams,
    PhysicsWrite,
};
use crate::schema::{TypedContract, output_schema_for};
use crate::{Result, Session};

/// The longest `dt` one step accepts (s).
const MAX_DT: f64 = 60.0;

/// A physics run: the design it was started on and everything it has worked out since.
pub struct PhysicsRun {
    runner: Runner,
    design: Value,
    settings: Physics,
    stamp: u64,
    axes: Option<[String; 3]>,
}

/// The run of a session, shared by its copies.
pub type SharedRun = Arc<Mutex<Option<PhysicsRun>>>;

fn lock(run: &SharedRun) -> MutexGuard<'_, Option<PhysicsRun>> {
    run.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

const LOAD: TypedContract<PhysicsLoadParams> = TypedContract::new();
const SETTINGS: TypedContract<PhysicsSettingsParams> = TypedContract::new();
const STEP: TypedContract<PhysicsStepParams> = TypedContract::new();
const RESET: TypedContract<PhysicsResetParams> = TypedContract::new();

pub static COMMANDS: &[CommandSpec] = &[
    CommandSpec::new("physics.load", "Start Physics", crate::checked!("physics.load", load))
        .icon("drive")
        .noundo()
        .contract(&LOAD)
        .output(output_schema_for::<PhysicsLoaded>)
        .params(
            "design: object (the design agent's JSON: meta.application, spec, geometry); settings?: object (the Physics page's settings; anything left out keeps its default); \
             axes?: [x, y, z] (joint names of a gantry's three axes) → application; axes. Starts a physics run on this document: it drives the joints the document has \
             (sway, rock, liquid levels, rope slack, carton and vessel sliders) and holds the gantry where a solid stops it",
        ),
    CommandSpec::new("physics.settings", "Physics Settings", crate::checked!("physics.settings", settings))
        .icon("edit")
        .noundo()
        .contract(&SETTINGS)
        .output(output_schema_for::<PhysicsSettings>)
        .params("settings: object (only what changes, as the Physics page's file names it: gravity, sway, collisions, plc, loads, plant, flow ...; a name the settings do not have is refused) → settings: every setting after the change"),
    CommandSpec::new("physics.step", "Step Physics", crate::checked!("physics.step", step))
        .icon("drive")
        .noundo()
        .contract(&STEP)
        .output(output_schema_for::<PhysicsStep>)
        .params(
            "dt: seconds since the last step (0 to 60; above 0.5 counts as 0.5); commanded?: {joint: value} (the PLC's gantry X, Y, Z); signals?: {key: number} (PLC signals the \
             physics reads: valves, pumps, belt drives; a bool is 0 or 1, above 1 a percentage); apply?: false (do not drive the joints); unsent?: [keys] (writes an earlier step \
             gave that did not reach the PLC) → drives: [{joint, value, applied}] (applied: the document has the joint and took the value); writes: [{key, value, bool}] (for the \
             PLC, only what changed); message; blocked: [x, y, z]; hits; peak_impact_n; on_belts; fallen; released; levels: {vessel: percent}",
        ),
    CommandSpec::new("physics.reset", "Reset Physics", crate::checked!("physics.reset", reset))
        .icon("contact_new")
        .noundo()
        .contract(&RESET)
        .output(output_schema_for::<PhysicsReset>)
        .params("→ reset: the swing, levels, cartons and vessels start again from rest (a run must have been started)"),
];

fn load(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "physics.load";
    let design = p.get("design").filter(|d| d.is_object()).cloned().ok_or_else(|| bad(cmd, "`design` must be the design agent's JSON object"))?;
    let mut settings = Physics::default();
    if let Some(given) = p.get("settings").filter(|v| !v.is_null()) {
        settings = merged(&settings, given).map_err(|e| bad(cmd, e))?;
    }
    let axes = match p.get("axes").filter(|v| !v.is_null()) {
        None => None,
        Some(a) => match a.as_array().map(|a| a.iter().filter_map(Value::as_str).map(str::to_string).collect::<Vec<_>>()) {
            Some(v) if v.len() == 3 => Some([v[0].clone(), v[1].clone(), v[2].clone()]),
            _ => return Err(bad(cmd, "`axes` must be three joint names: X, Y and Z")),
        },
    };
    let application = design["meta"]["application"].as_str().unwrap_or("plating").to_string();
    let mut slot = lock(&s.physics_run);
    let stamp = slot.as_ref().map(|r| r.stamp + 1).unwrap_or(1);
    *slot = Some(PhysicsRun { runner: Runner::new(), design, settings, stamp, axes: axes.clone() });
    Ok(json!({"application": application, "axes": axes.map(|a| a.to_vec()).unwrap_or_default()}))
}

/// `base` with the fields of `patch` laid over it, refused if the patch names a setting `base` does not have or gives one the wrong kind of value.
fn merged(base: &Physics, patch: &Value) -> std::result::Result<Physics, String> {
    let mut current = serde_json::to_value(base).map_err(|e| e.to_string())?;
    overlay(&mut current, patch, "")?;
    serde_json::from_value(current).map_err(|e| format!("the settings are not valid: {e}"))
}

fn overlay(into: &mut Value, patch: &Value, path: &str) -> std::result::Result<(), String> {
    match (into, patch) {
        (Value::Object(have), Value::Object(give)) => {
            for (k, v) in give {
                let here = if path.is_empty() { k.clone() } else { format!("{path}.{k}") };
                match have.get_mut(k) {
                    Some(slot) => overlay(slot, v, &here)?,
                    None => return Err(format!("there is no setting `{here}`")),
                }
            }
            Ok(())
        }
        (slot, v) => {
            if std::mem::discriminant(slot) != std::mem::discriminant(v) && !(slot.is_number() && v.is_number()) {
                return Err(format!("`{path}` is the wrong kind of value"));
            }
            *slot = v.clone();
            Ok(())
        }
    }
}

fn settings(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "physics.settings";
    let patch = p.get("settings").filter(|v| v.is_object()).ok_or_else(|| bad(cmd, "`settings` must be an object"))?;
    let mut slot = lock(&s.physics_run);
    let Some(run) = slot.as_mut() else { return Err(bad(cmd, "no physics run: physics.load starts one")) };
    run.settings = merged(&run.settings, patch).map_err(|e| bad(cmd, e))?;
    // a setting that shapes the world (wall thickness, solid parts) is picked up at the next step; a change of the design's stamp is not needed
    run.runner.invalidate();
    let all = serde_json::to_value(&run.settings).map_err(|e| bad(cmd, e.to_string()))?;
    Ok(json!({"settings": all}))
}

fn step(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "physics.step";
    let dt = p.get("dt").and_then(Value::as_f64).filter(|d| d.is_finite() && (0.0..=MAX_DT).contains(d)).ok_or_else(|| bad(cmd, format!("`dt` must be a number of seconds from 0 to {MAX_DT}")))?;
    let numbers = |key: &str| -> Result<std::collections::BTreeMap<String, f64>> {
        let mut out = std::collections::BTreeMap::new();
        if let Some(o) = p.get(key).filter(|v| !v.is_null()) {
            for (k, v) in o.as_object().ok_or_else(|| bad(cmd, format!("`{key}` must be an object of numbers")))? {
                let n = v.as_f64().filter(|n| n.is_finite()).ok_or_else(|| bad(cmd, format!("`{key}.{k}` must be a finite number")))?;
                out.insert(k.clone(), n);
            }
        }
        Ok(out)
    };
    let commanded = numbers("commanded")?;
    let signals = numbers("signals")?;
    let apply = p.get("apply").and_then(Value::as_bool).unwrap_or(true);
    let live: Vec<String> = s.doc.assembly.joints.iter().map(|j| j.name.clone()).collect();
    let (out, reply) = {
        let mut slot = lock(&s.physics_run);
        let Some(run) = slot.as_mut() else { return Err(bad(cmd, "no physics run: physics.load starts one")) };
        if let Some(keys) = p.get("unsent").and_then(Value::as_array) {
            for k in keys.iter().filter_map(Value::as_str) {
                run.runner.forget(k);
            }
        }
        let inp = Inputs { dt, axes: run.axes.clone(), commanded: &commanded, live_joints: Some(&live), signal: &|k| signals.get(k).copied() };
        let out = run.runner.step(&run.settings, &run.design, run.stamp, &inp);
        let r = &run.runner;
        let reply = PhysicsStep {
            drives: Vec::new(),
            writes: out
                .writes
                .iter()
                .map(|(key, v)| match v {
                    WriteVal::Bool(b) => PhysicsWrite { key: key.clone(), value: if *b { 1.0 } else { 0.0 }, bool: true },
                    WriteVal::Real(x) => PhysicsWrite { key: key.clone(), value: *x, bool: false },
                })
                .collect(),
            message: r.message.clone(),
            blocked: r.out.blocked.to_vec(),
            hits: r.hits,
            peak_impact_n: r.peak_impact,
            on_belts: r.line_out.on_belts,
            fallen: r.line_out.fallen,
            released: r.released,
            levels: r.plant_out.pct.iter().cloned().collect(),
        };
        (out, reply)
    };
    // the joints are driven with the lock released: each drive is a command of its own
    let mut drives = Vec::new();
    for (joint, value) in out.drives {
        let applied = apply && live.contains(&joint) && s.execute("joint.drive", &json!({"joint": joint, "value": value, "quiet": true})).is_ok();
        drives.push(PhysicsDrive { joint, value, applied });
    }
    let reply = PhysicsStep { drives, ..reply };
    serde_json::to_value(reply).map_err(|e| bad(cmd, e.to_string()))
}

fn reset(s: &mut Session, _: &Value) -> Result<Value> {
    let mut slot = lock(&s.physics_run);
    let Some(run) = slot.as_mut() else { return Err(bad("physics.reset", "no physics run: physics.load starts one")) };
    // a new stamp builds the world, the cartons and the vessels again from the design
    run.stamp += 1;
    run.runner = Runner::new();
    Ok(json!({"reset": true}))
}

#[cfg(test)]
mod tests;
