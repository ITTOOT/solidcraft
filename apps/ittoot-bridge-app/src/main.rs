//! ITTOOT Solidcraft connection bridge: settings, connections, dynamic tables, equipment sizes
//! inferred from the code, databases and documents.

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

mod pages;
mod theme;
mod widgets;
mod work;

use ittoot_bridge::area::AreaRole;
use ittoot_bridge::connection::{Connection, DeviceKind, LinkState, Protocol, Secrets};
use ittoot_bridge::discover::{auto_config, suggestions, Discovery, ScanLimits, Suggestion};
use ittoot_bridge::driver::Value;
use ittoot_bridge::infer::{infer, station_points, Inference, Record};
use ittoot_bridge::model::{self, ModelBinding, ModelPlan};
use ittoot_bridge::discover::Suggestion as Sugg;
use ittoot_bridge::intake::Intake;
use ittoot_bridge::settings::BridgeSettings;
use ittoot_bridge::table::Table;
use std::collections::HashMap;
use std::path::PathBuf;
use work::{Job, PollMsg, Poller};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Page {
    Settings,
    Connections,
    Tables,
    Equipment,
    Model,
    Databases,
    Documents,
}

impl Page {
    const ALL: [(Page, &'static str, &'static str); 7] = [
        (Page::Settings, "Settings", "settings"),
        (Page::Connections, "Connections", "connections"),
        (Page::Tables, "Tables", "tables"),
        (Page::Equipment, "Equipment", "equipment"),
        (Page::Model, "Model", "model"),
        (Page::Databases, "Databases", "databases"),
        (Page::Documents, "Documents", "documents"),
    ];
}

#[derive(Default)]
pub struct DbState {
    pub conn: Option<String>,
    pub tables: Vec<ittoot_db::TableInfo>,
    pub selected: Option<usize>,
    pub grid: ittoot_db::Grid,
    pub message: String,
    pub list_job: Option<Job<Result<Vec<ittoot_db::TableInfo>, String>>>,
    pub page_job: Option<Job<Result<ittoot_db::Grid, String>>>,
}

struct Shot {
    path: PathBuf,
    frames: u32,
    requested: bool,
    result: std::sync::Arc<std::sync::Mutex<Option<std::sync::Arc<egui::ColorImage>>>>,
}

pub struct App {
    pub settings: BridgeSettings,
    pub settings_path: Option<PathBuf>,
    pub secrets: Secrets,
    pub secrets_path: Option<PathBuf>,
    pub page: Page,
    pub tab_role: AreaRole,
    pub discovery: Discovery,
    pub auto_job: Option<Job<Discovery>>,
    pub auto_msg: String,
    pub suggestions: Vec<Suggestion>,
    pub tables: Vec<Table>,
    pub records: Vec<Record>,
    pub start_values: HashMap<String, Value>,
    pub build_job: Option<Job<work::Built>>,
    pub built_key: String,
    pub active_table: usize,
    pub filter: String,
    pub live: HashMap<String, Value>,
    pub poller: Option<Poller>,
    pub link_state: LinkState,
    pub inference: Inference,
    pub intake: Intake,
    pub intake_key: String,
    pub intake_job: Option<Job<Intake>>,
    pub sel_conn: usize,
    pub tests: HashMap<String, Result<String, String>>,
    pub test_jobs: HashMap<String, Job<Result<String, String>>>,
    pub db: DbState,
    pub pki: PathBuf,
    pub plan: Option<ModelPlan>,
    pub roles: Vec<(String, Vec<char>)>,
    pub roles_from: String,
    pub roles_key: String,
    pub all_keys: Vec<String>,
    pub signal_choices: Vec<Sugg>,
    pub auto_bindings: Vec<ModelBinding>,
    pub bindings: Vec<ModelBinding>,
    pub plan_key: String,
    /// Simulated joint values (the sliders on the Model page), by joint name.
    pub pose: HashMap<String, f64>,
    /// One entry per model target, by id: its Solidcraft process, link and status.
    pub runs: HashMap<String, ModelRun>,
    /// The model list as last written, to save only when it changes.
    pub models_saved: String,
    /// `--go`: press Start, then Build, then follow the machine, as a person would (0 off, 1 start, 2 build, 3 done).
    pub autorun: u8,
    pub last_push: std::time::Instant,
    pub export_msg: String,
    pub pending_table: Option<String>,
    shot: Option<Shot>,
    /// `--save-deployment`: write the record once the model is ready, then quit (no window needed by hand).
    save_deployment_and_quit: bool,
}

