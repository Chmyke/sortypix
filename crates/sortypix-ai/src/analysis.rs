//! Pure analyses on L2-normalized embeddings: no model, no I/O.
//!
//! Every function is deterministic (ties are broken by index, the only randomness is a seeded
//! generator), never panics on empty or tiny inputs, and assumes NaN-free vectors without
//! crashing on them (sorting uses `total_cmp`). The O(n^2) and k-means work runs on all the
//! machine's cores (`std::thread::scope`); results do not depend on the number of threads.

use crate::engine::{EMBED_DIM, Embedding};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, PoisonError};

/// Robust z-score above which a photo is considered off topic.
///
/// Calibration, 2026-10-23 (int8, k = 10, 448 px decode), on 5 real photo days from Commons
/// (`tests/ai/calibration.json`, 482 photos) with the apple and two far corpus photos injected
/// into each: in the 4 coherent days the highest photo scores 3.20 and the lowest injected one
/// 4.46, and 3.8 sits mid-gap (about 0.6 from both sides). Not separated everywhere: in the
/// museum-heavy Bukhara day a close-up of an exhibition map scores 5.36 and the injected apple
/// 3.30, and the apple among the 30 corpus cats scores 3.43. Mickaël, 2026-10-23: 3.8 kept; he
/// accepts missing an intruder among very varied photos and flagging a genuinely atypical one.
/// Probe criteria met: at most 1 flagged photo per 100 in each day (Bukhara 1/120, others 0),
/// injected photos flagged and set aside by `coverage_order` in 0.933 of the runs (needs 0.90).
pub const OFF_TOPIC_Z: f32 = 3.8;

/// Smallest spread (in cosine distance) used to scale off-topic scores. It keeps folders that
/// mix tight bursts and single shots from flagging every single shot, since the MAD then measures
/// density, not topic.
///
/// Calibration, 2026-10-23 (int8, k = 10): the measured spread (1.4826 x MAD) is below the floor
/// in 4 of the 5 real photo days (0.012 to 0.038; Bukhara 0.052) and among the 30 cats (0.028), so
/// the floor sets the scale there. Lowering it would raise false flags: at 0.04 the highest
/// photo of the forest march day (3.20 now) would reach 4.0, above `OFF_TOPIC_Z`.
pub const MIN_SPREAD: f32 = 0.05;

/// Cosine at or above which two shots of the same scene belong to the same burst (with
/// [`BURST_MAX_GAP_S`] and [`BURST_WINDOW`]). A burst is the same scene taken close in time
/// (Mickaël, 2026-10-23).
///
/// Calibration, 2026-10-23 (int8, 448 px decode), on the forest march day labelled by eye
/// (`tests/ai/calibration-bursts.json`: 120 photos, 31 bursts, 171 pairs), thresholds
/// 0.80..=0.99, gaps 2/5/10/30/60 s, windows 1 to 5: the highest pair recall at a precision
/// of at least 0.95 is 0.871 at 0.91 and 10 s (precision 1.000; windows 2 to 4 alike), short of the 0.90
/// asked; the missed pairs are moving crowds and speakers with a cosine of 0.79 to 0.91. Without
/// capture times (30 synthetic bursts) 0.91 gives precision 1.000 and recall 1.000.
///
/// Caveat: the setting was calibrated and judged on the same hand-labelled day (in-sample, so the
/// recall is an optimistic figure) and there is no holdout day yet. Mickael, 2026-10-23: the
/// recall bar on that day is 0.85 (precision stays at 0.95).
pub const BURST_THRESHOLD: f32 = 0.91;

/// Whether the photos of a burst other than its best-scored one are suggested for rejection.
/// When false, bursts are only marked.
///
/// Decided by the quality probe (`ai_quality`, rule of plan 2 task 2.4): suggestions need the
/// best-scored photo to be the same with int8 and fp32 in at least 0.80 of the 31 labelled
/// bursts of the forest march day. Measured 2026-10-23 (448 px decode): 0.581 (18 of 31). The
/// first and second scores of a burst are close (median gap 0.080) next to the int8 score
/// error (mean |int8 - fp32| 0.132 on that day): the gap exceeds it in 9 bursts of 31. So
/// bursts are marked, not suggested; the question goes back to Mickael.
pub const BURST_SUGGEST: bool = false;

/// How many previous members of a group a new shot is compared with (calibrated with
/// [`BURST_THRESHOLD`]: windows 2 to 4 gave the same result on the labelled day).
pub const BURST_WINDOW: usize = 3;

/// Largest gap in seconds between two shots of one burst, when both capture times are known
/// (same calibration: 5 s caps the recall at 0.819 with precision 0.95, 30 s lets other scenes
/// in, precision 0.801 at 0.91).
pub const BURST_MAX_GAP_S: f64 = 10.0;

/// Neighbours `k` given to [`outliers`] when [`OFF_TOPIC_Z`] and [`MIN_SPREAD`] were calibrated,
/// and used by [`coverage_order`]: another `k` gives scores on another scale.
pub const OFF_TOPIC_K: usize = 10;

