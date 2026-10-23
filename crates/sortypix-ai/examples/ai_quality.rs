//! Quality probe of the AI pipeline against the real models and the CC0 corpus.
//!
//! `cargo run -p sortypix-ai --release --example ai_quality`
//!
//! Needs `$SORTYPIX_AI_DIR` (the ai-dev root: `models/` and `corpus/`) and the labels in
//! `tests/ai/corpus.json`. Writes `target/reports/ai-quality.txt` (key=value) and panics, after
//! writing the whole report, when a criterion fails. Thresholds are never lowered here: a
//! failure is reported as measured.
//!
//! Images are decoded with `sortypix_app::decode` (EXIF orientation applied, as in the
//! application). Degradations are generated here, deterministically:
//! - blur: Gaussian blur, sigma = 3 px (`image::imageops::blur`, on sRGB values);
//! - jpeg: re-encoded at quality 10 then decoded;
//! - noise: Gaussian noise, sigma = 25 on the 0..255 scale, per channel, rounded and clamped;
//! - exposure: -2 EV, `srgb(linear(v) * 2^-2)` with the exact sRGB transfer functions.
//!
//! Every degradation is applied and gated on the image as the application decodes it (long edge
//! 448), since the application always embeds a decoded preview, never the full file (noise:
//! Mickaël, 2026-10-23; the other three: controller ruling, same day). The rates at the corpus
//! resolution (long edge 1600 to 1920 px) stay in the report as information.
//!
//! Off-topic and bursts are judged on the real photo days of `tests/ai/calibration.json`
//! (controller ruling, 2026-10-23): at most one flagged photo per 100 in each day, at least
//! 90 % of the injected photos flagged and set aside by `coverage_order`. Bursts (same scene,
//! taken close in time: Mickaël, 2026-10-23) are judged pairwise against the day labelled by
//! eye in `tests/ai/calibration-bursts.json` (precision 0.95; recall 0.85 since Mickael, 2026-10-23);
//! EXIF bursts of every day, the synthetic bursts (no capture times) and the apple among the
//! cats are reported (the synthetic precision is still gated).
//!
//! Likely reason of a low score (`sortypix_ai::reasons`): text probes judged on the four
//! degradations above plus +2 EV (calibration only), each degraded photo against the clean
//! corpus as its folder; the rule that sets `REASON_MIN` and the recognised probes is written
//! at `REASON_CLEAN_CLAIMS_MAX` before the measure.
//!
//! Optional: `SORTYPIX_AI_CORPUS` names a folder of real photos with a `labels.json`
//! (`{"bursts": [["a.jpg", "b.jpg"], ...], "rejects": ["c.jpg", ...]}`, file names only). It is
//! reported (counts only, no file name) and never fails the probe.

use image::codecs::jpeg::JpegEncoder;
use image::{DynamicImage, ImageFormat, RgbImage, RgbaImage, imageops};
use sortypix_ai::reasons::{FolderLean, REASONS, strongest};
use sortypix_ai::{
    Aesthetic, BURST_MAX_GAP_S, BURST_SUGGEST, BURST_THRESHOLD, BURST_WINDOW, DEFAULT_PRECISION,
    EMBED_DIM, Embedding, Engine, MIN_SPREAD, OFF_TOPIC_K, OFF_TOPIC_Z, Precision, REASON_MIN,
    Reason, ReasonProbes, SEARCH_MIN, bursts, cosine, coverage_order, coverage_order_with_outliers,
    models_dir, outliers, preprocess, search, search_prompt,
};
use sortypix_app::decode::decode;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

type Res<T> = Result<T, Box<dyn std::error::Error>>;

/// Total intra-op budget: three visual sessions of eight threads.
const THREADS: usize = 24;
/// Long edge of the application's reduced decode (as in `ai_speed`).
const WORKING_EDGE: u32 = 448;

const BLUR_SIGMA: f32 = 3.0;
const JPEG_QUALITY: u8 = 10;
const NOISE_SIGMA: f32 = 25.0;
const EXPOSURE_EV: f32 = -2.0;
const DEGRADATIONS: [&str; 4] = ["blur", "jpeg", "noise", "exposure"];
/// Reason probes (`sortypix_ai::reasons`): the degradation each probe should recognise. The
/// overexposure (+2 EV) is generated for this calibration only (no score gate).
const OVEREXPOSURE_EV: f32 = 2.0;
const REASON_CASES: [(&str, Reason); 5] = [
    ("blur", Reason::Blurry),
    ("jpeg", Reason::Compressed),
    ("noise", Reason::Noisy),
    ("exposure", Reason::Dark),
    ("overexposure", Reason::Overexposed),
];
/// Bars swept for `REASON_MIN` (relative lean, robust z).
const REASON_SWEEP: (f32, f32, f32) = (0.5, 6.0, 0.25);
/// Rule, written before the measure (2026-10-23): `REASON_MIN` is the smallest swept bar at
/// which at most this share of the clean corpus photos get a reason (any reason on a photo
/// that was not degraded counts as a false claim).
const REASON_CLEAN_CLAIMS_MAX: f64 = 0.10;
/// A probe is recognised (may be shown) when, at `REASON_MIN`, it is the shown reason of at
/// least this share of the photos with its degradation...
const REASON_SHOWN_MIN: f64 = 0.50;
/// ...and at least this share of all the photos it is shown for (clean and every degradation)
/// carry its degradation.
const REASON_PRECISION_MIN: f64 = 0.80;
const DEGRADATION_MIN: f64 = 0.85;

/// Regression guard of int8 against fp32, at the accepted level (measured 0.966 and 0.943).
/// Mickaël, 2026-10-23: int8 accepted, fp16/fp32 too slow on CPU (the plan asked 0.99 and 0.95).
const INT8_COSINE_MIN: f64 = 0.96;
const INT8_SPEARMAN_MIN: f64 = 0.93;

const TOP: usize = 5;
const TOP_RECALL_MIN: f64 = 0.8;
/// Thresholds swept for the search filter (`sortypix_ai::SEARCH_MIN`), in cosine.
const SEARCH_SWEEP: (f32, f32, f32) = (0.10, 0.40, 0.005);
/// Rule of the search filter, written before the measure (plan 4, task 4.3): the highest
/// threshold at which the mean recall over the categories of at least five photos is at least
/// this. Recall comes first: results are ranked best first, so a wrong photo kept sits at the end
/// of the grid, while a photo filtered out cannot be seen at all.
const SEARCH_RECALL_MIN: f64 = 0.90;
/// A filter that keeps more wrong photos than right ones is no filter (mean precision floor).
const SEARCH_PRECISION_MIN: f64 = 0.50;
/// Words absent from the corpus: what the filter keeps for them is reported.
const SEARCH_ABSENT: [&str; 3] = ["bicycle", "guitar", "airplane"];

const BURST_COUNT: usize = 30;
const BURST_FRAMES: usize = 5;
/// Each frame keeps a 94 % window whose center moves by up to +/-3 % on each axis.
const BURST_SHIFT: f32 = 0.03;
const BURST_EV: f32 = 0.3;
const BURST_PRECISION_MIN: f64 = 0.95;
/// Mickael, 2026-10-23: 0.85 on the labelled day (was 0.90). In-sample figure, no holdout day yet.
const BURST_RECALL_MIN: f64 = 0.85;

/// Share of the labelled bursts whose best-scored photo must be the same with int8 and fp32
/// for bursts to add their other photos to the rejection suggestions (plan 2, task 2.4).
const BURST_TOP_AGREEMENT_MIN: f64 = 0.80;

/// Off-topic photos injected into each photo day: the apple and two corpus photos.
const INJECTED: usize = 3;
/// Consecutive shots at most this many seconds apart, of the same orientation, form an EXIF
/// burst (reported on every photo day; the reference is the hand-labelled day).
const EXIF_BURST_GAP: i64 = 2;
/// Largest `window` of `bursts` tried on the labelled day.
const BURST_WINDOW_MAX: usize = 5;
/// Capture-time gaps (seconds) tried on the labelled day.
const BURST_GAPS: [f64; 5] = [2.0, 5.0, 10.0, 30.0, 60.0];
/// Share of injected off-topic photos that must be flagged and set aside (controller ruling,
/// 2026-10-23).
const INJECTED_FLAGGED_MIN: f64 = 0.90;
const COVERAGE_SEED: u64 = 0;
const STABILITY_SEEDS: u64 = 32;

struct Photo {
    file: String,
    category: String,
    path: PathBuf,
}

/// Deterministic generator (same algorithm as the crate's).
struct SplitMix64(u64);

impl SplitMix64 {
    fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Uniform in [0, 1).
    fn unit(&mut self) -> f32 {
        (self.next_u64() >> 40) as f32 / (1u64 << 24) as f32
    }

    /// Uniform in [-1, 1).
    fn signed(&mut self) -> f32 {
        self.unit() * 2.0 - 1.0
    }

    /// Standard normal (Box-Muller).
    fn normal(&mut self) -> f32 {
        let u1 = self.unit().max(f32::MIN_POSITIVE);
        let u2 = self.unit();
        (-2.0 * u1.ln()).sqrt() * (std::f32::consts::TAU * u2).cos()
    }
}

fn seed(a: usize, b: usize) -> SplitMix64 {
    SplitMix64(((a as u64) << 32) ^ (b as u64) ^ 0x5EED_0F50_471A_1111)
}

