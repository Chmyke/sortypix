//! ROLE: Sortypix AI only. Which photos the grid shows, in which order: pure, no egui, no model.
//! OWNER: `Ai::arrange` feeds it the facts of the folder in capture order (the worker's order).
//! INVARIANTS: The result is a list of distinct indices into the facts. Photos not analysed yet
//! never match a search, and follow the analysed ones in capture order in the AI orders. A burst
//! is shown whole or not at all, contiguous in the score, coverage, match and burst orders (the
//! capture orders keep it as it was shot). "Bursts" shows only the photos of a burst.
use std::collections::{HashMap, HashSet};

use sortypix_ai::{Likely, SEARCH_MIN};
use sortypix_app::order::FileOrder;

/// What the analysis knows of one photo of the folder (all `None` until it is analysed).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct Facts {
    /// Aesthetic score (LAION scale, about 1 to 10).
    pub score: Option<f32>,
    /// Off-topic score (robust z) from the latest folder analysis.
    pub off_topic: Option<f32>,
    /// The burst the photo belongs to, if any.
    pub burst: Option<Burst>,
    /// Rank in the coverage order ("Most varied first"), off-topic photos last.
    pub coverage: Option<usize>,
    /// Cosine with the current search query, when there is one.
    pub similarity: Option<f32>,
    /// Likely reason of a low score, from the latest folder analysis.
    pub reason: Option<Likely>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Burst {
    /// Index of the burst in the folder analysis.
    pub group: usize,
    /// Photos in the burst.
    pub size: usize,
}

/// The order menu, outside a search (and inside one when "Best match" is not chosen).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Order {
    Oldest,
    Newest,
    /// File name, size: shared with Sortypix ([`FileOrder`]), ranked by [`arrange_ranked`].
    Name,
    Largest,
    Smallest,
    LowestScore,
    HighestScore,
    MostVaried,
    /// Only the bursts, one after the other, each in capture order, scores ignored.
    Bursts,
}

impl Order {
    pub(crate) const ALL: [Order; 9] = [
        Order::Oldest,
        Order::Newest,
        Order::Name,
        Order::Largest,
        Order::Smallest,
        Order::LowestScore,
        Order::HighestScore,
        Order::MostVaried,
        Order::Bursts,
    ];

    pub(crate) fn label(self) -> &'static str {
        match self.file() {
            Some(file) => file.label(),
            None => match self {
                Order::LowestScore => "Lowest score first",
                Order::HighestScore => "Highest score first",
                Order::MostVaried => "Most varied first",
                _ => "Bursts",
            },
        }
    }

    /// Orders that need the analysis (off while the model is unavailable).
    pub(crate) fn needs_model(self) -> bool {
        matches!(
            self,
            Order::LowestScore | Order::HighestScore | Order::MostVaried | Order::Bursts
        )
    }

    /// The file order this is, shared with Sortypix; `None` for the AI orders.
    pub(crate) fn file(self) -> Option<FileOrder> {
        Some(match self {
            Order::Oldest => FileOrder::Oldest,
            Order::Newest => FileOrder::Newest,
            Order::Name => FileOrder::Name,
            Order::Largest => FileOrder::Largest,
            Order::Smallest => FileOrder::Smallest,
            _ => return None,
        })
    }

    pub(crate) fn from_file(file: FileOrder) -> Self {
        match file {
            FileOrder::Oldest => Order::Oldest,
            FileOrder::Newest => Order::Newest,
            FileOrder::Name => Order::Name,
            FileOrder::Largest => Order::Largest,
            FileOrder::Smallest => Order::Smallest,
        }
    }

    /// Ranked by name or size, which `arrange` cannot know from the facts.
    pub(crate) fn ranked(self) -> bool {
        matches!(self, Order::Name | Order::Largest | Order::Smallest)
    }
}

/// How the grid is arranged.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Arrangement {
    pub order: Order,
    /// A search query is applied: only matching photos are shown.
    pub searching: bool,
    /// Inside a search, ranked by similarity ("Best match") rather than by `order`.
    pub best_match: bool,
}

