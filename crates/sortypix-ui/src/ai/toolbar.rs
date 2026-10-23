//! ROLE: Sortypix AI only. Toolbar controls of the design board (2026-10-22, all A1): "Suggest"
//! slider and "Reject suggested (n)", search field, order menu, progress line, "AI off".
//! OWNER: `Gallery::toolbar` calls these from its right-to-left row, behind `cfg(feature = "ai")`.
//! INVARIANTS: "Reject suggested" goes through `Gallery::decide`: one gesture, one Ctrl+Z.
//! Controls that do not fit are left out, the button last.
use super::filters::{self, Kind};
use super::suggest::{PERCENT_MAX, PERCENT_STEP, reject_ids};
use super::view::Order;
use super::{SEARCH_ID, State, prefs};
use crate::gallery::Gallery;
use crate::icons::{self, Icon};
use crate::theme;
use eframe::egui::{self, Align, Layout, Rect, RichText, Sense, Stroke};
use sortypix_core::Decision;
use std::time::Instant;

/// Widths reserved for the controls, so they never push the folder icons off the bar.
const BUTTON_WIDTH: f32 = 170.;
const SLIDER_GROUP_WIDTH: f32 = 150.;
const SEARCH_WIDTH: f32 = 180.;
/// Left for the photo count and the folder name.
const RESERVED: f32 = 140.;

/// `1240` as "1,240" (the board's English figures).
pub(crate) fn grouped(n: usize) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// Magnifier of the search field, drawn like the other vector icons.
fn magnifier(painter: &egui::Painter, rect: Rect, color: egui::Color32) {
    let stroke = Stroke::new(1.5, color);
    let centre = rect.center() + egui::vec2(-1.5, -1.5);
    painter.circle_stroke(centre, 4.5, stroke);
    painter.line_segment(
        [centre + egui::vec2(3.3, 3.3), centre + egui::vec2(7., 7.)],
        stroke,
    );
}

/// The cross that clears the field.
fn cross(painter: &egui::Painter, rect: Rect, color: egui::Color32) {
    let stroke = Stroke::new(1.5, color);
    let r = Rect::from_center_size(rect.center(), egui::vec2(8., 8.));
    painter.line_segment([r.left_top(), r.right_bottom()], stroke);
    painter.line_segment([r.right_top(), r.left_bottom()], stroke);
}

impl Gallery {
    /// Right of the photo count, drawn right to left: "Reject suggested (n)", the share, the
    /// slider and "Suggest"; "AI off" when the model could not load.
    pub(crate) fn ai_controls(&mut self, ui: &mut egui::Ui, busy: bool) {
        self.ai.room = super::Room::default();
        self.ai.targets.reject = None;
        if self.viewer {
            return;
        }
        if let State::Off(why) = self.ai.state() {
            ui.label(RichText::new("AI off").color(theme::DIM)).on_hover_text(format!(
                "The AI model could not start, so there are no suggestions and no search.\nSorting works as usual.\n\n{why}"
            ));
            return;
        }
        let mut room = ui.available_width() - RESERVED;
        if room < BUTTON_WIDTH {
            return;
        }
        room -= BUTTON_WIDTH;
        let count = self.ai.marks.suggested.len();
        let button = egui::Button::new(
            RichText::new(format!("Reject suggested ({})", grouped(count))).color(theme::RED),
        )
        .stroke(Stroke::new(1., theme::RED));
        let response = ui
            .scope(|ui| {
                ui.spacing_mut().button_padding = egui::vec2(8., 3.);
                ui.add_enabled(!busy && count > 0, button)
            })
            .inner
            .on_hover_text("Move the suggested photos to rejected/ · Ctrl+Z puts them back")
            .on_disabled_hover_text(if busy {
                "Working…"
            } else {
                "No photo is suggested for now"
            });
        self.ai.targets.reject = Some(response.rect.center());
        if response.clicked() {
            let shown: Vec<_> = self.photos.iter().map(|p| p.id).collect();
            let ids = reject_ids(&self.ai.marks.suggested, &shown);
            self.decide(ids, Decision::Reject);
        }
        if room < SLIDER_GROUP_WIDTH {
            return;
        }
        room -= SLIDER_GROUP_WIDTH;
        self.ai.room.slider = true;
        let mut percent = prefs::suggest_percent(&self.settings);
        ui.monospace(format!("{percent}%"));
        let slider = ui
            .scope(|ui| {
                ui.spacing_mut().slider_width = 56.;
                ui.visuals_mut().widgets.inactive.bg_fill = theme::SLIDER_TRACK;
                ui.visuals_mut().selection.bg_fill = theme::RED;
                ui.add(
                    egui::Slider::new(&mut percent, 0..=PERCENT_MAX)
                        .step_by(f64::from(PERCENT_STEP))
                        .trailing_fill(true)
                        .show_value(false),
                )
            })
            .inner;
        let tip = "Share of the analysed photos suggested for rejection, lowest scores first, plus the off-topic ones";
        if slider.on_hover_text(tip).changed() {
            prefs::set_suggest_percent(&mut self.settings, percent);
            self.ai_marks_changed();
        }
        ui.label(RichText::new("Suggest").color(theme::DIM))
            .on_hover_text(tip);
        self.ai.room.search = room >= SEARCH_WIDTH;
    }

