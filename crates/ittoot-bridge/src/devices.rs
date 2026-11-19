//! The devices of a machine in the model: motors, limit sensors, station sensors and peripherals (cameras,
//! printers, scanners), each its own body with its own identity colour, and the colour it shows for its
//! live state so the user can see at a glance what the machine is doing.
//!
//! State colours (one meaning each; red here means stopped or at a limit, never "running"):
//!   motor / axis   red = off or at a limit, green = moving, blue = enabled and standing
//!   limit sensor   white = clear, red = active (the axis is at that limit)
//!   station sensor white = away, green = at position
//!   peripheral     white = idle, green = busy or active, red = fault; its own colour while no signal is bound
//! A state comes from the PLC signal when the signal is found and read, and from the axis positions when it
//! is not (the simulated machine, or a PLC that does not publish the bit), so the picture always works.

use crate::driver::Value;
use crate::infer::{words, SensorPoint, StationPoint};
use crate::model::{Body, ModelPlan, Part, GIRDER_WIDTH, STATION_DEFAULT_WIDTH, STATION_HEIGHT};
use serde_json::{json, Value as Json};
use std::collections::HashMap;

pub const MOTOR: [u8; 3] = [245, 124, 0];
pub const CAMERA: [u8; 3] = [214, 51, 200];
pub const PRINTER: [u8; 3] = [123, 77, 224];
pub const SCANNER: [u8; 3] = [0, 172, 193];
pub const SENSOR: [u8; 3] = [250, 250, 250];
pub const OFF_RED: [u8; 3] = [226, 52, 52];
pub const MOVING_GREEN: [u8; 3] = [46, 204, 64];
pub const READY_BLUE: [u8; 3] = [64, 140, 255];
/// The structure of the machine: quiet greys so every device stands out.
pub const RAIL: [u8; 3] = [150, 160, 172];
pub const COLUMN: [u8; 3] = [120, 130, 142];
pub const PAD: [u8; 3] = [176, 184, 192];
/// The transport itself, standing: yellow and orange (it turns green moving, red off or at a limit).
pub const BRIDGE_YELLOW: [u8; 3] = [255, 196, 0];
pub const TROLLEY_ORANGE: [u8; 3] = [255, 140, 0];
pub const HOOK_YELLOW: [u8; 3] = [255, 214, 51];

const SENSOR_SIZE: f64 = 140.0;
const MOTOR_SIZE: f64 = 320.0;
const PERIPHERAL_SIZE: [f64; 3] = [400.0, 400.0, 500.0];
/// Speeds the simulated machine is shown moving at, in mm/s. Assumed; the PLC's own speed is used when it is read.
pub const MOVING_THRESHOLD: f64 = 1.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Motor,
    Limit,
    Station,
    Camera,
    Printer,
    Scanner,
}

impl Kind {
    pub fn label(self) -> &'static str {
        match self {
            Kind::Motor => "motor",
            Kind::Limit => "limit sensor",
            Kind::Station => "station sensor",
            Kind::Camera => "camera",
            Kind::Printer => "printer",
            Kind::Scanner => "scanner",
        }
    }

    pub fn identity(self) -> [u8; 3] {
        match self {
            Kind::Motor => MOTOR,
            Kind::Limit | Kind::Station => SENSOR,
            Kind::Camera => CAMERA,
            Kind::Printer => PRINTER,
            Kind::Scanner => SCANNER,
        }
    }
}

/// Which limit of an axis a sensor is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Limit {
    LowLow,
    Low,
    High,
    HighHigh,
}

