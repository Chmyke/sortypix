//! ROLE: EXIF fields of a photo (orientation, capture time), read without loading the file.
//! INVARIANTS: A JPEG costs its header segments; a TIFF-based RAW costs a bounded prefix, the
//! whole file only when its EXIF sits further.
use std::{
    fs::File,
    io::{BufReader, Read, Seek, SeekFrom},
    path::Path,
};

/// Bytes read first from any file: the marker segments of a JPEG up to its frame (APP1 Exif,
/// quantization and Huffman tables) fit in it nearly always, and so do IFD0 and the EXIF IFD of
/// a TIFF-based RAW; a maker note or a preview can push them further, then more is read.
const PREFIX: usize = 64 * 1024;
/// Bytes of a TIFF-based file (RAW, DNG, TIFF) parsed before the whole file is read.
const TIFF_PREFIX: usize = 256 * 1024;

/// What one EXIF read gives the catalog, so a photo is parsed once.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Summary {
    /// EXIF orientation, 1 to 8; 1 without one.
    pub orientation: u16,
    pub capture_time: Option<i64>,
}
/// [`Summary`] with the stored size of a JPEG, read from the same bytes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Inspection {
    pub summary: Summary,
    /// Frame size of a JPEG as stored (not turned); `None` for other formats.
    pub jpeg_size: Option<(u32, u32)>,
}

