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
pub fn timestamp(path: &Path) -> i64 {
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
    if let Some(d) = date {
        return civil_seconds(
            d.year as i64,
            d.month as i64,
            d.day as i64,
            d.hour as i64,
            d.minute as i64,
            d.second as i64,
        );
    }
    path.metadata()
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_secs() as i64)
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
}