/// What one model target is doing: the Solidcraft this bridge started, the link to it, a note.
#[derive(Default)]
pub struct ModelRun {
    pub child: Option<std::process::Child>,
    pub wait: Option<Job<bool>>,
    pub link: Option<work::SolidLink>,
    pub status: String,
    pub push_live: bool,
}

pub fn data_dir() -> PathBuf {
    std::env::var_os("APPDATA").map(PathBuf::from).unwrap_or_else(std::env::temp_dir).join("ITTOOT").join("Bridge")
}

impl App {
    fn new(persist: bool) -> App {
        let dir = data_dir();
        let (settings_path, secrets_path) = if persist { (Some(dir.join("settings.json")), Some(dir.join("secrets.json"))) } else { (None, None) };
        let settings = settings_path.as_ref().and_then(|p| BridgeSettings::load(p).ok()).unwrap_or_default();
        let secrets = secrets_path.as_ref().and_then(|p| Secrets::load(p).ok()).unwrap_or_default();
        let mut app = App {
            settings,
            settings_path,
            secrets,
            secrets_path,
            page: Page::Settings,
            tab_role: AreaRole::Controls,
            discovery: Discovery::default(),
            auto_job: None,
            auto_msg: String::new(),
            suggestions: Vec::new(),
            tables: Vec::new(),
            records: Vec::new(),
            start_values: HashMap::new(),
            build_job: None,
            built_key: String::new(),
            active_table: 0,
            filter: String::new(),
            live: HashMap::new(),
            poller: None,
            link_state: LinkState::Disconnected,
            inference: Inference::default(),
            intake: Intake::default(),
            intake_key: String::new(),
            intake_job: None,
            sel_conn: 0,
            tests: HashMap::new(),
            test_jobs: HashMap::new(),
            db: DbState::default(),
            pki: dir.join("pki"),
            plan: None,
            roles: Vec::new(),
            roles_from: String::new(),
            roles_key: String::new(),
            all_keys: Vec::new(),
            signal_choices: Vec::new(),
            auto_bindings: Vec::new(),
            bindings: Vec::new(),
            plan_key: String::new(),
            pose: HashMap::new(),
            runs: HashMap::new(),
            models_saved: String::new(),
            autorun: 0,
            last_push: std::time::Instant::now(),
            export_msg: String::new(),
            pending_table: None,
            shot: None,
            save_deployment_and_quit: false,
        };
        app.ensure_simulated_link();
        app.ensure_models();
        app
    }

    /// There is always at least one model target; one is active.
    pub fn ensure_models(&mut self) {
        if self.settings.models.is_empty() {
            self.settings.models.push(ittoot_bridge::target::ModelTarget::new("model1", "Model 1", ittoot_bridge::target::DEFAULT_PORT));
        }
        let ok = self.settings.active_model.as_ref().is_some_and(|id| self.settings.models.iter().any(|m| &m.id == id));
        if !ok {
            self.settings.active_model = self.settings.models.first().map(|m| m.id.clone());
        }
    }

    pub fn active_model_index(&self) -> usize {
        self.settings.active_model.as_ref().and_then(|id| self.settings.models.iter().position(|m| &m.id == id)).unwrap_or(0)
    }

    pub fn ensure_simulated_link(&mut self) {
        if !self.settings.connections.iter().any(|c| c.protocol == Protocol::Simulated) {
            let mut c = Connection::new("sim", "Simulated PLC (start values)", DeviceKind::Plc, Protocol::Simulated, "");
            c.notes = "No PLC needed: the tables show the values written in the sources.".into();
            self.settings.connections.insert(0, c);
        }
    }

