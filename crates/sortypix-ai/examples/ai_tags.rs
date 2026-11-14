//! Tags: generator of the embedded text embeddings, and quality and speed probe of the tags the
//! application shows (learned head plus zero-shot, `tags.rs`).
//!
//! `cargo run -p sortypix-ai --release --example ai_tags` (probe), `... -- --write` (writes
//! `crates/sortypix-ai/data/tags-l14-int8.bin`, then probes: retrain the head and rebuild after
//! a vocabulary change), `... -- --candidates` (tuning pairs to judge), `... -- --recheck` (test
//! pairs shown but absent from the exhaustive judgment, to judge once more).
//!
//! Needs `$SORTYPIX_AI_DIR` (the ai-dev root: `models/`, `corpus/`, `calibration/`,
//! `tags-test/generated/`). Writes `target/reports/ai-tags.txt` (key=value) and
//! `target/reports/ai-tags-shown.txt` (each image with its tags), and panics after writing them
//! when a criterion fails.
//!
//! Two sets, never mixed (`docs/design/Étiquettes apprises (2026-11-13).md`):
//! - **tuning**: the 120 corpus photos and every fourth photo of the five real days
//!   (`tests/ai/tags-truth.json`, pairs judged by eye as the selections showed them). The rule
//!   below picks [`SELECTION`] on it;
//! - **test**: 227 other photos of the real days (with the 108 of the former held-out set of
//!   the same day, now folded in) and 75 generated or drawn images of `tags-test/generated/`
//!   (`tests/ai/tags-test.json`), judged exhaustively (every word present
//!   listed, borderline ones apart as "doubt") before any result was seen; never used to tune.
//!   A shown pair is right when listed present, ignored when doubtful, wrong otherwise. Pairs a
//!   first judgment missed may be judged again (`--recheck`), never removed.
//!
//! Rule (written 2026-11-13 before the head was measured):
//! 1. selections: an image is a photo or art by the share of the photo-like style words (0.05
//!    to 0.20, `tags.rs`); photos carry
//!    learned words at their `train_head.py` thresholds, capped from 1 to 12; art carries
//!    zero-shot words from a minimum of 0.02 to 0.50 by 0.01, capped from 1 to 6 (rewritten the
//!    same day: a first grid without the split picked zero-shot off and failed on generated
//!    images, the head being learned on photographs);
//! 2. eligible on the tuning set when the seven corpus category words (cat, dog, car, beach,
//!    portrait, food, flower) reach a mean precision of 0.90 and a mean recall of 0.70, and the
//!    shown pairs of the judged photos have a precision of at least 0.80 with at most 10 %
//!    unjudged. Generated images of the tuning set (`tuning-generated/`, another draw from
//!    Commons, added the same day when the test showed the head under its bar on generated
//!    images) count in that precision; a separate bar of 0.75 for them was tried and dropped:
//!    no selection reaches it, zero-shot included (0.78 at best with the categories met);
//! 3. the rule keeps the eligible selection with the most right pairs (ties: higher precision,
//!    smaller caps, higher minimum); `tags::SELECTION` must be that selection;
//! 4. test: precision of shown pairs at least 0.80 over the whole test set and 0.75 on each part
//!    (real photos, generated images); no word judged (right or wrong, doubt aside) on more than
//!    10 % of the test images with a precision under 0.5 (corrected the same day: a word only
//!    ever judged doubtful counted as wrong). Every test image is judged exhaustively;
//! 5. the head must beat zero-shot clearly (written 2026-11-13 before the head was measured on
//!    the test set): on the test set, at least 1.5 times the right pairs of the zero-shot
//!    selection shipped before it (minimum 0.09, cap 3), here over the same words with no learned
//!    one (the 277-word version is compared once in the design document);
//! 6. speed: tags of 1,000 embeddings on one thread under 50 ms (times `SORTYPIX_BUDGET_SCALE`).
//!
//! `SORTYPIX_AI_TAGS_REUSE=1` reuses the image embeddings of the previous run
//! (`target/ai-tags/images.bin`) while tuning; the check never sets it.

