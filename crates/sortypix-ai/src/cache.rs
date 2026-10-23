//! Embeddings and aesthetic scores kept on disk, so a folder is analysed once.
//!
//! One fixed-size record per photo, `<root>/<64 hex>.emb`. The key is the content of the file
//! as the system sees it (name, size, modification time) and the model label, never the
//! folder: a photo moved into `keep/` or `rejected/` keeps its record (`MoveFileW` and `rename`
//! keep the size and the time). Records are written to a temporary file then renamed; a record
//! of the wrong length, magic or version is ignored and recomputed by the caller.

use crate::engine::{EMBED_DIM, Embedding};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs,
    io::{self, Read, Write},
    path::{Path, PathBuf},
    sync::Mutex,
    time::SystemTime,
};

/// Label of the shipped model: CLIP ViT-L/14, int8 visual model. Part of the key and of the
/// folder, so another model never reads these records.
pub const MODEL_LABEL: &str = "clip-l14-int8";

/// Default disk budget: about 3 KB per photo, so 1 GiB holds some 340,000 photos.
pub const DEFAULT_BUDGET: u64 = 1024 * 1024 * 1024;

const MAGIC: [u8; 4] = *b"SPXE";
const VERSION: u32 = 1;
/// Magic, version, 768 floats, then the aesthetic score.
const RECORD_LEN: usize = 8 + EMBED_DIM * 4 + 4;
const EXTENSION: &str = "emb";

/// Records folder for `model` under the application cache folder: `<cache_dir>/ai-v1/<model>`.
pub fn cache_root(cache_dir: &Path, model: &str) -> PathBuf {
    cache_dir.join("ai-v1").join(model)
}

/// Content key of one photo for one model (SHA-256, see the module documentation).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ContentKey([u8; 32]);

impl ContentKey {
    /// Key of `path` as it is now on disk; `None` when the file cannot be read (gone, refused).
    pub fn of(path: &Path, model: &str) -> Option<ContentKey> {
        let name = path.file_name()?;
        let meta = fs::metadata(path).ok()?;
        if !meta.is_file() {
            return None;
        }
        let modified = meta
            .modified()
            .ok()?
            .duration_since(std::time::UNIX_EPOCH)
            .ok()?;
        let mut hash = Sha256::new();
        // Lengths first, so no two different inputs hash the same bytes.
        let name = name.as_encoded_bytes();
        hash.update((name.len() as u64).to_le_bytes());
        hash.update(name);
        hash.update(meta.len().to_le_bytes());
        hash.update(modified.as_nanos().to_le_bytes());
        hash.update((model.len() as u64).to_le_bytes());
        hash.update(model.as_bytes());
        Some(ContentKey(hash.finalize().into()))
    }

    fn file_name(&self) -> String {
        let mut name = String::with_capacity(64 + 1 + EXTENSION.len());
        for byte in self.0 {
            name.push_str(&format!("{byte:02x}"));
        }
        name.push('.');
        name.push_str(EXTENSION);
        name
    }
}

/// What the analysis keeps of one photo.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Record {
    pub embedding: Embedding,
    pub score: f32,
}

impl Record {
    fn to_bytes(self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(RECORD_LEN);
        bytes.extend_from_slice(&MAGIC);
        bytes.extend_from_slice(&VERSION.to_le_bytes());
        for v in self.embedding.iter().chain(Some(&self.score)) {
            bytes.extend_from_slice(&v.to_le_bytes());
        }
        bytes
    }

    fn from_bytes(bytes: &[u8]) -> Option<Record> {
        if bytes.len() != RECORD_LEN || bytes[..4] != MAGIC || bytes[4..8] != VERSION.to_le_bytes()
        {
            return None;
        }
        let mut values = bytes[8..]
            .as_chunks::<4>()
            .0
            .iter()
            .map(|c| f32::from_le_bytes(*c));
        let mut embedding = [0f32; EMBED_DIM];
        for v in &mut embedding {
            *v = values.next()?;
        }
        let score = values.next()?;
        if embedding.iter().chain(Some(&score)).any(|v| !v.is_finite()) {
            return None;
        }
        Some(Record { embedding, score })
    }
}

#[derive(Default)]
struct Index {
    entries: BTreeMap<PathBuf, (SystemTime, u64)>,
    total: u64,
}

/// Bounded folder of records; the oldest written go first once the budget is reached.
pub struct EmbeddingCache {
    root: PathBuf,
    budget: u64,
    /// Scanned on the first write only: reads never list the folder.
    index: Mutex<Option<Index>>,
}

