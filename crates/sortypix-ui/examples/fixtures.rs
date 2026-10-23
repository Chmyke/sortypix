use image::{Rgb, RgbImage};
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let dir = std::env::args().nth(1).ok_or("output directory required")?;
    std::fs::create_dir_all(&dir)?;
    if std::env::args().nth(2).as_deref() == Some("--gif") {
        return animated(&dir);
    }
    if std::env::args().nth(2).as_deref() == Some("--large") {
        return large(&dir);
    }
    for n in 0..28u32 {
        let (w, h) = [
            (1200, 800),
            (600, 900),
            (1600, 600),
            (800, 800),
            (900, 1200),
            (1400, 800),
        ][n as usize % 6];
        let mut image = RgbImage::new(w, h);
        for (x, y, p) in image.enumerate_pixels_mut() {
            let sky = y as f32 / h as f32;
            let horizon = h as f32 * 0.54
                + h as f32 * 0.1 * ((x as f32 / 190.) + n as f32).sin()
                + h as f32 * 0.075 * (x as f32 / 81.).sin();
            let ridge =
                h as f32 * 0.7 + h as f32 * 0.08 * ((x as f32 / 125.) + n as f32 * 0.3).sin();
            let sun = ((x as f32 - w as f32 * (0.71 + (n % 4) as f32 * 0.02)).powi(2)
                + (y as f32 - h as f32 * 0.225).powi(2))
                < (w.min(h) as f32 * 0.08).powi(2);
            let color = if y as f32 > ridge {
                [
                    25 + (n * 5 % 50) as u8,
                    62 + (n * 3 % 35) as u8,
                    66 + (n * 4 % 25) as u8,
                ]
            } else if y as f32 > horizon {
                [
                    55 + (n * 4 % 60) as u8,
                    94 + (n * 3 % 50) as u8,
                    108 + (n * 2 % 45) as u8,
                ]
            } else if sun {
                [255, 220, 160]
            } else {
                [
                    (115. + sky * 35.) as u8,
                    (155. + sky * 25.) as u8,
                    (176. + sky * 20.) as u8,
                ]
            };
            *p = Rgb(color);
        }
        image.save(format!("{dir}/Alpine-{:03}.jpg", n + 1))?;
    }
    Ok(())
}

/// 200 camera-sized JPEGs (4000 x 3000, quality 90, noisy like a sensor), for the Sortypix AI
/// benchmark. Files already there are kept: the folder lives under `target/` and is reused.
fn large(dir: &str) -> Result<(), Box<dyn std::error::Error>> {
    let next = std::sync::atomic::AtomicU32::new(0);
    let threads = std::thread::available_parallelism().map_or(4, |n| n.get().min(16));
    std::thread::scope(|s| {
        let workers: Vec<_> = (0..threads)
            .map(|_| {
                s.spawn(|| -> Result<(), String> {
                    loop {
                        let n = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        if n >= 200 {
                            return Ok(());
                        }
                        let path = std::path::Path::new(dir).join(format!("Large-{n:03}.jpg"));
                        if std::fs::metadata(&path).is_ok_and(|m| m.len() > 0) {
                            continue;
                        }
                        large_photo(&path, n).map_err(|e| e.to_string())?;
                    }
                })
            })
            .collect();
        workers.into_iter().try_for_each(|w| {
            w.join()
                .map_err(|_| "fixture thread panicked".to_string())?
        })
    })?;
    Ok(())
}

fn large_photo(path: &std::path::Path, n: u32) -> Result<(), Box<dyn std::error::Error>> {
    let (w, h) = (4000u32, 3000u32);
    let mut rng = 0x9E37_79B9_7F4A_7C15u64 ^ u64::from(n + 1);
    let mut image = RgbImage::new(w, h);
    for (x, y, p) in image.enumerate_pixels_mut() {
        rng ^= rng << 13;
        rng ^= rng >> 7;
        rng ^= rng << 17;
        let noise = (rng & 31) as i32 - 16;
        let (xf, yf) = (x as f32 * 0.3, y as f32);
        let horizon = h as f32 * (0.54 + 0.1 * (xf / 190. + n as f32).sin());
        let base = if yf > horizon {
            [55 + (n * 4 % 60), 94 + (n * 3 % 50), 108 + (n * 2 % 45)]
        } else {
            let sky = yf / h as f32;
            [
                (115. + sky * 35.) as u32,
                (155. + sky * 25.) as u32,
                (176. + sky * 20.) as u32,
            ]
        };
        *p = Rgb(base.map(|c| (c as i32 + noise).clamp(0, 255) as u8));
    }
    // Under a temporary name, then renamed: an interrupted run leaves no truncated photo.
    let temp = path.with_extension("tmp");
    let mut file = std::io::BufWriter::new(std::fs::File::create(&temp)?);
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut file, 90).encode_image(&image)?;
    std::io::Write::flush(&mut file)?;
    drop(file);
    std::fs::rename(&temp, path)?;
    Ok(())
}

/// Distinct looping GIFs: a ball crossing a tinted background, one colour per file.
fn animated(dir: &str) -> Result<(), Box<dyn std::error::Error>> {
    use image::{Delay, Frame, Rgba, RgbaImage, codecs::gif::GifEncoder};
    let (w, h, frames) = (480u32, 270u32, 36u32);
    for n in 0..40u32 {
        let file = std::fs::File::create(format!("{dir}/Loop-{:03}.gif", n + 1))?;
        let mut encoder = GifEncoder::new_with_speed(file, 30);
        encoder.set_repeat(image::codecs::gif::Repeat::Infinite)?;
        let background = Rgba([(n * 37 % 200) as u8, (n * 71 % 200) as u8, 90, 255]);
        encoder.encode_frames((0..frames).map(|f| {
            let mut image = RgbaImage::from_pixel(w, h, background);
            let cx = (w * f / frames) as f32;
            for (x, y, p) in image.enumerate_pixels_mut() {
                if (x as f32 - cx).powi(2) + (y as f32 - h as f32 / 2.).powi(2) < 900. {
                    *p = Rgba([250, 240, 220, 255]);
                }
            }
            Frame::from_parts(image, 0, 0, Delay::from_numer_denom_ms(40, 1))
        }))?;
    }
    Ok(())
}