    /// "14 of 3,260" in place of the photo count while a search or a "show only" filter is
    /// applied.
    pub(crate) fn ai_count_label(&self) -> Option<String> {
        let narrowed = self.ai.search.query().is_some() || self.ai_show().is_some();
        (narrowed && !self.viewer).then(|| {
            format!(
                "{} of {}",
                grouped(self.photos.len()),
                grouped(self.ai.total())
            )
        })
    }

    /// Left of the photo count: the search field, with its clearing cross.
    pub(crate) fn ai_search_field(&mut self, ui: &mut egui::Ui) {
        self.ai.targets.search = None;
        self.ai.targets.clear = None;
        if self.viewer || !self.ai.room.search {
            return;
        }
        let id = egui::Id::new(SEARCH_ID);
        let focused = ui.ctx().memory(|m| m.has_focus(id));
        let active = focused || self.ai.search.query().is_some();
        let now = Instant::now();
        ui.allocate_ui_with_layout(
            egui::vec2(SEARCH_WIDTH, 26.),
            Layout::left_to_right(Align::Center),
            |ui| {
                egui::Frame::new()
                    .fill(theme::CARD)
                    .stroke(Stroke::new(
                        1.,
                        if active { theme::BLUE } else { theme::LINE },
                    ))
                    .corner_radius(5)
                    .inner_margin(egui::Margin::symmetric(6, 3))
                    .show(ui, |ui| {
                        ui.spacing_mut().item_spacing.x = 4.;
                        let (icon, _) = ui.allocate_exact_size(egui::vec2(16., 16.), Sense::hover());
                        magnifier(ui.painter(), icon, theme::DIM);
                        let edit = egui::TextEdit::singleline(&mut self.ai.search.text)
                            .id(id)
                            .frame(egui::Frame::NONE)
                            .margin(egui::Margin::ZERO)
                            .hint_text(RichText::new("Search").color(theme::DIM))
                            .desired_width(SEARCH_WIDTH - 12. - 16. - 18. - 8.);
                        let response = ui.add(edit);
                        if response.changed() {
                            self.ai.search.edited(now);
                        }
                        if response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                            self.ai.search.submit();
                        }
                        self.ai.targets.search = Some(response.rect.center());
                        let tip = if let Some(error) = &self.ai.search.error {
                            format!("Search failed: {error}")
                        } else if self.ai.search.waiting() {
                            "Searching…".to_string()
                        } else {
                            "Find photos by what they show, in English (cat, beach, red car). Photos not analysed yet join as they are.".to_string()
                        };
                        response.on_hover_text(tip);
                        let (clear, clear_response) =
                            ui.allocate_exact_size(egui::vec2(16., 16.), Sense::click());
                        if !self.ai.search.text.is_empty() {
                            let color = if clear_response.hovered() {
                                theme::BLUE
                            } else {
                                theme::DIM
                            };
                            cross(ui.painter(), clear, color);
                            self.ai.targets.clear = Some(clear.center());
                            if clear_response
                                .on_hover_text("Clear the search")
                                .clicked()
                            {
                                self.ai.search.clear();
                            }
                        }
                    });
            },
        );
    }

    /// The order menu, in place of Sortypix's oldest/newest toggle.
    pub(crate) fn ai_order_menu(&mut self, ui: &mut egui::Ui) {
        let how = self.ai_arrangement();
        let model = !matches!(self.ai.state(), State::Off(_));
        let current = if how.searching && how.best_match {
            "Best match"
        } else {
            how.order.label()
        };
        let icon = if how.order == Order::Newest && !(how.searching && how.best_match) {
            Icon::Descending
        } else {
            Icon::Ascending
        };
        let show = self.ai_show();
        let hidden = prefs::hidden(&self.settings);
        let tip = match show {
            Some(kind) => format!(
                "{current}, {} · Change the order or what is shown",
                filters::show_label(Some(kind)).to_lowercase()
            ),
            None => format!("{current} · Change the order or what is shown"),
        };
        let response = icons::button(ui, icon, &tip, true);
        self.ai.targets.order = Some(response.rect.center());
        self.ai.targets.entries.clear();
        self.ai.targets.shows.clear();
        self.ai.targets.toggles.clear();
        let mut chosen: Option<Option<Order>> = None;
        let mut entries = Vec::new();
        let mut shows = Vec::new();
        let mut toggles = Vec::new();
        let mut show_chosen: Option<Option<Kind>> = None;
        let mut toggled: Option<Kind> = None;
        egui::Popup::menu(&response).show(|ui| {
            ui.spacing_mut().button_padding = egui::vec2(10., 4.);
            ui.spacing_mut().item_spacing.y = 2.;
            let entry = |ui: &mut egui::Ui, label: &str, checked: bool, enabled: bool| {
                // The default fonts have no check mark glyph: it is drawn in the left padding.
                let text = RichText::new(format!("     {label}"));
                let text = if checked {
                    text.color(theme::BLUE)
                } else {
                    text
                };
                let response = ui.add_enabled(enabled, egui::Button::new(text).frame(false));
                if checked {
                    let c = response.rect.left_center() + egui::vec2(9., 0.);
                    let stroke = Stroke::new(1.6, theme::BLUE);
                    let points = vec![
                        c + egui::vec2(-4., 0.),
                        c + egui::vec2(-1.5, 3.),
                        c + egui::vec2(4., -3.5),
                    ];
                    ui.painter().add(egui::Shape::line(points, stroke));
                }
                (response.clicked(), response.rect.center())
            };
            let (clicked, at) = entry(
                ui,
                "Best match (while searching)",
                how.searching && how.best_match,
                how.searching,
            );
            entries.push((None, at));
            if clicked {
                chosen = Some(None);
            }
            ui.separator();
            for order in Order::ALL {
                let checked = how.order == order && !(how.searching && how.best_match);
                let (clicked, at) =
                    entry(ui, order.label(), checked, model || !order.needs_model());
                entries.push((Some(order), at));
                if clicked {
                    chosen = Some(Some(order));
                }
            }
            ui.separator();
            ui.label(RichText::new("     Show").small().color(theme::DIM));
            for kind in [
                None,
                Some(Kind::Low),
                Some(Kind::OffTopic),
                Some(Kind::Burst),
            ] {
                let (clicked, at) = entry(ui, filters::show_label(kind), show == kind, model);
                shows.push((kind, at));
                if clicked {
                    show_chosen = Some(kind);
                }
            }
            ui.separator();
            ui.label(RichText::new("     Badges").small().color(theme::DIM));
            for kind in Kind::ALL {
                let label = format!("{} badge", kind.label());
                let (clicked, at) = entry(ui, &label, !hidden.has(kind), model);
                toggles.push((kind, at));
                if clicked {
                    toggled = Some(kind);
                }
            }
        });
        self.ai.targets.entries = entries;
        self.ai.targets.shows = shows;
        self.ai.targets.toggles = toggles;
        if let Some(kind) = toggled {
            let mut hidden = hidden;
            hidden.set(kind, !hidden.has(kind));
            prefs::set_hidden(&mut self.settings, hidden);
            return;
        }
        if let Some(kind) = show_chosen {
            prefs::set_show(&mut self.settings, kind);
            self.ai_arrange();
            self.selected = 0;
            self.grid_scroll_to_focus = true;
            return;
        }
        match chosen {
            Some(None) => self.ai.search.best_match = true,
            Some(Some(order)) => {
                prefs::set_order(&mut self.settings, order);
                self.ai.search.best_match = false;
            }
            None => return,
        }
        self.ai_arrange();
        self.selected = 0;
    }

    /// The 2 px progress line over the toolbar's separator while photos are analysed.
    pub(crate) fn ai_progress_line(&mut self, ui: &mut egui::Ui, separator: Rect) {
        let Some((done, total)) = self.ai.line else {
            return;
        };
        self.ai.line_shown = true;
        let y = separator.center().y;
        let share = done as f32 / total.max(1) as f32;
        let line = Rect::from_min_max(
            egui::pos2(separator.left(), y - 1.),
            egui::pos2(separator.left() + separator.width() * share, y + 1.),
        );
        ui.painter().rect_filled(line, 0, theme::BLUE);
        let band = Rect::from_min_max(
            egui::pos2(separator.left(), y - 2.),
            egui::pos2(separator.right(), y + 2.),
        );
        ui.interact(band, egui::Id::new("sortypix-ai-progress"), Sense::hover())
            .on_hover_text(format!(
                "Analyzing {} / {} pics",
                grouped(done),
                grouped(total)
            ));
    }
}

#[cfg(test)]
mod tests {
    use super::grouped;

    #[test]
    fn figures_are_grouped_by_thousands() {
        assert_eq!(grouped(0), "0");
        assert_eq!(grouped(999), "999");
        assert_eq!(grouped(1240), "1,240");
        assert_eq!(grouped(3260), "3,260");
        assert_eq!(grouped(1_234_567), "1,234,567");
    }
}
