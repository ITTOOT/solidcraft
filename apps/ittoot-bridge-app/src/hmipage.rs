//! The HMI page: how to run the HMI beside the emulation, by manufacturer, with buttons that open the Windows settings window to change.

use crate::theme::*;
use crate::widgets::*;
use crate::App;
use egui::{RichText, Ui};
use ittoot_bridge::hmiinfo::{self, Opener, MAKERS};

#[derive(Default)]
pub struct HmiState {
    pub maker: usize,
    pub message: String,
}

impl App {
    pub fn page_hmi(&mut self, ui: &mut Ui) {
        ui.horizontal(|ui| {
            ui.label(RichText::new("HMI").heading());
            ui.label(RichText::new("How to connect the HMI so it shows the same machine as the model").color(DIM));
        });
        panel(ui, |ui| {
            ui.label(RichText::new("HMI and model are not synchronised until both run against the same emulation").strong().color(WARN));
            ui.label(hmiinfo::INTRO);
        });
        ui.add_space(6.0);
        let idx = self.hmi.maker.min(MAKERS.len() - 1);
        let mut picked = None;
        ui.horizontal(|ui| {
            ui.label(RichText::new("Manufacturer").strong());
            egui::ComboBox::from_id_salt("hmi-maker").width(240.0).selected_text(MAKERS[idx].name).show_ui(ui, |ui| {
                for (i, m) in MAKERS.iter().enumerate() {
                    if ui.selectable_label(idx == i, m.name).clicked() {
                        picked = Some(i);
                    }
                }
            });
        });
        if let Some(i) = picked {
            self.hmi.maker = i;
            self.hmi.message.clear();
        }
        let m = &MAKERS[self.hmi.maker.min(MAKERS.len() - 1)];
        ui.add_space(6.0);
        panel(ui, |ui| {
            ui.label(RichText::new(m.products).color(INFO));
            ui.add_space(4.0);
            for (n, s) in m.steps.iter().enumerate() {
                ui.horizontal_top(|ui| {
                    ui.label(RichText::new(format!("{}.", n + 1)).strong().color(ACCENT));
                    ui.add(egui::Label::new(*s).wrap());
                });
            }
            ui.add_space(4.0);
            ui.label(RichText::new(m.note).color(DIM).size(13.0));
        });
        ui.add_space(6.0);
        ui.horizontal_wrapped(|ui| {
            let mut openers: Vec<Opener> = Vec::new();
            if let Some(o) = m.opener {
                openers.push(o);
            }
            if !openers.contains(&Opener::NetworkAdapters) {
                openers.push(Opener::NetworkAdapters);
            }
            for o in openers {
                let ok = o.available();
                let b = ui.add_enabled(ok, egui::Button::new(RichText::new(o.label()).strong()));
                if !ok {
                    b.on_disabled_hover_text("This window is installed with the manufacturer's engineering software, which is not on this PC");
                } else if b.on_hover_text("Opens the window; you change the setting, the bridge never does").clicked() {
                    self.hmi.message = match o.open() {
                        Ok(()) => format!("{} opened. Change the setting there, then start the HMI runtime.", o.label().trim_start_matches("Open ")),
                        Err(e) => format!("Could not open it: {e}"),
                    };
                }
            }
        });
        if !self.hmi.message.is_empty() {
            ui.label(RichText::new(&self.hmi.message).color(DIM).size(13.0));
        }
    }
}
