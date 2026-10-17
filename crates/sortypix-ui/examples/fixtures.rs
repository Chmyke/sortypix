use image::{Rgb, RgbImage};
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let dir = std::env::args().nth(1).ok_or("output directory required")?;
    std::fs::create_dir_all(&dir)?;
    if std::env::args().nth(2).as_deref() == Some("--gif") {
        return animated(&dir);
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
