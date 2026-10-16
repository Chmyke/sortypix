//! Native restart probe, launched only by check.ps1 against disposable fixtures/settings.
use eframe::egui;
use sortypix_app::settings::{Resume, Window};
use std::{
    path::PathBuf,
    time::{Duration, Instant},
};
pub(crate) struct Probe {
    mode: String,
    directory: PathBuf,
    start: Instant,
    prepared: bool,
    resume: Option<Resume>,
    window: Option<Window>,
}

fn session_probe_ready(
    busy: bool,
    scanning: bool,
    photos_empty: bool,
    catalog_final: bool,
) -> bool {
    !busy && !scanning && !photos_empty && catalog_final
}

impl crate::gallery::Gallery {
    pub fn with_session_probe(mut self, mode: Option<String>, directory: Option<PathBuf>) -> Self {
        if let (Some(mode), Some(directory)) = (mode, directory) {
            self.diagnostic = None;
            self.session_probe = Some(Probe {
                mode,
                directory,
                start: Instant::now(),
                prepared: false,
                resume: self.settings.resume.clone(),
                window: self.settings.window.clone(),
            });
        }
        self
    }
    pub(crate) fn run_session_probe(&mut self, ctx: &egui::Context) {
        let Some(probe) = &mut self.session_probe else {
            return;
        };
        if probe.start.elapsed() > Duration::from_secs(15) {
            let _ = std::fs::write(probe.directory.join("FAIL.txt"), "Session probe timeout");
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        }
        if !session_probe_ready(
            self.busy,
            self.scanning,
            self.photos.is_empty(),
            self.catalog_final,
        ) {
            return;
        }
        if probe.mode == "--session-store" && !probe.prepared {
            self.settings.thumbnail_size = 700.;
            self.selected = 18.min(self.photos.len() - 1);
            self.grid_scroll_to_focus = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(egui::vec2(1000., 700.)));
            ctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(egui::pos2(80., 80.)));
            probe.prepared = true;
            return;
        }
        if probe.start.elapsed() < Duration::from_secs(3) || self.textures.loaded == 0 {
            return;
        }
        let result = if probe.mode == "--session-store" {
            self.scroll_offset > 100.
                && self.settings.resume.is_some()
                && self.settings.window.is_some()
        } else if probe.mode == "--session-other" {
            self.scroll_offset < 1.
                && self.settings.resume.as_ref().map(|r| &r.folder)
                    != probe.resume.as_ref().map(|r| &r.folder)
        } else {
            let same_photo = self.settings.resume.as_ref().map(|r| &r.photo)
                == probe.resume.as_ref().map(|r| &r.photo);
            let same_window = match (&self.settings.window, &probe.window) {
                (Some(now), Some(saved)) => {
                    now.size
                        .iter()
                        .zip(saved.size)
                        .all(|(a, b)| (*a - b).abs() < 3.)
                        && now
                            .position_pixels
                            .iter()
                            .zip(saved.position_pixels)
                            .all(|(a, b)| (*a - b).abs() < 3.)
                }
                _ => false,
            };
            self.scroll_offset > 100. && same_photo && same_window
        };
        let name = if result {
            format!("{}.PASS.txt", probe.mode.trim_start_matches('-'))
        } else {
            "FAIL.txt".into()
        };
        let _ = std::fs::write(
            probe.directory.join(name),
            format!(
                "{}: result={result}, offset={}, resume={:?}, window={:?}",
                probe.mode, self.scroll_offset, self.settings.resume, self.settings.window
            ),
        );
        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provisional_catalog_is_not_ready_for_session_probe() {
        assert!(!session_probe_ready(false, false, false, false));
        assert!(session_probe_ready(false, false, false, true));
    }
}
