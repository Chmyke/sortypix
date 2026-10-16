use crate::{
    gallery::Gallery,
    shortcuts::{self, Action},
};
use eframe::egui;
use sortypix_app::settings::Shortcut;
impl Gallery {
    pub(crate) fn shortcut_dialog(&mut self, ctx: &egui::Context) {
        if !self.shortcut_editor.open {
            return;
        }
        let previous = self.settings.shortcuts.clone();
        let was_capturing = self.shortcut_editor.capturing.is_some();
        let changed = ctx.input(|i| self.shortcut_editor.capture(&mut self.settings, &i.events));
        if was_capturing {
            // Captured Enter/Space must not also activate the focused editor button.
            ctx.input_mut(|i| {
                i.events
                    .retain(|event| !matches!(event, egui::Event::Key { .. }))
            });
        }
        if changed {
            self.save_shortcuts(previous);
        }
        let previous = self.settings.shortcuts.clone();
        let mut changed = false;
        egui::Modal::new(egui::Id::new("shortcut-editor")).show(ctx, |ui| {
            ui.set_width(430.);
            ui.horizontal(|ui| {
                ui.heading("Shortcuts");
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("Done").clicked() {
                        self.shortcut_editor.open = false;
                    }
                });
            });
            ui.weak("Click a shortcut, then press your new combination.");
            if let Some(action) = self.shortcut_editor.capturing {
                ui.colored_label(
                    crate::theme::BLUE,
                    format!("{}: press keys · Esc cancels", action.label()),
                );
            }
            if let Some(error) = &self.shortcut_editor.error {
                ui.colored_label(crate::theme::RED, error);
            }
            ui.separator();
            egui::ScrollArea::vertical()
                .max_height((ctx.content_rect().height() - 220.).max(120.))
                .show(ui, |ui| {
                    egui::Grid::new("editable-shortcuts")
                        .spacing(egui::vec2(20., 7.))
                        .show(ui, |ui| {
                            for action in Action::ALL {
                                ui.label(action.label());
                                let capturing = self.shortcut_editor.capturing == Some(action);
                                let label = if capturing {
                                    "Press keys…".into()
                                } else {
                                    shortcuts::label(&shortcuts::binding(&self.settings, action))
                                };
                                let response = ui.add_sized(
                                    [140., 25.],
                                    egui::Button::new(label).selected(capturing),
                                );
                                if action == Action::Keep
                                    && let Some(d) = &mut self.diagnostic
                                {
                                    d.binding_target = Some(response.rect.center());
                                }
                                if response.clicked() {
                                    self.shortcut_editor.capturing = Some(action);
                                    self.shortcut_editor.error = None;
                                }
                                if ui
                                    .small_button("×")
                                    .on_hover_text("Remove this shortcut")
                                    .clicked()
                                {
                                    self.settings
                                        .shortcuts
                                        .insert(action.id().into(), Shortcut::default());
                                    self.shortcut_editor.capturing = None;
                                    self.shortcut_editor.error = None;
                                    changed = true;
                                }
                                ui.end_row();
                            }
                        });
                    ui.separator();
                    ui.strong("Mouse");
                    ui.label(if self.settings.inverted {
                        "Left: keep · Right: reject"
                    } else {
                        "Left: reject · Right: keep"
                    });
                    ui.weak("Middle / Ctrl+click: reject through this photo");
                    ui.weak("Drag at 100%: pan · Wheel over size: resize thumbnails");
                    ui.checkbox(&mut self.settings.inverted, "Swap left and right clicks");
                });
            ui.separator();
            if ui.button("Restore default shortcuts").clicked() {
                self.settings.shortcuts.clear();
                self.shortcut_editor.capturing = None;
                self.shortcut_editor.error = None;
                changed = true;
            }
        });
        if changed {
            self.save_shortcuts(previous);
        }
        if !was_capturing && ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.shortcut_editor.open = false;
        }
        if !self.shortcut_editor.open {
            self.shortcut_editor.capturing = None;
            self.shortcut_editor.error = None;
        }
    }
}
