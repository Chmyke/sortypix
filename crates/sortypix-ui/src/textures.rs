//! ROLE: Bounded image workers and GPU texture cache for the visible viewport.
//! INVARIANTS: Animations have their own byte budget and never evict what is on screen;
//! one that cannot fit is declined until it leaves the screen, its still preview stays.
use crate::preview_queue::{Key, PreviewQueue, Priority};
use eframe::egui;
use sortypix_app::{
    animation::Timeline,
    cache::{Cache, accept_result},
    decode::DecodedImage,
    thumbnail_cache::ThumbnailCache,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
    sync::{Arc, mpsc},
    time::{Duration, Instant},
};
const QUICK_EDGE: u32 = 256;
const STILL_BUDGET: usize = 256 * 1024 * 1024;
/// All playing animations together; separate so GIFs never push photos out.
const ANIMATION_BUDGET: usize = 192 * 1024 * 1024;
/// One grid cell: about 100 frames at 320 px.
const CELL_ANIMATION_BUDGET: usize = 16 * 1024 * 1024;
/// The full-screen photo and its two preloaded neighbours fit together.
const VIEW_ANIMATION_BUDGET: usize = ANIMATION_BUDGET / 3;
/// Finer than the still buckets: a GIF costs its frame count in memory. Never `QUICK_EDGE`.
const ANIMATION_EDGES: [u32; 4] = [160, 320, 480, 640];
/// Frames converted on the worker, so the UI thread only hands them to the GPU.
struct Frames {
    images: Vec<egui::ColorImage>,
    timeline: Timeline,
    native: (u32, u32),
    bytes: usize,
}
enum Decoded {
    Still(DecodedImage),
    Animation(Frames),
}
struct Ready {
    key: Key,
    generation: u64,
    image: Result<Decoded, String>,
}
/// Playing GIF: one GPU texture per frame, so playback only swaps texture ids.
pub struct Animated {
    pub frames: Vec<egui::TextureHandle>,
    timeline: Timeline,
    /// Original canvas size, for layout and 100% zoom whatever the stored frame size.
    pub size: egui::Vec2,
    /// `egui` time of the upload: each animation starts on its first frame.
    pub start: f64,
}
impl Animated {
    /// Texture to draw at `now` (`egui` time) and the delay before the next one, if any.
    pub fn frame(&self, now: f64) -> (&egui::TextureHandle, Option<Duration>) {
        let elapsed = ((now - self.start).max(0.) * 1000.) as u64;
        let (index, next) = self.timeline.frame_at(elapsed);
        (&self.frames[index], next.map(Duration::from_millis))
    }
}
#[derive(Clone)]
pub enum Picture {
    Still(egui::TextureHandle),
    Animated(Arc<Animated>),
}
impl Picture {
    /// Size to lay out, in pixels of the original image.
    pub fn size_vec2(&self) -> egui::Vec2 {
        match self {
            Picture::Still(texture) => texture.size_vec2(),
            Picture::Animated(animation) => animation.size,
        }
    }
}
pub struct Textures {
    cache: Cache<Key, egui::TextureHandle>,
    animations: Cache<Key, Arc<Animated>>,
    pending: BTreeSet<Key>,
    desired: BTreeSet<Key>,
    /// Animations that could not fit beside the visible ones; retried once off screen.
    declined: BTreeSet<Key>,
    errors: BTreeMap<Key, String>,
    queue: Arc<PreviewQueue>,
    receive: mpsc::Receiver<Ready>,
    generation: u64,
    pub loaded: usize,
    /// Animations admitted to the GPU since the folder opened; probes check GIFs play.
    pub animated: usize,
}
/// Quick previews the system already has: the shell's thumbnails on Windows, the
/// previews embedded in the file elsewhere.
#[cfg(windows)]
fn system_preview(path: &std::path::Path) -> Option<sortypix_app::decode::DecodedImage> {
    sortypix_app::shell_thumbnail::cached(path, QUICK_EDGE)
        .or_else(|| sortypix_app::shell_thumbnail::extract(path, QUICK_EDGE))
}
#[cfg(not(windows))]
fn system_preview(path: &std::path::Path) -> Option<sortypix_app::decode::DecodedImage> {
    sortypix_app::embedded::quick(path, QUICK_EDGE)
}
/// Quick previews come from the system; refined ones animate when the file is an animated GIF.
fn decode(disk: &ThumbnailCache, (path, edge): &Key) -> Result<Decoded, String> {
    if *edge == QUICK_EDGE {
        return system_preview(path)
            .map(Ok)
            .unwrap_or_else(|| disk.decode(path, QUICK_EDGE))
            .map(Decoded::Still);
    }
    let (max_edge, budget) = match *edge {
        0 => (None, VIEW_ANIMATION_BUDGET),
        edge => (Some(edge), CELL_ANIMATION_BUDGET),
    };
    // A broken or oversized animation still shows as its first frame.
    let Ok(Some(animation)) = sortypix_app::animation::decode(path, max_edge, budget) else {
        return disk.decode(path, *edge).map(Decoded::Still);
    };
    let size = [animation.width as usize, animation.height as usize];
    Ok(Decoded::Animation(Frames {
        bytes: animation.bytes(),
        images: animation
            .frames
            .into_iter()
            .map(|rgba| egui::ColorImage::from_rgba_unmultiplied(size, &rgba))
            .collect(),
        timeline: animation.timeline,
        native: animation.native,
    }))
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
                    let image = decode(&disk, &request.key);
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
            cache: Cache::new(STILL_BUDGET),
            animations: Cache::new(ANIMATION_BUDGET),
            pending: BTreeSet::new(),
            desired: BTreeSet::new(),
            declined: BTreeSet::new(),
            errors: BTreeMap::new(),
            queue,
            receive,
            generation: 0,
            loaded: 0,
            animated: 0,
        }
    }
    pub fn clear(&mut self) {
        self.generation += 1;
        self.cache = Cache::new(STILL_BUDGET);
        self.animations = Cache::new(ANIMATION_BUDGET);
        self.pending.clear();
        self.desired.clear();
        self.declined.clear();
        self.errors.clear();
        self.loaded = 0;
        self.animated = 0;
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
                Ok(Decoded::Animation(animation)) => {
                    let bytes = animation.bytes;
                    let start = ctx.input(|i| i.time);
                    let frames = animation
                        .images
                        .into_iter()
                        .map(|image| {
                            ctx.load_texture(
                                result.key.0.to_string_lossy(),
                                image,
                                egui::TextureOptions::LINEAR,
                            )
                        })
                        .collect();
                    let animated = Arc::new(Animated {
                        frames,
                        timeline: animation.timeline,
                        size: egui::vec2(animation.native.0 as f32, animation.native.1 as f32),
                        start,
                    });
                    let desired = &self.desired;
                    if self.animations.insert_protecting(
                        result.key.clone(),
                        animated,
                        bytes,
                        |key| desired.contains(key),
                    ) {
                        self.loaded += 1;
                        self.animated += 1;
                        imported += 1;
                    } else {
                        // Dropping the handles frees the frames; the still preview remains.
                        self.declined.insert(result.key);
                    }
                    if bytes > 8 * 1024 * 1024 {
                        break;
                    }
                }
                Ok(Decoded::Still(image)) => {
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
    /// Every preview wanted by the last frame is loaded, failed or declined.
    pub fn settled(&self) -> bool {
        self.pending.is_empty()
    }
    pub fn begin_frame(&mut self) {
        self.desired.clear();
    }
    pub fn finish_frame(&mut self) {
        self.queue.retain(self.generation, &self.desired);
        self.pending.retain(|key| self.desired.contains(key));
        self.declined.retain(|key| self.desired.contains(key));
    }
    pub fn cached(&mut self, path: &std::path::Path, edge: u32) -> Option<Picture> {
        let key = (path.to_path_buf(), edge);
        if let Some(animation) = self.animations.get(&key) {
            return Some(Picture::Animated(animation.clone()));
        }
        self.cache.get(&key).cloned().map(Picture::Still)
    }
    fn request(
        &mut self,
        path: &std::path::Path,
        edge: u32,
        priority: Priority,
    ) -> Option<Picture> {
        let key = (path.to_path_buf(), edge);
        self.desired.insert(key.clone());
        if let Some(picture) = self.cached(path, edge) {
            return Some(picture);
        }
        if !self.pending.contains(&key)
            && !self.errors.contains_key(&key)
            && !self.declined.contains(&key)
        {
            self.queue.request(key.clone(), priority, self.generation);
            self.pending.insert(key);
        }
        None
    }
    pub fn visible(&mut self, path: &std::path::Path, edge: u32) -> Option<Picture> {
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
    pub fn immediate(&mut self, path: &std::path::Path, edge: u32) -> Option<Picture> {
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
/// Draws the picture; an animation asks for a repaint exactly when its next frame is due.
pub fn paint(painter: &egui::Painter, rect: egui::Rect, picture: &Picture, turns: u8) {
    let texture = match picture {
        Picture::Still(texture) => texture,
        Picture::Animated(animation) => {
            let ctx = painter.ctx();
            let (texture, next) = animation.frame(ctx.input(|i| i.time));
            if let Some(delay) = next {
                ctx.request_repaint_after(delay);
            }
            texture
        }
    };
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
/// Pixels to request for a grid cell; animated GIFs use finer steps to bound their memory.
pub fn display_edge(path: &std::path::Path, size: egui::Vec2, pixels_per_point: f32) -> u32 {
    let edge = size.max_elem() * pixels_per_point;
    if sortypix_app::catalog::extension(path) == "gif"
        && let Some(step) = ANIMATION_EDGES.into_iter().find(|n| *n as f32 >= edge)
    {
        return step;
    }
    thumbnail_edge(size, pixels_per_point)
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
    #[test]
    fn grid_animations_are_sized_for_the_cell_not_the_still_buckets() {
        let gif = std::path::Path::new("a.GIF");
        let jpeg = std::path::Path::new("a.jpg");
        assert_eq!(display_edge(gif, egui::vec2(120., 60.), 1.), 160);
        assert_eq!(display_edge(gif, egui::vec2(240., 160.), 1.25), 320);
        assert_eq!(display_edge(gif, egui::vec2(1200., 800.), 1.), 1600);
        assert_eq!(display_edge(jpeg, egui::vec2(120., 60.), 1.), 800);
        for size in [10., 150., 300., 700.] {
            assert_ne!(display_edge(gif, egui::vec2(size, size), 1.), QUICK_EDGE);
        }
    }
    #[test]
    fn gifs_play_in_the_full_screen_view_and_the_grid() {
        use image::{Delay, Frame, Rgba, RgbaImage, codecs::gif::GifEncoder};
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("loop.gif");
        GifEncoder::new(std::fs::File::create(&path).unwrap())
            .encode_frames([[255, 0, 0], [0, 255, 0], [0, 0, 255]].map(|[r, g, b]| {
                Frame::from_parts(
                    RgbaImage::from_pixel(40, 20, Rgba([r, g, b, 255])),
                    0,
                    0,
                    Delay::from_numer_denom_ms(100, 1),
                )
            }))
            .unwrap();
        let ctx = egui::Context::default();
        let mut textures = Textures::new(ctx.clone(), dir.path().join("cache"));
        let mut animated = |request: &mut dyn FnMut(&mut Textures) -> Option<Picture>| {
            let started = Instant::now();
            while started.elapsed() < Duration::from_secs(10) {
                textures.begin_frame();
                let picture = request(&mut textures);
                textures.finish_frame();
                if let Some(Picture::Animated(animation)) = picture {
                    return animation;
                }
                textures.poll(&ctx);
                std::thread::sleep(Duration::from_millis(5));
            }
            panic!("the GIF never became animated");
        };

        let viewer = animated(&mut |t| t.immediate(&path, 0));
        assert_eq!(viewer.frames.len(), 3);
        assert_eq!(viewer.size, egui::vec2(40., 20.));
        let first = viewer.frame(viewer.start).0.id();
        let (second, next) = viewer.frame(viewer.start + 0.15);
        assert_ne!(second.id(), first);
        assert!(next.is_some_and(|d| d <= Duration::from_millis(50)));

        let edge = display_edge(&path, egui::vec2(120., 60.), 1.);
        let grid = animated(&mut |t| t.visible(&path, edge));
        assert_eq!(grid.frames.len(), 3);
        assert_eq!(grid.size, egui::vec2(40., 20.));
    }
}
