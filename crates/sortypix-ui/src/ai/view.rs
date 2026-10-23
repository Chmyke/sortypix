//! ROLE: Sortypix AI only. Which photos the grid shows, in which order: pure, no egui, no model.
//! OWNER: `Ai::arrange` feeds it the facts of the folder in capture order (the worker's order).
//! INVARIANTS: The result is a list of distinct indices into the facts. Photos not analysed yet
//! never match a search, and follow the analysed ones in capture order in the AI orders.
use sortypix_ai::{Likely, SEARCH_MIN};

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
    LowestScore,
    HighestScore,
    MostVaried,
}

impl Order {
    pub(crate) const ALL: [Order; 5] = [
        Order::Oldest,
        Order::Newest,
        Order::LowestScore,
        Order::HighestScore,
        Order::MostVaried,
    ];

    pub(crate) fn label(self) -> &'static str {
        match self {
            Order::Oldest => "Oldest first",
            Order::Newest => "Newest first",
            Order::LowestScore => "Lowest score first",
            Order::HighestScore => "Highest score first",
            Order::MostVaried => "Most varied first",
        }
    }

    /// Orders that need the analysis (off while the model is unavailable).
    pub(crate) fn needs_model(self) -> bool {
        matches!(
            self,
            Order::LowestScore | Order::HighestScore | Order::MostVaried
        )
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
pub(crate) fn arrange(facts: &[Facts], how: Arrangement) -> Vec<usize> {
    let mut shown: Vec<usize> = if how.searching {
        (0..facts.len())
            .filter(|&i| facts[i].similarity.is_some_and(|s| s >= SEARCH_MIN))
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
        return shown;
    }
    match how.order {
        Order::Oldest => {}
        Order::Newest => shown.reverse(),
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
    }
    shown
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

    #[test]
    fn labels_are_plain_english_without_long_dashes() {
        for order in Order::ALL {
            assert!(!order.label().contains('\u{2014}') && !order.label().contains('\u{2013}')); // dash-ok
        }
        assert!(Order::LowestScore.needs_model() && !Order::Newest.needs_model());
    }
}
