//! From what the bridge worked out to a model: the parts and joints of the machine, the commands that
//! build them in Solidcraft, and the bindings that tie a PLC signal to a joint so the model moves
//! when the machine does.
//!
//! The first template is a gantry with a hoist: a bridge that runs along X, a trolley that runs along
//! the bridge (Y) and a hook that hangs from the trolley (Z, down). Sizes come from the inference;
//! the sections of the parts that nothing in the code or documents says (girder depth and so on) are
//! assumed, named below, and the plan says so.

use crate::driver::Value;
use crate::infer::{words, Inference, StationPoint};
use serde_json::{json, Value as Json};
use std::collections::{BTreeMap, HashMap};

/// Sections nothing in the PLC data gives; assumed, in mm.
pub const GIRDER_WIDTH: f64 = 250.0;
pub const GIRDER_DEPTH: f64 = 300.0;
/// How far the girders run past the end of the bridge's travel.
pub const GIRDER_OVERRUN: f64 = 400.0;
pub const BRIDGE_WIDTH: f64 = 300.0;
pub const BRIDGE_DEPTH: f64 = 200.0;
pub const TROLLEY_SIZE: [f64; 3] = [400.0, 400.0, 200.0];
pub const HOOK_SIZE: [f64; 3] = [100.0, 100.0, 300.0];
/// Free height under the hook at the bottom of its travel.
pub const FLOOR_GAP: f64 = 100.0;
pub const STATION_HEIGHT: f64 = 100.0;
/// A station with no width in the data.
pub const STATION_DEFAULT_WIDTH: f64 = 300.0;

#[derive(Clone, Debug, PartialEq)]
pub struct Body {
    pub name: String,
    pub corner: [f64; 3],
    pub size: [f64; 3],
}

