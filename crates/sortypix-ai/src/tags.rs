//! Tags: the words of [`crate::vocabulary`] an image most likely shows.
//!
//! An image is first sorted by its style: CLIP's zero-shot competition among the Style words
//! (softmax of [`LOGIT_SCALE`] times the cosine). When the photo-like styles ([`PHOTO_STYLES`])
//! hold at least [`Selection::photo_min`] of it, the image is a **photo**; otherwise it is
//! drawn, rendered or generated (**art**). Measured 2026-11-13: real photos hold a median 0.36
//! to 0.40 of it, generated images 0.02 to 0.03 (style words compete among themselves, so even
//! a plain photo leaves much to "fantasy" or "digital art").
//! - A photo carries **learned** words: logistic outputs trained offline on human-verified Open
//!   Images labels over the same CLIP embeddings (`scripts/ai/tags/train_head.py`, classes in
//!   [`crate::tag_classes`]). Words are independent ("dog", "beach" and "sunset" at once); a word
//!   shows when its probability reaches its own threshold, chosen on the Open Images validation
//!   split for a precision target. At most [`Selection::photo_max`].
//! - Art carries CLIP's **zero-shot** words: the image's cosine with every word's text
//!   embedding, times the logit scale, turned into a probability over the whole vocabulary
//!   (softmax), from [`Params::min`], at most [`Params::max`]. Open Images is photographs: its
//!   head was measured wrong too often on drawings and renders.
//!
//! The likeliest come first. Tags are absolute, never relative to the folder: a folder of cats
//! says "cat" on every photo.
//!
//! Both parts are generated and built into the executable: the text embeddings
//! (`data/tags-l14-int8.bin`, `ai_tags --write`, the int8 text model the search uses) and the head
//! (`data/tags-head-l14.bin`, `train_head.py`). A file made for another vocabulary or model is
//! refused: no tags, never a panic. Measured by `examples/ai_tags.rs`
//! (`docs/design/Étiquettes apprises (2026-11-13).md`).
use crate::analysis::{dot, dot4};
use crate::cache::MODEL_LABEL;
use crate::engine::{EMBED_DIM, Embedding};
use crate::error::AiError;
use crate::vocabulary::{Group, VOCABULARY, vocabulary_hash};

/// CLIP ViT-L/14's learned logit scale (exp of its stored temperature, clamped at 100 in
/// training): fixed by the model, not tuned here.
pub const LOGIT_SCALE: f32 = 100.;

/// The zero-shot selection of art images.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Params {
    /// Smallest zero-shot probability.
    pub min: f32,
    /// Most tags, the likeliest first.
    pub max: usize,
}

/// How tags are selected; [`SELECTION`] is what the application uses.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Selection {
    /// Smallest share of the photo-like styles for an image to be a photo.
    pub photo_min: f32,
    /// Most learned tags on a photo.
    pub photo_max: usize,
    /// Zero-shot tags on art.
    pub art: Params,
}

/// Chosen by the rule of `ai_tags`; the probe fails when these are not the values it picks.
pub const SELECTION: Selection = Selection {
    photo_min: 0.2,
    photo_max: 4,
    art: Params { min: 0.17, max: 2 },
};

/// Style words that mean a photograph.
pub const PHOTO_STYLES: [&str; 6] = [
    "photo",
    "black and white",
    "sepia",
    "vintage",
    "film photo",
    "hdr",
];

const TEXT_MAGIC: [u8; 8] = *b"SPXTAGS\0";
const TEXT_VERSION: u32 = 2;
const HEAD_MAGIC: [u8; 8] = *b"SPXHEAD\0";
const HEAD_VERSION: u32 = 1;
const LABEL_LEN: usize = 32;
/// Magic, version, model label, vocabulary hash, count, dimension: both files.
const HEADER_LEN: usize = 8 + 4 + LABEL_LEN + 32 + 4 + 4;
/// One embedding, little-endian f32.
const ENTRY_LEN: usize = EMBED_DIM * 4;
/// One head entry: learned flag (u32), bias, threshold, then the weights.
const HEAD_ENTRY_LEN: usize = 12 + ENTRY_LEN;

