//! Several PLCs running one model, and several models following one PLC.
//!
//! PLCs: each connection in use has a poller of its own. A signal of the primary PLC (`settings.active_link`) has a plain
//! key; a signal of any other PLC is `<connection id>::<key>` (see `table::scoped`), so two PLCs with data blocks of
//! the same name never share a signal, and one joint or device may follow a signal of any of them.
//!
//! Models: every Solidcraft the bridge drives may show its own model project. The page's model project is the default;
//! a Solidcraft assigned another one gets a `Slot` of its own (its plan, its bindings, its device colours) that reads the
//! same live values, so one PLC can drive any number of models at once.

use crate::work::{self, PollMsg, Poller};
use crate::App;
use ittoot_bridge::connection::{LinkState, Protocol};
use ittoot_bridge::devices::{self, Motion, Shade};
use ittoot_bridge::driver::Value;
use ittoot_bridge::infer::{sensor_points, station_points};
use ittoot_bridge::job::{FreeBinding, ModelKind};
use ittoot_bridge::model::{self, ModelBinding, ModelPlan};
use ittoot_bridge::table::{scoped, split_scope};
use serde_json::Value as Json;
use std::collections::{BTreeMap, HashMap};
use std::time::Instant;

/// Seconds a position must stand still before its axis counts as stopped.
pub const STILL_AFTER: f64 = 1.2;

/// One model project shown by a Solidcraft other than the page's.
pub struct Slot {
    pub key: String,
    pub project: String,
    pub kind: ModelKind,
    pub plan: Option<ModelPlan>,
    pub bindings: Vec<ModelBinding>,
    pub scratch: Vec<FreeBinding>,
    pub shades: Vec<Shade>,
    pub colour_sent: String,
    pub track: HashMap<i64, (f64, Instant)>,
    pub moved_at: HashMap<i64, Instant>,
    pub speeds: HashMap<i64, f64>,
}

/// The joint positions (by axis) a plan's bindings give from the live values, in the PLC's units.
pub fn positions(plan: &ModelPlan, bindings: &[ModelBinding], live: &HashMap<String, Value>) -> HashMap<i64, f64> {
    let mut out = HashMap::new();
    for j in &plan.joints {
        let Some(b) = bindings.iter().find(|b| b.joint == j.name) else { continue };
        let raw = match live.get(&b.key) {
            Some(Value::Real(r)) => Some(*r),
            Some(Value::Int(i)) => Some(*i as f64),
            _ => None,
        };
        if let Some(r) = raw {
            out.insert(j.axis_index, r * b.scale + b.offset);
        }
    }
    out
}

/// The speed of each axis from how far its position moved since it last changed (a PLC is read about twice a second);
/// it holds until the position has been still for a moment.
pub fn track_speeds(track: &mut HashMap<i64, (f64, Instant)>, moved_at: &mut HashMap<i64, Instant>, speeds: &mut HashMap<i64, f64>, position: &HashMap<i64, f64>) {
    let now = Instant::now();
    for (a, v) in position {
        match track.get(a).copied() {
            Some((pv, pt)) if (v - pv).abs() > 0.01 => {
                speeds.insert(*a, (v - pv) / pt.elapsed().as_secs_f64().max(0.05));
                track.insert(*a, (*v, now));
                moved_at.insert(*a, now);
            }
            Some(_) => {
                if moved_at.get(a).is_none_or(|t| t.elapsed().as_secs_f64() > STILL_AFTER) {
                    speeds.insert(*a, 0.0);
                }
            }
            None => {
                track.insert(*a, (*v, now));
            }
        }
    }
}

