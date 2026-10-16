use image::{Rgb, RgbImage};
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let dir = std::env::args().nth(1).ok_or("output directory required")?;
    std::fs::create_dir_all(&dir)?;
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