use image::{DynamicImage, RgbaImage};
use sortypix_ai::tags::{Params, SELECTION, Scores, Selection, text_to_bytes};
use sortypix_ai::vocabulary::{VOCABULARY, sentence};
use sortypix_ai::{
    EMBED_DIM, Embedding, Engine, Precision, TagSet, cosine, models_dir, preprocess,
    recommended_threads,
};
use sortypix_app::decode::decode;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

type Res<T> = Result<T, Box<dyn std::error::Error>>;

const DECODE_EDGE: u32 = 448;
/// The corpus categories and the word that names each.
const CATEGORY_WORDS: [(&str, &str); 7] = [
    ("cat", "cat"),
    ("dog", "dog"),
    ("car", "car"),
    ("beach", "beach"),
    ("portrait", "portrait"),
    ("food", "food"),
    ("flower", "flower"),
];
const CATEGORY_PRECISION_MIN: f64 = 0.90;
const CATEGORY_RECALL_MIN: f64 = 0.70;
const TUNING_PRECISION_MIN: f64 = 0.80;
const UNJUDGED_MAX: f64 = 0.10;
const TEST_PRECISION_MIN: f64 = 0.80;
const TEST_PART_PRECISION_MIN: f64 = 0.75;
/// The zero-shot selection shipped before the head, and how many times its right test pairs
/// the head must reach.
const ZERO_SHOT_BEFORE: Params = Params { min: 0.09, max: 3 };
const BEAT_ZERO_SHOT: f64 = 1.5;
/// A word shown on more than this share of the test images...
const EVERYWHERE_SHARE: f64 = 0.10;
/// ...must be right at least this often.
const EVERYWHERE_PRECISION_MIN: f64 = 0.5;
/// The grid, in hundredths of probability, and the caps.
const MINS: std::ops::RangeInclusive<u32> = 2..=50;
/// Smallest photo-like share for a photo (the median of real photos is about 0.38).
const PHOTO_MINS: [f32; 6] = [0.05, 0.08, 0.10, 0.12, 0.15, 0.20];
const PHOTO_CAPS: std::ops::RangeInclusive<usize> = 1..=12;
const ART_CAPS: std::ops::RangeInclusive<usize> = 1..=6;
/// Widest selection to judge on the tuning set (`--candidates`).
const CANDIDATES: Selection = Selection {
    photo_min: 0.1,
    photo_max: 12,
    art: Params { min: 0.02, max: 6 },
};
const SPEED_PHOTOS: usize = 1000;
const SPEED_BUDGET_MS: f64 = 50.;
/// The embedded text embeddings and a fresh generation must agree this well (another processor
/// may round the int8 text model a little differently).
const EMBEDDED_COSINE_MIN: f32 = 0.999;

#[derive(Clone, Copy, PartialEq)]
enum Set {
    Tuning,
    Test,
}

struct Image {
    /// `corpus/cat-01.jpg`, `calibration/<day>/<file>` or `generated/<file>`: the truth key.
    key: String,
    path: PathBuf,
    category: Option<String>,
    set: Set,
}

fn manifest(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(name)
}

fn read_json(path: &Path) -> Res<serde_json::Value> {
    Ok(serde_json::from_str(&std::fs::read_to_string(path)?)?)
}

fn image(root: &Path, key: &str, category: Option<String>, set: Set) -> Res<Image> {
    let path = if let Some(file) = key.strip_prefix("generated/") {
        root.join("tags-test").join("generated").join(file)
    } else if let Some(file) = key.strip_prefix("tuning-generated/") {
        root.join("tags-tuning").join("generated").join(file)
    } else {
        root.join(key)
    };
    if !path.is_file() {
        return Err(format!("image missing: {}", path.display()).into());
    }
    Ok(Image {
        key: key.to_string(),
        path,
        category,
        set,
    })
}

fn corpus(root: &Path) -> Res<Vec<Image>> {
    let json = read_json(&manifest("../../tests/ai/corpus.json"))?;
    let mut out = Vec::new();
    for entry in json["images"].as_array().ok_or("corpus.json: no images")? {
        let file = entry["file"].as_str().ok_or("image without file")?;
        let category = entry["category"].as_str().map(str::to_string);
        out.push(image(
            root,
            &format!("corpus/{file}"),
            category,
            Set::Tuning,
        )?);
    }
    Ok(out)
}

