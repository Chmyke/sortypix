use crate::{
    gallery::Gallery,
    icons::{self, Icon},
    shortcuts::Action,
    theme,
};
use eframe::egui::{self, Sense};
use sortypix_app::worker::Command;
use sortypix_core::Decision;
use std::time::Instant;
impl Gallery {
    pub(crate) fn toolbar(&mut self, ui: &mut egui::Ui) {
        let now = Instant::now();
        let busy = crate::feedback::busy_shown(self.busy_since, now);
        let glow = |target: Decision, flash: Option<(Decision, Instant)>| {
            flash
                .filter(|(decision, _)| *decision == target)
                .map_or(0., |(_, at)| crate::feedback::glow(at, now))
        };
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.;
            ui.set_min_height(32.);
            if icons::button(ui, Icon::Folder, &self.shortcut_tip(Action::Open), !busy).clicked()
            {
                self.choose();
            }
            for (icon, action, lit) in [
                (Icon::SourceFolder, Action::OpenSource, 0.),
                (
                    Icon::KeptFolder,
                    Action::OpenKept,
                    glow(Decision::Keep, self.flash),
                ),
                (
                    Icon::RejectedFolder,
                    Action::OpenRejected,
                    glow(Decision::Reject, self.flash),
                ),
            ] {
                if icons::lit_button(
                    ui,
                    icon,
                    &self.shortcut_tip(action),
                    self.root.is_some(),
                    lit,
                )
                .clicked()
                {
                    self.perform(action, ui.ctx());
                }
            }
            ui.separator();
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let help = icons::button(
                    ui,
                    Icon::Keyboard,
                    "Edit shortcuts · Mouse & keyboard",
                    true,
                );
                if let Some(d) = &mut self.diagnostic {
                    d.help_target = Some(help.rect.center());
                }
                if help.clicked() {
                    self.shortcut_editor.open = true;
                }
                ui.separator();
                for (icon, action) in [
                    (Icon::RejectAll, Action::RejectAll),
                    (Icon::KeepAll, Action::KeepAll),
                ] {
                    let response = icons::button(
                        ui,
                        icon,
                        &self.shortcut_tip(action),
                        !busy && !self.photos.is_empty(),
                    );
                    if let Some(d) = &mut self.diagnostic {
                        if action == Action::KeepAll {
                            d.keep_all_target = Some(response.rect.center());
                        } else {
                            d.reject_all_target = Some(response.rect.center());
                        }
                    }
                    if response.clicked() {
                        self.perform(action, ui.ctx());
                    }
                }
                let (mouse, response) =
                    ui.allocate_exact_size(egui::vec2(32., 32.), Sense::click());
                if response.clicked() {
                    self.settings.inverted = !self.settings.inverted;
                    let [left, right] = crate::mouse::legend(self.settings.inverted);
                    self.notice = Some((format!("Buttons swapped · {left} · {right}"), now));
                }
                if let Some(d) = &mut self.diagnostic {
                    d.mouse_target = Some(mouse.center());
                }
                crate::mouse::paint(
                    ui.painter(),
                    egui::Rect::from_center_size(mouse.center(), crate::mouse::visual_size(32.)),
                    self.settings.inverted,
                );
                let [left, right] = crate::mouse::legend(self.settings.inverted);
                response
                    .on_hover_cursor(egui::CursorIcon::PointingHand)
                    .on_hover_text(format!(
                        "{left} · {right}\nMiddle / Ctrl+click: reject through\nClick to swap buttons"
                    ));
                ui.separator();
                if self.viewer {
                    if icons::button(
                        ui,
                        if self.zoom { Icon::Fit } else { Icon::Zoom },
                        &self.shortcut_tip(Action::Zoom),
                        true,
                    )
                    .clicked()
                    {
                        self.zoom = !self.zoom;
                        self.pan = egui::Vec2::ZERO;
                    }
                } else {
                    size_marker(ui, Icon::ThumbLarge);
                    size_slider(ui, &mut self.settings.thumbnail_size);
                    size_marker(ui, Icon::ThumbSmall);
                    let icon = if self.settings.newest_first {
                        Icon::Descending
                    } else {
                        Icon::Ascending
                    };
                    if icons::button(
                        ui,
                        icon,
                        if self.settings.newest_first {
                            "Newest first · Switch to oldest first"
                        } else {
                            "Oldest first · Switch to newest first"
                        },
                        true,
                    )
                    .clicked()
                    {
                        self.settings.newest_first = !self.settings.newest_first;
                        self.photos.reverse();
                        self.layout.invalidate();
                        self.turns.reverse();
                        self.selected = 0;
                    }
                }
                ui.separator();
                let undo = icons::button(
                    ui,
                    Icon::Undo,
                    &format!("{} ({})", self.shortcut_tip(Action::Undo), self.undo_count),
                    !busy && self.undo_count > 0,
                );
                if let Some(badge) = crate::feedback::undo_badge(self.undo_count) {
                    ui.painter().text(
                        undo.rect.right_top() + egui::vec2(-1., 1.),
                        egui::Align2::RIGHT_TOP,
                        badge,
                        egui::FontId::monospace(9.),
                        theme::DIM,
                    );
                }
                if undo.clicked() {
                    self.send(Command::Undo);
                }
                if busy || self.scanning {
                    ui.spinner().on_hover_text(if self.scanning {
                        "Checking exact copies…"
                    } else {
                        "Working…"
                    });
                } else if self.viewer {
                    ui.monospace(crate::feedback::position_label(
                        self.selected,
                        self.photos.len(),
                    ))
                    .on_hover_text("Position among the photos remaining");
                } else {
                    ui.monospace(photo_count_label(self.photos.len()))
                        .on_hover_text("Photos remaining");
                }
                let name = self
                    .root
                    .as_ref()
                    .map(|p| {
                        p.file_name()
                            .unwrap_or_default()
                            .to_string_lossy()
                            .into_owned()
                    })
                    .unwrap_or_else(|| "Open a photo folder".into());
                // The folder name takes whatever width the controls leave.
                ui.add_sized(
                    [ui.available_width().max(12.), 32.],
                    egui::Label::new(egui::RichText::new(&name).color(theme::DIM))
                        .truncate()
                        .halign(egui::Align::Min),
                )
                .on_hover_text(&name);
            });
        });
        ui.add(egui::Separator::default().spacing(4.));
    }
}
fn size_marker(ui: &mut egui::Ui, icon: Icon) {
    let (rect, response) = ui.allocate_exact_size(egui::vec2(16., 24.), Sense::hover());
    icons::paint(ui.painter(), rect, icon, theme::DIM);
    response.on_hover_text("Thumbnail size · Scroll over the slider to resize");
}
fn size_slider(ui: &mut egui::Ui, size: &mut f32) -> egui::Response {
    let response = ui
        .scope(|ui| {
            ui.spacing_mut().slider_width = 90.;
            ui.visuals_mut().widgets.inactive.bg_fill = theme::SLIDER_TRACK;
            ui.add(
                egui::Slider::new(
                    size,
                    sortypix_app::settings::MIN_THUMBNAIL_SIZE
                        ..=sortypix_app::settings::MAX_THUMBNAIL_SIZE,
                )
                .logarithmic(true)
                .trailing_fill(true)
                .show_value(false),
            )
        })
        .inner;
    if response.hovered() {
        let delta = ui.input_mut(|i| {
            let delta = i.smooth_scroll_delta.y;
            i.smooth_scroll_delta.y = 0.;
            delta
        });
        *size = (*size * (delta * 0.002).exp()).clamp(
            sortypix_app::settings::MIN_THUMBNAIL_SIZE,
            sortypix_app::settings::MAX_THUMBNAIL_SIZE,
        );
    }
    response.on_hover_text("Thumbnail size · Scroll to resize")
}
fn photo_count_label(count: usize) -> String {
    format!("{count} {}", if count == 1 { "pic" } else { "pics" })
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn compact_controls_remain_explicit_and_contrasted() {
        assert_eq!(photo_count_label(0), "0 pics");
        assert_eq!(photo_count_label(1), "1 pic");
        assert_eq!(photo_count_label(2000), "2000 pics");
        let mouse = crate::mouse::visual_size(32.);
        assert_eq!(mouse.y, 32.);
        assert!(mouse.x < 22.);
        let track = crate::theme::SLIDER_TRACK;
        let background = crate::theme::BG;
        let contrast = i16::from(track.r()) - i16::from(background.r()) + i16::from(track.g())
            - i16::from(background.g())
            + i16::from(track.b())
            - i16::from(background.b());
        assert!(contrast > 100);
    }
    #[test]
    fn wheel_resizes_only_the_hovered_slider_and_is_consumed() {
        let ctx = egui::Context::default();
        let mut size = 190.;
        let mut position = egui::Pos2::ZERO;
        for frame in 0..4 {
            let wheel = frame >= 2;
            let hovered = frame != 3;
            let mut events = vec![egui::Event::PointerMoved(if hovered {
                position
            } else {
                egui::pos2(500., 400.)
            })];
            if wheel {
                events.push(egui::Event::MouseWheel {
                    unit: egui::MouseWheelUnit::Line,
                    delta: egui::vec2(0., 1.),
                    phase: egui::TouchPhase::Move,
                    modifiers: egui::Modifiers::default(),
                });
            }
            let before = size;
            let output = ctx.run_ui(
                egui::RawInput {
                    events,
                    ..Default::default()
                },
                |ui| {
                    let response = size_slider(ui, &mut size);
                    position = response.rect.center();
                    if frame == 2 {
                        assert!(size > before);
                        assert_eq!(ui.input(|i| i.smooth_scroll_delta.y), 0.);
                    }
                    if frame == 3 {
                        assert_eq!(size, before);
                    }
                },
            );
            output.drop_without_applying_deltas();
        }
    }
}
