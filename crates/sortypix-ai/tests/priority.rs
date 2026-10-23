//! The analysis runs below the interface. A test binary of its own: it lowers its whole process.

#[cfg(windows)]
#[test]
fn existing_threads_stay_normal_and_new_ones_run_below() {
    use windows::Win32::System::Threading::{
        BELOW_NORMAL_PRIORITY_CLASS, GetCurrentProcess, GetCurrentThread, GetPriorityClass,
        GetThreadPriority, THREAD_PRIORITY_HIGHEST, THREAD_PRIORITY_NORMAL,
    };
    let (ready, started) = std::sync::mpsc::channel();
    let (finish, wait) = std::sync::mpsc::channel::<()>();
    // A thread that already exists, like the preview and file threads of the interface.
    let existing = std::thread::spawn(move || {
        ready.send(()).unwrap();
        wait.recv().unwrap();
        // SAFETY: reads the priority of the calling thread.
        unsafe { GetThreadPriority(GetCurrentThread()) }
    });
    started.recv().unwrap();
    sortypix_ai::priority::favour_interface();
    // SAFETY: reads the priority class of this process and of the calling thread.
    unsafe {
        assert_eq!(
            GetPriorityClass(GetCurrentProcess()),
            BELOW_NORMAL_PRIORITY_CLASS.0
        );
        assert_eq!(
            GetThreadPriority(GetCurrentThread()),
            THREAD_PRIORITY_HIGHEST.0
        );
    }
    finish.send(()).unwrap();
    assert_eq!(existing.join().unwrap(), THREAD_PRIORITY_HIGHEST.0);
    // A thread created afterwards (the analysis, ONNX Runtime) keeps the lowered class.
    let later = std::thread::spawn(|| {
        // SAFETY: reads the priority of the calling thread.
        unsafe { GetThreadPriority(GetCurrentThread()) }
    });
    assert_eq!(later.join().unwrap(), THREAD_PRIORITY_NORMAL.0);
}
