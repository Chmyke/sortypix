//! ROLE: Measures the actual virtualized grid and preloaded viewer in a native window.
use eframe::egui;
use std::{path::PathBuf, time::Instant};
#[derive(Default)]
struct OpeningMetrics {
    provisional_ms: Option<f64>,
    first_preview_ms: Option<f64>,
    viewport_ms: Option<f64>,
    metadata_ms: Option<f64>,
}
impl OpeningMetrics {
    fn catalog(&mut self, final_catalog: bool, elapsed_ms: f64) {
        if final_catalog {
            self.metadata_ms.get_or_insert(elapsed_ms);
        } else {
            self.provisional_ms.get_or_insert(elapsed_ms);
        }
    }
    fn previews(&mut self, loaded: usize, elapsed_ms: f64) {
        if loaded > 0 {
            self.first_preview_ms.get_or_insert(elapsed_ms);
        }
        if loaded >= 12 {
            self.viewport_ms.get_or_insert(elapsed_ms);
        }
    }
}
pub struct Benchmark {
    folder: PathBuf,
    started: Instant,
    last: Instant,
    frame: u32,
    costs: Vec<f32>,
    intervals: Vec<f64>,
    open_ms: f64,
    next_started: Option<Instant>,
    pub scroll: f32,
    opening: OpeningMetrics,
    /// Frames can still run while Close is pending; the verdict is written once.
    reported: bool,
}
impl Benchmark {
    pub fn new(folder: PathBuf) -> Self {
        Self {
            folder,
            started: Instant::now(),
            last: Instant::now(),
            frame: 0,
            costs: Vec::new(),
            intervals: Vec::new(),
            open_ms: 0.,
            next_started: None,
            scroll: 0.,
            opening: OpeningMetrics::default(),
            reported: false,
        }
    }
    pub(crate) fn observe_catalog(&mut self, final_catalog: bool) {
        self.opening
            .catalog(final_catalog, self.started.elapsed().as_secs_f64() * 1000.);
    }
}
impl crate::gallery::Gallery {
    pub(crate) fn run_benchmark(&mut self, ctx: &egui::Context, cpu: Option<f32>) {
        let Some(b) = &mut self.benchmark else {
            return;
        };
        if b.reported {
            return;
        }
        if b.started.elapsed().as_secs() > 50 {
            let _ = std::fs::write(b.folder.join("FAIL.txt"), "Benchmark timed out");
            b.reported = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        }
        ctx.request_repaint();
        b.opening.previews(
            self.textures.loaded,
            b.started.elapsed().as_secs_f64() * 1000.,
        );
        if self.busy
            || self.scanning
            || b.opening.viewport_ms.is_none()
            || b.opening.metadata_ms.is_none()
        {
            return;
        }
        if b.frame == 0 {
            // Scroll only once the first screen is fully refined: at 125 px per frame,
            // cells leave before their refined preview or animation could arrive.
            if !self.textures.settled() {
                return;
            }
            let start = Instant::now();
            let originals = self.photos.clone();
            self.photos = (0..50000)
                .map(|n| {
                    let mut photo = originals[n % originals.len()].clone();
                    photo.id = sortypix_core::PhotoId(n as u64);
                    photo
                })
                .collect();
            self.turns = vec![0; 50000];
            self.layout.invalidate();
            b.open_ms = start.elapsed().as_secs_f64() * 1000.;
            b.last = Instant::now();
            b.frame = 1;
            return;
        }
        if b.frame < 181 {
            if b.frame > 30 {
                if let Some(cpu) = cpu {
                    b.costs.push(cpu * 1000.);
                }
                b.intervals.push(b.last.elapsed().as_secs_f64() * 1000.);
            }
            b.last = Instant::now();
            b.scroll = b.frame as f32 * 125.;
            b.frame += 1;
            return;
        }
        if b.frame == 181 {
            self.viewer = true;
            self.selected = 0;
            let source = &self.photos[1].members[0];
            if self.textures.immediate(source, 0).is_none() {
                return;
            }
            self.selected = 1;
            b.next_started = Some(Instant::now());
            b.frame += 1;
            return;
        }
        let next = b
            .next_started
            .map_or(f64::INFINITY, |start| start.elapsed().as_secs_f64() * 1000.);
        b.costs.sort_by(f32::total_cmp);
        b.intervals.sort_by(f64::total_cmp);
        let cpu = b
            .costs
            .get(b.costs.len() * 95 / 100)
            .copied()
            .unwrap_or(f32::INFINITY);
        let interval = b
            .intervals
            .get(b.intervals.len() * 95 / 100)
            .copied()
            .unwrap_or(f64::INFINITY);
        let text = format!(
            "Grid entries: 50000\nProvisional catalog: {:.3} ms\nFirst preview: {:.3} ms\nFirst viewport: {:.3} ms\nFinal metadata: {:.3} ms\nList construction: {:.3} ms\nRendered frame CPU p95: {cpu:.3} ms\nFrame interval p95: {interval:.3} ms\nPreloaded next image: {next:.3} ms\nAnimations loaded: {}\nCache: runtime\n",
            b.opening.provisional_ms.unwrap_or(f64::INFINITY),
            b.opening.first_preview_ms.unwrap_or(f64::INFINITY),
            b.opening.viewport_ms.unwrap_or(f64::INFINITY),
            b.opening.metadata_ms.unwrap_or(f64::INFINITY),
            b.open_ms,
            self.textures.animated,
        );
        let _ = std::fs::create_dir_all(&b.folder);
        let _ = std::fs::write(b.folder.join("performance.txt"), &text);
        let scale = std::env::var("SORTYPIX_BUDGET_SCALE")
            .ok()
            .and_then(|s| s.parse::<f64>().ok())
            .unwrap_or(1.);
        let passed = b
            .opening
            .provisional_ms
            .is_some_and(|ms| ms < 1000. * scale)
            && b.opening
                .first_preview_ms
                .is_some_and(|ms| ms < 500. * scale)
            && b.open_ms < 1000. * scale
            && (cpu as f64) < 16.7 * scale
            && interval < 16.7 * scale
            && next < 50. * scale;
        let _ = std::fs::write(
            b.folder.join(if passed { "PASS.txt" } else { "FAIL.txt" }),
            text,
        );
        b.reported = true;
        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opening_milestones_keep_the_first_observation_of_each_stage() {
        let mut metrics = OpeningMetrics::default();
        metrics.catalog(false, 40.0);
        metrics.catalog(false, 70.0);
        metrics.previews(1, 90.0);
        metrics.previews(20, 140.0);
        metrics.catalog(true, 180.0);

        assert_eq!(metrics.provisional_ms, Some(40.0));
        assert_eq!(metrics.first_preview_ms, Some(90.0));
        assert_eq!(metrics.viewport_ms, Some(140.0));
        assert_eq!(metrics.metadata_ms, Some(180.0));
    }
}
