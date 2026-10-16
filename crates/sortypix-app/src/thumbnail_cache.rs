//! Disposable, bounded local previews. Originals are only ever read.
use crate::decode::{DecodedImage, Resolution, decode};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::Mutex,
    time::SystemTime,
};

#[derive(Default)]
struct CacheIndex {
    entries: BTreeMap<PathBuf, (SystemTime, u64)>,
    total: u64,
}

pub struct ThumbnailCache {
    root: PathBuf,
    budget: u64,
    index: Mutex<Option<CacheIndex>>,
    #[cfg(test)]
    scans: std::sync::atomic::AtomicUsize,
}
impl ThumbnailCache {
    pub fn new(root: PathBuf, budget: u64) -> Self {
        Self {
            root,
            budget,
            index: Mutex::new(None),
            #[cfg(test)]
            scans: std::sync::atomic::AtomicUsize::new(0),
        }
    }
    fn key(&self, source: &Path, edge: u32) -> Option<PathBuf> {
        let path = source.canonicalize().ok()?;
        let meta = fs::metadata(&path).ok()?;
        let modified = meta
            .modified()
            .ok()?
            .duration_since(std::time::UNIX_EPOCH)
            .ok()?;
        let mut hash = Sha256::new();
        hash.update(path.as_os_str().as_encoded_bytes());
        hash.update(meta.len().to_le_bytes());
        hash.update(modified.as_nanos().to_le_bytes());
        hash.update(edge.to_le_bytes());
        Some(self.root.join(format!("{:x}.rgba", hash.finalize())))
    }
    fn read(&self, path: &Path, edge: u32) -> Option<DecodedImage> {
        let mut file = fs::File::open(path).ok()?;
        let mut header = [0; 8];
        file.read_exact(&mut header).ok()?;
        let width = u32::from_le_bytes(header[..4].try_into().ok()?);
        let height = u32::from_le_bytes(header[4..].try_into().ok()?);
        if width == 0 || height == 0 || width > edge || height > edge {
            return None;
        }
        let length = u64::from(width) * u64::from(height) * 4;
        if length > 64 * 1024 * 1024 || file.metadata().ok()?.len() != length + 8 {
            return None;
        }
        let mut rgba = vec![0; length as usize];
        file.read_exact(&mut rgba).ok()?;
        Some(DecodedImage {
            width,
            height,
            rgba,
            resolution: Resolution::Preview,
        })
    }
    pub fn decode(&self, source: &Path, edge: u32) -> Result<DecodedImage, String> {
        if edge == 0 {
            return decode(source, None);
        }
        let key = self.key(source, edge);
        if let Some(image) = key.as_ref().and_then(|key| self.read(key, edge)) {
            return Ok(image);
        }
        let image = decode(source, Some(edge))?;
        if let Some(key) = key
            && self.key(source, edge).as_ref() == Some(&key)
        {
            let _ = self.write(&key, &image);
        }
        Ok(image)
    }
    fn write(&self, path: &Path, image: &DecodedImage) -> Option<()> {
        let size = image.rgba.len() as u64 + 8;
        if size > self.budget {
            return None;
        }
        let mut index = self.index.lock().ok()?;
        fs::create_dir_all(&self.root).ok()?;
        if index.is_none() {
            *index = Some(self.scan_index()?);
        }
        let index = index.as_mut()?;
        if let Some((_, previous)) = index.entries.get(path).copied() {
            if fs::remove_file(path).is_err() && path.exists() {
                return None;
            }
            index.entries.remove(path);
            index.total = index.total.saturating_sub(previous);
        }
        while index.total + size > self.budget {
            let (old, bytes) = index
                .entries
                .iter()
                .min_by_key(|(_, (modified, _))| *modified)
                .map(|(path, (_, bytes))| (path.clone(), *bytes))?;
            if fs::remove_file(&old).is_err() && old.exists() {
                return None;
            }
            index.entries.remove(&old);
            index.total = index.total.saturating_sub(bytes);
        }
        let temp = path.with_extension(format!("{}.tmp", std::process::id()));
        let result = (|| -> std::io::Result<()> {
            let mut file = fs::File::create(&temp)?;
            file.write_all(&image.width.to_le_bytes())?;
            file.write_all(&image.height.to_le_bytes())?;
            file.write_all(&image.rgba)?;
            drop(file);
            fs::rename(&temp, path)
        })();
        if result.is_err() {
            let _ = fs::remove_file(temp);
        } else {
            let modified = path
                .metadata()
                .and_then(|metadata| metadata.modified())
                .unwrap_or_else(|_| SystemTime::now());
            index.entries.insert(path.to_path_buf(), (modified, size));
            index.total += size;
        }
        result.ok()
    }

