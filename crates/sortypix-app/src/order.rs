//! ROLE: The file orders both editions offer: capture date, name, size. Pure, no egui.
//! OWNER: Settings.order persists the choice; the grid applies it to the worker's catalog,
//! which is always in capture order.
//! INVARIANTS: `arrange` returns every index once; ties keep capture order, so the result is
//! the same on every run.
use crate::catalog::PhysicalPhoto;
use std::cmp::Ordering;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FileOrder {
    #[default]
    Oldest,
    Newest,
    /// File name, digits compared as numbers (IMG_2 before IMG_10), as the Explorer does.
    Name,
    Largest,
    Smallest,
}

impl FileOrder {
    pub const ALL: [FileOrder; 5] = [
        FileOrder::Oldest,
        FileOrder::Newest,
        FileOrder::Name,
        FileOrder::Largest,
        FileOrder::Smallest,
    ];

    pub fn label(self) -> &'static str {
        match self {
            FileOrder::Oldest => "Oldest first",
            FileOrder::Newest => "Newest first",
            FileOrder::Name => "By name",
            FileOrder::Largest => "Largest first",
            FileOrder::Smallest => "Smallest first",
        }
    }

    /// The order a settings value names; an unknown value (a later version) reads as `None`.
    pub fn from_value(value: &serde_json::Value) -> Option<Self> {
        serde_json::from_value(value.clone()).ok()
    }
}

/// Indices of `photos` (capture order) in `order`.
pub fn arrange(photos: &[PhysicalPhoto], order: FileOrder) -> Vec<usize> {
    let mut shown: Vec<usize> = (0..photos.len()).collect();
    match order {
        FileOrder::Oldest => {}
        FileOrder::Newest => shown.reverse(),
        FileOrder::Name => {
            let names: Vec<String> = photos.iter().map(sort_name).collect();
            shown.sort_by(|&a, &b| natural_cmp(&names[a], &names[b]));
        }
        FileOrder::Largest => shown.sort_by(|&a, &b| photos[b].bytes.cmp(&photos[a].bytes)),
        FileOrder::Smallest => shown.sort_by_key(|&i| photos[i].bytes),
    }
    shown
}

fn sort_name(photo: &PhysicalPhoto) -> String {
    photo
        .members
        .first()
        .and_then(|p| p.file_name())
        .unwrap_or_default()
        .to_string_lossy()
        .to_lowercase()
}

/// Compares runs of digits by value, the rest character by character.
pub fn natural_cmp(a: &str, b: &str) -> Ordering {
    let (mut a, mut b) = (a.chars().peekable(), b.chars().peekable());
    loop {
        match (a.peek().copied(), b.peek().copied()) {
            (None, None) => return Ordering::Equal,
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(x), Some(y)) if x.is_ascii_digit() && y.is_ascii_digit() => {
                let (x, y) = (digits(&mut a), digits(&mut b));
                let (tx, ty) = (x.trim_start_matches('0'), y.trim_start_matches('0'));
                let by_value = tx.len().cmp(&ty.len()).then_with(|| tx.cmp(ty));
                // Same value: fewer leading zeros first, so the order stays total.
                match by_value.then_with(|| x.len().cmp(&y.len())) {
                    Ordering::Equal => {}
                    other => return other,
                }
            }
            (Some(x), Some(y)) => {
                if x != y {
                    return x.cmp(&y);
                }
                a.next();
                b.next();
            }
        }
    }
}

fn digits(chars: &mut std::iter::Peekable<std::str::Chars<'_>>) -> String {
    let mut run = String::new();
    while let Some(c) = chars.peek().copied().filter(char::is_ascii_digit) {
        run.push(c);
        chars.next();
    }
    run
}

#[cfg(test)]
mod tests {
    use super::*;
    use sortypix_core::PhotoId;
    use std::path::PathBuf;

    fn photo(name: &str, bytes: u64) -> PhysicalPhoto {
        PhysicalPhoto {
            id: PhotoId(0),
            members: vec![PathBuf::from(name)],
            aspect_ratio: 1.5,
            capture_time: None,
            bytes,
        }
    }

    #[test]
    fn natural_order_reads_numbers_as_numbers() {
        let mut names = [
            "img_10.jpg",
            "IMG_2.jpg",
            "img_1.jpg",
            "img_02.jpg",
            "a.jpg",
        ]
        .map(str::to_lowercase)
        .to_vec();
        names.sort_by(|a, b| natural_cmp(a, b));
        assert_eq!(
            names,
            [
                "a.jpg",
                "img_1.jpg",
                "img_2.jpg",
                "img_02.jpg",
                "img_10.jpg"
            ]
        );
    }

    #[test]
    fn every_order_keeps_each_photo_once_and_ties_in_capture_order() {
        let photos = vec![
            photo("C:/p/b10.jpg", 300),
            photo("C:/p/B2.jpg", 100),
            photo("C:/p/a.jpg", 300),
        ];
        assert_eq!(arrange(&photos, FileOrder::Oldest), [0, 1, 2]);
        assert_eq!(arrange(&photos, FileOrder::Newest), [2, 1, 0]);
        assert_eq!(arrange(&photos, FileOrder::Name), [2, 1, 0]);
        assert_eq!(arrange(&photos, FileOrder::Largest), [0, 2, 1]);
        assert_eq!(arrange(&photos, FileOrder::Smallest), [1, 0, 2]);
    }

    #[test]
    fn unknown_values_are_refused_not_guessed() {
        assert_eq!(
            FileOrder::from_value(&"largest".into()),
            Some(FileOrder::Largest)
        );
        assert_eq!(FileOrder::from_value(&"rating".into()), None);
    }
}
