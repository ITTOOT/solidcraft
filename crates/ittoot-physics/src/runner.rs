//! The physics step as a whole: a design and the PLC's commanded joint values in, the joint values to drive and the signals to write to the PLC out.
//! The bridge app calls it from its own loop and Solidcraft's engine calls it from the `physics.step` command, so a machine behaves the same either way.
//!
//! Everything it needs from outside goes through [`Inputs`] (time step, commanded joints, the joints the model has, a way to read a signal) and
//! everything it wants done comes back in [`StepOut`]; it touches no PLC and no document itself.

use crate::fluid::{self, Level, Supply};
use crate::loads::{Line, LineOut};
use crate::physics::{Outputs, Physics, World};
use crate::physio;
use crate::plant::{Plant, PlantOut};
use serde_json::Value as Json;
use std::collections::{BTreeMap, HashMap};

/// What one step needs from the outside.
pub struct Inputs<'a> {
    /// Seconds since the last step (the first step gives 0). Anything above half a second is taken as half a second.
    pub dt: f64,
    /// The joints of the gantry's X, Y and Z axes (a hoist design); `None` for a design with no hoist.
    pub axes: Option<[String; 3]>,
    /// The PLC's commanded value for each joint it drives.
    pub commanded: &'a BTreeMap<String, f64>,
    /// The joints the drawn model has; `None` until that is known (nothing is driven that the model might not have).
    pub live_joints: Option<&'a [String]>,
    /// A PLC signal by its key as a number (a bool is 0 or 1); `None` when it is not read.
    pub signal: &'a dyn Fn(&str) -> Option<f64>,
}

/// A value to write to a PLC signal.
#[derive(Clone, Debug, PartialEq)]
pub enum WriteVal {
    Bool(bool),
    Real(f64),
}

/// What one step wants done.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct StepOut {
    /// Joint values to drive (a commanded axis comes back with the physics' value: held where a solid stops it).
    pub drives: Vec<(String, f64)>,
    /// Signals to write to the PLC, only those that changed.
    pub writes: Vec<(String, WriteVal)>,
}

/// The state of one physics run, whatever the design is.
#[derive(Default)]
pub struct Runner {
    pub world: Option<World>,
    pub line: Option<Line>,
    pub line_out: LineOut,
    pub plant: Option<Plant>,
    pub plant_out: PlantOut,
    pub out: Outputs,
    pub levels: Vec<Level>,
    pub supply: Option<Supply>,
    pub message: String,
    pub hits: u32,
    pub peak_impact: f64,
    pub released: usize,
    was_touching: bool,
    built: Option<(u64, u64, String)>,
    line_built: Option<u64>,
    plant_built: Option<u64>,
    sent_level: Vec<f64>,
    sent: HashMap<String, f64>,
    drives_sent: HashMap<String, f64>,
}

/// What the design calls a joint of the transporter whose hoist joint is `z`: "Hoist Z" belongs to the only transporter (no prefix), "Transporter A Z" to
/// transporter A (prefix "Transporter A ").
fn prefix_of(z: &str) -> String {
    if z == "Hoist Z" {
        String::new()
    } else {
        z.strip_suffix('Z').unwrap_or("").to_string()
    }
}

/// The names of the two sway joints that go with the hoist joint `z` ("Hoist Z" gives "Sway Rx", "Transporter A Z" gives "Transporter A Sway Rx").
pub fn sway_names(z: &str) -> (String, String) {
    let prefix = prefix_of(z);
    (format!("{prefix}Sway Rx"), format!("{prefix}Sway Ry"))
}

/// The rope's slack slider for the hoist joint `z` ("Hoist Z" gives "Rope slack", "Transporter A Z" gives "Transporter A rope slack").
pub fn slack_name(z: &str) -> String {
    match prefix_of(z).as_str() {
        "" => "Rope slack".to_string(),
        prefix => format!("{prefix}rope slack"),
    }
}

