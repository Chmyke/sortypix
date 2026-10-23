//! Probable reason for a low score, estimated with paired CLIP text probes.
//!
//! The aesthetic head (LAION) gives a number, not a reason. Each probe pairs a good and a bad
//! description ("a sharp photo" / "a blurry photo"); a photo leans to the bad side by
//! `cos(image, bad) - cos(image, good)`. That lean means little on its own (a night street is
//! "dark" for CLIP), so it is compared to the rest of the folder: a robust z (median and MAD
//! over the folder's photos). The photo's likely reason is the probe it leans to most, shown
//! only when that z reaches [`REASON_MIN`] and the probe was recognised on its known
//! degradation by `ai_quality` ([`Reason::recognised`]); otherwise there is no clear reason.
//! It is an estimate: the interface says "Likely reason", never the cause.
use crate::analysis::{cosine, median_of};
use crate::engine::Embedding;
use crate::error::AiError;

/// Smallest relative lean (robust z over the folder) for a reason to be shown. Chosen by the
/// rule of `ai_quality` (the smallest value of its sweep at which at most 10 % of the clean
/// corpus photos get a reason); the probe fails when this constant is another one.
pub const REASON_MIN: f32 = 2.0;
/// Floor of the folder spread of a lean (cosine units): a folder of near identical photos
/// would otherwise turn noise into reasons.
pub const REASON_SPREAD_FLOOR: f32 = 0.002;

/// What may have pulled a score down.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Reason {
    Blurry,
    Dark,
    Overexposed,
    Noisy,
    Compressed,
}

/// Number of probes, the length of a lean vector.
pub const REASONS: usize = 5;

impl Reason {
    /// Probe order, the index space of lean vectors.
    pub const ALL: [Reason; REASONS] = [
        Reason::Blurry,
        Reason::Dark,
        Reason::Overexposed,
        Reason::Noisy,
        Reason::Compressed,
    ];

    /// The good and the bad description of the probe.
    pub fn prompts(self) -> (&'static str, &'static str) {
        match self {
            Reason::Blurry => ("a sharp photo in focus", "a blurry photo out of focus"),
            Reason::Dark => ("a well exposed photo", "a dark underexposed photo"),
            Reason::Overexposed => ("a well exposed photo", "an overexposed washed out photo"),
            Reason::Noisy => ("a clean photo", "a noisy grainy photo"),
            Reason::Compressed => (
                "a high quality photo",
                "a low quality photo with heavy jpeg compression artifacts",
            ),
        }
    }

    /// Plain words for the interface (after "Likely reason: ").
    pub fn label(self) -> &'static str {
        match self {
            Reason::Blurry => "blurry or out of focus",
            Reason::Dark => "too dark",
            Reason::Overexposed => "too bright, washed out",
            Reason::Noisy => "noisy or grainy",
            Reason::Compressed => "low quality, heavily compressed",
        }
    }

    /// The probe found its known degradation in `ai_quality` (blur, -2 EV, +2 EV, noise,
    /// JPEG q10) by the rule written there. An unrecognised probe still competes, but is never
    /// shown: its photos get "No clear reason". The probe fails when this disagrees.
    ///
    /// Measured 2026-10-23 (int8, edge 448, 120 photos): blur is found (shown right 0.950,
    /// precision 0.950); JPEG q10 (0.058), noise sigma 25 (0.033), -2 EV (0.142) and +2 EV
    /// (0.258) are not, so their probes are never shown.
    pub fn recognised(self) -> bool {
        match self {
            Reason::Blurry => true,
            Reason::Dark | Reason::Overexposed | Reason::Noisy | Reason::Compressed => false,
        }
    }

    pub fn index(self) -> usize {
        Reason::ALL.iter().position(|&r| r == self).unwrap_or(0)
    }
}

/// The estimate for one photo.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Likely {
    Clear(Reason),
    /// Nothing stands out enough, or only a probe that was not recognised.
    Unclear,
}

/// The text embeddings of the probes (computed once, off the interface thread).
#[derive(Clone, Debug)]
pub struct ReasonProbes {
    /// (good, bad) in [`Reason::ALL`] order.
    pairs: Vec<(Embedding, Embedding)>,
}

impl ReasonProbes {
    /// Embeds the descriptions with `embed_text` (the text model).
    pub fn embed(
        mut embed_text: impl FnMut(&str) -> Result<Embedding, AiError>,
    ) -> Result<ReasonProbes, AiError> {
        let mut pairs = Vec::with_capacity(REASONS);
        for reason in Reason::ALL {
            let (good, bad) = reason.prompts();
            pairs.push((embed_text(good)?, embed_text(bad)?));
        }
        Ok(ReasonProbes { pairs })
    }

    /// How far `image` leans to the bad side of each probe (cosine units).
    pub fn lean(&self, image: &Embedding) -> [f32; REASONS] {
        let mut out = [0f32; REASONS];
        for (slot, (good, bad)) in out.iter_mut().zip(&self.pairs) {
            *slot = cosine(image, bad) - cosine(image, good);
        }
        out
    }
}

/// Median and spread of each probe's lean over a folder.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FolderLean {
    median: [f32; REASONS],
    spread: [f32; REASONS],
}

impl FolderLean {
    /// Over `leans` (one per photo of the folder); an empty folder gives zeros and the floor.
    pub fn of(leans: &[[f32; REASONS]]) -> FolderLean {
        let mut median = [0f32; REASONS];
        let mut spread = [REASON_SPREAD_FLOOR; REASONS];
        if leans.is_empty() {
            return FolderLean { median, spread };
        }
        for k in 0..REASONS {
            let column: Vec<f32> = leans.iter().map(|l| l[k]).collect();
            let m = median_of(&column);
            let deviations: Vec<f32> = column.iter().map(|v| (v - m).abs()).collect();
            median[k] = m;
            spread[k] = (1.4826 * median_of(&deviations)).max(REASON_SPREAD_FLOOR);
        }
        FolderLean { median, spread }
    }

