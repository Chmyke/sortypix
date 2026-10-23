//! ROLE: Coordinates presentation, worker snapshots and keyboard commands.
//! OWNER: eframe owns Gallery; filesystem state stays on Worker.
use crate::textures::Textures;
use eframe::egui::{self, Color32, Rect};
use sortypix_app::{
    catalog::PhysicalPhoto,
    settings::Settings,
    worker::{Command, Worker},
};
use sortypix_core::{Decision, PhotoId};
use std::{
    path::PathBuf,
    sync::{Arc, mpsc},
    time::Instant,
};
pub struct Gallery {
    pub(crate) worker: Worker,
    pub(crate) photos: Vec<PhysicalPhoto>,
    pub(crate) turns: Vec<u8>,
    pub(crate) settings: Settings,
    settings_path: PathBuf,
    pub(crate) restore_position: bool,
    pub(crate) scroll_offset: f32,
    pub(crate) scroll_down: bool,
    pub(crate) grid_rect: egui::Rect,
    pub(crate) textures: Textures,
    pub(crate) root: Option<PathBuf>,
    pub(crate) selected: usize,
    pub(crate) viewer: bool,
    pub(crate) zoom: bool,
    pub(crate) pan: egui::Vec2,
    pub(crate) busy: bool,
    pub(crate) busy_since: Option<Instant>,
    /// Last sorting destination, lit briefly on its folder icon.
    pub(crate) flash: Option<(Decision, Instant)>,
    pub(crate) notice: Option<(String, Instant)>,
    pub(crate) undo_count: usize,
    error: Option<String>,
    pub(crate) scanning: bool,
    chooser: Option<mpsc::Receiver<Result<Option<PathBuf>, String>>>,
    pub(crate) diagnostic: Option<crate::diagnostic::Diagnostic>,
    pub(crate) benchmark: Option<crate::benchmark::Benchmark>,
    pub(crate) session_probe: Option<crate::session_probe::Probe>,
    pub(crate) grid_scroll_to_focus: bool,
    pub(crate) visible_ready_sent: bool,
    pub(crate) catalog_final: bool,
    pub(crate) pending_transition: Option<sortypix_app::settings::Resume>,
    pub(crate) layout: crate::layout::Layout,
    pub(crate) shortcut_editor: crate::shortcuts::Editor,
    #[cfg(feature = "ai")]
    pub(crate) ai: crate::ai::Ai,
}
pub const ACCENT: Color32 = crate::theme::BLUE;
impl Gallery {
    pub fn new(
        cc: &eframe::CreationContext<'_>,
        folder: Option<PathBuf>,
        diagnostic: Option<PathBuf>,
        benchmark: bool,
    ) -> Self {
        let ctx = cc.egui_ctx.clone();
        crate::theme::apply(&ctx);
        let (path, cache) = if let Some(dir) = &diagnostic {
            (dir.join("settings.json"), dir.join("cache"))
        } else {
            (
                sortypix_app::settings::settings_path(),
                sortypix_app::settings::cache_dir(),
            )
        };
        let settings = Settings::load(&path);
        let open = folder.or_else(|| settings.last_folder.clone());
        let wake_ctx = ctx.clone();
        let mut app = Self {
            worker: Worker::start(Arc::new(move || wake_ctx.request_repaint())),
            photos: Vec::new(),
            turns: Vec::new(),
            settings,
            textures: Textures::new(ctx, cache.join("v2")),
            settings_path: path,
            restore_position: false,
            scroll_offset: 0.,
            scroll_down: true,
            grid_rect: egui::Rect::NOTHING,
            root: None,
            selected: 0,
            viewer: false,
            zoom: false,
            pan: egui::Vec2::ZERO,
            busy: false,
            busy_since: None,
            flash: None,
            notice: None,
            undo_count: 0,
            error: None,
            scanning: false,
            chooser: None,
            diagnostic: if benchmark {
                None
            } else {
                diagnostic.clone().map(crate::diagnostic::Diagnostic::new)
            },
            benchmark: if benchmark {
                diagnostic.map(crate::benchmark::Benchmark::new)
            } else {
                None
            },
            session_probe: None,
            grid_scroll_to_focus: false,
            visible_ready_sent: false,
            catalog_final: true,
            pending_transition: None,
            layout: Default::default(),
            shortcut_editor: Default::default(),
            // Last: the worker and preview threads above must exist first (thread priorities).
            #[cfg(feature = "ai")]
            ai: crate::ai::Ai::new(&cache, cc),
        };
        if let Some(folder) = open {
            app.open(folder);
        }
        app
    }
    fn remember_window(&mut self, ctx: &egui::Context) {
        if self.diagnostic.is_some() || self.benchmark.is_some() || self.viewer {
            return;
        }
        let pixels_per_point = ctx.pixels_per_point();
        ctx.input(|i| {
            let v = i.viewport();
            if v.fullscreen == Some(true) || v.minimized == Some(true) {
                return;
            }
            if v.maximized == Some(true) {
                if let Some(window) = &mut self.settings.window {
                    window.maximized = true;
                }
            } else if let (Some(outer), Some(inner)) = (v.outer_rect, v.inner_rect) {
                let p = outer.min * pixels_per_point;
                self.settings.window = Some(sortypix_app::settings::Window {
                    position_pixels: [p.x, p.y],
                    size: [inner.width(), inner.height()],
                    maximized: false,
                });
            }
        });
    }
    pub(crate) fn send(&mut self, command: Command) -> bool {
        if self.busy {
            return false;
        }
        match self.worker.commands.try_send(command) {
            Ok(()) => {
                self.busy = true;
                self.busy_since = Some(Instant::now());
                self.error = None;
                true
            }
            Err(e) => {
                self.error = Some(e.to_string());
                false
            }
        }
    }
    fn open(&mut self, folder: PathBuf) {
        if self.busy {
            return;
        }
        self.restore_position = true;
        self.scroll_offset = 0.;
        self.scroll_down = true;
        self.viewer = false;
        self.selected = 0;
        self.textures.clear();
        self.visible_ready_sent = false;
        self.catalog_final = false;
        self.pending_transition = None;
        #[cfg(feature = "ai")]
        self.ai.folder_opened();
        self.send(Command::Open(folder));
    }
    pub(crate) fn decide(&mut self, ids: Vec<PhotoId>, decision: Decision) {
        if !ids.is_empty() {
            if self.send(Command::Decide(ids, decision)) {
                self.flash = Some((decision, Instant::now()));
            }
            self.pan = egui::Vec2::ZERO;
        }
    }
    pub(crate) fn current_id(&self) -> Option<PhotoId> {
        self.photos.get(self.selected).map(|p| p.id)
    }
    fn poll(&mut self, ctx: &egui::Context) {
        while let Ok(state) = self.worker.events.try_recv() {
            if let Some(benchmark) = &mut self.benchmark {
                benchmark.observe_catalog(state.catalog_final);
            }
            if !self.catalog_final && state.catalog_final && self.root == state.root {
                self.pending_transition = self.settings.resume.clone();
            }
            let previous_id = self.photos.get(self.selected).map(|p| p.id);
            self.photos = state.photos;
            self.layout.invalidate();
            self.turns = state.turns;
            self.root = state.root;
            self.undo_count = state.undo_count;
            self.error = state.error;
            self.scanning = state.duplicates_scanning;
            self.catalog_final = state.catalog_final;
            self.busy = false;
            self.busy_since = None;
            #[cfg(not(feature = "ai"))]
            if self.settings.newest_first {
                self.photos.reverse();
                self.turns.reverse();
            }
            #[cfg(feature = "ai")]
            self.ai_snapshot();
            self.selected = previous_id
                .and_then(|id| self.photos.iter().position(|p| p.id == id))
                .unwrap_or(self.selected)
                .min(self.photos.len().saturating_sub(1));
            if self.photos.is_empty() {
                self.viewer = false;
                ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(false));
            }
            self.settings.last_folder = self.root.clone();
        }
        self.textures.poll(ctx);
        #[cfg(feature = "ai")]
        self.ai_frame();
        if !self.visible_ready_sent
            && self.root.is_some()
            && self.textures.loaded > 0
            && self.worker.commands.try_send(Command::VisibleReady).is_ok()
        {
            self.visible_ready_sent = true;
        }
        if let Some(receiver) = &self.chooser
            && let Ok(result) = receiver.try_recv()
        {
            self.chooser = None;
            match result {
                Ok(Some(folder)) => self.open(folder),
                Ok(None) => {}
                Err(error) => self.error = Some(error),
            }
        }
    }
    pub(crate) fn save_shortcuts(
        &mut self,
        previous: std::collections::BTreeMap<String, sortypix_app::settings::Shortcut>,
    ) {
        if let Err(error) = self.settings.save(&self.settings_path) {
            self.settings.shortcuts = previous;
            self.shortcut_editor.error = Some(format!("Could not save shortcuts: {error}"));
        }
    }
    pub(crate) fn open_sorted_folder(&mut self, decision: Decision) {
        let Some(root) = &self.root else {
            return;
        };
        let folder = root.join(match decision {
            Decision::Keep => "keep",
            Decision::Reject => "rejected",
        });
        if !folder.is_dir() {
            self.error = Some(format!(
                "{} is not available yet. Sort a photo into this folder first.",
                folder.display()
            ));
            return;
        }
        self.open_explorer(folder);
    }
    pub(crate) fn open_explorer(&mut self, folder: PathBuf) {
        // The system file manager: Explorer, or the one the Linux desktop associates.
        let manager = if cfg!(windows) {
            "explorer.exe"
        } else {
            "xdg-open"
        };
        if let Err(error) = std::process::Command::new(manager).arg(folder).spawn() {
            self.error = Some(format!("Could not open folder: {error}"));
        }
    }
    pub(crate) fn choose(&mut self) {
        if self.chooser.is_some() {
            return;
        }
        let (send, receive) = mpsc::channel();
        self.chooser = Some(receive);
        std::thread::spawn(move || {
            let _ = send.send(sortypix_app::dialog::choose_folder());
        });
    }
}
impl Gallery {
    fn contents(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        self.poll(&ctx);
        self.remember_window(&ctx);
        self.keyboard(&ctx);
        let dropped = ctx.input(|i| i.raw.dropped_files.first().map(|f| f.path().to_path_buf()));
        if let Some(folder) = dropped
            && folder.is_dir()
            && !self.shortcut_editor.open
        {
            self.open(folder);
        }
        self.toolbar(ui);
        if self.root.is_none() {
            self.empty_screen(ui);
        } else {
            self.textures.begin_frame();
            if self.viewer {
                self.viewer(ui);
            } else {
                self.grid(ui);
            }
            self.textures.finish_frame();
        }
        let now = Instant::now();
        let notice = self
            .notice
            .as_ref()
            .filter(|(_, at)| crate::feedback::notice_shown(*at, now))
            .map(|(text, _)| text.as_str());
        crate::feedback::overlay(&ctx, &mut self.error, notice);
        if self
            .flash
            .is_some_and(|(_, at)| crate::feedback::glow(at, now) > 0.)
        {
            ctx.request_repaint();
        }
        #[cfg(feature = "ai")]
        self.ai_card(&ctx);
        self.shortcut_dialog(&ctx);
        self.run_diagnostic(&ctx);
        self.run_session_probe(&ctx);
        ctx.request_repaint_after(std::time::Duration::from_millis(100));
    }
}
impl Gallery {
    fn empty_screen(&mut self, ui: &mut egui::Ui) {
        ui.add_space(ui.available_height() * 0.2);
        ui.vertical_centered(|ui| {
            ui.heading("Open a photo folder");
            ui.add_space(12.);
            ui.label("Drop a folder of photos here, or open one to start sorting.");
            ui.add_space(20.);
            if ui.button("Open folder").clicked() {
                self.choose();
            }
            ui.add_space(28.);
            let (rect, _) = ui.allocate_exact_size(egui::vec2(48., 64.), egui::Sense::hover());
            crate::mouse::paint(
                ui.painter(),
                Rect::from_center_size(rect.center(), crate::mouse::visual_size(64.)),
                self.settings.swapped,
            );
            ui.add_space(8.);
            for line in crate::mouse::legend(self.settings.swapped) {
                ui.label(line);
            }
            ui.label(self.shortcut_tip(crate::shortcuts::Action::Undo));
            ui.add_space(20.);
            ui.weak("Nothing is deleted. Keep and reject move photos into separate folders.");
        });
    }
}
impl eframe::App for Gallery {
    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        egui::Frame::new()
            .fill(crate::theme::BG)
            .inner_margin(egui::Margin {
                left: 8,
                right: 8,
                top: 4,
                bottom: 4,
            })
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 4.;
                self.contents(ui);
            });
        self.run_benchmark(ui.ctx(), frame.info().cpu_usage);
    }
    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        if self.viewer
            && let Some(photo) = self.photos.get(self.selected)
            && let Some(root) = &self.root
        {
            self.settings.resume = Some(sortypix_app::settings::Resume {
                folder: root.clone(),
                photo: photo.display_source(false).to_path_buf(),
                index: self.selected,
                fraction: 0.,
            });
        }
        if let Some(d) = &self.diagnostic {
            d.record_incomplete(self.photos.len(), self.undo_count, self.viewer);
        }
        let _ = self.settings.save(&self.settings_path);
    }
    fn raw_input_hook(&mut self, ctx: &egui::Context, input: &mut egui::RawInput) {
        if let Some(d) = &mut self.diagnostic {
            d.inject(input);
        }
        if !self.viewer && !self.shortcut_editor.open {
            crate::grid::resize_input(
                input,
                ctx.input(|i| i.pointer.hover_pos()),
                self.grid_rect,
                &mut self.settings.thumbnail_size,
            );
        }
    }
}
