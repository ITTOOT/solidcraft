//! The IO Builder page: read an IO table and a circuit diagram, check one against the other, name the
//! signals, write an IO list for the PLC maker of your choice.

use crate::iowork::{self, BuildInput, BuildOutput};
use crate::theme::*;
use crate::widgets::*;
use crate::work::Job;
use crate::App;
use egui::{RichText, Ui};
use ittoot_bridge::discover::Suggestion;
use ittoot_io::export::{self, Format};
use ittoot_io::model::{IoPoint, Status};
use ittoot_io::naming::{self, Case, NameOptions, Postfix, Prefix, Scope};
use std::sync::{Arc, Mutex};

pub struct IoState {
    pub table_pdf: String,
    pub drawing_pdf: String,
    pub pages: String,
    pub deep: bool,
    pub job: Option<Job<Result<BuildOutput, String>>>,
    pub progress: Arc<Mutex<String>>,
    pub points: Vec<IoPoint>,
    pub notes: Vec<String>,
    pub summary: String,
    pub opts: NameOptions,
    pub custom_prefix: String,
    pub custom_post: String,
    pub format: Format,
    pub table_name: String,
    pub out_path: String,
    pub export_text: String,
    pub export_notes: Vec<String>,
    pub export_msg: String,
    pub filter: String,
    pub only: Option<Status>,
    pub selected: Option<usize>,
    pub suggestions: Vec<Suggestion>,
    pub suggestions_for: String,
    /// Your corrections, kept so a new reading of the drawings does not lose them.
    pub edits: Vec<ittoot_io::Edit>,
    pub edits_path: String,
}

impl Default for IoState {
    fn default() -> Self {
        IoState {
            table_pdf: String::new(),
            drawing_pdf: String::new(),
            pages: String::new(),
            deep: false,
            job: None,
            progress: Arc::new(Mutex::new(String::new())),
            points: Vec::new(),
            notes: Vec::new(),
            summary: String::new(),
            opts: NameOptions::default(),
            custom_prefix: "x".into(),
            custom_post: "01".into(),
            format: Format::SiemensTiaXml,
            table_name: "IO_Physical".into(),
            out_path: String::new(),
            export_text: String::new(),
            export_notes: Vec::new(),
            export_msg: String::new(),
            filter: String::new(),
            only: None,
            selected: None,
            suggestions: Vec::new(),
            suggestions_for: String::new(),
            edits: Vec::new(),
            edits_path: String::new(),
        }
    }
}

fn status_colour(s: Status) -> egui::Color32 {
    match s {
        Status::Confirmed => OK,
        Status::TableOnly => INFO,
        Status::DrawingOnly => WARN,
        Status::Differs => ERROR,
        Status::Spare => DIM,
    }
}

const PREFIXES: [(&str, u8); 6] = [("none", 0), ("i / q", 1), ("ix / qx (iw / qw)", 2), ("di / do / ai / ao", 3), ("x", 4), ("custom", 5)];
const POSTFIXES: [(&str, u8); 7] = [("none", 0), ("number (01, 02 ...) whole list", 1), ("number per station", 2), ("number per module", 3), ("area letter (A, B ...)", 4), ("address (20_3)", 5), ("custom", 6)];

fn prefix_index(p: &Prefix) -> u8 {
    match p {
        Prefix::None => 0,
        Prefix::IQ => 1,
        Prefix::IecX => 2,
        Prefix::DiDo => 3,
        Prefix::X => 4,
        Prefix::Custom(_) => 5,
    }
}

fn postfix_index(p: &Postfix) -> u8 {
    match p {
        Postfix::None => 0,
        Postfix::Number { scope: Scope::Global, .. } => 1,
        Postfix::Number { scope: Scope::Station, .. } => 2,
        Postfix::Number { scope: Scope::Module, .. } => 3,
        Postfix::AreaLetter => 4,
        Postfix::Address => 5,
        Postfix::ChannelLetter | Postfix::Custom(_) => 6,
    }
}

fn postfix_from(i: u8, custom: &str) -> Postfix {
    let num = |scope| Postfix::Number { start: 1, width: 2, scope };
    match i {
        1 => num(Scope::Global),
        2 => num(Scope::Station),
        3 => num(Scope::Module),
        4 => Postfix::AreaLetter,
        5 => Postfix::Address,
        6 => Postfix::Custom(custom.to_string()),
        _ => Postfix::None,
    }
}

