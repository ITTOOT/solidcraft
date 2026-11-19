//! The job, the PLC project and the model project: which job we are working on (its number titles every document and
//! names the folder the files go in), which PLC project is read, and which model project is shown, with a way to make a
//! new model from the gantry template, from the design agent's rail and baths, or from scratch.

use crate::theme::*;
use crate::widgets::*;
use crate::App;
use egui::{RichText, Ui};
use ittoot_bridge::connection::LinkState;
use ittoot_bridge::job::{self, FreeBinding, ModelKind, ModelProject, PlcLink, PROJECT_TYPES};

enum Act {
    None,
    MakePrimary(String),
    LinkPlc(String, Option<String>),
    SelectModel(Option<String>),
    New(ModelKind),
    AddFree,
    DropFree(usize),
    OpenFolder,
}

fn plc_label(path: &str) -> String {
    let p = std::path::Path::new(path);
    p.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| path.to_string())
}

impl App {
    pub fn refresh_models(&mut self) {
        self.model_projects = job::list_models(&self.settings.job);
        self.models_key = format!("{}|{}", self.settings.job.root, self.settings.job.number);
    }

    /// Make a model project the one in use: the design it needs is read, a scratch model's joints are loaded.
    pub fn apply_model_project(&mut self, name: Option<String>) {
        let mp = name.as_ref().and_then(|n| self.model_projects.iter().find(|m| &m.name == n)).cloned();
        self.settings.model_project = mp.as_ref().map(|m| m.name.clone());
        self.design_model = None;
        self.design_stamp += 1;
        self.settings.design_model_dir = match &mp {
            Some(m) if m.kind == ModelKind::Design => m.design_dir.clone(),
            _ => String::new(),
        };
        if let Some(m) = &mp
            && !m.plcs.is_empty()
        {
            // the model's own PLCs, the primary first, each with its connection
            let mut order: Vec<String> = m.plcs.iter().map(|p| p.project.clone()).filter(|p| !p.is_empty()).collect();
            let rest: Vec<String> = self.settings.projects.iter().filter(|p| !order.contains(p)).cloned().collect();
            order.extend(rest);
            self.settings.projects = order;
            self.settings.project_links = m.plcs.iter().filter(|p| !p.project.is_empty() && !p.connection.is_empty()).map(|p| (p.project.clone(), p.connection.clone())).collect();
            if let Some(first) = self.settings.projects.first().and_then(|p| self.settings.project_links.get(p)).cloned() {
                self.settings.active_link = Some(first);
            }
            self.settings.apply_links();
            self.pollers.clear();
            self.live_by.clear();
        }
        self.active_kind = mp.as_ref().map(|m| m.kind).unwrap_or_default();
        self.scratch = mp.as_ref().map(|m| m.free.clone()).unwrap_or_default();
        self.colour_sent.clear();
    }

    /// Write the model project in use back to its folder (a scratch model's bindings, a design's folder).
    pub fn save_active_model(&mut self) {
        let Some(name) = self.settings.model_project.clone() else { return };
        let Some(mut mp) = self.model_projects.iter().find(|m| m.name == name).cloned() else { return };
        mp.free = self.scratch.clone();
        mp.plcs = self.settings.projects.iter().map(|p| PlcLink { project: p.clone(), connection: self.settings.project_links.get(p).cloned().unwrap_or_else(|| if self.settings.projects.first() == Some(p) { self.settings.active_link.clone().unwrap_or_default() } else { String::new() }) }).collect();
        if mp.kind == ModelKind::Design {
            mp.design_dir = self.settings.design_model_dir.clone();
        }
        match job::save_model(&self.settings.job, &mp) {
            Ok(_) => self.refresh_models(),
            Err(e) => self.job_msg = e.to_string(),
        }
    }

