use sortypix_core::PhotoId;
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};
#[derive(Clone, Debug)]
pub struct PhysicalPhoto {
    pub id: PhotoId,
    pub members: Vec<PathBuf>,
    pub aspect_ratio: f32,
    /// EXIF capture time in seconds, once metadata is read; `None` without one.
    pub capture_time: Option<i64>,
    /// Bytes of all members, read while listing.
    pub bytes: u64,
}
#[derive(Clone, Debug)]
pub struct PhotoMetadata {
    pub key: PathBuf,
    pub timestamp: i64,
    pub aspect_ratio: f32,
    pub capture_time: Option<i64>,
}
impl PhysicalPhoto {
    pub fn display_source(&self, native: bool) -> &Path {
        if native && let Some(raw) = self.members.iter().find(|p| is_raw(p)) {
            return raw;
        }
        self.members
            .iter()
            .find(|p| is_jpeg(p))
            .unwrap_or(&self.members[0])
    }
}
pub fn is_raw(path: &Path) -> bool {
    matches!(
        extension(path).as_str(),
        "raw"
            | "nef"
            | "nrw"
            | "cr2"
            | "cr3"
            | "arw"
            | "sr2"
            | "srf"
            | "dng"
            | "orf"
            | "rw2"
            | "raf"
            | "pef"
            | "srw"
            | "3fr"
            | "iiq"
            | "rwl"
            | "x3f"
            | "erf"
            | "kdc"
            | "dcr"
            | "mef"
            | "mos"
    )
}
pub fn is_jpeg(path: &Path) -> bool {
    matches!(extension(path).as_str(), "jpg" | "jpeg" | "jpe" | "jfif")
}
pub fn extension(path: &Path) -> String {
    path.extension()
        .unwrap_or_default()
        .to_string_lossy()
        .to_lowercase()
}
pub fn scan_names(root: &Path) -> Result<Vec<PhysicalPhoto>, String> {
    scan_names_with(root, |_| {})
}

/// First partial listing, then one at most every [`PARTIAL_EVERY`].
pub const PARTIAL_FIRST: std::time::Duration = std::time::Duration::from_millis(40);
pub const PARTIAL_EVERY: std::time::Duration = std::time::Duration::from_millis(250);

/// `scan_names`, handing `partial` what is listed so far while a large or slow folder is read,
/// so the grid can show it. Each partial list is grouped and sorted like the final one.
pub fn scan_names_with(
    root: &Path,
    partial: impl FnMut(Vec<PhysicalPhoto>),
) -> Result<Vec<PhysicalPhoto>, String> {
    scan_paced(root, PARTIAL_FIRST, PARTIAL_EVERY, partial)
}

fn scan_paced(
    root: &Path,
    first: std::time::Duration,
    every: std::time::Duration,
    mut partial: impl FnMut(Vec<PhysicalPhoto>),
) -> Result<Vec<PhysicalPhoto>, String> {
    let started = std::time::Instant::now();
    let mut next = first;
    let mut files: Vec<(PathBuf, u64)> = Vec::new();
    for entry in fs::read_dir(root).map_err(|e| format!("Cannot open {}: {e}", root.display()))? {
        let entry = entry.map_err(|e| e.to_string())?;
        if !entry.file_type().map_err(|e| e.to_string())?.is_file() {
            continue;
        }
        let path = entry.path();
        if !is_photo(&path) {
            continue;
        }
        // The listing already carries the size on Windows: no extra file access there.
        let bytes = entry.metadata().map_or(0, |m| m.len());
        files.push((path, bytes));
        if started.elapsed() >= next {
            partial(group(&files));
            next = started.elapsed() + every;
        }
    }
    Ok(group(&files))
}

fn is_photo(path: &Path) -> bool {
    is_raw(path)
        || is_jpeg(path)
        || matches!(
            extension(path).as_str(),
            "png" | "webp" | "gif" | "bmp" | "tif" | "tiff" | "heic" | "heif" | "avif" | "jxl"
        )
}

