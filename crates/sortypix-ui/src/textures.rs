//! ROLE: Bounded image workers and GPU texture cache for the visible viewport.
use crate::preview_queue::{Key, PreviewQueue, Priority};
use eframe::egui;
use sortypix_app::{
    cache::{Cache, accept_result},
    thumbnail_cache::ThumbnailCache,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
    sync::{Arc, mpsc},
    time::{Duration, Instant},
};
const QUICK_EDGE: u32 = 256;
struct Ready {
    key: Key,
    generation: u64,
    image: Result<sortypix_app::decode::DecodedImage, String>,
}
pub struct Textures {
    cache: Cache<Key, egui::TextureHandle>,
    pending: BTreeSet<Key>,
    desired: BTreeSet<Key>,
    errors: BTreeMap<Key, String>,
    queue: Arc<PreviewQueue>,
    receive: mpsc::Receiver<Ready>,
    generation: u64,
    pub loaded: usize,
}
impl Textures {
    pub fn new(context: egui::Context, cache_dir: PathBuf) -> Self {
        let disk = Arc::new(ThumbnailCache::new(cache_dir, 512 * 1024 * 1024));
        let (results, receive) = mpsc::sync_channel::<Ready>(8);
        let queue = Arc::new(PreviewQueue::new());
        for _ in 0..4 {
            let disk = disk.clone();
            let queue = queue.clone();
            let results = results.clone();
            let ctx = context.clone();
            std::thread::spawn(move || {
                while let Some(request) = queue.pop() {
                    let image = if request.key.1 == QUICK_EDGE {
                        sortypix_app::shell_thumbnail::cached(&request.key.0, QUICK_EDGE)
                            .or_else(|| {
                                sortypix_app::shell_thumbnail::extract(&request.key.0, QUICK_EDGE)
                            })
                            .map(Ok)
                            .unwrap_or_else(|| disk.decode(&request.key.0, QUICK_EDGE))
                    } else {
                        disk.decode(&request.key.0, request.key.1)
                    };
                    if results
                        .send(Ready {
                            key: request.key,
                            generation: request.generation,
                            image,
                        })
                        .is_err()
                    {
                        break;
                    }
                    ctx.request_repaint();
                }
            });
        }
        Self {
            cache: Cache::new(256 * 1024 * 1024),
            pending: BTreeSet::new(),
            desired: BTreeSet::new(),
            errors: BTreeMap::new(),
            queue,
            receive,
            generation: 0,
            loaded: 0,
        }
    }
    pub fn clear(&mut self) {
        self.generation += 1;
        self.cache = Cache::new(256 * 1024 * 1024);
        self.pending.clear();
        self.desired.clear();
        self.errors.clear();
        self.loaded = 0;
        self.queue.set_generation(self.generation);
    }
    pub fn poll(&mut self, ctx: &egui::Context) {
        let started = Instant::now();
        let mut imported = 0;
        loop {
            if imported > 0 && started.elapsed() >= Duration::from_millis(3) {
                break;
            }
            let Ok(result) = self.receive.try_recv() else {
                break;
            };
            if !accept_result(self.generation, result.generation) {
                continue;
            }
            if !self.pending.remove(&result.key) {
                continue;
            }
            match result.image {
                Ok(image) => {
                    if result.key.1 == 0
                        && image.resolution != sortypix_app::decode::Resolution::Native
                    {
                        self.errors
                            .insert(result.key, "Native resolution unavailable".into());
                        continue;
                    }
                    let bytes = image.rgba.len();
                    if bytes > 256 * 1024 * 1024 {
                        self.errors
                            .insert(result.key, "Image exceeds the display memory budget".into());
                        continue;
                    }
                    let color = egui::ColorImage::from_rgba_unmultiplied(
                        [image.width as usize, image.height as usize],
                        &image.rgba,
                    );
                    let texture = ctx.load_texture(
                        result.key.0.to_string_lossy(),
                        color,
                        egui::TextureOptions::LINEAR,
                    );
                    self.cache.insert(result.key, texture, bytes);
                    self.loaded += 1;
                    imported += 1;
                    if bytes > 8 * 1024 * 1024 {
                        break;
                    }
                }
                Err(error) => {
                    self.errors.insert(result.key, error);
                }
            }
        }
    }
    pub fn begin_frame(&mut self) {
        self.desired.clear();
    }
    pub fn finish_frame(&mut self) {
        self.queue.retain(self.generation, &self.desired);
        self.pending.retain(|key| self.desired.contains(key));
    }
    pub fn cached(&mut self, path: &std::path::Path, edge: u32) -> Option<egui::TextureHandle> {
        self.cache.get(&(path.to_path_buf(), edge)).cloned()
    }
    fn request(
        &mut self,
        path: &std::path::Path,
        edge: u32,
        priority: Priority,
    ) -> Option<egui::TextureHandle> {
        let key = (path.to_path_buf(), edge);
        self.desired.insert(key.clone());
        if let Some(texture) = self.cache.get(&key) {
            return Some(texture.clone());
        }
        if !self.pending.contains(&key) && !self.errors.contains_key(&key) {
            self.queue.request(key.clone(), priority, self.generation);
            self.pending.insert(key);
        }
        None
    }
    pub fn visible(&mut self, path: &std::path::Path, edge: u32) -> Option<egui::TextureHandle> {
        let [quick_edge, refined_edge] = request_edges(edge);
        if let Some(texture) = self.cached(path, refined_edge) {
            self.desired.insert((path.to_path_buf(), refined_edge));
            return Some(texture);
        }
        let quick = self.request(path, quick_edge, Priority::Visible);
        let priority = if quick.is_some() {
            Priority::Visible
        } else {
            Priority::Refine
        };
        let refined = self.request(path, refined_edge, priority);
        refined.or(quick)
    }
    pub fn prefetch(&mut self, path: &std::path::Path, edge: u32) {
        let _ = edge;
        let _ = self.request(path, QUICK_EDGE, Priority::Ahead);
    }
    pub fn immediate(&mut self, path: &std::path::Path, edge: u32) -> Option<egui::TextureHandle> {
        if let Some(texture) = self.cached(path, edge) {
            self.desired.insert((path.to_path_buf(), edge));
            return Some(texture);
        }
        let quick = self.request(path, QUICK_EDGE, Priority::Immediate);
        self.request(path, edge, Priority::Immediate).or(quick)
    }
    pub fn error(&self, path: &std::path::Path, edge: u32) -> Option<&str> {
        self.errors
            .get(&(path.to_owned(), edge))
            .map(String::as_str)
    }
}
impl Drop for Textures {
    fn drop(&mut self) {
        self.queue.close();
    }
}
pub fn paint(painter: &egui::Painter, rect: egui::Rect, texture: &egui::TextureHandle, turns: u8) {
    let mut mesh = egui::Mesh::with_texture(texture.id());
    let corners = [
        rect.left_top(),
        rect.right_top(),
        rect.right_bottom(),
        rect.left_bottom(),
    ];
    let uv = [
        egui::pos2(0., 0.),
        egui::pos2(1., 0.),
        egui::pos2(1., 1.),
        egui::pos2(0., 1.),
    ];
    for (index, pos) in corners.into_iter().enumerate() {
        mesh.vertices.push(egui::epaint::Vertex {
            pos,
            uv: uv[(index + 4 - turns as usize % 4) % 4],
            color: egui::Color32::WHITE,
        });
    }
    mesh.indices.extend([0, 1, 2, 0, 2, 3]);
    painter.add(egui::Shape::mesh(mesh));
}
pub fn fit(size: egui::Vec2, bounds: egui::Vec2, turns: u8) -> egui::Vec2 {
    let size = if turns % 2 == 1 {
        egui::vec2(size.y, size.x)
    } else {
        size
    };
    size * (bounds.x / size.x).min(bounds.y / size.y)
}

pub fn thumbnail_edge(size: egui::Vec2, pixels_per_point: f32) -> u32 {
    let edge = size.max_elem() * pixels_per_point;
    [800, 1600, 3200, 4096]
        .into_iter()
        .find(|n| *n as f32 >= edge)
        .unwrap_or(4096)
}
fn request_edges(refined_edge: u32) -> [u32; 2] {
    [QUICK_EDGE, refined_edge]
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn large_thumbnails_request_pixels_for_size_and_display_scale() {
        assert_eq!(thumbnail_edge(egui::vec2(240., 160.), 1.), 800);
        assert_eq!(thumbnail_edge(egui::vec2(1200., 800.), 1.), 1600);
        assert_eq!(thumbnail_edge(egui::vec2(1200., 800.), 2.), 3200);
        assert_eq!(thumbnail_edge(egui::vec2(1200., 2400.), 2.), 4096);
    }
    #[test]
    fn visible_cells_request_a_quick_preview_before_refined_pixels() {
        assert_eq!(
            request_edges(thumbnail_edge(egui::vec2(240., 160.), 1.)),
            [256, 800]
        );
        assert_eq!(
            request_edges(thumbnail_edge(egui::vec2(1200., 800.), 2.)),
            [256, 3200]
        );
    }
}
