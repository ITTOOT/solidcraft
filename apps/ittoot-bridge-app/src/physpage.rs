//! The Physics page and the physics step. The PLC's axis positions go through the physics before they reach Solidcraft: the
//! jig swings on its rope, cannot pass through the walls, floors, columns and fences of the design, and the page shows what
//! it is doing and lets every number be changed. The settings are kept with the bridge's settings.

use crate::theme::*;
use crate::widgets::*;
use crate::App;
use egui::{RichText, Ui};
use ittoot_bridge::physics::{Physics, FIELDS, GROUPS, PRESETS};
use crate::work::PushMsg;
use ittoot_bridge::runner::{bath_id, Inputs, Runner, WriteVal};
use serde_json::{json, Value};
use std::time::Instant;

/// The physics run (see `ittoot_physics::runner`) and when it last stepped.
#[derive(Default)]
pub struct PhysState {
    pub run: Runner,
    last: Option<Instant>,
    pub eng: EngineState,
}

/// The physics running inside Solidcraft's engine instead of here: what has been sent to it and what it answered.
#[derive(Default)]
pub struct EngineState {
    /// The design stamp the engine's run was loaded for, and the settings it was last given (as JSON text).
    loaded: Option<u64>,
    settings: String,
    /// The design stamp the local runner was prepared for (it only names the signals to read).
    prepared: Option<u64>,
    /// A step is out and not answered yet; time that passed meanwhile is added to the next one.
    busy: bool,
    dt_acc: f64,
    /// Writes the engine asked for that did not reach the PLC.
    unsent: Vec<String>,
    /// The engine's last step reply, and the last error it gave.
    pub reply: Value,
    pub error: String,
}

impl std::ops::Deref for PhysState {
    type Target = Runner;
    fn deref(&self) -> &Runner {
        &self.run
    }
}

impl std::ops::DerefMut for PhysState {
    fn deref_mut(&mut self) -> &mut Runner {
        &mut self.run
    }
}

#[cfg(test)]
fn drive_value(cmds: &[Value], joint: &str) -> Option<f64> {
    cmds.iter().find(|c| c["params"]["joint"] == joint).and_then(|c| c["params"]["value"].as_f64())
}

fn set_drive(cmds: &mut [Value], joint: &str, v: f64) -> bool {
    let mut found = false;
    for c in cmds.iter_mut() {
        if c["params"]["joint"] == joint {
            c["params"]["value"] = json!(v);
            found = true;
        }
    }
    found
}

/// A PLC value as the number the physics reads: a bool is 0 or 1.
fn as_number(v: &ittoot_bridge::driver::Value) -> Option<f64> {
    match v {
        ittoot_bridge::driver::Value::Bool(b) => Some(if *b { 1.0 } else { 0.0 }),
        ittoot_bridge::driver::Value::Int(i) => Some(*i as f64),
        ittoot_bridge::driver::Value::Real(r) => Some(*r),
        ittoot_bridge::driver::Value::Text(_) => None,
    }
}

impl App {
    /// Whether the physics is on and there is a design for it to run on.
    pub fn physics_active(&self) -> bool {
        self.settings.physics.enabled && self.design_model.is_some()
    }

    /// The joints Solidcraft's model has now (asked after a build), or `None` until it has answered.
    fn live_joints(&self) -> Option<Vec<String>> {
        let id = self.settings.models.get(self.active_model_index()).map(|m| m.id.clone()).unwrap_or_default();
        self.runs.get(&id).and_then(|r| r.joints.clone())
    }

    /// Put the PLC's axis positions through the physics: the axes are held where the load is stopped, and the sway, rock, level, rope and carton joints get
    /// what the physics found. The step itself is `ittoot_physics::runner`; this hands it the PLC's signals and passes on what it wants driven and written.
    pub fn physics_step(&mut self, cmds: &mut Vec<Value>) {
        if !self.physics_active() {
            return;
        }
        let Some((design, _)) = self.design_model.clone() else { return };
        let now = Instant::now();
        let dt = self.phys.last.map(|t| now.duration_since(t).as_secs_f64()).unwrap_or(0.0).min(0.5);
        self.phys.last = Some(now);
        let commanded: std::collections::BTreeMap<String, f64> =
            cmds.iter().filter_map(|c| Some((c["params"]["joint"].as_str()?.to_string(), c["params"]["value"].as_f64()?))).collect();
        let axes = self.plan.as_ref().and_then(|plan| {
            let find = |letter: char| plan.joints.iter().find(|j| j.letter == letter).map(|j| j.name.clone());
            Some([find('X')?, find('Y')?, find('Z')?])
        });
        if self.settings.physics.engine {
            self.engine_step(cmds, &design, dt, &commanded, axes);
            return;
        }
        self.phys.eng.loaded = None;
        self.phys.eng.busy = false;
        let live = self.live_joints();
        let inp = Inputs { dt, axes, commanded: &commanded, live_joints: live.as_deref(), signal: &|k| self.live.get(k).and_then(as_number) };
        let out = self.phys.run.step(&self.settings.physics, &design, self.design_stamp, &inp);
        for (joint, v) in out.drives {
            if !set_drive(cmds, &joint, v) {
                cmds.push(json!({"command":"joint.drive","params":{"joint":joint,"value":v,"quiet":true}}));
            }
        }
        for (key, v) in out.writes {
            let value = match v {
                WriteVal::Bool(b) => ittoot_bridge::driver::Value::Bool(b),
                WriteVal::Real(r) => ittoot_bridge::driver::Value::Real(r),
            };
            if !self.plc_write(&key, value) {
                self.phys.run.forget(&key);
            }
        }
    }

