//! ROLE: Retrieves disposable Windows Shell thumbnails without exposing COM or GDI handles.
//! SCOPE: Windows adapter used only by background preview workers.
//! INVARIANTS: Every acquired HBITMAP is deleted; malformed bitmap layouts are rejected.
use crate::decode::{DecodedImage, Resolution};
use std::{os::windows::ffi::OsStrExt, path::Path};
use windows::{
    Win32::{
        Foundation::SIZE,
        Graphics::Gdi::{DIBSECTION, DeleteObject, GetObjectW, HBITMAP},
        System::Com::{COINIT_MULTITHREADED, CoInitializeEx, CoUninitialize},
        UI::Shell::{
            IShellItemImageFactory, SHCreateItemFromParsingName, SIIGBF, SIIGBF_BIGGERSIZEOK,
            SIIGBF_INCACHEONLY, SIIGBF_THUMBNAILONLY,
        },
    },
    core::PCWSTR,
};

struct ComGuard;
impl Drop for ComGuard {
    fn drop(&mut self) {
        unsafe { CoUninitialize() };
    }
}

struct BitmapGuard(HBITMAP);
impl Drop for BitmapGuard {
    fn drop(&mut self) {
        unsafe {
            let _ = DeleteObject(self.0.into());
        }
    }
}

pub fn cached(path: &Path, edge: u32) -> Option<DecodedImage> {
    get(
        path,
        edge,
        SIIGBF_INCACHEONLY | SIIGBF_THUMBNAILONLY | SIIGBF_BIGGERSIZEOK,
    )
}

pub fn extract(path: &Path, edge: u32) -> Option<DecodedImage> {
    get(path, edge, SIIGBF_THUMBNAILONLY | SIIGBF_BIGGERSIZEOK)
}

fn get(path: &Path, edge: u32, flags: SIIGBF) -> Option<DecodedImage> {
    let edge = edge.clamp(1, 4096) as i32;
    let name: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    unsafe {
        CoInitializeEx(None, COINIT_MULTITHREADED).ok().ok()?;
        let _com = ComGuard;
        let factory: IShellItemImageFactory =
            SHCreateItemFromParsingName(PCWSTR(name.as_ptr()), None).ok()?;
        let bitmap = BitmapGuard(factory.GetImage(SIZE { cx: edge, cy: edge }, flags).ok()?);
        copy_bitmap(bitmap.0)
    }
}

unsafe fn copy_bitmap(bitmap: HBITMAP) -> Option<DecodedImage> {
    let mut section = DIBSECTION::default();
    let copied = unsafe {
        GetObjectW(
            bitmap.into(),
            std::mem::size_of::<DIBSECTION>() as i32,
            Some((&mut section as *mut DIBSECTION).cast()),
        )
    };
    if copied != std::mem::size_of::<DIBSECTION>() as i32
        || section.dsBm.bmBitsPixel != 32
        || section.dsBm.bmBits.is_null()
    {
        return None;
    }
    let (width, height, stride, length) = validate_bitmap(
        section.dsBm.bmWidth,
        section.dsBm.bmHeight,
        section.dsBm.bmWidthBytes,
    )?;
    let bytes = unsafe { std::slice::from_raw_parts(section.dsBm.bmBits.cast::<u8>(), length) };
    Some(DecodedImage {
        rgba: convert_bgra(
            bytes,
            width as u32,
            height as u32,
            stride,
            section.dsBm.bmHeight > 0,
        )?,
        width: width as u32,
        height: height as u32,
        resolution: Resolution::Preview,
    })
}

fn validate_bitmap(width: i32, height: i32, stride: i32) -> Option<(usize, usize, usize, usize)> {
    let width = usize::try_from(width).ok()?;
    let height = usize::try_from(height.checked_abs()?).ok()?;
    let stride = usize::try_from(stride.checked_abs()?).ok()?;
    if width == 0
        || height == 0
        || width > 4096
        || height > 4096
        || stride < width.checked_mul(4)?
    {
        return None;
    }
    let length = stride.checked_mul(height)?;
    (length <= 64 * 1024 * 1024).then_some((width, height, stride, length))
}

fn convert_bgra(
    bytes: &[u8],
    width: u32,
    height: u32,
    stride: usize,
    bottom_up: bool,
) -> Option<Vec<u8>> {
    let width = usize::try_from(width).ok()?;
    let height = usize::try_from(height).ok()?;
    let row_bytes = width.checked_mul(4)?;
    if width == 0
        || height == 0
        || stride < row_bytes
        || bytes.len() < stride.checked_mul(height)?
    {
        return None;
    }
    let mut rgba = vec![0; row_bytes.checked_mul(height)?];
    for output_row in 0..height {
        let input_row = if bottom_up {
            height - 1 - output_row
        } else {
            output_row
        };
        let source = &bytes[input_row * stride..input_row * stride + row_bytes];
        let target = &mut rgba[output_row * row_bytes..(output_row + 1) * row_bytes];
        let (source_pixels, []) = source.as_chunks::<4>() else {
            return None;
        };
        let (target_pixels, []) = target.as_chunks_mut::<4>() else {
            return None;
        };
        for (bgra, rgba) in source_pixels.iter().zip(target_pixels) {
            let alpha = bgra[3];
            rgba.copy_from_slice(&[bgra[2], bgra[1], bgra[0], alpha]);
        }
    }
    Some(rgba)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bottom_up_bgra_rows_become_top_down_rgba() {
        let bottom_then_top = [0, 255, 0, 255, 0, 0, 255, 255];

        assert_eq!(
            convert_bgra(&bottom_then_top, 1, 2, 4, true).unwrap(),
            vec![255, 0, 0, 255, 0, 255, 0, 255]
        );
    }

    #[test]
    fn impossible_bitmap_layouts_are_rejected() {
        assert!(validate_bitmap(0, 20, 80).is_none());
        assert!(validate_bitmap(20, 20, 4).is_none());
        assert!(convert_bgra(&[0; 4], 2, 2, 8, false).is_none());
    }
}