/// Maximum number of k-means groups used by [`coverage_order`].
const MAX_GROUPS: usize = 256;
/// Number of k-means refinement passes.
const KMEANS_ITERATIONS: usize = 10;
/// Largest folder for which [`coverage_order`] computes the O(n^2) off-topic scores.
pub const OUTLIER_LIMIT: usize = 20_000;
/// Rows per tile of the pairwise distance computation of [`outliers`] (64 x 3 KB fits in L2).
const TILE: usize = 64;
/// Largest `k` for which [`outliers`] computes each pair once and keeps a per-thread table of
/// `n * k` nearest distances; above it each row is computed on its own (twice the work).
const SYMMETRIC_MAX_K: usize = 32;
/// Upper bound of the per-thread nearest tables of [`outliers`], in floats (64 MB in total).
const NEAREST_BUDGET: usize = 16 << 20;
/// Smallest number of items worth a thread of its own (spawning costs tens of microseconds).
const MIN_ITEMS_PER_THREAD: usize = 256;

/// Cosine similarity of two L2-normalized embeddings (their dot product).
pub fn cosine(a: &Embedding, b: &Embedding) -> f32 {
    dot(a, b)
}

/// Dot product over 16 independent accumulators, which the compiler turns into SIMD lanes
/// (a single running sum cannot be vectorized without changing its rounding). Symmetric:
/// `dot(a, b)` and `dot(b, a)` are bit for bit equal.
///
/// On x86-64 processors with AVX the same code is compiled with 8-wide registers (checked once at
/// run time, no fused multiply-add): each lane adds the same products in the same order, so the
/// result is bit for bit the one of the baseline build, about twice as fast.
pub(crate) fn dot(a: &[f32], b: &[f32]) -> f32 {
    #[cfg(target_arch = "x86_64")]
    if std::arch::is_x86_feature_detected!("avx") {
        // SAFETY: the processor supports AVX, checked just above.
        return unsafe { dot_avx(a, b) };
    }
    dot_lanes(a, b)
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx")]
fn dot_avx(a: &[f32], b: &[f32]) -> f32 {
    dot_lanes(a, b)
}

#[inline(always)]
fn dot_lanes(a: &[f32], b: &[f32]) -> f32 {
    const LANES: usize = 16;
    let (a_chunks, a_rest) = a.as_chunks::<LANES>();
    let (b_chunks, b_rest) = b.as_chunks::<LANES>();
    let mut acc = [0f32; LANES];
    for (x, y) in a_chunks.iter().zip(b_chunks) {
        for lane in 0..LANES {
            acc[lane] += x[lane] * y[lane];
        }
    }
    let tail: f32 = a_rest.iter().zip(b_rest).map(|(x, y)| x * y).sum();
    acc.iter().sum::<f32>() + tail
}

/// Threads worth using for `items` units of work: all logical cores, at most one per
/// [`MIN_ITEMS_PER_THREAD`] units, at least one.
fn threads_for(items: usize) -> usize {
    let cores = std::thread::available_parallelism().map_or(1, std::num::NonZero::get);
    cores.min(items / MIN_ITEMS_PER_THREAD).max(1)
}

/// Runs `f(i, &mut out[i])` for every slot on several threads, which take chunks in turn. The
/// calling thread works too, so a thread the system refuses to start only slows things down.
fn par_each<T: Send>(out: &mut [T], f: impl Fn(usize, &mut T) + Sync) {
    let threads = threads_for(out.len());
    let chunk = out.len().div_ceil(threads * 4).max(1);
    let parts = Mutex::new(out.chunks_mut(chunk).enumerate());
    let work = || {
        loop {
            // The lock is released before the chunk is processed.
            let next = parts.lock().unwrap_or_else(PoisonError::into_inner).next();
            let Some((c, part)) = next else { break };
            for (j, slot) in part.iter_mut().enumerate() {
                f(c * chunk + j, slot);
            }
        }
    };
    std::thread::scope(|s| {
        for _ in 1..threads {
            // A refused thread is not an error: the others share its chunks.
            let _ = std::thread::Builder::new().spawn_scoped(s, work);
        }
        work();
    });
}

/// Cosine between a photo and the text query of [`search_prompt`] at or above which the photo
/// matches the search (the grid keeps only those, best first).
///
/// Calibration, 2026-10-23 (int8, 448 px decode), by the quality probe (`examples/ai_quality.rs`,
/// which fails when this is not the value its rule picks): the 120-photo corpus, one query per
/// category, thresholds 0.100 to 0.400 by 0.005. Rule written before the measure: the highest
/// threshold with a mean recall of at least 0.90 over the categories of five photos or more
/// (results are ranked best first, so an extra photo sits at the end while a missed one cannot be
/// seen). It picks 0.175: mean precision 0.913, mean recall 0.905 (0.180 drops to 0.886).
/// Caveat: seven well-separated categories; on mixed real days a word matches fewer photos.
pub const SEARCH_MIN: f32 = 0.175;

/// The text embedded for a search: "a photo of a <words>", the prompt the quality probe measured.
pub fn search_prompt(words: &str) -> String {
    format!("a photo of a {}", words.trim())
}

/// Ranks `items` by similarity to `query`, best first; equal scores keep index order.
pub fn search(query: &Embedding, items: &[Embedding]) -> Vec<(usize, f32)> {
    let mut ranked: Vec<(usize, f32)> = items
        .iter()
        .enumerate()
        .map(|(i, item)| (i, cosine(query, item)))
        .collect();
    ranked.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
    ranked
}

/// Groups consecutive shots of the same scene taken close in time.
///
/// `order` is the shooting order, provided by the caller (by capture time, else file order): it
/// lists item indices, and an index out of range or already seen earlier in `order` is ignored.
/// `times` holds the capture time of each item in seconds, indexed like `items` (missing, short or
/// non-finite entries count as unknown). A photo joins the current group when, for at least one
/// of the last `window` members of that group, the cosine reaches `threshold` and, when both
/// capture times are known, they are at most `max_gap_s` seconds apart (similarity alone
/// otherwise, as for generated images without EXIF). Otherwise it starts a new group. Only groups
/// of two or more photos are returned. A `window` of 0 is treated as 1.
pub fn bursts(
    items: &[Embedding],
    order: &[usize],
    times: &[Option<f64>],
    threshold: f32,
    max_gap_s: f64,
    window: usize,
) -> Vec<Vec<usize>> {
    let window = window.max(1);
    let time = |i: usize| times.get(i).copied().flatten().filter(|t| t.is_finite());
    // One bit per item: a photo belongs to at most one group.
    let mut seen = vec![0u64; items.len().div_ceil(64)];
    let mut first_time = |i: usize| {
        let (word, bit) = (i / 64, 1u64 << (i % 64));
        let fresh = seen[word] & bit == 0;
        seen[word] |= bit;
        fresh
    };
    let mut groups: Vec<Vec<usize>> = Vec::new();
    let mut current: Vec<usize> = Vec::new();
    for &index in order.iter().filter(|&&i| i < items.len() && first_time(i)) {
        let joins = current.iter().rev().take(window).any(|&member| {
            let close = match (time(member), time(index)) {
                (Some(a), Some(b)) => (a - b).abs() <= max_gap_s,
                _ => true,
            };
            close && cosine(&items[member], &items[index]) >= threshold
        });
        if !current.is_empty() && !joins {
            if current.len() >= 2 {
                groups.push(std::mem::take(&mut current));
            } else {
                current.clear();
            }
        }
        current.push(index);
    }
    if current.len() >= 2 {
        groups.push(current);
    }
    groups
}

/// Robust off-topic score of every item.
///
/// The raw score is the mean distance (1 - cosine) to the `k` nearest other items (`k` is
/// clamped to `n - 1`, and `k = 0` or fewer than two items give all zeros); it is turned into
/// `(d - median) / max(1.4826 * MAD, MIN_SPREAD)`. Because of the floor, a positive flag
/// (`z > OFF_TOPIC_Z`) also needs an absolute gap of `OFF_TOPIC_Z * MIN_SPREAD` over the median,
/// whatever the MAD. Callers flag only `z > OFF_TOPIC_Z`, with `k = OFF_TOPIC_K` (the scale the
/// threshold was calibrated on).
///
/// O(n^2 * dim) by design, on all cores, each pair computed once (for `k` up to 32): meant for
/// probes and folders of at most 20,000 photos. The result does not depend on the number of
/// threads.
pub fn outliers(items: &[Embedding], k: usize) -> Vec<f32> {
    let n = items.len();
    let k = k.min(n.saturating_sub(1));
    if k == 0 {
        return vec![0.0; n];
    }
    let score = if k <= SYMMETRIC_MAX_K {
        nearest_mean_symmetric(items, k)
    } else {
        nearest_mean_by_row(items, k)
    };
    let median = median_of(&score);
    let deviations: Vec<f32> = score.iter().map(|d| (d - median).abs()).collect();
    let mad = median_of(&deviations);
    let spread = (1.4826 * mad).max(MIN_SPREAD);
    score.iter().map(|d| (d - median) / spread).collect()
}

/// Mean of the `k` smallest values (`1 <= k <= len`), summed smallest first so the result
/// depends only on the values, not on their order.
fn mean_of_smallest(values: &mut [f32], k: usize) -> f32 {
    values.select_nth_unstable_by(k - 1, f32::total_cmp);
    let nearest = &mut values[..k];
    nearest.sort_unstable_by(f32::total_cmp);
    nearest.iter().sum::<f32>() / k as f32
}

/// Mean distance to the `k` nearest others, each row on its own (any `k`, twice the work).
fn nearest_mean_by_row(items: &[Embedding], k: usize) -> Vec<f32> {
    let mut score = vec![0f32; items.len()];
    par_each(&mut score, |i, slot| {
        let mut distances: Vec<f32> = items
            .iter()
            .enumerate()
            .filter(|&(j, _)| j != i)
            .map(|(_, other)| 1.0 - cosine(&items[i], other))
            .collect();
        *slot = mean_of_smallest(&mut distances, k);
    });
    score
}

/// The `k` smallest distances seen so far for every row (unordered), with each row's largest
/// kept apart so most candidates are refused by a single comparison.
struct Nearest {
    k: usize,
    distances: Vec<f32>,
    worst: Vec<f32>,
}

impl Nearest {
    fn new(n: usize, k: usize) -> Self {
        Self {
            k,
            distances: vec![f32::INFINITY; n * k],
            worst: vec![f32::INFINITY; n],
        }
    }

    fn offer(&mut self, row: usize, d: f32) {
        if d < self.worst[row] {
            let slots = &mut self.distances[row * self.k..(row + 1) * self.k];
            let mut largest = 0;
            for i in 1..slots.len() {
                if slots[i].total_cmp(&slots[largest]).is_gt() {
                    largest = i;
                }
            }
            slots[largest] = d;
            self.worst[row] = slots.iter().copied().fold(f32::NEG_INFINITY, f32::max);
        }
    }
}

/// Mean distance to the `k` nearest others, each pair computed once.
///
/// Rows are cut in tiles of [`TILE`]; a thread takes the next row tile and pairs it with itself
/// and every later tile, offering each distance to both rows of its own [`Nearest`] table. The
/// tables are then merged row by row: the `k` nearest of a row are among the `k` nearest each
/// thread saw, and they are summed in sorted order, so the result does not depend on the threads.
fn nearest_mean_symmetric(items: &[Embedding], k: usize) -> Vec<f32> {
    let n = items.len();
    let tiles = n.div_ceil(TILE);
    let threads = threads_for(n).min((NEAREST_BUDGET / (n * k)).max(1));
    let next = AtomicUsize::new(0);
    let work = || {
        let mut nearest = Nearest::new(n, k);
        loop {
            let tile = next.fetch_add(1, Ordering::Relaxed);
            if tile >= tiles {
                break;
            }
            let rows = tile * TILE..((tile + 1) * TILE).min(n);
            for other in tile..tiles {
                let columns = other * TILE..((other + 1) * TILE).min(n);
                for i in rows.clone() {
                    for j in columns.clone().filter(|&j| j > i) {
                        let d = 1.0 - cosine(&items[i], &items[j]);
                        nearest.offer(i, d);
                        nearest.offer(j, d);
                    }
                }
            }
        }
        nearest
    };
    let tables: Vec<Nearest> = std::thread::scope(|s| {
        // A thread the system refuses to start is skipped: the others take its tiles.
        let handles: Vec<_> = (1..threads)
            .filter_map(|_| std::thread::Builder::new().spawn_scoped(s, work).ok())
            .collect();
        let mut tables = vec![work()];
        for handle in handles {
            match handle.join() {
                Ok(table) => tables.push(table),
                Err(panic) => std::panic::resume_unwind(panic),
            }
        }
        tables
    });
    let mut score = vec![0f32; n];
    par_each(&mut score, |row, slot| {
        let mut candidates: Vec<f32> = tables
            .iter()
            .flat_map(|t| &t.distances[row * k..(row + 1) * k])
            .copied()
            .collect();
        *slot = mean_of_smallest(&mut candidates, k);
    });
    score
}

pub(crate) fn median_of(values: &[f32]) -> f32 {
    let mut sorted = values.to_vec();
    sorted.sort_by(f32::total_cmp);
    let mid = sorted.len() / 2;
    if sorted.len() % 2 == 1 {
        sorted[mid]
    } else {
        (sorted[mid - 1] + sorted[mid]) / 2.0
    }
}

/// Orders items so that any prefix covers the folder's content as widely as possible.
///
/// Computes the off-topic scores (`outliers(items, OFF_TOPIC_K)`) for at most [`OUTLIER_LIMIT`]
/// (20,000) items, then calls [`coverage_order_with_outliers`]. Above the limit the off-topic
/// step is skipped (no item counts as off topic), the O(n^2) `outliers` being replaced by a search
/// within k-means groups in plan 2. The `ai_speed` probe times it at 20,000 and 100,000 items.
pub fn coverage_order(items: &[Embedding], seed: u64) -> Vec<usize> {
    let z = if items.len() <= OUTLIER_LIMIT {
        outliers(items, OFF_TOPIC_K)
    } else {
        Vec::new()
    };
    coverage_order_with_outliers(items, &z, seed)
}

/// [`coverage_order`] with off-topic scores computed by the caller (the same
/// `outliers(items, OFF_TOPIC_K)` that marks photos off topic), so they are computed once.
///
/// Off-topic items first: those with `z > OFF_TOPIC_Z` are set aside and appended at the very
/// end, by ascending z (the most off-topic last), since k-means may otherwise fold a lone apple
/// into a group of cats. A `z` whose length differs from `items` (empty, for instance) is
/// ignored: no item is off topic then.
///
/// The others go through spherical k-means (`k = clamp(m / 8, 1, 256)` for `m` remaining items,
/// 10 iterations, k-means++ seeding driven by `seed`, on all cores). Groups of two or more items
/// are visited in turns, the largest group first, one representative per turn, each group walked
/// from the item closest to its center to the farthest. Lone items (a group of one) come after
/// them, the one least like the rest of the folder last. O(n * k * dim * iterations).
pub fn coverage_order_with_outliers(items: &[Embedding], z: &[f32], seed: u64) -> Vec<usize> {
    let n = items.len();
    if n == 0 {
        return Vec::new();
    }
    let z = if z.len() == n { z } else { &[] };
    let off_topic = |i: &usize| z.get(*i).is_some_and(|&v| v > OFF_TOPIC_Z);
    let (mut off_topic, kept): (Vec<usize>, Vec<usize>) = (0..n).partition(off_topic);
    off_topic.sort_by(|&a, &b| z[a].total_cmp(&z[b]).then(a.cmp(&b)));
    let mut order = Vec::with_capacity(n);
    if !kept.is_empty() {
        let subset: Vec<Embedding> = kept.iter().map(|&i| items[i]).collect();
        let k = (subset.len() / 8).clamp(1, MAX_GROUPS);
        order.extend(
            group_and_interleave(&subset, seed, k)
                .into_iter()
                .map(|i| kept[i]),
        );
    }
    order.extend(off_topic);
    order
}

/// [`coverage_order`] with an explicit number of groups (`1..=items.len()`).
fn group_and_interleave(items: &[Embedding], seed: u64, k: usize) -> Vec<usize> {
    let n = items.len();
    let k = k.clamp(1, n);
    let mut rng = SplitMix64::new(seed);
    let mut centers = seed_centers(items, k, &mut rng);
    let mut assignment = vec![0usize; n];
    for _ in 0..KMEANS_ITERATIONS {
        assign(items, &centers, &mut assignment);
        update_centers(items, &assignment, &mut centers);
    }
    assign(items, &centers, &mut assignment);

    let mut groups: Vec<Vec<usize>> = vec![Vec::new(); centers.len()];
    for (i, &g) in assignment.iter().enumerate() {
        groups[g].push(i);
    }
    for (g, members) in groups.iter_mut().enumerate() {
        let center = &centers[g];
        members.sort_by(|&a, &b| {
            cosine(center, &items[b])
                .total_cmp(&cosine(center, &items[a]))
                .then(a.cmp(&b))
        });
    }
    // Stable sort: equal sizes keep their group number order.
    groups.retain(|g| !g.is_empty());
    groups.sort_by_key(|g| std::cmp::Reverse(g.len()));
    let (regular, lone): (Vec<Vec<usize>>, Vec<Vec<usize>>) =
        groups.into_iter().partition(|g| g.len() >= 2);

    let mut order = Vec::with_capacity(n);
    let rounds = regular.first().map_or(0, Vec::len);
    for round in 0..rounds {
        order.extend(regular.iter().filter_map(|g| g.get(round).copied()));
    }
    let mut tail: Vec<usize> = lone.into_iter().flatten().collect();
    if !tail.is_empty() {
        let mean = normalized_mean(items.iter());
        tail.sort_by(|&a, &b| {
            cosine(&mean, &items[b])
                .total_cmp(&cosine(&mean, &items[a]))
                .then(a.cmp(&b))
        });
    }
    order.extend(tail);
    order
}

/// k-means++ seeding: the first center is random, each next one is drawn with a probability
/// proportional to the squared distance to the closest center already chosen.
fn seed_centers(items: &[Embedding], k: usize, rng: &mut SplitMix64) -> Vec<Embedding> {
    let n = items.len();
    let mut centers = Vec::with_capacity(k);
    centers.push(items[rng.below(n)]);
    let mut weight = vec![0f64; n];
    let first = centers[0];
    par_each(&mut weight, |i, w| *w = squared_distance(&items[i], &first));
    while centers.len() < k {
        let total: f64 = weight.iter().sum();
        let pick = if total > 0.0 {
            let mut target = rng.unit() * total;
            let mut chosen = n - 1;
            for (i, w) in weight.iter().enumerate() {
                if target < *w {
                    chosen = i;
                    break;
                }
                target -= w;
            }
            chosen
        } else {
            // Everything coincides with a center already: any item will do.
            rng.below(n)
        };
        centers.push(items[pick]);
        let newest = items[pick];
        par_each(&mut weight, |i, w| {
            *w = w.min(squared_distance(&items[i], &newest));
        });
    }
    centers
}

fn squared_distance(a: &Embedding, b: &Embedding) -> f64 {
    f64::from((2.0 - 2.0 * cosine(a, b)).max(0.0))
}

/// Assigns each item to its most similar center (the lowest index on ties), on all cores.
fn assign(items: &[Embedding], centers: &[Embedding], assignment: &mut [usize]) {
    par_each(assignment, |i, slot| {
        let mut best = 0;
        let mut best_score = f32::NEG_INFINITY;
        for (c, center) in centers.iter().enumerate() {
            let score = cosine(&items[i], center);
            if score.total_cmp(&best_score).is_gt() {
                best = c;
                best_score = score;
            }
        }
        *slot = best;
    });
}

/// Moves each center to the normalized mean of its members; an empty group keeps its center.
fn update_centers(items: &[Embedding], assignment: &[usize], centers: &mut [Embedding]) {
    let mut sums = vec![[0f32; EMBED_DIM]; centers.len()];
    let mut counts = vec![0usize; centers.len()];
    for (item, &g) in items.iter().zip(assignment) {
        counts[g] += 1;
        for (s, v) in sums[g].iter_mut().zip(item) {
            *s += v;
        }
    }
    for ((center, sum), count) in centers.iter_mut().zip(&sums).zip(&counts) {
        if *count > 0 {
            *center = normalized(sum);
        }
    }
}

fn normalized_mean<'a>(items: impl Iterator<Item = &'a Embedding>) -> Embedding {
    let mut sum = [0f32; EMBED_DIM];
    for item in items {
        for (s, v) in sum.iter_mut().zip(item) {
            *s += v;
        }
    }
    normalized(&sum)
}

