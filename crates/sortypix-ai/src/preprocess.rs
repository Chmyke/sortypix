use image::RgbImage;
use image::imageops::{self, FilterType};

use crate::AiError;

/// CLIP input side, in pixels.
pub const INPUT: u32 = 224;

const MEAN: [f32; 3] = [0.481_454_66, 0.457_827_5, 0.408_210_73];
const STD: [f32; 3] = [0.268_629_54, 0.261_302_6, 0.275_777_1];

/// CLIP preprocessing: RGBA pixels (alpha ignored) to a normalized NCHW tensor.
///
/// The centered square of the source (side = its short edge) is cropped first, then resized to
/// 224 x 224 (Catmull-Rom, bicubic), then each channel is normalized with the CLIP mean and
/// standard deviation.
pub fn preprocess(rgba: &[u8], width: u32, height: u32) -> Result<Vec<f32>, AiError> {
    let expected = (width as usize)
        .checked_mul(height as usize)
        .and_then(|n| n.checked_mul(4));
    if width == 0 || height == 0 || expected != Some(rgba.len()) {
        return Err(AiError::Image(format!(
            "buffer of {} bytes does not match {width}x{height} RGBA",
            rgba.len()
        )));
    }

    let mut rgb = Vec::with_capacity(rgba.len() / 4 * 3);
    for px in rgba.as_chunks::<4>().0 {
        rgb.extend_from_slice(&px[..3]);
    }
    let img = RgbImage::from_raw(width, height, rgb)
        .ok_or_else(|| AiError::Image("pixel buffer rejected".into()))?;

    // Crop the centered square in source coordinates first (offset rounded, as in
    // HF CLIPImageProcessor / torchvision), then resize it to INPUT x INPUT.
    // Memory stays bounded by the source size whatever the aspect ratio.
    let side = width.min(height);
    let left = (width - side).div_ceil(2);
    let top = (height - side).div_ceil(2);
    let square = imageops::crop_imm(&img, left, top, side, side).to_image();
    let crop = imageops::resize(&square, INPUT, INPUT, FilterType::CatmullRom);

    let plane = (INPUT * INPUT) as usize;
    let mut out = vec![0f32; 3 * plane];
    for (i, px) in crop.pixels().enumerate() {
        for c in 0..3 {
            out[c * plane + i] = (f32::from(px.0[c]) / 255.0 - MEAN[c]) / STD[c];
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    const PLANE: usize = (INPUT * INPUT) as usize;

    fn solid(w: u32, h: u32, px: [u8; 4]) -> Vec<u8> {
        px.iter()
            .copied()
            .cycle()
            .take((w * h * 4) as usize)
            .collect()
    }

    #[test]
    fn uniform_mid_gray_normalizes_per_channel() {
        let out = preprocess(&solid(300, 200, [128, 128, 128, 255]), 300, 200).unwrap();
        assert_eq!(out.len(), 3 * PLANE);
        for c in 0..3 {
            let want = (128.0 / 255.0 - MEAN[c]) / STD[c];
            for v in &out[c * PLANE..(c + 1) * PLANE] {
                assert!((v - want).abs() < 1e-5, "channel {c}: {v} vs {want}");
            }
        }
    }

    #[test]
    fn center_crop_keeps_middle() {
        // 448 x 224: the crop keeps x 112..336. Red only on the left (x < 112, cropped out),
        // green in the central quarter (x 168..280), blue elsewhere.
        let (w, h) = (448u32, 224u32);
        let mut buf = Vec::new();
        for _y in 0..h {
            for x in 0..w {
                let px = if (168..280).contains(&x) {
                    [0, 255, 0, 255]
                } else if x < 112 {
                    [255, 0, 0, 255]
                } else {
                    [0, 0, 255, 255]
                };
                buf.extend_from_slice(&px);
            }
        }
        let out = preprocess(&buf, w, h).unwrap();
        let at = |c: usize, x: usize, y: usize| out[c * PLANE + y * INPUT as usize + x];
        let norm = |c: usize, v: f32| (v - MEAN[c]) / STD[c];
        // Center pixel is green.
        assert!((at(1, 112, 112) - norm(1, 1.0)).abs() < 1e-3);
        assert!((at(0, 112, 112) - norm(0, 0.0)).abs() < 1e-3);
        // Columns 0 and 223 are not red (crop covers x 112..336).
        for x in [0usize, 223] {
            assert!(
                (at(0, x, 112) - norm(0, 1.0)).abs() > 0.5,
                "column {x} is red"
            );
        }
    }

    #[test]
    fn extreme_shapes_do_not_panic() {
        for (w, h) in [(1u32, 1u32), (8000, 300), (300, 8000)] {
            let out = preprocess(&solid(w, h, [10, 20, 30, 255]), w, h).unwrap();
            assert_eq!(out.len(), 150_528);
        }
    }

    #[test]
    fn huge_aspect_ratio_is_bounded() {
        let out = preprocess(&solid(1, 100_000, [10, 20, 30, 255]), 1, 100_000).unwrap();
        assert_eq!(out.len(), 150_528);
    }

    #[test]
    fn rejects_inconsistent_buffer() {
        assert!(matches!(preprocess(&[0; 15], 2, 2), Err(AiError::Image(_))));
        assert!(matches!(preprocess(&[], 0, 4), Err(AiError::Image(_))));
        assert!(matches!(preprocess(&[], 4, 0), Err(AiError::Image(_))));
    }
}
