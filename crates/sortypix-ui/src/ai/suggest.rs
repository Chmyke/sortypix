//! ROLE: Sortypix AI only. Which shown photos are suggested for rejection, and the mark each
//! photo carries: pure, no egui, no model.
//! OWNER: `Ai::arrange` and `Ai::refresh` recompute the marks of the photos currently shown.
//! INVARIANTS: Only analysed photos currently shown are suggested, each once. Nothing here
//! moves a photo: "Reject suggested" sends [`reject_ids`] through the usual decision, one undo.
use super::filters::{Kind, Kinds};
use super::view::Facts;
use sortypix_ai::OFF_TOPIC_Z;
use sortypix_core::PhotoId;
use std::collections::{HashMap, HashSet};

/// Slider of the toolbar: share of the analysed photos suggested, by lowest score.
pub(crate) const PERCENT_MAX: u32 = 50;
pub(crate) const PERCENT_STEP: u32 = 5;
/// The value of the design board (2026-10-22).
pub(crate) const PERCENT_DEFAULT: u32 = 20;

/// The badge at the top right of a marked photo.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Badge {
    /// Suggested for its low score, shown with one decimal (red).
    Score(f32),
    /// Unlike the rest of the folder (amber).
    OffTopic,
    /// One of a burst of similar shots (grey).
    Burst,
}