/// How far a rope sags sideways (mm) when `slack` mm more has been paid out than the hook has gone down, over a rope of `rope` mm: the middle of a
/// loose rope hangs about sqrt(3 x slack x length / 8) off the straight line. It is kept to what the slider allows.
pub fn rope_sag_mm(slack: f64, rope: f64) -> f64 {
    (3.0 * slack.max(0.0) * rope.max(0.0) / 8.0).sqrt().min(350.0)
}

/// "Tank T2" gives "T2".
pub fn tank_id(name: &str) -> String {
    name.strip_prefix("Tank ").unwrap_or(name).to_string()
}

/// How far a valve is open from a signal: 0 or 1, or a percentage above 1.
pub fn opening(v: f64) -> f64 {
    (if v > 1.0 { v / 100.0 } else { v }).clamp(0.0, 1.0)
}

impl Runner {
    pub fn new() -> Runner {
        Runner::default()
    }

    fn open(inp: &Inputs, template: &str, id: &str) -> Option<f64> {
        if template.trim().is_empty() {
            return None;
        }
        (inp.signal)(&physio::key(template, id)).map(opening)
    }

    /// Build the world, cartons or vessels for this design without moving anything, so `input_keys` knows which signals are wanted.
    pub fn prepare(&mut self, s: &Physics, design: &Json, stamp: u64) {
        let none = BTreeMap::new();
        let inp = Inputs { dt: 0.0, axes: None, commanded: &none, live_joints: Some(&[]), signal: &|_| None };
        self.step(s, design, stamp, &inp);
        self.sent.clear();
        self.drives_sent.clear();
        self.message.clear();
    }

    /// The design changed in a way the stamp does not show (a setting that shapes the world): build again at the next step.
    pub fn invalidate(&mut self) {
        self.built = None;
        self.line_built = None;
        self.plant_built = None;
    }

    /// Tell the runner a write did not reach the PLC, so it offers the value again.
    pub fn forget(&mut self, key: &str) {
        self.sent.remove(key);
    }

    /// The PLC signals the physics reads for the design it has built.
    pub fn input_keys(&self, s: &Physics) -> Vec<String> {
        let p = &s.plc;
        let ids: Vec<String> = self.world.as_ref().map(|w| w.liquids.iter().map(|b| tank_id(&b.name)).collect()).unwrap_or_default();
        let mut keys = Vec::new();
        let belts: Vec<String> = self.line.as_ref().map(|l| l.belts.iter().map(|b| b.id.clone()).collect()).unwrap_or_default();
        if !p.zone_run_key.trim().is_empty() {
            keys.extend(belts.iter().map(|id| physio::key(&p.zone_run_key, id)));
        }
        if let Some(pl) = &self.plant {
            if !p.plant_pump_key.trim().is_empty() {
                keys.extend(pl.pumps.iter().map(|q| physio::key(&p.plant_pump_key, &q.tag)));
            }
            if !p.plant_valve_key.trim().is_empty() {
                keys.extend(pl.valves().iter().map(|v| physio::key(&p.plant_valve_key, v)));
            }
        }
        for t in [&p.fill_key, &p.drain_key] {
            if t.trim().is_empty() {
                continue;
            }
            if t.contains("{id}") {
                keys.extend(ids.iter().map(|id| physio::key(t, id)));
            } else {
                keys.push(t.trim().to_string());
            }
        }
        keys
    }

    /// One step. `stamp` changes when the design does, so the world is built again. Returns what to drive and what to write.
    pub fn step(&mut self, s: &Physics, design: &Json, stamp: u64, inp: &Inputs) -> StepOut {
        match design["meta"]["application"].as_str() {
            Some("conveyor_line" | "robot_cell") => self.line_step(s, design, stamp, inp),
            Some("process") => self.plant_step(s, design, stamp, inp),
            _ => self.hoist_step(s, design, stamp, inp),
        }
    }