fn normalized(v: &Embedding) -> Embedding {
    let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm > 0.0 {
        let mut out = *v;
        out.iter_mut().for_each(|x| *x /= norm);
        out
    } else {
        *v
    }
}

/// Small deterministic generator (SplitMix64): enough for seeding and for test data.
struct SplitMix64(u64);

impl SplitMix64 {
    fn new(seed: u64) -> Self {
        Self(seed)
    }

    fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Uniform in [0, 1).
    fn unit(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64
    }

    /// Uniform index in 0..n (n > 0).
    fn below(&mut self, n: usize) -> usize {
        (self.next_u64() % n as u64) as usize
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Unit vector along one axis.
    fn axis(i: usize) -> Embedding {
        let mut v = [0f32; EMBED_DIM];
        v[i] = 1.0;
        v
    }

    /// Normalized `center` plus a random perturbation of L2 norm about `noise`.
    fn around(center: &Embedding, noise: f32, rng: &mut SplitMix64) -> Embedding {
        let mut delta = [0f32; EMBED_DIM];
        for d in delta.iter_mut() {
            *d = (rng.unit() * 2.0 - 1.0) as f32;
        }
        let norm = delta.iter().map(|x| x * x).sum::<f32>().sqrt();
        let mut v = *center;
        for (x, d) in v.iter_mut().zip(delta) {
            *x += d / norm * noise;
        }
        normalized(&v)
    }

    fn random_unit(rng: &mut SplitMix64) -> Embedding {
        let mut v = [0f32; EMBED_DIM];
        for x in v.iter_mut() {
            *x = (rng.unit() * 2.0 - 1.0) as f32;
        }
        normalized(&v)
    }

    fn portraits(count: usize, rng: &mut SplitMix64) -> Vec<Embedding> {
        let center = axis(0);
        (0..count).map(|_| around(&center, 0.05, rng)).collect()
    }

    #[test]
    fn apple_among_portraits_is_last_and_off_topic() {
        let mut rng = SplitMix64::new(1);
        let mut items = portraits(500, &mut rng);
        items.push(axis(1));
        let apple = 500;

        let order = coverage_order(&items, 7);
        assert_eq!(order.len(), items.len());
        assert_eq!(order.last(), Some(&apple), "apple must come last");

        let z = outliers(&items, OFF_TOPIC_K);
        let max_portrait = z[..500].iter().copied().fold(f32::MIN, f32::max);
        println!("apple z = {}, max portrait z = {max_portrait}", z[apple]);
        assert!(z[apple] > OFF_TOPIC_Z);
        assert!(max_portrait <= OFF_TOPIC_Z);
    }

    #[test]
    fn off_topic_item_inside_a_group_still_comes_last() {
        // Three tight groups (12, 11, 7) and one item at a moderate distance from the smallest:
        // k-means (k = 3) folds it into that group, whose turns end before the others', so k-means
        // alone does not put it last; the off-topic step must.
        let mut rng = SplitMix64::new(12);
        let mut items = Vec::new();
        for (g, size) in [(0usize, 7usize), (1, 12), (2, 11)] {
            let center = axis(g * 10);
            items.extend((0..size).map(|_| around(&center, 0.05, &mut rng)));
        }
        let mut stray = axis(0);
        stray[1] = 1.5;
        let stray = normalized(&stray);
        items.push(stray);
        let index = items.len() - 1;
        let z = outliers(&items, OFF_TOPIC_K);
        assert!(z[index] > OFF_TOPIC_Z, "stray z = {}", z[index]);
        let k = (items.len() / 8).clamp(1, MAX_GROUPS);
        assert_ne!(
            group_and_interleave(&items, 3, k).last(),
            Some(&index),
            "the case must defeat k-means alone"
        );
        for seed in 0..8 {
            assert_eq!(coverage_order(&items, seed).last(), Some(&index));
        }
    }

    #[test]
    fn all_alike_has_no_off_topic() {
        let mut rng = SplitMix64::new(2);
        let items = portraits(500, &mut rng);
        let z = outliers(&items, OFF_TOPIC_K);
        let max = z.iter().copied().fold(f32::MIN, f32::max);
        println!("all alike max z = {max}");
        assert!(max <= OFF_TOPIC_Z);
    }

    #[test]
    fn all_different_has_no_off_topic() {
        let mut rng = SplitMix64::new(3);
        let items: Vec<Embedding> = (0..200).map(|_| random_unit(&mut rng)).collect();
        let z = outliers(&items, OFF_TOPIC_K);
        let max = z.iter().copied().fold(f32::MIN, f32::max);
        println!("all different max z = {max}");
        assert!(max <= OFF_TOPIC_Z);
    }

    #[test]
    fn two_densities_have_no_off_topic() {
        let mut rng = SplitMix64::new(8);
        let center = axis(0);
        let mut items: Vec<Embedding> = (0..300).map(|_| around(&center, 0.05, &mut rng)).collect();
        items.extend((0..200).map(|_| around(&center, 0.6, &mut rng)));
        let z = outliers(&items, OFF_TOPIC_K);
        let max = z.iter().copied().fold(f32::MIN, f32::max);
        println!("two densities max z = {max}");
        assert!(max <= OFF_TOPIC_Z);
    }

    #[test]
    fn coverage_alternates_groups() {
        let mut rng = SplitMix64::new(4);
        let mut items = Vec::new();
        let mut group_of = Vec::new();
        for (g, size) in [(0usize, 50usize), (1, 30), (2, 20)] {
            let center = axis(g * 10);
            for _ in 0..size {
                items.push(around(&center, 0.05, &mut rng));
                group_of.push(g);
            }
        }
        let order = group_and_interleave(&items, 11, 3);
        let firsts: Vec<usize> = order[..3].iter().map(|&i| group_of[i]).collect();
        assert_eq!(firsts, vec![0, 1, 2]);
    }

    #[test]
    fn coverage_is_deterministic_and_a_permutation() {
        let mut rng = SplitMix64::new(5);
        let items = portraits(100, &mut rng);
        let a = coverage_order(&items, 9);
        assert_eq!(a, coverage_order(&items, 9));
        let mut sorted = a.clone();
        sorted.sort_unstable();
        assert_eq!(sorted, (0..100).collect::<Vec<_>>());
    }

    #[test]
    fn bursts_split_on_scene_change() {
        let mut rng = SplitMix64::new(6);
        let (a, b, c) = (axis(0), axis(10), axis(20));
        let items = vec![
            around(&a, 0.02, &mut rng),
            around(&a, 0.02, &mut rng),
            around(&a, 0.02, &mut rng),
            around(&b, 0.02, &mut rng),
            around(&b, 0.02, &mut rng),
            around(&c, 0.02, &mut rng),
        ];
        let order: Vec<usize> = (0..6).collect();
        assert_eq!(
            bursts(&items, &order, &[], 0.95, 10.0, 2),
            vec![vec![0, 1, 2], vec![3, 4]]
        );
        assert_eq!(
            bursts(&items, &order, &[], 0.95, 10.0, 0),
            vec![vec![0, 1, 2], vec![3, 4]]
        );
        assert!(bursts(&items, &[0, 99], &[], 0.95, 10.0, 2).is_empty());
    }

    #[test]
    fn bursts_need_the_same_scene_close_in_time() {
        let mut rng = SplitMix64::new(7);
        let (a, b) = (axis(0), axis(10));
        let same = vec![around(&a, 0.02, &mut rng), around(&a, 0.02, &mut rng)];
        let order = [0, 1];
        // Same scene, 5 minutes apart: not a burst.
        assert!(bursts(&same, &order, &[Some(0.0), Some(300.0)], 0.95, 10.0, 3).is_empty());
        // Same scene, close in time: a burst.
        assert_eq!(
            bursts(&same, &order, &[Some(0.0), Some(2.0)], 0.95, 10.0, 3),
            vec![vec![0, 1]]
        );
        // Same scene, no capture times (or only one, or a non-finite one): similarity alone.
        for times in [
            vec![None, None],
            vec![Some(0.0), None],
            vec![Some(0.0), Some(f64::NAN)],
        ] {
            assert_eq!(
                bursts(&same, &order, &times, 0.95, 10.0, 3),
                vec![vec![0, 1]]
            );
        }
        // Different scenes, 1 second apart: not a burst.
        let different = vec![around(&a, 0.02, &mut rng), around(&b, 0.02, &mut rng)];
        assert!(bursts(&different, &order, &[Some(0.0), Some(1.0)], 0.95, 10.0, 3).is_empty());
    }

    #[test]
    fn search_ranks_nearest_first() {
        let items = vec![axis(0), axis(1), axis(0), axis(2)];
        let ranked = search(&axis(0), &items);
        let indices: Vec<usize> = ranked.iter().map(|r| r.0).collect();
        assert_eq!(indices, vec![0, 2, 1, 3]);
        assert!((ranked[0].1 - 1.0).abs() < 1e-6);
    }

    #[test]
    fn empty_and_tiny_inputs_never_panic() {
        let none: Vec<Embedding> = Vec::new();
        assert!(search(&axis(0), &none).is_empty());
        assert!(bursts(&none, &[], &[], 0.9, 10.0, 3).is_empty());
        assert!(outliers(&none, 10).is_empty());
        assert!(coverage_order(&none, 1).is_empty());
        let one = vec![axis(0)];
        assert_eq!(outliers(&one, 10), vec![0.0]);
        assert_eq!(coverage_order(&one, 1), vec![0]);
        let two = vec![axis(0), axis(1)];
        assert_eq!(outliers(&two, 10).len(), 2);
        assert_eq!(coverage_order(&two, 1).len(), 2);
    }

    #[test]
    fn bursts_ignore_an_index_seen_before() {
        let mut rng = SplitMix64::new(9);
        let a = axis(0);
        let items: Vec<Embedding> = (0..3).map(|_| around(&a, 0.02, &mut rng)).collect();
        // 1 comes back after 2, and 0 at the end: each photo is grouped once.
        let groups = bursts(&items, &[0, 1, 2, 1, 7, 0], &[], 0.95, 10.0, 3);
        assert_eq!(groups, vec![vec![0, 1, 2]]);
    }

    #[test]
    fn dot_matches_a_plain_sum_and_is_symmetric() {
        let mut rng = SplitMix64::new(10);
        let (a, b) = (random_unit(&mut rng), random_unit(&mut rng));
        let plain: f64 = a
            .iter()
            .zip(&b)
            .map(|(x, y)| f64::from(*x) * f64::from(*y))
            .sum();
        assert!((f64::from(cosine(&a, &b)) - plain).abs() < 1e-6);
        assert_eq!(cosine(&a, &b).to_bits(), cosine(&b, &a).to_bits());
        // Lengths that are not a multiple of the lanes.
        assert!((dot(&[1.0, 2.0, 3.0], &[4.0, 5.0, 6.0]) - 32.0).abs() < 1e-6);
        assert_eq!(dot(&[], &[]), 0.0);
        // The AVX build adds the same products in the same order: equal to the bit.
        #[cfg(target_arch = "x86_64")]
        if std::arch::is_x86_feature_detected!("avx") {
            for _ in 0..100 {
                let (a, b) = (random_unit(&mut rng), random_unit(&mut rng));
                // SAFETY: AVX support checked above.
                let avx = unsafe { dot_avx(&a, &b) };
                assert_eq!(avx.to_bits(), dot_lanes(&a, &b).to_bits());
            }
        }
    }

    /// Straightforward O(n^2) mean distance to the k nearest, in f64.
    fn reference_nearest_mean(items: &[Embedding], k: usize) -> Vec<f64> {
        (0..items.len())
            .map(|i| {
                let mut d: Vec<f64> = (0..items.len())
                    .filter(|&j| j != i)
                    .map(|j| {
                        let dot: f64 = items[i]
                            .iter()
                            .zip(&items[j])
                            .map(|(x, y)| f64::from(*x) * f64::from(*y))
                            .sum();
                        1.0 - dot
                    })
                    .collect();
                d.sort_by(f64::total_cmp);
                d[..k].iter().sum::<f64>() / k as f64
            })
            .collect()
    }

    #[test]
    fn both_nearest_paths_agree_with_the_reference() {
        // 700 items: several tiles and, on most machines, several threads.
        let mut rng = SplitMix64::new(11);
        let mut items = portraits(400, &mut rng);
        items.extend((0..300).map(|_| random_unit(&mut rng)));
        for k in [1, 10, SYMMETRIC_MAX_K] {
            let symmetric = nearest_mean_symmetric(&items, k);
            let by_row = nearest_mean_by_row(&items, k);
            // Same distances, summed in the same order: equal to the bit.
            assert_eq!(symmetric, by_row, "k = {k}");
            for (got, want) in symmetric.iter().zip(reference_nearest_mean(&items, k)) {
                assert!((f64::from(*got) - want).abs() < 1e-5, "k = {k}");
            }
        }
        // Above SYMMETRIC_MAX_K `outliers` takes the per-row path and still works.
        assert_eq!(outliers(&items, 50).len(), items.len());
    }

    #[test]
    fn precomputed_outliers_give_the_same_order() {
        let mut rng = SplitMix64::new(13);
        let mut items = portraits(300, &mut rng);
        items.push(axis(1));
        let z = outliers(&items, OFF_TOPIC_K);
        assert_eq!(
            coverage_order_with_outliers(&items, &z, 5),
            coverage_order(&items, 5)
        );
        // A z of the wrong length means no off-topic step, never a panic.
        let without = coverage_order_with_outliers(&items, &z[..10], 5);
        assert_eq!(without, coverage_order_with_outliers(&items, &[], 5));
        let mut sorted = without.clone();
        sorted.sort_unstable();
        assert_eq!(sorted, (0..items.len()).collect::<Vec<_>>());
    }
}
