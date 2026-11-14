use std::path::Path;
/// Read proportions without decoding pixels; orientation must match the rendered image.
pub fn aspect_ratio(path: &Path) -> Option<f32> {
    aspect_ratio_with(path, || crate::metadata::orientation(path))
}
/// [`aspect_ratio`] with the EXIF orientation supplied by the caller when it is needed, so
/// the catalog reads the EXIF of a photo once.
pub fn aspect_ratio_with(path: &Path, orientation: impl FnOnce() -> u16) -> Option<f32> {
    let turned = |(width, height): (u32, u32)| {
        let (width, height) = if matches!(orientation(), 5..=8) {
            (height, width)
        } else {
            (width, height)
        };
        (width > 0 && height > 0).then_some(width as f32 / height as f32)
    };
    match system_format(path) {
        Some(SystemFormat::Raw) => return raw_size(path).and_then(turned),
        Some(SystemFormat::Other) => return other_size(path).and_then(turned),
        None => {}
    }
    if crate::catalog::is_jpeg(path) {
        // The frame header alone: the image crate reads a whole JPEG to open it.
        return crate::embedded::jpeg_size(path).and_then(turned);
    }
    use image::ImageDecoder;
    if is_jxl(path) {
        // The decoder reports the oriented size.
        let (width, height) = jxl_decoder(path).ok()?.dimensions();
        return (width > 0 && height > 0).then_some(width as f32 / height as f32);
    }
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
        && crate::catalog::is_jpeg(path)
        && let Ok(image) = crate::wic::decode(path, Some(edge))
    {
        return Ok(image);
    }
    use image::ImageDecoder;
    if is_jxl(path) {
        let mut decoder = jxl_decoder(path)?;
        decoder.set_limits(limits()).map_err(|e| e.to_string())?;
        let image = image::DynamicImage::from_decoder(decoder).map_err(|e| e.to_string())?;
        return Ok(finish(image, max_edge));
    }
    let mut reader = image::ImageReader::open(path)
        .map_err(|e| e.to_string())?
        .with_guessed_format()
        .map_err(|e| e.to_string())?;
    reader.limits(limits());
    let mut decoder = reader.into_decoder().map_err(|e| e.to_string())?;
    let orientation = decoder.orientation().map_err(|e| e.to_string())?;
    let mut image = image::DynamicImage::from_decoder(decoder).map_err(|e| e.to_string())?;
    image.apply_orientation(orientation);
    Ok(finish(image, max_edge))
}
/// Starts the threads the decoders share (the global pool jxl-oxide decodes on), so they
/// exist before a priority change applies to threads created later (Sortypix AI lowers the
/// process and raises the threads it has at that moment).
pub fn start_shared_threads() {
    let _ = rayon_core::current_num_threads();
}
fn limits() -> image::Limits {
    let mut limits = image::Limits::default();
    limits.max_alloc = Some(512 * 1024 * 1024);
    limits
}
/// JPEG XL, read by jxl-oxide (pure Rust, MIT or Apache-2.0), oriented as the file says.
fn is_jxl(path: &Path) -> bool {
    crate::catalog::extension(path) == "jxl"
}
fn jxl_decoder(
    path: &Path,
) -> Result<jxl_oxide::integration::JxlDecoder<std::io::BufReader<std::fs::File>>, String> {
    let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    jxl_oxide::integration::JxlDecoder::new(std::io::BufReader::new(file))
        .map_err(|e| format!("{}: {e}", path.display()))
}
/// Scales `image` down to `max_edge`, never up.
fn finish(mut image: image::DynamicImage, max_edge: Option<u32>) -> DecodedImage {
    let native = (image.width(), image.height());
    // Never enlarge: a bigger bucket than the photo would only cost memory and time.
    if let Some(edge) = max_edge.filter(|&edge| image.width().max(image.height()) > edge) {
        image = image.thumbnail(edge.max(1), edge.max(1));
    }
    let (width, height) = (image.width(), image.height());
    DecodedImage {
        rgba: image.into_rgba8().into_raw(),
        width,
        height,
        resolution: if native == (width, height) {
            Resolution::Native
        } else {
            Resolution::Preview
        },
    }
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
/// A reduced size comes from the preview the camera embedded when it is large enough: three to
/// six times faster than the sensor (canon-350d.CR2, 2026-10-30). Windows decodes the sensor for
/// the native size (100 % zoom), and for a reduced size the previews are too small for;
/// without a RAW codec, and on other systems, the previews stand in.
#[cfg(windows)]
fn decode_raw(path: &Path, max_edge: Option<u32>) -> Result<DecodedImage, String> {
    // The previews are listed once, and none is decoded before its size is known to fit.
    let previews = crate::embedded::raw_previews(path);
    let preview = max_edge.and_then(|edge| {
        previews
            .last()
            .filter(|largest| large_enough(largest.edge(), edge))?;
        crate::embedded::decode_from(path, &previews, Some(edge))
            .ok()
            .filter(|image| large_enough(image.width.max(image.height), edge))
    });
    match preview {
        Some(image) => Ok(image),
        None => crate::wic::decode(path, max_edge).or_else(|error| {
            crate::embedded::decode_from(path, &previews, max_edge).map_err(|_| error)
        }),
    }
}
/// Within a few pixels of `edge` (previews and sensor differ by their margins).
#[cfg(windows)]
fn large_enough(longest: u32, edge: u32) -> bool {
    longest + 32 >= edge
}
#[cfg(not(windows))]
fn decode_raw(path: &Path, max_edge: Option<u32>) -> Result<DecodedImage, String> {
    crate::embedded::decode(path, max_edge)
}
/// Stored size of a RAW photo: the codec's, else its largest embedded preview's.
#[cfg(windows)]
fn raw_size(path: &Path) -> Option<(u32, u32)> {
    crate::wic::size(path).or_else(|| crate::embedded::largest_preview_size(path))
}
#[cfg(not(windows))]
fn raw_size(path: &Path) -> Option<(u32, u32)> {
    crate::embedded::largest_preview_size(path)
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
fn other_size(path: &Path) -> Option<(u32, u32)> {
    crate::wic::size(path)
}
#[cfg(not(windows))]
fn other_size(_path: &Path) -> Option<(u32, u32)> {
    None
}
