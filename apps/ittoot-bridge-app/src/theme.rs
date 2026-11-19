//! The house look from the OT Data Analyser: gunmetal grey, square, flat, no shadows.

use egui::{Color32, CornerRadius, FontData, FontDefinitions, FontFamily, FontId, Stroke, TextStyle};

pub const BG: Color32 = Color32::from_rgb(0x1b, 0x1f, 0x23);
pub const BG2: Color32 = Color32::from_rgb(0x23, 0x28, 0x2d);
pub const BG3: Color32 = Color32::from_rgb(0x2c, 0x32, 0x38);
pub const LINE: Color32 = Color32::from_rgb(0x43, 0x4b, 0x53);
pub const FG: Color32 = Color32::from_rgb(0xd3, 0xd8, 0xde);
pub const DIM: Color32 = Color32::from_rgb(0x8b, 0x94, 0x9e);
pub const ACCENT: Color32 = Color32::from_rgb(0xf0, 0xa0, 0x30);
pub const ERROR: Color32 = Color32::from_rgb(0xe0, 0x55, 0x4f);
pub const WARN: Color32 = Color32::from_rgb(0xe6, 0xc8, 0x4a);
pub const INFO: Color32 = Color32::from_rgb(0x6f, 0xa3, 0xd0);
pub const OK: Color32 = Color32::from_rgb(0x62, 0xb3, 0x6a);

pub fn apply(ctx: &egui::Context) {
    let mut fonts = FontDefinitions::default();
    // Bahnschrift is the Analyser's face; it ships with Windows. Without it the bundled font is used.
    if let Ok(bytes) = std::fs::read(r"C:\Windows\Fonts\bahnschrift.ttf") {
        fonts.font_data.insert("bahnschrift".into(), std::sync::Arc::new(FontData::from_owned(bytes)));
        if let Some(list) = fonts.families.get_mut(&FontFamily::Proportional) {
            list.insert(0, "bahnschrift".into());
        }
    }
    ctx.set_fonts(fonts);

    let mut v = egui::Visuals::dark();
    v.panel_fill = BG;
    v.window_fill = BG2;
    v.extreme_bg_color = BG;
    v.faint_bg_color = BG2;
    v.code_bg_color = BG3;
    v.override_text_color = Some(FG);
    v.hyperlink_color = INFO;
    v.warn_fg_color = WARN;
    v.error_fg_color = ERROR;
    v.window_stroke = Stroke::new(1.0, LINE);
    v.window_corner_radius = CornerRadius::ZERO;
    v.menu_corner_radius = CornerRadius::ZERO;
    v.window_shadow = egui::Shadow::NONE;
    v.popup_shadow = egui::Shadow::NONE;
    v.selection.bg_fill = ACCENT.gamma_multiply(0.35);
    v.selection.stroke = Stroke::new(1.0, ACCENT);
    let w = &mut v.widgets;
    for (state, fill) in [(&mut w.noninteractive, BG2), (&mut w.inactive, BG3), (&mut w.hovered, LINE), (&mut w.active, LINE), (&mut w.open, BG3)] {
        state.bg_fill = fill;
        state.weak_bg_fill = fill;
        state.corner_radius = CornerRadius::ZERO;
        state.bg_stroke = Stroke::new(1.0, LINE);
        state.fg_stroke = Stroke::new(1.0, FG);
        state.expansion = 0.0;
    }
    w.hovered.bg_stroke = Stroke::new(1.0, ACCENT);
    w.active.bg_stroke = Stroke::new(1.0, ACCENT);
    ctx.set_visuals(v);

    ctx.global_style_mut(|s| {
        s.spacing.item_spacing = egui::vec2(8.0, 6.0);
        s.spacing.button_padding = egui::vec2(10.0, 4.0);
        s.spacing.interact_size.y = 24.0;
        s.text_styles.insert(TextStyle::Body, FontId::new(14.0, FontFamily::Proportional));
        s.text_styles.insert(TextStyle::Button, FontId::new(14.0, FontFamily::Proportional));
        s.text_styles.insert(TextStyle::Heading, FontId::new(20.0, FontFamily::Proportional));
        s.text_styles.insert(TextStyle::Small, FontId::new(12.0, FontFamily::Proportional));
        s.text_styles.insert(TextStyle::Monospace, FontId::new(13.0, FontFamily::Monospace));
    });
}
