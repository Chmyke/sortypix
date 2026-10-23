//! Where the AI files live on disk.

use std::path::PathBuf;

/// Folder holding the runtime library and the model files.
///
/// `$SORTYPIX_AI_DIR/models` when the variable is set (it names the ai-dev root, which contains
/// `models/` and `corpus/`), else `ai-dev/models` under the platform cache root. The root mirrors
/// `sortypix_app::settings::cache_dir()` on purpose without importing it, to keep the crates
/// unlinked.
pub fn models_dir() -> PathBuf {
    match std::env::var_os("SORTYPIX_AI_DIR") {
        Some(root) if !root.is_empty() => PathBuf::from(root).join("models"),
        _ => cache_root().join("ai-dev").join("models"),
    }
}

fn cache_root() -> PathBuf {
    if cfg!(windows) {
        std::env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(std::env::temp_dir)
            .join("Sortypix")
    } else {
        // A relative value is ignored, as the XDG specification asks.
        let absolute = |p: PathBuf| p.is_absolute().then_some(p);
        std::env::var_os("XDG_CACHE_HOME")
            .map(PathBuf::from)
            .and_then(absolute)
            .or_else(|| {
                std::env::var_os("HOME")
                    .map(PathBuf::from)
                    .and_then(absolute)
                    .map(|home| home.join(".cache"))
            })
            .unwrap_or_else(std::env::temp_dir)
            .join("sortypix")
    }
}
