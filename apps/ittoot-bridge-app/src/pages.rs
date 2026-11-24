//! The six pages.

use crate::theme::*;
use crate::widgets::*;
use crate::work::{self, Job};
use crate::{App, Page};
use egui::{Align2, Color32, FontId, RichText, Sense, Stroke, Ui};
use ittoot_bridge::area::{AreaKind, AreaRole, Origin};
use ittoot_bridge::connection::{Connection, DeviceKind, LinkState, Protocol};
use ittoot_bridge::infer::{sensor_points, station_points, Confidence};
use ittoot_bridge::intake::{DocKind, ReqState};
use ittoot_bridge::model::{self, ModelPlan};
use ittoot_bridge::table::{Binding, ModelProperty};
use ittoot_bridge::driver::TypeClass;
use ittoot_bridge::table::Agreement;
use std::collections::HashMap;

const KIND_NAMES: [(AreaKind, &str); 5] = [
    (AreaKind::DataBlock, "Data block"),
    (AreaKind::Io, "I/O list"),
    (AreaKind::Folder, "Folder"),
    (AreaKind::File, "File"),
    (AreaKind::Database, "Database"),
];

fn kind_name(k: AreaKind) -> &'static str {
    KIND_NAMES.iter().find(|(x, _)| *x == k).map(|(_, n)| *n).unwrap_or("")
}

fn protocol_name(p: Protocol) -> &'static str {
    match p {
        Protocol::OpcUa => "OPC UA",
        Protocol::S7 => "S7 (PUT/GET)",
        Protocol::ModbusTcp => "Modbus TCP",
        Protocol::Postgres => "PostgreSQL",
        Protocol::SqlServer => "SQL Server",
        Protocol::MySql => "MySQL",
        Protocol::Http => "HTTP",
        Protocol::Simulated => "Simulated",
    }
}

const PROTOCOLS: [Protocol; 8] = [Protocol::OpcUa, Protocol::S7, Protocol::ModbusTcp, Protocol::Postgres, Protocol::SqlServer, Protocol::MySql, Protocol::Http, Protocol::Simulated];

fn device_name(k: DeviceKind) -> &'static str {
    match k {
        DeviceKind::Plc => "PLC",
        DeviceKind::Pc => "PC",
        DeviceKind::Hmi => "HMI",
        DeviceKind::Scada => "SCADA",
        DeviceKind::Database => "Database",
        DeviceKind::Other => "Other",
    }
}

const DEVICES: [DeviceKind; 6] = [DeviceKind::Plc, DeviceKind::Pc, DeviceKind::Hmi, DeviceKind::Scada, DeviceKind::Database, DeviceKind::Other];

enum ModelAct {
    None,
    Select(String),
    Add,
    Remove,
    Start,
    Stop,
    Connect,
    Disconnect,
    Build,
    Export,
}