    fn hoist_step(&mut self, s0: &Physics, design: &Json, stamp: u64, inp: &Inputs) -> StepOut {
        let mut res = StepOut::default();
        let key = (stamp, s0.wall_thickness_mm.to_bits(), s0.solid_parts.clone());
        if self.world.is_none() || self.built.as_ref() != Some(&key) {
            match World::from_design(design, s0) {
                Ok(mut w) => {
                    self.levels = w.tank_levels().into_iter().map(|(mm, max, area)| Level { mm, max_mm: max, area_m2: area }).collect();
                    self.sent_level.clear();
                    w.reset();
                    self.world = Some(w);
                    self.built = Some(key);
                    self.message.clear();
                }
                Err(e) => {
                    self.world = None;
                    self.message = format!("No physics for this design: {e}");
                    return res;
                }
            }
        }
        let Some([xn, yn, zn]) = inp.axes.clone() else { return res };
        let (Some(x), Some(y), Some(z)) = (inp.commanded.get(&xn).copied(), inp.commanded.get(&yn).copied(), inp.commanded.get(&zn).copied()) else { return res };
        let dt = inp.dt.clamp(0.0, 0.5);
        // the physics only moves what Solidcraft's own model has: with no sway joints there it treats the jig as rigid on its hook, and with no rock joints the tanks as bolted down,
        // so what is drawn is what was checked (a swing the picture does not show would let the drawn jig through a wall)
        let live_has = |n: &str| inp.live_joints.is_some_and(|l| l.iter().any(|j| j == n));
        let design_has = |n: &str| design["geometry"]["joints"].as_array().is_some_and(|a| a.iter().any(|j| j["name"] == n));
        let (rx, ry) = sway_names(&zn);
        let sway_ok = design_has(&rx) && design_has(&ry) && live_has(&rx) && live_has(&ry);
        let tank_names: Vec<String> = self.world.as_ref().map(|w| w.tanks.iter().map(|t| t.name.clone()).collect()).unwrap_or_default();
        let rock_ok = !tank_names.is_empty() && tank_names.iter().all(|t| live_has(&format!("{t} Rock Rx")) && live_has(&format!("{t} Rock Ry")));
        let mut s = s0.clone();
        let mut notes: Vec<&str> = Vec::new();
        if !sway_ok {
            s.sway = false;
            notes.push("no sway joints (the jig is treated as rigid on its hook)");
        }
        if !rock_ok {
            s.rock = false;
            notes.push("no tank rock joints (the tanks are treated as bolted down)");
        }
        self.message = if notes.is_empty() { String::new() } else { format!("Solidcraft's model has {}. Rebuild the model from the current design (Model page, Build) to get them.", notes.join(" and ")) };
        let rho = s.liquid_density;
        let mu = s.liquid_viscosity_mpas / 1000.0;
        let supply = fluid::supply(&s.flow, rho, mu);
        self.supply = Some(supply);
        // each tank fills and drains by its own valves: the PLC's signals when they are linked, else the settings
        let ids: Vec<String> = self.world.as_ref().map(|w| w.liquids.iter().map(|b| tank_id(&b.name)).collect()).unwrap_or_default();
        for (i, id) in ids.iter().enumerate() {
            let mut flow = s.flow.clone();
            if let Some(v) = Self::open(inp, &s.plc.fill_key, id) {
                flow.inlet_open = v;
            }
            if let Some(v) = Self::open(inp, &s.plc.drain_key, id) {
                flow.drain_open = v;
            }
            if let Some(l) = self.levels.get_mut(i) {
                l.step(dt, &flow, &supply, s.gravity);
            }
        }
        let Some(w) = self.world.as_mut() else { return res };
        w.apply(&s);
        let mms: Vec<f64> = self.levels.iter().map(|l| l.mm).collect();
        w.set_levels(&mms);
        let out = w.step(dt, [x, y, z]);
        let touching = out.contact.is_some() && out.blocked.iter().any(|b| *b);
        if touching && !self.was_touching {
            self.hits += 1;
        }
        self.was_touching = touching;
        self.peak_impact = self.peak_impact.max(out.impact);
        res.drives.push((xn.clone(), out.carrier[0]));
        res.drives.push((yn.clone(), out.carrier[1]));
        res.drives.push((zn.clone(), out.carrier[2]));
        let has = |n: &str| design_has(n) && live_has(n);
        if has(&rx) {
            res.drives.push((rx.clone(), out.sway_deg[0]));
        }
        if has(&ry) {
            res.drives.push((ry.clone(), out.sway_deg[1]));
        }
        // the tanks on their mounts tilt about X and about Y at the middle of their base
        for (tank, deg) in &out.rock_deg {
            for (suffix, v) in [("Rock Rx", deg[0]), ("Rock Ry", deg[1])] {
                let name = format!("{tank} {suffix}");
                if has(&name) {
                    res.drives.push((name, v));
                }
            }
        }
        // the liquid's surface in each tank
        let level0: Vec<(String, f64)> = self.world.as_ref().map(|w| w.liquids.iter().map(|b| (b.name.clone(), b.level0)).collect()).unwrap_or_default();
        for (i, (name, l0)) in level0.iter().enumerate() {
            let joint = format!("{name} Level");
            let mm = self.levels.get(i).map(|l| l.mm).unwrap_or(*l0);
            if has(&joint) && self.sent_level.get(i).is_none_or(|p| (p - mm).abs() > 0.5) {
                res.drives.push((joint, mm - l0));
                if self.sent_level.len() <= i {
                    self.sent_level.resize(i + 1, f64::NAN);
                }
                self.sent_level[i] = mm;
            }
        }
        // a slack rope hangs in a loop: the drum has paid out more than the hook has gone down
        let slack_joint = slack_name(&zn);
        let slack_mm = if out.blocked[2] { out.following_error[2].max(0.0) } else { 0.0 };
        let sag = rope_sag_mm(slack_mm, out.rope_m * 1000.0);
        if has(&slack_joint) && self.sent.get("rope sag").is_none_or(|p| (p - sag).abs() > 2.0) {
            res.drives.push((slack_joint, sag));
            self.sent.insert("rope sag".to_string(), sag);
        }
        // what the PLC is told: the axes stopped by a solid, slack rope, a drive at its limit, anything touched
        let stall_force = out.push_n >= 0.95 * s.axis_force_max && s.axis_force_max > 0.0;
        let slack = out.following_error[2] > 20.0 && out.blocked[2];
        let tells = [
            (s.plc.blocked_x_key.clone(), out.blocked[0]),
            (s.plc.blocked_y_key.clone(), out.blocked[1]),
            (s.plc.blocked_hoist_key.clone(), out.blocked[2]),
            (s.plc.slack_key.clone(), slack),
            (s.plc.overload_key.clone(), stall_force),
            (s.plc.contact_key.clone(), out.contact.is_some() && out.blocked.iter().any(|b| *b)),
        ];
        for (key, on) in tells {
            self.tell(&mut res, key, WriteVal::Bool(on), if on { 1.0 } else { 0.0 }, 0.5);
        }
        self.out = out;
        res
    }