    /// Load the HoistGantry test project the way a user would: give the project, press Auto-config.
    fn demo(&mut self, project: &str) {
        self.settings.projects = vec![project.to_string()];
        self.settings.document_folders = vec![project.rsplit_once(['\\', '/']).map(|(d, _)| d.to_string()).unwrap_or_default()];
        let d = auto_config(&self.settings.projects, &[], ScanLimits::default());
        self.finish_auto(d);
        if let Some(src) = self.discovery.projects.first().and_then(|p| p.sources.first()) {
            self.settings.document_folders.push(src.clone());
        }
        let mut plc = Connection::new("plc1", "Hoist gantry PLC", DeviceKind::Plc, Protocol::OpcUa, "192.168.0.1");
        plc.notes = "S7-1511-1 PN, OPC UA server enabled".into();
        self.settings.connections.push(plc);
        let mut db = Connection::new("db1", "Plant history", DeviceKind::Database, Protocol::SqlServer, "10.0.0.20");
        db.extra = "PlantHistory".into();
        db.username = "bridge_ro".into();
        self.settings.connections.push(db);
    }

    /// `--project`: use this project from the first start on. Saved work is kept: it only adds the
    /// project when it is not there yet, and finds its data blocks the first time.
    fn use_project(&mut self, project: &str) {
        if !self.settings.projects.iter().any(|p| p == project) {
            self.settings.projects.push(project.to_string());
            if let Some((dir, _)) = project.rsplit_once(['\\', '/'])
                && !self.settings.document_folders.iter().any(|d| d == dir)
            {
                self.settings.document_folders.push(dir.to_string());
            }
        }
        if self.settings.areas.is_empty() {
            let d = auto_config(&self.settings.projects, &[], ScanLimits::default());
            self.finish_auto(d);
        }
    }

    pub fn start_auto(&mut self) {
        let projects = self.settings.projects.clone();
        self.auto_msg = "Scanning...".into();
        self.auto_job = Some(Job::spawn(move || auto_config(&projects, &[], ScanLimits::default())));
    }

    fn finish_auto(&mut self, d: Discovery) {
        let (added, removed) = self.settings.merge_auto(d.areas());
        let c = d.candidates.iter().filter(|c| c.area.role == AreaRole::Controls).count();
        let m = d.candidates.len() - c;
        self.auto_msg = format!("Found {} data areas ({c} controls, {m} management). {added} added, {removed} no longer found.", d.candidates.len());
        self.suggestions = suggestions(&d);
        self.discovery = d;
    }

    fn areas_key(&self) -> String {
        self.settings.areas.iter().filter(|a| a.enabled).map(|a| format!("{:?}|{:?}|{}", a.role, a.kind, a.source)).collect::<Vec<_>>().join("\n")
    }

    fn save(&mut self) {
        if let Some(p) = &self.settings_path {
            let _ = self.settings.save(p);
        }
        if let Some(p) = &self.secrets_path {
            let _ = self.secrets.save(p);
        }
    }