impl App {
    /// PDFs in the document folders, for the file boxes' dropdowns.
    fn io_suggestions(&mut self) {
        let key = self.settings.document_folders.join("|");
        if self.io.suggestions_for == key {
            return;
        }
        self.io.suggestions_for = key;
        let mut v = Vec::new();
        for f in &self.settings.document_folders {
            for p in ittoot_bridge::table::files_under(std::path::Path::new(f)).into_iter().take(4000) {
                if p.extension().is_some_and(|e| e.eq_ignore_ascii_case("pdf")) {
                    v.push(Suggestion { value: p.display().to_string(), detail: p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default() });
                }
            }
        }
        self.io.suggestions = v;
    }

    pub fn page_io(&mut self, ui: &mut Ui) {
        self.io_suggestions();
        ui.horizontal(|ui| {
            ui.label(RichText::new("IO Builder").heading());
            ui.label(RichText::new("Reads the IO table and the circuit diagram, checks one against the other, names the signals and writes the IO list for your PLC.").color(DIM));
        });
        // collect a finished read
        if let Some(r) = self.io.job.as_ref().and_then(Job::poll) {
            self.io.job = None;
            match r {
                Ok(out) => {
                    self.io.summary = format!("{} channels read; {} drawing sheets looked at, {} show IO addresses", out.points.len(), out.sheets_read, out.sheets_with_addresses);
                    self.io.points = out.points;
                    self.io.notes = out.notes;
                    self.io.selected = None;
                    self.io_rename();
                }
                Err(e) => self.io.summary = format!("Reading failed: {e}"),
            }
        }
        ui.add_space(6.0);
        let mut read = false;
        let mut sources_changed = false;
        panel(ui, |ui| {
            ui.label(RichText::new("Sources").strong());
            egui::Grid::new("io-src").num_columns(2).spacing([12.0, 6.0]).show(ui, |ui| {
                ui.label("IO table (PDF)");
                sources_changed |= search_box(ui, ui.make_persistent_id("io-table"), &mut self.io.table_pdf, &self.io.suggestions, 760.0, "Pick or paste the IO table PDF");
                ui.end_row();
                ui.label("Circuit diagram (PDF)");
                sources_changed |= search_box(ui, ui.make_persistent_id("io-drawing"), &mut self.io.drawing_pdf, &self.io.suggestions, 760.0, "Pick or paste the electrical drawing PDF");
                ui.end_row();
                ui.label("Drawing pages");
                ui.horizontal(|ui| {
                    ui.add_sized([110.0, 24.0], egui::TextEdit::singleline(&mut self.io.pages).hint_text("all, or 51-83"));
                    ui.checkbox(&mut self.io.deep, "read every page in depth (slow); otherwise only pages where a quick look sees addresses");
                });
                ui.end_row();
            });
            ui.horizontal(|ui| {
                let busy = self.io.job.is_some();
                ui.add_enabled_ui(!busy, |ui| {
                    if start_button(ui, "READ THE DRAWINGS").clicked() {
                        read = true;
                    }
                });
                if busy {
                    let p = self.io.progress.lock().map(|g| g.clone()).unwrap_or_default();
                    ui.label(RichText::new(format!("working: {p}")).color(WARN));
                } else if !self.io.summary.is_empty() {
                    ui.label(RichText::new(&self.io.summary).color(INFO));
                }
            });
            ui.label(RichText::new("Scans are read with Windows' own OCR on this PC (nothing leaves it). OCR makes mistakes: every reading is kept as a draft you can correct in the table below.").color(DIM).size(12.0));
        });
        let _ = sources_changed;
        if read && self.io.job.is_none() {
            let range = {
                let t = self.io.pages.trim();
                if t.is_empty() || t.eq_ignore_ascii_case("all") {
                    None
                } else {
                    let (a, b) = t.split_once('-').unwrap_or((t, t));
                    a.trim().parse().ok().zip(b.trim().parse().ok())
                }
            };
            let input = BuildInput { table_pdf: self.io.table_pdf.clone(), drawing_pdf: self.io.drawing_pdf.clone(), drawing_pages: range, deep: self.io.deep };
            let progress = self.io.progress.clone();
            self.io.summary.clear();
            self.io.job = Some(Job::spawn(move || {
                iowork::build(&input, |m| {
                    if let Ok(mut g) = progress.lock() {
                        *g = m;
                    }
                })
            }));
        }

        ui.add_space(8.0);
        let mut changed = false;
        panel(ui, |ui| {
            ui.label(RichText::new("Names").strong());
            ui.horizontal_wrapped(|ui| {
                ui.label("Case");
                egui::ComboBox::from_id_salt("io-case").selected_text(self.io.opts.case.label()).show_ui(ui, |ui| {
                    for c in Case::ALL {
                        changed |= ui.selectable_value(&mut self.io.opts.case, c, c.label()).changed();
                    }
                });
                ui.label("Prefix");
                let mut pi = prefix_index(&self.io.opts.prefix);
                egui::ComboBox::from_id_salt("io-prefix").selected_text(PREFIXES.iter().find(|(_, i)| *i == pi).map(|(l, _)| *l).unwrap_or("none")).show_ui(ui, |ui| {
                    for (l, i) in PREFIXES {
                        changed |= ui.selectable_value(&mut pi, i, l).changed();
                    }
                });
                if pi != prefix_index(&self.io.opts.prefix) || (pi == 5 && matches!(&self.io.opts.prefix, Prefix::Custom(t) if *t != self.io.custom_prefix)) {
                    self.io.opts.prefix = match pi {
                        1 => Prefix::IQ,
                        2 => Prefix::IecX,
                        3 => Prefix::DiDo,
                        4 => Prefix::X,
                        5 => Prefix::Custom(self.io.custom_prefix.clone()),
                        _ => Prefix::None,
                    };
                }
                if pi == 5 {
                    changed |= ui.add_sized([60.0, 24.0], egui::TextEdit::singleline(&mut self.io.custom_prefix)).changed();
                    self.io.opts.prefix = Prefix::Custom(self.io.custom_prefix.clone());
                }
                for n in 1..=2 {
                    ui.label(format!("Postfix {n}"));
                    let cur = if n == 1 { postfix_index(&self.io.opts.postfix1) } else { postfix_index(&self.io.opts.postfix2) };
                    let mut pi = cur;
                    egui::ComboBox::from_id_salt(("io-post", n)).selected_text(POSTFIXES.iter().find(|(_, i)| *i == pi).map(|(l, _)| *l).unwrap_or("none")).show_ui(ui, |ui| {
                        for (l, i) in POSTFIXES {
                            changed |= ui.selectable_value(&mut pi, i, l).changed();
                        }
                    });
                    if pi == 6 {
                        changed |= ui.add_sized([60.0, 24.0], egui::TextEdit::singleline(&mut self.io.custom_post)).changed();
                    }
                    let new = postfix_from(pi, &self.io.custom_post);
                    if n == 1 {
                        self.io.opts.postfix1 = new;
                    } else {
                        self.io.opts.postfix2 = new;
                    }
                }
            });
            ui.horizontal(|ui| {
                changed |= ui.checkbox(&mut self.io.opts.strip_area_suffix, "drop the area letter the description ends with (-A)").changed();
                changed |= ui.checkbox(&mut self.io.opts.name_spares, "name spare channels (spare_i20_3)").changed();
                if let Some(p) = self.io.points.iter().find(|p| !p.words().is_empty()) {
                    ui.label(RichText::new(format!("e.g. {}  =  {}", p.address.siemens(), p.final_name())).color(INFO));
                }
            });
        });
        if changed {
            self.io_rename();
        }

        ui.add_space(8.0);
        self.io_table(ui);
        ui.add_space(8.0);
        self.io_export(ui);
    }

