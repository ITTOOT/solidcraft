//! The Design page: a prompt that goes to the design agent (any AI the agent is connected to), designs built from the
//! brief, the drawings and the IO list, and the result (report, drawings, Solidcraft model) back in the bridge.

use crate::designwork::{self, DEFAULT_URL};
use crate::theme::*;
use crate::widgets::*;
use crate::work::{self, Job};
use crate::App;
use egui::{RichText, Ui};
use serde_json::{json, Value};

pub struct Artifact {
    pub project: String,
    pub report_url: String,
    pub script_path: String,
}

pub struct DesignState {
    pub prompt: String,
    pub project: String,
    pub conversation: Vec<(bool, String)>,
    pub steps: Vec<String>,
    pub artifacts: Vec<Artifact>,
    pub health: Option<Result<Value, String>>,
    pub health_job: Option<Job<Result<Value, String>>>,
    pub last_check: Option<std::time::Instant>,
    pub chat_job: Option<Job<Result<Value, String>>>,
    pub design_job: Option<Job<Result<Value, String>>>,
    pub message: String,
    pub key_entry: String,
    pub detail_full: bool,
}

impl Default for DesignState {
    fn default() -> Self {
        DesignState {
            prompt: String::new(),
            project: "design1".into(),
            conversation: Vec::new(),
            steps: Vec::new(),
            artifacts: Vec::new(),
            health: None,
            health_job: None,
            last_check: None,
            chat_job: None,
            design_job: None,
            message: String::new(),
            key_entry: String::new(),
            detail_full: true,
        }
    }
}

fn artifacts_from(v: &Value, base: &str) -> Vec<Artifact> {
    v["artifacts"]
        .as_array()
        .map(|a| {
            a.iter()
                .map(|x| Artifact {
                    project: x["project"].as_str().unwrap_or("").to_string(),
                    report_url: format!("{}{}", base.trim_end_matches('/'), x["report"].as_str().unwrap_or("")),
                    script_path: format!("{}\\model-script.json", x["folder"].as_str().unwrap_or("")),
                })
                .collect()
        })
        .unwrap_or_default()
}

impl App {
    fn agent_url(&self) -> String {
        if self.settings.design_agent.trim().is_empty() { DEFAULT_URL.to_string() } else { self.settings.design_agent.trim().to_string() }
    }

    /// What the agent is told about the work in hand: the IO list, the documents, the project.
    fn design_context(&mut self) -> Value {
        let dir = crate::data_dir().join("io");
        let _ = std::fs::create_dir_all(&dir);
        let mut points_file = String::new();
        if !self.io.points.is_empty() {
            let f = dir.join("points.json");
            if std::fs::write(&f, serde_json::to_string(&self.io.points).unwrap_or_default()).is_ok() {
                points_file = f.display().to_string();
            }
        }
        let docs: Vec<String> = self.intake.docs.iter().take(60).map(|d| format!("{:?}: {}", d.kind, d.path.display())).collect();
        json!({
            "points_file": points_file,
            "projects": self.settings.projects.clone(),
            "document_folders": self.settings.document_folders.clone(),
            "documents": docs,
            "drawing_pdf": self.io.drawing_pdf.clone(),
            "io_table_pdf": self.io.table_pdf.clone(),
            "job_title": self.settings.job.title(),
            "application": self.settings.job.application(),
            "design_folder": self.settings.job.folder().map(|f| f.join("Design").display().to_string()).unwrap_or_default(),
        })
    }

