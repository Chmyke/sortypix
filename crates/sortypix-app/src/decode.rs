use std::path::Path;
/// Read proportions without decoding pixels; orientation must match the rendered image.
pub fn aspect_ratio(path: &Path) -> Option<f32> {
    if crate::catalog::is_raw(path)
        || matches!(
            crate::catalog::extension(path).as_str(),
            "heic" | "heif" | "avif"
        )
    {
        return crate::wic::aspect_ratio(path);
    }
    use image::ImageDecoder;
    let mut decoder = image::ImageReader::open(path)
        .ok()?
        .with_guessed_format()
        .ok()?
        .into_decoder()
        .ok()?;
    let (mut width, mut height) = decoder.dimensions();
    if matches!(decoder.orientation().ok()?.to_exif(), 5..=8) {
        std::mem::swap(&mut width, &mut height);
    }
    (width > 0 && height > 0).then_some(width as f32 / height as f32)
}
#[derive(Debug, PartialEq, Eq)]
pub enum Resolution {
    Native,
    Preview,
}
pub struct DecodedImage {
    pub rgba: Vec<u8>,
    pub width: u32,
    pub height: u32,
    pub resolution: Resolution,
}
pub fn decode(path: &Path, max_edge: Option<u32>) -> Result<DecodedImage, String> {
    if crate::catalog::is_raw(path)
        || matches!(
            crate::catalog::extension(path).as_str(),
            "heic" | "heif" | "avif"
        )
    {
        return crate::wic::decode(path, max_edge);
    }
    use image::ImageDecoder;
    let mut reader = image::ImageReader::open(path)
        .map_err(|e| e.to_string())?
        .with_guessed_format()
        .map_err(|e| e.to_string())?;
    let mut limits = image::Limits::default();
    limits.max_alloc = Some(512 * 1024 * 1024);
    reader.limits(limits);
    let mut decoder = reader.into_decoder().map_err(|e| e.to_string())?;
    let orientation = decoder.orientation().map_err(|e| e.to_string())?;
    let mut image = image::DynamicImage::from_decoder(decoder).map_err(|e| e.to_string())?;
    image.apply_orientation(orientation);
    let native = (image.width(), image.height());
    if let Some(edge) = max_edge {
        image = image.thumbnail(edge.max(1), edge.max(1));
    }
    let (width, height) = (image.width(), image.height());
    Ok(DecodedImage {
        rgba: image.into_rgba8().into_raw(),
        width,
        height,
        resolution: if native == (width, height) {
            Resolution::Native
        } else {
            Resolution::Preview
        },
    })
}
