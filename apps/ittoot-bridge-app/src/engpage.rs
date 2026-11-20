//! The Engineering page: change the model with specific tools and prompts. Say WHERE (the area box, or pick a part), WHAT to change
//! (the prompt box), and HOW and WHY (so the change is done the way you mean and the reason is kept). The request goes to the design
//! agent, which changes only that area and rebuilds; every request is kept in the model project's change log.

use crate::designwork::{self, DEFAULT_URL};
use crate::theme::*;
use crate::widgets::*;
use crate::work::Job;
use crate::App;
use egui::{RichText, Ui};
use ittoot_bridge::engineering::{self, ChangeRequest, LogEntry};
use ittoot_bridge::job::ModelKind;
use serde_json::{json, Value};

#[derive(Default)]
pub struct EngState {
    pub req: ChangeRequest,
    pub tool: usize,
    pub job: Option<Job<Result<Value, String>>>,
    pub reply: String,
    pub message: String,
}

fn multiline(ui: &mut Ui, label: &str, hint: &str, text: &mut String, rows: usize) {
    ui.label(RichText::new(label).strong());
    ui.add(egui::TextEdit::multiline(text).hint_text(hint).desired_rows(rows).desired_width(f32::INFINITY));
}

impl App {
    /// Where Solidcraft target `id` saves its design: beside the deployment record, `Models\<model project>\<title>.sc` (another
    /// target on the same model adds its id to the file name, so two designs never overwrite each other). `None` without a job number.
    pub fn solid_path(&self, id: &str) -> Option<std::path::PathBuf> {
        let own = self.settings.models.iter().find(|m| m.id == id).map(|m| m.model_project.clone()).filter(|p| !p.is_empty());
        let model = own.or_else(|| self.settings.model_project.clone().filter(|n| !n.trim().is_empty())).unwrap_or_else(|| "Default".to_string());
        let p = self.settings.job.solid_file(&model)?;
        let first = self.settings.models.first().is_none_or(|m| m.id == id);
        if first {
            return Some(p);
        }
        let stem = p.file_stem()?.to_string_lossy().to_string();
        let label = self.settings.models.iter().find(|m| m.id == id).map(|m| m.name.clone()).filter(|n| !n.trim().is_empty()).unwrap_or_else(|| id.to_string());
        Some(p.with_file_name(format!("{stem} - {}.sc", ittoot_bridge::job::safe(&label))))
    }

    fn model_in_use(&self) -> Option<ittoot_bridge::job::ModelProject> {
        let name = self.settings.model_project.as_ref()?;
        self.model_projects.iter().find(|m| &m.name == name).cloned()
    }