    /// Give every point its name again (names you typed stay).
    pub fn io_rename(&mut self) {
        naming::apply(&mut self.io.points, &self.io.opts);
    }

    fn io_table(&mut self, ui: &mut Ui) {
        let counts: Vec<(Status, usize)> = [Status::Confirmed, Status::TableOnly, Status::DrawingOnly, Status::Differs, Status::Spare].into_iter().map(|s| (s, self.io.points.iter().filter(|p| p.status == s).count())).collect();
        ui.horizontal(|ui| {
            ui.label("Show");
            if ui.selectable_label(self.io.only.is_none(), format!("all {}", self.io.points.len())).clicked() {
                self.io.only = None;
            }
            for (s, n) in counts {
                if ui.selectable_label(self.io.only == Some(s), format!("{} {n}", s.label())).clicked() {
                    self.io.only = Some(s);
                }
            }
            ui.add_sized([180.0, 24.0], egui::TextEdit::singleline(&mut self.io.filter).hint_text("filter: address, name, words"));
        });
        if self.io.points.is_empty() {
            ui.add_space(10.0);
            ui.label(RichText::new("Nothing read yet. Give the IO table and the circuit diagram above and press READ THE DRAWINGS.").color(DIM));
            return;
        }
        let f = self.io.filter.trim().to_ascii_lowercase();
        let rows: Vec<usize> = self
            .io
            .points
            .iter()
            .enumerate()
            .filter(|(_, p)| self.io.only.is_none_or(|s| p.status == s))
            .filter(|(_, p)| f.is_empty() || format!("{} {} {} {} {}", p.address.siemens(), p.final_name(), p.description, p.drawing_label, p.station).to_ascii_lowercase().contains(&f))
            .map(|(i, _)| i)
            .collect();
        panel(ui, |ui| {
            ui.horizontal(|ui| {
                for (h, w) in [("Station", 130.0), ("Address", 62.0), ("Module", 66.0), ("Pin", 28.0), ("IO table says", 230.0), ("Drawing shows", 170.0), ("Name", 190.0), ("Status", 86.0)] {
                    ui.add_sized([w, 18.0], egui::Label::new(RichText::new(h).color(DIM).size(12.0).strong()));
                }
            });
            let row_h = 26.0;
            let mut rename = false;
            egui::ScrollArea::vertical().max_height(360.0).auto_shrink([false, true]).show_rows(ui, row_h, rows.len(), |ui, range| {
                for r in range {
                    let Some(&i) = rows.get(r) else { continue };
                    let sel = self.io.selected == Some(i);
                    let Some(p) = self.io.points.get_mut(i) else { continue };
                    ui.horizontal(|ui| {
                        if sel {
                            ui.painter().rect_filled(ui.available_rect_before_wrap().shrink2(egui::vec2(0.0, 1.0)), 0.0, BG3);
                        }
                        let lbl = |ui: &mut Ui, t: &str, w: f32, c: egui::Color32| {
                            ui.add_sized([w, row_h - 4.0], egui::Label::new(RichText::new(t).color(c)).truncate()).clicked()
                        };
                        let mut clicked = lbl(ui, &p.station, 130.0, DIM);
                        clicked |= lbl(ui, &p.address.siemens(), 62.0, FG);
                        clicked |= lbl(ui, &p.module, 66.0, DIM);
                        clicked |= lbl(ui, &p.terminal.to_string(), 28.0, DIM);
                        let before = p.description.clone();
                        ui.add_sized([230.0, row_h - 4.0], egui::TextEdit::singleline(&mut p.description));
                        if p.description != before {
                            if p.status == Status::Spare && !p.description.is_empty() {
                                p.status = Status::TableOnly;
                            }
                            p.notes.push("description edited by you".into());
                            let (station, address, description) = (p.station.clone(), p.address.siemens(), p.description.clone());
                            self.io.edits.retain(|e| !(e.station == station && e.address == address));
                            self.io.edits.push(ittoot_io::Edit { station, address, description });
                            rename = true;
                        }
                        clicked |= lbl(ui, &p.drawing_label, 170.0, INFO);
                        let mut nm = p.final_name().to_string();
                        if ui.add_sized([190.0, row_h - 4.0], egui::TextEdit::singleline(&mut nm)).changed() {
                            p.name_override = if nm.is_empty() { None } else { Some(nm) };
                        }
                        let col = status_colour(p.status);
                        clicked |= ui.add_sized([86.0, row_h - 4.0], egui::Label::new(RichText::new(p.status.label()).color(col).strong())).clicked();
                        if !p.notes.is_empty() {
                            ui.label(RichText::new(format!("{} note(s)", p.notes.len())).color(DIM).size(11.0));
                        }
                        if clicked {
                            self.io.selected = Some(i);
                        }
                    });
                }
            });
            if rename {
                self.io_rename();
            }
        });
        if let Some(p) = self.io.selected.and_then(|i| self.io.points.get(i)) {
            panel(ui, |ui| {
                ui.label(RichText::new(format!("{}  {}  ({} / {})", p.address.siemens(), p.final_name(), p.station, p.module)).strong());
                for n in &p.notes {
                    ui.label(RichText::new(format!("\u{2022} {n}")).color(WARN).size(12.0));
                }
                for s in p.sources.iter().take(6) {
                    ui.label(RichText::new(format!("{} page {}: {}", s.doc, s.page, s.text)).color(DIM).size(11.0));
                }
            });
        }
        if !self.io.notes.is_empty() {
            egui::CollapsingHeader::new(RichText::new(format!("{} reading notes (pages skipped, addresses worked out)", self.io.notes.len())).color(DIM)).show(ui, |ui| {
                for n in &self.io.notes {
                    ui.label(RichText::new(n).color(DIM).size(11.0));
                }
            });
        }
    }

