use crate::catalog::PhysicalPhoto;
use sha2::{Digest, Sha256};
use sortypix_core::PhotoId;
use std::{
    collections::BTreeMap,
    fs::File,
    io::{BufReader, Read},
    path::Path,
};
pub fn exact_duplicate_groups(photos: &[PhysicalPhoto]) -> Result<Vec<PhotoId>, String> {
    let mut ordered: Vec<_> = photos.iter().collect();
    ordered.sort_by_cached_key(|p| {
        let name = p.members[0]
            .file_stem()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();
        (name.chars().count(), name, p.members.clone())
    });
    let mut sizes: BTreeMap<Vec<(String, u64)>, Vec<&PhysicalPhoto>> = BTreeMap::new();
    for photo in ordered {
        let key = photo
            .members
            .iter()
            .map(|p| {
                Ok((
                    crate::catalog::extension(p),
                    p.metadata().map_err(|e| e.to_string())?.len(),
                ))
            })
            .collect::<Result<_, String>>()?;
        sizes.entry(key).or_default().push(photo);
    }
    let mut rejected = Vec::new();
    for candidates in sizes.values().filter(|g| g.len() > 1) {
        let mut hashes: BTreeMap<Vec<[u8; 32]>, Vec<&PhysicalPhoto>> = BTreeMap::new();
        for photo in candidates {
            let key = photo
                .members
                .iter()
                .map(|p| fingerprint(p))
                .collect::<Result<_, _>>()?;
            let matches = hashes.entry(key).or_default();
            let mut duplicate = false;
            for kept in matches.iter() {
                let mut equal = true;
                for (a, b) in kept.members.iter().zip(&photo.members) {
                    if !same_bytes(a, b)? {
                        equal = false;
                        break;
                    }
                }
                if equal {
                    duplicate = true;
                    break;
                }
            }
            if duplicate {
                rejected.push(photo.id);
            } else {
                matches.push(photo);
            }
        }
    }
    rejected.sort();
    Ok(rejected)
}
pub fn fingerprint(path: &Path) -> Result<[u8; 32], String> {
    let mut file = BufReader::new(File::open(path).map_err(|e| e.to_string())?);
    let mut hash = Sha256::new();
    let mut buffer = [0u8; 65536];
    loop {
        let n = file.read(&mut buffer).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        hash.update(&buffer[..n]);
    }
    Ok(hash.finalize().into())
}
fn same_bytes(a: &Path, b: &Path) -> Result<bool, String> {
    let mut a = BufReader::new(File::open(a).map_err(|e| e.to_string())?);
    let mut b = BufReader::new(File::open(b).map_err(|e| e.to_string())?);
    let mut left = [0u8; 65536];
    let mut right = [0u8; 65536];
    loop {
        let n = a.read(&mut left).map_err(|e| e.to_string())?;
        if b.read_exact(&mut right[..n]).is_err() {
            return Ok(false);
        }
        if left[..n] != right[..n] {
            return Ok(false);
        }
        if n == 0 {
            return b
                .read(&mut right[..1])
                .map(|n| n == 0)
                .map_err(|e| e.to_string());
        }
    }
}