    pub fn page_design(&mut self, ui: &mut Ui) {
        let url = self.agent_url();
        // poll the agent's health: now, and every few seconds while the page is open
        if let Some(r) = self.design.health_job.as_ref().and_then(Job::poll) {
            self.design.health_job = None;
            self.design.health = Some(r);
        }
        let due = self.design.last_check.is_none_or(|t| t.elapsed() > std::time::Duration::from_secs(5));
        if self.design.health_job.is_none() && due {
            let u = url.clone();
            self.design.health_job = Some(Job::spawn(move || designwork::get(&u, "/api/health")));
            self.design.last_check = Some(std::time::Instant::now());
        }
        for (slot, label) in [(0u8, "chat"), (1u8, "design")] {
            let job = if slot == 0 { self.design.chat_job.as_ref() } else { self.design.design_job.as_ref() };
            if let Some(r) = job.and_then(Job::poll) {
                if slot == 0 { self.design.chat_job = None } else { self.design.design_job = None }
                match r {
                    Ok(v) => {
                        let reply = v["reply"].as_str().map(str::to_string).unwrap_or_else(|| {
                            let s = &v["summary"];
                            format!("Designed {} stations with {} sensors and {} valves. {} assumptions are listed in the report.", s["stations"], s["sensors"], s["valves"], s["assumptions"].as_array().map_or(0, Vec::len))
                        });
                        self.design.conversation.push((false, reply));
                        self.design.steps = v["steps"].as_array().map(|a| a.iter().map(|s| format!("{}: {}", s["tool"].as_str().unwrap_or("?"), if s["ok"].as_bool().unwrap_or(false) { "ok".to_string() } else { s["summary"].as_str().unwrap_or("failed").to_string() })).collect()).unwrap_or_default();
                        let arts = if slot == 0 {
                            artifacts_from(&v, &url)
                        } else {
                            vec![Artifact { project: v["project"].as_str().unwrap_or("").into(), report_url: format!("{}/projects/{}/report.html", url.trim_end_matches('/'), v["project"].as_str().unwrap_or("")), script_path: format!("{}\\model-script.json", v["folder"].as_str().unwrap_or("")) }]
                        };
                        if !arts.is_empty() {
                            self.design.artifacts = arts;
                        }
                        self.design.message.clear();
                    }
                    Err(e) => {
                        self.design.conversation.push((false, format!("({label} failed) {e}")));
                        self.design.message = e;
                    }
                }
            }
        }

        ui.horizontal(|ui| {
            ui.label(RichText::new("Design").heading());
            ui.label(RichText::new("Describe what to design, or give it the brief, drawings and the IO list. The design agent applies sound-principle rules, shows its working, and returns drawings, a Solidcraft model and a report.").color(DIM));
        });
        ui.add_space(6.0);

        // the agent connection
        let mut recheck = false;
        let mut save_key: Option<String> = None;
        panel(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new("Design agent").strong());
                let mut u = self.settings.design_agent.clone();
                ui.add_sized([330.0, 24.0], egui::TextEdit::singleline(&mut u).hint_text(DEFAULT_URL));
                self.settings.design_agent = u;
                if ui.button("Check").clicked() {
                    recheck = true;
                }
                match &self.design.health {
                    Some(Ok(h)) => {
                        chip(ui, &format!("connected: {} v{}", h["agent"].as_str().unwrap_or("agent"), h["version"].as_str().unwrap_or("?")), OK);
                        match h["provider"].as_object() {
                            Some(p) => {
                                let has = p.get("hasKey").and_then(Value::as_bool).unwrap_or(false);
                                chip(ui, &format!("{} · {}", p.get("label").and_then(Value::as_str).unwrap_or("model"), p.get("model").and_then(Value::as_str).unwrap_or("")), if has { OK } else { WARN });
                                if !has {
                                    ui.add_sized([220.0, 24.0], egui::TextEdit::singleline(&mut self.design.key_entry).password(true).hint_text("API key for this model"));
                                    if ui.button("Save key").clicked() && !self.design.key_entry.trim().is_empty() {
                                        save_key = Some(p.get("name").and_then(Value::as_str).unwrap_or("claude").to_string());
                                    }
                                }
                            }
                            None => chip(ui, "no model set", WARN),
                        }
                    }
                    Some(Err(_)) => chip(ui, "agent not running", ERROR),
                    None => chip(ui, "checking...", WARN),
                }
            });
            if let Some(Err(e)) = &self.design.health {
                ui.label(RichText::new(format!("{e}. Start it with: node server.mjs in AI Office\\design-agent (or scripts\\start-bridge.ps1 -Design).")).color(DIM).size(12.0));
            }
            ui.label(RichText::new("The model is the agent's choice (Claude, OpenRouter or any OpenAI-compatible endpoint, set on the agent's own page). The key stays with the agent.").color(DIM).size(12.0));
        });
        if recheck {
            self.design.last_check = None;
        }
        if let Some(provider) = save_key {
            let key = std::mem::take(&mut self.design.key_entry);
            let u = url.clone();
            let r = designwork::post(&u, "/api/provider", &json!({"provider": provider, "key": key, "active": provider}), false);
            self.design.message = match r {
                Ok(_) => "Key saved with the agent.".into(),
                Err(e) => e,
            };
            self.design.last_check = None;
        }

        ui.add_space(8.0);
        // the prompt
        let busy = self.design.chat_job.is_some() || self.design.design_job.is_some();
        let mut send = false;
        let mut direct = false;
        panel(ui, |ui| {
            ui.label(RichText::new("Prompt").strong());
            ui.add(egui::TextEdit::multiline(&mut self.design.prompt).desired_rows(4).desired_width(f32::INFINITY).hint_text("e.g. Design the transporter for this line from the IO list and the drawings: two transporters, 150 kg jigs, service areas at both ends. Tell me what is missing."));
            ui.horizontal_wrapped(|ui| {
                ui.add_enabled_ui(!busy && !self.design.prompt.trim().is_empty(), |ui| {
                    if start_button(ui, "SEND TO THE DESIGN AGENT").clicked() {
                        send = true;
                    }
                });
                ui.label("Project");
                ui.add_sized([130.0, 24.0], egui::TextEdit::singleline(&mut self.design.project));
                ui.add_enabled_ui(!busy && !self.io.points.is_empty(), |ui| {
                    if ui.button("Design from the IO list (no AI)").on_hover_text("Uses the stations and wired signals the IO Builder read. Deterministic: same rules, every assumption listed, no model needed.").clicked() {
                        direct = true;
                    }
                });
                ui.checkbox(&mut self.design.detail_full, "pipes and trays in the 3D model");
                if busy {
                    ui.label(RichText::new("working: the agent is reading, calculating and drawing...").color(WARN));
                }
            });
            ui.label(RichText::new(format!("The agent is told: {} IO channels, {} document(s), {} project(s).", self.io.points.len(), self.intake.docs.len(), self.settings.projects.len())).color(DIM).size(12.0));
        });
        let project: String = self.design.project.chars().map(|c| if c.is_ascii_alphanumeric() || c == '_' || c == '-' { c } else { '_' }).collect();
        if send {
            let prompt = std::mem::take(&mut self.design.prompt);
            self.design.conversation.push((true, prompt.clone()));
            let mut ctx = self.design_context();
            ctx["project_name_to_use"] = json!(project);
            let u = url.clone();
            self.design.chat_job = Some(Job::spawn(move || designwork::post(&u, "/api/chat", &json!({"prompt": prompt, "conversation": "bridge", "context": ctx}), true)));
        }
        if direct {
            let ctx = self.design_context();
            let u = url.clone();
            let detail = if self.design.detail_full { "full" } else { "basic" };
            let points = ctx["points_file"].as_str().unwrap_or("").to_string();
            let out = self.settings.job.design_dir(&project).map(|d| d.display().to_string());
            let (title, app) = (self.settings.job.title(), self.settings.job.application().to_string());
            self.design.conversation.push((true, format!("(design from the IO list: {} channels)", self.io.points.len())));
            self.design.design_job = Some(Job::spawn(move || designwork::post(&u, "/api/design/from-io", &json!({"project": project, "points_file": points, "detail": detail, "output_dir": out, "spec": {"name": title, "application": app}}), false)));
        }

        // the conversation and what the agent did
        ui.add_space(8.0);
        let mut open_url: Option<String> = None;
        let mut build_script: Option<String> = None;
        panel(ui, |ui| {
            if !self.design.artifacts.is_empty() {
                for a in &self.design.artifacts {
                    ui.horizontal_wrapped(|ui| {
                        ui.label(RichText::new(format!("Design {}", a.project)).strong());
                        if accent_button(ui, "Open the report").clicked() {
                            open_url = Some(a.report_url.clone());
                        }
                        if ui.button("Build it in Solidcraft").on_hover_text("Pushes the design's model to the Solidcraft the Model page started (Start Solidcraft there first)").clicked() {
                            build_script = Some(a.script_path.clone());
                        }
                        ui.label(RichText::new(&a.script_path).color(DIM).size(11.0));
                    });
                }
                ui.add_space(6.0);
            }
            if !self.design.message.is_empty() {
                ui.label(RichText::new(&self.design.message).color(WARN));
            }
            egui::ScrollArea::vertical().max_height(360.0).auto_shrink([false, true]).stick_to_bottom(true).show(ui, |ui| {
                if self.design.conversation.is_empty() {
                    ui.label(RichText::new("Nothing yet. Send a prompt, or press the IO-list button after reading the drawings on the IO Builder page.").color(DIM));
                }
                for (you, text) in &self.design.conversation {
                    ui.label(RichText::new(if *you { "You" } else { "Design agent" }).color(if *you { ACCENT } else { INFO }).strong());
                    ui.label(text);
                    ui.add_space(6.0);
                }
                if !self.design.steps.is_empty() {
                    ui.label(RichText::new("What the agent did").color(DIM).size(12.0).strong());
                    for s in &self.design.steps {
                        ui.label(RichText::new(s).color(DIM).size(11.0));
                    }
                }
            });
        });
        if let Some(u) = open_url {
            designwork::open(&u);
        }
        if let Some(path) = build_script {
            self.design.message = self.push_design_script(&path);
        }
    }

    /// Send a design's model script to the active model's Solidcraft, the same way the Model page builds.
    fn push_design_script(&mut self, path: &str) -> String {
        let text = match std::fs::read_to_string(path) {
            Ok(t) => t,
            Err(e) => return format!("Could not read {path}: {e}"),
        };
        let cmds: Vec<Value> = match serde_json::from_str(&text) {
            Ok(c) => c,
            Err(e) => return format!("The model script is not valid: {e}"),
        };
        self.ensure_models();
        let id = self.settings.models.get(self.active_model_index()).map(|m| m.id.clone()).unwrap_or_default();
        match self.runs.get(&id).and_then(|r| r.link.as_ref()) {
            Some(link) => {
                link.send(work::PushMsg::Build(cmds.clone()));
                if let Some(p) = self.solid_path(&id) {
                    link.send(work::PushMsg::SaveAs(p));
                }
                if let Some(r) = self.runs.get_mut(&id) {
                    r.status = "building the design...".into();
                }
                format!("Sent {} commands to Solidcraft (model '{}'). Progress shows on the Model page.", cmds.len(), id)
            }
            None => "Solidcraft is not linked. Press START SOLIDCRAFT on the Model page first, then build again.".into(),
        }
    }
}