/// Indices of `facts` (capture order) to show, in display order.
///
/// A search shows the matching photos with the whole burst of each. Outside the capture orders,
/// a burst stays together, in capture order, at the place of its best placed photo; "Bursts"
/// keeps only the bursts, in the capture order of their first photo.
pub(crate) fn arrange(facts: &[Facts], how: Arrangement) -> Vec<usize> {
    let mut shown: Vec<usize> = if how.searching {
        let matches = |i: usize| facts[i].similarity.is_some_and(|s| s >= SEARCH_MIN);
        let bursts: HashSet<usize> = (0..facts.len())
            .filter(|&i| matches(i))
            .filter_map(|i| facts[i].burst.map(|b| b.group))
            .collect();
        (0..facts.len())
            .filter(|&i| matches(i) || facts[i].burst.is_some_and(|b| bursts.contains(&b.group)))
            .collect()
    } else {
        (0..facts.len()).collect()
    };
    if how.searching && how.best_match {
        shown.sort_by(|&a, &b| {
            let (sa, sb) = (facts[a].similarity, facts[b].similarity);
            sb.unwrap_or(f32::MIN)
                .total_cmp(&sa.unwrap_or(f32::MIN))
                .then(a.cmp(&b))
        });
        return together(facts, &shown);
    }
    match how.order {
        // The ranked orders start from capture order: see `arrange_ranked`.
        Order::Oldest | Order::Name | Order::Largest | Order::Smallest => return shown,
        Order::Newest => {
            shown.reverse();
            return shown;
        }
        // Analysed photos by the key, then the others in capture order (stable sort).
        Order::LowestScore => shown.sort_by(|&a, &b| {
            let (ka, kb) = (facts[a].score, facts[b].score);
            ka.is_none()
                .cmp(&kb.is_none())
                .then(ka.unwrap_or(0.).total_cmp(&kb.unwrap_or(0.)))
        }),
        Order::HighestScore => shown.sort_by(|&a, &b| {
            let (ka, kb) = (facts[a].score, facts[b].score);
            ka.is_none()
                .cmp(&kb.is_none())
                .then(kb.unwrap_or(0.).total_cmp(&ka.unwrap_or(0.)))
        }),
        Order::MostVaried => shown.sort_by_key(|&i| facts[i].coverage.unwrap_or(usize::MAX)),
        Order::Bursts => shown.retain(|&i| facts[i].burst.is_some()),
    }
    together(facts, &shown)
}

/// `arrange` for the orders by name or size: `rank[i]` is the place of photo `i` in that order.
/// A burst stays together, at the place of its best placed photo, as in the score orders.
pub(crate) fn arrange_ranked(facts: &[Facts], how: Arrangement, rank: &[usize]) -> Vec<usize> {
    let mut shown = arrange(facts, how);
    if how.order.ranked() && !(how.searching && how.best_match) {
        shown.sort_by_key(|&i| rank[i]);
        return together(facts, &shown);
    }
    shown
}