/// The generated files.
static EMBEDDED_TEXT: &[u8] = include_bytes!("../data/tags-l14-int8.bin");
static EMBEDDED_HEAD: &[u8] = include_bytes!("../data/tags-head-l14.bin");

/// A word an image carries.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tag {
    /// Index in [`VOCABULARY`].
    pub word: u16,
    /// How likely the word is (learned: its own probability; zero-shot: its share among all
    /// words): ranks tags and matches.
    pub probability: f32,
}

impl Tag {
    pub fn name(&self) -> &'static str {
        VOCABULARY
            .get(usize::from(self.word))
            .map_or("", |w| w.name)
    }
}

/// A word's trained logistic output.
#[derive(Clone, Debug, PartialEq)]
pub struct Learned {
    pub weights: Embedding,
    pub bias: f32,
    /// Smallest probability that shows the word.
    pub threshold: f32,
}

fn sigmoid(z: f32) -> f32 {
    1. / (1. + (-z).exp())
}

/// `image` dotted with each column, four at a time (`dot4` reads the image once per four).
fn dots(image: &Embedding, columns: &[&[f32]]) -> Vec<f32> {
    let mut out = Vec::with_capacity(columns.len());
    let (quads, rest) = columns.as_chunks::<4>();
    for quad in quads {
        out.extend(dot4(image, *quad));
    }
    out.extend(rest.iter().map(|c| dot(image, c)));
    out
}

/// What the selection of one image reads.
#[derive(Clone, Debug, PartialEq)]
pub struct Scores {
    /// Share of the photo-like styles in the style competition.
    pub photo_share: f32,
    /// Learned words' probabilities (zero-shot words 0).
    pub learned: Vec<f32>,
    /// Every word's share of the zero-shot softmax over the whole vocabulary.
    pub zero_shot: Vec<f32>,
}

/// Text embeddings and learned outputs of the vocabulary.
#[derive(Clone, Debug)]
pub struct TagSet {
    embeddings: Vec<Embedding>,
    /// One per word: `None` for a word without a learned output.
    head: Vec<Option<Learned>>,
}

impl TagSet {
    /// The set built into the executable, checked against this vocabulary and model.
    pub fn embedded() -> Result<TagSet, AiError> {
        TagSet::new(
            text_from_bytes(EMBEDDED_TEXT)?,
            head_from_bytes(EMBEDDED_HEAD)?,
        )
    }

    /// One embedding and one head entry per word of [`VOCABULARY`], in its order.
    pub fn new(embeddings: Vec<Embedding>, head: Vec<Option<Learned>>) -> Result<TagSet, AiError> {
        if embeddings.len() != VOCABULARY.len() || head.len() != VOCABULARY.len() {
            return Err(AiError::Model(format!(
                "tags: {} embeddings and {} head entries for {} words",
                embeddings.len(),
                head.len(),
                VOCABULARY.len()
            )));
        }
        Ok(TagSet { embeddings, head })
    }

    pub fn embeddings(&self) -> &[Embedding] {
        &self.embeddings
    }

    pub fn head(&self) -> &[Option<Learned>] {
        &self.head
    }

    /// Everything a selection reads, every product computed (probes; the application calls
    /// [`TagSet::tags`], which computes only the branch it needs).
    pub fn scores(&self, image: &Embedding) -> Scores {
        let cosines = self.cosines(image);
        let style: Vec<f32> = style_words().map(|w| cosines[w]).collect();
        Scores {
            photo_share: photo_share(&style),
            learned: self.learned(image),
            zero_shot: zero_shot(&cosines),
        }
    }

    /// The image's tags as the application selects them ([`SELECTION`]): the style words
    /// first, then only the branch the image takes.
    pub fn tags(&self, image: &Embedding) -> Vec<Tag> {
        let style: Vec<&[f32]> = style_words()
            .map(|w| self.embeddings[w].as_slice())
            .collect();
        if photo_share(&dots(image, &style)) >= SELECTION.photo_min {
            pick_learned(&self.learned(image), &self.head, SELECTION.photo_max)
        } else {
            pick_zero_shot(&zero_shot(&self.cosines(image)), SELECTION.art)
        }
    }