pub fn read(path: &Path) -> Option<exif::Exif> {
    read_with_size(path).0
}
/// One open and, nearly always, one read: the EXIF and, for a JPEG, its frame size.
pub fn inspect(path: &Path) -> Inspection {
    let (exif, jpeg_size) = read_with_size(path);
    Inspection {
        summary: Summary {
            orientation: exif.as_ref().map_or(1, orientation_of),
            capture_time: exif.as_ref().and_then(capture_time_of),
        },
        jpeg_size,
    }
}
fn read_with_size(path: &Path) -> (Option<exif::Exif>, Option<(u32, u32)>) {
    let Ok(mut file) = File::open(path) else {
        return (None, None);
    };
    let mut prefix = vec![0; PREFIX];
    let Ok(read) = fill(&mut file, &mut prefix) else {
        return (None, None);
    };
    prefix.truncate(read);
    if prefix.starts_with(&[0xFF, 0xD8]) {
        return jpeg(&mut file, &prefix);
    }
    if crate::embedded::is_tiff(&prefix) {
        return (tiff(&mut file, prefix), None);
    }
    if file.seek(SeekFrom::Start(0)).is_err() {
        return (None, None);
    }
    let exif = exif::Reader::new()
        .read_from_container(&mut BufReader::new(file))
        .ok();
    (exif, None)
}
/// The APP1 segment alone: the EXIF reader would otherwise walk the scan data of a JPEG
/// without EXIF to its end. The segments are read from `prefix`; a header longer than it
/// (a large colour profile before the frame) is walked in the file.
fn jpeg(file: &mut File, prefix: &[u8]) -> (Option<exif::Exif>, Option<(u32, u32)>) {
    let Ok(length) = file.metadata().map(|m| m.len()) else {
        return (None, None);
    };
    let (exif, size) = match crate::embedded::jpeg_header(prefix, length) {
        Some(header) => (header.exif, header.size),
        None => (
            crate::embedded::exif_segment(file, length),
            crate::embedded::measure(file, length, 0, None).map(|p| (p.width, p.height)),
        ),
    };
    let exif = exif.and_then(|(start, size)| {
        let segment = match prefix.get(start as usize..(start + size) as usize) {
            Some(segment) => segment.to_vec(),
            None => {
                let mut segment = vec![0; usize::try_from(size).ok()?];
                file.seek(SeekFrom::Start(start)).ok()?;
                file.read_exact(&mut segment).ok()?;
                segment
            }
        };
        exif::Reader::new().read_raw(segment).ok()
    });
    (exif, size)
}
/// IFD0 and the EXIF IFD sit near the start of a RAW: the prefix is tried first, then more of
/// the file, then all of it.
fn tiff(file: &mut File, mut prefix: Vec<u8>) -> Option<exif::Exif> {
    let reader = exif::Reader::new();
    for limit in [PREFIX, TIFF_PREFIX] {
        if prefix.len() < limit {
            // The file ends within the prefix: it holds everything there is.
            return reader.read_raw(prefix).ok();
        }
        if let Ok(exif) = reader.read_raw(prefix.clone()) {
            return Some(exif);
        }
        let from = prefix.len();
        prefix.resize(TIFF_PREFIX.max(from), 0);
        let read = fill(file, &mut prefix[from..]).ok()?;
        prefix.truncate(from + read);
    }
    file.seek(SeekFrom::Start(0)).ok()?;
    reader.read_from_container(&mut BufReader::new(file)).ok()
}
fn fill(file: &mut File, buffer: &mut [u8]) -> std::io::Result<usize> {
    let mut read = 0;
    while read < buffer.len() {
        match file.read(&mut buffer[read..]) {
            Ok(0) => break,
            Ok(n) => read += n,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
            Err(e) => return Err(e),
        }
    }
    Ok(read)
}
pub fn summary(path: &Path) -> Summary {
    inspect(path).summary
}
pub fn orientation(path: &Path) -> u16 {
    read(path).as_ref().map_or(1, orientation_of)
}
fn orientation_of(exif: &exif::Exif) -> u16 {
    exif.get_field(exif::Tag::Orientation, exif::In::PRIMARY)
        .and_then(|f| f.value.get_uint(0))
        .map_or(1, |o| o as u16)
}
/// Sorting time of a photo: its capture time, else its modification time, else 0.
pub fn timestamp(path: &Path) -> i64 {
    timestamp_from(path, capture_time(path))
}
/// [`timestamp`] once the capture time is known, so EXIF is read only once.
pub fn timestamp_from(path: &Path, capture_time: Option<i64>) -> i64 {
    if let Some(seconds) = capture_time {
        return seconds;
    }
    path.metadata()
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_secs() as i64)
}
/// Capture time from EXIF alone (DateTimeOriginal, then DateTime), in seconds of local civil
/// time counted like Unix time; `None` without a readable date.
pub fn capture_time(path: &Path) -> Option<i64> {
    read(path).as_ref().and_then(capture_time_of)
}
fn capture_time_of(exif: &exif::Exif) -> Option<i64> {
    let field = exif
        .get_field(exif::Tag::DateTimeOriginal, exif::In::PRIMARY)
        .or_else(|| exif.get_field(exif::Tag::DateTime, exif::In::PRIMARY))?;
    let date = if let exif::Value::Ascii(values) = &field.value {
        exif::DateTime::from_ascii(values.first()?).ok()
    } else {
        None
    };
    date.map(|d| {
        civil_seconds(
            d.year as i64,
            d.month as i64,
            d.day as i64,
            d.hour as i64,
            d.minute as i64,
            d.second as i64,
        )
    })
}
fn civil_seconds(year: i64, month: i64, day: i64, hour: i64, minute: i64, second: i64) -> i64 {
    let y = year - i64::from(month <= 2);
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let m = month + if month > 2 { -3 } else { 9 };
    let doy = (153 * m + 2) / 5 + day - 1;
    let days = era * 146097 + yoe * 365 + yoe / 4 - yoe / 100 + doy - 719468;
    days * 86400 + hour * 3600 + minute * 60 + second
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn calendar_epoch_and_leap_day() {
        assert_eq!(civil_seconds(1970, 1, 1, 0, 0, 0), 0);
        assert_eq!(
            civil_seconds(2000, 3, 1, 0, 0, 0) - civil_seconds(2000, 2, 28, 0, 0, 0),
            172800
        );
    }
    #[test]
    fn capture_time_is_exif_only_and_timestamp_falls_back_to_the_file_time() {
        // `SORTYPIX_FIXTURES` points to a copy where the build path does not exist (Linux).
        let fixture = std::env::var_os("SORTYPIX_FIXTURES")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures"))
            .join("orientation-1.jpg");
        let taken = civil_seconds(2025, 6, 1, 12, 30, 0);
        assert_eq!(capture_time(&fixture), Some(taken));
        assert_eq!(timestamp(&fixture), taken);
        let dir = tempfile::tempdir().unwrap();
        let plain = dir.path().join("plain.jpg");
        std::fs::write(&plain, b"no exif").unwrap();
        assert_eq!(capture_time(&plain), None);
        assert!(timestamp(&plain) > 0);
        assert_eq!(capture_time(&dir.path().join("gone.jpg")), None);
    }
}
#[cfg(test)]
mod bounded_tests {
    use super::*;
    /// A little-endian TIFF whose only IFD holds an orientation, placed at `at`.
    fn tiff_with_ifd_at(at: u32) -> Vec<u8> {
        let mut file = b"II*\0".to_vec();
        file.extend(at.to_le_bytes());
        file.resize(at as usize, 0);
        file.extend(1u16.to_le_bytes());
        file.extend(0x0112u16.to_le_bytes());
        file.extend(3u16.to_le_bytes());
        file.extend(1u32.to_le_bytes());
        file.extend(6u32.to_le_bytes());
        file.extend(0u32.to_le_bytes());
        file
    }
    #[test]
    fn tiff_exif_is_found_in_the_prefix_or_further_in_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let near = dir.path().join("near.dng");
        std::fs::write(&near, tiff_with_ifd_at(8)).unwrap();
        assert_eq!(summary(&near).orientation, 6);
        let far = dir.path().join("far.dng");
        std::fs::write(&far, tiff_with_ifd_at(TIFF_PREFIX as u32 + 100)).unwrap();
        assert_eq!(summary(&far).orientation, 6);
        let truncated = dir.path().join("cut.dng");
        std::fs::write(&truncated, &tiff_with_ifd_at(8)[..12]).unwrap();
        assert_eq!(
            summary(&truncated),
            Summary {
                orientation: 1,
                capture_time: None
            }
        );
    }
    #[test]
    fn jpeg_summary_reads_the_exif_segment_once_and_a_bare_jpeg_has_none() {
        let fixture = std::env::var_os("SORTYPIX_FIXTURES")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures"))
            .join("orientation-8.jpg");
        let found = summary(&fixture);
        assert_eq!(found.orientation, 8);
        assert_eq!(found.capture_time, capture_time(&fixture));
        assert!(found.capture_time.is_some());
        let dir = tempfile::tempdir().unwrap();
        let bare = dir.path().join("bare.jpg");
        let image = image::RgbImage::from_pixel(8, 8, image::Rgb([1, 2, 3]));
        image.save(&bare).unwrap();
        assert_eq!(
            summary(&bare),
            Summary {
                orientation: 1,
                capture_time: None
            }
        );
    }
}
