//! ROLE: Finds and decodes the JPEG previews cameras embed in RAW files, and the EXIF
//! thumbnail of JPEG files, reading only the bytes it needs.
//! SCOPE: Portable. The RAW decoder where the system has none (Linux), and a fallback
//! where its codec is missing (Windows without a RAW extension).
//! INVARIANTS: Originals are only read; offsets are bounded by the file length.
use crate::decode::{DecodedImage, Resolution};
use std::{
    collections::BTreeSet,
    fs::File,
    io::{Read, Seek, SeekFrom},
    path::Path,
};

/// One embedded baseline or progressive JPEG and its decoded size.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Preview {
    pub offset: u64,
    /// `None` when found by scanning: the decoder stops at its end marker.
    pub length: Option<u64>,
    pub width: u32,
    pub height: u32,
}
impl Preview {
    fn edge(&self) -> u32 {
        self.width.max(self.height)
    }
}

const MAX_IFDS: usize = 64;
const MAX_ENTRIES: u64 = 2048;
const SCAN_CHUNK: usize = 4 << 20;
/// Below this long edge, the structured previews are completed by a scan of the file.
const SCAN_BELOW_EDGE: u32 = 1024;
/// Largest embedded preview accepted, as for decoded photos (`decode.rs`).
const MAX_PIXELS: u64 = 128 * 1024 * 1024;

/// Every embedded preview of a RAW file, smallest first.
pub fn raw_previews(path: &Path) -> Vec<Preview> {
    let Ok(mut file) = File::open(path) else {
        return Vec::new();
    };
    let length = file.metadata().map_or(0, |m| m.len());
    let mut header = [0; 92];
    if read_at(&mut file, 0, &mut header).is_err() {
        return Vec::new();
    }
    let found = if header.starts_with(b"FUJIFILMCCD-RAW ") {
        // RAF: a big-endian pointer to the full JPEG preview in the fixed header.
        let offset = u32::from_be_bytes(header[84..88].try_into().unwrap_or_default());
        let size = u32::from_be_bytes(header[88..92].try_into().unwrap_or_default());
        vec![(u64::from(offset), Some(u64::from(size)))]
    } else if tiff_order(&header).is_some() {
        tiff_previews(&mut file, 0, length)
    } else {
        Vec::new()
    };
    let mut previews = measure_all(&mut file, length, found);
    // CR3 and other containers, or a large preview hidden in a maker note (Olympus):
    // look for JPEG start markers through the whole file.
    if previews.last().is_none_or(|p| p.edge() < SCAN_BELOW_EDGE) {
        let starts = scan(&mut file, length);
        let mut scanned = measure_all(&mut file, length, starts);
        previews.append(&mut scanned);
        // A preview found both ways keeps its known length.
        previews.sort_by_key(|p| (p.offset, p.length.is_none()));
        previews.dedup_by_key(|p| p.offset);
    }
    previews.sort_by_key(|p| (p.edge(), p.offset));
    previews
}
fn measure_all(file: &mut File, length: u64, found: Vec<(u64, Option<u64>)>) -> Vec<Preview> {
    let mut previews: Vec<_> = found
        .into_iter()
        .filter_map(|(offset, size)| measure(file, length, offset, size))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    previews.sort_by_key(|p| (p.edge(), p.offset));
    previews
}

/// The EXIF thumbnail of a JPEG file, when it has one.
pub fn jpeg_thumbnail(path: &Path) -> Option<Preview> {
    let mut file = File::open(path).ok()?;
    let length = file.metadata().ok()?.len();
    let base = exif_segment(&mut file, length)?;
    let (offset, size) = tiff_previews(&mut file, base, length).into_iter().next()?;
    measure(&mut file, length, offset, size)
}

