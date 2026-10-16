//! ROLE: Read-only Windows codec adapter; COM objects remain on the calling worker.
use crate::decode::{DecodedImage, Resolution};
use std::{os::windows::ffi::OsStrExt, path::Path};
use windows::{
    Win32::{Foundation::GENERIC_READ, Graphics::Imaging::*, System::Com::*},
    core::{Interface, PCWSTR},
};
struct ComGuard;
pub fn aspect_ratio(path: &Path) -> Option<f32> {
    let (mut width, mut height) = unsafe {
        CoInitializeEx(None, COINIT_MULTITHREADED).ok().ok()?;
        let _com = ComGuard;
        let factory: IWICImagingFactory =
            CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER).ok()?;
        let name: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
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
        (width, height)
    };
    if matches!(crate::metadata::orientation(path), 5..=8) {
        std::mem::swap(&mut width, &mut height);
    }
    (width > 0 && height > 0).then_some(width as f32 / height as f32)
}
impl Drop for ComGuard {
    fn drop(&mut self) {
        unsafe { CoUninitialize() };
    }
}
pub fn decode(path: &Path, max_edge: Option<u32>) -> Result<DecodedImage, String> {
    // PITFALL: All COM interfaces and buffers are owned by this thread, within this scope.
    let mut decoded = unsafe { decode_com(path, max_edge) }.map_err(|e| {
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
unsafe fn decode_com(path: &Path, max_edge: Option<u32>) -> windows::core::Result<DecodedImage> {
    // SAFETY: pointers refer to initialized local buffers; COM lifetime is paired by the guard.
    unsafe {
        CoInitializeEx(None, COINIT_MULTITHREADED).ok()?;
        let _com = ComGuard;
        let factory: IWICImagingFactory =
            CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER)?;
        let name: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
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
