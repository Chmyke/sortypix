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
}
#[derive(Clone, Debug)]
pub struct PhotoMetadata {
    pub key: PathBuf,
    pub timestamp: i64,
    pub aspect_ratio: f32,
}
impl PhysicalPhoto {
    pub fn display_source(&self, native: bool) -> &Path {
        if native && let Some(raw) = self.members.iter().find(|p| is_raw(p)) {
            return raw;
        }
        self.members
            .iter()
            .find(|p| matches!(extension(p).as_str(), "jpg" | "jpeg"))
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
    )
}
pub fn extension(path: &Path) -> String {
    path.extension()
        .unwrap_or_default()
        .to_string_lossy()
        .to_lowercase()
}
pub fn scan_names(root: &Path) -> Result<Vec<PhysicalPhoto>, String> {
    let mut stems: BTreeMap<String, Vec<PathBuf>> = BTreeMap::new();
    for entry in fs::read_dir(root).map_err(|e| format!("Cannot open {}: {e}", root.display()))? {
        let entry = entry.map_err(|e| e.to_string())?;
        if !entry.file_type().map_err(|e| e.to_string())?.is_file() {
            continue;
        }
        let path = entry.path();
        if !is_raw(&path)
            && !matches!(
                extension(&path).as_str(),
                "jpg"
                    | "jpeg"
                    | "png"
                    | "webp"
                    | "gif"
                    | "bmp"
                    | "tif"
                    | "tiff"
                    | "heic"
                    | "heif"
                    | "avif"
            )
        {
            continue;
        }
        let stem = path
            .file_stem()
            .unwrap_or_default()
            .to_string_lossy()
            .to_lowercase();
        stems.entry(stem).or_default().push(path);
    }
    let mut groups = Vec::new();
    for mut members in stems.into_values() {
        members.sort();
        if members.iter().any(|p| is_raw(p)) {
            let (paired, others): (Vec<_>, Vec<_>) = members
                .into_iter()
                .partition(|p| is_raw(p) || matches!(extension(p).as_str(), "jpg" | "jpeg"));
            groups.push(paired);
            groups.extend(others.into_iter().map(|p| vec![p]));
        } else {
            groups.extend(members.into_iter().map(|p| vec![p]));
        }
    }
    groups.sort_by(|a, b| a[0].cmp(&b[0]));
    Ok(groups
        .into_iter()
        .enumerate()
        .map(|(i, members)| PhysicalPhoto {
            id: PhotoId(i as u64),
            members,
            aspect_ratio: 1.5,
        })
        .collect())
}

pub fn read_metadata(photos: &[PhysicalPhoto]) -> Vec<PhotoMetadata> {
    photos
        .iter()
        .filter_map(|photo| {
            Some(PhotoMetadata {
                key: photo.members.first()?.clone(),
                timestamp: crate::metadata::timestamp(&photo.members[0]),
                aspect_ratio: crate::decode::aspect_ratio(photo.display_source(false))
                    .unwrap_or(1.5),
            })
        })
        .collect()
}

pub fn scan(root: &Path) -> Result<Vec<PhysicalPhoto>, String> {
    let mut photos = scan_names(root)?;
    let metadata: BTreeMap<_, _> = read_metadata(&photos)
        .into_iter()
        .map(|metadata| (metadata.key, (metadata.timestamp, metadata.aspect_ratio)))
        .collect();
    for photo in &mut photos {
        if let Some((_, aspect_ratio)) = metadata.get(&photo.members[0]) {
            photo.aspect_ratio = *aspect_ratio;
        }
    }
    photos.sort_by_cached_key(|photo| {
        (
            metadata
                .get(&photo.members[0])
                .map_or(0, |(timestamp, _)| *timestamp),
            photo.members[0].clone(),
        )
    });
    Ok(photos)
}