    fn pump(&mut self, ctx: &egui::Context) {
        if let Some(d) = self.auto_job.as_ref().and_then(Job::poll) {
            self.auto_job = None;
            self.finish_auto(d);
        }
        let key = self.areas_key();
        if self.build_job.is_none() && key != self.built_key {
            self.built_key = key;
            let areas = self.settings.areas.clone();
            self.build_job = Some(Job::spawn(move || work::build(&areas)));
        }
        if let Some(b) = self.build_job.as_ref().and_then(Job::poll) {
            self.build_job = None;
            self.tables = b.tables;
            self.records = b.records;
            self.start_values = b.start_values;
            self.active_table = self.active_table.min(self.tables.len().saturating_sub(1));
            self.all_keys = self.tables.iter().flat_map(|t| t.rows.iter().map(|r| r.key.clone())).collect();
            self.signal_choices = self
                .all_keys
                .iter()
                .filter(|k| {
                    let w = ittoot_bridge::infer::words(k);
                    w.iter().any(|x| x == "actual") && w.iter().any(|x| x == "position")
                })
                .map(|k| Suggestion { value: k.clone(), detail: k.rsplit('.').take(3).collect::<Vec<_>>().into_iter().rev().collect::<Vec<_>>().join(".") })
                .collect();
            self.plan_key.clear(); // bind again against the new signals
        }
        self.inference = infer(&self.records, &self.settings.overrides);
        self.update_model();

        let ikey = self.settings.document_folders.join("\n");
        if self.intake_job.is_none() && ikey != self.intake_key {
            self.intake_key = ikey;
            let folders = self.settings.document_folders.clone();
            self.intake_job = Some(Job::spawn(move || Intake::scan(&folders)));
        }
        if let Some(i) = self.intake_job.as_ref().and_then(Job::poll) {
            self.intake_job = None;
            self.intake = i;
            self.roles_key.clear(); // read the documents again
        }

        let done: Vec<String> = self.test_jobs.iter().filter_map(|(k, j)| j.poll().map(|r| (k.clone(), r))).map(|(k, r)| {
            self.tests.insert(k.clone(), r);
            k
        }).collect();
        for k in done {
            self.test_jobs.remove(&k);
        }
        if let Some(j) = &self.db.list_job
            && let Some(r) = j.poll()
        {
            self.db.list_job = None;
            match r {
                Ok(t) => {
                    self.db.message = format!("{} tables", t.len());
                    self.db.tables = t;
                    self.db.selected = None;
                    self.db.grid = Default::default();
                }
                Err(e) => self.db.message = e,
            }
        }
        if let Some(j) = &self.db.page_job
            && let Some(r) = j.poll()
        {
            self.db.page_job = None;
            match r {
                Ok(g) => {
                    self.db.message = format!("{} rows shown", g.rows.len());
                    self.db.grid = g;
                }
                Err(e) => self.db.message = e,
            }
        }
        self.pump_link();
        self.pump_solid();
        if self.build_job.is_some() || self.auto_job.is_some() || self.intake_job.is_some() || !self.test_jobs.is_empty() || self.db.list_job.is_some() || self.db.page_job.is_some() || self.poller.is_some() || !self.runs.is_empty() {
            ctx.request_repaint_after(std::time::Duration::from_millis(150));
        }
    }

    /// Where the tables get their values: the active link, else the start values in the sources.
    fn pump_link(&mut self) {
        let active = self.settings.active_link.clone();
        let conn = active.as_ref().and_then(|id| self.settings.connections.iter().find(|c| &c.id == id)).cloned();
        match conn {
            Some(c) if c.protocol == Protocol::OpcUa => {
                if self.poller.is_none() {
                    let pw = self.secrets.get(&c.id).unwrap_or("").to_string();
                    self.poller = Some(Poller::spawn(work::opcua_config(&c, &pw, &self.pki)));
                    self.link_state = LinkState::Connecting;
                }
                if let Some(p) = &self.poller {
                    while let Ok(m) = p.rx.try_recv() {
                        match m {
                            PollMsg::State(s) => self.link_state = s,
                            PollMsg::Values(v) => self.live = v,
                        }
                    }
                    // the table on show, and always the signals the model's joints follow
                    let mut keys: Vec<String> = self.tables.get(self.active_table).map(|t| t.rows.iter().map(|r| r.key.clone()).collect()).unwrap_or_default();
                    for b in &self.bindings {
                        if !keys.contains(&b.key) {
                            keys.push(b.key.clone());
                        }
                    }
                    p.want(keys);
                }
            }
            _ => {
                self.poller = None;
                self.live = self.start_values.clone();
                // the simulated machine: the sliders on the Model page stand in for the axes
                for b in &self.bindings {
                    if let Some(v) = self.pose.get(&b.joint) {
                        self.live.insert(b.key.clone(), ittoot_bridge::driver::Value::Real(*v));
                    }
                }
                self.link_state = if conn.is_some() { LinkState::Failed("No driver for this protocol".into()) } else { LinkState::Connected };
            }
        }
    }