    /// Tags of `scores` under `selection`.
    pub fn select(&self, scores: &Scores, selection: Selection) -> Vec<Tag> {
        if scores.photo_share >= selection.photo_min {
            pick_learned(&scores.learned, &self.head, selection.photo_max)
        } else {
            pick_zero_shot(&scores.zero_shot, selection.art)
        }
    }

    fn cosines(&self, image: &Embedding) -> Vec<f32> {
        let all: Vec<&[f32]> = self.embeddings.iter().map(|e| e.as_slice()).collect();
        dots(image, &all)
    }

    /// Learned words' probabilities (other words 0).
    fn learned(&self, image: &Embedding) -> Vec<f32> {
        let columns: Vec<&[f32]> = self
            .head
            .iter()
            .flatten()
            .map(|l| l.weights.as_slice())
            .collect();
        let mut z = dots(image, &columns).into_iter();
        self.head
            .iter()
            .map(|h| match h {
                Some(l) => z.next().map_or(0., |z| sigmoid(z + l.bias)),
                None => 0.,
            })
            .collect()
    }
}

fn style_words() -> impl Iterator<Item = usize> {
    VOCABULARY
        .iter()
        .enumerate()
        .filter(|(_, w)| w.group == Group::Style)
        .map(|(i, _)| i)
}

/// Share of the photo-like styles in the softmax over the style words (cosines in their order).
fn photo_share(style_cosines: &[f32]) -> f32 {
    let logits: Vec<f32> = style_cosines.iter().map(|c| LOGIT_SCALE * c).collect();
    style_words()
        .zip(softmax(&logits))
        .filter(|(w, _)| PHOTO_STYLES.contains(&VOCABULARY[*w].name))
        .map(|(_, p)| p)
        .sum()
}

fn zero_shot(cosines: &[f32]) -> Vec<f32> {
    let logits: Vec<f32> = cosines.iter().map(|c| LOGIT_SCALE * c).collect();
    softmax(&logits)
}

/// Likeliest first (ties: vocabulary order), at most `max`.
fn ranked(mut tags: Vec<Tag>, max: usize) -> Vec<Tag> {
    tags.sort_by(|a, b| {
        b.probability
            .total_cmp(&a.probability)
            .then(a.word.cmp(&b.word))
    });
    tags.truncate(max);
    tags
}

/// Learned words at their own threshold.
fn pick_learned(p: &[f32], head: &[Option<Learned>], max: usize) -> Vec<Tag> {
    let tags = p
        .iter()
        .zip(head)
        .enumerate()
        .filter_map(|(i, (p, h))| {
            h.as_ref().filter(|l| *p >= l.threshold).map(|_| Tag {
                word: i as u16,
                probability: *p,
            })
        })
        .collect();
    ranked(tags, max)
}

/// Words of zero-shot probability at least `params.min`.
fn pick_zero_shot(p: &[f32], params: Params) -> Vec<Tag> {
    let tags = p
        .iter()
        .enumerate()
        .filter(|(_, p)| **p >= params.min)
        .map(|(i, p)| Tag {
            word: i as u16,
            probability: *p,
        })
        .collect();
    ranked(tags, params.max)
}

/// The header both files share, checked; returns the bytes after it.
fn check_header<'a>(
    bytes: &'a [u8],
    magic: [u8; 8],
    version: u32,
    what: &str,
) -> Result<&'a [u8], AiError> {
    let fail = |m: &str| Err(AiError::Model(format!("tags ({what}): {m}")));
    if bytes.len() < HEADER_LEN || bytes[..8] != magic {
        return fail("not generated (see the ai_tags example and train_head.py)");
    }
    if u32_at(bytes, 8) != version {
        return fail("unknown version");
    }
    if bytes[12..12 + LABEL_LEN] != padded_label() {
        return fail("made for another model");
    }
    if bytes[12 + LABEL_LEN..12 + LABEL_LEN + 32] != vocabulary_hash() {
        return fail("made from another vocabulary (regenerate it)");
    }
    if u32_at(bytes, HEADER_LEN - 8) as usize != VOCABULARY.len()
        || u32_at(bytes, HEADER_LEN - 4) as usize != EMBED_DIM
    {
        return fail("wrong size");
    }
    Ok(&bytes[HEADER_LEN..])
}