/// A fast first look at `edge`: a RAW file's fitting preview, or a JPEG's EXIF thumbnail
/// when it has the photo's proportions (some cameras pad it with black bars).
pub fn quick(path: &Path, edge: u32) -> Option<DecodedImage> {
    if crate::catalog::is_raw(path) {
        return decode(path, Some(edge)).ok();
    }
    let thumbnail = jpeg_thumbnail(path)?;
    let mut file = File::open(path).ok()?;
    let length = file.metadata().ok()?.len();
    let photo = measure(&mut file, length, 0, None)?;
    let ratio = |p: Preview| p.width as f32 / p.height as f32;
    if (ratio(thumbnail) / ratio(photo) - 1.).abs() > 0.02 {
        return None;
    }
    decode_preview(path, thumbnail, Some(edge), false).ok()
}

/// Decodes the preview fitting `max_edge` best: the smallest at least that large, else the
/// largest. Native means the largest preview at its full size, the best this file offers.
pub fn decode(path: &Path, max_edge: Option<u32>) -> Result<DecodedImage, String> {
    let previews = raw_previews(path);
    let largest = *previews
        .last()
        .ok_or_else(|| format!("No embedded preview in {}", path.display()))?;
    // Preferred order: large enough from the smallest up, then the smaller ones from the
    // largest down. A damaged preview gives way to the next one.
    let fits = max_edge.map_or(previews.len() - 1, |edge| {
        previews
            .iter()
            .position(|p| p.edge() >= edge)
            .unwrap_or(previews.len() - 1)
    });
    let order = previews[fits..].iter().chain(previews[..fits].iter().rev());
    let mut error = String::new();
    for &preview in order {
        match decode_preview(path, preview, max_edge, preview == largest) {
            Ok(image) => return Ok(image),
            Err(e) => error = e,
        }
    }
    Err(format!(
        "Cannot decode the preview of {}: {error}",
        path.display()
    ))
}

/// Decodes one preview, turned like the photo and scaled down to `max_edge`.
pub fn decode_preview(
    path: &Path,
    preview: Preview,
    max_edge: Option<u32>,
    largest: bool,
) -> Result<DecodedImage, String> {
    let mut file = File::open(path).map_err(|e| e.to_string())?;
    let length = file.metadata().map_err(|e| e.to_string())?.len();
    let size = preview
        .length
        .unwrap_or(length.saturating_sub(preview.offset))
        .min(length.saturating_sub(preview.offset));
    let mut bytes = vec![0; usize::try_from(size).map_err(|e| e.to_string())?];
    read_at(&mut file, preview.offset, &mut bytes).map_err(|e| e.to_string())?;
    let mut decoder = image::codecs::jpeg::JpegDecoder::new(std::io::Cursor::new(bytes))
        .map_err(|e| e.to_string())?;
    let mut limits = image::Limits::default();
    limits.max_alloc = Some(512 * 1024 * 1024);
    image::ImageDecoder::set_limits(&mut decoder, limits).map_err(|e| e.to_string())?;
    let mut image = image::DynamicImage::from_decoder(decoder).map_err(|e| e.to_string())?;
    let mut scaled = false;
    if let Some(edge) = max_edge.filter(|&edge| image.width().max(image.height()) > edge) {
        image = image.thumbnail(edge.max(1), edge.max(1));
        scaled = true;
    }
    // Cameras store previews as the sensor saw them; the file's orientation turns them.
    if let Some(orientation) =
        image::metadata::Orientation::from_exif(crate::metadata::orientation(path) as u8)
    {
        image.apply_orientation(orientation);
    }
    let (width, height) = (image.width(), image.height());
    Ok(DecodedImage {
        rgba: image.into_rgba8().into_raw(),
        width,
        height,
        resolution: if largest && !scaled {
            Resolution::Native
        } else {
            Resolution::Preview
        },
    })
}

/// Proportions of the largest preview, turned like the photo.
pub fn aspect_ratio(path: &Path) -> Option<f32> {
    let largest = *raw_previews(path).last()?;
    let (mut width, mut height) = (largest.width, largest.height);
    if matches!(crate::metadata::orientation(path), 5..=8) {
        std::mem::swap(&mut width, &mut height);
    }
    Some(width as f32 / height as f32)
}

