//! ROLE: Sortypix AI only. The analysis progress on the application's taskbar button (Windows),
//! so it shows while the window is minimised. Nothing on other systems.
//! OWNER: `Ai` holds one `Taskbar`, made from the window handle at start and updated each frame
//! with the same progress as the toolbar line.
//! INVARIANTS: Main thread only (COM, apartment threaded). Never panics: when COM or the
//! taskbar is unavailable the progress is simply not shown, sorting is untouched. Calls the
//! shell only when the shown value changes (and retries a failed call at most once a second).

/// What the taskbar button shows: nothing, or `done` of `total` photos analysed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Shown {
    Nothing,
    Progress(usize, usize),
}

impl Shown {
    /// The progress line's value (`None`: analysis done, cached, off or not started).
    pub(crate) fn of(line: Option<(usize, usize)>) -> Shown {
        match line {
            Some((done, total)) if total > 0 => Shown::Progress(done.min(total), total),
            _ => Shown::Nothing,
        }
    }
}

#[cfg(windows)]
mod imp {
    use super::Shown;
    use std::time::{Duration, Instant};
    use windows::Win32::Foundation::HWND;
    use windows::Win32::System::Com::{
        CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx,
    };
    use windows::Win32::UI::Shell::{ITaskbarList3, TBPF_NOPROGRESS, TBPF_NORMAL, TaskbarList};

    /// Pause before calling the shell again after a failure (the button may not exist yet).
    const RETRY: Duration = Duration::from_secs(1);

    pub(crate) struct Taskbar {
        hwnd: Option<HWND>,
        list: Option<ITaskbarList3>,
        /// COM was tried and failed: never again.
        broken: bool,
        /// What the button shows now, as far as the shell said yes.
        shown: Shown,
        retry_at: Option<Instant>,
    }

    impl Taskbar {
        pub(crate) fn new(cc: &eframe::CreationContext<'_>) -> Taskbar {
            use raw_window_handle::{HasWindowHandle, RawWindowHandle};
            let hwnd = cc
                .window_handle()
                .ok()
                .and_then(|handle| match handle.as_raw() {
                    RawWindowHandle::Win32(h) => Some(HWND(h.hwnd.get() as *mut std::ffi::c_void)),
                    _ => None,
                });
            Taskbar {
                hwnd,
                list: None,
                broken: false,
                shown: Shown::Nothing,
                retry_at: None,
            }
        }

        /// The taskbar list, created on first use.
        fn list(&mut self) -> Option<&ITaskbarList3> {
            if self.list.is_none() && !self.broken {
                // SAFETY: plain COM calls on the interface thread; a failure only leaves the
                // taskbar without progress. winit has usually initialised COM already (then
                // this returns S_FALSE or RPC_E_CHANGED_MODE, both fine for the call below).
                let list = unsafe {
                    let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
                    CoCreateInstance::<_, ITaskbarList3>(&TaskbarList, None, CLSCTX_INPROC_SERVER)
                        .and_then(|list| list.HrInit().map(|()| list))
                };
                match list {
                    Ok(list) => self.list = Some(list),
                    Err(_) => self.broken = true,
                }
            }
            self.list.as_ref()
        }

        pub(crate) fn show(&mut self, wanted: Shown) {
            if wanted == self.shown || self.retry_at.is_some_and(|at| Instant::now() < at) {
                return;
            }
            let Some(hwnd) = self.hwnd else {
                return;
            };
            let Some(list) = self.list() else {
                return;
            };
            // SAFETY: `hwnd` is this application's window, alive as long as the interface.
            let done = unsafe {
                match wanted {
                    Shown::Nothing => list.SetProgressState(hwnd, TBPF_NOPROGRESS),
                    Shown::Progress(done, total) => list
                        .SetProgressState(hwnd, TBPF_NORMAL)
                        .and_then(|()| list.SetProgressValue(hwnd, done as u64, total as u64)),
                }
            };
            if done.is_ok() {
                self.shown = wanted;
                self.retry_at = None;
            } else {
                self.retry_at = Some(Instant::now() + RETRY);
            }
        }

        /// What the button shows (checked by the diagnostic).
        pub(crate) fn shown(&self) -> Shown {
            self.shown
        }
    }
}

#[cfg(not(windows))]
mod imp {
    use super::Shown;

    /// No taskbar progress outside Windows.
    pub(crate) struct Taskbar;

    impl Taskbar {
        pub(crate) fn new(_cc: &eframe::CreationContext<'_>) -> Taskbar {
            Taskbar
        }

        pub(crate) fn show(&mut self, _wanted: Shown) {}

        pub(crate) fn shown(&self) -> Shown {
            Shown::Nothing
        }
    }
}

pub(crate) use imp::Taskbar;

#[cfg(test)]
mod tests {
    use super::Shown;

    #[test]
    fn the_button_follows_the_progress_line() {
        assert_eq!(Shown::of(None), Shown::Nothing);
        assert_eq!(Shown::of(Some((3, 0))), Shown::Nothing);
        assert_eq!(Shown::of(Some((3, 10))), Shown::Progress(3, 10));
        assert_eq!(Shown::of(Some((12, 10))), Shown::Progress(10, 10));
    }
}