fn u32_at(bytes: &[u8], at: usize) -> u32 {
    let mut b = [0u8; 4];
    b.copy_from_slice(&bytes[at..at + 4]);
    u32::from_le_bytes(b)
}

fn f32_at(bytes: &[u8], at: usize) -> f32 {
    f32::from_bits(u32_at(bytes, at))
}

fn embedding(bytes: &[u8]) -> Option<Embedding> {
    let mut e = [0f32; EMBED_DIM];
    for (v, c) in e.iter_mut().zip(bytes.as_chunks::<4>().0) {
        *v = f32::from_le_bytes(*c);
    }
    e.iter().all(|v| v.is_finite()).then_some(e)
}

fn header(magic: [u8; 8], version: u32, capacity: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(capacity);
    out.extend_from_slice(&magic);
    out.extend_from_slice(&version.to_le_bytes());
    out.extend_from_slice(&padded_label());
    out.extend_from_slice(&vocabulary_hash());
    out.extend_from_slice(&(VOCABULARY.len() as u32).to_le_bytes());
    out.extend_from_slice(&(EMBED_DIM as u32).to_le_bytes());
    out
}

/// Parses the text embeddings file ([`text_to_bytes`]).
pub fn text_from_bytes(bytes: &[u8]) -> Result<Vec<Embedding>, AiError> {
    let body = check_header(bytes, TEXT_MAGIC, TEXT_VERSION, "text")?;
    let bad = || AiError::Model("tags (text): wrong size or non-finite value".into());
    if body.len() != VOCABULARY.len() * ENTRY_LEN {
        return Err(bad());
    }
    body.chunks(ENTRY_LEN)
        .map(|c| embedding(c).ok_or_else(bad))
        .collect()
}

/// The text embeddings file: header, then each word's embedding (little-endian f32).
pub fn text_to_bytes(embeddings: &[Embedding]) -> Vec<u8> {
    let mut out = header(
        TEXT_MAGIC,
        TEXT_VERSION,
        HEADER_LEN + embeddings.len() * ENTRY_LEN,
    );
    for e in embeddings {
        for v in e {
            out.extend_from_slice(&v.to_le_bytes());
        }
    }
    out
}

/// Parses the head file: header, then per word a learned flag (u32, 0 or 1), the bias, the
/// threshold (a probability in 0..=1) and the weights (little-endian f32). Written by
/// `train_head.py`, which copies the header of the text file.
pub fn head_from_bytes(bytes: &[u8]) -> Result<Vec<Option<Learned>>, AiError> {
    let body = check_header(bytes, HEAD_MAGIC, HEAD_VERSION, "head")?;
    let bad = |m: &str| AiError::Model(format!("tags (head): {m}"));
    if body.len() != VOCABULARY.len() * HEAD_ENTRY_LEN {
        return Err(bad("wrong size"));
    }
    body.chunks(HEAD_ENTRY_LEN)
        .map(|c| match u32_at(c, 0) {
            0 => Ok(None),
            1 => {
                let bias = f32_at(c, 4);
                let threshold = f32_at(c, 8);
                let weights = embedding(&c[12..]).ok_or_else(|| bad("non-finite weight"))?;
                if !bias.is_finite() || !(0.0..=1.0).contains(&threshold) {
                    return Err(bad("bad bias or threshold"));
                }
                Ok(Some(Learned {
                    weights,
                    bias,
                    threshold,
                }))
            }
            _ => Err(bad("bad flag")),
        })
        .collect()
}