fn read_at(file: &mut File, offset: u64, buffer: &mut [u8]) -> std::io::Result<()> {
    file.seek(SeekFrom::Start(offset))?;
    file.read_exact(buffer)
}

#[derive(Clone, Copy)]
enum Order {
    Little,
    Big,
}
impl Order {
    fn u16(self, b: &[u8]) -> u16 {
        let b = [b[0], b[1]];
        match self {
            Order::Little => u16::from_le_bytes(b),
            Order::Big => u16::from_be_bytes(b),
        }
    }
    fn u32(self, b: &[u8]) -> u32 {
        let b = [b[0], b[1], b[2], b[3]];
        match self {
            Order::Little => u32::from_le_bytes(b),
            Order::Big => u32::from_be_bytes(b),
        }
    }
}
/// TIFF byte order, including the variants of Olympus (`IIRO`, `IIRS`) and Panasonic (`IIU`).
fn tiff_order(header: &[u8]) -> Option<Order> {
    match header.get(..4)? {
        [b'I', b'I', 42 | 0x55, 0] | [b'I', b'I', b'R', b'O' | b'S'] => Some(Order::Little),
        [b'M', b'M', 0, 42] => Some(Order::Big),
        _ => None,
    }
}

/// JPEG streams referenced by a TIFF structure at `base` (offsets in it are relative to it):
/// thumbnail pointers, single-strip JPEG images and Panasonic's JpgFromRaw, in every IFD
/// and sub-IFD.
fn tiff_previews(file: &mut File, base: u64, length: u64) -> Vec<(u64, Option<u64>)> {
    let mut header = [0; 8];
    if read_at(file, base, &mut header).is_err() {
        return Vec::new();
    }
    let Some(order) = tiff_order(&header) else {
        return Vec::new();
    };
    let mut found = Vec::new();
    let mut queue = vec![u64::from(order.u32(&header[4..]))];
    let mut seen = BTreeSet::new();
    while let Some(ifd) = queue.pop() {
        if ifd == 0 || seen.len() >= MAX_IFDS || !seen.insert(ifd) {
            continue;
        }
        let mut count = [0; 2];
        if read_at(file, base + ifd, &mut count).is_err() {
            continue;
        }
        let count = u64::from(order.u16(&count)).min(MAX_ENTRIES);
        let mut entries = vec![0; count as usize * 12 + 4];
        if base + ifd + 2 + entries.len() as u64 > length
            || read_at(file, base + ifd + 2, &mut entries).is_err()
        {
            continue;
        }
        let (mut jpeg, mut jpeg_length, mut strip, mut strip_length) = (None, None, None, None);
        let mut compressed = false;
        for entry in entries[..count as usize * 12].as_chunks::<12>().0 {
            let (tag, kind, n) = (
                order.u16(entry),
                order.u16(&entry[2..]),
                order.u32(&entry[4..]),
            );
            // SHORT values sit in the first half of the value field, LONG ones fill it.
            let value = if kind == 3 {
                u32::from(order.u16(&entry[8..]))
            } else {
                order.u32(&entry[8..])
            };
            match tag {
                0x0103 => compressed = matches!(value, 6 | 7),
                0x0111 if n == 1 => strip = Some(value),
                0x0117 if n == 1 => strip_length = Some(value),
                0x0201 => jpeg = Some(value),
                0x0202 => jpeg_length = Some(value),
                // JpgFromRaw: an UNDEFINED value holding a whole JPEG file.
                0x002E if kind == 7 && n > 4 => {
                    found.push((base + u64::from(value), Some(n.into())))
                }
                0x014A => sub_ifds(file, base, order, kind, n, value, &mut queue),
                _ => {}
            }
        }
        if let (Some(offset), Some(size)) = (jpeg, jpeg_length) {
            found.push((base + u64::from(offset), Some(size.into())));
        }
        if let (true, Some(offset), Some(size)) = (compressed, strip, strip_length) {
            found.push((base + u64::from(offset), Some(size.into())));
        }
        queue.push(u64::from(order.u32(&entries[count as usize * 12..])));
    }
    found
}
fn sub_ifds(
    file: &mut File,
    base: u64,
    order: Order,
    kind: u16,
    count: u32,
    value: u32,
    queue: &mut Vec<u64>,
) {
    if !matches!(kind, 4 | 13) || count == 0 {
        return;
    }
    if count == 1 {
        queue.push(value.into());
        return;
    }
    let mut offsets = vec![0; count.min(16) as usize * 4];
    if read_at(file, base + u64::from(value), &mut offsets).is_ok() {
        queue.extend(
            offsets
                .as_chunks::<4>()
                .0
                .iter()
                .map(|o| u64::from(order.u32(o))),
        );
    }
}