impl EmbeddingCache {
    pub fn new(root: PathBuf, budget: u64) -> Self {
        Self {
            root,
            budget,
            index: Mutex::new(None),
        }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// The record of `key`, if one was written and reads back whole.
    pub fn get(&self, key: &ContentKey) -> Option<Record> {
        let file = fs::File::open(self.root.join(key.file_name())).ok()?;
        let mut bytes = Vec::with_capacity(RECORD_LEN + 1);
        // One byte more than a record: a longer file is refused, not truncated.
        file.take(RECORD_LEN as u64 + 1)
            .read_to_end(&mut bytes)
            .ok()?;
        Record::from_bytes(&bytes)
    }

    /// Writes the record of `key`, evicting the oldest records beyond the budget. An error
    /// (folder not writable, disk full) leaves the cache as it was and is only reported.
    pub fn put(&self, key: &ContentKey, record: &Record) -> io::Result<()> {
        let size = RECORD_LEN as u64;
        if size > self.budget {
            return Err(io::Error::other("cache budget below one record"));
        }
        let mut index = self.index.lock().unwrap_or_else(|e| e.into_inner());
        fs::create_dir_all(&self.root)?;
        if index.is_none() {
            *index = Some(self.scan()?);
        }
        let Some(index) = index.as_mut() else {
            return Err(io::Error::other("cache index unavailable"));
        };
        let path = self.root.join(key.file_name());
        if let Some((_, previous)) = index.entries.remove(&path) {
            index.total = index.total.saturating_sub(previous);
        }
        if index.total + size > self.budget {
            self.evict(index, self.budget - size);
        }
        let temp = path.with_extension(format!("{}.tmp", std::process::id()));
        let result = (|| -> io::Result<()> {
            let mut file = fs::File::create(&temp)?;
            file.write_all(&record.to_bytes())?;
            drop(file);
            fs::rename(&temp, &path)
        })();
        match result {
            Ok(()) => {
                let modified = path
                    .metadata()
                    .and_then(|m| m.modified())
                    .unwrap_or_else(|_| SystemTime::now());
                index.entries.insert(path, (modified, size));
                index.total += size;
                Ok(())
            }
            Err(e) => {
                let _ = fs::remove_file(temp);
                Err(e)
            }
        }
    }

    /// Removes the oldest records until the total is at most `target` minus a tenth of the
    /// budget: one sort per eviction round rather than one scan per write.
    fn evict(&self, index: &mut Index, target: u64) {
        let target = target.saturating_sub(self.budget / 10);
        let mut by_age: Vec<(SystemTime, PathBuf, u64)> = index
            .entries
            .iter()
            .map(|(path, (modified, bytes))| (*modified, path.clone(), *bytes))
            .collect();
        by_age.sort();
        for (_, path, bytes) in by_age {
            if index.total <= target {
                break;
            }
            if fs::remove_file(&path).is_err() && path.exists() {
                continue;
            }
            index.entries.remove(&path);
            index.total = index.total.saturating_sub(bytes);
        }
    }

    fn scan(&self) -> io::Result<Index> {
        let mut index = Index::default();
        // Only our exact file names count. Never recurse or remove foreign files.
        for entry in fs::read_dir(&self.root)?.flatten() {
            let name = entry.file_name();
            let Some(name) = name.to_str() else {
                continue;
            };
            let Some(stem) = name.strip_suffix(".emb") else {
                continue;
            };
            if stem.len() != 64 || !stem.bytes().all(|b| b.is_ascii_hexdigit()) {
                continue;
            }
            let Ok(meta) = entry.metadata() else {
                continue;
            };
            if !meta.is_file() {
                continue;
            }
            index.total += meta.len();
            index.entries.insert(
                entry.path(),
                (
                    meta.modified().unwrap_or(SystemTime::UNIX_EPOCH),
                    meta.len(),
                ),
            );
        }
        Ok(index)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(seed: f32) -> Record {
        let mut embedding = [0f32; EMBED_DIM];
        for (i, v) in embedding.iter_mut().enumerate() {
            *v = seed + i as f32 * 0.001;
        }
        Record {
            embedding,
            score: 5.0 + seed,
        }
    }

    fn photo(dir: &Path, name: &str, bytes: &[u8]) -> PathBuf {
        let path = dir.join(name);
        fs::write(&path, bytes).unwrap();
        path
    }

    #[test]
    fn record_round_trips_and_survives_a_restart() {
        let dir = tempfile::tempdir().unwrap();
        let path = photo(dir.path(), "a.jpg", b"pixels");
        let key = ContentKey::of(&path, MODEL_LABEL).unwrap();
        let root = cache_root(&dir.path().join("cache"), MODEL_LABEL);
        let cache = EmbeddingCache::new(root.clone(), DEFAULT_BUDGET);
        assert_eq!(cache.get(&key), None);
        cache.put(&key, &record(1.0)).unwrap();
        assert_eq!(cache.get(&key), Some(record(1.0)));
        let restarted = EmbeddingCache::new(root, DEFAULT_BUDGET);
        assert_eq!(restarted.get(&key), Some(record(1.0)));
    }

    #[test]
    fn truncated_or_foreign_record_is_recomputed() {
        let dir = tempfile::tempdir().unwrap();
        let path = photo(dir.path(), "a.jpg", b"pixels");
        let key = ContentKey::of(&path, MODEL_LABEL).unwrap();
        let cache = EmbeddingCache::new(dir.path().join("cache"), DEFAULT_BUDGET);
        cache.put(&key, &record(1.0)).unwrap();
        let file = dir.path().join("cache").join(key.file_name());
        let whole = fs::read(&file).unwrap();
        fs::write(&file, &whole[..whole.len() - 1]).unwrap();
        assert_eq!(cache.get(&key), None);
        let mut longer = whole.clone();
        longer.push(0);
        fs::write(&file, &longer).unwrap();
        assert_eq!(cache.get(&key), None);
        let mut other_version = whole.clone();
        other_version[4] = 9;
        fs::write(&file, &other_version).unwrap();
        assert_eq!(cache.get(&key), None);
        // Rewriting replaces the broken record.
        cache.put(&key, &record(2.0)).unwrap();
        assert_eq!(cache.get(&key), Some(record(2.0)));
    }

    #[test]
    fn key_survives_a_move_to_another_folder() {
        let dir = tempfile::tempdir().unwrap();
        let path = photo(dir.path(), "a.jpg", b"pixels");
        let before = ContentKey::of(&path, MODEL_LABEL).unwrap();
        let keep = dir.path().join("keep");
        fs::create_dir(&keep).unwrap();
        let moved = keep.join("a.jpg");
        fs::rename(&path, &moved).unwrap();
        assert_eq!(ContentKey::of(&moved, MODEL_LABEL), Some(before));
        assert_ne!(ContentKey::of(&moved, "other-model"), Some(before));
    }

    #[test]
    fn key_changes_when_the_file_is_modified() {
        let dir = tempfile::tempdir().unwrap();
        let path = photo(dir.path(), "a.jpg", b"pixels");
        let before = ContentKey::of(&path, MODEL_LABEL).unwrap();
        let file = fs::File::options().write(true).open(&path).unwrap();
        file.set_modified(SystemTime::now() + std::time::Duration::from_secs(5))
            .unwrap();
        drop(file);
        assert_ne!(ContentKey::of(&path, MODEL_LABEL), Some(before));
        // Same name, same time, another size.
        let renamed = photo(dir.path(), "b.jpg", b"pixels");
        assert_ne!(
            ContentKey::of(&renamed, MODEL_LABEL),
            ContentKey::of(&path, MODEL_LABEL)
        );
    }

    #[test]
    fn missing_file_has_no_key() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(
            ContentKey::of(&dir.path().join("gone.jpg"), MODEL_LABEL),
            None
        );
        assert_eq!(ContentKey::of(dir.path(), MODEL_LABEL), None);
    }

    #[test]
    fn unwritable_root_is_an_error_not_a_panic() {
        let dir = tempfile::tempdir().unwrap();
        let path = photo(dir.path(), "a.jpg", b"pixels");
        let key = ContentKey::of(&path, MODEL_LABEL).unwrap();
        // A folder cannot be created under a file, on every system.
        let blocker = photo(dir.path(), "blocker", b"");
        let cache = EmbeddingCache::new(blocker.join("cache"), DEFAULT_BUDGET);
        assert!(cache.put(&key, &record(1.0)).is_err());
        assert_eq!(cache.get(&key), None);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let root = dir.path().join("read-only");
            fs::create_dir(&root).unwrap();
            fs::set_permissions(&root, fs::Permissions::from_mode(0o555)).unwrap();
            let cache = EmbeddingCache::new(root.clone(), DEFAULT_BUDGET);
            // Root may write anywhere; the call must simply not panic.
            let _ = cache.put(&key, &record(1.0));
            fs::set_permissions(&root, fs::Permissions::from_mode(0o755)).unwrap();
        }
    }

