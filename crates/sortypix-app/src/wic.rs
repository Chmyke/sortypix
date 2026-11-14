//! ROLE: Read-only Windows codec adapter; COM objects remain on the calling worker.
//! INVARIANTS: COM is entered once per thread and left when the thread ends; the imaging
//! factory lives as long as its thread, so a decode never pays for creating it.
use crate::decode::{DecodedImage, Resolution};
use std::{os::windows::ffi::OsStrExt, path::Path};
use windows::{
    Win32::{Foundation::GENERIC_READ, Graphics::Imaging::*, System::Com::*},
    core::{Interface, PCWSTR},
};

/// Leaves COM when the thread ends.
struct ComGuard;
impl Drop for ComGuard {
    fn drop(&mut self) {
        unsafe { CoUninitialize() };
    }
}
/// The factory before the guard: fields drop in order, so COM is left after its last object.
struct Wic {
    factory: IWICImagingFactory,
    _com: ComGuard,
}
// Each entry pairs its own CoInitializeEx with its CoUninitialize (COM counts them per
// thread), so the order in which thread-local destructors run does not matter.
thread_local! {
    static COM: Option<ComGuard> = enter_com();
    static WIC: Option<Wic> = {
        let com = enter_com()?;
        let factory = unsafe {
            CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER).ok()?
        };
        Some(Wic { factory, _com: com })
    };
}
fn enter_com() -> Option<ComGuard> {
    unsafe { CoInitializeEx(None, COINIT_MULTITHREADED).ok().ok()? };
    Some(ComGuard)
}
/// Enters COM on this thread for its lifetime; `false` when the thread cannot (an apartment
/// of another kind already owns it).
pub(crate) fn ensure_com() -> bool {
    COM.with(|com| com.is_some())
}
fn with_factory<R>(f: impl FnOnce(&IWICImagingFactory) -> R) -> Option<R> {
    WIC.with(|wic| wic.as_ref().map(|wic| f(&wic.factory)))
}
fn wide(path: &Path) -> Vec<u16> {
    path.as_os_str().encode_wide().chain(Some(0)).collect()
}

/// Stored pixel size of the first frame, as the codec reports it (not turned).
pub fn size(path: &Path) -> Option<(u32, u32)> {
    let (width, height) = with_factory(|factory| unsafe {
        let name = wide(path);
        let decoder = factory
            .CreateDecoderFromFilename(
                PCWSTR(name.as_ptr()),
                None,
                GENERIC_READ,
                WICDecodeMetadataCacheOnDemand,
            )
            .ok()?;
        let frame = decoder.GetFrame(0).ok()?;
        let (mut width, mut height) = (0, 0);
        frame.GetSize(&mut width, &mut height).ok()?;
        Some((width, height))
    })??;
    (width > 0 && height > 0).then_some((width, height))
}
pub fn aspect_ratio(path: &Path) -> Option<f32> {
    let (mut width, mut height) = size(path)?;
    if matches!(crate::metadata::orientation(path), 5..=8) {
        std::mem::swap(&mut width, &mut height);
    }
    Some(width as f32 / height as f32)
}
pub fn decode(path: &Path, max_edge: Option<u32>) -> Result<DecodedImage, String> {
    // PITFALL: All COM interfaces and buffers are owned by this thread, within this scope.
    let mut decoded = with_factory(|factory| unsafe { decode_com(factory, path, max_edge) })
        .unwrap_or_else(|| {
            Err(windows::core::Error::new(
                windows::Win32::Foundation::E_FAIL,
                "Windows Imaging Component is not available on this thread",
            ))
        })
        .map_err(|e| {
            format!(
                "Cannot decode {}. Windows may need a compatible image codec: {e}",
                path.display()
            )
        })?;
    let orientation = crate::metadata::orientation(path);
    if orientation != 1 {
        let buffer = image::RgbaImage::from_raw(decoded.width, decoded.height, decoded.rgba)
            .ok_or("Invalid decoded buffer")?;
        let mut image = image::DynamicImage::ImageRgba8(buffer);
        if let Some(orientation) = image::metadata::Orientation::from_exif(orientation as u8) {
            image.apply_orientation(orientation);
        }
        decoded.width = image.width();
        decoded.height = image.height();
        decoded.rgba = image.into_rgba8().into_raw();
    }
    Ok(decoded)
}
unsafe fn decode_com(
    factory: &IWICImagingFactory,
    path: &Path,
    max_edge: Option<u32>,
) -> windows::core::Result<DecodedImage> {
    // SAFETY: pointers refer to initialized local buffers; COM lifetime is the thread's.
    unsafe {
        let name = wide(path);
        let decoder = factory.CreateDecoderFromFilename(
            PCWSTR(name.as_ptr()),
            None,
            GENERIC_READ,
            WICDecodeMetadataCacheOnDemand,
        )?;
        let frame = decoder.GetFrame(0)?;
        let (mut width, mut height) = (0, 0);
        frame.GetSize(&mut width, &mut height)?;
        let native = (width, height);
        if width == 0 || height == 0 || (width as u64 * height as u64) > 128 * 1024 * 1024 {
            return Err(windows::core::Error::new(
                windows::Win32::Foundation::E_INVALIDARG,
                "Image exceeds the decoding budget",
            ));
        }
        let mut source: IWICBitmapSource = frame.cast()?;
        if let Some(edge) = max_edge {
            let ratio = (edge.max(1) as f64 / width.max(height) as f64).min(1.0);
            width = (width as f64 * ratio).round().max(1.0) as u32;
            height = (height as f64 * ratio).round().max(1.0) as u32;
            let scaler = factory.CreateBitmapScaler()?;
            scaler.Initialize(&source, width, height, WICBitmapInterpolationModeFant)?;
            source = scaler.cast()?;
        }
        let converter = factory.CreateFormatConverter()?;
        converter.Initialize(
            &source,
            &GUID_WICPixelFormat32bppRGBA,
            WICBitmapDitherTypeNone,
            None,
            0.0,
            WICBitmapPaletteTypeCustom,
        )?;
        let mut rgba = vec![0; width as usize * height as usize * 4];
        converter.CopyPixels(std::ptr::null(), width * 4, &mut rgba)?;
        Ok(DecodedImage {
            rgba,
            width,
            height,
            resolution: if native == (width, height) {
                Resolution::Native
            } else {
                Resolution::Preview
            },
        })
    }
}
