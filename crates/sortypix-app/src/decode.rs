use std::path::Path;
/// Read proportions without decoding pixels; orientation must match the rendered image.
pub fn aspect_ratio(path: &Path) -> Option<f32> {
    match system_format(path) {
        Some(SystemFormat::Raw) => return raw_aspect_ratio(path),
        Some(SystemFormat::Other) => return other_aspect_ratio(path),
        None => {}
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
    match system_format(path) {
        Some(SystemFormat::Raw) => return decode_raw(path, max_edge),
        Some(SystemFormat::Other) => return decode_other(path, max_edge),
        None => {}
    }
    // WIC decodes a JPEG straight at a reduced DCT scale: 2.5 to 3 times faster than a full
    // decode then a resize on 12 and 24 MP photos (docs/recherche, 2026-10-22).
    #[cfg(windows)]
    if let Some(edge) = max_edge
        && matches!(crate::catalog::extension(path).as_str(), "jpg" | "jpeg")
        && let Ok(image) = crate::wic::decode(path, Some(edge))
    {
        return Ok(image);
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
    // Never enlarge: a bigger bucket than the photo would only cost memory and time.
    if let Some(edge) = max_edge.filter(|&edge| image.width().max(image.height()) > edge) {
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

/// Formats the image crate does not read: RAW, and HEIF/AVIF.
enum SystemFormat {
    Raw,
    Other,
}
fn system_format(path: &Path) -> Option<SystemFormat> {
    if crate::catalog::is_raw(path) {
        Some(SystemFormat::Raw)
    } else if matches!(
        crate::catalog::extension(path).as_str(),
        "heic" | "heif" | "avif"
    ) {
        Some(SystemFormat::Other)
    } else {
        None
    }
}
/// Windows decodes the sensor data when a RAW codec is installed; otherwise, and on other
/// systems, the preview the camera embedded stands in.
#[cfg(windows)]
fn decode_raw(path: &Path, max_edge: Option<u32>) -> Result<DecodedImage, String> {
    crate::wic::decode(path, max_edge)
        .or_else(|error| crate::embedded::decode(path, max_edge).map_err(|_| error))
}
#[cfg(not(windows))]
fn decode_raw(path: &Path, max_edge: Option<u32>) -> Result<DecodedImage, String> {
    crate::embedded::decode(path, max_edge)
}
#[cfg(windows)]
fn raw_aspect_ratio(path: &Path) -> Option<f32> {
    crate::wic::aspect_ratio(path).or_else(|| crate::embedded::aspect_ratio(path))
}
#[cfg(not(windows))]
fn raw_aspect_ratio(path: &Path) -> Option<f32> {
    crate::embedded::aspect_ratio(path)
}
#[cfg(windows)]
fn decode_other(path: &Path, max_edge: Option<u32>) -> Result<DecodedImage, String> {
    crate::wic::decode(path, max_edge)
}
#[cfg(not(windows))]
fn decode_other(path: &Path, _max_edge: Option<u32>) -> Result<DecodedImage, String> {
    Err(format!(
        "{} cannot be shown: HEIC and AVIF photos are not supported on this system yet",
        path.display()
    ))
}
#[cfg(windows)]
fn other_aspect_ratio(path: &Path) -> Option<f32> {
    crate::wic::aspect_ratio(path)
}
#[cfg(not(windows))]
fn other_aspect_ratio(_path: &Path) -> Option<f32> {
    None
}
