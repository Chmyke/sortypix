//! ROLE: Asks the person for one folder with the system's own dialog. Blocking: run it
//! off the interface thread.
use std::path::PathBuf;
#[cfg(windows)]
pub fn choose_folder() -> Result<Option<PathBuf>, String> {
    use windows::Win32::{System::Com::*, UI::Shell::*};
    // PITFALL: Dialog COM interfaces are created and released on one dedicated STA thread.
    unsafe {
        CoInitializeEx(None, COINIT_APARTMENTTHREADED)
            .ok()
            .map_err(|e| e.to_string())?;
        let result = (|| -> windows::core::Result<Option<PathBuf>> {
            let dialog: IFileOpenDialog =
                CoCreateInstance(&FileOpenDialog, None, CLSCTX_INPROC_SERVER)?;
            dialog.SetOptions(FOS_PICKFOLDERS | FOS_FORCEFILESYSTEM | FOS_PATHMUSTEXIST)?;
            if dialog.Show(None).is_err() {
                return Ok(None);
            }
            let item = dialog.GetResult()?;
            let name = item.GetDisplayName(SIGDN_FILESYSPATH)?;
            let path = PathBuf::from(name.to_string()?);
            CoTaskMemFree(Some(name.0.cast()));
            Ok(Some(path))
        })();
        CoUninitialize();
        result.map_err(|e| e.to_string())
    }
}
/// Linux has no dialog library every desktop ships: GNOME and most others bring zenity,
/// KDE brings kdialog. `Ok(None)` is a cancel; an error says no dialog is available.
#[cfg(not(windows))]
pub fn choose_folder() -> Result<Option<PathBuf>, String> {
    use std::process::Command;
    let dialogs: [(&str, &[&str]); 2] = [
        (
            "zenity",
            &["--file-selection", "--directory", "--title=Open a folder"],
        ),
        (
            "kdialog",
            &["--getexistingdirectory", "--title", "Open a folder"],
        ),
    ];
    for (program, arguments) in dialogs {
        let Ok(output) = Command::new(program).args(arguments).output() else {
            continue;
        };
        if !output.status.success() {
            return Ok(None);
        }
        let mut path = output.stdout;
        while path.last() == Some(&b'\n') {
            path.pop();
        }
        if path.is_empty() {
            return Ok(None);
        }
        use std::os::unix::ffi::OsStringExt;
        return Ok(Some(PathBuf::from(std::ffi::OsString::from_vec(path))));
    }
    Err("No folder dialog found: install zenity, or drop a folder on the window.".into())
}