    /// The physics runs in Solidcraft's engine: load the design there once, keep its settings current, and send one `physics.step` at a time with the PLC's
    /// commanded positions and signals. The engine drives the model's joints itself, the gantry's axes included, so those are taken out of the plain drives.
    fn engine_step(&mut self, cmds: &mut Vec<Value>, design: &Value, dt: f64, commanded: &std::collections::BTreeMap<String, f64>, axes: Option<[String; 3]>) {
        let stamp = self.design_stamp;
        // the local runner is only built, to know which PLC signals the physics wants
        if self.phys.eng.prepared != Some(stamp) {
            self.phys.run.prepare(&self.settings.physics, design, stamp);
            self.phys.eng.prepared = Some(stamp);
        }
        if let Some(ax) = &axes {
            cmds.retain(|c| !ax.iter().any(|a| c["params"]["joint"] == a.as_str()));
        }
        let id = self.settings.models.get(self.active_model_index()).map(|m| m.id.clone()).unwrap_or_default();
        let settings_json = serde_json::to_value(&self.settings.physics).unwrap_or(Value::Null);
        let text = settings_json.to_string();
        let keys = self.phys.run.input_keys(&self.settings.physics);
        let signals: std::collections::BTreeMap<String, f64> = keys.iter().filter_map(|k| Some((k.clone(), as_number(self.live.get(k)?)?))).collect();
        let Some(link) = self.runs.get(&id).and_then(|r| r.link.as_ref()) else {
            self.phys.eng.error = "Solidcraft is not connected: start it from the Model page".to_string();
            return;
        };
        if self.phys.eng.loaded != Some(stamp) {
            link.send(PushMsg::Call { tag: "load".into(), command: "physics.load".into(), params: json!({"design": design, "settings": settings_json, "axes": axes}) });
            self.phys.eng.loaded = Some(stamp);
            self.phys.eng.settings = text;
            self.phys.eng.busy = false;
            self.phys.eng.dt_acc = 0.0;
            return;
        }
        if text != self.phys.eng.settings {
            link.send(PushMsg::Call { tag: "settings".into(), command: "physics.settings".into(), params: json!({"settings": settings_json}) });
            self.phys.eng.settings = text;
        }
        self.phys.eng.dt_acc += dt;
        if self.phys.eng.busy {
            return;
        }
        let unsent = std::mem::take(&mut self.phys.eng.unsent);
        let params = json!({"dt": self.phys.eng.dt_acc.min(60.0), "commanded": commanded, "signals": signals, "unsent": unsent});
        link.send(PushMsg::Call { tag: "step".into(), command: "physics.step".into(), params });
        self.phys.eng.busy = true;
        self.phys.eng.dt_acc = 0.0;
    }

    /// An answer from the engine's physics commands (`call:<tag>:<json>` from the link).
    pub fn engine_answer(&mut self, _model: &str, tag: &str, body: &str) {
        let v: Value = serde_json::from_str(body).unwrap_or(Value::Null);
        if tag == "step" {
            self.phys.eng.busy = false;
        }
        if let Some(e) = v["error"].as_str() {
            self.phys.eng.error = format!("{tag}: {e}");
            // Solidcraft was restarted or its document replaced: the run is gone, load it again
            if e.contains("no physics run") {
                self.phys.eng.loaded = None;
            }
            return;
        }
        self.phys.eng.error.clear();
        if tag != "step" {
            return;
        }
        for w in v["writes"].as_array().into_iter().flatten() {
            let (Some(key), Some(x)) = (w["key"].as_str(), w["value"].as_f64()) else { continue };
            let value = if w["bool"].as_bool().unwrap_or(false) { ittoot_bridge::driver::Value::Bool(x >= 0.5) } else { ittoot_bridge::driver::Value::Real(x) };
            if !self.plc_write(key, value) {
                self.phys.eng.unsent.push(key.to_string());
            }
        }
        self.phys.eng.reply = v;
    }