/// The head file of [`head_from_bytes`] (tests; `train_head.py` writes the real one).
pub fn head_to_bytes(head: &[Option<Learned>]) -> Vec<u8> {
    let mut out = header(
        HEAD_MAGIC,
        HEAD_VERSION,
        HEADER_LEN + head.len() * HEAD_ENTRY_LEN,
    );
    for entry in head {
        let (flag, bias, threshold, weights) = match entry {
            Some(l) => (1u32, l.bias, l.threshold, l.weights),
            None => (0, 0., 1., [0f32; EMBED_DIM]),
        };
        out.extend_from_slice(&flag.to_le_bytes());
        out.extend_from_slice(&bias.to_le_bytes());
        out.extend_from_slice(&threshold.to_le_bytes());
        for v in weights {
            out.extend_from_slice(&v.to_le_bytes());
        }
    }
    out
}

/// Probabilities from logits, computed from the largest so nothing overflows.
fn softmax(logits: &[f32]) -> Vec<f32> {
    let top = logits.iter().copied().fold(f32::NEG_INFINITY, f32::max);
    let exp: Vec<f32> = logits.iter().map(|l| (l - top).exp()).collect();
    let sum: f32 = exp.iter().sum();
    if !(sum.is_finite() && sum > 0.) {
        return vec![0.; logits.len()];
    }
    exp.iter().map(|e| e / sum).collect()
}

fn padded_label() -> [u8; LABEL_LEN] {
    let mut label = [0u8; LABEL_LEN];
    let bytes = MODEL_LABEL.as_bytes();
    let n = bytes.len().min(LABEL_LEN);
    label[..n].copy_from_slice(&bytes[..n]);
    label
}

#[cfg(test)]
mod tests {
    use super::*;

    fn axis(i: usize) -> Embedding {
        let mut e = [0f32; EMBED_DIM];
        e[i % EMBED_DIM] = 1.0;
        e
    }

    fn zero_shot_set() -> TagSet {
        let n = VOCABULARY.len();
        TagSet::new((0..n).map(axis).collect(), vec![None; n]).unwrap()
    }

    #[test]
    fn the_embedded_set_matches_this_vocabulary_and_model() {
        // Fails when a word changed and the files were not generated again.
        let set = TagSet::embedded().expect("embedded tags");
        assert_eq!(set.embeddings().len(), VOCABULARY.len());
        for e in set.embeddings() {
            let norm: f32 = e.iter().map(|v| v * v).sum::<f32>().sqrt();
            assert!((norm - 1.).abs() < 1e-3);
        }
        assert!(set.head().iter().any(Option::is_some), "no learned word");
    }

    #[test]
    fn bytes_round_trip_and_bad_files_are_refused() {
        let set = zero_shot_set();
        let bytes = text_to_bytes(set.embeddings());
        assert_eq!(text_from_bytes(&bytes).unwrap(), set.embeddings());
        assert!(text_from_bytes(&[]).is_err());
        assert!(text_from_bytes(&bytes[..bytes.len() - 1]).is_err());
        let mut other_vocabulary = bytes.clone();
        other_vocabulary[12 + LABEL_LEN] ^= 1;
        assert!(text_from_bytes(&other_vocabulary).is_err());
        let mut other_model = bytes.clone();
        other_model[12] ^= 1;
        assert!(text_from_bytes(&other_model).is_err());
        let mut old_version = bytes.clone();
        old_version[8] = 1;
        assert!(text_from_bytes(&old_version).is_err());
        let mut not_finite = bytes.clone();
        not_finite[HEADER_LEN..HEADER_LEN + 4].copy_from_slice(&f32::NAN.to_le_bytes());
        assert!(text_from_bytes(&not_finite).is_err());
        // The head's own magic: a text file is not a head.
        assert!(head_from_bytes(&bytes).is_err());
        assert!(TagSet::new(vec![], vec![]).is_err());
    }