    #[test]
    fn oldest_records_go_first_beyond_the_budget() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("cache");
        // Room for ten records: the eleventh evicts down to nine minus a tenth of the budget.
        let cache = EmbeddingCache::new(root.clone(), RECORD_LEN as u64 * 10);
        let mut keys = Vec::new();
        for i in 0..11 {
            let path = photo(dir.path(), &format!("{i}.jpg"), b"pixels");
            let key = ContentKey::of(&path, MODEL_LABEL).unwrap();
            cache.put(&key, &record(i as f32)).unwrap();
            // Distinct, increasing write times even on coarse file systems.
            let file = fs::File::options()
                .write(true)
                .open(root.join(key.file_name()))
                .unwrap();
            file.set_modified(SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1000 + i))
                .unwrap();
            keys.push(key);
            // The index keeps the time it saw; a restart rescans the real times.
            if i == 9 {
                drop(cache.index.lock().unwrap().take());
            }
        }
        assert_eq!(cache.get(&keys[10]), Some(record(10.0)));
        assert_eq!(cache.get(&keys[0]), None);
        let left = fs::read_dir(&root).unwrap().count() as u64;
        assert!(left * RECORD_LEN as u64 <= RECORD_LEN as u64 * 10, "{left}");
        // A foreign file in the folder is never removed.
        fs::write(root.join("notes.txt"), b"mine").unwrap();
        for i in 11..30 {
            let path = photo(dir.path(), &format!("{i}.jpg"), b"pixels");
            cache
                .put(&ContentKey::of(&path, MODEL_LABEL).unwrap(), &record(1.0))
                .unwrap();
        }
        assert!(root.join("notes.txt").is_file());
    }
}
