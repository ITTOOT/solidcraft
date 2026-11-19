//! Small shared controls: chips, headings, and the search box with a dropdown (the OT Analyser's
//! `Combo`: the arrow lists everything, typing narrows the list, a click picks).

use crate::theme::*;
use egui::{Align2, Color32, FontId, Id, Margin, RichText, Sense, Stroke, Ui};
use ittoot_bridge::discover::{filter_suggestions, Suggestion};

pub fn chip(ui: &mut Ui, text: &str, color: Color32) {
    egui::Frame::new()
        .fill(color.gamma_multiply(0.18))
        .stroke(Stroke::new(1.0, color))
        .inner_margin(Margin::symmetric(6, 1))
        .show(ui, |ui| {
            ui.label(RichText::new(text).color(color).size(12.0));
        });
}

pub fn heading(ui: &mut Ui, text: &str) {
    ui.add_space(4.0);
    ui.label(RichText::new(text.to_uppercase()).color(DIM).size(12.0).strong());
    ui.add_space(2.0);
}

pub fn accent_button(ui: &mut Ui, text: &str) -> egui::Response {
    ui.add(egui::Button::new(RichText::new(text).color(Color32::BLACK).strong()).fill(ACCENT).stroke(Stroke::new(1.0, ACCENT)))
}

/// The big orange button that starts something (Solidcraft): the IT-TO-OT orange, larger than the rest.
pub fn start_button(ui: &mut Ui, text: &str) -> egui::Response {
    ui.add(
        egui::Button::new(RichText::new(text).color(Color32::BLACK).strong().size(18.0))
            .fill(ACCENT)
            .stroke(Stroke::new(2.0, Color32::from_rgb(0xff, 0xc0, 0x60)))
            .min_size(egui::vec2(240.0, 42.0)),
    )
}

pub fn panel(ui: &mut Ui, add: impl FnOnce(&mut Ui)) {
    egui::Frame::new().fill(BG2).stroke(Stroke::new(1.0, LINE)).inner_margin(Margin::same(10)).show(ui, add);
}

/// A text box with an arrow. Returns true when the text changed (typed or picked).
pub fn search_box(ui: &mut Ui, id: Id, text: &mut String, items: &[Suggestion], width: f32, hint: &str) -> bool {
    let open_id = id.with("open");
    let narrow_id = id.with("narrow");
    let mut open: bool = ui.data(|d| d.get_temp(open_id)).unwrap_or(false);
    let mut narrow: bool = ui.data(|d| d.get_temp(narrow_id)).unwrap_or(false);
    let mut changed = false;

    let arrow_w = 26.0;
    let (te, arrow, full) = ui
        .horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 0.0;
            let te = ui.add_sized([(width - arrow_w).max(40.0), 24.0], egui::TextEdit::singleline(text).hint_text(hint).id(id));
            let arrow = ui.add_sized([arrow_w, 24.0], egui::Button::new(""));
            {
                // a small down triangle, drawn: the interface font has no arrow glyph
                let c = arrow.rect.center();
                ui.painter().add(egui::Shape::convex_polygon(vec![egui::pos2(c.x - 5.0, c.y - 2.5), egui::pos2(c.x + 5.0, c.y - 2.5), egui::pos2(c.x, c.y + 3.5)], FG, Stroke::NONE));
            }
            let full = te.rect.union(arrow.rect);
            (te, arrow, full)
        })
        .inner;
    if te.changed() {
        changed = true;
        open = true;
        narrow = true;
    }
    if arrow.clicked() {
        open = !open;
        narrow = false;
    }
    if te.has_focus() && ui.input(|i| i.key_pressed(egui::Key::ArrowDown)) {
        open = true;
        narrow = false;
    }

    if open {
        let shown = filter_suggestions(items, text, narrow);
        if narrow && shown.is_empty() {
            open = false;
        } else {
            let mut picked: Option<String> = None;
            let area = egui::Area::new(id.with("list")).order(egui::Order::Foreground).fixed_pos(full.left_bottom()).show(ui.ctx(), |ui| {
                egui::Frame::popup(ui.style()).fill(BG2).stroke(Stroke::new(1.0, ACCENT)).show(ui, |ui| {
                    ui.set_width(width.max(360.0));
                    egui::ScrollArea::vertical().max_height(280.0).show(ui, |ui| {
                        if shown.is_empty() {
                            ui.label(RichText::new("Nothing to choose from yet. Run Auto-config or add a project.").color(DIM));
                        }
                        for s in shown.iter().take(300) {
                            let r = ui.add(egui::Button::new("").frame(false).min_size(egui::vec2(ui.available_width(), 38.0)));
                            let p = ui.painter();
                            if r.hovered() {
                                p.rect_filled(r.rect, 0.0, BG3);
                            }
                            p.text(r.rect.left_top() + egui::vec2(6.0, 3.0), Align2::LEFT_TOP, &s.detail, FontId::proportional(14.0), FG);
                            p.text(r.rect.left_top() + egui::vec2(6.0, 21.0), Align2::LEFT_TOP, &s.value, FontId::proportional(11.0), DIM);
                            if r.clicked() {
                                picked = Some(s.value.clone());
                            }
                        }
                    });
                });
            });
            if let Some(v) = picked {
                *text = v;
                changed = true;
                open = false;
            } else if ui.input(|i| i.pointer.any_pressed()) {
                let pos = ui.input(|i| i.pointer.interact_pos());
                if let Some(p) = pos
                    && !full.contains(p)
                    && !area.response.rect.contains(p)
                {
                    open = false;
                }
            }
            if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                open = false;
            }
        }
    }
    ui.data_mut(|d| {
        d.insert_temp(open_id, open);
        d.insert_temp(narrow_id, narrow);
    });
    let _ = Sense::click();
    changed
}
