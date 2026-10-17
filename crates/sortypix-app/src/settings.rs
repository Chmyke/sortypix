pub const MIN_THUMBNAIL_SIZE: f32 = 100.;
pub const MAX_THUMBNAIL_SIZE: f32 = 2400.;
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};
#[derive(Clone, Default, serde::Serialize, serde::Deserialize, Debug, PartialEq, Eq)]
#[serde(default)]
pub struct Shortcut {
    pub key: String,
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
}
#[derive(serde::Serialize, serde::Deserialize, Debug, PartialEq)]
#[serde(default)]
pub struct Settings {
    pub last_folder: Option<PathBuf>,
    pub resume: Option<Resume>,
    pub window: Option<Window>,
    pub thumbnail_size: f32,
    pub inverted: bool,
    pub newest_first: bool,
    pub shortcuts: BTreeMap<String, Shortcut>,
}
#[derive(Clone, serde::Serialize, serde::Deserialize, Debug, PartialEq)]
pub struct Resume {
    pub folder: PathBuf,
    pub photo: PathBuf,
    pub index: usize,
    pub fraction: f32,
}
impl Resume {
    pub fn locate(&self, folder: &Path, photos: &[crate::catalog::PhysicalPhoto]) -> Option<usize> {
        if folder != self.folder || photos.is_empty() {
            return None;
        }
        Some(
            photos
                .iter()
                .position(|p| p.members.contains(&self.photo))
                .unwrap_or(self.index.min(photos.len() - 1)),
        )
    }
}
#[derive(Clone, serde::Serialize, serde::Deserialize, Debug, PartialEq)]
pub struct Window {
    pub position_pixels: [f32; 2],
    pub size: [f32; 2],
    pub maximized: bool,
}
/// The person's settings: `%LOCALAPPDATA%/Sortypix` on Windows, the XDG configuration
/// folder elsewhere.
pub fn settings_path() -> PathBuf {
    if cfg!(windows) {
        local_app_data().join("settings.json")
    } else {
        xdg_home("XDG_CONFIG_HOME", ".config").join("settings.json")
    }
}
/// Disposable previews: beside the settings on Windows, in the XDG cache elsewhere, which
/// backups leave out.
pub fn cache_dir() -> PathBuf {
    if cfg!(windows) {
        local_app_data().join("cache")
    } else {
        xdg_home("XDG_CACHE_HOME", ".cache")
    }
}
fn local_app_data() -> PathBuf {
    std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join("Sortypix")
}
/// `$variable/sortypix`, else `~/fallback/sortypix`. A relative value is ignored, as the
/// XDG specification asks.
fn xdg_home(variable: &str, fallback: &str) -> PathBuf {
    let absolute = |p: PathBuf| p.is_absolute().then_some(p);
    std::env::var_os(variable)
        .map(PathBuf::from)
        .and_then(absolute)
        .or_else(|| {
            std::env::var_os("HOME")
                .map(PathBuf::from)
                .and_then(absolute)
                .map(|home| home.join(fallback))
        })
        .unwrap_or_else(std::env::temp_dir)
        .join("sortypix")
}
impl Settings {
    pub fn load(path: &Path) -> Self {
        let mut settings: Self = std::fs::read(path)
            .ok()
            .and_then(|s| serde_json::from_slice(&s).ok())
            .unwrap_or_default();
        if !settings.thumbnail_size.is_finite() {
            settings.thumbnail_size = 190.0;
        }
        settings.thumbnail_size = settings
            .thumbnail_size
            .clamp(MIN_THUMBNAIL_SIZE, MAX_THUMBNAIL_SIZE);
        settings
    }
    pub fn save(&self, path: &Path) -> Result<(), String> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let temp = path.with_extension(format!("{}.tmp", std::process::id()));
        std::fs::write(
            &temp,
            serde_json::to_vec_pretty(self).map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
        // PITFALL: Only settings are replaced, never photographs.
        let result = replace(&temp, path);
        if result.is_err() {
            let _ = std::fs::remove_file(temp);
        }
        result
    }
}
#[cfg(windows)]
fn replace(from: &Path, to: &Path) -> Result<(), String> {
    use std::os::windows::ffi::OsStrExt;
    use windows::{
        Win32::Storage::FileSystem::{
            MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW,
        },
        core::PCWSTR,
    };
    let from: Vec<u16> = from.as_os_str().encode_wide().chain(Some(0)).collect();
    let to: Vec<u16> = to.as_os_str().encode_wide().chain(Some(0)).collect();
    unsafe {
        MoveFileExW(
            PCWSTR(from.as_ptr()),
            PCWSTR(to.as_ptr()),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    }
    .map_err(|e| e.to_string())
}
/// POSIX rename replaces atomically; the data is flushed first so a crash leaves the old
/// file or the new one, never an empty one.
#[cfg(not(windows))]
fn replace(from: &Path, to: &Path) -> Result<(), String> {
    std::fs::File::open(from)
        .and_then(|f| f.sync_all())
        .and_then(|()| std::fs::rename(from, to))
        .map_err(|e| e.to_string())
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            last_folder: None,
            resume: None,
            window: None,
            thumbnail_size: 190.0,
            inverted: false,
            newest_first: false,
            shortcuts: BTreeMap::new(),
        }
    }
}
