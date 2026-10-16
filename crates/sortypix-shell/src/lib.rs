//! ROLE: Tiny COM command for the modern Windows 11 folder context menu.
//! OWNER: Windows owns COM objects; no gallery, cache or photo processing in the shell.
//! INVARIANTS: Only one filesystem folder is accepted. No shell command interpolation.
#![allow(non_snake_case)] // WHY: COM interface and DLL export names are defined by Windows.
use std::{
    ffi::c_void,
    os::windows::ffi::{OsStrExt, OsStringExt},
    path::PathBuf,
    sync::atomic::{AtomicU32, Ordering},
};
use windows::{
    Win32::{
        Foundation::{
            CLASS_E_CLASSNOTAVAILABLE, CLASS_E_NOAGGREGATION, E_INVALIDARG, E_NOTIMPL, E_POINTER,
            HMODULE, S_FALSE, S_OK,
        },
        System::{
            Com::{CoTaskMemFree, IBindCtx, IClassFactory, IClassFactory_Impl},
            LibraryLoader::{
                GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS,
                GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT, GetModuleFileNameW,
                GetModuleHandleExW,
            },
        },
        UI::Shell::{
            ECF_DEFAULT, ECS_ENABLED, ECS_HIDDEN, IEnumExplorerCommand, IExplorerCommand,
            IExplorerCommand_Impl, IShellItemArray, SHStrDupW, SIGDN_FILESYSPATH,
        },
    },
    core::{
        BOOL, Error, GUID, HRESULT, IUnknown, Interface, PCWSTR, PWSTR, Ref, Result, implement,
    },
};

pub const COMMAND_ID: GUID = GUID::from_u128(0x46de7f78_c29e_4a6c_a1d7_aa3a6c960281);
// PITFALL: Process-wide DLL lifetime bookkeeping only, never application state.
static REFERENCES: AtomicU32 = AtomicU32::new(0);
struct Lifetime;
impl Lifetime {
    fn new() -> Self {
        REFERENCES.fetch_add(1, Ordering::Relaxed);
        Self
    }
}
impl Drop for Lifetime {
    fn drop(&mut self) {
        REFERENCES.fetch_sub(1, Ordering::Release);
    }
}