fn load_corpus(root: &Path) -> Res<Vec<Photo>> {
    let labels = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/ai/corpus.json");
    let json: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&labels)?)?;
    let images = json["images"]
        .as_array()
        .ok_or("corpus.json has no images array")?;
    let mut photos = Vec::with_capacity(images.len());
    for image in images {
        let file = image["file"].as_str().ok_or("image without file")?;
        let category = image["category"].as_str().ok_or("image without category")?;
        let path = root.join("corpus").join(file);
        if !path.is_file() {
            return Err(format!("corpus image missing: {}", path.display()).into());
        }
        photos.push(Photo {
            file: file.to_string(),
            category: category.to_string(),
            path,
        });
    }
    Ok(photos)
}

fn load_rgb(path: &Path, max_edge: Option<u32>) -> RgbImage {
    let d = decode(path, max_edge).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let rgba = RgbaImage::from_raw(d.width, d.height, d.rgba).expect("decoded buffer");
    DynamicImage::ImageRgba8(rgba).to_rgb8()
}

fn to_input(img: &RgbImage) -> Vec<f32> {
    let mut rgba = Vec::with_capacity(img.as_raw().len() / 3 * 4);
    for px in img.pixels() {
        rgba.extend_from_slice(&[px.0[0], px.0[1], px.0[2], 255]);
    }
    preprocess(&rgba, img.width(), img.height()).expect("preprocess")
}

fn srgb_to_linear(c: f32) -> f32 {
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

fn linear_to_srgb(l: f32) -> f32 {
    if l <= 0.003_130_8 {
        12.92 * l
    } else {
        1.055 * l.powf(1.0 / 2.4) - 0.055
    }
}

/// Exposure change of `ev` stops: `srgb(linear(v) * 2^ev)`, clamped to 0..255.
fn expose(img: &RgbImage, ev: f32) -> RgbImage {
    let gain = ev.exp2();
    let lut: Vec<u8> = (0..=255u8)
        .map(|v| {
            let l = srgb_to_linear(f32::from(v) / 255.0) * gain;
            (linear_to_srgb(l.min(1.0)) * 255.0)
                .round()
                .clamp(0.0, 255.0) as u8
        })
        .collect();
    let mut out = img.clone();
    for v in out.iter_mut() {
        *v = lut[usize::from(*v)];
    }
    out
}

fn degrade(img: &RgbImage, kind: &str, rng: &mut SplitMix64) -> RgbImage {
    match kind {
        "blur" => imageops::blur(img, BLUR_SIGMA),
        "jpeg" => {
            let mut bytes = Vec::new();
            JpegEncoder::new_with_quality(&mut bytes, JPEG_QUALITY)
                .encode_image(img)
                .expect("jpeg encode");
            image::load_from_memory_with_format(&bytes, ImageFormat::Jpeg)
                .expect("jpeg decode")
                .to_rgb8()
        }
        "noise" => {
            let mut out = img.clone();
            for v in out.iter_mut() {
                *v = (f32::from(*v) + rng.normal() * NOISE_SIGMA)
                    .round()
                    .clamp(0.0, 255.0) as u8;
            }
            out
        }
        "exposure" => expose(img, EXPOSURE_EV),
        other => unreachable!("unknown degradation {other}"),
    }
}

/// One burst frame: a 94 % window moved by up to +/-3 % per axis, then +/-0.3 EV.
fn burst_frame(img: &RgbImage, rng: &mut SplitMix64) -> RgbImage {
    let (w, h) = img.dimensions();
    let keep = 1.0 - 2.0 * BURST_SHIFT;
    let (cw, ch) = (
        (w as f32 * keep).round() as u32,
        (h as f32 * keep).round() as u32,
    );
    let left = ((w - cw) as f32 / 2.0 + rng.signed() * BURST_SHIFT * w as f32)
        .round()
        .clamp(0.0, (w - cw) as f32) as u32;
    let top = ((h - ch) as f32 / 2.0 + rng.signed() * BURST_SHIFT * h as f32)
        .round()
        .clamp(0.0, (h - ch) as f32) as u32;
    let crop = imageops::crop_imm(img, left, top, cw, ch).to_image();
    expose(&crop, rng.signed() * BURST_EV)
}

/// Embeds `count` inputs built by `make`, on twice as many threads as visual sessions.
fn embed_all(
    engine: &Engine,
    count: usize,
    make: impl Fn(usize) -> Vec<f32> + Sync,
) -> Vec<Embedding> {
    let next = AtomicUsize::new(0);
    let out = Mutex::new(vec![[0f32; EMBED_DIM]; count]);
    std::thread::scope(|s| {
        for _ in 0..engine.visual_sessions() * 2 {
            s.spawn(|| {
                loop {
                    let i = next.fetch_add(1, Ordering::Relaxed);
                    if i >= count {
                        break;
                    }
                    let input = make(i);
                    let e = engine
                        .embed_images(std::slice::from_ref(&input))
                        .expect("embedding")[0];
                    out.lock().expect("results")[i] = e;
                }
            });
        }
    });
    out.into_inner().expect("results")
}

/// Ranks with ties sharing their average rank.
fn ranks(values: &[f32]) -> Vec<f64> {
    let mut idx: Vec<usize> = (0..values.len()).collect();
    idx.sort_by(|&a, &b| values[a].total_cmp(&values[b]));
    let mut r = vec![0f64; values.len()];
    let mut i = 0;
    while i < idx.len() {
        let mut j = i;
        while j + 1 < idx.len() && values[idx[j + 1]] == values[idx[i]] {
            j += 1;
        }
        let avg = (i + j) as f64 / 2.0;
        for &k in &idx[i..=j] {
            r[k] = avg;
        }
        i = j + 1;
    }
    r
}

fn spearman(a: &[f32], b: &[f32]) -> f64 {
    let (ra, rb) = (ranks(a), ranks(b));
    let n = ra.len() as f64;
    let (ma, mb) = (ra.iter().sum::<f64>() / n, rb.iter().sum::<f64>() / n);
    let cov: f64 = ra.iter().zip(&rb).map(|(x, y)| (x - ma) * (y - mb)).sum();
    let va: f64 = ra.iter().map(|x| (x - ma).powi(2)).sum();
    let vb: f64 = rb.iter().map(|y| (y - mb).powi(2)).sum();
    cov / (va * vb).sqrt()
}

fn mean(values: impl Iterator<Item = f64>) -> f64 {
    let (sum, n) = values.fold((0.0, 0usize), |(s, n), v| (s + v, n + 1));
    if n == 0 { 0.0 } else { sum / n as f64 }
}

fn median(values: &[f32]) -> f32 {
    let mut v = values.to_vec();
    v.sort_by(f32::total_cmp);
    let mid = v.len() / 2;
    if v.is_empty() {
        0.0
    } else if v.len() % 2 == 1 {
        v[mid]
    } else {
        (v[mid - 1] + v[mid]) / 2.0
    }
}

/// Raw off-topic score of `outliers` (mean distance to the k nearest), to see the spread.
fn raw_spread(items: &[Embedding], k: usize) -> (f32, f32) {
    let k = k.min(items.len().saturating_sub(1)).max(1);
    let score: Vec<f32> = items
        .iter()
        .enumerate()
        .map(|(i, a)| {
            let mut d: Vec<f32> = items
                .iter()
                .enumerate()
                .filter(|&(j, _)| j != i)
                .map(|(_, b)| 1.0 - cosine(a, b))
                .collect();
            d.sort_by(f32::total_cmp);
            d[..k].iter().sum::<f32>() / k as f32
        })
        .collect();
    let m = median(&score);
    let dev: Vec<f32> = score.iter().map(|d| (d - m).abs()).collect();
    (m, 1.4826 * median(&dev))
}

/// Pairwise precision and recall of predicted groups against true burst labels.
fn pair_scores(groups: &[Vec<usize>], label: &dyn Fn(usize) -> Option<usize>) -> (f64, f64, usize) {
    let mut predicted = 0usize;
    let mut correct = 0usize;
    for g in groups {
        for (a, &x) in g.iter().enumerate() {
            for &y in &g[a + 1..] {
                predicted += 1;
                if label(x).is_some() && label(x) == label(y) {
                    correct += 1;
                }
            }
        }
    }
    (
        if predicted == 0 {
            1.0
        } else {
            correct as f64 / predicted as f64
        },
        correct as f64,
        predicted,
    )
}

fn precision_name(p: Precision) -> &'static str {
    match p {
        Precision::Fp32 => "fp32",
        Precision::Fp16 => "fp16",
        Precision::Int8 => "int8",
    }
}

fn top_z(report: &mut String, tag: &str, names: &[String], z: &[f32]) {
    let mut idx: Vec<usize> = (0..z.len()).collect();
    idx.sort_by(|&a, &b| z[b].total_cmp(&z[a]));
    let top: Vec<String> = idx
        .iter()
        .take(6)
        .map(|&i| format!("{}:{:.2}", names[i], z[i]))
        .collect();
    let _ = writeln!(report, "{tag}_z_top={}", top.join(","));
    let _ = writeln!(report, "{tag}_z_median={:.2}", median(z));
    let _ = writeln!(
        report,
        "{tag}_above_off_topic_z={}",
        z.iter().filter(|&&v| v > OFF_TOPIC_Z).count()
    );
}

struct Day {
    id: String,
    paths: Vec<PathBuf>,
    /// Capture time in seconds, in file (shooting) order.
    taken: Vec<i64>,
}