    #[test]
    fn head_round_trips_and_refuses_bad_entries() {
        let mut head: Vec<Option<Learned>> = vec![None; VOCABULARY.len()];
        head[3] = Some(Learned {
            weights: axis(3),
            bias: -2.,
            threshold: 0.6,
        });
        let bytes = head_to_bytes(&head);
        assert_eq!(head_from_bytes(&bytes).unwrap(), head);
        let entry = HEADER_LEN + 3 * HEAD_ENTRY_LEN;
        let mut bad_threshold = bytes.clone();
        bad_threshold[entry + 8..entry + 12].copy_from_slice(&1.5f32.to_le_bytes());
        assert!(head_from_bytes(&bad_threshold).is_err());
        let mut bad_flag = bytes.clone();
        bad_flag[entry] = 7;
        assert!(head_from_bytes(&bad_flag).is_err());
        let mut nan_weight = bytes;
        nan_weight[entry + 12..entry + 16].copy_from_slice(&f32::NAN.to_le_bytes());
        assert!(head_from_bytes(&nan_weight).is_err());
    }

    #[test]
    fn zero_shot_probabilities_sum_to_one_and_the_likeliest_come_first() {
        // Close to words 2 and 1, a little to word 4, nothing else.
        let mut cosines = vec![0f32; VOCABULARY.len()];
        cosines[1] = 0.30;
        cosines[2] = 0.31;
        cosines[4] = 0.26;
        let p = zero_shot(&cosines);
        assert!((p.iter().sum::<f32>() - 1.).abs() < 1e-4);
        let words =
            |params| -> Vec<u16> { pick_zero_shot(&p, params).iter().map(|t| t.word).collect() };
        assert_eq!(words(Params { min: 0.0, max: 3 }), vec![2, 1, 4]);
        assert_eq!(words(Params { min: 0.2, max: 8 }), vec![2, 1]);
        assert_eq!(words(Params { min: 0.2, max: 1 }), vec![2]);
    }

    fn index(name: &str) -> usize {
        VOCABULARY.iter().position(|w| w.name == name).unwrap()
    }

    #[test]
    fn photos_get_learned_words_and_art_gets_zero_shot_words() {
        let n = VOCABULARY.len();
        // Text embeddings on their own axes (`axis`); the head reads axis 700.
        let mut head: Vec<Option<Learned>> = vec![None; n];
        for (w, threshold) in [(5, 0.7), (6, 0.95)] {
            let mut weights = [0f32; EMBED_DIM];
            weights[700] = 10.;
            head[w] = Some(Learned {
                weights,
                bias: -5.,
                threshold,
            });
        }
        let set = TagSet::new((0..n).map(axis).collect(), head).unwrap();
        let mut image = [0f32; EMBED_DIM];
        image[index("photo") % EMBED_DIM] = 0.3;
        image[700] = 1.;
        let scores = set.scores(&image);
        assert!(scores.photo_share >= SELECTION.photo_min);
        let sigmoid_5 = 1. / (1. + (-5f32).exp());
        assert!((scores.learned[5] - sigmoid_5).abs() < 1e-5);
        // Both learned words pass, independent of each other.
        let words: Vec<u16> = set.tags(&image).iter().map(|t| t.word).collect();
        assert_eq!(words, vec![5, 6]);
        assert_eq!(set.select(&scores, SELECTION), set.tags(&image));
        image[700] = 0.6; // sigmoid(1) = 0.73: only the lower threshold
        let words: Vec<u16> = set.tags(&image).iter().map(|t| t.word).collect();
        assert_eq!(words, vec![5]);
        // Drawn instead: anime wins the style competition, zero-shot words show.
        image[index("photo") % EMBED_DIM] = 0.;
        image[index("anime") % EMBED_DIM] = 0.3;
        let scores = set.scores(&image);
        assert!(scores.photo_share < SELECTION.photo_min);
        let tags = set.tags(&image);
        assert_eq!(usize::from(tags[0].word), index("anime"));
        assert_eq!(set.select(&scores, SELECTION), tags);
    }

    #[test]
    fn softmax_never_overflows_or_divides_by_zero() {
        let p = softmax(&[1000., 1000.]);
        assert_eq!(p, vec![0.5, 0.5]);
        assert_eq!(softmax(&[f32::NAN, 1.]), vec![0., 0.]);
        assert!(softmax(&[]).is_empty());
    }
}
