use crate::{
    gallery::Gallery,
    shortcuts::{self, Action},
};
use eframe::egui;
use sortypix_app::worker::Command;
use sortypix_core::Decision;
impl Gallery {
    pub(crate) fn shortcut_tip(&self, action: Action) -> String {
        let binding = shortcuts::binding(&self.settings, action);
        if binding.key.is_empty() {
            action.label().into()
        } else {
            format!("{} · {}", action.label(), shortcuts::label(&binding))
        }
    }
    pub(crate) fn rotate_tip(&self) -> String {
        format!(
            "{} · Shift+click: rotate left",
            self.shortcut_tip(Action::RotateRight)
        )
    }
    pub(crate) fn keyboard(&mut self, ctx: &egui::Context) {
        if self.busy || self.shortcut_editor.open {
            return;
        }
        let action = ctx.input(|i| {
            i.events.iter().find_map(|event| {
                if let egui::Event::Key {
                    key,
                    modifiers,
                    pressed: true,
                    repeat: false,
                    ..
                } = event
                {
                    shortcuts::resolve(&self.settings, *key, *modifiers)
                        .map(|action| (action, *key, *modifiers))
                } else {
                    None
                }
            })
        });
        if let Some((action, key, modifiers)) = action {
            ctx.input_mut(|i| i.consume_key(modifiers, key));
            self.perform(action, ctx);
        }
    }
    pub(crate) fn perform(&mut self, action: Action, ctx: &egui::Context) {
        match action {
            Action::Open => self.choose(),
            Action::OpenSource => {
                if let Some(root) = self.root.clone() {
                    self.open_explorer(root);
                }
            }
            Action::OpenKept => self.open_sorted_folder(Decision::Keep),
            Action::OpenRejected => self.open_sorted_folder(Decision::Reject),
            Action::Undo => {
                self.send(Command::Undo);
            }
            Action::KeepAll | Action::RejectAll => self.decide(
                self.photos.iter().map(|p| p.id).collect(),
                if action == Action::KeepAll {
                    Decision::Keep
                } else {
                    Decision::Reject
                },
            ),
            Action::Keep | Action::Reject | Action::RotateRight | Action::RotateLeft => {
                if let Some(id) = self.current_id() {
                    match action {
                        Action::Keep => self.decide(vec![id], Decision::Keep),
                        Action::Reject => self.decide(vec![id], Decision::Reject),
                        _ => {
                            self.send(Command::Rotate(id, action == Action::RotateRight));
                        }
                    }
                }
            }
            Action::Collapse => {
                self.viewer = false;
                self.grid_scroll_to_focus = true;
                ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(false));
            }
            Action::Expand if !self.photos.is_empty() => {
                self.viewer = true;
                self.zoom = false;
                self.pan = egui::Vec2::ZERO;
                ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(true));
            }
            Action::Zoom if self.viewer => {
                self.zoom = !self.zoom;
                self.pan = egui::Vec2::ZERO;
            }
            Action::Next | Action::Previous | Action::Down | Action::Up => {
                if self.viewer && matches!(action, Action::Down | Action::Up) {
                    return;
                }
                self.selected = match action {
                    Action::Down => self.layout.vertical(self.selected, true),
                    Action::Up => self.layout.vertical(self.selected, false),
                    Action::Next => (self.selected + 1).min(self.photos.len().saturating_sub(1)),
                    _ => self.selected.saturating_sub(1),
                };
                self.pan = egui::Vec2::ZERO;
                self.grid_scroll_to_focus = true;
            }
            _ => {}
        }
    }
}