/// Seconds of an ISO `YYYY-MM-DDTHH:MM:SS` on a proleptic day count (only differences matter).
fn seconds(iso: &str) -> Option<i64> {
    let n = |r: std::ops::Range<usize>| iso.get(r)?.parse::<i64>().ok();
    let (y, mo, d) = (n(0..4)?, n(5..7)?, n(8..10)?);
    let (h, mi, s) = (n(11..13)?, n(14..16)?, n(17..19)?);
    let (y, mo) = if mo <= 2 { (y - 1, mo + 12) } else { (y, mo) };
    let days = 365 * y + y / 4 - y / 100 + y / 400 + (153 * (mo - 3) + 2) / 5 + d;
    Some(days * 86_400 + h * 3600 + mi * 60 + s)
}

fn load_days(root: &Path) -> Res<Vec<Day>> {
    let list = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/ai/calibration.json");
    let json: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&list)?)?;
    let mut days = Vec::new();
    for set in json["sets"]
        .as_array()
        .ok_or("calibration.json has no sets")?
    {
        let id = set["id"].as_str().ok_or("set without id")?.to_string();
        let mut paths = Vec::new();
        let mut taken = Vec::new();
        for image in set["images"].as_array().ok_or("set without images")? {
            let file = image["file"].as_str().ok_or("image without file")?;
            let path = root.join("calibration").join(&id).join(file);
            if !path.is_file() {
                return Err(format!("calibration image missing: {}", path.display()).into());
            }
            paths.push(path);
            taken.push(
                seconds(image["taken"].as_str().unwrap_or_default()).ok_or("bad taken time")?,
            );
        }
        days.push(Day { id, paths, taken });
    }
    Ok(days)
}

fn quantile(values: &[f32], q: f64) -> f32 {
    let mut v = values.to_vec();
    v.sort_by(f32::total_cmp);
    if v.is_empty() {
        return 0.0;
    }
    v[((v.len() - 1) as f64 * q).round() as usize]
}

fn mean_embedding(items: &[Embedding]) -> Embedding {
    let mut sum = [0f32; EMBED_DIM];
    for item in items {
        for (s, v) in sum.iter_mut().zip(item) {
            *s += v;
        }
    }
    let norm = sum
        .iter()
        .map(|x| x * x)
        .sum::<f32>()
        .sqrt()
        .max(f32::MIN_POSITIVE);
    sum.map(|x| x / norm)
}

/// EXIF burst labels of one day: consecutive shots at most `EXIF_BURST_GAP` seconds apart and
/// of the same orientation share a label. Returns the labels and the number of true pairs.
fn exif_bursts(taken: &[i64], portrait: &[bool]) -> (Vec<Option<usize>>, usize) {
    let n = taken.len();
    let mut label: Vec<Option<usize>> = vec![None; n];
    for i in 1..n {
        if taken[i] - taken[i - 1] <= EXIF_BURST_GAP && portrait[i] == portrait[i - 1] {
            let id = *label[i - 1].get_or_insert(i - 1);
            label[i] = Some(id);
        }
    }
    let mut sizes: BTreeMap<usize, usize> = BTreeMap::new();
    for l in label.iter().flatten() {
        *sizes.entry(*l).or_default() += 1;
    }
    let true_pairs = sizes.values().map(|k| k * (k - 1) / 2).sum();
    (label, true_pairs)
}

/// One photo day ready for the burst sweep: embeddings and capture times in shooting order,
/// and the reference labels (EXIF or by eye).
struct BurstDay {
    id: String,
    items: Vec<Embedding>,
    times: Vec<Option<f64>>,
    label: Vec<Option<usize>>,
    true_pairs: usize,
}

/// Labels of the day hand-labelled in `tests/ai/calibration-bursts.json`: its id and, for each
/// photo of that day (given by file name, in shooting order), its group.
fn hand_bursts(
    files_of: &dyn Fn(&str) -> Option<Vec<String>>,
) -> Res<(String, Vec<Option<usize>>, usize)> {
    let list = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/ai/calibration-bursts.json");
    let json: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&list)?)?;
    let day = json["day"]
        .as_str()
        .ok_or("calibration-bursts.json has no day")?;
    let files = files_of(day).ok_or("labelled day not in calibration.json")?;
    let mut label: Vec<Option<usize>> = vec![None; files.len()];
    let mut true_pairs = 0usize;
    for (g, group) in json["groups"]
        .as_array()
        .ok_or("calibration-bursts.json has no groups")?
        .iter()
        .enumerate()
    {
        let names = group.as_array().ok_or("group is not a list")?;
        for name in names {
            let name = name.as_str().ok_or("file name is not a string")?;
            let i = files
                .iter()
                .position(|f| f == name)
                .ok_or_else(|| format!("labelled file not in the day: {name}"))?;
            label[i] = Some(g);
        }
        true_pairs += names.len() * names.len().saturating_sub(1) / 2;
    }
    Ok((day.to_string(), label, true_pairs))
}

/// Pair precision and recall of `bursts(threshold, max_gap_s, window)` over the given days.
fn day_burst_scores(
    days: &[BurstDay],
    threshold: f32,
    max_gap_s: f64,
    window: usize,
) -> (f64, f64, usize) {
    let (mut correct, mut predicted, mut truth) = (0f64, 0usize, 0usize);
    for day in days {
        let order: Vec<usize> = (0..day.items.len()).collect();
        let groups = bursts(&day.items, &order, &day.times, threshold, max_gap_s, window);
        let (_, c, p) = pair_scores(&groups, &|i| day.label[i]);
        correct += c;
        predicted += p;
        truth += day.true_pairs;
    }
    (
        if predicted == 0 {
            1.0
        } else {
            correct / predicted as f64
        },
        if truth == 0 {
            1.0
        } else {
            correct / truth as f64
        },
        predicted,
    )
}