impl App {
    pub fn draw(&mut self, root: &mut Ui) {
        // --compact: only the live page, no title bar and no navigation (a narrow window to film or to leave on a second screen)
        // (while --go is still starting and building Solidcraft the Model page does that work, so it is shown until then)
        if self.hide_job && self.autorun != 1 && self.autorun != 2 {
            egui::CentralPanel::no_frame().frame(egui::Frame::new().fill(BG).inner_margin(egui::Margin::same(12))).show(root, |ui| self.page_live(ui));
            return;
        }
        egui::Panel::top("top").frame(egui::Frame::new().fill(BG2).inner_margin(egui::Margin::symmetric(14, 8)).stroke(Stroke::new(1.0, LINE))).show(root, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new("ITTOOT").color(ACCENT).strong().size(18.0));
                ui.label(RichText::new("SOLIDCRAFT").strong().size(18.0));
                ui.label(RichText::new("  OT Connection Bridge").color(DIM).size(15.0));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let ready = self.intake.ready();
                    chip(ui, if ready { "documents: ready" } else { "documents: missing" }, if ready { OK } else { WARN });
                    chip(ui, &format!("{} areas", self.settings.areas.iter().filter(|a| a.enabled).count()), INFO);
                    let (t, c) = match &self.link_state {
                        LinkState::Connected => ("link: connected", OK),
                        LinkState::Connecting => ("link: connecting", WARN),
                        LinkState::Disconnected => ("link: off", DIM),
                        LinkState::Failed(_) => ("link: failed", ERROR),
                    };
                    chip(ui, &format!("{t}  ({})", self.link_name()), c);
                });
            });
        });
        egui::Panel::left("nav").exact_size(190.0).resizable(false).frame(egui::Frame::new().fill(BG2).inner_margin(egui::Margin::same(10)).stroke(Stroke::new(1.0, LINE))).show(root, |ui| {
            for (page, name, _) in Page::ALL.iter().copied() {
                let sel = self.page == page;
                let count = match page {
                    Page::Tables => Some(self.tables.len()),
                    Page::Connections => Some(self.settings.connections.len()),
                    Page::Equipment => Some(self.inference.items.len()),
                    Page::Model => self.plan.as_ref().map(|p| p.joints.len()),
                    _ => None,
                };
                let text = match count {
                    Some(n) => format!("{name}   {n}"),
                    None => name.to_string(),
                };
                let b = egui::Button::new(RichText::new(text).size(15.0).color(if sel { Color32::BLACK } else { FG })).fill(if sel { ACCENT } else { Color32::TRANSPARENT }).stroke(Stroke::NONE).min_size(egui::vec2(ui.available_width(), 32.0));
                if ui.add(b).clicked() {
                    self.page = page;
                }
            }
            ui.add_space(16.0);
            ui.label(RichText::new("Rust core: ittoot-bridge\nOPC UA, PostgreSQL, SQL Server, MySQL").color(DIM).size(11.0));
        });
        egui::CentralPanel::no_frame().frame(egui::Frame::new().fill(BG).inner_margin(egui::Margin::same(14))).show(root, |ui| match self.page {
            Page::Settings => self.page_settings(ui),
            Page::Connections => self.page_connections(ui),
            Page::Tables => self.page_tables(ui),
            Page::Equipment => self.page_equipment(ui),
            Page::Model => self.page_model(ui),
            Page::Databases => self.page_databases(ui),
            Page::Documents => self.page_documents(ui),
            Page::IoBuilder => self.page_io(ui),
            Page::Design => self.page_design(ui),
            Page::Engineering => self.page_engineering(ui),
            Page::Hmi => self.page_hmi(ui),
            Page::Physics => self.page_physics(ui),
            Page::Live => self.page_live(ui),
        });
    }

    // ---------------------------------------------------------------- Live
    /// What the machine is doing now, narrow enough to sit beside the model: the axes' positions and every device's colour, state and number.
    fn page_live(&mut self, ui: &mut Ui) {
        ui.horizontal(|ui| {
            ui.label(RichText::new("Live").heading());
            let (t, c) = match &self.link_state {
                LinkState::Connected => ("PLC connected", OK),
                LinkState::Connecting => ("connecting", WARN),
                LinkState::Disconnected => ("off", DIM),
                LinkState::Failed(_) => ("failed", ERROR),
            };
            chip(ui, t, c);
        });
        ui.label(RichText::new(self.link_name()).color(DIM).size(12.0));
        ui.add_space(6.0);
        if let Some(plan) = self.plan.clone() {
            panel(ui, |ui| {
                egui::Grid::new("live-axes").num_columns(2).spacing([18.0, 6.0]).show(ui, |ui| {
                    let at = crate::multi::positions(&plan, &self.bindings, &self.live);
                    for j in &plan.joints {
                        ui.label(RichText::new(&j.name).strong().size(16.0));
                        let v = at.get(&j.axis_index).map(|v| format!("{v:.0} mm")).unwrap_or_else(|| "-".into());
                        ui.label(RichText::new(v).color(INFO).size(16.0));
                        ui.end_row();
                    }
                });
            });
        }
        ui.add_space(6.0);
        // narrow (beside the model): one list; wide (under the model): two lists side by side so it stays short
        let wide = ui.available_width() > 700.0;
        let half = if wide { self.shades.len().div_ceil(2) } else { self.shades.len() };
        panel(ui, |ui| {
            ui.horizontal_top(|ui| {
                for (g, chunk) in self.shades.chunks(half.max(1)).enumerate() {
                    egui::Grid::new(format!("live-devices-{g}")).num_columns(4).spacing([12.0, 5.0]).show(ui, |ui| {
                        for s in chunk {
                            let c = Color32::from_rgb(s.rgb[0], s.rgb[1], s.rgb[2]);
                            let (r, _) = ui.allocate_exact_size(egui::vec2(18.0, 18.0), Sense::hover());
                            ui.painter().circle_filled(r.center(), 7.0, c);
                            ui.painter().circle_stroke(r.center(), 7.0, Stroke::new(1.0, LINE));
                            ui.label(RichText::new(&s.name).strong().size(14.0));
                            ui.label(RichText::new(&s.state).color(c).size(14.0));
                            ui.label(RichText::new(&s.number).color(INFO).size(13.0));
                            ui.end_row();
                        }
                    });
                    ui.add_space(24.0);
                }
            });
        });
    }

    // ---------------------------------------------------------------- Settings
    fn page_settings(&mut self, ui: &mut Ui) {
        ui.label(RichText::new("Settings").heading());
        heading(ui, "Controls projects");
        let mut remove: Option<usize> = None;
        let mut start = false;
        panel(ui, |ui| {
            for (i, p) in self.settings.projects.iter_mut().enumerate() {
                ui.horizontal(|ui| {
                    let w = ui.available_width() - 40.0;
                    ui.add_sized([w, 24.0], egui::TextEdit::singleline(p).hint_text("Path to a TIA project (.ap18), or a folder of exported sources"));
                    if ui.button("\u{00d7}").clicked() {
                        remove = Some(i);
                    }
                });
            }
            ui.horizontal(|ui| {
                if ui.button("+  Add project").clicked() {
                    self.settings.projects.push(String::new());
                }
                if accent_button(ui, "Auto-config").clicked() {
                    start = true;
                }
                ui.label(RichText::new(&self.auto_msg).color(DIM));
            });
            for n in &self.discovery.notes {
                ui.label(RichText::new(format!("! {n}")).color(WARN).size(12.0));
            }
        });
        if let Some(i) = remove {
            self.settings.projects.remove(i);
        }
        if start {
            self.start_auto();
        }

        ui.add_space(10.0);
        ui.horizontal(|ui| {
            for (role, name) in [(AreaRole::Controls, "Controls data areas"), (AreaRole::Management, "Management data areas")] {
                let n = self.settings.areas_of(role).count();
                let sel = self.tab_role == role;
                let b = egui::Button::new(RichText::new(format!("{name}  ({n})")).color(if sel { Color32::BLACK } else { FG })).fill(if sel { ACCENT } else { BG3 });
                if ui.add(b).clicked() {
                    self.tab_role = role;
                }
            }
        });
        ui.label(
            RichText::new(match self.tab_role {
                AreaRole::Controls => "What runs the machine now: live control and status, I/O, drives. These drive the model.",
                AreaRole::Management => "What configures and supervises it: settings, modes, messages, logs, heartbeat.",
            })
            .color(DIM)
            .size(12.0),
        );
        let role = self.tab_role;
        let mut remove: Option<String> = None;
        let mut add = false;
        let sugg = &self.suggestions;
        panel(ui, |ui| {
            egui::ScrollArea::vertical().auto_shrink([false, false]).max_height(ui.available_height() - 40.0).show(ui, |ui| {
                for a in self.settings.areas.iter_mut().filter(|a| a.role == role) {
                    ui.horizontal(|ui| {
                        let before = (a.kind, a.source.clone());
                        egui::ComboBox::from_id_salt(("kind", &a.id)).width(110.0).selected_text(kind_name(a.kind)).show_ui(ui, |ui| {
                            for (k, n) in KIND_NAMES {
                                ui.selectable_value(&mut a.kind, k, n);
                            }
                        });
                        let w = (ui.available_width() - 250.0).max(240.0);
                        let hint = match a.kind {
                            AreaKind::DataBlock => "Search data blocks, or a symbolic name",
                            AreaKind::Io => "Search I/O lists",
                            AreaKind::Folder => "A folder: every data block and I/O list in it is used",
                            _ => "Search or type a path",
                        };
                        search_box(ui, ui.make_persistent_id(("src", &a.id)), &mut a.source, sugg, w, hint);
                        if (a.kind, a.source.clone()) != before {
                            a.origin = Origin::User;
                        }
                        ui.checkbox(&mut a.enabled, "");
                        match a.origin {
                            Origin::Auto => chip(ui, "auto", INFO),
                            Origin::User => chip(ui, "yours", ACCENT),
                        }
                        ui.label(RichText::new(&a.label).color(DIM).size(12.0));
                        if ui.button("\u{00d7}").clicked() {
                            remove = Some(a.id.clone());
                        }
                    });
                }
                if ui.button(RichText::new("+").strong().size(16.0)).on_hover_text("Add a data area").clicked() {
                    add = true;
                }
            });
        });
        if let Some(id) = remove {
            self.settings.remove_area(&id);
        }
        if add {
            self.settings.add_area(role);
        }
    }

    // ------------------------------------------------------------- Connections
    fn page_connections(&mut self, ui: &mut Ui) {
        ui.label(RichText::new("Connections").heading());
        ui.label(RichText::new("Every device the bridge talks to. Passwords are kept in a separate file, not in the settings.").color(DIM));
        ui.add_space(6.0);
        let mut add: Option<Connection> = None;
        ui.horizontal(|ui| {
            ui.label("Add:");
            let n = self.settings.connections.len() + 1;
            let mk = |name: &str, kind, proto, host: &str| Connection::new(format!("c{n}"), name, kind, proto, host);
            if ui.button("Siemens PLC (OPC UA)").clicked() {
                add = Some(mk("New PLC", DeviceKind::Plc, Protocol::OpcUa, "192.168.0.1"));
            }
            if ui.button("PostgreSQL").clicked() {
                add = Some(mk("New PostgreSQL", DeviceKind::Database, Protocol::Postgres, "localhost"));
            }
            if ui.button("SQL Server").clicked() {
                add = Some(mk("New SQL Server", DeviceKind::Database, Protocol::SqlServer, "localhost"));
            }
            if ui.button("MySQL").clicked() {
                add = Some(mk("New MySQL", DeviceKind::Database, Protocol::MySql, "localhost"));
            }
            if ui.button("PC / HMI / other").clicked() {
                add = Some(mk("New device", DeviceKind::Pc, Protocol::Http, ""));
            }
        });
        if let Some(c) = add {
            self.settings.connections.push(c);
            self.sel_conn = self.settings.connections.len() - 1;
        }
        ui.add_space(6.0);
        self.sel_conn = self.sel_conn.min(self.settings.connections.len().saturating_sub(1));

        ui.columns(2, |cols| {
            // list
            let ui = &mut cols[0];
            panel(ui, |ui| {
                ui.set_min_height(ui.available_height() - 6.0);
                for (i, c) in self.settings.connections.iter().enumerate() {
                    let sel = i == self.sel_conn;
                    let (rect, resp) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 46.0), Sense::click());
                    let p = ui.painter();
                    p.rect_filled(rect, 0.0, if sel { BG3 } else { BG2 });
                    p.rect_stroke(rect, 0.0, Stroke::new(1.0, if sel { ACCENT } else { LINE }), egui::StrokeKind::Inside);
                    p.text(rect.left_top() + egui::vec2(10.0, 7.0), Align2::LEFT_TOP, &c.name, FontId::proportional(15.0), FG);
                    let sub = format!("{} \u{00b7} {} \u{00b7} {}", device_name(c.kind), protocol_name(c.protocol), if c.host.is_empty() { "-" } else { &c.host });
                    p.text(rect.left_top() + egui::vec2(10.0, 27.0), Align2::LEFT_TOP, sub, FontId::proportional(12.0), DIM);
                    let (txt, col) = match self.tests.get(&c.id) {
                        Some(Ok(_)) => ("tested ok", OK),
                        Some(Err(_)) => ("test failed", ERROR),
                        None if self.test_jobs.contains_key(&c.id) => ("testing...", WARN),
                        None => ("not tested", DIM),
                    };
                    p.text(rect.right_top() + egui::vec2(-10.0, 8.0), Align2::RIGHT_TOP, txt, FontId::proportional(12.0), col);
                    if self.settings.active_link.as_deref() == Some(c.id.as_str()) {
                        p.text(rect.right_top() + egui::vec2(-10.0, 27.0), Align2::RIGHT_TOP, "feeds the tables", FontId::proportional(12.0), ACCENT);
                    }
                    if resp.clicked() {
                        self.sel_conn = i;
                    }
                    ui.add_space(2.0);
                }
            });
            // editor
            let ui = &mut cols[1];
            let mut remove = false;
            let mut test = false;
            let mut use_it = false;
            let idx = self.sel_conn;
            if let Some(c) = self.settings.connections.get_mut(idx) {
                panel(ui, |ui| {
                    ui.label(RichText::new("Details").strong());
                    egui::Grid::new("conn-form").num_columns(2).spacing([12.0, 8.0]).show(ui, |ui| {
                        ui.label("Name");
                        ui.text_edit_singleline(&mut c.name);
                        ui.end_row();
                        ui.label("Kind");
                        egui::ComboBox::from_id_salt("dev-kind").selected_text(device_name(c.kind)).show_ui(ui, |ui| {
                            for k in DEVICES {
                                ui.selectable_value(&mut c.kind, k, device_name(k));
                            }
                        });
                        ui.end_row();
                        ui.label("Protocol");
                        egui::ComboBox::from_id_salt("dev-proto").selected_text(protocol_name(c.protocol)).show_ui(ui, |ui| {
                            for p in PROTOCOLS {
                                ui.selectable_value(&mut c.protocol, p, protocol_name(p));
                            }
                        });
                        ui.end_row();
                        if c.protocol != Protocol::Simulated {
                            ui.label("Address");
                            ui.text_edit_singleline(&mut c.host);
                            ui.end_row();
                            ui.label("Port");
                            let mut port = c.port.map(|p| p.to_string()).unwrap_or_default();
                            let hint = c.effective_port().map(|p| p.to_string()).unwrap_or_default();
                            ui.add(egui::TextEdit::singleline(&mut port).hint_text(hint));
                            c.port = port.trim().parse().ok();
                            ui.end_row();
                        }
                        if c.protocol == Protocol::S7 {
                            ui.label("Rack / slot");
                            ui.horizontal(|ui| {
                                let (mut r, mut s) = (c.rack.unwrap_or(0) as u32, c.slot.unwrap_or(1) as u32);
                                ui.add(egui::DragValue::new(&mut r).range(0..=7));
                                ui.add(egui::DragValue::new(&mut s).range(0..=31));
                                c.rack = Some(r as u8);
                                c.slot = Some(s as u8);
                            });
                            ui.end_row();
                        }
                        if c.is_database() {
                            ui.label("Database");
                            ui.text_edit_singleline(&mut c.extra);
                            ui.end_row();
                        }
                        if c.protocol != Protocol::Simulated {
                            ui.label("User");
                            ui.add(egui::TextEdit::singleline(&mut c.username).hint_text(if c.protocol == Protocol::SqlServer { "empty = sign in as the Windows user" } else { "empty = anonymous" }));
                            ui.end_row();
                            ui.label("Password");
                            let mut pw = self.secrets.get(&c.id).unwrap_or("").to_string();
                            if ui.add(egui::TextEdit::singleline(&mut pw).password(true)).changed() {
                                self.secrets.set(&c.id, &pw);
                            }
                            ui.end_row();
                        }
                        ui.label("Notes");
                        ui.text_edit_singleline(&mut c.notes);
                        ui.end_row();
                    });
                    for p in c.problems() {
                        ui.label(RichText::new(format!("! {p}")).color(WARN).size(12.0));
                    }
                    ui.add_space(6.0);
                    ui.horizontal(|ui| {
                        if accent_button(ui, "Test connection").clicked() {
                            test = true;
                        }
                        if matches!(c.protocol, Protocol::OpcUa | Protocol::Simulated) && ui.button("Use for the tables").clicked() {
                            use_it = true;
                        }
                        if ui.button("Remove").clicked() {
                            remove = true;
                        }
                    });
                    match self.tests.get(&c.id) {
                        Some(Ok(m)) => ui.label(RichText::new(m).color(OK)),
                        Some(Err(m)) => ui.label(RichText::new(m).color(ERROR)),
                        None => ui.label(""),
                    };
                });
                if test {
                    let (cc, pw, pki) = (c.clone(), self.secrets.get(&c.id).unwrap_or("").to_string(), self.pki.clone());
                    self.tests.remove(&c.id);
                    self.test_jobs.insert(c.id.clone(), Job::spawn(move || work::test_connection(&cc, &pw, &pki)));
                }
                if use_it {
                    let id = c.id.clone();
                    self.settings.active_link = if c.protocol == Protocol::Simulated { None } else { Some(id) };
                    self.pollers.clear();
                    self.live_by.clear();
                }
            }
            if remove {
                let id = self.settings.connections.remove(idx).id;
                self.secrets.set(&id, "");
                if self.settings.active_link.as_deref() == Some(id.as_str()) {
                    self.settings.active_link = None;
                }
            }
        });
    }

    // ------------------------------------------------------------------ Tables
    fn page_tables(&mut self, ui: &mut Ui) {
        if let Some(t) = self.pending_table.clone()
            && let Some(i) = self.tables.iter().position(|x| x.title == t)
        {
            self.active_table = i;
            self.pending_table = None;
        }
        ui.horizontal(|ui| {
            ui.label(RichText::new("Tables").heading());
            ui.label(RichText::new("One per data area, named from what is in it. Values come from:").color(DIM));
            ui.label(RichText::new(self.link_name()).color(ACCENT));
        });
        if self.tables.is_empty() {
            ui.add_space(20.0);
            let msg = if self.build_job.is_some() { "Reading the data areas..." } else { "No tables yet. In Settings, give a project and press Auto-config, or add data areas with +." };
            ui.label(RichText::new(msg).color(DIM));
            return;
        }
        ui.add_space(4.0);
        for (role, name) in [(AreaRole::Controls, "Controls"), (AreaRole::Management, "Management")] {
            ui.horizontal_wrapped(|ui| {
                ui.set_max_width(ui.available_width());
                ui.label(RichText::new(format!("{name}:")).color(DIM).size(12.0));
                let mut pick = None;
                for (i, t) in self.tables.iter().enumerate().filter(|(_, t)| t.role == role) {
                    let sel = i == self.active_table;
                    let b = egui::Button::new(RichText::new(&t.title).size(13.0).color(if sel { Color32::BLACK } else { FG })).fill(if sel { ACCENT } else { BG3 }).min_size(egui::vec2(0.0, 22.0));
                    if ui.add(b).clicked() {
                        pick = Some(i);
                    }
                }
                if let Some(i) = pick {
                    self.active_table = i;
                }
            });
        }
        ui.add_space(6.0);
        let link_up = matches!(self.link_state, LinkState::Connected);
        let idx = self.active_table.min(self.tables.len() - 1);
        let model_props: HashMap<String, ModelProperty> = self.plan.iter().flat_map(|p| p.joints.iter()).map(|j| (format!("{}.position", j.name), ModelProperty { class: TypeClass::Real })).collect();
        for r in &mut self.tables[idx].rows {
            r.binding = self.bindings.iter().find(|b| b.key == r.key).map(|b| Binding { object: b.joint.clone(), property: "position".into() });
        }
        self.tables[idx].cross_reference(&self.live, &model_props, link_up);
        let table = &self.tables[idx];
        ui.horizontal(|ui| {
            ui.label(RichText::new(&table.title).strong().size(16.0));
            let s = table.summary();
            for (k, col) in [("ok", OK), ("unbound", DIM), ("missing in model", WARN), ("missing in PLC", ERROR), ("type mismatch", ERROR), ("not read", DIM)] {
                if let Some(n) = s.get(k) {
                    chip(ui, &format!("{n} {k}"), col);
                }
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.add_sized([260.0, 24.0], egui::TextEdit::singleline(&mut self.filter).hint_text("Filter rows"));
            });
        });
        if let Some(n) = &table.note {
            ui.label(RichText::new(n).color(WARN).size(12.0));
        }
        let f = self.filter.trim().to_lowercase();
        let rows: Vec<usize> = table
            .rows
            .iter()
            .enumerate()
            .filter(|(_, r)| f.is_empty() || r.label.to_lowercase().contains(&f) || r.comment.to_lowercase().contains(&f) || r.key.to_lowercase().contains(&f))
            .map(|(i, _)| i)
            .collect();
        const W: [f32; 5] = [360.0, 110.0, 150.0, 190.0, 130.0];
        ui.add_space(4.0);
        egui::Frame::new().fill(BG2).stroke(Stroke::new(1.0, LINE)).show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 0.0;
                for (w, h) in W.iter().zip(["Signal", "Type", "PLC value", "Model", "Status"]) {
                    left_cell(ui, *w, 24.0, h, DIM, true);
                }
                ui.label(RichText::new("Comment").color(DIM).size(12.0).strong());
            });
            ui.separator();
            egui::ScrollArea::vertical().auto_shrink([false, false]).show_rows(ui, 22.0, rows.len(), |ui, range| {
                for k in range {
                    let Some(r) = rows.get(k).and_then(|i| table.rows.get(*i)) else { continue };
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 0.0;
                        let cell = |ui: &mut Ui, w: f32, t: &str, c: Color32| left_cell(ui, w, 22.0, t, c, false);
                        cell(ui, W[0], &r.label, FG);
                        cell(ui, W[1], &r.ty, DIM);
                        let v = r.plc.as_ref().map(|v| v.show()).unwrap_or_else(|| "\u{2014}".into());
                        cell(ui, W[2], &v, if r.plc.is_some() { INFO } else { DIM });
                        cell(ui, W[3], r.binding.as_ref().map(|b| format!("{}.{}", b.object, b.property)).as_deref().unwrap_or("not on the model"), DIM);
                        let (t, c) = match r.status {
                            Agreement::Ok => ("ok", OK),
                            Agreement::Unbound => ("not on model yet", DIM),
                            Agreement::MissingInModel => ("missing in model", WARN),
                            Agreement::MissingInPlc => ("missing in PLC", ERROR),
                            Agreement::TypeMismatch => ("type mismatch", ERROR),
                            Agreement::NotRead => ("not read", DIM),
                        };
                        cell(ui, W[4], t, c);
                        ui.add(egui::Label::new(RichText::new(&r.comment).color(DIM).size(12.0)).truncate());
                    });
                }
            });
        });
    }

    // --------------------------------------------------------------- Equipment
    fn page_equipment(&mut self, ui: &mut Ui) {
        ui.horizontal(|ui| {
            ui.label(RichText::new("Equipment").heading());
            ui.label(RichText::new("Sizes and positions worked out from the PLC data, where no FDS says. Change any number: yours always wins.").color(DIM));
        });
        if self.inference.items.is_empty() {
            ui.add_space(20.0);
            ui.label(RichText::new("Nothing to infer yet: the data areas hold no limit-sensor or station rows with start values.").color(DIM));
            return;
        }
        for w in &self.inference.warnings {
            ui.label(RichText::new(format!("! {w}")).color(WARN));
        }
        let len = |k: &str| self.inference.get(k).map(|i| i.value);
        let (lx, ly, lz) = (len("axis0.length"), len("axis1.length"), len("axis2.length"));
        let sensors = sensor_points(&self.records);
        let stations = station_points(&self.records);
        ui.add_space(6.0);
        let h = 250.0;
        let w = (ui.available_width() - 12.0) / 2.0;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 12.0;
            // top view: X across, Y down the page
            diagram(ui, w, h, "Top view  (X across, Y up the page)", lx, ly, |p, map| {
                for s in sensors.iter().filter(|s| s.axis == 0) {
                    let (x, y) = map(s.pos, 0.0);
                    tick(p, x, y, true, s.high_end);
                }
                for s in sensors.iter().filter(|s| s.axis == 1) {
                    let (x, y) = map(0.0, s.pos);
                    tick(p, x, y, false, s.high_end);
                }
                for (n, st) in stations.iter().enumerate() {
                    if let Some(y) = st.y {
                        let (x, y) = map(st.x, y);
                        let half = 5.0;
                        p.rect_filled(egui::Rect::from_center_size(egui::pos2(x, y), egui::vec2(2.0 * half, 2.0 * half)), 0.0, ACCENT);
                        p.text(egui::pos2(x, y - 9.0), Align2::CENTER_BOTTOM, format!("{}", n + 1), FontId::proportional(11.0), FG);
                    }
                }
            }, true);
            // side view: X across, Z down
            diagram(ui, w, h, "Side view  (X across, Z down)", lx, lz, |p, map| {
                for s in sensors.iter().filter(|s| s.axis == 0) {
                    let (x, y) = map(s.pos, 0.0);
                    tick(p, x, y, true, s.high_end);
                }
                for s in sensors.iter().filter(|s| s.axis == 2) {
                    let (x, y) = map(0.0, s.pos);
                    tick(p, x, y, false, s.high_end);
                }
                for st in &stations {
                    if let (Some(up), Some(down)) = (st.z_up, st.z_down) {
                        let (x0, y0) = map(st.x, up);
                        let (_, y1) = map(st.x, down);
                        p.line_segment([egui::pos2(x0, y0), egui::pos2(x0, y1)], Stroke::new(2.0, ACCENT));
                        p.rect_filled(egui::Rect::from_center_size(egui::pos2(x0, y1), egui::vec2(8.0, 4.0)), 0.0, ACCENT);
                    }
                }
            }, false);
        });
        ui.add_space(8.0);
        let mut set: Option<(String, Option<f64>)> = None;
        panel(ui, |ui| {
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                egui::Grid::new("infer").num_columns(6).spacing([14.0, 7.0]).striped(false).show(ui, |ui| {
                    for h in ["Subject", "Quantity", "Value", "Sure?", "How it was worked out", ""] {
                        ui.label(RichText::new(h).color(DIM).size(12.0).strong());
                    }
                    ui.end_row();
                    for it in &self.inference.items {
                        ui.label(&it.subject);
                        ui.label(&it.quantity);
                        let mut v = it.value;
                        let unit = if it.unit.is_empty() { String::new() } else { format!(" {}", it.unit) };
                        if ui.add(egui::DragValue::new(&mut v).speed(10.0).suffix(unit)).changed() {
                            set = Some((it.key.clone(), Some(v)));
                        }
                        let (t, c) = match it.confidence {
                            Confidence::High => ("high", OK),
                            Confidence::Medium => ("medium", WARN),
                            Confidence::Low => ("low", ERROR),
                        };
                        chip(ui, if it.overridden { "yours" } else { t }, if it.overridden { ACCENT } else { c });
                        let r = ui.scope(|ui| {
                            ui.set_max_width(620.0);
                            ui.add(egui::Label::new(RichText::new(&it.method).color(DIM).size(12.0)).sense(Sense::hover()))
                        }).inner;
                        r.on_hover_ui(|ui| {
                            ui.label(RichText::new("Evidence").strong());
                            for e in &it.evidence {
                                ui.label(format!("{}: {}", e.source, e.detail));
                            }
                        });
                        if it.overridden && ui.button("\u{21ba}").on_hover_text("Go back to the worked-out value").clicked() {
                            set = Some((it.key.clone(), None));
                        } else {
                            ui.label("");
                        }
                        ui.end_row();
                    }
                });
            });
        });
        if let Some((k, v)) = set {
            match v {
                Some(v) => self.settings.overrides.insert(k, v),
                None => self.settings.overrides.remove(&k),
            };
        }
    }

    // ------------------------------------------------------------------- Model
    fn page_model(&mut self, ui: &mut Ui) {
        ui.horizontal(|ui| {
            ui.label(RichText::new("Model").heading());
            ui.label(RichText::new("The machine as parts and joints, built from the sizes in Equipment. Each joint follows a PLC signal.").color(DIM));
        });
        if !self.hide_job {
            self.job_panel(ui);
            ui.add_space(6.0);
        }
        let Some(plan) = self.plan.clone() else {
            ui.add_space(20.0);
            ui.label(RichText::new("No model yet: the limit sensors give no X and Y lengths. See Equipment.").color(DIM));
            return;
        };
        ui.add_space(4.0);
        if !self.roles.is_empty() {
            let names: Vec<String> = self.roles.iter().map(|(r, l)| format!("{r} = {}", l.iter().map(|c| c.to_string()).collect::<Vec<_>>().join(", "))).collect();
            ui.label(RichText::new(format!("Part names from the documentation ({}): {}", short(&self.roles_from), names.join("; "))).color(DIM).size(12.0));
        }
        for n in &plan.notes {
            ui.label(RichText::new(n).color(DIM).size(12.0));
        }
        ui.add_space(6.0);

        // joints and their signals
        let mut set: Vec<(String, String)> = Vec::new();
        let simulated = self.settings.active_link.is_none();
        let sugg = &self.signal_choices;
        panel(ui, |ui| {
            egui::Grid::new("joints").num_columns(6).spacing([14.0, 8.0]).show(ui, |ui| {
                for h in ["Joint", "Travel", "PLC signal (actual position)", "", "Value", "Pose (simulated)"] {
                    ui.label(RichText::new(h).color(DIM).size(12.0).strong());
                }
                ui.end_row();
                for j in &plan.joints {
                    ui.label(RichText::new(&j.name).strong());
                    ui.label(RichText::new(format!("0 to {:.0} mm", j.limits.1)).color(DIM));
                    let eff = self.bindings.iter().find(|b| b.joint == j.name);
                    let mut text = eff.map(|b| b.key.clone()).unwrap_or_default();
                    if search_box(ui, ui.make_persistent_id(("sig", &j.name)), &mut text, sugg, 470.0, "Search a signal, or leave empty for none") {
                        set.push((j.name.clone(), text.clone()));
                    }
                    match eff {
                        Some(b) if b.auto => chip(ui, "auto", INFO),
                        Some(_) => chip(ui, "yours", ACCENT),
                        None => chip(ui, "no signal", WARN),
                    }
                    let v = eff.and_then(|b| self.live.get(&b.key)).map(|v| v.show()).unwrap_or_else(|| "\u{2014}".into());
                    ui.label(RichText::new(v).color(INFO));
                    if simulated {
                        let rest = ittoot_bridge::devices::rest_position(&plan, j.axis_index);
                        let e = self.pose.entry(j.name.clone()).or_insert(rest);
                        ui.add(egui::Slider::new(e, j.limits.0..=j.limits.1).show_value(true));
                    } else {
                        ui.label(RichText::new("the PLC drives it").color(DIM).size(12.0));
                    }
                    ui.end_row();
                }
            });
        });
        for (joint, key) in set {
            self.settings.bindings.insert(joint, key);
        }

        // the devices: what each motor, sensor and peripheral shows, with its colour and a number
        ui.add_space(8.0);
        let colours: HashMap<String, Color32> = ittoot_bridge::devices::body_colours(&plan, &self.shades).into_iter().map(|(n, c)| (n, Color32::from_rgb(c[0], c[1], c[2]))).collect();
        let simulated_now = self.settings.active_link.is_none();
        panel(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new("Devices").strong());
                ui.label(RichText::new("Motors red = off or at a limit, green = moving, blue = ready. Limit sensors white = clear, red = active. Station sensors white = away, green = at position. Peripherals white = idle, green = active, red = fault.").color(DIM).size(12.0));
            });
            if simulated_now {
                ui.horizontal(|ui| {
                    ui.checkbox(&mut self.demo_on, "Run the demo cycle (no PLC connected: a simulated transport visits each station, lowers the hoist, then drives into the high limit)");
                });
            }
            ui.horizontal_wrapped(|ui| {
                if !self.design_msg.is_empty() {
                    ui.label(RichText::new(&self.design_msg).color(DIM).size(12.0));
                }
            });
            ui.add_space(4.0);
            egui::Grid::new("devices").num_columns(5).spacing([14.0, 4.0]).show(ui, |ui| {
                for h in ["", "Device", "Kind", "State", "Number"] {
                    ui.label(RichText::new(h).color(DIM).size(12.0).strong());
                }
                ui.end_row();
                for s in &self.shades {
                    let c = Color32::from_rgb(s.rgb[0], s.rgb[1], s.rgb[2]);
                    let (r, _) = ui.allocate_exact_size(egui::vec2(18.0, 18.0), Sense::hover());
                    ui.painter().circle_filled(r.center(), 7.0, c);
                    ui.painter().circle_stroke(r.center(), 7.0, Stroke::new(1.0, LINE));
                    ui.label(RichText::new(&s.name).strong());
                    ui.label(RichText::new(s.kind.label()).color(DIM));
                    ui.label(RichText::new(&s.state).color(c));
                    ui.label(RichText::new(&s.number).color(INFO));
                    ui.end_row();
                }
            });
        });

        // the live picture
        ui.add_space(8.0);
        let h = 230.0;
        let w = (ui.available_width() - 12.0) / 2.0;
        let pose: HashMap<String, f64> = plan.joints.iter().map(|j| (j.name.clone(), self.joint_value(&j.name).unwrap_or(0.0).clamp(j.limits.0, j.limits.1))).collect();
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 12.0;
            scene(ui, w, h, &plan, &pose, true, &colours);
            scene(ui, w, h, &plan, &pose, false, &colours);
        });

        // Solidcraft: one panel per model target, with + for another
        ui.add_space(8.0);
        self.ensure_models();
        let ai = self.active_model_index();
        let orig = self.settings.models[ai].clone();
        let mut m = orig.clone();
        let mut tok = self.secrets.get(&format!("model:{}", m.id)).unwrap_or("").to_string();
        let tok0 = tok.clone();
        let (linked, starting, running, status, mut push) = match self.runs.get(&m.id) {
            Some(r) => (r.link.is_some(), r.wait.is_some(), r.child.is_some(), r.status.clone(), r.push_live),
            None => (false, false, false, String::new(), false),
        };
        let exe = ittoot_bridge::target::find_exe(&m.exe);
        let names: Vec<(String, String)> = self.settings.models.iter().map(|x| (x.id.clone(), x.name.clone())).collect();
        let many = names.len() > 1;
        let mut act = ModelAct::None;
        match self.autorun {
            1 if !linked && !starting => {
                act = ModelAct::Start;
                self.autorun = 2;
            }
            2 if linked => {
                act = ModelAct::Build;
                push = true;
                self.autorun = 3;
            }
            _ => {}
        }
        panel(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new("Models").strong());
                for (id, name) in &names {
                    if ui.selectable_label(*id == m.id, name).clicked() {
                        act = ModelAct::Select(id.clone());
                    }
                }
                if ui.button("+").on_hover_text("Add another model: its own Solidcraft, port, token and deployment record").clicked() {
                    act = ModelAct::Add;
                }
            });
            ui.add_space(6.0);
            ui.horizontal_wrapped(|ui| {
                if linked {
                    chip(ui, "linked", OK);
                    if ui.button("Build the model in Solidcraft").clicked() {
                        act = ModelAct::Build;
                    }
                    ui.checkbox(&mut push, "Move the joints with the machine");
                    if ui.button("Disconnect").clicked() {
                        act = ModelAct::Disconnect;
                    }
                    if running && ui.button("Close Solidcraft").clicked() {
                        act = ModelAct::Stop;
                    }
                } else if starting {
                    chip(ui, "starting", WARN);
                    ui.label(RichText::new("waiting for Solidcraft to open its control channel...").color(DIM));
                } else {
                    if m.is_local() {
                        if start_button(ui, "START SOLIDCRAFT").on_hover_text("Opens Solidcraft on this PC with its control channel, then connects the bridge to it").clicked() {
                            act = ModelAct::Start;
                        }
                        if exe.is_none() {
                            chip(ui, "solidcraft.exe not found: set its path below, or run scripts\\start-bridge.ps1", WARN);
                        }
                    }
                    if ui.button(if m.is_local() { "Connect to one already open" } else { "Connect" }).clicked() {
                        act = ModelAct::Connect;
                    }
                }
                if ui.button("Save deployment record").clicked() {
                    act = ModelAct::Export;
                }
            });
            ui.add_space(6.0);
            egui::CollapsingHeader::new(RichText::new("Details of this model").color(DIM)).id_salt(("model-details", m.id.clone())).default_open(true).show(ui, |ui| {
                egui::Grid::new(("model-form", m.id.clone())).num_columns(2).spacing([12.0, 6.0]).show(ui, |ui| {
                    ui.label("Name");
                    ui.add_sized([300.0, 24.0], egui::TextEdit::singleline(&mut m.name));
                    ui.end_row();
                    ui.label("Address");
                    ui.add_sized([300.0, 24.0], egui::TextEdit::singleline(&mut m.host).hint_text("127.0.0.1 starts it here; another PC's address connects to it"));
                    ui.end_row();
                    ui.label("Control port");
                    let mut port = m.port.to_string();
                    ui.add_sized([90.0, 24.0], egui::TextEdit::singleline(&mut port));
                    m.port = port.trim().parse().unwrap_or(m.port);
                    ui.end_row();
                    ui.label("Token");
                    ui.add_sized([300.0, 24.0], egui::TextEdit::singleline(&mut tok).password(true).hint_text("made for you when the bridge starts Solidcraft"));
                    ui.end_row();
                    if m.is_local() {
                        ui.label("solidcraft.exe");
                        ui.add_sized([520.0, 24.0], egui::TextEdit::singleline(&mut m.exe).hint_text(exe.as_ref().map(|p| p.display().to_string()).unwrap_or_else(|| "path to solidcraft.exe".into())));
                        ui.end_row();
                        ui.label("Design to open");
                        ui.add_sized([520.0, 24.0], egui::TextEdit::singleline(&mut m.design).hint_text("optional .sc file"));
                        ui.end_row();
                        ui.label("Software render");
                        ui.checkbox(&mut m.cpu_render, "for a PC whose graphics card cannot be read back");
                        ui.end_row();
                    }
                    ui.label("Model project");
                    let shown = if m.model_project.is_empty() { "the one the page works on".to_string() } else { m.model_project.clone() };
                    egui::ComboBox::from_id_salt(("model-of", m.id.clone())).width(320.0).selected_text(shown).show_ui(ui, |ui| {
                        if ui.selectable_label(m.model_project.is_empty(), "the one the page works on").clicked() {
                            m.model_project.clear();
                        }
                        for mp in &self.model_projects {
                            if ui.selectable_label(m.model_project == mp.name, &mp.name).clicked() {
                                m.model_project = mp.name.clone();
                            }
                        }
                    });
                    ui.end_row();
                    ui.label("Notes");
                    ui.add_sized([520.0, 24.0], egui::TextEdit::singleline(&mut m.notes));
                    ui.end_row();
                });
                for p in m.problems() {
                    ui.label(RichText::new(format!("! {p}")).color(WARN).size(12.0));
                }
                if many && ui.button("Remove this model").clicked() {
                    act = ModelAct::Remove;
                }
            });
            if !self.export_msg.is_empty() {
                ui.label(RichText::new(&self.export_msg).color(DIM).size(12.0));
            }
            if !status.is_empty() {
                ui.label(RichText::new(&status).color(INFO).size(12.0));
            }
        });
        if m != orig {
            self.settings.models[ai] = m.clone();
        }
        if tok != tok0 {
            self.secrets.set(&format!("model:{}", m.id), tok.trim());
        }
        if let Some(r) = self.runs.get_mut(&m.id) {
            r.push_live = push;
        }
        match act {
            ModelAct::None => {}
            ModelAct::Select(id) => self.settings.active_model = Some(id),
            ModelAct::Add => {
                let id = ittoot_bridge::target::next_id(&self.settings.models);
                let port = ittoot_bridge::target::next_free_port(&self.settings.models);
                let n = self.settings.models.len() + 1;
                self.settings.models.push(ittoot_bridge::target::ModelTarget::new(id.clone(), format!("Model {n}"), port));
                self.settings.active_model = Some(id);
            }
            ModelAct::Remove => {
                self.runs.remove(&m.id);
                self.secrets.set(&format!("model:{}", m.id), "");
                self.settings.models.remove(ai);
                self.settings.active_model = None;
                self.ensure_models();
            }
            ModelAct::Export => self.export_msg = self.save_deployment(&plan),
            ModelAct::Connect => {
                let dir = self.deployment_dir();
                let r = self.runs.entry(m.id.clone()).or_default();
                let link = work::SolidLink::spawn(m.address(), tok.trim().to_string(), dir);
                link.send(work::PushMsg::Probe);
                r.link = Some(link);
                r.joints = None;
                r.status = format!("connecting to {}...", m.address());
            }
            ModelAct::Disconnect => {
                if let Some(r) = self.runs.get_mut(&m.id) {
                    r.link = None;
                    r.push_live = false;
                    r.status = "disconnected".into();
                }
            }
            ModelAct::Stop => {
                if let Some(r) = self.runs.get_mut(&m.id) {
                    r.link = None;
                    r.push_live = false;
                    if let Some(c) = r.child.as_mut() {
                        let _ = c.kill();
                    }
                }
            }
            ModelAct::Start => {
                let msg = match &exe {
                    None => Err("solidcraft.exe was not found: put its path in Details, or run scripts\\start-bridge.ps1".to_string()),
                    Some(path) => {
                        let token = if tok.trim().len() >= 32 { tok.trim().to_string() } else { ittoot_bridge::target::new_token() };
                        self.secrets.set(&format!("model:{}", m.id), &token);
                        let mut cmd = std::process::Command::new(path);
                        cmd.args(m.launch_args()).envs(m.launch_env(&token));
                        match cmd.spawn() {
                            Ok(child) => {
                                let addr = m.address();
                                let r = self.runs.entry(m.id.clone()).or_default();
                                r.child = Some(child);
                                r.wait = Some(Job::spawn(move || work::wait_for_port(&addr, 60)));
                                Ok(format!("started {} on port {}", path.display(), m.port))
                            }
                            Err(e) => Err(format!("could not start {}: {e}", path.display())),
                        }
                    }
                };
                let r = self.runs.entry(m.id.clone()).or_default();
                r.status = msg.unwrap_or_else(|e| e);
            }
            ModelAct::Build => {
                // the record first, so the journal of this build sits beside what it was built from
                self.export_msg = self.save_deployment(&plan);
                let script = self.build_for(&m.id);
                let save = self.solid_path(&m.id);
                let unsaved = save.is_none();
                if let Some(r) = self.runs.get_mut(&m.id) {
                    if let Some(s) = &r.link {
                        s.send(work::PushMsg::Build(script));
                        if let Some(p) = save {
                            s.send(work::PushMsg::SaveAs(p));
                        }
                    }
                    r.status = if unsaved { "building... (not saved: give the job a number first, then build again)".into() } else { "building...".into() };
                }
            }
        }
        self.save_models();
    }

    /// Keep the settings and the model tokens on disk as they change (written atomically).
    fn save_models(&mut self) {
        let key = serde_json::to_string(&(&self.settings, &self.secrets)).unwrap_or_default();
        if key != self.models_saved {
            self.models_saved = key;
            self.save();
        }
    }

    /// Where this project's deployment record lives: one folder per project, in the bridge's data folder.
    pub fn deployment_dir(&self) -> std::path::PathBuf {
        let id = self.settings.models.get(self.active_model_index()).map(|m| m.id.clone()).unwrap_or_default();
        self.deployment_dir_for(&id)
    }

    /// The same, for one model: the first keeps the plain folder, others add their id, so models never share a journal.
    pub fn deployment_dir_for(&self, model: &str) -> std::path::PathBuf {
        let first = self.settings.models.first().is_none_or(|m| m.id == model);
        let own = self.settings.models.iter().find(|m| m.id == model).map(|m| m.model_project.clone()).filter(|p| !p.is_empty());
        let name = self
            .settings
            .projects
            .first()
            .and_then(|p| std::path::Path::new(p).parent().and_then(|d| d.file_name()).map(|n| n.to_string_lossy().into_owned()))
            .unwrap_or_else(|| "model".to_string());
        let safe: String = name.chars().map(|c| if c.is_alphanumeric() || c == '-' || c == '_' || c == '.' { c } else { '_' }).collect();
        let safe = if first { safe } else { format!("{safe}__{}", model.replace(|c: char| !c.is_alphanumeric(), "_")) };
        // in the job's folder when there is a job number: Models\<model project>\deployment (another Solidcraft adds its id)
        if let Some(d) = ittoot_bridge::job::deployment_dir(&self.settings.job, own.as_deref().or(self.settings.model_project.as_deref()).unwrap_or("Default")) {
            return if first { d } else { d.join(model.replace(|c: char| !c.is_alphanumeric(), "_")) };
        }
        crate::data_dir().join("deployments").join(safe)
    }

    /// Write the deployment record: what was decided, the script, how to rebuild it.
    pub fn save_deployment(&mut self, plan: &ModelPlan) -> String {
        let dir = self.deployment_dir();
        let d = ittoot_bridge::deploy::Deployment::from_state(&ittoot_bridge::deploy::Inputs {
            projects: &self.settings.projects,
            document_folders: &self.settings.document_folders,
            roles_from: &self.roles_from,
            roles: &self.roles,
            inference: &self.inference,
            plan,
            bindings: &self.bindings,
            solidcraft_version: env!("CARGO_PKG_VERSION"),
            job: &self.settings.job.title(),
        });
        match d.save(&dir) {
            Ok(_) => format!("Deployment record saved in {}: deployment.json, model-script.json, REBUILD.txt (journal.jsonl grows as the model is built in Solidcraft).", dir.display()),
            Err(e) => format!("Could not save the deployment record: {e}"),
        }
    }

    /// The joint's value now: from its signal, else the simulated pose.
    pub fn joint_value(&self, joint: &str) -> Option<f64> {
        let plan = self.plan.as_ref()?;
        let eff = self.bindings.iter().find(|b| b.joint == joint)?;
        let drive = model::drive_commands(plan, std::slice::from_ref(eff), &self.live);
        drive.first().and_then(|c| c["params"]["value"].as_f64())
    }

    // --------------------------------------------------------------- Databases
    fn page_databases(&mut self, ui: &mut Ui) {
        ui.label(RichText::new("Databases").heading());
        ui.label(RichText::new("PostgreSQL, SQL Server and MySQL, read only. Tables are listed from the server and shown a page at a time.").color(DIM));
        let dbs: Vec<(String, String)> = self.settings.connections.iter().filter(|c| c.is_database()).map(|c| (c.id.clone(), c.name.clone())).collect();
        if dbs.is_empty() {
            ui.add_space(16.0);
            ui.label(RichText::new("No database connections yet. Add one on the Connections page.").color(DIM));
            return;
        }
        if self.db.conn.is_none() {
            self.db.conn = dbs.first().map(|d| d.0.clone());
        }
        let mut load = false;
        ui.horizontal(|ui| {
            let cur = dbs.iter().find(|d| Some(&d.0) == self.db.conn.as_ref()).map(|d| d.1.clone()).unwrap_or_default();
            egui::ComboBox::from_id_salt("dbconn").width(260.0).selected_text(cur).show_ui(ui, |ui| {
                for (id, name) in &dbs {
                    ui.selectable_value(&mut self.db.conn, Some(id.clone()), name);
                }
            });
            if accent_button(ui, "Load tables").clicked() {
                load = true;
            }
            ui.label(RichText::new(&self.db.message).color(DIM));
        });
        let conn = self.db.conn.as_ref().and_then(|id| self.settings.connections.iter().find(|c| &c.id == id)).cloned();
        let cfg = conn.as_ref().and_then(|c| work::db_config(c, self.secrets.get(&c.id).unwrap_or("")));
        if load && let Some(cfg) = cfg.clone() {
            self.db.message = "Connecting...".into();
            self.db.list_job = Some(Job::spawn(move || work::db_tables(&cfg)));
        }
        ui.add_space(6.0);
        let mut pick: Option<usize> = None;
        ui.columns(2, |cols| {
            let ui = &mut cols[0];
            panel(ui, |ui| {
                ui.set_min_height(ui.available_height() - 6.0);
                egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                    for (i, t) in self.db.tables.iter().enumerate() {
                        if ui.selectable_label(self.db.selected == Some(i), t.display()).clicked() {
                            pick = Some(i);
                        }
                    }
                    if self.db.tables.is_empty() {
                        ui.label(RichText::new("Press Load tables.").color(DIM));
                    }
                });
            });
            let ui = &mut cols[1];
            panel(ui, |ui| {
                ui.set_min_height(ui.available_height() - 6.0);
                egui::ScrollArea::both().auto_shrink([false, false]).show(ui, |ui| {
                    if !self.db.grid.columns.is_empty() {
                        egui::Grid::new("dbgrid").striped(true).spacing([16.0, 4.0]).show(ui, |ui| {
                            for c in &self.db.grid.columns {
                                ui.label(RichText::new(c).color(DIM).size(12.0).strong());
                            }
                            ui.end_row();
                            for r in &self.db.grid.rows {
                                for v in r {
                                    ui.label(v);
                                }
                                ui.end_row();
                            }
                        });
                    }
                });
            });
        });
        if let (Some(i), Some(cfg)) = (pick, cfg) {
            self.db.selected = Some(i);
            if let Some(t) = self.db.tables.get(i).cloned() {
                self.db.message = "Reading...".into();
                self.db.page_job = Some(Job::spawn(move || work::db_page(&cfg, &t, 200, 0)));
            }
        }
    }

    // --------------------------------------------------------------- Documents
    fn page_documents(&mut self, ui: &mut Ui) {
        ui.label(RichText::new("Documents").heading());
        ui.label(RichText::new("Folders holding the drawings, schematics, CAD models, FDS, DDS, briefs, photos and code. The status shows what a model still needs.").color(DIM));
        ui.add_space(6.0);
        let mut remove = None;
        panel(ui, |ui| {
            for (i, f) in self.settings.document_folders.iter_mut().enumerate() {
                ui.horizontal(|ui| {
                    let w = ui.available_width() - 40.0;
                    ui.add_sized([w, 24.0], egui::TextEdit::singleline(f).hint_text("A folder or a file"));
                    if ui.button("\u{00d7}").clicked() {
                        remove = Some(i);
                    }
                });
            }
            if ui.button("+  Add folder").clicked() {
                self.settings.document_folders.push(String::new());
            }
        });
        if let Some(i) = remove {
            self.settings.document_folders.remove(i);
        }
        ui.add_space(8.0);
        heading(ui, "What the model needs");
        panel(ui, |ui| {
            egui::Grid::new("reqs").num_columns(4).spacing([14.0, 8.0]).show(ui, |ui| {
                for s in self.intake.status() {
                    let (t, c) = match s.state {
                        ReqState::Met => ("found", OK),
                        ReqState::MissingRequired => ("missing", ERROR),
                        ReqState::MissingOptional => ("optional", DIM),
                    };
                    chip(ui, t, c);
                    ui.label(RichText::new(s.requirement.label).strong());
                    ui.label(RichText::new(format!("{} file{}", s.found, if s.found == 1 { "" } else { "s" })).color(DIM));
                    ui.label(RichText::new(s.requirement.why).color(DIM).size(12.0));
                    ui.end_row();
                }
            });
        });
        ui.add_space(8.0);
        heading(ui, "Files");
        let mut change: Option<(std::path::PathBuf, DocKind)> = None;
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            let kinds = [DocKind::PlcCode, DocKind::HmiCode, DocKind::IoList, DocKind::Fds, DocKind::Dds, DocKind::Brief, DocKind::Booklet, DocKind::Drawing, DocKind::Schematic, DocKind::Cad, DocKind::Photo, DocKind::Other];
            for k in kinds {
                let docs: Vec<_> = self.intake.docs.iter().filter(|d| d.kind == k).collect();
                if docs.is_empty() {
                    continue;
                }
                egui::CollapsingHeader::new(RichText::new(format!("{}  ({})", k.label(), docs.len())).strong()).id_salt(("kind", k.label())).default_open(k != DocKind::PlcCode && k != DocKind::Other).show(ui, |ui| {
                    for d in docs.iter().take(150) {
                        ui.horizontal(|ui| {
                            let mut kind = d.kind;
                            egui::ComboBox::from_id_salt(&d.path).width(110.0).selected_text(kind.label()).show_ui(ui, |ui| {
                                for k2 in kinds {
                                    ui.selectable_value(&mut kind, k2, k2.label());
                                }
                            });
                            if kind != d.kind {
                                change = Some((d.path.clone(), kind));
                            }
                            ui.add(egui::Label::new(RichText::new(d.path.display().to_string()).color(DIM).size(12.0)).truncate());
                        });
                    }
                    if docs.len() > 150 {
                        ui.label(RichText::new(format!("... and {} more", docs.len() - 150)).color(DIM));
                    }
                });
            }
        });
        if let Some((p, k)) = change {
            self.intake.reclassify(&p, k);
        }
    }
}