/// Start of the TIFF structure inside the APP1 Exif segment of a JPEG file.
fn exif_segment(file: &mut File, length: u64) -> Option<u64> {
    let mut marker = [0; 4];
    read_at(file, 0, &mut marker[..2]).ok()?;
    if marker[..2] != [0xFF, 0xD8] {
        return None;
    }
    let mut at = 2;
    while at + 10 < length {
        read_at(file, at, &mut marker).ok()?;
        if marker[0] != 0xFF || matches!(marker[1], 0xDA | 0xD9) {
            return None;
        }
        let size = u64::from(u16::from_be_bytes([marker[2], marker[3]]));
        if marker[1] == 0xE1 {
            let mut tag = [0; 6];
            read_at(file, at + 4, &mut tag).ok()?;
            if &tag == b"Exif\0\0" {
                return Some(at + 10);
            }
        }
        at += 2 + size;
    }
    None
}

/// Offsets of JPEG start markers, read in chunks so memory stays bounded.
fn scan(file: &mut File, length: u64) -> Vec<(u64, Option<u64>)> {
    let mut found = Vec::new();
    let mut buffer = vec![0; SCAN_CHUNK + 2];
    let mut at = 0;
    while at < length {
        let size = (length - at).min(buffer.len() as u64) as usize;
        if read_at(file, at, &mut buffer[..size]).is_err() {
            break;
        }
        for (i, window) in buffer[..size].windows(3).enumerate() {
            if window == [0xFF, 0xD8, 0xFF] {
                found.push((at + i as u64, None));
            }
        }
        // Chunks overlap by two bytes, so a marker on a boundary is still seen once.
        at += SCAN_CHUNK as u64;
    }
    found.dedup();
    found
}