/// Photo days: the off-topic scores of coherent days, then of injected corpus photos, and the
/// burst threshold against EXIF bursts.
///
/// Off-topic criteria (controller ruling, 2026-10-23, after Mickaël kept `OFF_TOPIC_Z` = 3.8):
/// at most one flagged photo per 100 in each day (before and after injection), and at least
/// `INJECTED_FLAGGED_MIN` of the injected photos flagged and placed after every unflagged photo
/// by `coverage_order`. Bursts: the threshold and window are swept on the real days; the setting
/// must reach `BURST_PRECISION_MIN` and `BURST_RECALL_MIN` (precision first, since every burst
/// shot but the best is suggested for rejection).
fn calibration(
    report: &mut String,
    failures: &mut Vec<String>,
    engine: &Engine,
    root: &Path,
    photos: &[Photo],
    queries: &[(String, Embedding)],
) -> Res<()> {
    let days = load_days(root)?;
    // The corpus at the decoded edge, to inject; its category centroids pick what is off topic.
    let corpus = embed_all(engine, photos.len(), |i| {
        to_input(&load_rgb(&photos[i].path, Some(WORKING_EDGE)))
    });
    let mut categories: Vec<&str> = Vec::new();
    for p in photos {
        if !categories.contains(&p.category.as_str()) {
            categories.push(&p.category);
        }
    }
    let mut false_z: Vec<f32> = Vec::new();
    let mut injected_z: Vec<f32> = Vec::new();
    let mut crowded_days: Vec<String> = Vec::new();
    let (mut injected_ok, mut injected_runs, mut all_last) = (0usize, 0usize, 0usize);
    let mut burst_days: Vec<BurstDay> = Vec::new();
    for day in &days {
        let shapes = Mutex::new(vec![(0u32, 0u32); day.paths.len()]);
        let items = embed_all(engine, day.paths.len(), |i| {
            let img = load_rgb(&day.paths[i], Some(WORKING_EDGE));
            shapes.lock().expect("shapes")[i] = img.dimensions();
            to_input(&img)
        });
        let portrait: Vec<bool> = shapes
            .into_inner()
            .expect("shapes")
            .iter()
            .map(|&(w, h)| h > w)
            .collect();
        let n = items.len();
        // What the search filter keeps of a real day (report only: days are not labelled).
        let kept: Vec<String> = queries
            .iter()
            .map(|(word, query)| {
                let count = items
                    .iter()
                    .filter(|e| cosine(query, e) >= SEARCH_MIN)
                    .count();
                format!("{word}={count}")
            })
            .collect();
        let _ = writeln!(
            report,
            "search_filter_day_{}_kept_of_{n} {} (report only)",
            day.id,
            kept.join(" ")
        );
        let z = outliers(&items, OFF_TOPIC_K);
        let flags = z.iter().filter(|&&v| v > OFF_TOPIC_Z).count();
        false_z.extend_from_slice(&z);
        let _ = writeln!(
            report,
            "day_{}_photos={n} raw_spread={:.4} z_max={:.2} ({}) z_p99={:.2} z_p95={:.2} flagged={flags}",
            day.id,
            raw_spread(&items, OFF_TOPIC_K).1,
            z.iter().copied().fold(f32::MIN, f32::max),
            (0..n)
                .max_by(|&a, &b| z[a].total_cmp(&z[b]))
                .and_then(|i| day.paths[i].file_name())
                .map_or(String::new(), |f| f.to_string_lossy().into_owned()),
            quantile(&z, 0.99),
            quantile(&z, 0.95)
        );

        let (label, true_pairs) = exif_bursts(&day.taken, &portrait);
        burst_days.push(BurstDay {
            id: day.id.clone(),
            items: items.clone(),
            times: day.taken.iter().map(|&t| Some(t as f64)).collect(),
            label,
            true_pairs,
        });

        // Injection: the apple, then the corpus photo least like the day in each of the
        // categories least like it.
        let centre = mean_embedding(&items);
        let mut by_category: Vec<(&str, f32)> = categories
            .iter()
            .filter(|&&c| c != "apple")
            .map(|&c| {
                let members: Vec<Embedding> = (0..photos.len())
                    .filter(|&i| photos[i].category == c)
                    .map(|i| corpus[i])
                    .collect();
                (c, cosine(&centre, &mean_embedding(&members)))
            })
            .collect();
        by_category.sort_by(|a, b| a.1.total_cmp(&b.1));
        let mut chosen: Vec<usize> = (0..photos.len())
            .filter(|&i| photos[i].category == "apple")
            .collect();
        for (c, _) in by_category.iter().take(INJECTED - 1) {
            let far = (0..photos.len())
                .filter(|&i| photos[i].category == *c)
                .min_by(|&a, &b| {
                    cosine(&centre, &corpus[a]).total_cmp(&cosine(&centre, &corpus[b]))
                })
                .ok_or("empty category")?;
            chosen.push(far);
        }
        let mut mixed = items.clone();
        mixed.extend(chosen.iter().map(|&i| corpus[i]));
        let injected: Vec<usize> = (n..mixed.len()).collect();
        let zi = outliers(&mixed, OFF_TOPIC_K);
        let max_day = zi[..n].iter().copied().fold(f32::MIN, f32::max);
        let flags_after = zi[..n].iter().filter(|&&v| v > OFF_TOPIC_Z).count();
        if flags.max(flags_after) * 100 > n {
            crowded_days.push(format!("{} {}/{n}", day.id, flags.max(flags_after)));
        }
        false_z.extend_from_slice(&zi[..n]);
        injected_z.extend(injected.iter().map(|&i| zi[i]));
        let tail: BTreeSet<usize> = injected.iter().copied().collect();
        let (mut seeds_last, mut day_ok) = (0usize, 0usize);
        for seed in 0..STABILITY_SEEDS {
            // The scores of the amber mark, computed once, drive the order too.
            let order = coverage_order_with_outliers(&mixed, &zi, seed);
            let end: BTreeSet<usize> = order[order.len() - injected.len()..]
                .iter()
                .copied()
                .collect();
            seeds_last += usize::from(end == tail);
            // An injected photo counts when it is flagged and comes after every unflagged one.
            let last_unflagged = order
                .iter()
                .rposition(|&i| zi[i] <= OFF_TOPIC_Z)
                .unwrap_or(0);
            day_ok += injected
                .iter()
                .filter(|&&i| {
                    zi[i] > OFF_TOPIC_Z
                        && order.iter().position(|&o| o == i).unwrap_or(0) > last_unflagged
                })
                .count();
        }
        injected_ok += day_ok;
        injected_runs += injected.len() * STABILITY_SEEDS as usize;
        all_last += seeds_last;
        let shown: Vec<String> = chosen
            .iter()
            .zip(&injected)
            .map(|(&c, &i)| format!("{}:{:.2}", photos[c].file, zi[i]))
            .collect();
        let _ = writeln!(
            report,
            "day_{}_injected={} day_z_max_after={max_day:.2} day_flagged_after={flags_after} injected_flagged_and_set_aside={day_ok}/{} all_injected_last_seeds={seeds_last}/{STABILITY_SEEDS}",
            day.id,
            shown.join(","),
            injected.len() * STABILITY_SEEDS as usize
        );
    }
    let max_false = false_z.iter().copied().fold(f32::MIN, f32::max);
    let min_injected = injected_z.iter().copied().fold(f32::MAX, f32::min);
    let flagged_false = false_z.iter().filter(|&&v| v > OFF_TOPIC_Z).count();
    let missed = injected_z.iter().filter(|&&v| v <= OFF_TOPIC_Z).count();
    let share = injected_ok as f64 / injected_runs.max(1) as f64;
    let _ = writeln!(
        report,
        "days={} day_photos={}",
        days.len(),
        days.iter().map(|d| d.paths.len()).sum::<usize>()
    );
    let _ = writeln!(
        report,
        "days_false_z_max={max_false:.2} p99={:.2} p999={:.2} above_off_topic_z={flagged_false} (before and after injection) days_above_1_per_100={}",
        quantile(&false_z, 0.99),
        quantile(&false_z, 0.999),
        crowded_days.len()
    );
    let _ = writeln!(
        report,
        "days_injected_z_min={min_injected:.2} median={:.2} missed={missed}/{}",
        median(&injected_z),
        injected_z.len()
    );
    let _ = writeln!(
        report,
        "days_injected_flagged_and_set_aside={share:.3} ({injected_ok}/{injected_runs}, needs {INJECTED_FLAGGED_MIN}) all_injected_last_runs={all_last}/{} (report only)",
        days.len() * STABILITY_SEEDS as usize
    );
    if !crowded_days.is_empty() {
        failures.push(format!(
            "Days: more than 1 flagged photo per 100 above OFF_TOPIC_Z {OFF_TOPIC_Z} in {}",
            crowded_days.join(", ")
        ));
    }
    if share < INJECTED_FLAGGED_MIN {
        failures.push(format!(
            "Days: injected photos flagged and set aside {share:.3} (needs {INJECTED_FLAGGED_MIN})"
        ));
    }

    // Bursts: calibrated on the hand-labelled day (same scene, same moment), pairwise.
    let files_of = |id: &str| {
        days.iter().find(|d| d.id == id).map(|d| {
            d.paths
                .iter()
                .map(|p| {
                    p.file_name()
                        .map_or(String::new(), |f| f.to_string_lossy().into_owned())
                })
                .collect::<Vec<String>>()
        })
    };
    let (hand_id, hand_label, hand_pairs) = hand_bursts(&files_of)?;
    let hand_day = {
        let exif = burst_days
            .iter()
            .find(|d| d.id == hand_id)
            .ok_or("labelled day not measured")?;
        BurstDay {
            id: hand_id.clone(),
            items: exif.items.clone(),
            times: exif.times.clone(),
            label: hand_label,
            true_pairs: hand_pairs,
        }
    };
    let hand = std::slice::from_ref(&hand_day);
    let _ = writeln!(
        report,
        "bursts_labelled_day={hand_id} photos={} burst_shots={} groups={} true_pairs={hand_pairs} (tests/ai/calibration-bursts.json, pairwise scores)",
        hand_day.items.len(),
        hand_day.label.iter().flatten().count(),
        hand_day
            .label
            .iter()
            .flatten()
            .collect::<BTreeSet<_>>()
            .len()
    );
    // Best setting: precision >= BURST_PRECISION_MIN first, then the highest recall, then the
    // highest precision; remaining ties keep the smallest window, gap and threshold.
    let mut best: Option<(f64, f64, f32, f64, usize)> = None;
    for window in 1..=BURST_WINDOW_MAX {
        for gap in BURST_GAPS {
            let mut row = Vec::new();
            for t in 80..=99u32 {
                let threshold = t as f32 / 100.0;
                let (p, r, _) = day_burst_scores(hand, threshold, gap, window);
                row.push(format!("{threshold:.2}:{p:.3}/{r:.3}"));
                if p >= BURST_PRECISION_MIN && best.is_none_or(|b| r > b.1 || (r == b.1 && p > b.0))
                {
                    best = Some((p, r, threshold, gap, window));
                }
            }
            let _ = writeln!(
                report,
                "bursts_labelled_window_{window}_gap_{gap}s_precision/recall={}",
                row.join(" ")
            );
        }
    }
    match best {
        Some((bp, br, t, g, w)) => {
            let _ = writeln!(
                report,
                "bursts_labelled_best threshold={t:.2} max_gap_s={g} window={w} precision={bp:.3} recall={br:.3}"
            );
        }
        None => {
            let _ = writeln!(report, "bursts_labelled_best=none");
        }
    }
    let (p, r, predicted) = day_burst_scores(hand, BURST_THRESHOLD, BURST_MAX_GAP_S, BURST_WINDOW);
    let _ = writeln!(
        report,
        "bursts_labelled_at_constant threshold={BURST_THRESHOLD:.2} max_gap_s={BURST_MAX_GAP_S} window={BURST_WINDOW} predicted_pairs={predicted} precision={p:.3} recall={r:.3}"
    );
    if p < BURST_PRECISION_MIN || r < BURST_RECALL_MIN {
        failures.push(format!(
            "Bursts (labelled day {hand_id}): BURST_THRESHOLD {BURST_THRESHOLD:.2}, BURST_MAX_GAP_S {BURST_MAX_GAP_S}, window {BURST_WINDOW} give precision {p:.3}, recall {r:.3} (needs {BURST_PRECISION_MIN} and {BURST_RECALL_MIN}; best {})",
            best.map_or("none".to_string(), |(bp, br, t, g, w)| format!(
                "{t:.2}/{g} s/window {w}: {bp:.3}/{br:.3}"
            ))
        ));
    }
    let day_paths = &days
        .iter()
        .find(|d| d.id == hand_id)
        .ok_or("labelled day not loaded")?
        .paths;
    burst_ranking(report, failures, engine, &hand_day, day_paths)?;
    // Information: every day against EXIF bursts (<= EXIF_BURST_GAP s, same orientation).
    for day in &burst_days {
        let (p, r, predicted) = day_burst_scores(
            std::slice::from_ref(day),
            BURST_THRESHOLD,
            BURST_MAX_GAP_S,
            BURST_WINDOW,
        );
        let _ = writeln!(
            report,
            "day_{}_exif_bursts_at_constant true_pairs={} predicted_pairs={predicted} precision={p:.3} recall={r:.3} (report only)",
            day.id, day.true_pairs
        );
    }
    Ok(())
}