    /// Offer a write when it differs from the last one offered by more than `delta`.
    fn tell(&mut self, res: &mut StepOut, key: String, v: WriteVal, shown: f64, delta: f64) {
        if key.trim().is_empty() {
            return;
        }
        if self.sent.get(&key).is_none_or(|q| (q - shown).abs() > delta) {
            self.sent.insert(key.clone(), shown);
            res.writes.push((key, v));
        }
    }

    /// Drive a joint when its value moved by more than half a millimetre (or degree) since last driven.
    fn drive(&mut self, res: &mut StepOut, joint: &str, v: f64) {
        if self.drives_sent.get(joint).is_none_or(|q| (q - v).abs() > 0.5) {
            self.drives_sent.insert(joint.to_string(), v);
            res.drives.push((joint.to_string(), v));
        }
    }

    /// The vessels of a process plant: pumps move liquid through the valves the PLC opens, each liquid surface follows its vessel's level, and the
    /// PLC gets the levels and the high and low switches back.
    fn plant_step(&mut self, s: &Physics, design: &Json, stamp: u64, inp: &Inputs) -> StepOut {
        let mut res = StepOut::default();
        if self.plant.is_none() || self.plant_built != Some(stamp) {
            match Plant::from_design(design) {
                Ok(pl) => {
                    self.plant = Some(pl);
                    self.plant_built = Some(stamp);
                    self.drives_sent.clear();
                    self.message.clear();
                }
                Err(e) => {
                    self.plant = None;
                    self.message = format!("No physics for this design: {e}");
                    return res;
                }
            }
        }
        let dt = inp.dt.clamp(0.0, 0.5);
        let (pump_key, valve_key) = (s.plc.plant_pump_key.clone(), s.plc.plant_valve_key.clone());
        let Some(mut plant) = self.plant.take() else { return res };
        let out = plant.step(dt, &s.plant, &|t| Self::open(inp, &pump_key, t), &|t| Self::open(inp, &valve_key, t));
        self.plant = Some(plant);
        let mut missing = false;
        for (joint, v) in &out.joints {
            if !inp.live_joints.is_some_and(|l| l.iter().any(|j| j == joint)) {
                missing = true;
                continue;
            }
            self.drive(&mut res, joint, *v);
        }
        self.message = if missing { "Solidcraft's model has no liquid surfaces yet. Build the design in Solidcraft from the Design page to get them.".to_string() } else { String::new() };
        let p = &s.plc;
        for (tag, pct) in &out.pct {
            if !p.plant_level_key.trim().is_empty() {
                self.tell(&mut res, physio::key(&p.plant_level_key, tag), WriteVal::Real(*pct), *pct, 0.2);
            }
        }
        for (tag, on) in &out.high {
            if !p.plant_high_key.trim().is_empty() {
                self.tell(&mut res, physio::key(&p.plant_high_key, tag), WriteVal::Bool(*on), if *on { 1.0 } else { 0.0 }, 0.2);
            }
        }
        for (tag, on) in &out.low {
            if !p.plant_low_key.trim().is_empty() {
                self.tell(&mut res, physio::key(&p.plant_low_key, tag), WriteVal::Bool(*on), if *on { 1.0 } else { 0.0 }, 0.2);
            }
        }
        self.plant_out = out;
        res
    }

