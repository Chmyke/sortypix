//! Restore a normal window on a connected monitor, including negative desktop coordinates.
use crate::settings::Window;
use windows::Win32::{
    Foundation::POINT,
    Graphics::Gdi::{GetMonitorInfoW, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromPoint},
    UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI},
};

pub fn initialize_dpi() {
    // Must precede monitor queries, as winit normally enables DPI awareness later.
    use windows::Win32::UI::HiDpi::{
        DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, SetProcessDpiAwarenessContext,
    };
    let _ = unsafe { SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) };
}

struct Placement {
    position: [f32; 2],
    size: [f32; 2],
    scale: f32,
}

fn placement(saved: &Window) -> Option<Placement> {
    if !saved
        .position_pixels
        .iter()
        .chain(&saved.size)
        .all(|v| v.is_finite())
    {
        return None;
    }
    let monitor = unsafe {
        MonitorFromPoint(
            POINT {
                x: saved.position_pixels[0] as i32,
                y: saved.position_pixels[1] as i32,
            },
            MONITOR_DEFAULTTONEAREST,
        )
    };
    let mut info = MONITORINFO {
        cbSize: std::mem::size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };
    if !unsafe { GetMonitorInfoW(monitor, &mut info) }.as_bool() {
        return None;
    }
    let (mut x, mut y) = (96, 96);
    let _ = unsafe { GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut x, &mut y) };
    let scale = x.max(96) as f32 / 96.;
    let work = info.rcWork;
    let (position, size) = clamp(
        saved,
        [
            work.left as f32,
            work.top as f32,
            work.right as f32,
            work.bottom as f32,
        ],
        scale,
    );
    Some(Placement {
        position,
        size,
        scale,
    })
}
/// Place the created native window in physical desktop coordinates. Logical builder
/// coordinates would use the initial monitor's DPI, which may not be the saved monitor.
pub fn restore(handle: std::num::NonZeroIsize, saved: &Window) {
    use windows::Win32::{
        Foundation::{HWND, RECT},
        UI::{
            HiDpi::AdjustWindowRectExForDpi,
            WindowsAndMessaging::{
                GWL_EXSTYLE, GWL_STYLE, GetWindowLongPtrW, SW_MAXIMIZE, SWP_NOACTIVATE,
                SWP_NOZORDER, SetWindowPos, ShowWindow, WINDOW_EX_STYLE, WINDOW_STYLE,
            },
        },
    };
    let Some(p) = placement(saved) else {
        return;
    };
    let hwnd = HWND(handle.get() as *mut std::ffi::c_void);
    unsafe {
        let mut rect = RECT {
            left: 0,
            top: 0,
            right: (p.size[0] * p.scale).round() as i32,
            bottom: (p.size[1] * p.scale).round() as i32,
        };
        if AdjustWindowRectExForDpi(
            &mut rect,
            WINDOW_STYLE(GetWindowLongPtrW(hwnd, GWL_STYLE) as u32),
            false,
            WINDOW_EX_STYLE(GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32),
            (96. * p.scale).round() as u32,
        )
        .is_err()
        {
            return;
        }
        let _ = SetWindowPos(
            hwnd,
            None,
            p.position[0].round() as i32,
            p.position[1].round() as i32,
            rect.right - rect.left,
            rect.bottom - rect.top,
            SWP_NOZORDER | SWP_NOACTIVATE,
        );
        if saved.maximized {
            let _ = ShowWindow(hwnd, SW_MAXIMIZE);
        }
    }
}

fn clamp(saved: &Window, work: [f32; 4], scale: f32) -> ([f32; 2], [f32; 2]) {
    let size = [
        saved.size[0].max(820.).min((work[2] - work[0]) / scale),
        saved.size[1]
            .max(560.)
            .min(((work[3] - work[1]) / scale - 40.).max(100.)),
    ];
    let position = [
        saved.position_pixels[0].clamp(work[0], (work[2] - size[0] * scale).max(work[0])),
        saved.position_pixels[1].clamp(work[1], (work[3] - (size[1] + 40.) * scale).max(work[1])),
    ];
    (position, size)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn disconnected_monitor_is_recovered_and_negative_monitors_are_preserved() {
        let mut saved = Window {
            position_pixels: [-1800., 100.],
            size: [1000., 700.],
            maximized: false,
        };
        assert_eq!(clamp(&saved, [-1920., 0., 0., 1080.], 1.).0, [-1800., 100.]);
        assert_eq!(clamp(&saved, [0., 0., 1920., 1040.], 1.).0, [0., 100.]);
        saved.position_pixels = [2100., 150.];
        assert_eq!(
            clamp(&saved, [1920., 0., 4480., 1440.], 1.5).0,
            [2100., 150.]
        );
    }
}
