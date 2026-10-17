//! Exercise the actual exported DLL and its process launch, without opening user photos.
// The shell extension exists only on Windows; elsewhere this probe does nothing.
#[cfg(not(windows))]
fn main() {}
#[cfg(windows)]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    probe::main()
}
#[cfg(windows)]
mod probe {
    use std::{ffi::c_void, os::windows::ffi::OsStrExt, path::PathBuf};
    use windows::{
        Win32::{
            System::{
                Com::{
                    COINIT_APARTMENTTHREADED, CoInitializeEx, CoTaskMemFree, CoUninitialize,
                    IClassFactory,
                },
                LibraryLoader::{GetProcAddress, LoadLibraryW},
            },
            UI::Shell::{
                IExplorerCommand, IShellItem, IShellItemArray, SHCreateItemFromParsingName,
                SHCreateShellItemArrayFromShellItem,
            },
        },
        core::{GUID, HRESULT, Interface, PCWSTR, s},
    };
    fn wide(p: &std::path::Path) -> Vec<u16> {
        p.as_os_str().encode_wide().chain(Some(0)).collect()
    }
    pub fn main() -> Result<(), Box<dyn std::error::Error>> {
        let args: Vec<_> = std::env::args_os().skip(1).map(PathBuf::from).collect();
        if args.len() != 3 {
            return Err("Expected DLL, folder and receipt paths".into());
        }
        // PITFALL: Export signatures are the documented COM DLL ABI. All objects are dropped
        // before COM shutdown; Windows releases the module on this short-lived process exit.
        unsafe {
            CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok()?;
            let module = LoadLibraryW(PCWSTR(wide(&args[0]).as_ptr()))?;
            type GetClass =
                unsafe extern "system" fn(*const GUID, *const GUID, *mut *mut c_void) -> HRESULT;
            let proc =
                GetProcAddress(module, s!("DllGetClassObject")).ok_or("Class export missing")?;
            let get_class: GetClass = std::mem::transmute(proc);
            let mut raw = std::ptr::null_mut();
            get_class(&sortypix_shell::COMMAND_ID, &IClassFactory::IID, &mut raw).ok()?;
            let factory = IClassFactory::from_raw(raw);
            let command: IExplorerCommand = factory.CreateInstance(None)?;
            let title = command.GetTitle(None)?;
            let title_string = title.to_string()?;
            CoTaskMemFree(Some(title.0.cast()));
            if title_string != "Open with Sortypix" {
                return Err("Unexpected menu label".into());
            }
            let item: IShellItem =
                SHCreateItemFromParsingName(PCWSTR(wide(&args[1]).as_ptr()), None)?;
            let items: IShellItemArray = SHCreateShellItemArrayFromShellItem(&item)?;
            if command.GetState(&items, false)? != 0 {
                return Err("Folder command is not enabled".into());
            }
            command.Invoke(&items, None)?;
            for _ in 0..100 {
                if args[2].exists() {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
            if std::fs::read(&args[2])? != args[1].as_os_str().as_encoded_bytes() {
                return Err("Wrong folder argument".into());
            }
            drop(items);
            drop(item);
            drop(command);
            drop(factory);
            let unload =
                GetProcAddress(module, s!("DllCanUnloadNow")).ok_or("Unload export missing")?;
            let unload: unsafe extern "system" fn() -> HRESULT = std::mem::transmute(unload);
            if unload() != windows::Win32::Foundation::S_OK {
                return Err("COM references still alive after release".into());
            }
            CoUninitialize();
        }
        println!(
            "Explorer DLL exports, COM factory, title, state, Unicode/spaced folder launch and lifetime: PASS"
        );
        Ok(())
    }
}