    pub fn new_model(&mut self, kind: ModelKind) {
        if self.settings.job.folder().is_none() {
            self.job_msg = "Give the job a number first: the model is saved in the job's folder.".into();
            return;
        }
        let mut name = self.new_model_name.trim().to_string();
        if name.is_empty() {
            name = match kind {
                ModelKind::Template => "Gantry",
                ModelKind::Design => "Rail and baths",
                ModelKind::Scratch => "Model",
            }
            .to_string();
        }
        let base = name.clone();
        let mut n = 1;
        while self.model_projects.iter().any(|m| job::safe(&m.name) == job::safe(&name)) {
            n += 1;
            name = format!("{base} {n}");
        }
        if kind == ModelKind::Design {
            self.start_design_model(&name);
            return;
        }
        let mp = ModelProject { name: name.clone(), kind, plc_project: self.settings.projects.first().cloned().unwrap_or_default(), ..Default::default() };
        match job::save_model(&self.settings.job, &mp) {
            Ok(p) => {
                self.job_msg = format!("Model project saved: {}", p.display());
                self.refresh_models();
                self.apply_model_project(Some(name));
                self.new_model_name.clear();
            }
            Err(e) => self.job_msg = e.to_string(),
        }
    }

    /// The Job, PLC project and Model project panel at the top of the Model page.
    pub fn job_panel(&mut self, ui: &mut Ui) {
        if self.models_key != format!("{}|{}", self.settings.job.root, self.settings.job.number) {
            self.refresh_models();
        }
        let mut act = Act::None;
        let before = self.settings.job.clone();
        let folder = self.settings.job.folder();
        let plcs: Vec<String> = self.settings.projects.clone();
        let conns: Vec<(String, String)> = self.settings.connections.iter().filter(|c| c.protocol != ittoot_bridge::connection::Protocol::Simulated).map(|c| (c.id.clone(), c.name.clone())).collect();
        let models: Vec<ModelProject> = self.model_projects.clone();
        let current = self.settings.model_project.clone();
        panel(ui, |ui| {
            ui.label(RichText::new("Job").strong());
            egui::Grid::new("job-grid").num_columns(2).spacing([12.0, 6.0]).show(ui, |ui| {
                ui.label("Type of project");
                egui::ComboBox::from_id_salt("job-type").width(220.0).selected_text(self.settings.job.project_type.clone()).show_ui(ui, |ui| {
                    for (label, _) in PROJECT_TYPES {
                        ui.selectable_value(&mut self.settings.job.project_type, (*label).to_string(), *label);
                    }
                });
                ui.end_row();
                ui.label("Job number");
                ui.add_sized([220.0, 24.0], egui::TextEdit::singleline(&mut self.settings.job.number).hint_text("titles every document and model"));
                ui.end_row();
                ui.label("Name");
                ui.add_sized([220.0, 24.0], egui::TextEdit::singleline(&mut self.settings.job.name).hint_text("optional, left blank for now"));
                ui.end_row();
                ui.label("Folder for jobs");
                ui.add_sized([520.0, 24.0], egui::TextEdit::singleline(&mut self.settings.job.root).hint_text(job::DEFAULT_ROOT));
                ui.end_row();
            });
            ui.horizontal_wrapped(|ui| {
                match &folder {
                    Some(f) => {
                        ui.label(RichText::new(format!("Title: {}", self.settings.job.title())).strong());
                        ui.label(RichText::new(format!("Everything for this job is saved in {}", f.display())).color(DIM).size(12.0));
                        if ui.button("Open the folder").clicked() {
                            act = Act::OpenFolder;
                        }
                    }
                    None => chip(ui, "no job number yet: files go to the app's own data folder until there is one", WARN),
                }
            });
            ui.add_space(8.0);
            ui.label(RichText::new("PLC project and model project").strong());
            ui.label(RichText::new("Several PLCs may run one model: give each its connection. The first is the primary (its signals keep their plain names); a joint or device can follow a signal of any of them.").color(DIM).size(12.0));
            egui::Grid::new("plc-grid").num_columns(4).spacing([12.0, 6.0]).show(ui, |ui| {
                if plcs.is_empty() {
                    ui.label(RichText::new("No PLC project yet: add one on the Settings page.").color(WARN));
                    ui.end_row();
                }
                for (i, p) in plcs.iter().enumerate() {
                    if i == 0 {
                        chip(ui, "primary", ACCENT);
                    } else if ui.button("make primary").clicked() {
                        act = Act::MakePrimary(p.clone());
                    }
                    ui.label(RichText::new(plc_label(p)).strong()).on_hover_text(p);
                    let linked = if i == 0 { self.settings.active_link.clone() } else { self.settings.project_links.get(p).cloned() };
                    let linked_name = linked.as_ref().and_then(|id| conns.iter().find(|(c, _)| c == id)).map(|(_, n)| n.clone()).unwrap_or_else(|| "none: the start values in its sources".into());
                    egui::ComboBox::from_id_salt(("plc-link", p)).width(320.0).selected_text(linked_name).show_ui(ui, |ui| {
                        if ui.selectable_label(linked.is_none(), "none: the start values in its sources").clicked() {
                            act = Act::LinkPlc(p.clone(), None);
                        }
                        for (id, name) in &conns {
                            if ui.selectable_label(linked.as_ref() == Some(id), name).clicked() {
                                act = Act::LinkPlc(p.clone(), Some(id.clone()));
                            }
                        }
                    });
                    let state = linked.as_ref().and_then(|id| if i == 0 { Some(&self.link_state) } else { self.link_states.get(id) });
                    match state {
                        Some(LinkState::Connected) => chip(ui, "connected", OK),
                        Some(LinkState::Connecting) => chip(ui, "connecting", WARN),
                        Some(LinkState::Failed(_)) => chip(ui, "failed", ERROR),
                        _ => chip(ui, "off", DIM),
                    }
                    ui.end_row();
                }
            });
            ui.add_space(4.0);
            egui::Grid::new("proj-grid").num_columns(3).spacing([12.0, 6.0]).show(ui, |ui| {
                ui.label("Model project");
                let shown = current.clone().unwrap_or_else(|| "none: the plain gantry template".into());
                egui::ComboBox::from_id_salt("model-project").width(420.0).selected_text(shown).show_ui(ui, |ui| {
                    if ui.selectable_label(current.is_none(), "none: the plain gantry template").clicked() {
                        act = Act::SelectModel(None);
                    }
                    for m in &models {
                        if ui.selectable_label(current.as_deref() == Some(m.name.as_str()), format!("{}   ({})", m.name, m.kind.label())).clicked() {
                            act = Act::SelectModel(Some(m.name.clone()));
                        }
                    }
                });
                if let Some(m) = models.iter().find(|m| Some(&m.name) == current.as_ref()) {
                    chip(ui, m.kind.label(), INFO);
                }
                ui.end_row();
            });
            ui.horizontal_wrapped(|ui| {
                ui.label("New model");
                ui.add_sized([180.0, 24.0], egui::TextEdit::singleline(&mut self.new_model_name).hint_text("name"));
                if ui.button("from the gantry template").on_hover_text("Sized from the PLC's data: a bridge, trolley and hoist with the PLC's axes bound to the joints").clicked() {
                    act = Act::New(ModelKind::Template);
                }
                if ui.button("from the design agent's rail and baths").on_hover_text("One bath for each PLC station on the design agent's plating-line model; the transport runs along its rail. The design agent must be running.").clicked() {
                    act = Act::New(ModelKind::Design);
                }
                if ui.button("from scratch").on_hover_text("An empty model: draw it in Solidcraft, then tie each joint to a PLC signal below").clicked() {
                    act = Act::New(ModelKind::Scratch);
                }
            });
            if !self.job_msg.is_empty() {
                ui.label(RichText::new(&self.job_msg).color(DIM).size(12.0));
            }

            // a model made from scratch: its joints and the PLC signal that drives each
            if self.active_kind == ModelKind::Scratch {
                ui.add_space(8.0);
                ui.label(RichText::new("Joints of this model").strong());
                ui.label(RichText::new("Name each joint exactly as it is in Solidcraft; the value sent is scale x signal + offset (Solidcraft keeps it within the joint's own limits).").color(DIM).size(12.0));
                let before_free = self.scratch.clone();
                let mut drop = None;
                let sugg = self.signal_choices.clone();
                egui::Grid::new("scratch-grid").num_columns(5).spacing([10.0, 6.0]).show(ui, |ui| {
                    for h in ["Joint", "PLC signal", "Scale", "Offset", ""] {
                        ui.label(RichText::new(h).color(DIM).size(12.0).strong());
                    }
                    ui.end_row();
                    for (i, b) in self.scratch.iter_mut().enumerate() {
                        ui.add_sized([160.0, 24.0], egui::TextEdit::singleline(&mut b.joint));
                        search_box(ui, ui.make_persistent_id(("free", i)), &mut b.key, &sugg, 420.0, "Search a signal");
                        ui.add(egui::DragValue::new(&mut b.scale).speed(0.01));
                        ui.add(egui::DragValue::new(&mut b.offset).speed(1.0));
                        if ui.button("remove").clicked() {
                            drop = Some(i);
                        }
                        ui.end_row();
                    }
                });
                if let Some(i) = drop {
                    act = Act::DropFree(i);
                }
                if ui.button("+ Add a joint").clicked() {
                    act = Act::AddFree;
                }
                if before_free != self.scratch {
                    self.scratch_dirty = true;
                }
            }
        });
        // the job's own fields changed: save with the settings, and look again for its models
        if before != self.settings.job {
            self.refresh_models();
        }
        match act {
            Act::None => {}
            Act::OpenFolder => {
                if let Some(f) = folder {
                    let _ = std::fs::create_dir_all(&f);
                    crate::designwork::open(&f.display().to_string());
                }
            }
            Act::MakePrimary(p) => {
                self.settings.projects.retain(|x| x != &p);
                self.settings.projects.insert(0, p.clone());
                if let Some(c) = self.settings.project_links.get(&p).cloned() {
                    self.settings.active_link = Some(c);
                }
                self.settings.apply_links();
                self.pollers.clear();
                self.live_by.clear();
                self.job_msg = "Primary PLC changed: its signals now keep their plain names, so check the joint bindings.".into();
                self.scratch_dirty = true;
            }
            Act::LinkPlc(p, c) => {
                match &c {
                    Some(id) => {
                        self.settings.project_links.insert(p.clone(), id.clone());
                    }
                    None => {
                        self.settings.project_links.remove(&p);
                    }
                }
                if self.settings.projects.first() == Some(&p) {
                    self.settings.active_link = c;
                }
                self.settings.apply_links();
                self.pollers.clear();
                self.live_by.clear();
                // areas found before projects were tracked carry no project: look again
                if self.settings.areas.iter().any(|a| a.project.is_empty()) {
                    self.start_auto();
                }
                self.scratch_dirty = true;
            }
            Act::SelectModel(n) => {
                self.apply_model_project(n);
            }
            Act::New(k) => self.new_model(k),
            Act::AddFree => {
                self.scratch.push(FreeBinding { joint: String::new(), key: String::new(), scale: 1.0, offset: 0.0 });
                self.scratch_dirty = true;
            }
            Act::DropFree(i) => {
                if i < self.scratch.len() {
                    self.scratch.remove(i);
                }
                self.scratch_dirty = true;
            }
        }
        if self.scratch_dirty && !ui.ctx().input(|i| i.pointer.any_down()) {
            self.scratch_dirty = false;
            self.save_active_model();
        }
    }
}
