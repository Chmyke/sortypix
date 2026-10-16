//! Shared color palette of the interface.
use eframe::egui::{self, Color32, Stroke};
pub const BG: Color32 = Color32::from_rgb(13, 17, 23);
pub const CARD: Color32 = Color32::from_rgb(22, 27, 34);
pub const CARD2: Color32 = Color32::from_rgb(28, 35, 44);
pub const LINE: Color32 = Color32::from_rgb(40, 48, 57);
pub const SLIDER_TRACK: Color32 = Color32::from_rgb(61, 72, 86);
pub const TEXT: Color32 = Color32::from_rgb(230, 237, 243);
pub const DIM: Color32 = Color32::from_rgb(139, 152, 165);
pub const BLUE: Color32 = Color32::from_rgb(88, 166, 255);
pub const GREEN: Color32 = Color32::from_rgb(63, 185, 80);
pub const RED: Color32 = Color32::from_rgb(248, 81, 73);
pub const AMBER: Color32 = Color32::from_rgb(210, 153, 34);
pub fn apply(ctx: &egui::Context) {
    ctx.set_theme(egui::Theme::Dark);
    let mut style = (*ctx.style_of(egui::Theme::Dark)).clone();
    style.visuals = egui::Visuals::dark();
    style.visuals.panel_fill = BG;
    style.visuals.window_fill = CARD;
    style.visuals.extreme_bg_color = BG;
    style.visuals.override_text_color = Some(TEXT);
    style.visuals.weak_text_color = Some(DIM);
    style.visuals.selection.bg_fill = CARD2;
    style.visuals.selection.stroke = Stroke::new(1., BLUE);
    style.visuals.hyperlink_color = BLUE;
    for widget in [
        &mut style.visuals.widgets.inactive,
        &mut style.visuals.widgets.noninteractive,
        &mut style.visuals.widgets.hovered,
        &mut style.visuals.widgets.active,
        &mut style.visuals.widgets.open,
    ] {
        widget.bg_fill = CARD2;
        widget.weak_bg_fill = CARD2;
        widget.bg_stroke = Stroke::new(1., LINE);
        widget.fg_stroke = Stroke::new(1., TEXT);
        widget.expansion = 0.;
    }
    style.visuals.widgets.hovered.bg_stroke = Stroke::new(1., BLUE);
    style.visuals.widgets.active.bg_stroke = Stroke::new(1., BLUE);
    style.spacing.item_spacing = egui::vec2(12., 10.);
    style.spacing.button_padding = egui::vec2(14., 9.);
    ctx.set_style_of(egui::Theme::Dark, style);
}