impl Limit {
    fn of(name: &str, high_end: bool) -> Limit {
        let w = words(name);
        let twice = |a: &str| w.iter().filter(|x| x.as_str() == a).count() >= 2;
        let outer = twice("low") || twice("high") || twice("min") || twice("max") || w.iter().any(|x| matches!(x.as_str(), "ll" | "hh" | "end"));
        match (high_end, outer) {
            (false, true) => Limit::LowLow,
            (false, false) => Limit::Low,
            (true, false) => Limit::High,
            (true, true) => Limit::HighHigh,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Limit::LowLow => "low low",
            Limit::Low => "low",
            Limit::High => "high",
            Limit::HighHigh => "high high",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Device {
    pub name: String,
    pub kind: Kind,
    /// The part whose bodies carry it (so it moves with that part).
    pub part: String,
    pub body: String,
    /// The axis it belongs to (motors and limit sensors).
    pub axis: Option<i64>,
    pub limit: Option<Limit>,
    /// Where a limit sensor trips along its axis, in mm.
    pub trigger: Option<f64>,
    /// A station sensor: where the station is (x, y) and how wide.
    pub station: Option<(f64, f64, f64)>,
    /// A peripheral's state and fault signals.
    pub key: Option<String>,
    pub fault_key: Option<String>,
}

/// What a device shows now.
#[derive(Clone, Debug, PartialEq)]
pub struct Shade {
    pub name: String,
    pub kind: Kind,
    pub rgb: [u8; 3],
    pub state: String,
    /// A number to read: the position, the speed.
    pub number: String,
}

pub fn hex(c: [u8; 3]) -> String {
    format!("#{:02x}{:02x}{:02x}", c[0], c[1], c[2])
}

/// Where an axis rests in the simulated machine: clear of its low limit sensors.
pub fn rest_position(plan: &ModelPlan, axis: i64) -> f64 {
    plan.devices
        .iter()
        .filter(|d| d.kind == Kind::Limit && d.axis == Some(axis) && matches!(d.limit, Some(Limit::Low | Limit::LowLow)))
        .filter_map(|d| d.trigger)
        .fold(0.0_f64, f64::max)
        + if plan.devices.iter().any(|d| d.kind == Kind::Limit && d.axis == Some(axis)) { crate::demo::HOME_CLEARANCE } else { 0.0 }
}

/// The first body of a part, if any.
fn first_body(plan: &ModelPlan, part: &str) -> Option<Body> {
    plan.parts.iter().find(|p| p.name == part).and_then(|p| p.bodies.first().cloned())
}

/// Add the device bodies to the plan's parts and list the devices. Bodies are placed from the parts that are there (so the
/// same code serves the template and a design's rail); a device's logic stays in the PLC's own units, only its place in the
/// model goes through the joint's map. The structure gets its quiet colours in `commands`.
pub fn add_devices(plan: &mut ModelPlan, sensors: &[SensorPoint], stations: &[StationPoint], peripheral_keys: &[&str]) {
    let Some(jx) = plan.joints.iter().find(|j| j.letter == 'X').cloned() else { return };
    let jmap = |letter: char, v: f64| plan.joints.iter().find(|j| j.letter == letter).map(|j| j.to_joint(v)).unwrap_or(v);
    let trolley_bottom = plan.joints.iter().find(|j| j.letter == 'Z').map(|j| j.origin[2]);
    let part_of = |letter: char| plan.joints.iter().find(|j| j.letter == letter).map(|j| j.b.clone());
    let (bridge, trolley) = (part_of('X'), part_of('Y'));
    let rail_s = plan.parts.iter().find(|p| p.name == jx.a).and_then(|p| p.bodies.first().cloned());
    let rail_n = plan.parts.iter().find(|p| p.name == jx.a).and_then(|p| p.bodies.get(1).cloned());
    let bridge_body = bridge.as_deref().and_then(|b| first_body(plan, b));
    let trolley_body = trolley.as_deref().and_then(|t| first_body(plan, t));
    let rail_part = jx.a.clone();
    let is_design = plan.raw.is_some();
    let mut devices: Vec<Device> = Vec::new();
    let mut extra: Vec<(String, Body)> = Vec::new(); // (part, body)
    let mut put = |part: &str, name: &str, corner: [f64; 3], size: [f64; 3]| {
        extra.push((part.to_string(), Body { name: name.to_string(), corner, size }));
    };
    let dev = |name: &str, kind: Kind, part: &str, axis: Option<i64>| Device { name: name.into(), kind, part: part.into(), body: name.into(), axis, limit: None, trigger: None, station: None, key: None, fault_key: None };

    // motors: X at the south end of the bridge, Y and Z beside the trolley
    let m = MOTOR_SIZE;
    if let (Some(b), Some(bb)) = (&bridge, &bridge_body) {
        put(b, "Motor X", [bb.corner[0] + bb.size[0] / 2.0 - m / 2.0, bb.corner[1] - m - 40.0, bb.corner[2] + bb.size[2] / 2.0 - m / 2.0], [m, m, m]);
        devices.push(dev("Motor X", Kind::Motor, b, Some(0)));
    }
    if let (Some(t), Some(tb)) = (&trolley, &trolley_body) {
        put(t, "Motor Y", [tb.corner[0] + tb.size[0], tb.corner[1] + tb.size[1] / 2.0 - m / 2.0, tb.corner[2]], [m, m, m]);
        devices.push(dev("Motor Y", Kind::Motor, t, Some(1)));
        if plan.joints.iter().any(|j| j.letter == 'Z') {
            put(t, "Motor Z", [tb.corner[0] - m, tb.corner[1] + tb.size[1] / 2.0 - m / 2.0, tb.corner[2]], [m, m, m]);
            devices.push(dev("Motor Z", Kind::Motor, t, Some(2)));
        }
    }

    // limit sensors, where each axis' end stop would sit
    let s = SENSOR_SIZE;
    for sp in sensors {
        let lim = Limit::of(&sp.name, sp.high_end);
        let name = format!("Limit {} {}", ["X", "Y", "Z"].get(sp.axis as usize).unwrap_or(&"?"), lim.label());
        let (part, corner) = match sp.axis {
            0 => match &rail_s {
                Some(r) => (rail_part.clone(), [jmap('X', sp.pos) - s / 2.0, r.corner[1] - s - 20.0, r.corner[2] + r.size[2] - s]),
                None => continue,
            },
            1 => match (&bridge, &bridge_body) {
                (Some(b), Some(bb)) => (b.clone(), [bb.corner[0] + bb.size[0], jmap('Y', sp.pos) - s / 2.0, bb.corner[2]]),
                _ => continue,
            },
            2 => match (&trolley, &trolley_body, trolley_bottom) {
                (Some(t), Some(tb), Some(tbot)) => (t.clone(), [tb.corner[0] + tb.size[0] / 2.0 - s / 2.0, tb.corner[1] + tb.size[1] + 40.0, tbot - jmap('Z', sp.pos) - s / 2.0]),
                _ => continue,
            },
            _ => continue,
        };
        put(&part, &name, corner, [s, s, s]);
        devices.push(Device { limit: Some(lim), trigger: Some(sp.pos), ..dev(&name, Kind::Limit, &part, Some(sp.axis)) });
    }

    // station sensors: on a design's far (north) girder over each station's place, on the pads of the template
    for (n, st) in stations.iter().enumerate() {
        let Some(y) = st.y else { continue };
        let w = st.width.unwrap_or(STATION_DEFAULT_WIDTH);
        let label = if stations.iter().filter(|o| o.name == st.name).count() == 1 && !st.name.trim().is_empty() { st.name.clone() } else { format!("Station {}", n + 1) };
        let name = format!("At {label}");
        let (part, corner) = if is_design {
            match &rail_n {
                Some(r) => (rail_part.clone(), [jmap('X', st.x) - s / 2.0, r.corner[1] + r.size[1] + 20.0, r.corner[2] + r.size[2] - s]),
                None => continue,
            }
        } else {
            (label.clone(), [st.x - s / 2.0, y - w / 2.0 - s, STATION_HEIGHT])
        };
        put(&part, &name, corner, [s, s, s]);
        devices.push(Device { station: Some((st.x, y, w)), ..dev(&name, Kind::Station, &part, None) });
    }

    // peripherals: any signal group named for a camera, printer or scanner
    let mut seen: Vec<String> = Vec::new();
    let south = rail_s.as_ref().map(|r| r.corner[1]).unwrap_or(-GIRDER_WIDTH);
    let x0 = plan.origin[0];
    for k in peripheral_keys {
        let Some((base, kind)) = peripheral_base(k) else { continue };
        if seen.contains(&base) {
            continue;
        }
        seen.push(base.clone());
        let group: Vec<&&str> = peripheral_keys.iter().filter(|x| x.starts_with(&base)).collect();
        let fault_key = group.iter().find(|x| words(x).iter().any(|w| matches!(w.as_str(), "fault" | "error" | "alarm" | "failed"))).map(|x| x.to_string());
        let key = group
            .iter()
            .find(|x| Some(x.to_string()) != fault_key && words(x).iter().any(|w| matches!(w.as_str(), "busy" | "active" | "running" | "printing" | "capturing" | "scanning" | "triggered" | "ready" | "ok" | "online" | "status" | "done")))
            .map(|x| x.to_string());
        let i = devices.iter().filter(|d| matches!(d.kind, Kind::Camera | Kind::Printer | Kind::Scanner)).count() as f64;
        let short = base.rsplit('.').next().unwrap_or(&base).to_string();
        let name = format!("{} {}", kind.label(), short);
        put("Peripherals", &name, [x0 + i * (PERIPHERAL_SIZE[0] + 300.0), south - 1200.0, 0.0], PERIPHERAL_SIZE);
        devices.push(Device { key, fault_key, ..dev(&name, kind, "Peripherals", None) });
    }

    for (part, body) in extra {
        if let Some(p) = plan.parts.iter_mut().find(|p| p.name == part) {
            p.bodies.push(body);
        } else {
            plan.parts.push(Part { name: part, bodies: vec![body], grounded: true });
        }
    }
    if devices.iter().any(|d| d.kind == Kind::Motor) {
        plan.notes.push(format!("{} devices added: motors, limit sensors, station sensors{}; each has its own colour and shows its state (see the Devices table).", devices.len(), if devices.iter().any(|d| matches!(d.kind, Kind::Camera | Kind::Printer | Kind::Scanner)) { " and peripherals" } else { "; no camera, printer or scanner signals were found in the data" }));
    }
    plan.devices = devices;
}

fn peripheral_base(key: &str) -> Option<(String, Kind)> {
    for seg in key.split('.') {
        let w = words(seg);
        let kind = if w.iter().any(|x| x == "camera" || x == "cam") {
            Kind::Camera
        } else if w.iter().any(|x| x == "printer" || x == "label") {
            Kind::Printer
        } else if w.iter().any(|x| x == "scanner" || x == "barcode" || x == "reader") {
            Kind::Scanner
        } else {
            continue;
        };
        let end = key.find(seg).map(|i| i + seg.len())?;
        return Some((key[..end].to_string(), kind));
    }
    None
}

/// The signal key for a limit sensor or an axis status bit, found by its words in the data's keys.
pub fn find_key(keys: &[String], axis: i64, must: &[&str], lows: usize, highs: usize) -> Option<String> {
    let mut found: Vec<&String> = keys
        .iter()
        .filter(|k| {
            let w = words(k);
            let has = |s: &str| w.iter().any(|x| x == s);
            let count = |s: &str| w.iter().filter(|x| x.as_str() == s).count();
            let by_row = w.windows(2).any(|p| (p[0] == "axes" || p[0] == "axis" || p[0] == "drives") && p[1].parse::<i64>().ok() == Some(axis));
            by_row && must.iter().all(|m| has(m)) && count("low") == lows && count("high") == highs && !["cmd", "hmi", "set", "setpoint", "config", "settings", "use"].iter().any(|b| has(b))
        })
        .collect();
    found.sort();
    found.first().map(|k| (*k).clone())
}

fn truthy(v: &Value) -> Option<bool> {
    match v {
        Value::Bool(b) => Some(*b),
        Value::Int(i) => Some(*i != 0),
        Value::Real(r) => Some(*r != 0.0),
        Value::Text(_) => None,
    }
}

fn number(v: &Value) -> Option<f64> {
    match v {
        Value::Real(r) => Some(*r),
        Value::Int(i) => Some(*i as f64),
        _ => None,
    }
}

/// What every joint is at (by axis index) and how fast it is moving (mm/s), the inputs of a state.
pub struct Motion<'a> {
    pub position: &'a HashMap<i64, f64>,
    pub speed: &'a HashMap<i64, f64>,
    pub live: &'a HashMap<String, Value>,
    pub keys: &'a [String],
}

impl Motion<'_> {
    fn flag(&self, axis: i64, must: &[&str], lows: usize, highs: usize) -> Option<bool> {
        let k = find_key(self.keys, axis, must, lows, highs)?;
        truthy(self.live.get(&k)?)
    }
}

/// The shade of every device now.
pub fn shades(plan: &ModelPlan, m: &Motion) -> Vec<Shade> {
    let letter = |a: i64| ["X", "Y", "Z"].get(a as usize).copied().unwrap_or("?");
    let at = |a: i64| m.position.get(&a).copied().unwrap_or(0.0);
    let limit_hit = |axis: i64| -> Option<Limit> {
        for d in plan.devices.iter().filter(|d| d.kind == Kind::Limit && d.axis == Some(axis)) {
            let Some(l) = d.limit else { continue };
            let (lows, highs) = match l {
                Limit::LowLow => (2, 0),
                Limit::Low => (1, 0),
                Limit::High => (0, 1),
                Limit::HighHigh => (0, 2),
            };
            let active = m.flag(axis, &["limit"], lows, highs).unwrap_or_else(|| match (l, d.trigger) {
                (Limit::LowLow | Limit::Low, Some(t)) => at(axis) <= t,
                (Limit::High | Limit::HighHigh, Some(t)) => at(axis) >= t,
                _ => false,
            });
            if active {
                return Some(l);
            }
        }
        None
    };
    plan.devices
        .iter()
        .map(|d| {
            let (rgb, state, number) = match d.kind {
                Kind::Motor => {
                    let a = d.axis.unwrap_or(0);
                    let speed = m.speed.get(&a).copied().unwrap_or(0.0);
                    // the PLC's bit or the measured speed: a simulated drive may never set its own `moving` bit
                    let moving = m.flag(a, &["moving"], 0, 0).unwrap_or(false) || speed.abs() > MOVING_THRESHOLD;
                    let enabled = m.flag(a, &["enabled"], 0, 0).unwrap_or(true);
                    let num = format!("{} = {:.0} mm, {:.0} mm/s", letter(a), at(a), speed.abs());
                    match limit_hit(a) {
                        Some(l) => (OFF_RED, format!("AT {} LIMIT", l.label().to_uppercase()), num),
                        None if !enabled => (OFF_RED, "off".into(), num),
                        None if moving => (MOVING_GREEN, "moving".into(), num),
                        None => (READY_BLUE, "ready".into(), num),
                    }
                }
                Kind::Limit => {
                    let a = d.axis.unwrap_or(0);
                    let active = limit_hit(a).is_some_and(|l| Some(l) == d.limit) || {
                        // a lower limit can be active while a higher-priority one reports first: ask this one directly
                        d.limit.is_some_and(|l| {
                            let (lows, highs) = match l {
                                Limit::LowLow => (2, 0),
                                Limit::Low => (1, 0),
                                Limit::High => (0, 1),
                                Limit::HighHigh => (0, 2),
                            };
                            m.flag(a, &["limit"], lows, highs).unwrap_or(false)
                        })
                    };
                    let num = format!("trips at {:.0} mm", d.trigger.unwrap_or(0.0));
                    if active { (OFF_RED, "ACTIVE".into(), num) } else { (SENSOR, "clear".into(), num) }
                }
                Kind::Station => {
                    let (sx, sy, w) = d.station.unwrap_or((0.0, 0.0, 0.0));
                    let on = (at(0) - sx).abs() <= w / 2.0 && (at(1) - sy).abs() <= w / 2.0;
                    let num = format!("{:.0} mm from it", ((at(0) - sx).powi(2) + (at(1) - sy).powi(2)).sqrt());
                    if on { (MOVING_GREEN, "at position".into(), num) } else { (SENSOR, "away".into(), num) }
                }
                Kind::Camera | Kind::Printer | Kind::Scanner => {
                    let fault = d.fault_key.as_ref().and_then(|k| m.live.get(k)).and_then(truthy).unwrap_or(false);
                    let state = d.key.as_ref().and_then(|k| m.live.get(k));
                    let num = state.and_then(number).map(|n| format!("{n:.0}")).unwrap_or_default();
                    match (fault, state.and_then(truthy)) {
                        (true, _) => (OFF_RED, "FAULT".into(), num),
                        (false, Some(true)) => (MOVING_GREEN, "active".into(), num),
                        (false, Some(false)) => (SENSOR, "idle".into(), num),
                        (false, None) => (d.kind.identity(), "no signal".into(), num),
                    }
                }
            };
            Shade { name: d.name.clone(), kind: d.kind, rgb, state, number }
        })
        .collect()
}

/// The colour of each moving part: its axis' motor shade, so "the hoist axis" is red off, green moving.
pub fn part_shade(plan: &ModelPlan, shades: &[Shade], part: &str) -> Option<[u8; 3]> {
    let axis = plan.joints.iter().position(|j| j.b == part)? as i64;
    let motor = plan.devices.iter().find(|d| d.kind == Kind::Motor && d.axis == Some(axis))?;
    let c = shades.iter().find(|s| s.name == motor.name).map(|s| s.rgb).unwrap_or(READY_BLUE);
    // standing (blue "ready") the transport shows its own colours; the moving and fault colours are the motor's
    Some(if c == READY_BLUE { [BRIDGE_YELLOW, TROLLEY_ORANGE, HOOK_YELLOW][(axis as usize).min(2)] } else { c })
}

/// The colour of every body of the plan: structure quiet, moving parts in their motor's shade, devices in theirs.
pub fn body_colours(plan: &ModelPlan, shades: &[Shade]) -> Vec<(String, [u8; 3])> {
    let mut out = Vec::new();
    for p in &plan.parts {
        let moving = part_shade(plan, shades, &p.name);
        for b in &p.bodies {
            let c = if let Some(d) = plan.devices.iter().find(|d| d.body == b.name) {
                shades.iter().find(|s| s.name == d.name).map(|s| s.rgb).unwrap_or_else(|| d.kind.identity())
            } else if let Some(c) = moving {
                c
            } else if plan.raw.is_some() {
                continue; // a design's own palette (pipes, trays, panels...) stays as its script set it
            } else if p.name == "Rail" {
                RAIL
            } else if p.grounded {
                PAD
            } else {
                COLUMN
            };
            out.push((b.name.clone(), c));
        }
    }
    out
}

/// `appearance.assign` commands for the structure's quiet colours and every device's shade, one command per colour.
pub fn colour_commands(plan: &ModelPlan, shades: &[Shade]) -> Vec<Json> {
    let mut by: Vec<([u8; 3], Vec<String>)> = Vec::new();
    for (body, c) in body_colours(plan, shades) {
        match by.iter_mut().find(|(k, _)| *k == c) {
            Some((_, v)) => v.push(body),
            None => by.push((c, vec![body])),
        }
    }
    by.into_iter().map(|(c, bodies)| json!({"command":"appearance.assign","params":{"bodies":bodies,"color":hex(c)}})).collect()
}

/// The signals worth reading for the devices: status bits of each axis, limit bits and the peripherals' signals.
pub fn device_keys(plan: &ModelPlan, keys: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for a in 0..3 {
        for must in [&["moving"][..], &["enabled"][..]] {
            out.extend(find_key(keys, a, must, 0, 0));
        }
        for (l, h) in [(2, 0), (1, 0), (0, 1), (0, 2)] {
            out.extend(find_key(keys, a, &["limit"], l, h));
        }
    }
    for d in &plan.devices {
        out.extend(d.key.clone());
        out.extend(d.fault_key.clone());
    }
    out.sort();
    out.dedup();
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infer::{infer, sensor_points, station_points, Record};
    use crate::model::plan;
    use std::collections::BTreeMap;

    fn rec(array: &str, index: usize, f: &[(&str, &str)]) -> Record {
        Record { db: "D".into(), array: array.into(), index, fields: f.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect() }
    }

    fn machine() -> ModelPlan {
        let mut recs = Vec::new();
        for (axis, hi) in [(0, 11900.0), (1, 5900.0), (2, 2900.0)] {
            for (i, (n, p)) in [("LIM_LOW_LOW", 20.0), ("LIM_HIGH_HIGH", hi + 80.0)].into_iter().enumerate() {
                recs.push(rec("rows", axis * 2 + i, &[("axis", &axis.to_string()), ("sensor", &format!("'{n}'")), ("triggerPosition", &p.to_string())]));
            }
        }
        recs.push(rec("stations", 0, &[("name", "'Station 1'"), ("x", "2000.0"), ("y", "3000.0"), ("zUp", "150.0"), ("zDown", "2000.0"), ("width", "300.0")]));
        let st = station_points(&recs);
        let inf = infer(&recs, &BTreeMap::new());
        let mut p = plan(&inf, &st, &[("Gantry".to_string(), vec!['X', 'Y']), ("Hoist".to_string(), vec!['Z'])]).unwrap();
        add_devices(&mut p, &sensor_points(&recs), &st, &["P.camera_01.busy", "P.camera_01.fault", "P.printer_01.printing", "P.printer_01.error"]);
        p
    }

    fn motion<'a>(pos: &'a HashMap<i64, f64>, speed: &'a HashMap<i64, f64>, live: &'a HashMap<String, Value>, keys: &'a [String]) -> Motion<'a> {
        Motion { position: pos, speed, live, keys }
    }

    #[test]
    fn devices_are_added_with_their_own_bodies() {
        let p = machine();
        let kinds = |k: Kind| p.devices.iter().filter(|d| d.kind == k).count();
        assert_eq!(kinds(Kind::Motor), 3);
        assert_eq!(kinds(Kind::Limit), 6);
        assert_eq!(kinds(Kind::Station), 1);
        assert_eq!(kinds(Kind::Camera), 1);
        assert_eq!(kinds(Kind::Printer), 1);
        for d in &p.devices {
            assert!(p.parts.iter().any(|x| x.name == d.part && x.bodies.iter().any(|b| b.name == d.body)), "{} has a body", d.name);
        }
        // the Z motor rides on the trolley, an X limit sits on the fixed rail
        assert_eq!(p.devices.iter().find(|d| d.name == "Motor Z").unwrap().part, "Gantry trolley");
        assert_eq!(p.devices.iter().find(|d| d.kind == Kind::Limit && d.axis == Some(0)).unwrap().part, "Rail");
    }

    #[test]
    fn axis_is_red_off_green_moving_and_red_again_at_a_limit() {
        let p = machine();
        let (live, keys) = (HashMap::new(), Vec::<String>::new());
        let mut pos: HashMap<i64, f64> = [(0, 3000.0), (1, 0.0), (2, 0.0)].into();
        let mut speed: HashMap<i64, f64> = HashMap::new();
        let s = shades(&p, &motion(&pos, &speed, &live, &keys));
        let mz = s.iter().find(|x| x.name == "Motor X").unwrap();
        assert_eq!((mz.rgb, mz.state.as_str()), (READY_BLUE, "ready"));
        speed.insert(0, 400.0);
        let s = shades(&p, &motion(&pos, &speed, &live, &keys));
        assert_eq!(s.iter().find(|x| x.name == "Motor X").unwrap().rgb, MOVING_GREEN);
        // into the high-high limit: the motor and that sensor go red, the other sensors stay white
        pos.insert(0, 11990.0);
        let s = shades(&p, &motion(&pos, &speed, &live, &keys));
        let m = s.iter().find(|x| x.name == "Motor X").unwrap();
        assert_eq!(m.rgb, OFF_RED);
        assert!(m.state.contains("LIMIT"), "{}", m.state);
        assert_eq!(s.iter().find(|x| x.name == "Limit X high high").unwrap().rgb, OFF_RED);
        assert_eq!(s.iter().find(|x| x.name == "Limit X low low").unwrap().rgb, SENSOR);
        // a PLC whose moving bit stays FALSE while the axis travels still shows green
        let k2 = vec!["T.axes[0].drive.status.moving".to_string()];
        let l2: HashMap<String, Value> = [(k2[0].clone(), Value::Bool(false))].into();
        pos.insert(0, 3000.0);
        let s = shades(&p, &motion(&pos, &speed, &l2, &k2));
        assert_eq!(s.iter().find(|x| x.name == "Motor X").unwrap().rgb, MOVING_GREEN);
        // the PLC's own bits win over the positions: enabled false = off
        let keys = vec!["T.axes[0].drive.status.enabled".to_string()];
        let live: HashMap<String, Value> = [(keys[0].clone(), Value::Bool(false))].into();
        pos.insert(0, 3000.0);
        let s = shades(&p, &motion(&pos, &speed, &live, &keys));
        let m = s.iter().find(|x| x.name == "Motor X").unwrap();
        assert_eq!((m.rgb, m.state.as_str()), (OFF_RED, "off"));
    }

    #[test]
    fn station_sensor_is_white_away_green_at_position_with_a_number() {
        let p = machine();
        let (live, keys, speed) = (HashMap::new(), Vec::<String>::new(), HashMap::new());
        let mut pos: HashMap<i64, f64> = [(0, 0.0), (1, 0.0), (2, 0.0)].into();
        let s = shades(&p, &motion(&pos, &speed, &live, &keys));
        assert_eq!(s.iter().find(|x| x.kind == Kind::Station).unwrap().rgb, SENSOR);
        pos.insert(0, 2050.0);
        pos.insert(1, 3000.0);
        let s = shades(&p, &motion(&pos, &speed, &live, &keys));
        let st = s.iter().find(|x| x.kind == Kind::Station).unwrap();
        assert_eq!((st.rgb, st.state.as_str(), st.number.as_str()), (MOVING_GREEN, "at position", "50 mm from it"));
    }

    #[test]
    fn peripherals_use_their_signals_and_their_own_colour_until_then() {
        let p = machine();
        let (speed, pos) = (HashMap::new(), HashMap::from([(0, 500.0), (1, 500.0), (2, 500.0)]));
        let s = shades(&p, &motion(&pos, &speed, &HashMap::new(), &[]));
        assert_eq!(s.iter().find(|x| x.kind == Kind::Camera).unwrap().rgb, CAMERA);
        assert_eq!(s.iter().find(|x| x.kind == Kind::Printer).unwrap().rgb, PRINTER);
        let live: HashMap<String, Value> = [("P.camera_01.busy".to_string(), Value::Bool(true)), ("P.printer_01.error".to_string(), Value::Bool(true)), ("P.printer_01.printing".to_string(), Value::Bool(false))].into();
        let s = shades(&p, &motion(&pos, &speed, &live, &[]));
        assert_eq!(s.iter().find(|x| x.kind == Kind::Camera).unwrap().rgb, MOVING_GREEN);
        assert_eq!(s.iter().find(|x| x.kind == Kind::Printer).unwrap().rgb, OFF_RED);
    }

    #[test]
    fn colour_commands_colour_every_body_once_and_moving_parts_follow_their_motor() {
        let p = machine();
        let (live, keys, pos) = (HashMap::new(), Vec::<String>::new(), HashMap::from([(0, 500.0), (1, 500.0), (2, 500.0)]));
        let speed: HashMap<i64, f64> = [(2, 250.0)].into();
        let s = shades(&p, &motion(&pos, &speed, &live, &keys));
        let cmds = colour_commands(&p, &s);
        let mut all: Vec<&str> = cmds.iter().flat_map(|c| c["params"]["bodies"].as_array().unwrap().iter().map(|b| b.as_str().unwrap())).collect();
        let n = all.len();
        all.sort();
        all.dedup();
        assert_eq!(all.len(), n, "no body coloured twice");
        assert_eq!(n, p.parts.iter().map(|x| x.bodies.len()).sum::<usize>(), "every body is coloured");
        let green = cmds.iter().find(|c| c["params"]["color"] == hex(MOVING_GREEN)).unwrap();
        let bodies: Vec<&str> = green["params"]["bodies"].as_array().unwrap().iter().map(|b| b.as_str().unwrap()).collect();
        assert!(bodies.contains(&"Hoist hook body") && bodies.contains(&"Motor Z"), "{bodies:?}");
        assert!(!bodies.contains(&"Gantry bridge beam"));
    }

    #[test]
    fn plc_limit_bit_is_found_by_its_words() {
        let keys: Vec<String> = ["T.axes[0].limit.lowLow", "T.axes[0].limit.low", "T.axes[0].limit.high", "T.axes[0].limit.highHigh", "T.axes[1].limit.high"].iter().map(|s| s.to_string()).collect();
        assert_eq!(find_key(&keys, 0, &["limit"], 2, 0).as_deref(), Some("T.axes[0].limit.lowLow"));
        assert_eq!(find_key(&keys, 0, &["limit"], 0, 2).as_deref(), Some("T.axes[0].limit.highHigh"));
        assert_eq!(find_key(&keys, 0, &["limit"], 0, 1).as_deref(), Some("T.axes[0].limit.high"));
        assert_eq!(find_key(&keys, 1, &["limit"], 0, 1).as_deref(), Some("T.axes[1].limit.high"));
    }
}