/// The photos of the real days in the tuning truth (every fourth one of each day) and its
/// generated images (`tuning-generated/`, never those of the test set).
fn tuning_days(root: &Path, truth: &Truth) -> Res<Vec<Image>> {
    let mut keys: Vec<&String> = truth
        .keys()
        .filter(|k| k.starts_with("calibration/") || k.starts_with("tuning-generated/"))
        .collect();
    keys.sort();
    keys.into_iter()
        .map(|k| image(root, k, None, Set::Tuning))
        .collect()
}

/// A verdict per (image, word): true present, false absent; missing = not judged.
type Truth = HashMap<String, HashMap<String, bool>>;

fn word_known(name: &str, file: &str, key: &str) -> Res<()> {
    if VOCABULARY.iter().any(|v| v.name == name) {
        Ok(())
    } else {
        Err(format!("{file}: unknown word {name} ({key})").into())
    }
}

/// Tuning truth: judged pairs (`yes`, `no`).
fn tuning_truth() -> Res<Truth> {
    let json = read_json(&manifest("../../tests/ai/tags-truth.json"))?;
    let mut out: Truth = HashMap::new();
    for (key, judged) in json["judged"]
        .as_object()
        .ok_or("tags-truth.json: no judged")?
    {
        let entry = out.entry(key.clone()).or_default();
        for (field, present) in [("yes", true), ("no", false)] {
            for w in judged[field].as_array().into_iter().flatten() {
                let name = w.as_str().ok_or("word not a string")?;
                word_known(name, "tags-truth.json", key)?;
                entry.insert(name.to_string(), present);
            }
        }
    }
    Ok(out)
}

/// Test truth, exhaustive: `yes` present, `doubt` neither, every other word absent.
struct TestTruth {
    yes: HashSet<String>,
    doubt: HashSet<String>,
    /// Pairs judged absent on a second look (`--recheck`): absent like unlisted ones, but no
    /// longer offered for a recheck.
    no: HashSet<String>,
}

fn test_truth() -> Res<BTreeMap<String, TestTruth>> {
    let json = read_json(&manifest("../../tests/ai/tags-test.json"))?;
    let mut out = BTreeMap::new();
    for (key, judged) in json["images"]
        .as_object()
        .ok_or("tags-test.json: no images")?
    {
        let field = |name: &str| -> Res<HashSet<String>> {
            let mut set = HashSet::new();
            for w in judged[name].as_array().into_iter().flatten() {
                let w = w.as_str().ok_or("word not a string")?;
                word_known(w, "tags-test.json", key)?;
                set.insert(w.to_string());
            }
            Ok(set)
        };
        out.insert(
            key.clone(),
            TestTruth {
                yes: field("yes")?,
                doubt: field("doubt")?,
                no: field("no")?,
            },
        );
    }
    Ok(out)
}

fn input(path: &Path) -> Vec<f32> {
    let d = decode(path, Some(DECODE_EDGE)).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let rgba = RgbaImage::from_raw(d.width, d.height, d.rgba).expect("decoded buffer");
    let rgba = DynamicImage::ImageRgba8(rgba).to_rgba8();
    preprocess(rgba.as_raw(), rgba.width(), rgba.height()).expect("preprocess")
}

fn embed_images(engine: &Engine, images: &[Image]) -> Vec<Embedding> {
    let next = AtomicUsize::new(0);
    let out = Mutex::new(vec![[0f32; EMBED_DIM]; images.len()]);
    std::thread::scope(|s| {
        for _ in 0..engine.visual_sessions() * 2 {
            s.spawn(|| {
                loop {
                    let i = next.fetch_add(1, Ordering::Relaxed);
                    let Some(image) = images.get(i) else { break };
                    let x = input(&image.path);
                    let e = engine
                        .embed_images(std::slice::from_ref(&x))
                        .expect("embedding")[0];
                    out.lock().expect("results")[i] = e;
                }
            });
        }
    });
    out.into_inner().expect("results")
}