/// Precision and recall of the search filter per category for each swept threshold; picks the
/// threshold by the rule of [`SEARCH_RECALL_MIN`] and fails when `SEARCH_MIN` is another one or
/// keeps too many wrong photos. Returns the queries (categories, then absent words) for the
/// report on the real days.
fn search_filter(
    report: &mut String,
    failures: &mut Vec<String>,
    engine: &Engine,
    photos: &[Photo],
    items: &[Embedding],
) -> Res<Vec<(String, Embedding)>> {
    let mut categories: Vec<&str> = Vec::new();
    for p in photos {
        if !categories.contains(&p.category.as_str()) {
            categories.push(&p.category);
        }
    }
    let _ = writeln!(
        report,
        "search_filter_rule=highest threshold with mean recall >= {SEARCH_RECALL_MIN} over the categories of 5+ photos (query search_prompt, decoded edge {WORKING_EDGE}); gate: SEARCH_MIN is that threshold, mean precision >= {SEARCH_PRECISION_MIN}"
    );
    let mut queries: Vec<(String, Embedding)> = Vec::new();
    for word in categories.iter().copied().chain(SEARCH_ABSENT) {
        queries.push((word.to_string(), engine.embed_text(&search_prompt(word))?));
    }
    let sims: Vec<Vec<f32>> = queries
        .iter()
        .map(|(_, q)| items.iter().map(|e| cosine(q, e)).collect())
        .collect();
    let gated: Vec<usize> = (0..categories.len())
        .filter(|&c| {
            photos
                .iter()
                .filter(|p| p.category == categories[c])
                .count()
                >= 5
        })
        .collect();
    // (precision, recall) of category `c` at threshold `t`; precision 1 when nothing is kept.
    let measure = |c: usize, t: f32| -> (f64, f64) {
        let mut kept = 0usize;
        let mut hits = 0usize;
        for (i, &s) in sims[c].iter().enumerate() {
            if s >= t {
                kept += 1;
                hits += usize::from(photos[i].category == categories[c]);
            }
        }
        let size = photos
            .iter()
            .filter(|p| p.category == categories[c])
            .count();
        let precision = if kept == 0 {
            1.0
        } else {
            hits as f64 / kept as f64
        };
        (precision, hits as f64 / size.max(1) as f64)
    };
    let means = |t: f32| -> (f64, f64) {
        let pairs: Vec<(f64, f64)> = gated.iter().map(|&c| measure(c, t)).collect();
        (
            mean(pairs.iter().map(|p| p.0)),
            mean(pairs.iter().map(|p| p.1)),
        )
    };
    let (low, high, step) = SEARCH_SWEEP;
    let steps = ((high - low) / step).round() as usize;
    let mut picked: Option<f32> = None;
    for k in 0..=steps {
        let t = low + k as f32 * step;
        let (precision, recall) = means(t);
        let _ = writeln!(
            report,
            "search_filter_sweep t={t:.3} mean_precision={precision:.3} mean_recall={recall:.3}"
        );
        if recall >= SEARCH_RECALL_MIN {
            picked = Some(t);
        }
    }
    for (c, cat) in categories.iter().enumerate() {
        let (precision, recall) = measure(c, SEARCH_MIN);
        let kept = sims[c].iter().filter(|&&s| s >= SEARCH_MIN).count();
        let _ = writeln!(
            report,
            "search_filter_{cat}_at_{SEARCH_MIN:.3} precision={precision:.3} recall={recall:.3} kept={kept}{}",
            if gated.contains(&c) {
                ""
            } else {
                " (report only)"
            }
        );
    }
    for (q, (word, _)) in queries.iter().enumerate().skip(categories.len()) {
        let kept = sims[q].iter().filter(|&&s| s >= SEARCH_MIN).count();
        let best = sims[q].iter().copied().fold(f32::MIN, f32::max);
        let _ = writeln!(
            report,
            "search_filter_absent_{word}_kept={kept} of {} best_cosine={best:.3} (report only)",
            items.len()
        );
    }
    let (precision, recall) = means(SEARCH_MIN);
    let _ = writeln!(
        report,
        "search_filter_at_{SEARCH_MIN:.3} mean_precision={precision:.3} mean_recall={recall:.3} rule_picks={}",
        picked.map_or("none".to_string(), |t| format!("{t:.3}"))
    );
    match picked {
        Some(t) if (t - SEARCH_MIN).abs() < step / 2.0 => {}
        Some(t) => failures.push(format!(
            "Search filter: SEARCH_MIN is {SEARCH_MIN:.3}, the rule picks {t:.3}"
        )),
        None => failures.push(format!(
            "Search filter: no threshold from {low} reaches a mean recall of {SEARCH_RECALL_MIN}"
        )),
    }
    if precision < SEARCH_PRECISION_MIN {
        failures.push(format!(
            "Search filter: mean precision {precision:.3} at SEARCH_MIN (needs {SEARCH_PRECISION_MIN})"
        ));
    }
    Ok(queries)
}

/// Calibrates the likely reason of a low score (`sortypix_ai::reasons`) on the known
/// degradations, by the rule of [`REASON_CLEAN_CLAIMS_MAX`], [`REASON_SHOWN_MIN`] and
/// [`REASON_PRECISION_MIN`] (written before the measure). The folder is the clean corpus at
/// the decoded edge; each degraded photo is judged against it, as one bad photo among good
/// ones. `degraded` follows [`REASON_CASES`]. Fails when `REASON_MIN` or a probe's
/// `recognised()` disagrees with the rule.
fn reason_probe(
    report: &mut String,
    failures: &mut Vec<String>,
    engine: &Engine,
    clean: &[Embedding],
    degraded: &[Vec<Embedding>],
) -> Res<()> {
    let _ = writeln!(
        report,
        "reason_rule=lean_k = cos(image, bad_k) - cos(image, good_k); z_k = robust z of lean_k over the clean corpus at edge {WORKING_EDGE}; shown reason = strongest probe when its z >= REASON_MIN; REASON_MIN = smallest bar of {REASON_SWEEP:?} with at most {REASON_CLEAN_CLAIMS_MAX} of clean photos given a reason; a probe is recognised when shown for >= {REASON_SHOWN_MIN} of the photos with its degradation and >= {REASON_PRECISION_MIN} of the photos it is shown for carry it; unrecognised probes are never shown"
    );
    let probes = ReasonProbes::embed(|text| engine.embed_text(text))?;
    let leans: Vec<[f32; REASONS]> = clean.iter().map(|e| probes.lean(e)).collect();
    let folder = FolderLean::of(&leans);
    let strongest_of = |e: &Embedding| strongest(&folder.relative(&probes.lean(e)));
    let clean_top: Vec<(Reason, f32)> = clean.iter().map(strongest_of).collect();
    let sets_top: Vec<Vec<(Reason, f32)>> = degraded
        .iter()
        .map(|set| set.iter().map(strongest_of).collect())
        .collect();
    let (low, high, step) = REASON_SWEEP;
    let steps = ((high - low) / step).round() as usize;
    let mut picked: Option<f32> = None;
    for k in 0..=steps {
        let t = low + k as f32 * step;
        let claims = clean_top.iter().filter(|(_, z)| *z >= t).count();
        let share = claims as f64 / clean.len().max(1) as f64;
        let shown: Vec<String> = REASON_CASES
            .iter()
            .zip(&sets_top)
            .map(|((kind, reason), top)| {
                let right = top.iter().filter(|(r, z)| r == reason && *z >= t).count();
                format!("{kind}={:.3}", right as f64 / top.len().max(1) as f64)
            })
            .collect();
        let _ = writeln!(
            report,
            "reason_sweep t={t:.2} clean_claims={share:.3} right_shown {}",
            shown.join(" ")
        );
        if picked.is_none() && share <= REASON_CLEAN_CLAIMS_MAX {
            picked = Some(t);
        }
    }
    let _ = writeln!(
        report,
        "reason_min_rule_picks={} REASON_MIN={REASON_MIN:.2}",
        picked.map_or("none".to_string(), |t| format!("{t:.2}"))
    );
    match picked {
        Some(t) if (t - REASON_MIN).abs() < step / 2.0 => {}
        Some(t) => failures.push(format!(
            "Reasons: REASON_MIN is {REASON_MIN:.2}, the rule picks {t:.2}"
        )),
        None => failures.push(format!(
            "Reasons: no bar up to {high} keeps the clean claims under {REASON_CLEAN_CLAIMS_MAX}"
        )),
    }
    // At the constant: what each degradation gets, and the precision of each shown reason.
    let t = REASON_MIN;
    let label = |r: Reason| format!("{r:?}");
    let clean_claims: Vec<String> = Reason::ALL
        .iter()
        .map(|&r| {
            let c = clean_top.iter().filter(|(x, z)| *x == r && *z >= t).count();
            format!("{}={c}", label(r))
        })
        .collect();
    let _ = writeln!(
        report,
        "reason_clean_at_{t:.2} claims {} of {}",
        clean_claims.join(" "),
        clean.len()
    );
    for (case, ((kind, reason), top)) in REASON_CASES.iter().zip(&sets_top).enumerate() {
        let size = top.len().max(1) as f64;
        let first = top.iter().filter(|(r, _)| r == reason).count() as f64 / size;
        let right = top.iter().filter(|(r, z)| r == reason && *z >= t).count() as f64 / size;
        let wrong = top.iter().filter(|(r, z)| r != reason && *z >= t).count() as f64 / size;
        // Every photo with this degradation and the reason it got (shown or not).
        let mut tally: BTreeMap<String, usize> = BTreeMap::new();
        for (r, z) in top {
            let key = if *z >= t {
                label(*r)
            } else {
                "none".to_string()
            };
            *tally.entry(key).or_default() += 1;
        }
        let tally: Vec<String> = tally.iter().map(|(k, v)| format!("{k}={v}")).collect();
        let mut shown_for = clean_top
            .iter()
            .filter(|(r, z)| r == reason && *z >= t)
            .count();
        let mut carried = 0usize;
        for (other, other_top) in sets_top.iter().enumerate() {
            let count = other_top
                .iter()
                .filter(|(r, z)| r == reason && *z >= t)
                .count();
            shown_for += count;
            if other == case {
                carried = count;
            }
        }
        let precision = if shown_for == 0 {
            0.0
        } else {
            carried as f64 / shown_for as f64
        };
        let recognised = right >= REASON_SHOWN_MIN && precision >= REASON_PRECISION_MIN;
        let _ = writeln!(
            report,
            "reason_{kind}_expects={} ranks_first={first:.3} shown_right={right:.3} shown_wrong={wrong:.3} precision={precision:.3} recognised={recognised} constant={} got {}",
            label(*reason),
            reason.recognised(),
            tally.join(" ")
        );
        if recognised != reason.recognised() {
            failures.push(format!(
                "Reasons: {} recognised is {}, the rule says {recognised} (shown right {right:.3}, precision {precision:.3})",
                label(*reason),
                reason.recognised()
            ));
        }
    }
    Ok(())
}