/// A table cell: text from the left edge, cut with an ellipsis when it does not fit.
fn left_cell(ui: &mut Ui, w: f32, h: f32, text: &str, color: Color32, header: bool) {
    ui.allocate_ui_with_layout(egui::vec2(w, h), egui::Layout::left_to_right(egui::Align::Center), |ui| {
        ui.set_width(w);
        ui.add_space(6.0);
        let t = if header { RichText::new(text).color(color).size(12.0).strong() } else { RichText::new(text).color(color) };
        ui.add(egui::Label::new(t).truncate());
    });
}

fn short(path: &str) -> String {
    path.rsplit(['\\', '/']).next().unwrap_or(path).to_string()
}

/// The machine at its present pose: a top view (X across, Y up) or a side view (X across, Z down).
fn scene(ui: &mut Ui, w: f32, h: f32, plan: &ModelPlan, pose: &HashMap<String, f64>, top: bool, colours: &HashMap<String, Color32>) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(w, h), Sense::hover());
    let p = ui.painter_at(rect);
    p.rect_filled(rect, 0.0, BG2);
    p.rect_stroke(rect, 0.0, Stroke::new(1.0, LINE), egui::StrokeKind::Inside);
    p.text(rect.left_top() + egui::vec2(10.0, 8.0), Align2::LEFT_TOP, if top { "Live: top view" } else { "Live: side view" }, FontId::proportional(13.0), DIM);
    let [ex, ey, ez] = plan.envelope;
    let vertical = if top { ey } else { ez };
    let area = egui::Rect::from_min_max(rect.left_top() + egui::vec2(20.0, 34.0), rect.right_bottom() - egui::vec2(20.0, 14.0));
    let s = ((area.width() as f64) / ex).min((area.height() as f64) / vertical) as f32;
    let size = egui::vec2(ex as f32 * s, vertical as f32 * s);
    let o = egui::Rect::from_min_size(area.left_top() + egui::vec2((area.width() - size.x) / 2.0, (area.height() - size.y) / 2.0), size);
    let (x0, y0) = (-plan.origin[0], -plan.origin[1]); // the model's lower-left corner in the world
    let at = |wx: f64, wy: f64, wz: f64| {
        let px = o.left() + ((wx + x0) as f32) * s;
        let py = if top { o.bottom() - ((wy + y0) as f32) * s } else { o.bottom() - (wz as f32) * s };
        egui::pos2(px, py)
    };
    let get = |n: &str| plan.joints.iter().find(|j| j.letter.to_string() == n).and_then(|j| pose.get(&j.name)).copied().unwrap_or(0.0);
    let (jx, jy, jz) = (get("X"), get("Y"), get("Z"));
    let draw = |b: &model::Body, dx: f64, dy: f64, dz: f64, c: Color32| {
        let a = at(b.corner[0] + dx, b.corner[1] + dy, b.corner[2] + dz);
        let z = at(b.corner[0] + dx + b.size[0], b.corner[1] + dy + b.size[1], b.corner[2] + dz + b.size[2]);
        let mut r = egui::Rect::from_two_pos(a, z);
        // a device is drawn solid and never smaller than a few pixels, so it can always be found
        let device = plan.devices.iter().any(|d| d.body == b.name);
        if device {
            r = egui::Rect::from_center_size(r.center(), egui::vec2(r.width().max(7.0), r.height().max(7.0)));
        }
        p.rect_filled(r, 0.0, if device { c } else { c.gamma_multiply(0.55) });
        p.rect_stroke(r, 0.0, Stroke::new(if device { 1.5 } else { 1.0 }, if device { FG } else { c }), egui::StrokeKind::Inside);
    };
    // floor
    if !top {
        p.line_segment([at(-x0, 0.0, 0.0), at(ex - x0, 0.0, 0.0)], Stroke::new(1.0, LINE));
    }
    for part in &plan.parts {
        let (dx, dy, dz, c) = match part.name.as_str() {
            "Rail" => (0.0, 0.0, 0.0, DIM),
            n if plan.joints.first().is_some_and(|j| j.b == n) => (jx, 0.0, 0.0, INFO),
            n if plan.joints.get(1).is_some_and(|j| j.b == n) => (jx, jy, 0.0, WARN),
            n if plan.joints.get(2).is_some_and(|j| j.b == n) => (jx, jy, -jz, OK),
            _ => (0.0, 0.0, 0.0, ACCENT),
        };
        for b in &part.bodies {
            draw(b, dx, dy, dz, colours.get(&b.name).copied().unwrap_or(c));
        }
    }
}

