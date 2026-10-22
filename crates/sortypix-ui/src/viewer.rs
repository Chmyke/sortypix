use crate::{
    commands::{UiCommand, click},
    gallery::Gallery,
    textures,
};
use eframe::egui::{self, Color32, Rect, Sense};
use sortypix_app::worker::Command;
use sortypix_core::Decision;
impl Gallery {
    pub(crate) fn viewer(&mut self, ui: &mut egui::Ui) {
        let Some(photo) = self.photos.get(self.selected).cloned() else {
            return;
        };
        let turns = self.turns[self.selected];
        let (rect, response) = ui.allocate_exact_size(ui.available_size(), Sense::click_and_drag());
        let native_source = photo.display_source(self.zoom);
        let preview_only = self.zoom
            && self.textures.error(native_source, 0).is_some()
            && photo.display_source(false) != native_source;
        let source = if preview_only {
            photo.display_source(false)
        } else {
            native_source
        };
        if let Some(texture) = self.textures.immediate(source, 0) {
            let native = if turns % 2 == 1 {
                egui::vec2(texture.size_vec2().y, texture.size_vec2().x)
            } else {
                texture.size_vec2()
            };
            let size = if self.zoom {
                native / ui.ctx().pixels_per_point()
            } else {
                textures::fit(texture.size_vec2(), rect.size(), turns)
            };
            if self.zoom && response.dragged() {
                self.pan += response.drag_delta();
            }
            let bound = (size - rect.size()).max(egui::Vec2::ZERO) * 0.5;
            self.pan = self.pan.clamp(-bound, bound);
            textures::paint(
                &ui.painter().with_clip_rect(rect),
                Rect::from_center_size(rect.center() + self.pan, size),
                &texture,
                turns,
            );
        } else {
            let text = self
                .textures
                .error(source, 0)
                .unwrap_or("Loading full resolution…");
            ui.painter().text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                text,
                egui::FontId::proportional(16.),
                crate::theme::DIM,
            );
        }
        if preview_only {
            ui.painter().text(
                rect.left_top() + egui::vec2(12., 12.),
                egui::Align2::LEFT_TOP,
                "JPEG preview · RAW native resolution unavailable",
                egui::FontId::proportional(14.),
                crate::theme::AMBER,
            );
        }
        let collapse_rect = crate::icons::corner(rect);
        let collapse = corner_button(
            ui,
            collapse_rect,
            ("collapse", photo.id.0),
            crate::icons::Icon::Contract,
        );
        if let Some(d) = &mut self.diagnostic {
            d.collapse_target = Some(collapse_rect.center());
        }
        if let Some(rotate_rect) = crate::icons::left_corner(rect) {
            let rotate = corner_button(
                ui,
                rotate_rect,
                ("rotate", photo.id.0),
                crate::icons::Icon::Rotate,
            );
            if rotate.on_hover_text(self.rotate_tip()).clicked() && !self.shortcut_editor.open {
                let right = !ui.input(|i| i.modifiers.shift);
                self.send(Command::Rotate(photo.id, right));
                return;
            }
        }
        if collapse
            .on_hover_cursor(egui::CursorIcon::PointingHand)
            .on_hover_text(self.shortcut_tip(crate::shortcuts::Action::Collapse))
            .clicked()
        {
            self.viewer = false;
            self.grid_scroll_to_focus = true;
            ui.ctx()
                .send_viewport_cmd(egui::ViewportCommand::Fullscreen(false));
            return;
        }
        if !self.busy
            && !self.shortcut_editor.open
            && (response.clicked() || response.secondary_clicked() || response.middle_clicked())
            && !self.zoom
        {
            match click(
                photo.id,
                response.secondary_clicked(),
                self.settings.swapped,
                ui.input(|i| i.modifiers.ctrl),
                response.middle_clicked(),
                response.dragged(),
            ) {
                Some(UiCommand::Decide(id, decision)) => self.decide(vec![id], decision),
                Some(UiCommand::RejectThrough(_)) => self.decide(
                    self.photos[..=self.selected].iter().map(|p| p.id).collect(),
                    Decision::Reject,
                ),
                None => {}
            }
        }
        for index in [self.selected.saturating_sub(1), self.selected + 1] {
            if let Some(next) = self.photos.get(index) {
                let source = next.display_source(self.zoom);
                self.textures.immediate(source, 0);
            }
        }
    }
}

/// Permanent corner control of the full-screen view; never sorts the photo.
fn corner_button(
    ui: &mut egui::Ui,
    rect: Rect,
    salt: (&str, u64),
    icon: crate::icons::Icon,
) -> egui::Response {
    let response = ui.interact(rect, ui.id().with(salt), Sense::click());
    ui.painter()
        .rect_filled(rect, 7, Color32::from_black_alpha(220));
    crate::icons::paint(
        ui.painter(),
        rect,
        icon,
        if response.hovered() {
            crate::theme::BLUE
        } else {
            crate::theme::TEXT
        },
    );
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}