/// Which photo of a burst the aesthetic score keeps, int8 against fp32, on the labelled bursts.
///
/// Rule (plan 2, task 2.4): when the best-scored photo of at least `BURST_TOP_AGREEMENT_MIN` of
/// the labelled bursts is the same with int8 and fp32, bursts add their other photos to the
/// rejection suggestions (`BURST_SUGGEST = true`); otherwise they are only marked, and the
/// question goes back to Mickael. The probe fails when `BURST_SUGGEST` disagrees with the rule.
/// Also reported: the gap between the first and second int8 scores of each burst, and the share
/// of bursts where that gap exceeds the measured int8 score error (mean |int8 - fp32| on the day).
fn burst_ranking(
    report: &mut String,
    failures: &mut Vec<String>,
    engine: &Engine,
    day: &BurstDay,
    paths: &[PathBuf],
) -> Res<()> {
    let head = engine.aesthetic();
    // One fp32 session is enough for one day of photos.
    let fp32 = Engine::load(&models_dir(), Precision::Fp32, 8)?;
    let reference = embed_all(&fp32, paths.len(), |i| {
        to_input(&load_rgb(&paths[i], Some(WORKING_EDGE)))
    });
    drop(fp32);
    let int8: Vec<f32> = day.items.iter().map(|e| head.score(e)).collect();
    let full: Vec<f32> = reference.iter().map(|e| head.score(e)).collect();
    let error = mean(
        int8.iter()
            .zip(&full)
            .map(|(a, b)| f64::from((a - b).abs())),
    );
    let mut groups: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    for (i, label) in day.label.iter().enumerate() {
        if let Some(g) = label {
            groups.entry(*g).or_default().push(i);
        }
    }
    let best = |scores: &[f32], members: &[usize]| {
        members
            .iter()
            .copied()
            .max_by(|&a, &b| scores[a].total_cmp(&scores[b]).then(b.cmp(&a)))
    };
    let (mut agree, mut wide) = (0usize, 0usize);
    let mut gaps: Vec<f32> = Vec::new();
    for members in groups.values() {
        if best(&int8, members) == best(&full, members) {
            agree += 1;
        }
        let mut sorted: Vec<f32> = members.iter().map(|&i| int8[i]).collect();
        sorted.sort_by(|a, b| b.total_cmp(a));
        let gap = sorted[0] - sorted.get(1).copied().unwrap_or(sorted[0]);
        if f64::from(gap) > error {
            wide += 1;
        }
        gaps.push(gap);
    }
    let count = groups.len().max(1) as f64;
    let agreement = agree as f64 / count;
    let rule = agreement >= BURST_TOP_AGREEMENT_MIN;
    let _ = writeln!(
        report,
        "bursts_labelled_top_agreement_int8_fp32={agreement:.3} ({agree} of {} bursts) needs {BURST_TOP_AGREEMENT_MIN} for suggestions",
        groups.len()
    );
    let _ = writeln!(
        report,
        "bursts_labelled_int8_score_error_mean={error:.3} top_gap_p50={:.3} top_gap_min={:.3} top_gap_max={:.3} gap_above_error={:.3} ({wide} of {})",
        quantile(&gaps, 0.5),
        gaps.iter().copied().fold(f32::MAX, f32::min),
        gaps.iter().copied().fold(f32::MIN, f32::max),
        wide as f64 / count,
        groups.len()
    );
    let _ = writeln!(
        report,
        "burst_suggest_rule={rule} BURST_SUGGEST={BURST_SUGGEST}"
    );
    if rule != BURST_SUGGEST {
        failures.push(format!(
            "Bursts: int8/fp32 agreement on the best photo {agreement:.3} (rule {BURST_TOP_AGREEMENT_MIN}) means BURST_SUGGEST = {rule}, the constant says {BURST_SUGGEST}"
        ));
    }
    Ok(())
}

/// Optional real corpus: report only, never fails.
fn real_corpus(report: &mut String, engine: &Engine, head: &Aesthetic) {
    let Some(dir) = std::env::var_os("SORTYPIX_AI_CORPUS").filter(|v| !v.is_empty()) else {
        let _ = writeln!(report, "real_corpus=not set");
        return;
    };
    let dir = PathBuf::from(dir);
    let run = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        real_corpus_measures(&dir, engine, head)
    }));
    match run {
        Ok(Ok(text)) => report.push_str(&text),
        Ok(Err(e)) => {
            let _ = writeln!(report, "real_corpus_error={e}");
        }
        Err(_) => {
            let _ = writeln!(report, "real_corpus_error=panic while measuring");
        }
    }
}

fn real_corpus_measures(dir: &Path, engine: &Engine, head: &Aesthetic) -> Res<String> {
    let labels: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(dir.join("labels.json"))?)?;
    let names = |v: &serde_json::Value| -> Vec<String> {
        v.as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|s| s.as_str().map(str::to_lowercase))
                    .collect()
            })
            .unwrap_or_default()
    };
    let burst_of: BTreeMap<String, usize> = labels["bursts"]
        .as_array()
        .map(|a| {
            a.iter()
                .enumerate()
                .flat_map(|(b, g)| names(g).into_iter().map(move |n| (n, b)))
                .collect()
        })
        .unwrap_or_default();
    let rejects: BTreeSet<String> = names(&labels["rejects"]).into_iter().collect();
    // File-name order stands for shooting order.
    let photos = sortypix_app::catalog::scan_names(dir)?;
    let file_names: Vec<Vec<String>> = photos
        .iter()
        .map(|p| {
            p.members
                .iter()
                .map(|m| {
                    m.file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .to_lowercase()
                })
                .collect()
        })
        .collect();
    let decoded: Vec<Option<Vec<f32>>> = photos
        .iter()
        .map(|p| {
            decode(p.display_source(false), Some(WORKING_EDGE))
                .ok()
                .and_then(|d| preprocess(&d.rgba, d.width, d.height).ok())
        })
        .collect();
    let usable: Vec<usize> = (0..photos.len())
        .filter(|&i| decoded[i].is_some())
        .collect();
    let emb = embed_all(engine, usable.len(), |k| {
        decoded[usable[k]].clone().unwrap_or_default()
    });
    let mut out = String::new();
    let _ = writeln!(out, "real_corpus_photos={}", photos.len());
    let _ = writeln!(
        out,
        "real_corpus_undecodable={}",
        photos.len() - usable.len()
    );
    let label = |k: usize| {
        file_names[usable[k]]
            .iter()
            .find_map(|n| burst_of.get(n).copied())
    };
    let order: Vec<usize> = (0..usable.len()).collect();
    let groups = bursts(
        &emb,
        &order,
        &[],
        BURST_THRESHOLD,
        BURST_MAX_GAP_S,
        BURST_WINDOW,
    );
    let (precision, correct, _) = pair_scores(&groups, &label);
    let true_pairs = {
        let mut per: BTreeMap<usize, usize> = BTreeMap::new();
        for k in 0..usable.len() {
            if let Some(b) = label(k) {
                *per.entry(b).or_default() += 1;
            }
        }
        per.values().map(|n| n * (n - 1) / 2).sum::<usize>()
    };
    let _ = writeln!(out, "real_corpus_burst_groups={}", groups.len());
    let _ = writeln!(out, "real_corpus_burst_pair_precision={precision:.3}");
    let recall = if true_pairs == 0 {
        1.0
    } else {
        correct / true_pairs as f64
    };
    let _ = writeln!(
        out,
        "real_corpus_burst_pair_recall={recall:.3} (labelled pairs {true_pairs})"
    );
    let scores: Vec<f32> = emb.iter().map(|e| head.score(e)).collect();
    let is_reject = |k: usize| file_names[usable[k]].iter().any(|n| rejects.contains(n));
    let (rej, kept): (Vec<usize>, Vec<usize>) = (0..usable.len()).partition(|&k| is_reject(k));
    let mut wins = 0usize;
    for &a in &kept {
        for &b in &rej {
            wins += usize::from(scores[a] > scores[b]);
        }
    }
    let pairs = kept.len() * rej.len();
    let _ = writeln!(out, "real_corpus_rejects={}", rej.len());
    let _ = writeln!(
        out,
        "real_corpus_kept_scores_higher_share={}",
        if pairs == 0 {
            "n/a".to_string()
        } else {
            format!("{:.3}", wins as f64 / pairs as f64)
        }
    );
    let z = outliers(&emb, OFF_TOPIC_K);
    let _ = writeln!(
        out,
        "real_corpus_above_off_topic_z={}",
        z.iter().filter(|&&v| v > OFF_TOPIC_Z).count()
    );
    Ok(out)
}