fn tick(p: &egui::Painter, x: f32, y: f32, horizontal_axis: bool, high: bool) {
    let c = if high { WARN } else { INFO };
    if horizontal_axis {
        p.line_segment([egui::pos2(x, y - 7.0), egui::pos2(x, y + 7.0)], Stroke::new(2.0, c));
    } else {
        p.line_segment([egui::pos2(x - 7.0, y), egui::pos2(x + 7.0, y)], Stroke::new(2.0, c));
    }
}

/// A to-scale outline of the working envelope. `draw` gets the painter and a map from machine
/// coordinates (mm) to the screen. `y_up` puts Y up the page (a top view); otherwise Z hangs down.
fn diagram(ui: &mut Ui, w: f32, h: f32, title: &str, lx: Option<f64>, ly: Option<f64>, draw: impl FnOnce(&egui::Painter, &dyn Fn(f64, f64) -> (f32, f32)), y_up: bool) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(w, h), Sense::hover());
    let p = ui.painter_at(rect);
    p.rect_filled(rect, 0.0, BG2);
    p.rect_stroke(rect, 0.0, Stroke::new(1.0, LINE), egui::StrokeKind::Inside);
    p.text(rect.left_top() + egui::vec2(10.0, 8.0), Align2::LEFT_TOP, title, FontId::proportional(13.0), DIM);
    let (Some(lx), Some(ly)) = (lx, ly) else {
        p.text(rect.center(), Align2::CENTER_CENTER, "length not known for this axis", FontId::proportional(13.0), DIM);
        return;
    };
    let area = egui::Rect::from_min_max(rect.left_top() + egui::vec2(48.0, 34.0), rect.right_bottom() - egui::vec2(28.0, 34.0));
    let s = ((area.width() as f64) / lx).min((area.height() as f64) / ly) as f32;
    let size = egui::vec2(lx as f32 * s, ly as f32 * s);
    let origin = egui::Rect::from_min_size(area.left_top() + egui::vec2(0.0, (area.height() - size.y) / 2.0), size);
    p.rect_stroke(origin, 0.0, Stroke::new(1.5, FG), egui::StrokeKind::Middle);
    let map = move |x: f64, y: f64| {
        let px = origin.left() + x as f32 * s;
        let py = if y_up { origin.bottom() - y as f32 * s } else { origin.top() + y as f32 * s };
        (px, py)
    };
    p.text(egui::pos2(origin.center().x, origin.bottom() + 6.0), Align2::CENTER_TOP, format!("X {lx:.0} mm"), FontId::proportional(12.0), ACCENT);
    p.text(egui::pos2(origin.left() - 6.0, origin.center().y), Align2::RIGHT_CENTER, format!("{ly:.0}"), FontId::proportional(12.0), ACCENT);
    draw(&p, &map);
}
