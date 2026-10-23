//! ROLE: Sortypix AI only. Mark kinds, the "show only" filter of the order menu and the badges
//! hidden by choice: pure, no egui, no model.
//! OWNER: `Ai::arrange` filters the arranged view with [`apply`]; `marks::paint` asks a mark
//! for its first badge not hidden.
//! INVARIANTS: Filtering never changes the marks: they are computed on the whole arranged view
//! first, so "Low score only" shows the same photos the grid marks as low. A hidden badge hides
//! the badge only; the red outline of a suggested photo stays (it is what "Reject suggested"
//! would move).
use super::suggest::Marks;

/// The three kinds of marks of the design board.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Kind {
    /// Among the lowest scores of the "Suggest" share (red score badge).
    Low,
    /// Unlike the rest of the folder (amber).
    OffTopic,
    /// One of a burst of similar shots (grey).
    Burst,
}

impl Kind {
    pub(crate) const ALL: [Kind; 3] = [Kind::Low, Kind::OffTopic, Kind::Burst];

    /// Name in the menu ("Low score only", "Low score badge").
    pub(crate) fn label(self) -> &'static str {
        match self {
            Kind::Low => "Low score",
            Kind::OffTopic => "Off-topic",
            Kind::Burst => "Burst",
        }
    }

    /// Key in the shared settings.
    pub(crate) fn key(self) -> &'static str {
        match self {
            Kind::Low => "low",
            Kind::OffTopic => "off_topic",
            Kind::Burst => "burst",
        }
    }

    pub(crate) fn from_key(key: &str) -> Option<Kind> {
        Kind::ALL.into_iter().find(|k| k.key() == key)
    }
}

/// A set of kinds (the kinds of one mark, or the badges hidden).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Kinds {
    pub low: bool,
    pub off_topic: bool,
    pub burst: bool,
}

impl Kinds {
    pub(crate) fn has(self, kind: Kind) -> bool {
        match kind {
            Kind::Low => self.low,
            Kind::OffTopic => self.off_topic,
            Kind::Burst => self.burst,
        }
    }

    pub(crate) fn set(&mut self, kind: Kind, on: bool) {
        match kind {
            Kind::Low => self.low = on,
            Kind::OffTopic => self.off_topic = on,
            Kind::Burst => self.burst = on,
        }
    }
}

/// The "Show" entries of the menu: everything, or only the photos with one kind of mark.
pub(crate) fn show_label(show: Option<Kind>) -> &'static str {
    match show {
        None => "All photos",
        Some(Kind::Low) => "Low score only",
        Some(Kind::OffTopic) => "Off-topic only",
        Some(Kind::Burst) => "Bursts only",
    }
}

/// The photos of `view` (indices, display order) that `show` keeps; `id` gives a photo's key
/// in `marks`. Order is kept.
pub(crate) fn apply(
    view: &[usize],
    id: impl Fn(usize) -> u64,
    marks: &Marks,
    show: Option<Kind>,
) -> Vec<usize> {
    let Some(kind) = show else {
        return view.to_vec();
    };
    view.iter()
        .copied()
        .filter(|&i| marks.by_id.get(&id(i)).is_some_and(|m| m.kinds.has(kind)))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::suggest::{Badge, Mark};

    fn mark(kinds: Kinds) -> Mark {
        Mark {
            suggested: kinds.low || kinds.off_topic,
            badges: vec![Badge::Burst],
            kinds,
            tip: String::new(),
        }
    }

    #[test]
    fn show_only_keeps_the_photos_of_that_kind_in_order() {
        let mut marks = Marks::default();
        marks.by_id.insert(
            10,
            mark(Kinds {
                low: true,
                burst: true,
                ..Kinds::default()
            }),
        );
        marks.by_id.insert(
            11,
            mark(Kinds {
                off_topic: true,
                ..Kinds::default()
            }),
        );
        marks.by_id.insert(
            12,
            mark(Kinds {
                burst: true,
                ..Kinds::default()
            }),
        );
        // View indices 0..4 hold ids 13, 12, 11, 10 (13 has no mark).
        let view = [0, 1, 2, 3];
        let id = |i: usize| 13 - i as u64;
        assert_eq!(apply(&view, id, &marks, None), view);
        assert_eq!(apply(&view, id, &marks, Some(Kind::Low)), [3]);
        assert_eq!(apply(&view, id, &marks, Some(Kind::OffTopic)), [2]);
        assert_eq!(apply(&view, id, &marks, Some(Kind::Burst)), [1, 3]);
        assert!(apply(&[], id, &marks, Some(Kind::Burst)).is_empty());
        assert!(apply(&view, id, &Marks::default(), Some(Kind::Low)).is_empty());
    }

    #[test]
    fn kinds_and_keys_round_trip() {
        let mut kinds = Kinds::default();
        for kind in Kind::ALL {
            assert_eq!(Kind::from_key(kind.key()), Some(kind));
            kinds.set(kind, true);
            assert!(kinds.has(kind));
            for label in [kind.label(), show_label(Some(kind))] {
                assert!(!label.contains('\u{2014}') && !label.contains(" \u{2013} ")); // dash-ok
            }
        }
        assert_eq!(
            kinds,
            Kinds {
                low: true,
                off_topic: true,
                burst: true
            }
        );
        assert_eq!(Kind::from_key("everything"), None);
    }
}