/// Reads a JPEG's markers up to its frame header: its size, if it is one the decoder reads.
fn measure(file: &mut File, length: u64, offset: u64, size: Option<u64>) -> Option<Preview> {
    let end = size.map_or(length, |s| offset.saturating_add(s));
    if end > length || offset + 4 > end {
        return None;
    }
    let mut head = [0; 2];
    read_at(file, offset, &mut head).ok()?;
    if head != [0xFF, 0xD8] {
        return None;
    }
    let mut at = offset + 2;
    let mut marker = [0; 9];
    while at + 9 <= end {
        read_at(file, at, &mut marker).ok()?;
        if marker[0] != 0xFF {
            return None;
        }
        match marker[1] {
            0xFF => at += 1,
            0x01 | 0xD0..=0xD7 => at += 2,
            // Baseline, extended and progressive Huffman: what the image crate decodes.
            0xC0..=0xC2 => {
                let height = u32::from(u16::from_be_bytes([marker[5], marker[6]]));
                let width = u32::from(u16::from_be_bytes([marker[7], marker[8]]));
                let fits = u64::from(width) * u64::from(height) <= MAX_PIXELS;
                return (width > 0 && height > 0 && fits).then_some(Preview {
                    offset,
                    length: size,
                    width,
                    height,
                });
            }
            // Lossless (raw sensor data in CR2 and DNG), arithmetic, end or scan: not a preview.
            0xC3 | 0xC5..=0xC7 | 0xC9..=0xCB | 0xCD..=0xCF | 0xD9 | 0xDA => return None,
            _ => at += 2 + u64::from(u16::from_be_bytes([marker[2], marker[3]])),
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    fn jpeg(width: u32, height: u32) -> Vec<u8> {
        let image = image::RgbImage::from_pixel(width, height, image::Rgb([200, 40, 20]));
        let mut bytes = Vec::new();
        image::codecs::jpeg::JpegEncoder::new(&mut bytes)
            .encode_image(&image)
            .unwrap();
        bytes
    }
    /// A little-endian TIFF whose IFD0 points to `big` by strip and IFD1 to `small` by
    /// thumbnail pointer, like a CR2.
    fn tiff(big: &[u8], small: &[u8]) -> Vec<u8> {
        let mut file = b"II*\0".to_vec();
        file.extend(8u32.to_le_bytes());
        let ifd1 = 8 + 2 + 3 * 12 + 4;
        let data = ifd1 + 2 + 2 * 12 + 4;
        let entry = |tag: u16, kind: u16, value: u32| {
            let mut e = tag.to_le_bytes().to_vec();
            e.extend(kind.to_le_bytes());
            e.extend(1u32.to_le_bytes());
            e.extend(value.to_le_bytes());
            e
        };
        file.extend(3u16.to_le_bytes());
        file.extend(entry(0x0103, 3, 6));
        file.extend(entry(0x0111, 4, data));
        file.extend(entry(0x0117, 4, big.len() as u32));
        file.extend(ifd1.to_le_bytes());
        file.extend(2u16.to_le_bytes());
        file.extend(entry(0x0201, 4, data + big.len() as u32));
        file.extend(entry(0x0202, 4, small.len() as u32));
        file.extend(0u32.to_le_bytes());
        file.extend(big);
        file.extend(small);
        file
    }
    #[test]
    fn tiff_raw_previews_are_found_and_the_fitting_one_is_decoded() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("photo.cr2");
        std::fs::write(&path, tiff(&jpeg(640, 480), &jpeg(160, 120))).unwrap();
        let previews = raw_previews(&path);
        let sizes: Vec<_> = previews.iter().map(|p| (p.width, p.height)).collect();
        assert_eq!(sizes, [(160, 120), (640, 480)]);
        let quick = decode(&path, Some(150)).unwrap();
        assert_eq!(
            (quick.width, quick.height, quick.resolution),
            (150, 113, Resolution::Preview)
        );
        let full = decode(&path, None).unwrap();
        assert_eq!(
            (full.width, full.height, full.resolution),
            (640, 480, Resolution::Native)
        );
        assert!((aspect_ratio(&path).unwrap() - 4. / 3.).abs() < 1e-6);
    }
    #[test]
    fn container_raw_previews_are_found_by_scanning() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("photo.cr3");
        let mut file = b"\0\0\0\x18ftypcrx ".to_vec();
        file.extend([0xFF, 0xD8, 0xFF, 0xC3, 0, 2]);
        file.extend(jpeg(320, 200));
        file.extend([7; 100]);
        std::fs::write(&path, file).unwrap();
        let decoded = decode(&path, None).unwrap();
        assert_eq!((decoded.width, decoded.height), (320, 200));
    }
    #[test]
    fn truncated_and_foreign_files_have_no_preview() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("photo.nef");
        let mut file = tiff(&jpeg(640, 480), &jpeg(160, 120));
        // Cut inside the first preview's header, before its frame size.
        file.truncate(100);
        std::fs::write(&path, &file).unwrap();
        assert!(raw_previews(&path).is_empty());
        assert!(decode(&path, None).is_err());
        std::fs::write(&path, b"not a photo").unwrap();
        assert!(raw_previews(&path).is_empty());
        assert!(raw_previews(&dir.path().join("missing.nef")).is_empty());
    }
}