fn wide(value: &std::ffi::OsStr) -> Vec<u16> {
    value.encode_wide().chain(Some(0)).collect()
}
fn text(value: &str) -> Result<PWSTR> {
    let value = wide(std::ffi::OsStr::new(value));
    // PITFALL: SHStrDup allocates with the COM allocator; the shell owns the result.
    unsafe { SHStrDupW(PCWSTR(value.as_ptr())) }
}
fn executable() -> Result<PathBuf> {
    let mut module = HMODULE::default();
    let mut buffer = vec![0; 32768];
    // PITFALL: FROM_ADDRESS interprets this exported function address, not a string.
    unsafe {
        GetModuleHandleExW(
            GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS | GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
            PCWSTR(DllGetClassObject as *const () as *const u16),
            &mut module,
        )?;
        let count = GetModuleFileNameW(Some(module), &mut buffer) as usize;
        if count == 0 || count >= buffer.len() {
            return Err(Error::from_thread());
        }
        let module_path = PathBuf::from(std::ffi::OsString::from_wide(&buffer[..count]));
        Ok(module_path
            .parent()
            .ok_or_else(|| Error::from_hresult(E_INVALIDARG))?
            .join("sortypix.exe"))
    }
}
fn folder(items: &IShellItemArray) -> Result<PathBuf> {
    // PITFALL: GetDisplayName returns COM-allocated memory, freed even on invalid paths.
    unsafe {
        if items.GetCount()? != 1 {
            return Err(Error::from_hresult(E_INVALIDARG));
        }
        let item = items.GetItemAt(0)?;
        let name = item.GetDisplayName(SIGDN_FILESYSPATH)?;
        let path = PathBuf::from(std::ffi::OsString::from_wide(name.as_wide()));
        CoTaskMemFree(Some(name.0.cast()));
        if !path.is_absolute() || !path.is_dir() {
            return Err(Error::from_hresult(E_INVALIDARG));
        }
        Ok(path)
    }
}
#[implement(IExplorerCommand)]
struct OpenCommand {
    _lifetime: Lifetime,
}
impl IExplorerCommand_Impl for OpenCommand_Impl {
    fn GetTitle(&self, _: Ref<IShellItemArray>) -> Result<PWSTR> {
        text("Open with Sortypix")
    }
    fn GetIcon(&self, _: Ref<IShellItemArray>) -> Result<PWSTR> {
        let exe = executable()?;
        text(&format!("{},0", exe.display()))
    }
    fn GetToolTip(&self, _: Ref<IShellItemArray>) -> Result<PWSTR> {
        text("Open this photo folder in Sortypix")
    }
    fn GetCanonicalName(&self) -> Result<GUID> {
        Ok(COMMAND_ID)
    }
    fn GetState(&self, items: Ref<IShellItemArray>, _: BOOL) -> Result<u32> {
        // Directory-only registration supplies the type. Never access the disk here.
        let enabled = items
            .as_ref()
            .is_some_and(|items| unsafe { items.GetCount().ok() == Some(1) });
        Ok(if enabled { ECS_ENABLED.0 } else { ECS_HIDDEN.0 } as u32)
    }
    fn Invoke(&self, items: Ref<IShellItemArray>, _: Ref<IBindCtx>) -> Result<()> {
        let directory = folder(items.ok()?)?;
        std::process::Command::new(executable()?)
            .arg(directory)
            .spawn()
            .map_err(|e| Error::new(windows::Win32::Foundation::E_FAIL, e.to_string()))?;
        Ok(())
    }
    fn GetFlags(&self) -> Result<u32> {
        Ok(ECF_DEFAULT.0 as u32)
    }
    fn EnumSubCommands(&self) -> Result<IEnumExplorerCommand> {
        Err(Error::from_hresult(E_NOTIMPL))
    }
}
#[implement(IClassFactory)]
struct Factory {
    _lifetime: Lifetime,
}
impl IClassFactory_Impl for Factory_Impl {
    fn CreateInstance(
        &self,
        outer: Ref<IUnknown>,
        iid: *const GUID,
        object: *mut *mut c_void,
    ) -> Result<()> {
        if iid.is_null() || object.is_null() {
            return Err(Error::from_hresult(E_POINTER));
        }
        // PITFALL: COM calls provide valid output storage; never leave an old pointer on failure.
        unsafe {
            *object = std::ptr::null_mut();
        }
        if !outer.is_null() {
            return Err(Error::from_hresult(CLASS_E_NOAGGREGATION));
        }
        let command: IExplorerCommand = OpenCommand {
            _lifetime: Lifetime::new(),
        }
        .into();
        unsafe { command.query(iid, object).ok() }
    }
    fn LockServer(&self, lock: BOOL) -> Result<()> {
        if lock.as_bool() {
            REFERENCES.fetch_add(1, Ordering::Relaxed);
        } else {
            let _ =
                REFERENCES.fetch_update(Ordering::Release, Ordering::Relaxed, |n| n.checked_sub(1));
        }
        Ok(())
    }
}
/// # Safety
/// Standard COM ABI: GUID pointers and the interface output must point to valid storage.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn DllGetClassObject(
    clsid: *const GUID,
    iid: *const GUID,
    object: *mut *mut c_void,
) -> HRESULT {
    if clsid.is_null() || iid.is_null() || object.is_null() {
        return E_POINTER;
    }
    // PITFALL: Only the documented COM ABI crosses this raw-pointer boundary.
    unsafe {
        *object = std::ptr::null_mut();
        if *clsid != COMMAND_ID {
            return CLASS_E_CLASSNOTAVAILABLE;
        }
        let factory: IClassFactory = Factory {
            _lifetime: Lifetime::new(),
        }
        .into();
        factory.query(iid, object)
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn DllCanUnloadNow() -> HRESULT {
    if REFERENCES.load(Ordering::Acquire) == 0 {
        S_OK
    } else {
        S_FALSE
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn com_factory_title_lifetime_and_invalid_requests() {
        unsafe {
            let mut raw = std::ptr::null_mut();
            assert_eq!(
                DllGetClassObject(&GUID::zeroed(), &IClassFactory::IID, &mut raw),
                CLASS_E_CLASSNOTAVAILABLE
            );
            assert!(raw.is_null());
            assert_eq!(
                DllGetClassObject(&COMMAND_ID, &IClassFactory::IID, &mut raw),
                S_OK
            );
            let factory = IClassFactory::from_raw(raw);
            assert_eq!(DllCanUnloadNow(), S_FALSE);
            let command: IExplorerCommand = factory.CreateInstance(None).unwrap();
            let title = command.GetTitle(None).unwrap();
            assert_eq!(title.to_string().unwrap(), "Open with Sortypix");
            CoTaskMemFree(Some(title.0.cast()));
            assert_eq!(command.GetState(None, false).unwrap(), ECS_HIDDEN.0 as u32);
            assert!(command.Invoke(None, None).is_err());
            assert_eq!(command.GetCanonicalName().unwrap(), COMMAND_ID);
            drop(command);
            drop(factory);
            assert_eq!(DllCanUnloadNow(), S_OK);
        }
    }
    #[test]
    fn unicode_folder_is_resolved_without_interpolating_a_command() {
        use windows::Win32::{
            System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx, CoUninitialize},
            UI::Shell::{
                IShellItem, SHCreateItemFromParsingName, SHCreateShellItemArrayFromShellItem,
            },
        };
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("photos espace é & (test)");
        std::fs::create_dir(&path).unwrap();
        let encoded = wide(path.as_os_str());
        unsafe {
            CoInitializeEx(None, COINIT_APARTMENTTHREADED).unwrap();
            let item: IShellItem =
                SHCreateItemFromParsingName(PCWSTR(encoded.as_ptr()), None).unwrap();
            let items: IShellItemArray = SHCreateShellItemArrayFromShellItem(&item).unwrap();
            assert_eq!(folder(&items).unwrap(), path);
            std::fs::remove_dir(&path).unwrap();
            assert!(folder(&items).is_err());
            drop(items);
            drop(item);
            CoUninitialize();
        }
    }
}