    /// The PLC signals the physics reads.
    pub fn physics_in_keys(&self) -> Vec<String> {
        self.phys.run.input_keys(&self.settings.physics)
    }

    pub fn page_physics(&mut self, ui: &mut Ui) {
        ui.horizontal(|ui| {
            ui.label(RichText::new("Physics").heading());
            ui.label(RichText::new("What the machine would really do: gravity, the swing of the load, solid walls, air, liquid, flow and pressure").color(DIM));
        });
        let active = self.physics_active();
        let mut changed = false;
        let mut rebuild = false;
        panel(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                changed |= ui.checkbox(&mut self.settings.physics.enabled, "Physics on").changed();
                changed |= ui.checkbox(&mut self.settings.physics.engine, "Run inside Solidcraft").on_hover_text("The physics runs in Solidcraft's own engine (the physics.* commands) and drives its joints there; this app only sends the PLC's positions and signals and takes back what the PLC should be told. Needs Solidcraft started from the Model page with the design built.").changed();
                changed |= ui.checkbox(&mut self.settings.physics.sway, "Sway").changed();
                changed |= ui.checkbox(&mut self.settings.physics.collisions, "Solid walls (collisions)").changed();
                ui.separator();
                ui.label("Preset");
                egui::ComboBox::from_id_salt("phys-preset").selected_text("choose").width(210.0).show_ui(ui, |ui| {
                    for n in PRESETS {
                        if let (true, Some(p)) = (ui.selectable_label(false, *n).clicked(), Physics::preset(n)) {
                            let (mass, parts) = (self.settings.physics.load_mass, self.settings.physics.solid_parts.clone());
                            self.settings.physics = Physics { load_mass: mass, solid_parts: parts, ..p };
                            changed = true;
                            rebuild = true;
                        }
                    }
                });
                if ui.button("Defaults").on_hover_text("Every setting back to a real plating jig in air at 20 C").clicked() {
                    self.settings.physics = Physics::default();
                    changed = true;
                    rebuild = true;
                }
                if let (true, Some(w)) = (ui.button("Settle the load").on_hover_text("Stop the swing and start again from rest").clicked(), self.phys.world.as_mut()) {
                    w.reset();
                }
            });
            if !active {
                ui.label(RichText::new("The physics runs on a design from the design agent (Design page, or a model project of kind Design). With none loaded the PLC's axes drive the model directly.").color(WARN).size(13.0));
            }
            if !self.phys.message.is_empty() {
                ui.label(RichText::new(&self.phys.message).color(WARN).size(13.0));
            }
        });
        ui.add_space(6.0);
        if self.settings.physics.engine {
            let r = self.phys.eng.reply.clone();
            panel(ui, |ui| {
                ui.label(RichText::new("Running inside Solidcraft").strong());
                if !self.phys.eng.error.is_empty() {
                    ui.label(RichText::new(&self.phys.eng.error).color(WARN).size(13.0));
                }
                if let Some(m) = r["message"].as_str().filter(|m| !m.is_empty()) {
                    ui.label(RichText::new(m).color(WARN).size(13.0));
                }
                if r.is_object() {
                    let blocked: Vec<&str> = ["X", "Y", "hoist"].iter().zip(r["blocked"].as_array().into_iter().flatten()).filter(|(_, b)| b.as_bool() == Some(true)).map(|(n, _)| *n).collect();
                    ui.label(format!(
                        "Held by a solid: {}. Hits {}, hardest {:.0} N. Cartons on belts {}, fallen {}, released {}.",
                        if blocked.is_empty() { "nothing".to_string() } else { blocked.join(", ") },
                        r["hits"].as_u64().unwrap_or(0),
                        r["peak_impact_n"].as_f64().unwrap_or(0.0),
                        r["on_belts"].as_u64().unwrap_or(0),
                        r["fallen"].as_u64().unwrap_or(0),
                        r["released"].as_u64().unwrap_or(0)
                    ));
                    for (tag, pct) in r["levels"].as_object().into_iter().flatten() {
                        ui.label(format!("{tag}: {:.0} %", pct.as_f64().unwrap_or(0.0)));
                    }
                }
            });
            ui.add_space(6.0);
        }
        // what it is doing now
        let o = self.phys.out.clone();
        if let Some(vols) = self.phys.plant.as_ref().map(|pl| pl.vessels.iter().map(|v| (v.tag.clone(), v.volume_l)).collect::<Vec<_>>()) {
            let po = self.phys.plant_out.clone();
            panel(ui, |ui| {
                ui.label(RichText::new("Process plant").strong());
                for (tag, pct) in &po.pct {
                    let litres = vols.iter().find(|v| &v.0 == tag).map(|v| v.1).unwrap_or(0.0);
                    ui.label(format!("{tag}: {pct:.0} % ({litres:.0} L)"));
                }
                let flows: Vec<String> = po.flows.iter().map(|(t, q)| format!("{t} {q:.0} L/min")).collect();
                ui.label(RichText::new(format!("Pumps: {}", if flows.is_empty() { "none commanded".to_string() } else { flows.join(", ") })).color(DIM).size(13.0));
                if !po.dry.is_empty() {
                    ui.label(RichText::new(format!("Running dry: {}", po.dry.join(", "))).color(egui::Color32::from_rgb(220, 60, 60)));
                }
                if ui.button("Reset the levels").clicked()
                    && let Some(l) = self.phys.plant.as_mut()
                {
                    l.reset();
                }
            });
            ui.add_space(6.0);
        }
        if let Some(line) = &self.phys.line {
            let lo = self.phys.line_out.clone();
            let summary = format!("{} belts, {} cartons in the model: {} on a belt or carried, {} fallen, {} released so far.", line.belts.len(), line.loads.len(), lo.on_belts, lo.fallen, self.phys.released);
            let eyes: Vec<String> = line.belts.iter().zip(&lo.occupied).map(|(b, on)| format!("{} {}", b.id, if *on { "occupied" } else { "clear" })).collect();
            let mut clear = false;
            panel(ui, |ui| {
                ui.label(RichText::new("Cartons on the belts").strong());
                ui.label(summary);
                ui.label(RichText::new(format!("Photo-eyes: {}", eyes.join(", "))).color(DIM).size(13.0));
                clear = ui.button("Clear all cartons").clicked();
            });
            if clear {
                if let Some(l) = self.phys.line.as_mut() {
                    l.reset();
                }
                self.phys.released = 0;
            }
            ui.add_space(6.0);
        }
        panel(ui, |ui| {
            ui.label(RichText::new("Now").strong());
            egui::Grid::new("phys-now").num_columns(4).spacing([26.0, 4.0]).show(ui, |ui| {
                let sway = format!("{:+.2} deg toward X, {:+.2} deg toward Y", o.phi_deg[0], o.phi_deg[1]);
                ui.label("Swing");
                ui.label(RichText::new(sway).color(INFO));
                ui.label("Rope");
                ui.label(RichText::new(format!("{:.2} m, a swing every {:.1} s", o.rope_m, o.period_s)).color(INFO));
                ui.end_row();
                let blocked: Vec<&str> = ["X", "Y", "hoist"].iter().zip(o.blocked).filter(|(_, b)| *b).map(|(n, _)| *n).collect();
                ui.label("Stopped by a solid");
                if blocked.is_empty() {
                    ui.label(RichText::new("nothing").color(OK));
                } else {
                    ui.label(RichText::new(format!("{} ({})", blocked.join(", "), o.contact.clone().unwrap_or_default())).color(ERROR));
                }
                ui.label("Behind the PLC by");
                ui.label(RichText::new(format!("X {:.0}  Y {:.0}  hoist {:.0} mm", o.following_error[0], o.following_error[1], o.following_error[2])).color(if o.following_error.iter().any(|e| e.abs() > 1.0) { WARN } else { DIM }));
                ui.end_row();
                ui.label("In the liquid");
                ui.label(RichText::new(format!("{:.0} %", o.immersion * 100.0)).color(INFO));
                ui.label("Drag on the load");
                ui.label(RichText::new(format!("{:.1} N", o.drag_n)).color(INFO));
                ui.end_row();
                ui.label("Hits so far");
                ui.label(RichText::new(format!("{} (hardest {:.2} m/s)", self.phys.hits, self.phys.peak_impact)).color(if self.phys.hits > 0 { WARN } else { DIM }));
                ui.label("Air density");
                ui.label(RichText::new(format!("{:.3} kg/m3", self.settings.physics.air_density())).color(INFO));
                ui.end_row();
                let rock = o.rock_deg.iter().map(|(n, a)| (n.clone(), a[0].abs().max(a[1].abs()))).fold((String::new(), 0.0_f64), |b, c| if c.1 > b.1 { c } else { b });
                ui.label("Baths rocking");
                if rock.1 > 0.02 {
                    ui.label(RichText::new(format!("{} by {:.2} deg", rock.0, rock.1)).color(WARN));
                } else {
                    ui.label(RichText::new("still").color(OK));
                }
                ui.label("Drive pushing");
                ui.label(RichText::new(format!("{:.0} N", o.push_n)).color(if o.push_n > 0.0 { WARN } else { DIM }));
                ui.label("Slack rope");
                let slack = if o.blocked[2] { o.following_error[2].max(0.0) } else { 0.0 };
                ui.label(RichText::new(format!("{slack:.0} mm paid out")).color(if slack > 20.0 { WARN } else { DIM }));
                ui.end_row();
            });
            if let (Some(l), Some(s)) = (self.phys.levels.first(), &self.phys.supply) {
                ui.add_space(4.0);
                ui.label(RichText::new(format!(
                    "Bath level {:.0} mm of {:.0} ({:.0} litres).  Supply: {:.2} m/s, {:.0} litres/min at full open, Reynolds {:.0}, friction {:.3}, {:.0} kPa lost in the run.",
                    l.mm,
                    l.max_mm,
                    l.litres(),
                    s.speed,
                    s.flow * 60000.0,
                    s.reynolds,
                    s.friction,
                    s.lost_kpa
                )).color(DIM).size(13.0));
            }
        });
        ui.add_space(6.0);
        // every setting, by group
        for g in GROUPS {
            ui.collapsing(RichText::new(*g).strong().size(16.0), |ui| {
                egui::Grid::new(format!("phys-{g}")).num_columns(3).spacing([14.0, 5.0]).show(ui, |ui| {
                    for f in FIELDS.iter().filter(|f| f.group == *g) {
                        ui.label(f.label).on_hover_text(f.help);
                        let mut v = self.settings.physics.get(f.key).unwrap_or(0.0);
                        let before = v;
                        let r = ui.add(egui::DragValue::new(&mut v).range(f.min..=f.max).speed(f.step * 2.0).max_decimals(if f.step < 0.01 { 4 } else if f.step < 1.0 { 2 } else { 0 }));
                        r.on_hover_text(f.help);
                        ui.label(RichText::new(f.unit).color(DIM));
                        ui.end_row();
                        if (v - before).abs() > f64::EPSILON {
                            self.settings.physics.set(f.key, v);
                            changed = true;
                            if f.key == "wall_thickness_mm" {
                                rebuild = true;
                            }
                        }
                    }
                    if *g == "Solids and collisions" {
                        ui.label("Solid parts").on_hover_text("Parts of the design the load cannot pass through: name fragments, comma-separated");
                        let r = ui.add(egui::TextEdit::singleline(&mut self.settings.physics.solid_parts).desired_width(360.0));
                        if r.changed() {
                            changed = true;
                        }
                        ui.label("");
                        ui.end_row();
                    }
                });
            });
        }
        // the walls the physics uses are the setting; the design's own walls are changed by asking the design agent
        if (self.settings.physics.wall_thickness_mm - 0.0).abs() > 0.5 {
            panel(ui, |ui| {
                ui.label(RichText::new(format!("The physics uses walls {:.0} mm thick; the drawn walls keep the design's thickness until the design is changed.", self.settings.physics.wall_thickness_mm)).color(WARN).size(13.0));
                if ui.button("Ask the design agent to make the walls this thick").on_hover_text("Fills in the Engineering page; you send it from there").clicked() {
                    let t = self.settings.physics.wall_thickness_mm;
                    self.eng.req = ittoot_bridge::engineering::ChangeRequest {
                        area: "All baths (walls and floors)".into(),
                        what: format!("Set the bath wall thickness to {t:.0} mm"),
                        how: "set the design's tank.wallMm; regenerate into the same project and output folder; keep every other size".into(),
                        why: "the physics page's wall thickness setting, so the drawn walls match what the load hits".into(),
                    };
                    self.page = crate::Page::Engineering;
                }
            });
        }
        // the PLC signals the physics reads and writes
        let keys = self.all_keys.clone();
        ui.collapsing(RichText::new("PLC links").strong().size(16.0), |ui| {
            ui.horizontal(|ui| {
                if ui.button("Use the Physics_01 data block").on_hover_text("Fills every link below with the member of the data block Physics_01 that has the same job").clicked() {
                    self.settings.physics.plc = ittoot_bridge::physio::standard_links();
                    changed = true;
                }
                if ui.button("Copy the block's source").on_hover_text("The SCL source of Physics_01 for this design, to import into the PLC project").clicked()
                    && let Some((design, _)) = self.design_model.clone()
                {
                    let baths: Vec<String> = self.phys.world.as_ref().map(|w| w.baths.iter().map(|b| bath_id(&b.name)).collect()).unwrap_or_default();
                    ui.ctx().copy_text(ittoot_bridge::physio::scl(&design, &baths));
                }
            });
            ui.label(RichText::new("Optional. The valves are read from the PLC (a bool is 0 or 1, a number above 1 a percentage); what the physics found is written to it as TRUE or FALSE. `{id}` in a valve key is the bath's id (T2, T3 ...). The PLC project needs points to match.").color(DIM).size(13.0));
            egui::Grid::new("phys-plc").num_columns(3).spacing([14.0, 5.0]).show(ui, |ui| {
                let mut row = |ui: &mut Ui, label: &str, help: &str, v: &mut String| {
                    ui.label(label).on_hover_text(help);
                    let r = ui.add(egui::TextEdit::singleline(v).desired_width(380.0).hint_text("not linked"));
                    if r.changed() {
                        changed = true;
                    }
                    let typed = v.to_lowercase();
                    egui::ComboBox::from_id_salt(format!("pick-{label}")).selected_text("pick a signal").width(150.0).show_ui(ui, |ui| {
                        for k in keys.iter().filter(|k| typed.is_empty() || k.to_lowercase().contains(&typed)).take(40) {
                            if ui.selectable_label(false, k).clicked() {
                                *v = k.clone();
                                changed = true;
                            }
                        }
                    });
                    ui.end_row();
                };
                let p = &mut self.settings.physics.plc;
                row(ui, "Fill valve (read)", "How far the fill valve of the bath is open", &mut p.fill_key);
                row(ui, "Drain valve (read)", "How far the drain valve of the bath is open", &mut p.drain_key);
                row(ui, "X stopped by a solid (write)", "TRUE while the X axis is held back by something the load has met", &mut p.blocked_x_key);
                row(ui, "Y stopped by a solid (write)", "TRUE while the Y axis is held back", &mut p.blocked_y_key);
                row(ui, "Hoist stopped by a solid (write)", "TRUE while the hoist is held back: the jig is on a rim, a floor or a wall", &mut p.blocked_hoist_key);
                row(ui, "Slack rope (write)", "TRUE while the hoist has paid out more than 20 mm of rope the hook has not gone down", &mut p.slack_key);
                row(ui, "Drive at its limit (write)", "TRUE while an axis is pushing at its force limit against a solid: an overload", &mut p.overload_key);
                row(ui, "Anything touched (write)", "TRUE while the load is held by any solid", &mut p.contact_key);
                row(ui, "Pump run (read)", "A process plant: TRUE while the pump is commanded to run. {id} is the pump's tag (P-101 ...)", &mut p.plant_pump_key);
                row(ui, "Valve open (read)", "A process plant: TRUE while the line's valve is open. {id} is the valve's tag (XV-101 ...)", &mut p.plant_valve_key);
                row(ui, "Vessel level (write)", "A process plant: the vessel's fill in percent. {id} is the vessel's tag (T-101 ...)", &mut p.plant_level_key);
                row(ui, "High level (write)", "A process plant: TRUE at the high level switch of the vessel ({id})", &mut p.plant_high_key);
                row(ui, "Low level (write)", "A process plant: TRUE at the low level switch of the vessel ({id})", &mut p.plant_low_key);
                row(ui, "Belt drive (read)", "A conveyor line or robot cell: how fast each belt is driven. {id} is the belt's id (Z1, Z2, IN, OUT)", &mut p.zone_run_key);
                row(ui, "Photo-eye (write)", "A conveyor line or robot cell: TRUE while a carton is over the eye at the discharge end of the belt ({id})", &mut p.zone_pec_key);
            });
        });
        if rebuild {
            self.phys.run.invalidate();
        }
        if changed {
            self.save();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ittoot_bridge::runner::{rope_sag_mm, slack_name, sway_names};

    #[test]
    fn a_slack_rope_sags_by_the_loop_a_loose_rope_makes() {
        assert_eq!(rope_sag_mm(0.0, 2000.0), 0.0);
        assert_eq!(rope_sag_mm(-5.0, 2000.0), 0.0);
        // 100 mm too much rope over 2 m hangs sqrt(3 x 100 x 2000 / 8) = 274 mm off the line
        assert!((rope_sag_mm(100.0, 2000.0) - 273.9).abs() < 0.5);
        // more slack sags more, and the slider's range is kept
        assert!(rope_sag_mm(200.0, 2000.0) > rope_sag_mm(100.0, 2000.0));
        assert_eq!(rope_sag_mm(5000.0, 3000.0), 350.0);
    }

    #[test]
    fn the_sway_and_slack_joints_are_named_for_the_hoist_joint() {
        assert_eq!(sway_names("Hoist Z"), ("Sway Rx".to_string(), "Sway Ry".to_string()));
        assert_eq!(sway_names("Transporter A Z").0, "Transporter A Sway Rx");
        assert_eq!(slack_name("Hoist Z"), "Rope slack");
        assert_eq!(slack_name("Transporter B Z"), "Transporter B rope slack");
        assert_eq!(bath_id("Tank T2"), "T2");
    }

    /// An app with a design of this application, physics on and Solidcraft answering with these joints.
    fn app_with(design: Value, joints: &[&str]) -> crate::App {
        let mut app = crate::App::new(false);
        app.settings.physics.enabled = true;
        app.design_model = Some((design, Vec::new()));
        app.design_stamp = 1;
        let id = app.settings.models.get(app.active_model_index()).map(|m| m.id.clone()).unwrap_or_default();
        app.runs.entry(id).or_default().joints = Some(joints.iter().map(|j| j.to_string()).collect());
        // a long time since the last step: one step of the longest length
        app.phys.last = Some(Instant::now() - std::time::Duration::from_secs(10));
        app
    }

    fn driven(cmds: &[Value], joint: &str) -> Option<f64> {
        drive_value(cmds, joint)
    }

    fn plant_design() -> Value {
        json!({"meta": {"application": "process"}, "geometry": {"process": {
            "vessels": [
                {"tag": "T-1", "joint": "Vessel T-1 Level", "heightMm": 2000, "baseMm": 50, "volumeL": 1000, "level0Mm": 1000},
                {"tag": "T-2", "joint": "Vessel T-2 Level", "heightMm": 2000, "baseMm": 50, "volumeL": 1000, "level0Mm": 0}],
            "pumps": [{"tag": "P-1", "flowLpm": 600}],
            "lines": [{"from": "T-1", "to": "P-1", "valve": "XV-1", "flowLpm": 600}, {"from": "P-1", "to": "T-2", "valve": "XV-2", "flowLpm": 600}]}}})
    }

    #[test]
    fn the_plant_moves_liquid_only_while_the_plc_runs_the_pump_and_opens_the_valves() {
        use ittoot_bridge::driver::Value as V;
        let mut app = app_with(plant_design(), &["Vessel T-1 Level", "Vessel T-2 Level"]);
        app.settings.physics.plc = ittoot_bridge::physio::standard_links();
        // nothing is read from the PLC yet: nothing moves, and no joint is driven
        let mut cmds = Vec::new();
        app.physics_step(&mut cmds);
        assert!(cmds.is_empty() || driven(&cmds, "Vessel T-2 Level").is_some_and(|v| v.abs() < 0.5), "{cmds:?}");
        // the PLC runs P-1 with both valves open: the keys are the data block's members, with the ids as tag names
        for k in ["\"Physics_01\".pump_P_1", "\"Physics_01\".valve_XV_1", "\"Physics_01\".valve_XV_2"] {
            app.live.insert(k.to_string(), V::Bool(true));
        }
        assert!(app.physics_in_keys().contains(&"\"Physics_01\".pump_P_1".to_string()));
        app.phys.last = Some(Instant::now() - std::time::Duration::from_secs(10));
        let mut cmds = Vec::new();
        app.physics_step(&mut cmds);
        // half a second at 600 L/min: 5 L of 1000 moved, so the second vessel's surface is 10 mm up and the first's 10 mm down
        let up = driven(&cmds, "Vessel T-2 Level").expect("driven");
        let down = driven(&cmds, "Vessel T-1 Level").expect("driven");
        assert!((up - 10.0).abs() < 1.5 && (down + 10.0).abs() < 1.5, "{up} {down}");
        assert!(app.phys.message.is_empty());
        // the pump is stopped: the surfaces stay
        app.live.insert("\"Physics_01\".pump_P_1".to_string(), V::Bool(false));
        app.phys.last = Some(Instant::now() - std::time::Duration::from_secs(10));
        let mut cmds = Vec::new();
        app.physics_step(&mut cmds);
        assert!(cmds.is_empty(), "{cmds:?}");
    }

    #[test]
    fn a_surface_the_drawn_model_lacks_is_not_driven_and_the_page_says_to_build_it() {
        let mut app = app_with(plant_design(), &[]);
        let mut cmds = Vec::new();
        app.physics_step(&mut cmds);
        assert!(cmds.is_empty());
        assert!(app.phys.message.contains("liquid surfaces"), "{}", app.phys.message);
    }

    fn line_design() -> Value {
        json!({"meta": {"application": "conveyor_line"}, "spec": {"speedMmS": 500, "loadKg": 30},
            "geometry": {"belts": [{"id": "Z1", "x0": 0, "x1": 3000, "y": 350, "widthMm": 700, "topMm": 800}],
                "loads": [{"slot": 1, "joints": ["Load 1 X", "Load 1 Z"], "park": [-2500.0, 350, -1500], "lengthMm": 600, "widthMm": 400, "heightMm": 300}]}})
    }

    #[test]
    fn a_carton_released_on_a_belt_is_driven_on_its_two_joints_and_the_photo_eye_is_written() {
        let mut app = app_with(line_design(), &["Load 1 X", "Load 1 Z"]);
        app.settings.physics.plc = ittoot_bridge::physio::standard_links();
        app.settings.physics.loads.first_after_s = 0.0;
        app.settings.physics.loads.creator_every_s = 1.0;
        let mut cmds = Vec::new();
        for _ in 0..4 {
            app.phys.last = Some(Instant::now() - std::time::Duration::from_secs(1));
            app.physics_step(&mut cmds);
        }
        assert!(driven(&cmds, "Load 1 X").is_some() && driven(&cmds, "Load 1 Z").is_some_and(|z| (z - 2300.0).abs() < 1.0), "{cmds:?}");
        assert!(app.phys.released >= 1 && app.phys.line_out.on_belts == 1);
        assert!(app.phys.message.is_empty());
        // a model without the carton joints is told to build
        let mut bare = app_with(line_design(), &[]);
        bare.settings.physics.loads.first_after_s = 0.0;
        let mut cmds = Vec::new();
        bare.physics_step(&mut cmds);
        assert!(cmds.is_empty() && bare.phys.message.contains("carton joints"));
    }

    #[test]
    fn the_engines_answers_are_taken_and_a_lost_run_is_loaded_again() {
        let mut app = app_with(plant_design(), &[]);
        app.phys.eng.busy = true;
        app.phys.eng.loaded = Some(1);
        // a step answer: busy ends, its write cannot reach a PLC that is not linked and is offered again at the next step, the reply is kept
        let body = json!({"writes": [{"key": "\"Physics_01\".level_T_2", "value": 12.0, "bool": false}], "message": "", "levels": {"T-2": 12.0}}).to_string();
        app.engine_answer("", "step", &body);
        assert!(!app.phys.eng.busy && app.phys.eng.unsent == vec!["\"Physics_01\".level_T_2".to_string()]);
        assert_eq!(app.phys.eng.reply["levels"]["T-2"], 12.0);
        assert!(app.phys.eng.error.is_empty());
        // Solidcraft lost the run (restarted, or a new document): the error is shown and the next tick loads the run again
        app.engine_answer("", "step", &json!({"error": "physics.step: no physics run: physics.load starts one"}).to_string());
        assert!(app.phys.eng.loaded.is_none() && app.phys.eng.error.contains("no physics run") && !app.phys.eng.busy);
        // any other error is shown and does not reload
        app.phys.eng.loaded = Some(1);
        app.engine_answer("", "settings", &json!({"error": "there is no setting `x`"}).to_string());
        assert_eq!(app.phys.eng.loaded, Some(1));
        assert!(app.phys.eng.error.contains("no setting"));
    }

    #[test]
    fn with_the_engine_on_and_no_solidcraft_the_page_says_so_and_the_axes_are_left_alone() {
        let mut app = app_with(plant_design(), &[]);
        app.settings.physics.engine = true;
        let mut cmds = vec![json!({"command": "joint.drive", "params": {"joint": "Gantry X", "value": 5.0}})];
        app.physics_step(&mut cmds);
        assert!(app.phys.eng.error.contains("not connected"), "{}", app.phys.eng.error);
        // the signals it wants are known from the design, with no stepping done here
        assert!(app.phys.eng.prepared.is_some());
        assert!(app.phys.run.plant.is_some());
    }

    #[test]
    fn a_design_the_physics_cannot_move_gets_a_message_and_physics_off_does_nothing() {
        let mut app = app_with(json!({"meta": {"application": "process"}, "geometry": {}}), &[]);
        let mut cmds = Vec::new();
        app.physics_step(&mut cmds);
        assert!(cmds.is_empty() && app.phys.message.contains("No physics"), "{}", app.phys.message);
        app.settings.physics.enabled = false;
        app.phys.message.clear();
        app.physics_step(&mut cmds);
        assert!(cmds.is_empty() && app.phys.message.is_empty());
    }
}