impl App {
    /// The plan for a kind of model from the PLC's data (and a design, for the design kind), devices included.
    pub fn plan_for(&self, kind: ModelKind, design: Option<&(Json, Vec<Json>)>) -> Result<ModelPlan, String> {
        let stations = station_points(&self.records);
        if kind == ModelKind::Scratch {
            return Ok(ModelPlan::default());
        }
        let mut plan = model::plan(&self.inference, &stations, &self.roles).ok_or("the data gives no X and Y lengths to build a model from")?;
        if let (ModelKind::Design, Some((d, s))) = (kind, design) {
            plan = ittoot_bridge::designmodel::from_design(&plan, d, s.clone(), &stations).map_err(|e| format!("The design could not be used: {e}"))?;
        }
        let keys: Vec<&str> = self.all_keys.iter().map(String::as_str).collect();
        devices::add_devices(&mut plan, &sensor_points(&self.records), &stations, &keys);
        Ok(plan)
    }

    /// The model project a Solidcraft shows when it is not the page's: its own slot.
    pub fn slot_of(&self, target: &str) -> Option<&Slot> {
        self.slots.get(target)
    }

    /// Build, refresh and colour the slots of the Solidcrafts that show a model project other than the page's.
    pub fn update_slots(&mut self) {
        let page = self.settings.model_project.clone();
        let wanted: Vec<(String, String)> = self.settings.models.iter().filter(|m| !m.model_project.is_empty() && Some(&m.model_project) != page.as_ref()).map(|m| (m.id.clone(), m.model_project.clone())).collect();
        self.slots.retain(|id, _| wanted.iter().any(|(i, _)| i == id));
        for (id, name) in wanted {
            let Some(mp) = self.model_projects.iter().find(|m| m.name == name).cloned() else { continue };
            let key = format!("{name}|{:?}|{}|{}|{}|{}", mp.kind, mp.design_dir, mp.free.len(), self.records.len(), self.all_keys.len());
            let mut slot = match self.slots.remove(&id) {
                Some(s) if s.key == key => s,
                _ => {
                    let design = if mp.kind == ModelKind::Design { crate::load_design_dir(std::path::Path::new(&mp.design_dir)) } else { None };
                    let plan = self.plan_for(mp.kind, design.as_ref()).ok();
                    let keys: Vec<&str> = self.all_keys.iter().map(String::as_str).collect();
                    let bindings = plan.as_ref().map(|p| model::effective(p, &model::auto_bind(p, &keys), &self.settings.bindings)).unwrap_or_default();
                    Slot { key, project: name.clone(), kind: mp.kind, plan, bindings, scratch: mp.free.clone(), shades: Vec::new(), colour_sent: String::new(), track: HashMap::new(), moved_at: HashMap::new(), speeds: HashMap::new() }
                }
            };
            if let Some(plan) = slot.plan.clone() {
                let position = positions(&plan, &slot.bindings, &self.live);
                track_speeds(&mut slot.track, &mut slot.moved_at, &mut slot.speeds, &position);
                slot.shades = devices::shades(&plan, &Motion { position: &position, speed: &slot.speeds, live: &self.live, keys: &self.all_keys });
                let colour_key: String = slot.shades.iter().map(|s| format!("{:?}", s.rgb)).collect();
                if colour_key != slot.colour_sent {
                    slot.colour_sent = colour_key;
                    if let Some(r) = self.runs.get(&id)
                        && let (Some(link), true) = (&r.link, r.push_live)
                    {
                        link.send(work::PushMsg::Colour(devices::colour_commands(&plan, &slot.shades)));
                    }
                }
            }
            self.slots.insert(id, slot);
        }
    }

    /// The `joint.drive` commands for one Solidcraft: its own slot's, else the page's model's.
    pub fn drive_for(&self, target: &str, page_cmds: &[Json]) -> Vec<Json> {
        match self.slots.get(target) {
            Some(s) if s.kind == ModelKind::Scratch => model::free_drive_commands(&s.scratch, &self.live),
            Some(s) => s.plan.as_ref().map(|p| model::drive_commands(p, &s.bindings, &self.live)).unwrap_or_default(),
            None => page_cmds.to_vec(),
        }
    }

