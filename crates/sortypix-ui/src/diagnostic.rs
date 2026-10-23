use eframe::egui;
use std::{
    path::PathBuf,
    time::{Duration, Instant},
};
pub struct Diagnostic {
    pub folder: PathBuf,
    start: Instant,
    pub(crate) stage: usize,
    last: Instant,
    pub keys: Vec<(egui::Key, bool)>,
    pub click_target: Option<egui::Pos2>,
    pub collapse_target: Option<egui::Pos2>,
    pub help_target: Option<egui::Pos2>,
    pub binding_target: Option<egui::Pos2>,
    pub keep_all_target: Option<egui::Pos2>,
    pub reject_all_target: Option<egui::Pos2>,
    pub rotate_target: Option<egui::Pos2>,
    pub mouse_target: Option<egui::Pos2>,
    turns_before: u8,
    click_pending: bool,
    wheel: Option<egui::Pos2>,
    count: usize,
    pub(crate) pointer: Option<(egui::Pos2, bool)>,
    held_at: Option<egui::Pos2>,
    held_id: Option<sortypix_core::PhotoId>,
    /// Sortypix AI: text typed into the focused field, a pointer move, the AI stages.
    #[cfg(feature = "ai")]
    pub(crate) text: Option<String>,
    #[cfg(feature = "ai")]
    pub(crate) hover: Option<egui::Pos2>,
    /// Sortypix AI: a plain wheel turn down at this point (scrolls the shortcuts dialog).
    #[cfg(feature = "ai")]
    pub(crate) scroll: Option<egui::Pos2>,
    #[cfg(feature = "ai")]
    pub(crate) ai: Option<crate::ai::diagnostic::Run>,
}
/// Sortypix AI also waits for its model and the analysis of the folder.
#[cfg(not(feature = "ai"))]
const LIMIT: Duration = Duration::from_secs(45);
#[cfg(feature = "ai")]
const LIMIT: Duration = Duration::from_secs(150);
impl Diagnostic {
    pub fn record_incomplete(&self, photos: usize, undo: usize, viewer: bool) {
        if !self.folder.join("PASS.txt").exists() && !self.folder.join("FAIL.txt").exists() {
            let _ = std::fs::write(
                self.folder.join("FAIL.txt"),
                format!(
                    "Diagnostic closed at stage {}: photos {photos}/{}, undo {undo}, viewer {viewer}",
                    self.stage, self.count
                ),
            );
        }
    }
    pub fn new(folder: PathBuf) -> Self {
        Self {
            folder,
            start: Instant::now(),
            stage: 0,
            last: Instant::now(),
            keys: Vec::new(),
            click_target: None,
            collapse_target: None,
            help_target: None,
            binding_target: None,
            keep_all_target: None,
            reject_all_target: None,
            rotate_target: None,
            mouse_target: None,
            turns_before: 0,
            click_pending: false,
            wheel: None,
            count: 0,
            pointer: None,
            held_at: None,
            held_id: None,
            #[cfg(feature = "ai")]
            text: None,
            #[cfg(feature = "ai")]
            hover: None,
            #[cfg(feature = "ai")]
            scroll: None,
            #[cfg(feature = "ai")]
            ai: None,
        }
    }
    pub fn inject(&mut self, input: &mut egui::RawInput) {
        if let Some(pos) = self.wheel.take() {
            input.events.push(egui::Event::PointerMoved(pos));
            input.events.push(egui::Event::MouseWheel {
                unit: egui::MouseWheelUnit::Line,
                delta: egui::vec2(0., 2.),
                phase: egui::TouchPhase::Move,
                modifiers: egui::Modifiers::CTRL,
            });
        }
        if let Some((pos, pressed)) = self.pointer.take() {
            input.events.push(egui::Event::PointerMoved(pos));
            input.events.push(egui::Event::PointerButton {
                pos,
                pressed,
                button: egui::PointerButton::Primary,
                modifiers: egui::Modifiers::default(),
            });
        }
        #[cfg(feature = "ai")]
        if let Some(pos) = self.hover.take() {
            input.events.push(egui::Event::PointerMoved(pos));
        }
        #[cfg(feature = "ai")]
        if let Some(pos) = self.scroll.take() {
            input.events.push(egui::Event::PointerMoved(pos));
            input.events.push(egui::Event::MouseWheel {
                unit: egui::MouseWheelUnit::Line,
                delta: egui::vec2(0., -40.),
                phase: egui::TouchPhase::Move,
                modifiers: egui::Modifiers::NONE,
            });
        }
        input
            .events
            .push(egui::Event::ModifiersChanged(egui::Modifiers::default()));
        #[cfg(feature = "ai")]
        if let Some(text) = self.text.take() {
            input.events.push(egui::Event::Text(text));
        }
        for (key, ctrl) in self.keys.drain(..) {
            input
                .events
                .push(egui::Event::ModifiersChanged(egui::Modifiers {
                    ctrl,
                    ..Default::default()
                }));
            for pressed in [true, false] {
                input.events.push(egui::Event::Key {
                    key,
                    physical_key: None,
                    pressed,
                    repeat: false,
                    modifiers: egui::Modifiers {
                        ctrl,
                        ..Default::default()
                    },
                });
            }
        }
        if self.click_pending {
            self.click_pending = false;
            if let Some(pos) = self.click_target {
                let modifiers = egui::Modifiers {
                    ctrl: true,
                    ..Default::default()
                };
                input.events.push(egui::Event::ModifiersChanged(modifiers));
                input.events.push(egui::Event::PointerMoved(pos));
                for pressed in [true, false] {
                    input.events.push(egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers,
                    });
                }
            }
        }
    }
}
impl crate::gallery::Gallery {
    pub(crate) fn run_diagnostic(&mut self, ctx: &egui::Context) {
        let Some(d) = &mut self.diagnostic else {
            return;
        };
        for event in ctx.input(|i| i.events.clone()) {
            if let egui::Event::Screenshot { image, .. } = event {
                let bytes: Vec<u8> = image.pixels.iter().flat_map(|p| p.to_array()).collect();
                let _ = std::fs::create_dir_all(&d.folder);
                let _ = image::save_buffer(
                    d.folder.join(format!("screen-{}.png", d.stage)),
                    &bytes,
                    image.width() as u32,
                    image.height() as u32,
                    image::ColorType::Rgba8,
                );
            }
        }
        if d.start.elapsed() > LIMIT {
            d.record_incomplete(self.photos.len(), self.undo_count, self.viewer);
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        }
        #[cfg(feature = "ai")]
        if d.ai.is_some() {
            self.ai_diagnostic(ctx);
            return;
        }
        if self.busy
            || self.scanning
            || self.textures.loaded < self.photos.len().min(12)
            || d.last.elapsed() < Duration::from_millis(700)
        {
            return;
        }
        d.last = Instant::now();
        d.stage += 1;
        match d.stage {
            1 => {
                d.count = self.photos.len();
                ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));
            }
            3 | 5 | 7 | 9 | 11 | 13 | 15 => {
                let valid = match d.stage {
                    3 => {
                        self.photos.len() + 1 == d.count
                            && self.flash.is_some_and(|(decision, _)| {
                                decision == sortypix_core::Decision::Keep
                            })
                    }
                    5 | 9 => self.photos.len() == d.count && self.undo_count == 0,
                    7 => self.photos.len() + 3 == d.count,
                    11 => self.viewer,
                    13 => self.zoom,
                    15 => self.turns.get(self.selected) == Some(&1),
                    _ => false,
                };
                if !valid {
                    let _ = std::fs::write(
                        d.folder.join("FAIL.txt"),
                        format!(
                            "Stage {} failed: photos {}, history {}, viewer {}, zoom {}",
                            d.stage,
                            self.photos.len(),
                            self.undo_count,
                            self.viewer,
                            self.zoom
                        ),
                    );
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    return;
                }
                ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()))
            }
            2 => d.keys.push((egui::Key::ArrowUp, false)),
            4 => d.keys.push((egui::Key::Z, true)),
            6 => d.click_pending = true,
            8 => d.keys.push((egui::Key::Z, true)),
            10 => d.keys.push((egui::Key::Enter, false)),
            12 => d.keys.push((egui::Key::Z, false)),
            14 => d.keys.push((egui::Key::R, false)),
            16 => d.keys.push((egui::Key::Z, true)),
            17 => d.keys.push((egui::Key::Escape, false)),
            18 => {
                d.held_at = d.click_target;
                d.held_id = self.photos.get(2).map(|p| p.id);
                d.pointer = d.held_at.map(|pos| (pos, true));
            }
            19 => {
                ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(egui::vec2(900., 620.)));
                ctx.set_zoom_factor(1.25);
            }
            20 => d.pointer = d.held_at.map(|pos| (pos, false)),
            21 => {
                let removed = d.count.saturating_sub(self.photos.len());
                let wrong_photo =
                    removed == 1 && self.photos.iter().any(|p| Some(p.id) == d.held_id);
                if removed > 1 || self.undo_count > 1 || wrong_photo {
                    let _ = std::fs::write(
                        d.folder.join("FAIL.txt"),
                        "Resize/scale changed the gesture target or caused repeated sorting",
                    );
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    return;
                }
                if self.undo_count == 1 {
                    d.keys.push((egui::Key::Z, true));
                }
            }
            22 => ctx.set_zoom_factor(1.0),
            23 => d.keys.push((egui::Key::Enter, false)),
            24 => d.pointer = d.collapse_target.map(|p| (p, true)),
            25 => d.pointer = d.collapse_target.map(|p| (p, false)),
            26 => {
                if self.photos.len() != d.count || self.undo_count != 0 || self.viewer {
                    let _ = std::fs::write(
                        d.folder.join("FAIL.txt"),
                        format!(
                            "Collapse changed sorting state: photos={}/{}, undo={}, viewer={}, target={:?}",
                            self.photos.len(),
                            d.count,
                            self.undo_count,
                            self.viewer,
                            d.collapse_target
                        ),
                    );
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    return;
                }
                ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));
                d.pointer = d.help_target.map(|p| (p, true));
            }
            27 => d.pointer = d.help_target.map(|p| (p, false)),
            28 => d.pointer = d.binding_target.map(|p| (p, true)),
            29 => d.pointer = d.binding_target.map(|p| (p, false)),
            30 => d.keys.push((egui::Key::ArrowDown, false)),
            31 => {
                if self.shortcut_editor.error.is_none()
                    || !self.settings.shortcuts.is_empty()
                    || self.photos.len() != d.count
                {
                    let _ = std::fs::write(
                        d.folder.join("FAIL.txt"),
                        "Shortcut editor did not reject conflict or sorted while capturing",
                    );
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    return;
                }
                d.keys.push((egui::Key::Space, false));
            }
            32 => {
                let saved = sortypix_app::settings::Settings::load(&d.folder.join("settings.json"));
                if crate::shortcuts::resolve(&saved, egui::Key::Space, egui::Modifiers::NONE)
                    != Some(crate::shortcuts::Action::Keep)
                    || self.photos.len() != d.count
                    || self.shortcut_editor.capturing.is_some()
                {
                    let _ = std::fs::write(
                        d.folder.join("FAIL.txt"),
                        "Remapping was not persisted or changed photos",
                    );
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    return;
                }
                ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));
            }
            33 => d.keys.push((egui::Key::Escape, false)),
            34 => d.keys.push((egui::Key::ArrowUp, false)),
            35 => {
                if self.photos.len() != d.count || self.shortcut_editor.open {
                    let _ = std::fs::write(
                        d.folder.join("FAIL.txt"),
                        "Old shortcut still active or editor did not close",
                    );
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    return;
                }
                d.keys.push((egui::Key::Space, false));
            }
            36 => {
                if self.photos.len() + 1 != d.count {
                    let _ = std::fs::write(
                        d.folder.join("FAIL.txt"),
                        "New shortcut did not keep photo",
                    );
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    return;
                }
                d.keys.push((egui::Key::Z, true));
            }
            37 => d.pointer = d.keep_all_target.map(|p| (p, true)),
            38 => d.pointer = d.keep_all_target.map(|p| (p, false)),
            39 | 42 => {
                if !self.photos.is_empty() || self.undo_count != 1 {
                    let _ = std::fs::write(
                        d.folder.join("FAIL.txt"),
                        format!("Batch icon failed at stage {}", d.stage),
                    );
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    return;
                }
                d.keys.push((egui::Key::Z, true));
            }
            40 => d.pointer = d.reject_all_target.map(|p| (p, true)),
            41 => d.pointer = d.reject_all_target.map(|p| (p, false)),
            43 => self.settings.thumbnail_size = sortypix_app::settings::MAX_THUMBNAIL_SIZE,
            44 => {
                if self.layout.cells.first().is_none_or(|r| r.width() < 700.) {
                    let _ = std::fs::write(
                        d.folder.join("FAIL.txt"),
                        "Maximum size did not enlarge photos across the window",
                    );
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    return;
                }
                ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));
            }
            45 => {
                self.settings.thumbnail_size = 190.;
                d.wheel = Some(self.grid_rect.center());
            }
            46 => {
                if self.settings.thumbnail_size <= 190.
                    || ctx.zoom_factor() != 1.
                    || self.photos.len() != d.count
                {
                    let _ = std::fs::write(
                        d.folder.join("FAIL.txt"),
                        "Ctrl+wheel failed or changed UI scale/sorted a photo",
                    );
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    return;
                }
                self.settings.thumbnail_size = 190.;
            }
            47 => {
                d.held_id = self.photos.get(2).map(|p| p.id);
                d.turns_before = self.turns.get(2).copied().unwrap_or_default();
                d.pointer = d.rotate_target.map(|p| (p, true));
            }
            48 => d.pointer = d.rotate_target.map(|p| (p, false)),
            49 | 51 => {
                let expected = if d.stage == 49 {
                    (d.turns_before + 1) % 4
                } else {
                    d.turns_before
                };
                let turns = self
                    .photos
                    .iter()
                    .position(|p| Some(p.id) == d.held_id)
                    .and_then(|i| self.turns.get(i).copied());
                let history = usize::from(d.stage == 49);
                if turns != Some(expected)
                    || self.photos.len() != d.count
                    || self.undo_count != history
                {
                    let _ = std::fs::write(
                        d.folder.join("FAIL.txt"),
                        format!(
                            "Rotate on photo failed at stage {}: turns {turns:?}, photos {}/{}, undo {}",
                            d.stage,
                            self.photos.len(),
                            d.count,
                            self.undo_count
                        ),
                    );
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    return;
                }
                if d.stage == 49 {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(
                        egui::UserData::default(),
                    ));
                    d.keys.push((egui::Key::Z, true));
                } else {
                    d.pointer = d.mouse_target.map(|p| (p, true));
                }
            }
            50 => {}
            52 => d.pointer = d.mouse_target.map(|p| (p, false)),
            53 | 55 => {
                let swapped = d.stage == 53;
                if self.settings.swapped != swapped || self.notice.is_none() {
                    let _ = std::fs::write(
                        d.folder.join("FAIL.txt"),
                        format!("Mouse swap notice failed at stage {}", d.stage),
                    );
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    return;
                }
                if swapped {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(
                        egui::UserData::default(),
                    ));
                    d.pointer = d.mouse_target.map(|p| (p, true));
                }
            }
            54 => d.pointer = d.mouse_target.map(|p| (p, false)),
            _ => {
                let passed = self.photos.len() == d.count && self.undo_count == 0 && !self.viewer;
                // Sortypix AI goes on with its own stages, which write the verdict.
                #[cfg(feature = "ai")]
                if passed {
                    d.ai = Some(Default::default());
                    return;
                }
                if passed {
                    let _ = std::fs::write(
                        d.folder.join("PASS.txt"),
                        "Keyboard, batch click, undo, full screen, zoom, rotation, resize/UI scale, collapse, shortcut editing/conflicts/persistence/dispatch keep/reject-all icons very large thumbnails, Ctrl+wheel, rotate on photo, decision glow and mouse swap notice verified",
                    );
                } else {
                    d.record_incomplete(self.photos.len(), self.undo_count, self.viewer);
                }
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
        }
    }
}