    /// The model plan from the inferred sizes, the stations and the documents; the signal for each joint.
    fn update_model(&mut self) {
        use ittoot_bridge::intake::DocKind;
        if self.roles_key.is_empty() {
            self.roles_key = "read".into();
            self.roles.clear();
            self.roles_from.clear();
            for d in self.intake.docs.iter().filter(|d| matches!(d.kind, DocKind::Booklet | DocKind::Fds | DocKind::Dds | DocKind::Brief)) {
                if let Some(text) = ittoot_bridge::docread::read_text(&d.path) {
                    let r = ittoot_bridge::docread::axis_roles(&text);
                    if !r.is_empty() {
                        self.roles = r;
                        self.roles_from = d.path.display().to_string();
                        break;
                    }
                }
            }
        }
        let stations = station_points(&self.records);
        self.plan = model::plan(&self.inference, &stations, &self.roles);
        let key = format!("{:?}", self.plan.as_ref().map(|p| &p.joints));
        if key != self.plan_key {
            self.plan_key = key;
            self.auto_bindings = match &self.plan {
                Some(p) => model::auto_bind(p, &self.all_keys.iter().map(String::as_str).collect::<Vec<_>>()),
                None => Vec::new(),
            };
        }
        self.bindings = match &self.plan {
            Some(p) => model::effective(p, &self.auto_bindings, &self.settings.bindings),
            None => Vec::new(),
        };
    }

    /// Keep Solidcraft's joints at the machine's positions.
    fn pump_solid(&mut self) {
        let due = self.last_push.elapsed() >= std::time::Duration::from_millis(200);
        let cmds = match &self.plan {
            Some(p) if due => model::drive_commands(p, &self.bindings, &self.live),
            _ => Vec::new(),
        };
        let ids: Vec<String> = self.runs.keys().cloned().collect();
        for id in ids {
            // a Solidcraft this bridge started: once its control port answers, connect to it
            let ready = self.runs.get(&id).and_then(|r| r.wait.as_ref()).and_then(Job::poll);
            if let Some(up) = ready {
                let target = self.settings.models.iter().find(|m| m.id == id).cloned();
                let token = self.secrets.get(&format!("model:{id}")).unwrap_or("").to_string();
                let dir = self.deployment_dir_for(&id);
                if let (Some(r), Some(m)) = (self.runs.get_mut(&id), target) {
                    r.wait = None;
                    if up {
                        r.link = Some(work::SolidLink::spawn(m.address(), token, dir));
                        r.status = format!("connected to Solidcraft on {}", m.address());
                    } else {
                        r.status = format!("Solidcraft did not open its control port {} within a minute", m.port);
                    }
                }
            }
            let Some(r) = self.runs.get_mut(&id) else { continue };
            // the window was closed by hand
            if let Some(child) = r.child.as_mut()
                && let Ok(Some(code)) = child.try_wait()
            {
                r.child = None;
                r.link = None;
                r.push_live = false;
                r.status = format!("Solidcraft closed ({code})");
            }
            if let Some(s) = &r.link {
                while let Ok(m) = s.rx.try_recv() {
                    r.status = m;
                }
                if r.push_live && !cmds.is_empty() {
                    s.send(work::PushMsg::Drive(cmds.clone()));
                }
            }
        }
        if due {
            self.last_push = std::time::Instant::now();
        }
    }

    pub fn link_name(&self) -> String {
        match self.settings.active_link.as_ref().and_then(|id| self.settings.connections.iter().find(|c| &c.id == id)) {
            Some(c) => c.name.clone(),
            None => "Start values in the sources".into(),
        }
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        let ctx = &ctx;
        self.pump(ctx);
        self.draw(ui);
        let before = self.settings_path.is_some();
        if before && ctx.input(|i| i.pointer.any_released() || i.key_released(egui::Key::Enter)) {
            self.save();
        }
        self.screenshot_step(ctx);
        self.deployment_step(ctx);
    }
}