impl Badge {
    pub(crate) fn kind(self) -> Kind {
        match self {
            Badge::Score(_) => Kind::Low,
            Badge::OffTopic => Kind::OffTopic,
            Badge::Burst => Kind::Burst,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Mark {
    /// Suggested for rejection: red dashed outline, counted by "Reject suggested".
    pub suggested: bool,
    /// Badges that apply, the one drawn first (off-topic, then score, then burst).
    pub badges: Vec<Badge>,
    /// Kinds of the mark, for the "show only" filter.
    pub kinds: Kinds,
    /// Hover text, one short line per reason.
    pub tip: String,
}

impl Mark {
    /// The badge drawn: the first whose kind is not in `hidden`.
    pub(crate) fn badge(&self, hidden: Kinds) -> Option<Badge> {
        self.badges.iter().copied().find(|b| !hidden.has(b.kind()))
    }
}

/// Marks of the shown photos, and the suggested ones in display order.
#[derive(Debug, Default, PartialEq)]
pub(crate) struct Marks {
    /// Keyed by `PhotoId.0`.
    pub by_id: HashMap<u64, Mark>,
    pub suggested: Vec<PhotoId>,
}

/// Marks of `shown` (display order, with their facts). With `percent` 0 nothing is suggested;
/// otherwise the lowest `percent` % of the analysed photos by score, every off-topic photo
/// (above [`OFF_TOPIC_Z`]) and, when `burst_suggest`, each burst's photos but its best-scored one.
/// Off-topic photos and bursts keep their badge when not suggested. A repeated id counts once.
pub(crate) fn suggest(shown: &[(PhotoId, Facts)], percent: u32, burst_suggest: bool) -> Marks {
    let mut seen = HashSet::new();
    let shown: Vec<&(PhotoId, Facts)> = shown.iter().filter(|(id, _)| seen.insert(id.0)).collect();
    let percent = percent.min(100) as usize;
    let mut analysed: Vec<usize> = (0..shown.len())
        .filter(|&i| shown[i].1.score.is_some())
        .collect();
    analysed.sort_by(|&a, &b| {
        let (sa, sb) = (shown[a].1.score, shown[b].1.score);
        sa.unwrap_or(0.)
            .total_cmp(&sb.unwrap_or(0.))
            .then(a.cmp(&b))
    });
    let low: HashSet<usize> = analysed
        .iter()
        .take(analysed.len() * percent / 100)
        .copied()
        .collect();
    // Best score of each burst among the shown photos (ties: first shown).
    let mut best: HashMap<usize, (f32, usize)> = HashMap::new();
    for (i, (_, facts)) in shown.iter().enumerate() {
        if let (Some(burst), Some(score)) = (facts.burst, facts.score) {
            let entry = best.entry(burst.group).or_insert((score, i));
            if score > entry.0 {
                *entry = (score, i);
            }
        }
    }
    let mut marks = Marks::default();
    for (i, (id, facts)) in shown.iter().enumerate() {
        let off_topic = facts.score.is_some() && facts.off_topic.is_some_and(|z| z > OFF_TOPIC_Z);
        let better = facts
            .burst
            .and_then(|b| best.get(&b.group))
            .filter(|(_, at)| *at != i)
            .map(|(score, _)| *score);
        let burst_suggested = burst_suggest && better.is_some();
        let mut tip = Vec::new();
        if off_topic {
            tip.push("Off-topic: unlike the rest of the folder".to_string());
        }
        if low.contains(&i)
            && let Some(score) = facts.score
        {
            tip.push(format!("Low score {score:.1}"));
        }
        if let Some(burst) = facts.burst {
            tip.push(match better {
                Some(score) if burst_suggest => format!("Burst: a better shot scores {score:.1}"),
                _ => format!("Burst: one of {} similar shots taken together", burst.size),
            });
        }
        let is_low = low.contains(&i);
        let suggested = percent > 0 && (off_topic || is_low || burst_suggested);
        let mut badges = Vec::new();
        if off_topic {
            badges.push(Badge::OffTopic);
        }
        if suggested
            && (is_low || burst_suggested)
            && let Some(score) = facts.score
        {
            badges.push(Badge::Score(score));
        }
        if facts.burst.is_some() {
            badges.push(Badge::Burst);
        }
        if badges.is_empty() {
            continue;
        }
        if suggested {
            marks.suggested.push(*id);
        }
        marks.by_id.insert(
            id.0,
            Mark {
                suggested,
                badges,
                kinds: Kinds {
                    low: is_low,
                    off_topic,
                    burst: facts.burst.is_some(),
                },
                tip: tip.join("\n"),
            },
        );
    }
    marks
}

/// What "Reject suggested" sends: the suggested photos still shown, each once, in display order.
pub(crate) fn reject_ids(suggested: &[PhotoId], shown: &[PhotoId]) -> Vec<PhotoId> {
    let wanted: HashSet<u64> = suggested.iter().map(|id| id.0).collect();
    let mut sent = HashSet::new();
    shown
        .iter()
        .copied()
        .filter(|id| wanted.contains(&id.0) && sent.insert(id.0))
        .collect()
}

/// Clamps a stored percentage to the slider's range and step.
pub(crate) fn clamp_percent(value: u64) -> u32 {
    let value = value.min(u64::from(PERCENT_MAX)) as u32;
    value / PERCENT_STEP * PERCENT_STEP
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::view::Burst;

    fn photo(id: u64, score: Option<f32>) -> (PhotoId, Facts) {
        (
            PhotoId(id),
            Facts {
                score,
                ..Facts::default()
            },
        )
    }

    #[test]
    fn the_lowest_share_of_analysed_photos_is_suggested() {
        // Ten analysed photos with scores 1..=10 (shuffled), two not analysed.
        let scores = [7., 3., 10., 1., 5., 9., 2., 8., 4., 6.];
        let mut shown: Vec<_> = (0..10)
            .map(|i| photo(i, Some(scores[i as usize])))
            .collect();
        shown.push(photo(10, None));
        shown.push(photo(11, None));
        let marks = suggest(&shown, 20, false);
        // 20 % of the 10 analysed (not of the 12 shown): scores 1 and 2, in display order.
        assert_eq!(marks.suggested, [PhotoId(3), PhotoId(6)]);
        assert_eq!(
            marks.by_id[&3].badge(Kinds::default()),
            Some(Badge::Score(1.))
        );
        assert_eq!(marks.by_id[&3].tip, "Low score 1.0");
        assert!(!marks.by_id.contains_key(&10));
        assert_eq!(suggest(&shown, 50, false).suggested.len(), 5);
        assert_eq!(suggest(&shown, 5, false).suggested.len(), 0);
        assert_eq!(suggest(&shown, 0, false), Marks::default());
        assert_eq!(suggest(&[], 20, false), Marks::default());
    }

    #[test]
    fn off_topic_photos_are_suggested_and_keep_their_badge_at_zero() {
        let mut shown: Vec<_> = (0..4).map(|i| photo(i, Some(5. + i as f32))).collect();
        shown[2].1.off_topic = Some(OFF_TOPIC_Z + 0.1);
        shown[3].1.off_topic = Some(OFF_TOPIC_Z);
        let marks = suggest(&shown, 25, false);
        // The lowest score (id 0) and the off-topic photo (id 2); exactly at the bar is not.
        assert_eq!(marks.suggested, [PhotoId(0), PhotoId(2)]);
        assert_eq!(
            marks.by_id[&2].badge(Kinds::default()),
            Some(Badge::OffTopic)
        );
        assert!(!marks.by_id.contains_key(&3));
        let none = suggest(&shown, 0, false);
        assert!(none.suggested.is_empty());
        assert!(!none.by_id[&2].suggested);
        assert_eq!(
            none.by_id[&2].badge(Kinds::default()),
            Some(Badge::OffTopic)
        );
    }

    #[test]
    fn bursts_are_marked_and_suggested_only_when_allowed() {
        let burst = Some(Burst { group: 0, size: 3 });
        let mut shown: Vec<_> = [6., 8., 7., 9.]
            .iter()
            .enumerate()
            .map(|(i, &s)| photo(i as u64, Some(s)))
            .collect();
        for photo in &mut shown[..3] {
            photo.1.burst = burst;
        }
        let marked = suggest(&shown, 5, false);
        assert!(marked.suggested.is_empty());
        for id in 0..3 {
            let mark = &marked.by_id[&id];
            assert_eq!(mark.badge(Kinds::default()), Some(Badge::Burst));
            assert_eq!(mark.tip, "Burst: one of 3 similar shots taken together");
        }
        let suggested = suggest(&shown, 5, true);
        // The best of the burst (id 1, 8.0) stays; the two others are suggested.
        assert_eq!(suggested.suggested, [PhotoId(0), PhotoId(2)]);
        assert_eq!(suggested.by_id[&0].tip, "Burst: a better shot scores 8.0");
        assert_eq!(
            suggested.by_id[&0].badge(Kinds::default()),
            Some(Badge::Score(6.))
        );
    }

    #[test]
    fn hidden_badges_fall_back_to_the_next_kind_and_keep_the_outline() {
        let burst = Some(Burst { group: 0, size: 2 });
        let mut shown: Vec<_> = (0..4).map(|i| photo(i, Some(1. + i as f32))).collect();
        shown[0].1.burst = burst;
        shown[1].1.burst = burst;
        shown[0].1.off_topic = Some(OFF_TOPIC_Z + 1.);
        let marks = suggest(&shown, 25, false);
        let first = &marks.by_id[&0];
        assert_eq!(
            first.badges,
            [Badge::OffTopic, Badge::Score(1.), Badge::Burst]
        );
        assert!(first.kinds.low && first.kinds.off_topic && first.kinds.burst);
        let hide = |kinds: &[Kind]| {
            let mut hidden = Kinds::default();
            for &k in kinds {
                hidden.set(k, true);
            }
            hidden
        };
        assert_eq!(first.badge(hide(&[Kind::OffTopic])), Some(Badge::Score(1.)));
        assert_eq!(
            first.badge(hide(&[Kind::OffTopic, Kind::Low])),
            Some(Badge::Burst)
        );
        assert_eq!(first.badge(hide(&Kind::ALL)), None);
        // Still suggested: the outline and "Reject suggested" do not depend on the badges.
        assert!(first.suggested);
        // A burst photo that is not low never shows a score badge.
        let second = &marks.by_id[&1];
        assert_eq!(second.badges, [Badge::Burst]);
        assert!(!second.kinds.low && second.kinds.burst);
    }

    #[test]
    fn repeated_ids_count_once() {
        let shown = vec![photo(1, Some(1.)), photo(1, Some(1.)), photo(2, Some(9.))];
        let marks = suggest(&shown, 50, false);
        assert_eq!(marks.suggested, [PhotoId(1)]);
    }

    #[test]
    fn reject_sends_only_suggested_photos_still_shown_once() {
        let suggested = [PhotoId(4), PhotoId(1), PhotoId(9), PhotoId(1)];
        // 9 was sorted out meanwhile; 1 is shown twice by mistake.
        let shown = [PhotoId(1), PhotoId(2), PhotoId(4), PhotoId(1)];
        assert_eq!(reject_ids(&suggested, &shown), [PhotoId(1), PhotoId(4)]);
        assert!(reject_ids(&[], &shown).is_empty());
        assert!(reject_ids(&suggested, &[]).is_empty());
    }

    #[test]
    fn stored_percentages_snap_to_the_slider() {
        assert_eq!(clamp_percent(20), 20);
        assert_eq!(clamp_percent(23), 20);
        assert_eq!(clamp_percent(500), PERCENT_MAX);
        assert_eq!(clamp_percent(0), 0);
        assert_eq!(PERCENT_DEFAULT % PERCENT_STEP, 0);
    }

    #[test]
    fn tips_have_no_long_dash() {
        let mut shown = vec![photo(0, Some(1.)), photo(1, Some(2.))];
        shown[0].1.off_topic = Some(9.);
        shown[0].1.burst = Some(Burst { group: 0, size: 2 });
        shown[1].1.burst = Some(Burst { group: 0, size: 2 });
        for burst_suggest in [false, true] {
            for mark in suggest(&shown, 50, burst_suggest).by_id.values() {
                assert!(!mark.tip.contains('\u{2014}') && !mark.tip.contains(" \u{2013} ")); // dash-ok
            }
        }
    }
}