    fn io_export(&mut self, ui: &mut Ui) {
        panel(ui, |ui| {
            ui.label(RichText::new("Export").strong());
            ui.horizontal_wrapped(|ui| {
                ui.label("Format");
                egui::ComboBox::from_id_salt("io-format").width(430.0).selected_text(self.io.format.label()).show_ui(ui, |ui| {
                    for f in Format::ALL {
                        ui.selectable_value(&mut self.io.format, f, f.label());
                    }
                });
                ui.label("Table name");
                ui.add_sized([130.0, 24.0], egui::TextEdit::singleline(&mut self.io.table_name));
            });
            ui.horizontal(|ui| {
                ui.label("Save to");
                if self.io.out_path.is_empty() {
                    let name = self.io.points.iter().find(|p| !p.station.is_empty()).map(|_| "io_list".to_string()).unwrap_or_else(|| "io_list".into());
                    self.io.out_path = crate::data_dir().join("io").join(name).display().to_string();
                }
                ui.add_sized([560.0, 24.0], egui::TextEdit::singleline(&mut self.io.out_path));
                let ext = self.io.format.extension();
                if accent_button(ui, "Make the file").clicked() {
                    let e = export::render(self.io.format, &self.io.points, &self.io.table_name);
                    let mut path = std::path::PathBuf::from(&self.io.out_path);
                    if path.extension().is_none_or(|x| x != ext) {
                        path.set_extension(ext);
                    }
                    if let Some(dir) = path.parent() {
                        let _ = std::fs::create_dir_all(dir);
                    }
                    self.io.export_msg = match std::fs::write(&path, &e.text) {
                        Ok(()) => format!("{} channels written to {}", e.count, path.display()),
                        Err(err) => format!("Could not write {}: {err}", path.display()),
                    };
                    self.io.export_notes = e.notes;
                    self.io.export_text = e.text;
                }
                if ui.button("Preview").clicked() {
                    let e = export::render(self.io.format, &self.io.points, &self.io.table_name);
                    self.io.export_notes = e.notes;
                    self.io.export_text = e.text;
                    self.io.export_msg = format!("{} channels", e.count);
                }
            });
            ui.horizontal(|ui| {
                ui.label("Your corrections");
                if self.io.edits_path.is_empty() {
                    self.io.edits_path = crate::data_dir().join("io").join("io_edits.json").display().to_string();
                }
                ui.add_sized([560.0, 24.0], egui::TextEdit::singleline(&mut self.io.edits_path));
                if ui.button("Save").clicked() {
                    let path = std::path::PathBuf::from(&self.io.edits_path);
                    if let Some(dir) = path.parent() {
                        let _ = std::fs::create_dir_all(dir);
                    }
                    self.io.export_msg = match std::fs::write(&path, serde_json::to_string_pretty(&self.io.edits).unwrap_or_default()) {
                        Ok(()) => format!("{} corrections saved to {}", self.io.edits.len(), path.display()),
                        Err(e) => format!("Could not save: {e}"),
                    };
                }
                if ui.button("Load and apply").clicked() {
                    self.io.export_msg = match std::fs::read_to_string(&self.io.edits_path).map_err(|e| e.to_string()).and_then(|t| serde_json::from_str::<Vec<ittoot_io::Edit>>(&t).map_err(|e| e.to_string())) {
                        Ok(edits) => {
                            let n = ittoot_io::apply_edits(&mut self.io.points, &edits);
                            self.io.edits = edits;
                            self.io_rename();
                            format!("{n} corrections applied")
                        }
                        Err(e) => format!("Could not load: {e}"),
                    };
                }
            });
            if !self.io.export_msg.is_empty() {
                ui.label(RichText::new(&self.io.export_msg).color(INFO).size(12.0));
            }
            for n in &self.io.export_notes {
                ui.label(RichText::new(format!("! {n}")).color(WARN).size(12.0));
            }
            if !self.io.export_text.is_empty() {
                let shown: String = self.io.export_text.lines().take(24).collect::<Vec<_>>().join("\n");
                ui.add(egui::TextEdit::multiline(&mut shown.as_str()).desired_rows(8).desired_width(f32::INFINITY).font(egui::TextStyle::Monospace));
            }
        });
    }
}