impl App {
    fn deployment_step(&mut self, ctx: &egui::Context) {
        if !self.save_deployment_and_quit {
            return;
        }
        ctx.request_repaint();
        let busy = self.build_job.is_some() || self.auto_job.is_some() || self.intake_job.is_some();
        if busy || (self.tables.is_empty() && !self.settings.areas.is_empty()) {
            return;
        }
        if let Some(plan) = self.plan.clone() {
            println!("{}", self.save_deployment(&plan));
            println!("folder: {}", self.deployment_dir().display());
        } else {
            println!("no model plan could be made");
        }
        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
    }

    fn screenshot_step(&mut self, ctx: &egui::Context) {
        let busy = self.build_job.is_some() || self.auto_job.is_some() || self.intake_job.is_some();
        let waiting = self.tables.is_empty() && !self.settings.areas.is_empty();
        let Some(shot) = &mut self.shot else { return };
        ctx.request_repaint();
        if busy || waiting {
            return;
        }
        shot.frames += 1;
        if shot.frames == 8 && !shot.requested {
            shot.requested = true;
            let slot = shot.result.clone();
            ctx.request_screenshot(move |img| {
                if let Ok(mut g) = slot.lock() {
                    *g = Some(img);
                }
            });
        }
        let image = shot.result.lock().ok().and_then(|g| g.clone());
        if let Some(img) = image {
            let [w, h] = img.size;
            let mut out = image::RgbaImage::new(w as u32, h as u32);
            for (n, px) in img.pixels.iter().enumerate() {
                out.put_pixel((n % w) as u32, (n / w) as u32, image::Rgba([px.r(), px.g(), px.b(), 255]));
            }
            let _ = out.save(&shot.path);
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }
}

fn arg(args: &[String], name: &str) -> Option<String> {
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned()
}

fn main() -> eframe::Result {
    let args: Vec<String> = std::env::args().collect();
    let shot_path = arg(&args, "--shot");
    let demo = arg(&args, "--demo");
    let page = arg(&args, "--page");
    let mut app = App::new(shot_path.is_none() && demo.is_none() && !args.iter().any(|a| a == "--save-deployment"));
    if let Some(p) = &demo {
        app.demo(p);
    }
    if let Some(p) = arg(&args, "--project") {
        app.use_project(&p);
    }
    if let Some(name) = page
        && let Some((p, _, _)) = Page::ALL.iter().find(|(_, _, key)| *key == name)
    {
        app.page = *p;
    }
    if let Some(n) = arg(&args, "--conn").and_then(|n| n.parse::<usize>().ok()) {
        app.sel_conn = n;
    }
    if let Some(t) = arg(&args, "--table") {
        app.pending_table = Some(t);
    }
    app.autorun = u8::from(args.iter().any(|a| a == "--go"));
    app.save_deployment_and_quit = args.iter().any(|a| a == "--save-deployment");
    if let Some(p) = shot_path {
        app.shot = Some(Shot { path: PathBuf::from(p), frames: 0, requested: false, result: Default::default() });
    }
    let size = arg(&args, "--size").and_then(|s| s.split_once('x').and_then(|(w, h)| Some([w.parse::<f32>().ok()?, h.parse::<f32>().ok()?]))).unwrap_or([1500.0, 920.0]);
    let options = eframe::NativeOptions { viewport: egui::ViewportBuilder::default().with_inner_size(size).with_min_inner_size([1000.0, 640.0]).with_title("ITTOOT Solidcraft - OT Connection Bridge"), ..Default::default() };
    eframe::run_native(
        "ittoot-bridge",
        options,
        Box::new(|cc| {
            theme::apply(&cc.egui_ctx);
            Ok(Box::new(app))
        }),
    )
}