#[derive(Clone, Debug, PartialEq)]
pub struct Part {
    pub name: String,
    pub bodies: Vec<Body>,
    pub grounded: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct JointPlan {
    /// `Gantry X`
    pub name: String,
    pub letter: char,
    /// The axis number in the PLC's axis rows.
    pub axis_index: i64,
    pub a: String,
    pub b: String,
    pub origin: [f64; 3],
    pub axis: [f64; 3],
    /// The travel in the PLC's units (what the sliders and the PLC's positions run over).
    pub limits: (f64, f64),
    /// The travel of the model's joint, when the model is not the PLC's own scale (a design's rail).
    pub drive_limits: (f64, f64),
    /// PLC position -> joint position, piecewise linear through these points (empty: the same).
    pub map: Vec<(f64, f64)>,
}

impl JointPlan {
    /// The joint's value for a PLC position.
    pub fn to_joint(&self, plc: f64) -> f64 {
        map_through(&self.map, plc).clamp(self.drive_limits.0, self.drive_limits.1)
    }
}

/// Piecewise-linear through `pts` (sorted by the first number), continuing at the end slopes; the same when there are none.
pub fn map_through(pts: &[(f64, f64)], v: f64) -> f64 {
    match pts {
        [] => v,
        [(a, b)] => v - a + b,
        _ => {
            let seg = pts.windows(2).find(|w| v <= w[1].0).unwrap_or(&pts[pts.len() - 2..]);
            let (p, q) = (seg[0], seg[1]);
            if (q.0 - p.0).abs() < 1e-9 { p.1 } else { p.1 + (v - p.0) * (q.1 - p.1) / (q.0 - p.0) }
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ModelPlan {
    /// Where the model's bounding box starts (X, Y) in mm, for drawing.
    pub origin: [f64; 3],
    /// A design's own build script (rail, baths, pipes...), sent instead of the template's commands.
    pub raw: Option<Vec<Json>>,
    pub parts: Vec<Part>,
    pub joints: Vec<JointPlan>,
    /// What was assumed or left out, for the user to read.
    pub notes: Vec<String>,
    /// Overall size X, Y, Z in mm.
    pub envelope: [f64; 3],
    /// Motors, sensors and peripherals added by `devices::add_devices`.
    pub devices: Vec<crate::devices::Device>,
}

fn role_for(roles: &[(String, Vec<char>)], letter: char) -> Option<&str> {
    roles.iter().find(|(_, ls)| ls.contains(&letter)).map(|(r, _)| r.as_str())
}

/// The plan for a gantry with a hoist. `None` when the data gives no X and Y lengths.
pub fn plan(inf: &Inference, stations: &[StationPoint], roles: &[(String, Vec<char>)]) -> Option<ModelPlan> {
    let len = |i: i64| inf.get(&format!("axis{i}.length")).map(|v| v.value).filter(|v| *v > 0.0);
    let (lx, ly) = (len(0)?, len(1)?);
    let lz = len(2);
    let mut notes = vec![format!("Gantry-and-hoist template. Axis 0 is X ({lx:.0} mm), axis 1 is Y ({ly:.0} mm){}.", lz.map(|z| format!(", axis 2 is Z down ({z:.0} mm)")).unwrap_or_default())];
    notes.push(format!("Assumed (nothing in the code or documents says): girder {GIRDER_WIDTH}x{GIRDER_DEPTH} mm, bridge {BRIDGE_WIDTH}x{BRIDGE_DEPTH} mm, trolley {:?} mm, hook {:?} mm.", TROLLEY_SIZE, HOOK_SIZE));

    let lz_v = lz.unwrap_or(0.0);
    let h = lz_v + HOOK_SIZE[2] + TROLLEY_SIZE[2] + BRIDGE_DEPTH + FLOOR_GAP; // height of the girders' underside
    let name_of = |letter: char, what: &str, fallback: &str| match role_for(roles, letter) {
        Some(r) => format!("{r} {what}"),
        None => fallback.to_string(),
    };
    let (bridge, trolley, hook) = (name_of('X', "bridge", "Bridge"), name_of('Y', "trolley", "Trolley"), name_of('Z', "hook", "Hook"));

    let girder = |n: &str, y: f64| Body { name: n.into(), corner: [-GIRDER_OVERRUN, y, h], size: [lx + 2.0 * GIRDER_OVERRUN, GIRDER_WIDTH, GIRDER_DEPTH] };
    let mut parts = vec![
        Part { name: "Rail".into(), bodies: vec![girder("Rail south", -GIRDER_WIDTH), girder("Rail north", ly)], grounded: true },
        Part {
            name: bridge.clone(),
            bodies: vec![Body { name: format!("{bridge} beam"), corner: [-BRIDGE_WIDTH / 2.0, -GIRDER_WIDTH, h - BRIDGE_DEPTH], size: [BRIDGE_WIDTH, ly + 2.0 * GIRDER_WIDTH, BRIDGE_DEPTH] }],
            grounded: false,
        },
        Part {
            name: trolley.clone(),
            bodies: vec![Body {
                name: format!("{trolley} body"),
                corner: [-TROLLEY_SIZE[0] / 2.0, -TROLLEY_SIZE[1] / 2.0, h - BRIDGE_DEPTH - TROLLEY_SIZE[2]],
                size: TROLLEY_SIZE,
            }],
            grounded: false,
        },
    ];
    let trolley_bottom = h - BRIDGE_DEPTH - TROLLEY_SIZE[2];
    let mut joints = vec![
        JointPlan { name: name_of('X', "X", "Axis X"), letter: 'X', axis_index: 0, a: "Rail".into(), b: bridge.clone(), origin: [0.0, 0.0, h], axis: [1.0, 0.0, 0.0], limits: (0.0, lx), drive_limits: (0.0, lx), map: Vec::new() },
        JointPlan { name: name_of('Y', "Y", "Axis Y"), letter: 'Y', axis_index: 1, a: bridge, b: trolley.clone(), origin: [0.0, 0.0, h - BRIDGE_DEPTH], axis: [0.0, 1.0, 0.0], limits: (0.0, ly), drive_limits: (0.0, ly), map: Vec::new() },
    ];
    if let Some(lz) = lz {
        parts.push(Part {
            name: hook.clone(),
            bodies: vec![Body { name: format!("{hook} body"), corner: [-HOOK_SIZE[0] / 2.0, -HOOK_SIZE[1] / 2.0, trolley_bottom - HOOK_SIZE[2]], size: HOOK_SIZE }],
            grounded: false,
        });
        joints.push(JointPlan { name: name_of('Z', "Z", "Axis Z"), letter: 'Z', axis_index: 2, a: trolley, b: hook, origin: [0.0, 0.0, trolley_bottom], axis: [0.0, 0.0, -1.0], limits: (0.0, lz), drive_limits: (0.0, lz), map: Vec::new() });
    } else {
        notes.push("No Z length could be worked out, so there is no hook.".into());
    }
    let mut placed = 0;
    for (n, s) in stations.iter().enumerate() {
        let Some(y) = s.y else { continue };
        let w = s.width.unwrap_or(STATION_DEFAULT_WIDTH);
        let name = if stations.iter().filter(|o| o.name == s.name).count() == 1 && !s.name.trim().is_empty() { s.name.clone() } else { format!("Station {}", n + 1) };
        parts.push(Part { name: name.clone(), bodies: vec![Body { name: format!("{name} pad"), corner: [s.x - w / 2.0, y - w / 2.0, 0.0], size: [w, w, STATION_HEIGHT] }], grounded: true });
        placed += 1;
    }
    if placed > 0 {
        notes.push(format!("{placed} stations placed on the floor from the station table."));
    }
    Some(ModelPlan { origin: [-GIRDER_OVERRUN, -GIRDER_WIDTH, 0.0], raw: None, parts, joints, notes, envelope: [lx + 2.0 * GIRDER_OVERRUN, ly + 2.0 * GIRDER_WIDTH, h + GIRDER_DEPTH], devices: Vec::new() })
}

impl ModelPlan {
    /// The Solidcraft command script that builds the model: bodies, components, grounding, joints.
    pub fn commands(&self) -> Vec<Json> {
        if let Some(raw) = &self.raw {
            let mut c = raw.clone();
            c.extend(crate::devices::colour_commands(self, &[]));
            return c;
        }
        let mut c: Vec<Json> = Vec::new();
        for p in &self.parts {
            for b in &p.bodies {
                c.push(json!({"command":"solid.box","params":{"length":b.size[0],"width":b.size[1],"height":b.size[2],"corner":b.corner,"operation":"new","body_name":b.name}}));
            }
        }
        for p in &self.parts {
            c.push(json!({"command":"component.create","params":{"name":p.name,"parent":"root","activate":false}}));
            let bodies: Vec<&str> = p.bodies.iter().map(|b| b.name.as_str()).collect();
            c.push(json!({"command":"component.move_bodies","params":{"bodies":bodies,"component":p.name}}));
        }
        for p in self.parts.iter().filter(|p| p.grounded) {
            c.push(json!({"command":"occurrence.ground","params":{"occurrence":format!("{}:1", p.name),"grounded":true}}));
        }
        for j in &self.joints {
            c.push(json!({"command":"joint.create","params":{
                "type":"slider","name":j.name,"flip":true,"limits":[[j.limits.0, j.limits.1]],
                "a":{"occurrence":j.a,"point":j.origin,"z":j.axis},
                "b":{"occurrence":j.b,"point":j.origin,"z":j.axis}}}));
        }
        // the quiet structure colours and each device's own colour; the live shades are sent over these
        c.extend(crate::devices::colour_commands(self, &[]));
        c
    }
}

/// One joint driven by one PLC signal: `joint value = scale * signal + offset`.
#[derive(Clone, Debug, PartialEq)]
pub struct ModelBinding {
    pub joint: String,
    pub key: String,
    pub scale: f64,
    pub offset: f64,
    /// Found by the rules, not set by the user.
    pub auto: bool,
}

fn score(key_words: &[String], axis_index: i64, letter: char) -> i32 {
    let has = |w: &str| key_words.iter().any(|x| x == w);
    if !(has("actual") && has("position")) {
        return 0;
    }
    let lower = letter.to_ascii_lowercase().to_string();
    let by_row = key_words.windows(2).any(|p| (p[0] == "axes" || p[0] == "axis" || p[0] == "drives") && p[1].parse::<i64>().ok() == Some(axis_index));
    let by_letter = has(&lower) && (has("drive") || has("control"));
    let mut s = if by_row {
        10
    } else if by_letter {
        8
    } else {
        return 0;
    };
    if has("drive") {
        s += 3;
    }
    if has("status") {
        s += 1;
    }
    if ["cmd", "hmi", "set", "setpoint", "target", "home", "offset", "use"].iter().any(|w| has(w)) {
        s -= 5;
    }
    s
}

/// Pick the PLC signal for each joint: the actual position of its axis. A joint with no clear single
/// best signal gets none, and the page says so.
pub fn auto_bind(plan: &ModelPlan, keys: &[&str]) -> Vec<ModelBinding> {
    let words_of: Vec<(&str, Vec<String>)> = keys.iter().map(|k| (*k, words(k))).collect();
    let mut out = Vec::new();
    for j in &plan.joints {
        let mut scored: Vec<(i32, &str)> = words_of.iter().map(|(k, w)| (score(w, j.axis_index, j.letter), *k)).filter(|(s, _)| *s >= 10).collect();
        scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(b.1)));
        // best score first; among equals the first by name, so the pick is the same every run
        if let Some((_, key)) = scored.first().copied() {
            out.push(ModelBinding { joint: j.name.clone(), key: key.to_string(), scale: 1.0, offset: 0.0, auto: true });
        }
    }
    out
}

/// The bindings in force: what the rules found, then the user's choices over them (a key per joint;
/// an empty key unbinds the joint).
pub fn effective(plan: &ModelPlan, auto: &[ModelBinding], user: &BTreeMap<String, String>) -> Vec<ModelBinding> {
    plan.joints
        .iter()
        .filter_map(|j| match user.get(&j.name) {
            Some(k) if k.trim().is_empty() => None,
            Some(k) => Some(ModelBinding { joint: j.name.clone(), key: k.trim().to_string(), scale: 1.0, offset: 0.0, auto: false }),
            None => auto.iter().find(|b| b.joint == j.name).cloned(),
        })
        .collect()
}

fn number(v: &Value) -> Option<f64> {
    match v {
        Value::Real(r) => Some(*r),
        Value::Int(i) => Some(*i as f64),
        Value::Bool(_) | Value::Text(_) => None,
    }
}

/// `joint.drive` for every bound joint whose signal is present and a number, clamped to the joint's limits.
pub fn drive_commands(plan: &ModelPlan, bindings: &[ModelBinding], values: &HashMap<String, Value>) -> Vec<Json> {
    bindings
        .iter()
        .filter_map(|b| {
            let j = plan.joints.iter().find(|j| j.name == b.joint)?;
            let raw = number(values.get(&b.key)?)?;
            let v = j.to_joint(raw * b.scale + b.offset);
            Some(json!({"command":"joint.drive","params":{"joint":b.joint,"value":v,"quiet":true}}))
        })
        .collect()
}

/// `joint.drive` for a model made from scratch: each free binding whose signal is present and a number
/// (Solidcraft clamps to the joint's own limits).
pub fn free_drive_commands(bindings: &[crate::job::FreeBinding], values: &HashMap<String, Value>) -> Vec<Json> {
    bindings
        .iter()
        .filter(|b| !b.joint.trim().is_empty())
        .filter_map(|b| {
            let raw = number(values.get(&b.key)?)?;
            Some(json!({"command":"joint.drive","params":{"joint":b.joint.trim(),"value":raw * b.scale + b.offset,"quiet":true}}))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infer::{infer, Record};

    fn rec(array: &str, index: usize, f: &[(&str, &str)]) -> Record {
        Record { db: "D".into(), array: array.into(), index, fields: f.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect() }
    }

    fn sample() -> (Inference, Vec<StationPoint>) {
        let mut recs = Vec::new();
        for (axis, hi) in [(0, 11900.0), (1, 5900.0), (2, 2900.0)] {
            for (i, (n, p)) in [("LIM_LOW_LOW", 20.0), ("LIM_HIGH_HIGH", hi + 80.0)].into_iter().enumerate() {
                recs.push(rec("rows", axis * 2 + i, &[("axis", &axis.to_string()), ("sensor", &format!("'{n}'")), ("triggerPosition", &p.to_string())]));
            }
        }
        recs.push(rec("stations", 0, &[("name", "'Station 1'"), ("x", "2000.0"), ("y", "3000.0"), ("zUp", "150.0"), ("zDown", "2000.0"), ("width", "300.0")]));
        let stations = crate::infer::station_points(&recs);
        (infer(&recs, &BTreeMap::new()), stations)
    }

    #[test]
    fn plan_uses_inferred_lengths_and_document_roles() {
        let (inf, st) = sample();
        let roles = vec![("Gantry".to_string(), vec!['X', 'Y']), ("Hoist".to_string(), vec!['Z'])];
        let p = plan(&inf, &st, &roles).unwrap();
        let names: Vec<&str> = p.joints.iter().map(|j| j.name.as_str()).collect();
        assert_eq!(names, ["Gantry X", "Gantry Y", "Hoist Z"]);
        assert_eq!(p.joints[0].limits, (0.0, 12000.0));
        assert_eq!(p.joints[2].axis, [0.0, 0.0, -1.0]);
        assert!(p.parts.iter().any(|x| x.name == "Station 1" && x.grounded));
        // the hook never reaches the floor at full travel
        let hook = p.parts.iter().find(|x| x.name == "Hoist hook").unwrap();
        assert!(hook.bodies[0].corner[2] - 2900.0 - 100.0 > -1e-9, "{:?}", hook.bodies[0]);
        let cmds = p.commands();
        assert_eq!(cmds.iter().filter(|c| c["command"] == "joint.create").count(), 3);
        assert_eq!(cmds.iter().filter(|c| c["command"] == "solid.box").count(), 2 + 1 + 1 + 1 + 1);
        assert!(cmds.iter().any(|c| c["command"] == "appearance.assign"), "the structure is coloured");
        // without roles it still plans, with plain names
        assert_eq!(plan(&inf, &st, &[]).unwrap().joints[0].name, "Axis X");
    }

    #[test]
    fn a_scratch_model_drives_its_own_joints_by_scale_and_offset() {
        let b = vec![
            crate::job::FreeBinding { joint: "Lift".into(), key: "a".into(), scale: 0.5, offset: 10.0 },
            crate::job::FreeBinding { joint: "".into(), key: "a".into(), scale: 1.0, offset: 0.0 },
            crate::job::FreeBinding { joint: "Gone".into(), key: "missing".into(), scale: 1.0, offset: 0.0 },
        ];
        let vals: HashMap<String, Value> = [("a".to_string(), Value::Real(100.0))].into();
        let d = free_drive_commands(&b, &vals);
        assert_eq!(d.len(), 1, "no joint name and no signal give nothing");
        assert_eq!((d[0]["params"]["joint"].as_str(), d[0]["params"]["value"].as_f64()), (Some("Lift"), Some(60.0)));
    }

    #[test]
    fn no_lengths_no_plan() {
        assert!(plan(&Inference::default(), &[], &[]).is_none());
    }

    #[test]
    fn actual_position_signals_are_picked_per_axis() {
        let (inf, st) = sample();
        let p = plan(&inf, &st, &[]).unwrap();
        let keys = [
            "T.axes[0].drive.status.actualPosition",
            "T.axes[0].drive.cmd.position",
            "T.axes[0].control.status.actualVelocity",
            "T.axes[1].drive.status.actualPosition",
            "T.axes[2].drive.status.actualPosition",
            "T.axes[2].control.status.actualPosition",
        ];
        let b = auto_bind(&p, &keys);
        assert_eq!(b.len(), 3);
        assert_eq!(b[0].key, "T.axes[0].drive.status.actualPosition");
        assert_eq!(b[2].key, "T.axes[2].drive.status.actualPosition", "drive beats control");
        let user: BTreeMap<String, String> = [("Axis Y".to_string(), String::new()), ("Axis X".to_string(), "Mine.x".to_string())].into();
        let eff = effective(&p, &b, &user);
        assert_eq!(eff.len(), 2);
        assert_eq!(eff[0].key, "Mine.x");
        assert!(!eff[0].auto);
    }

    #[test]
    fn drive_is_clamped_and_skips_missing_or_non_numbers() {
        let (inf, st) = sample();
        let p = plan(&inf, &st, &[]).unwrap();
        let b = vec![
            ModelBinding { joint: "Axis X".into(), key: "x".into(), scale: 1.0, offset: 0.0, auto: false },
            ModelBinding { joint: "Axis Y".into(), key: "y".into(), scale: 1.0, offset: 0.0, auto: false },
            ModelBinding { joint: "Axis Z".into(), key: "z".into(), scale: 1.0, offset: 0.0, auto: false },
        ];
        let vals: HashMap<String, Value> = [("x".to_string(), Value::Real(99999.0)), ("y".to_string(), Value::Bool(true))].into();
        let d = drive_commands(&p, &b, &vals);
        assert_eq!(d.len(), 1);
        assert_eq!(d[0]["params"]["value"], 12000.0);
    }
}