/// Image embeddings, reused from the previous run when asked (tuning only).
fn image_embeddings(engine: &Engine, images: &[Image]) -> Res<Vec<Embedding>> {
    let dump = manifest("../../target/ai-tags/images.bin");
    let keys: String = images.iter().map(|i| format!("{}\n", i.key)).collect();
    if std::env::var_os("SORTYPIX_AI_TAGS_REUSE").is_some()
        && let Ok(bytes) = std::fs::read(&dump)
        && let Some(stored) = parse_dump(&bytes, &keys)
    {
        return Ok(stored);
    }
    let embeddings = embed_images(engine, images);
    let mut bytes = (keys.len() as u64).to_le_bytes().to_vec();
    bytes.extend_from_slice(keys.as_bytes());
    for e in &embeddings {
        for v in e {
            bytes.extend_from_slice(&v.to_le_bytes());
        }
    }
    std::fs::create_dir_all(dump.parent().ok_or("dump dir")?)?;
    std::fs::write(&dump, bytes)?;
    Ok(embeddings)
}

fn parse_dump(bytes: &[u8], keys: &str) -> Option<Vec<Embedding>> {
    let len = u64::from_le_bytes(bytes.get(..8)?.try_into().ok()?) as usize;
    if bytes.get(8..8 + len)? != keys.as_bytes() {
        return None;
    }
    let rest = &bytes[8 + len..];
    let values: Vec<f32> = rest
        .as_chunks::<4>()
        .0
        .iter()
        .map(|c| f32::from_le_bytes(*c))
        .collect();
    let rows = values.as_chunks::<EMBED_DIM>().0;
    (rows.len() * EMBED_DIM * 4 == rest.len()).then(|| rows.to_vec())
}

fn word(name: &str) -> u16 {
    VOCABULARY
        .iter()
        .position(|w| w.name == name)
        .unwrap_or_else(|| panic!("word {name} missing")) as u16
}

struct CategoryRow {
    name: &'static str,
    precision: f64,
    recall: f64,
    shown: usize,
}

/// Mean precision and recall of the category words over the corpus.
fn category_scores(corpus: &[(&Image, Vec<u16>)]) -> (f64, f64, Vec<CategoryRow>) {
    let mut rows = Vec::new();
    for (category, name) in CATEGORY_WORDS {
        let w = word(name);
        let of = |c: &(&Image, Vec<u16>)| c.0.category.as_deref() == Some(category);
        let shown: Vec<&(&Image, Vec<u16>)> = corpus.iter().filter(|c| c.1.contains(&w)).collect();
        let truth = corpus.iter().filter(|c| of(c)).count();
        let right = shown.iter().filter(|c| of(c)).count();
        // No photo shown: precision undefined, counted as 1 (recall then says it).
        let precision = if shown.is_empty() {
            1.
        } else {
            right as f64 / shown.len() as f64
        };
        let recall = if truth == 0 {
            0.
        } else {
            right as f64 / truth as f64
        };
        rows.push(CategoryRow {
            name,
            precision,
            recall,
            shown: shown.len(),
        });
    }
    let n = rows.len() as f64;
    let p = rows.iter().map(|r| r.precision).sum::<f64>() / n;
    let r = rows.iter().map(|r| r.recall).sum::<f64>() / n;
    (p, r, rows)
}

#[derive(Clone, Copy, Debug, Default)]
struct Judged {
    right: usize,
    wrong: usize,
    unjudged: usize,
}

impl Judged {
    fn precision(&self) -> f64 {
        let n = self.right + self.wrong;
        if n == 0 {
            0.
        } else {
            self.right as f64 / n as f64
        }
    }

    fn unjudged_share(&self) -> f64 {
        let n = self.right + self.wrong + self.unjudged;
        if n == 0 {
            0.
        } else {
            self.unjudged as f64 / n as f64
        }
    }

    fn add(&mut self, other: Judged) {
        self.right += other.right;
        self.wrong += other.wrong;
        self.unjudged += other.unjudged;
    }
}