/// Photos of `files`: a RAW and its JPEG of the same name are one photo; sorted by path.
fn group(files: &[(PathBuf, u64)]) -> Vec<PhysicalPhoto> {
    // PITFALL: comparing whole paths walks every component; one folder: compare the names.
    let name = |i: usize| files[i].0.file_name().unwrap_or_default();
    let mut stems: std::collections::HashMap<String, Vec<usize>> =
        std::collections::HashMap::with_capacity(files.len());
    for (i, (path, _)) in files.iter().enumerate() {
        let stem = path
            .file_stem()
            .unwrap_or_default()
            .to_string_lossy()
            .to_lowercase();
        stems.entry(stem).or_default().push(i);
    }
    let mut groups: Vec<Vec<usize>> = Vec::with_capacity(files.len());
    for mut members in stems.into_values() {
        if members.len() == 1 {
            groups.push(members);
            continue;
        }
        members.sort_by(|&a, &b| name(a).cmp(name(b)));
        if members.iter().any(|&i| is_raw(&files[i].0)) {
            let (paired, others): (Vec<_>, Vec<_>) = members.into_iter().partition(|&i| {
                let path = &files[i].0;
                is_raw(path) || is_jpeg(path)
            });
            groups.push(paired);
            groups.extend(others.into_iter().map(|i| vec![i]));
        } else {
            groups.extend(members.into_iter().map(|i| vec![i]));
        }
    }
    groups.sort_unstable_by(|a, b| name(a[0]).cmp(name(b[0])));
    groups
        .into_iter()
        .enumerate()
        .map(|(n, members)| PhysicalPhoto {
            id: PhotoId(n as u64),
            bytes: members.iter().map(|&i| files[i].1).sum(),
            members: members.iter().map(|&i| files[i].0.clone()).collect(),
            aspect_ratio: 1.5,
            capture_time: None,
        })
        .collect()
}

pub fn read_metadata(photos: &[PhysicalPhoto]) -> Vec<PhotoMetadata> {
    photos
        .iter()
        .filter_map(|photo| {
            let key = photo.members.first()?;
            let capture_time = crate::metadata::capture_time(key);
            Some(PhotoMetadata {
                key: key.clone(),
                timestamp: crate::metadata::timestamp_from(key, capture_time),
                aspect_ratio: crate::decode::aspect_ratio(photo.display_source(false))
                    .unwrap_or(1.5),
                capture_time,
            })
        })
        .collect()
}

pub fn scan(root: &Path) -> Result<Vec<PhysicalPhoto>, String> {
    let mut photos = scan_names(root)?;
    let metadata: BTreeMap<_, _> = read_metadata(&photos)
        .into_iter()
        .map(|m| (m.key, (m.timestamp, m.aspect_ratio, m.capture_time)))
        .collect();
    for photo in &mut photos {
        if let Some((_, aspect_ratio, capture_time)) = metadata.get(&photo.members[0]) {
            photo.aspect_ratio = *aspect_ratio;
            photo.capture_time = *capture_time;
        }
    }
    photos.sort_by_cached_key(|photo| {
        (
            metadata
                .get(&photo.members[0])
                .map_or(0, |(timestamp, _, _)| *timestamp),
            photo.members[0].clone(),
        )
    });
    Ok(photos)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn partial_listings_are_grouped_sorted_and_lead_to_the_final_one() {
        let dir = tempfile::tempdir().unwrap();
        for name in ["b.jpg", "a.NEF", "a.JPG", "c.png", "notes.txt", "a.png"] {
            fs::write(dir.path().join(name), name).unwrap();
        }
        let mut partials = Vec::new();
        let zero = std::time::Duration::ZERO;
        let last = scan_paced(dir.path(), zero, zero, |p| partials.push(p)).unwrap();
        assert_eq!(partials.len(), 5, "one per photo file listed");
        let paths = |photos: &[PhysicalPhoto]| -> Vec<Vec<PathBuf>> {
            photos.iter().map(|p| p.members.clone()).collect()
        };
        for partial in &partials {
            assert!(
                partial
                    .windows(2)
                    .all(|w| w[0].members[0] < w[1].members[0])
            );
        }
        assert_eq!(paths(&partials[4]), paths(&last));
        assert_eq!(paths(&last), paths(&scan_names(dir.path()).unwrap()));
        let pair = last.iter().find(|p| p.members.len() == 2).unwrap();
        assert_eq!(pair.bytes, 10, "both members are counted");
    }
}
