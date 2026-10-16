use std::path::PathBuf;
use windows::Win32::{System::Com::*, UI::Shell::*};
pub fn choose_folder() -> Result<Option<PathBuf>, String> {
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