/// `ranked` with each burst gathered at its first member, in capture order (index order).
fn together(facts: &[Facts], ranked: &[usize]) -> Vec<usize> {
    let mut members: HashMap<usize, Vec<usize>> = HashMap::new();
    for &i in ranked {
        if let Some(b) = facts[i].burst {
            members.entry(b.group).or_default().push(i);
        }
    }
    let mut out = Vec::with_capacity(ranked.len());
    for &i in ranked {
        match facts[i].burst {
            None => out.push(i),
            Some(b) => {
                if let Some(mut group) = members.remove(&b.group) {
                    group.sort_unstable();
                    out.extend(group);
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn how(order: Order) -> Arrangement {
        Arrangement {
            order,
            searching: false,
            best_match: false,
        }
    }

    fn scored(scores: &[Option<f32>]) -> Vec<Facts> {
        scores
            .iter()
            .map(|&score| Facts {
                score,
                ..Facts::default()
            })
            .collect()
    }

    #[test]
    fn capture_orders_keep_every_photo() {
        let facts = scored(&[None, Some(2.), None]);
        assert_eq!(arrange(&facts, how(Order::Oldest)), [0, 1, 2]);
        assert_eq!(arrange(&facts, how(Order::Newest)), [2, 1, 0]);
        assert!(arrange(&[], how(Order::Newest)).is_empty());
    }

    #[test]
    fn lowest_score_first_puts_photos_not_analysed_after_in_capture_order() {
        let facts = scored(&[None, Some(5.), Some(3.), None, Some(3.), Some(4.)]);
        assert_eq!(arrange(&facts, how(Order::LowestScore)), [2, 4, 5, 1, 0, 3]);
    }

    #[test]
    fn highest_score_first_reverses_the_scores_not_the_photos_left_to_analyse() {
        let facts = scored(&[None, Some(5.), Some(3.), None, Some(3.), Some(4.)]);
        // Ties keep capture order; photos not analysed still follow in capture order.
        assert_eq!(
            arrange(&facts, how(Order::HighestScore)),
            [1, 5, 2, 4, 0, 3]
        );
        assert!(Order::HighestScore.needs_model());
    }

    #[test]
    fn most_varied_first_follows_coverage_then_capture_order() {
        let mut facts = vec![Facts::default(); 5];
        facts[3].coverage = Some(0);
        facts[1].coverage = Some(1);
        facts[4].coverage = Some(2);
        assert_eq!(arrange(&facts, how(Order::MostVaried)), [3, 1, 4, 0, 2]);
    }

    #[test]
    fn search_keeps_analysed_photos_at_or_above_the_threshold_best_first() {
        let sims = [
            Some(SEARCH_MIN + 0.01),
            None,
            Some(SEARCH_MIN - 0.01),
            Some(SEARCH_MIN),
            Some(SEARCH_MIN + 0.05),
        ];
        let mut facts: Vec<Facts> = sims
            .iter()
            .map(|&similarity| Facts {
                similarity,
                ..Facts::default()
            })
            .collect();
        facts[0].score = Some(1.);
        facts[3].score = Some(9.);
        facts[4].score = Some(5.);
        let mut search = Arrangement {
            order: Order::Oldest,
            searching: true,
            best_match: true,
        };
        assert_eq!(arrange(&facts, search), [4, 0, 3]);
        // Another order chosen during a search: the same photos, in that order.
        search.best_match = false;
        assert_eq!(arrange(&facts, search), [0, 3, 4]);
        search.order = Order::Newest;
        assert_eq!(arrange(&facts, search), [4, 3, 0]);
        search.order = Order::LowestScore;
        assert_eq!(arrange(&facts, search), [0, 4, 3]);
        search.order = Order::HighestScore;
        assert_eq!(arrange(&facts, search), [3, 4, 0]);
        // No query applied: everything, the search is ignored.
        assert_eq!(arrange(&facts, how(Order::Oldest)), [0, 1, 2, 3, 4]);
    }

    #[test]
    fn a_search_with_nothing_analysed_shows_nothing() {
        let facts = vec![Facts::default(); 3];
        let search = Arrangement {
            order: Order::Oldest,
            searching: true,
            best_match: true,
        };
        assert!(arrange(&facts, search).is_empty());
    }

    /// Five photos in capture order, scores 5, 6, 2, 7, 4; photos 1, 2 and 3 form a burst.
    fn with_burst() -> Vec<Facts> {
        let mut facts = scored(&[Some(5.), Some(6.), Some(2.), Some(7.), Some(4.)]);
        for f in &mut facts[1..=3] {
            f.burst = Some(Burst { group: 0, size: 3 });
        }
        facts
    }

    #[test]
    fn score_orders_keep_a_burst_together_in_capture_order() {
        let facts = with_burst();
        // The burst takes the place of its lowest photo (2), then of its best one (7).
        assert_eq!(arrange(&facts, how(Order::LowestScore)), [1, 2, 3, 4, 0]);
        assert_eq!(arrange(&facts, how(Order::HighestScore)), [1, 2, 3, 0, 4]);
    }

    #[test]
    fn most_varied_first_places_a_burst_at_its_best_ranked_photo() {
        let mut facts = with_burst();
        for (i, rank) in [3, 4, 2, 0, 1].into_iter().enumerate() {
            facts[i].coverage = Some(rank);
        }
        assert_eq!(arrange(&facts, how(Order::MostVaried)), [1, 2, 3, 4, 0]);
    }

    #[test]
    fn capture_orders_are_unchanged_by_bursts() {
        let facts = with_burst();
        assert_eq!(arrange(&facts, how(Order::Oldest)), [0, 1, 2, 3, 4]);
        assert_eq!(arrange(&facts, how(Order::Newest)), [4, 3, 2, 1, 0]);
    }

    #[test]
    fn a_search_brings_the_whole_burst_of_a_match() {
        let mut facts = with_burst();
        let sims = [0.30, 0.10, 0.20, 0.10, 0.25];
        for (f, s) in facts.iter_mut().zip(sims) {
            f.similarity = Some(s);
        }
        let mut search = Arrangement {
            order: Order::Oldest,
            searching: true,
            best_match: true,
        };
        // Photo 2 matches (0.20), its sisters 1 and 3 (0.10) come with it, at its place.
        assert_eq!(arrange(&facts, search), [0, 4, 1, 2, 3]);
        search.best_match = false;
        assert_eq!(arrange(&facts, search), [0, 1, 2, 3, 4]);
        search.order = Order::LowestScore;
        assert_eq!(arrange(&facts, search), [1, 2, 3, 4, 0]);
        // No member of the burst matches: none is shown.
        facts[2].similarity = Some(0.10);
        assert_eq!(arrange(&facts, search), [4, 0]);
    }

    #[test]
    fn bursts_shows_only_the_bursts_in_capture_order_whatever_the_scores() {
        let mut facts = with_burst();
        facts.push(Facts {
            score: Some(1.),
            burst: Some(Burst { group: 1, size: 2 }),
            ..Facts::default()
        });
        facts[0].burst = Some(Burst { group: 1, size: 2 });
        // Burst 1 starts first (photo 0), so it comes first, whole, before burst 0.
        assert_eq!(arrange(&facts, how(Order::Bursts)), [0, 5, 1, 2, 3]);
        assert!(arrange(&scored(&[Some(1.), Some(2.)]), how(Order::Bursts)).is_empty());
        assert!(Order::Bursts.needs_model());
    }

    #[test]
    fn every_order_keeps_each_burst_contiguous_and_every_photo_once() {
        // Two bursts interleaved with single photos, scores and ranks scattered.
        let scores = [3., 9., 1., 8., 5., 2., 7., 6.];
        let groups = [
            None,
            Some(0),
            Some(0),
            None,
            Some(1),
            Some(1),
            Some(1),
            None,
        ];
        let mut facts = Vec::new();
        for (i, (&s, &g)) in scores.iter().zip(&groups).enumerate() {
            facts.push(Facts {
                score: Some(s),
                coverage: Some((i * 5) % 8),
                burst: g.map(|group| Burst {
                    group,
                    size: 2 + group,
                }),
                ..Facts::default()
            });
        }
        // A rank that scatters the bursts, as a name or a size order could.
        let rank = [4, 0, 7, 2, 6, 1, 5, 3];
        for order in Order::ALL {
            let shown = arrange_ranked(&facts, how(order), &rank);
            let mut sorted = shown.clone();
            sorted.sort_unstable();
            let expected: Vec<usize> = (0..8)
                .filter(|&i| order != Order::Bursts || groups[i].is_some())
                .collect();
            assert_eq!(sorted, expected, "{order:?}");
            for group in 0..2 {
                let at: Vec<usize> = (0..shown.len())
                    .filter(|&p| groups[shown[p]] == Some(group))
                    .collect();
                assert_eq!(at.last().unwrap() - at[0] + 1, at.len(), "{order:?}");
                let members: Vec<usize> = at.iter().map(|&p| shown[p]).collect();
                if order != Order::Newest {
                    assert!(members.windows(2).all(|w| w[0] < w[1]), "{order:?}");
                }
            }
        }
    }

    #[test]
    fn name_and_size_orders_follow_the_rank_with_bursts_at_their_best_place() {
        let facts = with_burst();
        // Photo 3 of the burst ranks first: the whole burst comes first, in capture order.
        let rank = [1, 4, 2, 0, 3];
        for order in [Order::Name, Order::Largest, Order::Smallest] {
            assert_eq!(arrange_ranked(&facts, how(order), &rank), [1, 2, 3, 0, 4]);
        }
        // The other orders ignore the rank.
        assert_eq!(
            arrange_ranked(&facts, how(Order::Oldest), &rank),
            arrange(&facts, how(Order::Oldest))
        );
        for file in FileOrder::ALL {
            assert_eq!(Order::from_file(file).file(), Some(file));
        }
    }

    #[test]
    fn labels_are_plain_english_without_long_dashes() {
        for order in Order::ALL {
            assert!(!order.label().contains('\u{2014}') && !order.label().contains('\u{2013}')); // dash-ok
        }
        assert!(Order::LowestScore.needs_model() && !Order::Newest.needs_model());
    }
}