    /// The commands that build one Solidcraft's model, with the colours it shows now.
    pub fn build_for(&self, target: &str) -> Vec<Json> {
        let (plan, shades) = match self.slots.get(target) {
            Some(s) => (s.plan.clone(), s.shades.clone()),
            None => (self.plan.clone(), self.shades.clone()),
        };
        let Some(plan) = plan else { return Vec::new() };
        let mut c = plan.commands();
        c.extend(devices::colour_commands(&plan, &shades));
        c
    }

    /// Every signal worth reading: the table on show, what each joint follows, and the status bits of the devices, for
    /// the page's model and for every slot.
    fn wanted_keys(&self) -> Vec<String> {
        let mut keys: Vec<String> = self.tables.get(self.active_table).map(|t| t.rows.iter().map(|r| r.key.clone()).collect()).unwrap_or_default();
        keys.extend(self.bindings.iter().map(|b| b.key.clone()));
        keys.extend(self.scratch.iter().filter(|b| !b.key.is_empty()).map(|b| b.key.clone()));
        if let Some(p) = &self.plan {
            keys.extend(devices::device_keys(p, &self.all_keys));
        }
        keys.extend(self.physics_in_keys());
        for s in self.slots.values() {
            keys.extend(s.bindings.iter().map(|b| b.key.clone()));
            keys.extend(s.scratch.iter().filter(|b| !b.key.is_empty()).map(|b| b.key.clone()));
            if let Some(p) = &s.plan {
                keys.extend(devices::device_keys(p, &self.all_keys));
            }
        }
        keys.sort();
        keys.dedup();
        keys
    }

    /// Write a value to the PLC signal `key` (scoped to its connection, or the primary's when plain). False when no connection is up for it.
    pub fn plc_write(&self, key: &str, value: ittoot_bridge::driver::Value) -> bool {
        let primary = self.settings.active_link.clone();
        let (c, raw) = split_scope(key);
        let Some(id) = c.map(str::to_string).or(primary) else { return false };
        match self.pollers.get(&id) {
            Some(p) => {
                p.write(raw, value);
                true
            }
            None => false,
        }
    }