    /// Robust z of each probe's lean of one photo against the folder.
    pub fn relative(&self, lean: &[f32; REASONS]) -> [f32; REASONS] {
        let mut z = [0f32; REASONS];
        for k in 0..REASONS {
            z[k] = (lean[k] - self.median[k]) / self.spread[k];
        }
        z
    }
}

/// The probe a photo leans to most, with its relative lean (ties: first in probe order).
pub fn strongest(z: &[f32; REASONS]) -> (Reason, f32) {
    let mut best = (Reason::ALL[0], z[0]);
    for (k, &value) in z.iter().enumerate().skip(1) {
        if value > best.1 {
            best = (Reason::ALL[k], value);
        }
    }
    best
}

/// The likely reason given the relative leans: the strongest probe when it reaches
/// [`REASON_MIN`] and is recognised, else [`Likely::Unclear`].
pub fn likely(z: &[f32; REASONS]) -> Likely {
    likely_with(z, REASON_MIN, Reason::recognised)
}

/// [`likely`] with another bar and set of recognised probes (the calibration sweeps them).
pub fn likely_with(z: &[f32; REASONS], min: f32, recognised: impl Fn(Reason) -> bool) -> Likely {
    let (reason, value) = strongest(z);
    if value >= min && recognised(reason) {
        Likely::Clear(reason)
    } else {
        Likely::Unclear
    }
}

/// Likely reasons of every photo of a folder, against the folder itself.
pub fn folder_reasons(probes: &ReasonProbes, items: &[Embedding]) -> Vec<Likely> {
    let leans: Vec<[f32; REASONS]> = items.iter().map(|e| probes.lean(e)).collect();
    let folder = FolderLean::of(&leans);
    leans
        .iter()
        .map(|lean| likely(&folder.relative(lean)))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::EMBED_DIM;

    fn axis(i: usize) -> Embedding {
        let mut e = [0f32; EMBED_DIM];
        e[i] = 1.0;
        e
    }

    #[test]
    fn the_strongest_recognised_lean_above_the_bar_is_the_reason() {
        let mut z = [0.5, 1.0, -1.0, 0.0, 0.2];
        assert_eq!(likely_with(&z, 2.0, |_| true), Likely::Unclear);
        z[3] = 2.5;
        assert_eq!(strongest(&z), (Reason::Noisy, 2.5));
        assert_eq!(likely_with(&z, 2.0, |_| true), Likely::Clear(Reason::Noisy));
        // Exactly at the bar counts.
        assert_eq!(likely_with(&z, 2.5, |_| true), Likely::Clear(Reason::Noisy));
        // A strongest probe that was not recognised is never replaced by the runner-up.
        z[0] = 2.2;
        assert_eq!(
            likely_with(&z, 2.0, |r| r != Reason::Noisy),
            Likely::Unclear
        );
    }

    #[test]
    fn leans_are_relative_to_the_folder() {
        // Probe 0 compares axis 1 (bad) with axis 0 (good); the others are neutral.
        let mut pairs = vec![(axis(0), axis(1))];
        for k in 1..REASONS {
            pairs.push((axis(10 + k), axis(10 + k)));
        }
        let probes = ReasonProbes { pairs };
        // Most photos lean a little to the bad side; one leans far more.
        let mut items: Vec<Embedding> = (0..9)
            .map(|i| {
                let mut e = [0f32; EMBED_DIM];
                e[0] = 0.8;
                e[1] = 0.2 + 0.01 * i as f32;
                e
            })
            .collect();
        let mut odd = [0f32; EMBED_DIM];
        odd[0] = 0.2;
        odd[1] = 0.8;
        items.push(odd);
        let reasons = folder_reasons(&probes, &items);
        assert_eq!(reasons.len(), 10);
        assert_eq!(
            reasons[9],
            likely(&{
                let leans: Vec<_> = items.iter().map(|e| probes.lean(e)).collect();
                FolderLean::of(&leans).relative(&leans[9])
            })
        );
        let leans: Vec<_> = items.iter().map(|e| probes.lean(e)).collect();
        let z = FolderLean::of(&leans).relative(&leans[9]);
        assert_eq!(strongest(&z).0, Reason::Blurry);
        assert!(z[0] > 5.0);
        // A typical photo of the folder has no clear reason, whatever its absolute lean.
        assert_eq!(reasons[4], Likely::Unclear);
    }

    #[test]
    fn a_folder_of_identical_photos_has_no_reason() {
        let probes = ReasonProbes {
            pairs: (0..REASONS)
                .map(|k| (axis(2 * k), axis(2 * k + 1)))
                .collect(),
        };
        let items = vec![axis(1); 4];
        assert!(
            folder_reasons(&probes, &items)
                .iter()
                .all(|r| *r == Likely::Unclear)
        );
        assert!(folder_reasons(&probes, &[]).is_empty());
        let empty = FolderLean::of(&[]).relative(&[1.0; REASONS]);
        assert!(
            empty
                .iter()
                .all(|z| (z - 1.0 / REASON_SPREAD_FLOOR).abs() < 0.01)
        );
    }

    #[test]
    fn labels_are_plain_without_long_dashes() {
        for reason in Reason::ALL {
            let label = reason.label();
            assert!(!label.contains('\u{2014}') && !label.contains(" \u{2013} ")); // dash-ok
            assert_eq!(Reason::ALL[reason.index()], reason);
        }
    }
}
