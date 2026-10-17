//! ROLE: Decodes animated GIFs into uniform, memory-bounded frames and times their playback.
//! INVARIANTS: All frames share one size; the frame bytes never exceed the caller's budget.
use std::{fs, io::BufReader, path::Path};

/// Below this edge an animation is no longer worth showing; the caller keeps a still.
const MIN_EDGE: u32 = 64;

pub struct Animation {
    /// Size of every stored frame, after downscaling.
    pub width: u32,
    pub height: u32,
    /// Size of the original canvas, which drives layout and zoom.
    pub native: (u32, u32),
    /// Full-canvas RGBA frames, already composited.
    pub frames: Vec<Vec<u8>>,
    pub timeline: Timeline,
}

impl Animation {
    pub fn bytes(&self) -> usize {
        self.frames.iter().map(Vec::len).sum()
    }
}

/// When each frame shows, from cumulative frame ends in milliseconds.
#[derive(Clone, Debug)]
pub struct Timeline {
    ends: Vec<u64>,
    plays: Option<u32>,
}

impl Timeline {
    /// `plays` counts complete plays; `None` loops forever.
    pub fn new(delays_ms: &[u32], plays: Option<u32>) -> Self {
        let ends = delays_ms
            .iter()
            .scan(0, |end, delay| {
                *end += u64::from((*delay).max(1));
                Some(*end)
            })
            .collect();
        Self { ends, plays }
    }

    pub fn duration_ms(&self) -> u64 {
        self.ends.last().copied().unwrap_or(0)
    }

    pub fn plays(&self) -> Option<u32> {
        self.plays
    }

    /// Frame to show after `elapsed_ms`, and the time until it changes (`None`: never).
    pub fn frame_at(&self, elapsed_ms: u64) -> (usize, Option<u64>) {
        let duration = self.duration_ms();
        let last = self.ends.len().saturating_sub(1);
        if duration == 0
            || self
                .plays
                .is_some_and(|plays| elapsed_ms >= duration * u64::from(plays))
        {
            return (last, None);
        }
        let position = elapsed_ms % duration;
        let index = self.ends.partition_point(|end| *end <= position);
        (index, Some(self.ends[index] - position))
    }
}

/// `Ok(None)` for anything that is not an animated GIF: the caller decodes it as a still.
/// Frames fit within `max_edge` and, by halving their size, within `budget` bytes.
pub fn decode(
    path: &Path,
    max_edge: Option<u32>,
    budget: usize,
) -> Result<Option<Animation>, String> {
    use image::{AnimationDecoder, ImageDecoder, codecs::gif::GifDecoder, imageops};
    if crate::catalog::extension(path) != "gif" {
        return Ok(None);
    }
    let file = fs::File::open(path).map_err(|e| e.to_string())?;
    let mut decoder = GifDecoder::new(BufReader::new(file)).map_err(|e| e.to_string())?;
    let mut limits = image::Limits::default();
    limits.max_alloc = Some(512 * 1024 * 1024);
    decoder.set_limits(limits).map_err(|e| e.to_string())?;
    let native = decoder.dimensions();
    if native.0 == 0 || native.1 == 0 {
        return Err("Empty GIF canvas".into());
    }
    let plays = match decoder.loop_count() {
        image::metadata::LoopCount::Infinite => None,
        image::metadata::LoopCount::Finite(repeats) => Some(repeats.get().saturating_add(1)),
    };
    let (mut width, mut height) = fit(native, max_edge.unwrap_or(u32::MAX));
    let mut frames: Vec<Vec<u8>> = Vec::new();
    let mut delays = Vec::new();
    for frame in decoder.into_frames() {
        let frame = frame.map_err(|e| e.to_string())?;
        let (numerator, denominator) = frame.delay().numer_denom_ms();
        let delay = numerator / denominator.max(1);
        // Browsers play delays of 10 ms or less at 100 ms; so must we, for the same pace.
        delays.push(if delay <= 10 { 100 } else { delay });
        let mut canvas = frame.into_buffer();
        if canvas.dimensions() != (width, height) {
            canvas = imageops::thumbnail(&canvas, width, height);
        }
        frames.push(canvas.into_raw());
        while frames.len() * (width as usize) * (height as usize) * 4 > budget {
            let previous = (width, height);
            (width, height) = (width / 2, height / 2);
            if width.max(height) < MIN_EDGE || width == 0 || height == 0 {
                return Err("Animation exceeds the display memory budget".into());
            }
            for rgba in &mut frames {
                let image =
                    image::RgbaImage::from_raw(previous.0, previous.1, std::mem::take(rgba))
                        .ok_or("Inconsistent animation frame")?;
                *rgba = imageops::thumbnail(&image, width, height).into_raw();
            }
        }
    }
    if frames.len() < 2 {
        return Ok(None);
    }
    Ok(Some(Animation {
        width,
        height,
        native,
        frames,
        timeline: Timeline::new(&delays, plays),
    }))
}

/// Largest size within `edge` keeping proportions; never enlarges.
fn fit((width, height): (u32, u32), edge: u32) -> (u32, u32) {
    let edge = edge.max(1);
    if width <= edge && height <= edge {
        return (width, height);
    }
    let scale = f64::from(edge) / f64::from(width.max(height));
    (
        ((f64::from(width) * scale).round() as u32).max(1),
        ((f64::from(height) * scale).round() as u32).max(1),
    )
}