    fn scan_index(&self) -> Option<CacheIndex> {
        #[cfg(test)]
        self.scans.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let mut index = CacheIndex::default();
        // Only our exact cache filenames participate. Never recurse or remove foreign files.
        for entry in fs::read_dir(&self.root).ok()?.flatten() {
            let name = entry.file_name();
            let Some(name) = name.to_str() else {
                continue;
            };
            if name.len() != 69
                || !name.ends_with(".rgba")
                || !name[..64].bytes().all(|byte| byte.is_ascii_hexdigit())
            {
                continue;
            }
            let Ok(metadata) = entry.metadata() else {
                continue;
            };
            if !metadata.is_file() {
                continue;
            }
            let size = metadata.len();
            index.total += size;
            index.entries.insert(
                entry.path(),
                (metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH), size),
            );
        }
        Some(index)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn previews_survive_restart_invalidate_and_recover_from_corruption() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("photo.png");
        let root = dir.path().join("cache");
        image::RgbaImage::from_pixel(20, 10, image::Rgba([1, 2, 3, 255]))
            .save(&source)
            .unwrap();
        let cache = ThumbnailCache::new(root.clone(), 4096);
        let first = cache.decode(&source, 10).unwrap();
        let key = cache.key(&source, 10).unwrap();
        assert!(key.is_file());
        let restarted = ThumbnailCache::new(root, 4096);
        assert_eq!(restarted.read(&key, 10).unwrap().rgba, first.rgba);
        fs::write(&key, b"broken").unwrap();
        assert_eq!(restarted.decode(&source, 10).unwrap().rgba, first.rgba);
        image::RgbaImage::from_pixel(30, 20, image::Rgba([7, 8, 9, 255]))
            .save(&source)
            .unwrap();
        assert_ne!(cache.key(&source, 10).unwrap(), key);
        assert_eq!(
            &cache.decode(&source, 10).unwrap().rgba[..4],
            &[7, 8, 9, 255]
        );
        assert!(cache.decode(&source, 0).unwrap().resolution == Resolution::Native);
    }
    #[test]
    fn budget_and_unavailable_cache_do_not_endanger_originals() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("cache");
        fs::create_dir(&root).unwrap();
        fs::write(root.join("keep.txt"), b"foreign").unwrap();
        let cache = ThumbnailCache::new(root.clone(), 900);
        for n in 0..5 {
            let source = dir.path().join(format!("{n}.png"));
            image::RgbaImage::new(10, 10).save(&source).unwrap();
            let before = fs::read(&source).unwrap();
            cache.decode(&source, 10).unwrap();
            assert_eq!(fs::read(&source).unwrap(), before);
        }
        assert_eq!(fs::read(root.join("keep.txt")).unwrap(), b"foreign");
        let bytes: u64 = fs::read_dir(&root)
            .unwrap()
            .flatten()
            .filter(|e| e.path().extension().is_some_and(|s| s == "rgba"))
            .map(|e| e.metadata().unwrap().len())
            .sum();
        assert!(bytes <= 900);
        let unavailable = ThumbnailCache::new(root.join("keep.txt"), 900);
        assert!(unavailable.decode(&dir.path().join("0.png"), 10).is_ok());
    }
    #[test]
    fn repeated_writes_scan_the_cache_directory_once() {
        let dir = tempfile::tempdir().unwrap();
        let cache = ThumbnailCache::new(dir.path().join("cache"), 4096);
        for name in ["first.png", "second.png"] {
            let source = dir.path().join(name);
            image::RgbaImage::new(10, 10).save(&source).unwrap();
            cache.decode(&source, 10).unwrap();
        }

        assert_eq!(cache.scans.load(std::sync::atomic::Ordering::SeqCst), 1);
    }
}