fn main() -> Res<()> {
    let dir = models_dir();
    let root = dir
        .parent()
        .ok_or("models dir has no parent")?
        .to_path_buf();
    let photos = load_corpus(&root)?;
    let names: Vec<String> = photos.iter().map(|p| p.file.clone()).collect();
    let mut report = String::new();
    let mut failures: Vec<String> = Vec::new();
    let _ = writeln!(report, "corpus_images={}", photos.len());
    let _ = writeln!(
        report,
        "decode=sortypix_app::decode, EXIF orientation applied"
    );
    let _ = writeln!(report, "threads={THREADS}");

    // Originals at corpus resolution, preprocessed once (about 600 KB each).
    let originals: Vec<Vec<f32>> = {
        let next = AtomicUsize::new(0);
        let out = Mutex::new(vec![Vec::new(); photos.len()]);
        std::thread::scope(|s| {
            for _ in 0..8 {
                s.spawn(|| {
                    loop {
                        let i = next.fetch_add(1, Ordering::Relaxed);
                        let Some(p) = photos.get(i) else { break };
                        let input = to_input(&load_rgb(&p.path, None));
                        out.lock().expect("inputs")[i] = input;
                    }
                });
            }
        });
        out.into_inner().expect("inputs")
    };

    // 1. Precision: fp32 is the reference; one engine loaded at a time.
    let mut by_precision: BTreeMap<&str, Vec<Embedding>> = BTreeMap::new();
    // The aesthetic head comes with the first engine (the same file for every precision).
    let mut head: Option<Aesthetic> = None;
    for p in [Precision::Fp32, Precision::Fp16, Precision::Int8] {
        let engine = Engine::load(&dir, p, THREADS)?;
        head.get_or_insert_with(|| engine.aesthetic().clone());
        let e = embed_all(&engine, originals.len(), |i| originals[i].clone());
        by_precision.insert(precision_name(p), e);
    }
    let head = head.ok_or("no engine loaded")?;
    let reference = &by_precision["fp32"];
    let reference_scores: Vec<f32> = reference.iter().map(|e| head.score(e)).collect();
    for name in ["int8", "fp16"] {
        let e = &by_precision[name];
        let cos: Vec<f64> = e
            .iter()
            .zip(reference)
            .map(|(a, b)| f64::from(cosine(a, b)))
            .collect();
        let cos_mean = mean(cos.iter().copied());
        let cos_min = cos.iter().copied().fold(f64::MAX, f64::min);
        let scores: Vec<f32> = e.iter().map(|x| head.score(x)).collect();
        let rho = spearman(&scores, &reference_scores);
        let score_delta = mean(
            scores
                .iter()
                .zip(&reference_scores)
                .map(|(a, b)| f64::from((a - b).abs())),
        );
        let _ = writeln!(report, "precision_{name}_vs_fp32_cosine_mean={cos_mean:.4}");
        let _ = writeln!(report, "precision_{name}_vs_fp32_cosine_min={cos_min:.4}");
        let _ = writeln!(report, "precision_{name}_vs_fp32_score_spearman={rho:.4}");
        let _ = writeln!(
            report,
            "precision_{name}_vs_fp32_score_abs_delta_mean={score_delta:.3}"
        );
        if name == "int8" {
            let ok = cos_mean >= INT8_COSINE_MIN && rho >= INT8_SPEARMAN_MIN;
            let _ = writeln!(
                report,
                "precision_int8_guard_pass={ok} (cosine >= {INT8_COSINE_MIN}, spearman >= {INT8_SPEARMAN_MIN})"
            );
            if !ok {
                failures.push(format!(
                    "Precision: int8 against fp32 cosine {cos_mean:.4}, spearman {rho:.4} (guard {INT8_COSINE_MIN}, {INT8_SPEARMAN_MIN})"
                ));
            }
        }
    }
    let _ = writeln!(
        report,
        "precision_default={}",
        precision_name(DEFAULT_PRECISION)
    );
    if DEFAULT_PRECISION != Precision::Int8 {
        failures.push(format!(
            "Precision: DEFAULT_PRECISION is {}, the accepted choice is int8",
            precision_name(DEFAULT_PRECISION)
        ));
    }

    // Everything below runs on the shipped precision.
    let engine = Engine::load(&dir, DEFAULT_PRECISION, THREADS)?;
    let base = by_precision[precision_name(DEFAULT_PRECISION)].clone();
    let base_scores: Vec<f32> = base.iter().map(|e| head.score(e)).collect();
    let _ = writeln!(
        report,
        "score_original_mean={:.3}",
        mean(base_scores.iter().map(|&s| f64::from(s)))
    );

    // 2. Degradations, at corpus resolution (report) then at the working edge (gated).
    let n = photos.len();
    let jobs = n * DEGRADATIONS.len();
    let degraded = embed_all(&engine, jobs, |j| {
        let (i, d) = (j / DEGRADATIONS.len(), j % DEGRADATIONS.len());
        let img = load_rgb(&photos[i].path, None);
        to_input(&degrade(&img, DEGRADATIONS[d], &mut seed(i, d)))
    });
    let working = embed_all(&engine, n * (DEGRADATIONS.len() + 1), |j| {
        let (i, d) = (j / (DEGRADATIONS.len() + 1), j % (DEGRADATIONS.len() + 1));
        let img = load_rgb(&photos[i].path, Some(WORKING_EDGE));
        if d == 0 {
            to_input(&img)
        } else {
            to_input(&degrade(&img, DEGRADATIONS[d - 1], &mut seed(i, d - 1)))
        }
    });
    for (d, kind) in DEGRADATIONS.iter().enumerate() {
        let scores: Vec<f32> = (0..n)
            .map(|i| head.score(&degraded[i * DEGRADATIONS.len() + d]))
            .collect();
        let wins = (0..n).filter(|&i| base_scores[i] > scores[i]).count();
        let share = wins as f64 / n as f64;
        let delta = mean((0..n).map(|i| f64::from(base_scores[i] - scores[i])));
        let _ = writeln!(
            report,
            "degradation_{kind}_original_higher_share={share:.3} ({wins}/{n}, report only)"
        );
        let _ = writeln!(report, "degradation_{kind}_score_drop_mean={delta:.3}");
        let stride = DEGRADATIONS.len() + 1;
        let w_orig: Vec<f32> = (0..n).map(|i| head.score(&working[i * stride])).collect();
        let w_deg: Vec<f32> = (0..n)
            .map(|i| head.score(&working[i * stride + d + 1]))
            .collect();
        let w_wins = (0..n).filter(|&i| w_orig[i] > w_deg[i]).count();
        let w_share = w_wins as f64 / n as f64;
        let _ = writeln!(
            report,
            "degradation_{kind}_at_edge_{WORKING_EDGE}_original_higher_share={w_share:.3} ({w_wins}/{n})"
        );
        if w_share < DEGRADATION_MIN {
            failures.push(format!(
                "Degradation {kind}: original scores higher in {w_share:.3} of pairs at the decoded edge (needs {DEGRADATION_MIN})"
            ));
        }
        let _ = writeln!(
            report,
            "degradation_{kind}_at_edge_{WORKING_EDGE}_score_drop_mean={:.3}",
            mean((0..n).map(|i| f64::from(w_orig[i] - w_deg[i])))
        );
    }

    // 3. Text search: "a photo of a {category}", share of the top 5 in the category.
    let mut categories: Vec<&str> = Vec::new();
    for p in &photos {
        if !categories.contains(&p.category.as_str()) {
            categories.push(&p.category);
        }
    }
    for cat in &categories {
        let query = engine.embed_text(&search_prompt(cat))?;
        let ranked = search(&query, &base);
        let size = photos.iter().filter(|p| p.category == *cat).count();
        let hits = ranked
            .iter()
            .take(TOP)
            .filter(|(i, _)| photos[*i].category == *cat)
            .count();
        let recall = hits as f64 / TOP.min(size) as f64;
        let _ = writeln!(
            report,
            "search_{cat}_recall_at_{TOP}={recall:.2} ({hits}/{})",
            TOP.min(size)
        );
        if recall < TOP_RECALL_MIN {
            let top: Vec<&str> = ranked
                .iter()
                .take(TOP)
                .map(|(i, _)| photos[*i].file.as_str())
                .collect();
            let _ = writeln!(report, "search_{cat}_top={}", top.join(","));
            failures.push(format!(
                "Search {cat}: recall at {TOP} = {recall:.2} (needs {TOP_RECALL_MIN})"
            ));
        }
    }

    // 3b. Search filter: the threshold the grid applies (`SEARCH_MIN`), at the decoded edge.
    let stride = DEGRADATIONS.len() + 1;
    let at_edge: Vec<Embedding> = (0..n).map(|i| working[i * stride]).collect();
    let queries = search_filter(&mut report, &mut failures, &engine, &photos, &at_edge)?;

    // 3c. Likely reason of a low score: text probes against the known degradations.
    let overexposed = embed_all(&engine, n, |i| {
        to_input(&expose(
            &load_rgb(&photos[i].path, Some(WORKING_EDGE)),
            OVEREXPOSURE_EV,
        ))
    });
    let mut sets: Vec<Vec<Embedding>> = (0..DEGRADATIONS.len())
        .map(|d| (0..n).map(|i| working[i * stride + d + 1]).collect())
        .collect();
    sets.push(overexposed);
    reason_probe(&mut report, &mut failures, &engine, &at_edge, &sets)?;

    // 4. Bursts: 30 synthetic bursts placed among the distinct photos of their category.
    let sources: Vec<usize> = (0..n)
        .filter(|&i| photos[i].category != "apple")
        .enumerate()
        .filter(|(k, _)| k % 4 == 0)
        .map(|(_, i)| i)
        .take(BURST_COUNT)
        .collect();
    let frames = embed_all(&engine, sources.len() * BURST_FRAMES, |j| {
        let (b, f) = (j / BURST_FRAMES, j % BURST_FRAMES);
        let img = load_rgb(&photos[sources[b]].path, None);
        to_input(&burst_frame(&img, &mut seed(1000 + b, f)))
    });
    // Items: the originals, then the frames; a burst source is replaced by its frames.
    let mut items = base.clone();
    items.extend_from_slice(&frames);
    let mut order = Vec::new();
    let mut burst_label: Vec<Option<usize>> = vec![None; items.len()];
    for i in 0..n {
        match sources.iter().position(|&s| s == i) {
            Some(b) => {
                for f in 0..BURST_FRAMES {
                    let item = n + b * BURST_FRAMES + f;
                    burst_label[item] = Some(b);
                    order.push(item);
                }
            }
            None => order.push(i),
        }
    }
    let true_pairs = sources.len() * BURST_FRAMES * (BURST_FRAMES - 1) / 2;
    let _ = writeln!(
        report,
        "bursts_synthetic={} frames={BURST_FRAMES} shift=+/-{BURST_SHIFT} ev=+/-{BURST_EV} window={BURST_WINDOW} sequence={} true_pairs={true_pairs} times=none (similarity only)",
        sources.len(),
        order.len()
    );
    let label = |i: usize| burst_label[i];
    let mut best: Option<u32> = None;
    for t in 80..=99u32 {
        let threshold = t as f32 / 100.0;
        let groups = bursts(
            &items,
            &order,
            &[],
            threshold,
            BURST_MAX_GAP_S,
            BURST_WINDOW,
        );
        let (precision, correct, predicted) = pair_scores(&groups, &label);
        let recall = correct / true_pairs as f64;
        let ok = precision >= BURST_PRECISION_MIN && recall >= BURST_RECALL_MIN;
        let _ = writeln!(
            report,
            "bursts_threshold_{threshold:.2}_precision={precision:.3} recall={recall:.3} predicted_pairs={predicted} pass={ok}"
        );
        if ok {
            best = Some(t);
        }
    }
    let _ = writeln!(
        report,
        "bursts_highest_passing_threshold={}",
        best.map_or("none".to_string(), |t| format!("{:.2}", t as f32 / 100.0))
    );
    // Sanity check only since 2026-10-23 (controller ruling): the threshold is calibrated on
    // the real photo days; the synthetic bursts must keep their precision at that threshold.
    let groups = bursts(
        &items,
        &order,
        &[],
        BURST_THRESHOLD,
        BURST_MAX_GAP_S,
        BURST_WINDOW,
    );
    let (precision, correct, _) = pair_scores(&groups, &label);
    let recall = correct / true_pairs as f64;
    let _ = writeln!(
        report,
        "bursts_synthetic_at_constant threshold={BURST_THRESHOLD:.2} window={BURST_WINDOW} precision={precision:.3} recall={recall:.3}"
    );
    if precision < BURST_PRECISION_MIN {
        failures.push(format!(
            "Bursts (synthetic): BURST_THRESHOLD {BURST_THRESHOLD:.2} gives precision {precision:.3}"
        ));
    }
    // The floor of the off-topic spread on a burst-heavy folder (the case it was made for).
    let seq: Vec<Embedding> = order.iter().map(|&i| items[i]).collect();
    let seq_names: Vec<String> = order
        .iter()
        .map(|&i| match burst_label[i] {
            Some(b) => format!("{}#{}", photos[sources[b]].file, (i - n) % BURST_FRAMES),
            None => photos[i].file.clone(),
        })
        .collect();
    let zs = outliers(&seq, OFF_TOPIC_K);
    let (m, spread) = raw_spread(&seq, OFF_TOPIC_K);
    let _ = writeln!(
        report,
        "burst_folder_raw_median={m:.4} raw_spread={spread:.4} min_spread={MIN_SPREAD}"
    );
    top_z(&mut report, "burst_folder", &seq_names, &zs);

    // 5. The apple among the 30 cats; then the off-topic distribution on the whole corpus.
    let cats: Vec<usize> = (0..n).filter(|&i| photos[i].category == "cat").collect();
    let apple = (0..n)
        .find(|&i| photos[i].category == "apple")
        .ok_or("no apple in corpus")?;
    let mut set: Vec<usize> = cats.clone();
    set.push(apple);
    let set_items: Vec<Embedding> = set.iter().map(|&i| base[i]).collect();
    let set_names: Vec<String> = set.iter().map(|&i| photos[i].file.clone()).collect();
    let apple_pos = set.len() - 1;
    let order = coverage_order(&set_items, COVERAGE_SEED);
    let last = order.last() == Some(&apple_pos);
    let stable = (0..STABILITY_SEEDS)
        .filter(|&s| coverage_order(&set_items, s).last() == Some(&apple_pos))
        .count();
    let z = outliers(&set_items, OFF_TOPIC_K);
    let max_cat = z[..apple_pos].iter().copied().fold(f32::MIN, f32::max);
    let (m, spread) = raw_spread(&set_items, OFF_TOPIC_K);
    let _ = writeln!(
        report,
        "apple_last_in_coverage_order={last} (seed {COVERAGE_SEED}, position {} of {})",
        order
            .iter()
            .position(|&i| i == apple_pos)
            .map_or(0, |p| p + 1),
        order.len()
    );
    let _ = writeln!(report, "apple_last_seeds={stable}/{STABILITY_SEEDS}");
    let _ = writeln!(
        report,
        "apple_z={:.2} max_cat_z={max_cat:.2} off_topic_z={OFF_TOPIC_Z}",
        z[apple_pos]
    );
    let _ = writeln!(
        report,
        "cats_apple_raw_median={m:.4} raw_spread={spread:.4} min_spread={MIN_SPREAD}"
    );
    // What the scores would be with the MAD alone (no floor), to judge MIN_SPREAD.
    let unfloored = spread.max(MIN_SPREAD) / spread;
    let _ = writeln!(
        report,
        "cats_apple_without_floor_apple_z={:.2} max_cat_z={:.2} (report only)",
        z[apple_pos] * unfloored,
        max_cat * unfloored
    );
    top_z(&mut report, "cats_apple", &set_names, &z);
    // Report only: Mickaël, 2026-10-23, keeps OFF_TOPIC_Z = 3.8 and accepts that an intruder
    // among very varied photos (the apple among 30 varied cats, z 3.43 with int8) is missed.
    let _ = writeln!(
        report,
        "apple_among_cats_flagged={} (accepted miss, report only)",
        z[apple_pos] > OFF_TOPIC_Z
    );
    let z_all = outliers(&base, OFF_TOPIC_K);
    let (m, spread) = raw_spread(&base, OFF_TOPIC_K);
    let _ = writeln!(
        report,
        "full_corpus_raw_median={m:.4} raw_spread={spread:.4} min_spread={MIN_SPREAD}"
    );
    top_z(&mut report, "full_corpus", &names, &z_all);

    // 6. Real photo days (tests/ai/calibration.json): false flags, then injected off-topic photos.
    calibration(
        &mut report,
        &mut failures,
        &engine,
        &root,
        &photos,
        &queries,
    )?;

    // 7. Optional real corpus.
    real_corpus(&mut report, &engine, &head);

    let _ = writeln!(
        report,
        "verdict={}",
        if failures.is_empty() { "PASS" } else { "FAIL" }
    );
    for f in &failures {
        let _ = writeln!(report, "failure={f}");
    }
    println!("{report}");
    std::fs::create_dir_all("target/reports")?;
    std::fs::write("target/reports/ai-quality.txt", &report)?;
    assert!(failures.is_empty(), "{}", failures.join("; "));
    Ok(())
}
