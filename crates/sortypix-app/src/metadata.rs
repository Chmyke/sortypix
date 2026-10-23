use std::{fs::File, io::BufReader, path::Path};
pub fn read(path: &Path) -> Option<exif::Exif> {
    exif::Reader::new()
        .read_from_container(&mut BufReader::new(File::open(path).ok()?))
        .ok()
}
pub fn orientation(path: &Path) -> u16 {
    read(path)
        .and_then(|e| {
            e.get_field(exif::Tag::Orientation, exif::In::PRIMARY)
                .and_then(|f| f.value.get_uint(0))
        })
        .unwrap_or(1) as u16
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
    let date = read(path).and_then(|e| {
        let field = e
            .get_field(exif::Tag::DateTimeOriginal, exif::In::PRIMARY)
            .or_else(|| e.get_field(exif::Tag::DateTime, exif::In::PRIMARY))?;
        if let exif::Value::Ascii(values) = &field.value {
            exif::DateTime::from_ascii(values.first()?).ok()
        } else {
            None
        }
    });
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