    pub fn page_engineering(&mut self, ui: &mut Ui) {
        // the agent's answer
        if let Some(r) = self.eng.job.as_ref().and_then(Job::poll) {
            self.eng.job = None;
            let model = self.settings.model_project.clone().unwrap_or_default();
            let path = engineering::log_path(&self.settings.job, &model);
            match r {
                Ok(v) => {
                    self.eng.reply = v["reply"].as_str().unwrap_or("The agent made the change.").to_string();
                    self.eng.message = "The agent has answered. Rebuild the model in Solidcraft to see it.".into();
                    if let Some(p) = path {
                        let _ = engineering::append_log(&p, &LogEntry { unix: ittoot_bridge::deploy::now_unix(), area: self.eng.req.area.clone(), what: self.eng.req.what.clone(), status: "done".into(), ..Default::default() });
                    }
                }
                Err(e) => {
                    self.eng.message = format!("The agent could not do it: {e}");
                    if let Some(p) = path {
                        let _ = engineering::append_log(&p, &LogEntry { unix: ittoot_bridge::deploy::now_unix(), area: self.eng.req.area.clone(), what: self.eng.req.what.clone(), status: format!("failed: {e}"), ..Default::default() });
                    }
                }
            }
        }

        ui.horizontal(|ui| {
            ui.label(RichText::new("Engineering").heading());
            ui.label(RichText::new("Change the model: say where, what, how and why. The design agent changes only that area and rebuilds it; every request is logged.").color(DIM));
        });
        let model = self.model_in_use();
        let ready = matches!(&model, Some(m) if m.kind == ModelKind::Design) && self.settings.job.folder().is_some();
        panel(ui, |ui| {
            match &model {
                Some(m) => {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("Model project").strong());
                        ui.label(&m.name);
                        chip(ui, m.kind.label(), INFO);
                        ui.label(RichText::new(self.settings.job.title()).color(DIM));
                    });
                    if m.kind != ModelKind::Design {
                        ui.label(RichText::new("Changes are made to a design: make this model from the design agent's rail and baths (Model page, New model), then come back.").color(WARN).size(13.0));
                    }
                }
                None => {
                    ui.label(RichText::new("No model project chosen: pick or make one on the Model page first (it needs a job number).").color(WARN));
                }
            }
        });
        ui.add_space(6.0);

        // the tools and the area picker
        let mut picked_tool = None;
        let mut picked_area: Option<String> = None;
        let names: Vec<String> = self.plan.as_ref().map(|p| p.parts.iter().map(|x| x.name.clone()).collect()).unwrap_or_default();
        let choices: Vec<String> = engineering::area_choices(&names);
        panel(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new("Tool").strong());
                egui::ComboBox::from_id_salt("eng-tool").width(260.0).selected_text(engineering::TOOLS[self.eng.tool.min(engineering::TOOLS.len() - 1)].0).show_ui(ui, |ui| {
                    for (i, t) in engineering::TOOLS.iter().enumerate() {
                        if ui.selectable_label(self.eng.tool == i, t.0).clicked() {
                            picked_tool = Some(i);
                        }
                    }
                });
                ui.label(RichText::new("fills the boxes with a starting point to edit").color(DIM).size(12.0));
            });
            ui.add_space(4.0);
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new("Area").strong());
                ui.add_sized([420.0, 24.0], egui::TextEdit::singleline(&mut self.eng.req.area).hint_text("where: a bath, a station, an axis, a part, or a region of a drawing (sheet, x and y range)"));
                egui::ComboBox::from_id_salt("eng-area").width(220.0).selected_text("pick a part").show_ui(ui, |ui| {
                    for c in &choices {
                        if ui.selectable_label(false, c).clicked() {
                            picked_area = Some(c.clone());
                        }
                    }
                });
            });
            ui.add_space(4.0);
            multiline(ui, "What to change (the prompt)", "e.g. Make the bath at station 3 ... mm longer", &mut self.eng.req.what, 4);
            ui.add_space(4.0);
            ui.columns(2, |cols| {
                multiline(&mut cols[0], "How", "how it should be done: method, sizes, what must stay as it is", &mut self.eng.req.how, 4);
                multiline(&mut cols[1], "Why", "the reason: it is kept in the change log and printed in the report", &mut self.eng.req.why, 4);
            });
        });
        if let Some(i) = picked_tool {
            self.eng.tool = i;
            let t = engineering::TOOLS[i];
            self.eng.req = ChangeRequest { area: t.1.into(), what: t.2.into(), how: t.3.into(), why: t.4.into() };
        }
        if let Some(a) = picked_area {
            self.eng.req.area = a;
        }

        // send
        ui.add_space(6.0);
        let problems = engineering::problems(&self.eng.req);
        let busy = self.eng.job.is_some();
        ui.horizontal_wrapped(|ui| {
            if ui.add_enabled(ready && problems.is_empty() && !busy, egui::Button::new(RichText::new("SEND TO THE DESIGN AGENT").strong()).fill(ACCENT)).clicked() {
                self.send_change();
            }
            if ui.add_enabled(ready && problems.is_empty(), egui::Button::new("Log it only")).on_hover_text("Keep the request in the change log without asking the agent to act on it yet").clicked() {
                self.log_change("requested");
            }
            if ui.button("Open the change log").clicked() {
                let model = self.settings.model_project.clone().unwrap_or_default();
                if let Some(p) = engineering::log_path(&self.settings.job, &model) {
                    designwork::open(&p.display().to_string());
                }
            }
            if busy {
                ui.label(RichText::new("working: the agent is reading the design, changing the area and rebuilding...").color(WARN));
            }
        });
        for p in &problems {
            ui.label(RichText::new(p).color(WARN).size(13.0));
        }
        if !self.eng.message.is_empty() {
            ui.label(RichText::new(&self.eng.message).color(DIM).size(13.0));
        }
        if !self.eng.reply.is_empty() {
            ui.add_space(4.0);
            panel(ui, |ui| {
                ui.label(RichText::new("The agent").strong());
                ui.label(&self.eng.reply);
            });
        }

        // the change log of this model
        let model_name = self.settings.model_project.clone().unwrap_or_default();
        if let Some(p) = engineering::log_path(&self.settings.job, &model_name) {
            let log = engineering::read_log(&p);
            if !log.is_empty() {
                ui.add_space(6.0);
                ui.label(RichText::new(format!("Change log ({} requests)", log.len())).strong());
                panel(ui, |ui| {
                    egui::Grid::new("eng-log").num_columns(3).spacing([14.0, 4.0]).show(ui, |ui| {
                        for e in log.iter().rev().take(8) {
                            ui.label(RichText::new(e.status.split(':').next().unwrap_or("")).color(if e.status.starts_with("failed") { ERROR } else if e.status == "done" { OK } else { INFO }));
                            ui.label(RichText::new(&e.area).strong());
                            ui.label(RichText::new(if e.what.is_empty() { &e.how } else { &e.what }).color(DIM));
                            ui.end_row();
                        }
                    });
                });
            }
        }
    }

    fn log_change(&mut self, status: &str) {
        let model = self.settings.model_project.clone().unwrap_or_default();
        let r = self.eng.req.clone();
        if let Some(p) = engineering::log_path(&self.settings.job, &model) {
            match engineering::append_log(&p, &LogEntry { unix: ittoot_bridge::deploy::now_unix(), area: r.area, what: r.what, how: r.how, why: r.why, status: status.into() }) {
                Ok(()) => self.eng.message = format!("Logged in {}", p.display()),
                Err(e) => self.eng.message = e.to_string(),
            }
        }
    }

    fn send_change(&mut self) {
        let Some(model) = self.settings.model_project.clone() else { return };
        let Some((project, out)) = engineering::target(&self.settings.job, &model) else { return };
        self.log_change("requested");
        let prompt = engineering::compose_prompt(&self.eng.req, &self.settings.job.title(), self.settings.job.application(), &project, &out.display().to_string());
        let url = if self.settings.design_agent.trim().is_empty() { DEFAULT_URL.to_string() } else { self.settings.design_agent.trim().to_string() };
        let ctx = json!({"job_title": self.settings.job.title(), "application": self.settings.job.application(), "project_name_to_use": project, "design_folder": out.display().to_string()});
        self.eng.message = "sent".into();
        self.eng.reply.clear();
        self.eng.job = Some(Job::spawn(move || designwork::post(&url, "/api/chat", &json!({"prompt": prompt, "conversation": "engineering", "context": ctx}), true)));
    }
}
