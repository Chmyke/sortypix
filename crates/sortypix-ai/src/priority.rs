//! Keeps the analysis behind the interface: sorting must never wait for the model.
//!
//! ONNX Runtime creates its own threads, whose priority cannot be set from here. On Windows the
//! whole process is lowered instead (`BELOW_NORMAL_PRIORITY_CLASS`) and the threads that serve
//! the person are raised back to the normal level (`THREAD_PRIORITY_HIGHEST` in that class);
//! every thread created afterwards, ONNX Runtime's included, stays below. On Linux priority
//! (`nice`) is per thread and inherited, so the analysis thread lowers itself before it
//! creates the model and its workers.

/// Windows: raises every thread the process has now to the normal level, then lowers the
/// process class, so only threads created later run below normal. Call it once, after the
/// interface, preview and file threads exist and before [`crate::service::Analyzer::start`].
/// Elsewhere: nothing. A refused call leaves the priorities as they were.
pub fn favour_interface() {
    #[cfg(windows)]
    windows_priority::favour_existing_threads();
}

/// Linux: lowers the calling thread by 10 `nice` steps; threads it creates inherit it.
/// Elsewhere nothing (the Windows process class already covers new threads).
pub(crate) fn lower_current_thread() {
    #[cfg(target_os = "linux")]
    // SAFETY: `nice` only changes the scheduling priority of the calling thread on Linux.
    unsafe {
        libc::nice(10);
    }
}

#[cfg(windows)]
mod windows_priority {
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, TH32CS_SNAPTHREAD, THREADENTRY32, Thread32First, Thread32Next,
    };
    use windows::Win32::System::Threading::{
        BELOW_NORMAL_PRIORITY_CLASS, GetCurrentProcess, GetCurrentProcessId, OpenThread,
        SetPriorityClass, SetThreadPriority, THREAD_PRIORITY_HIGHEST, THREAD_SET_INFORMATION,
    };

    pub(super) fn favour_existing_threads() {
        // SAFETY: plain Win32 calls on this process and its own threads; every handle opened
        // here is closed here, and a thread that exits meanwhile only makes a call fail.
        unsafe {
            let process = GetCurrentProcessId();
            if let Ok(snapshot) = CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0) {
                let mut entry = THREADENTRY32 {
                    dwSize: std::mem::size_of::<THREADENTRY32>() as u32,
                    ..Default::default()
                };
                let mut more = Thread32First(snapshot, &mut entry).is_ok();
                while more {
                    if entry.th32OwnerProcessID == process
                        && let Ok(thread) =
                            OpenThread(THREAD_SET_INFORMATION, false, entry.th32ThreadID)
                    {
                        let _ = SetThreadPriority(thread, THREAD_PRIORITY_HIGHEST);
                        let _ = CloseHandle(thread);
                    }
                    more = Thread32Next(snapshot, &mut entry).is_ok();
                }
                let _ = CloseHandle(snapshot);
            }
            let _ = SetPriorityClass(GetCurrentProcess(), BELOW_NORMAL_PRIORITY_CLASS);
        }
    }
}