    /// The cartons of a conveyor line or a robot cell: advance them, drive each one's two slider joints, and tell the PLC which photo-eyes see one.
    fn line_step(&mut self, s: &Physics, design: &Json, stamp: u64, inp: &Inputs) -> StepOut {
        let mut res = StepOut::default();
        if self.line.is_none() || self.line_built != Some(stamp) {
            match Line::from_design(design) {
                Ok(l) => {
                    self.line = Some(l);
                    self.line_built = Some(stamp);
                    self.drives_sent.clear();
                    self.message.clear();
                }
                Err(e) => {
                    self.line = None;
                    self.message = format!("No physics for this design: {e}");
                    return res;
                }
            }
        }
        let dt = inp.dt.clamp(0.0, 0.5);
        let ids: Vec<String> = self.line.as_ref().map(|l| l.belts.iter().map(|b| b.id.clone()).collect()).unwrap_or_default();
        let run: Vec<Option<f64>> = ids.iter().map(|id| Self::open(inp, &s.plc.zone_run_key, id)).collect();
        let Some(line) = self.line.as_mut() else { return res };
        if s.loads.belt_speed_mm_s > 0.0 {
            line.speed_mm_s = s.loads.belt_speed_mm_s;
        }
        let out = line.step(dt, &s.loads, s.gravity, &run);
        self.released += out.released;
        let mut missing = false;
        for p in &out.poses {
            for (joint, v) in [(&p.joints[0], p.x), (&p.joints[1], p.z)] {
                if !inp.live_joints.is_some_and(|l| l.iter().any(|j| j == joint)) {
                    missing = true;
                    continue;
                }
                self.drive(&mut res, joint, v);
            }
        }
        self.message = if missing { "Solidcraft's model has no carton joints yet. Build the design in Solidcraft from the Design page to get them.".to_string() } else { String::new() };
        if !s.plc.zone_pec_key.trim().is_empty() {
            for (id, on) in ids.iter().zip(&out.occupied) {
                self.tell(&mut res, physio::key(&s.plc.zone_pec_key, id), WriteVal::Bool(*on), if *on { 1.0 } else { 0.0 }, 0.5);
            }
        }
        self.line_out = out;
        res
    }
}

#[cfg(test)]
mod tests;