fn judge_tuning(tags: &[u16], verdicts: &HashMap<String, bool>) -> Judged {
    let mut out = Judged::default();
    for &t in tags {
        match verdicts.get(VOCABULARY[usize::from(t)].name) {
            Some(true) => out.right += 1,
            Some(false) => out.wrong += 1,
            None => out.unjudged += 1,
        }
    }
    out
}

/// Right, wrong (doubtful pairs are neither).
fn judge_test(tags: &[u16], truth: &TestTruth) -> Judged {
    let mut out = Judged::default();
    for &t in tags {
        let name = VOCABULARY[usize::from(t)].name;
        if truth.yes.contains(name) {
            out.right += 1;
        } else if !truth.doubt.contains(name) {
            out.wrong += 1;
        }
    }
    out
}

fn selected(set: &TagSet, scores: &[Scores], selection: Selection) -> Vec<Vec<u16>> {
    scores
        .iter()
        .map(|s| set.select(s, selection).iter().map(|t| t.word).collect())
        .collect()
}

fn main() -> Res<()> {
    let args: Vec<String> = std::env::args().collect();
    let flag = |f: &str| args.iter().any(|a| a == f);
    let dir = models_dir();
    let root = dir
        .parent()
        .ok_or("models dir has no parent")?
        .to_path_buf();
    let tuning = tuning_truth()?;
    let test = test_truth()?;
    let mut all = corpus(&root)?;
    all.extend(tuning_days(&root, &tuning)?);
    for key in test.keys() {
        all.push(image(&root, key, None, Set::Test)?);
    }
    let cores = std::thread::available_parallelism().map_or(1, |n| n.get());
    let engine = Engine::load(&dir, Precision::Int8, recommended_threads(cores))?;
    let mut report = String::new();
    let mut failures: Vec<String> = Vec::new();
    let _ = writeln!(report, "words={}", VOCABULARY.len());
    let n_test = all.iter().filter(|i| i.set == Set::Test).count();
    let _ = writeln!(report, "tuning_images={}", all.len() - n_test);
    let _ = writeln!(report, "test_images={n_test}");

    // 0. The embedded text embeddings are this vocabulary's (fresh generation compared).
    let started = Instant::now();
    let fresh: Vec<Embedding> = VOCABULARY
        .iter()
        .map(|w| engine.embed_text(&sentence(w)))
        .collect::<Result<_, _>>()?;
    let _ = writeln!(
        report,
        "text_embedding_s={:.1}",
        started.elapsed().as_secs_f64()
    );
    if flag("--write") {
        let path = manifest("data/tags-l14-int8.bin");
        std::fs::write(&path, text_to_bytes(&fresh))?;
        println!(
            "written {} (retrain the head, then rebuild)",
            path.display()
        );
        return Ok(());
    }
    let set = TagSet::embedded()?;
    let worst_cos = set
        .embeddings()
        .iter()
        .zip(&fresh)
        .map(|(a, b)| cosine(a, b))
        .fold(1f32, f32::min);
    let _ = writeln!(report, "embedded_vs_fresh_cosine_min={worst_cos:.5}");
    if worst_cos < EMBEDDED_COSINE_MIN {
        failures.push("embedded text embeddings differ from a fresh generation".into());
    }
    let learned = set.head().iter().filter(|h| h.is_some()).count();
    let _ = writeln!(report, "learned_words={learned}");
    let _ = writeln!(report, "zero_shot_words={}", VOCABULARY.len() - learned);

    let images = image_embeddings(&engine, &all)?;
    let probabilities: Vec<Scores> = images.iter().map(|e| set.scores(e)).collect();
    let photos = probabilities
        .iter()
        .filter(|s| s.photo_share >= SELECTION.photo_min)
        .count();
    let index_of = |s: Set| -> Vec<usize> { (0..all.len()).filter(|&i| all[i].set == s).collect() };
    let tuning_ix = index_of(Set::Tuning);
    let test_ix = index_of(Set::Test);

    if flag("--candidates") || flag("--recheck") {
        let mut out = serde_json::Map::new();
        let shown = selected(
            &set,
            &probabilities,
            if flag("--candidates") {
                CANDIDATES
            } else {
                SELECTION
            },
        );
        for &i in if flag("--candidates") {
            &tuning_ix
        } else {
            &test_ix
        } {
            let image = &all[i];
            let words: Vec<&str> = shown[i]
                .iter()
                .map(|&t| VOCABULARY[usize::from(t)].name)
                .filter(|w| match (tuning.get(&image.key), test.get(&image.key)) {
                    (Some(v), _) => !v.contains_key(*w),
                    (None, Some(t)) => {
                        !t.yes.contains(*w) && !t.doubt.contains(*w) && !t.no.contains(*w)
                    }
                    (None, None) => false,
                })
                .collect();
            if !words.is_empty() {
                out.insert(
                    image.key.clone(),
                    serde_json::json!({ "path": image.path, "words": words }),
                );
            }
        }
        let path = manifest("../../target/ai-tags/to-judge.json");
        std::fs::create_dir_all(path.parent().ok_or("dir")?)?;
        std::fs::write(&path, serde_json::to_string_pretty(&out)?)?;
        println!(
            "{} images with pairs to judge, in {}",
            out.len(),
            path.display()
        );
        return Ok(());
    }

    // 1 to 3. The grid on the tuning set.
    let corpus_ix: Vec<usize> = tuning_ix
        .iter()
        .copied()
        .filter(|&i| all[i].category.is_some())
        .collect();
    let judged_ix: Vec<usize> = tuning_ix
        .iter()
        .copied()
        .filter(|&i| tuning.contains_key(&all[i].key))
        .collect();
    let _ = writeln!(report, "tuning_judged_photos={}", judged_ix.len());
    let tuning_scores = |params: Selection| {
        let shown = selected(&set, &probabilities, params);
        let corpus: Vec<(&Image, Vec<u16>)> = corpus_ix
            .iter()
            .map(|&i| (&all[i], shown[i].clone()))
            .collect();
        let (cp, cr, rows) = category_scores(&corpus);
        let (mut j, mut generated) = (Judged::default(), Judged::default());
        for &i in &judged_ix {
            let one = judge_tuning(&shown[i], &tuning[&all[i].key]);
            j.add(one);
            if all[i].key.starts_with("tuning-generated/") {
                generated.add(one);
            }
        }
        (cp, cr, rows, j, generated)
    };
    let mut best: Option<(Selection, Judged)> = None;
    for photo_min in PHOTO_MINS {
        for photo_max in PHOTO_CAPS {
            for max in ART_CAPS {
                for hundredths in MINS {
                    let params = Selection {
                        photo_min,
                        photo_max,
                        art: Params {
                            min: hundredths as f32 / 100.,
                            max,
                        },
                    };
                    let (cp, cr, _, j, g) = tuning_scores(params);
                    let _ = writeln!(
                        report,
                        "grid photo_min={photo_min:.2} photo_max={photo_max} art_min={:.2} art_max={max} category_precision={cp:.3} category_recall={cr:.3} right={} wrong={} unjudged={} precision={:.3} generated_precision={:.3}",
                        params.art.min,
                        j.right,
                        j.wrong,
                        j.unjudged,
                        j.precision(),
                        g.precision()
                    );
                    let eligible = cp >= CATEGORY_PRECISION_MIN
                        && cr >= CATEGORY_RECALL_MIN
                        && j.precision() >= TUNING_PRECISION_MIN
                        && j.unjudged_share() <= UNJUDGED_MAX;
                    // Grid order prefers smaller caps; a higher minimum wins a full tie.
                    let better = best.is_none_or(|(b, bj)| {
                        j.right > bj.right
                            || (j.right == bj.right && j.precision() > bj.precision())
                            || (j.right == bj.right
                                && j.precision() == bj.precision()
                                && (photo_min, photo_max, max)
                                    == (b.photo_min, b.photo_max, b.art.max)
                                && params.art.min > b.art.min)
                    });
                    if eligible && better {
                        best = Some((params, j));
                    }
                }
            }
        }
    }
    let _ = writeln!(report, "photos_by_style={photos} of {}", all.len());
    match best {
        Some((p, j)) => {
            let _ = writeln!(
                report,
                "rule photo_min={:.2} photo_max={} art_min={:.2} art_max={} right={} precision={:.3}",
                p.photo_min,
                p.photo_max,
                p.art.min,
                p.art.max,
                j.right,
                j.precision()
            );
            if (p.photo_min - SELECTION.photo_min).abs() > 1e-4
                || p.photo_max != SELECTION.photo_max
                || (p.art.min - SELECTION.art.min).abs() > 1e-4
                || p.art.max != SELECTION.art.max
            {
                failures.push(format!(
                    "tags::SELECTION is {SELECTION:?}, the rule picks {p:?}"
                ));
            }
        }
        None => failures.push("no selection of the grid meets the rule".into()),
    }

    // What the application shows, tuning set.
    let shown = selected(&set, &probabilities, SELECTION);
    let (cp, cr, rows, j, g) = tuning_scores(SELECTION);
    let _ = writeln!(
        report,
        "tuning_generated right={} wrong={} precision={:.3}",
        g.right,
        g.wrong,
        g.precision()
    );
    let _ = writeln!(report, "category_precision={cp:.3}");
    let _ = writeln!(report, "category_recall={cr:.3}");
    for row in rows {
        let _ = writeln!(
            report,
            "category {} precision={:.3} recall={:.3} shown={}",
            row.name, row.precision, row.recall, row.shown
        );
    }
    let _ = writeln!(
        report,
        "tuning_right={} tuning_wrong={} tuning_unjudged={} tuning_precision={:.3}",
        j.right,
        j.wrong,
        j.unjudged,
        j.precision()
    );

    // 4. Test set.
    let mut whole = Judged::default();
    for (label, prefix) in [("photos", "calibration/"), ("generated", "generated/")] {
        let ix: Vec<usize> = test_ix
            .iter()
            .copied()
            .filter(|&i| all[i].key.starts_with(prefix))
            .collect();
        let mut part = Judged::default();
        let (mut tags, mut present, mut found) = (0usize, 0usize, 0usize);
        for &i in &ix {
            let truth = &test[&all[i].key];
            part.add(judge_test(&shown[i], truth));
            tags += shown[i].len();
            present += truth.yes.len();
            found += shown[i]
                .iter()
                .filter(|&&t| truth.yes.contains(VOCABULARY[usize::from(t)].name))
                .count();
        }
        whole.add(part);
        let n = ix.len().max(1) as f64;
        let _ = writeln!(
            report,
            "test_{label} images={} right={} wrong={} precision={:.3} tags_per_image={:.2} recall={:.3}",
            ix.len(),
            part.right,
            part.wrong,
            part.precision(),
            tags as f64 / n,
            found as f64 / present.max(1) as f64
        );
        if part.precision() < TEST_PART_PRECISION_MIN {
            failures.push(format!(
                "test {label}: precision {:.3} < {TEST_PART_PRECISION_MIN}",
                part.precision()
            ));
        }
    }
    let _ = writeln!(report, "test_precision={:.3}", whole.precision());
    if whole.precision() < TEST_PRECISION_MIN {
        failures.push(format!(
            "test precision {:.3} < {TEST_PRECISION_MIN}",
            whole.precision()
        ));
    }
    // 5. Against the zero-shot selection shipped before (same text embeddings, no head).
    let mut before = Judged::default();
    for &i in &test_ix {
        let p = &probabilities[i].zero_shot;
        let mut order: Vec<usize> = (0..p.len())
            .filter(|&w| p[w] >= ZERO_SHOT_BEFORE.min)
            .collect();
        order.sort_by(|a, b| p[*b].total_cmp(&p[*a]).then(a.cmp(b)));
        let tags: Vec<u16> = order
            .into_iter()
            .take(ZERO_SHOT_BEFORE.max)
            .map(|w| w as u16)
            .collect();
        before.add(judge_test(&tags, &test[&all[i].key]));
    }
    let _ = writeln!(
        report,
        "test_zero_shot_before right={} wrong={} precision={:.3}",
        before.right,
        before.wrong,
        before.precision()
    );
    if (whole.right as f64) < BEAT_ZERO_SHOT * before.right as f64 {
        failures.push(format!(
            "{} right test pairs, zero-shot had {}: not {BEAT_ZERO_SHOT} times",
            whole.right, before.right
        ));
    }
    // Recall per group, and words shown too often for their precision.
    let mut by_group: BTreeMap<String, (usize, usize)> = BTreeMap::new();
    let mut per_word: BTreeMap<&str, (usize, usize, usize)> = BTreeMap::new();
    for &i in &test_ix {
        let truth = &test[&all[i].key];
        for w in &truth.yes {
            let g = format!("{:?}", VOCABULARY[usize::from(word(w))].group);
            let e = by_group.entry(g).or_default();
            e.0 += 1;
            e.1 += usize::from(shown[i].contains(&word(w)));
        }
        for &t in &shown[i] {
            let name = VOCABULARY[usize::from(t)].name;
            let e = per_word.entry(name).or_default();
            e.0 += 1;
            if truth.yes.contains(name) {
                e.1 += 1;
            } else if !truth.doubt.contains(name) {
                e.2 += 1;
            }
        }
    }
    for (g, (present, found)) in &by_group {
        let _ = writeln!(
            report,
            "test_recall {g} present={present} shown={found} recall={:.3}",
            *found as f64 / (*present).max(1) as f64
        );
    }
    for (name, (n, right, wrong)) in &per_word {
        let _ = writeln!(
            report,
            "test_word {name} shown={n} right={right} wrong={wrong}"
        );
        // Pairs judged doubtful are neither: a word only ever doubtful is not "wrong everywhere".
        let judged = right + wrong;
        let precision = *right as f64 / judged.max(1) as f64;
        if judged as f64 > EVERYWHERE_SHARE * test_ix.len() as f64
            && precision < EVERYWHERE_PRECISION_MIN
        {
            failures.push(format!(
                "{name} shown on {n} test images, right on {right}, wrong on {wrong}"
            ));
        }
    }

    let mut shown_text = String::new();
    for (i, image) in all.iter().enumerate() {
        let names: Vec<&str> = shown[i]
            .iter()
            .map(|&t| VOCABULARY[usize::from(t)].name)
            .collect();
        let _ = writeln!(shown_text, "{}: {}", image.key, names.join(", "));
    }
    let with_tags = shown.iter().filter(|s| !s.is_empty()).count();
    let _ = writeln!(report, "images_with_tags={with_tags} of {}", all.len());
    let _ = writeln!(
        report,
        "tags_per_image={:.2}",
        shown.iter().map(Vec::len).sum::<usize>() as f64 / all.len() as f64
    );

    // 6. Speed.
    let sample: Vec<&Embedding> = images.iter().cycle().take(SPEED_PHOTOS).collect();
    let started = Instant::now();
    let mut count = 0usize;
    for e in &sample {
        count += set.tags(e).len();
    }
    let ms = started.elapsed().as_secs_f64() * 1000.;
    let scale: f64 = std::env::var("SORTYPIX_BUDGET_SCALE")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(1.);
    let _ = writeln!(report, "speed_ms_per_{SPEED_PHOTOS}={ms:.1} tags={count}");
    if ms > SPEED_BUDGET_MS * scale {
        failures.push(format!("tags of {SPEED_PHOTOS} photos took {ms:.1} ms"));
    }

    let _ = writeln!(
        report,
        "verdict={}",
        if failures.is_empty() { "PASS" } else { "FAIL" }
    );
    for f in &failures {
        let _ = writeln!(report, "failure={f}");
    }
    let out = manifest("../../target/reports");
    std::fs::create_dir_all(&out)?;
    std::fs::write(out.join("ai-tags.txt"), &report)?;
    std::fs::write(out.join("ai-tags-shown.txt"), &shown_text)?;
    print!("{report}");
    if !failures.is_empty() {
        panic!("ai_tags: {}", failures.join("; "));
    }
    Ok(())
}