    /// Where the live values come from: one poller for each connection in use, merged into `live`.
    pub fn pump_link(&mut self) {
        let primary = self.settings.active_link.clone().filter(|id| self.settings.connections.iter().any(|c| &c.id == id));
        // the keys each connection must read (its own key, scope removed); a plain key belongs to the primary
        let mut by: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for k in self.wanted_keys() {
            let (c, raw) = split_scope(&k);
            if let Some(id) = c.map(str::to_string).or_else(|| primary.clone()) {
                by.entry(id).or_default().push(raw.to_string());
            }
        }
        if let Some(p) = &primary {
            by.entry(p.clone()).or_default();
        }
        self.pollers.retain(|id, _| by.contains_key(id));
        self.live_by.retain(|id, _| by.contains_key(id));
        for (id, raw) in &by {
            let Some(c) = self.settings.connections.iter().find(|c| &c.id == id).cloned() else { continue };
            if c.protocol != Protocol::OpcUa {
                self.link_states.insert(id.clone(), LinkState::Failed("No driver for this protocol".into()));
                continue;
            }
            if !self.pollers.contains_key(id) {
                let pw = self.secrets.get(&c.id).unwrap_or("").to_string();
                self.pollers.insert(id.clone(), Poller::spawn(work::opcua_config(&c, &pw, &self.pki)));
                self.link_states.insert(id.clone(), LinkState::Connecting);
            }
            if let Some(p) = self.pollers.get(id) {
                while let Ok(m) = p.rx.try_recv() {
                    match m {
                        PollMsg::State(s) => {
                            self.link_states.insert(id.clone(), s);
                        }
                        PollMsg::Values(v) => {
                            self.live_by.insert(id.clone(), v);
                        }
                    }
                }
                p.want(raw.clone());
                p.fast(self.settings.physics.enabled && self.design_model.is_some());
            }
        }
        self.link_states.retain(|id, _| by.contains_key(id));

        // merge: start values, then each PLC's values under its scope (the primary's also under the plain key)
        let primary_conn = primary.as_ref().and_then(|id| self.settings.connections.iter().find(|c| &c.id == id));
        let primary_opc = primary_conn.is_some_and(|c| c.protocol == Protocol::OpcUa);
        let mut live = if primary_conn.is_none() || !primary_opc { self.start_values.clone() } else { HashMap::new() };
        for (id, vals) in &self.live_by {
            for (k, v) in vals {
                live.insert(scoped(id, k), v.clone());
                if Some(id) == primary.as_ref() {
                    live.insert(k.clone(), v.clone());
                }
            }
        }
        let primary_state = primary.as_ref().and_then(|id| self.link_states.get(id)).cloned();
        self.link_state = match (primary_conn, primary_opc, primary_state) {
            (None, _, _) => LinkState::Connected, // the start values in the sources
            (Some(_), false, _) => LinkState::Failed("No driver for this protocol".into()),
            (Some(_), true, Some(s)) => s,
            (Some(_), true, None) => LinkState::Connecting,
        };
        // the simulated machine (no PLC, or a PLC that is not answering): the sliders and the demo stand in for the axes
        let simulated = primary_conn.is_none() || !primary_opc;
        let stand_in = simulated || (self.demo.is_some() && matches!(self.link_state, LinkState::Failed(_)));
        if stand_in {
            let mut all: Vec<&ModelBinding> = self.bindings.iter().collect();
            all.extend(self.slots.values().flat_map(|s| s.bindings.iter()));
            for b in all {
                if let Some(v) = self.pose.get(&b.joint) {
                    live.insert(b.key.clone(), Value::Real(*v));
                }
            }
        }
        self.live = live;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ittoot_bridge::model::JointPlan;

    fn plan() -> ModelPlan {
        let j = |name: &str, i: i64| JointPlan { name: name.into(), letter: 'X', axis_index: i, a: String::new(), b: String::new(), origin: [0.0; 3], axis: [0.0; 3], limits: (0.0, 100.0), drive_limits: (0.0, 100.0), map: Vec::new() };
        ModelPlan { joints: vec![j("A", 0), j("B", 1)], ..Default::default() }
    }

    #[test]
    fn positions_come_from_each_models_own_bindings_of_the_same_live_values() {
        let live: HashMap<String, Value> = [("x".to_string(), Value::Real(40.0)), ("plc2::y".to_string(), Value::Int(7))].into();
        let page = vec![ModelBinding { joint: "A".into(), key: "x".into(), scale: 1.0, offset: 0.0, auto: false }];
        let other = vec![ModelBinding { joint: "A".into(), key: "plc2::y".into(), scale: 2.0, offset: 1.0, auto: false }, ModelBinding { joint: "B".into(), key: "x".into(), scale: 1.0, offset: 0.0, auto: false }];
        assert_eq!(positions(&plan(), &page, &live).get(&0), Some(&40.0));
        let p2 = positions(&plan(), &other, &live);
        assert_eq!((p2.get(&0), p2.get(&1)), (Some(&15.0), Some(&40.0)), "another model, another PLC's signal, the same live map");
    }

    #[test]
    fn speed_is_measured_between_changes_and_holds_until_the_axis_is_still() {
        let (mut track, mut moved, mut speeds) = (HashMap::new(), HashMap::new(), HashMap::new());
        track_speeds(&mut track, &mut moved, &mut speeds, &HashMap::from([(0, 100.0)]));
        std::thread::sleep(std::time::Duration::from_millis(100));
        track_speeds(&mut track, &mut moved, &mut speeds, &HashMap::from([(0, 150.0)]));
        assert!(speeds[&0] > 100.0 && speeds[&0] < 600.0, "{}", speeds[&0]);
        track_speeds(&mut track, &mut moved, &mut speeds, &HashMap::from([(0, 150.0)]));
        assert!(speeds[&0] > 0.0, "holds for a moment between polls");
        moved.insert(0, Instant::now() - std::time::Duration::from_secs(5));
        track_speeds(&mut track, &mut moved, &mut speeds, &HashMap::from([(0, 150.0)]));
        assert_eq!(speeds[&0], 0.0);
    }
}
