//! Speed probe of the AI pipeline against the real models.
//!
//! `cargo run -p sortypix-ai --release --example ai_speed`
//!
//! Needs the models in `models_dir()` (`$SORTYPIX_AI_DIR/models`). Writes
//! `target/reports/ai-speed.txt` and panics when a budget (times `SORTYPIX_BUDGET_SCALE`) fails.
//! Pipeline measured: decode threads feed one inference worker per visual session of the engine pool.
//! Then the analyses on synthetic embeddings: `coverage_order` at 20,000 items (off-topic scores
//! included, O(n^2)) and at 100,000 (above 20,000 the off-topic step is skipped: k-means only).

use image::{Rgb, RgbImage, codecs::jpeg::JpegEncoder};
use sortypix_ai::{
    EMBED_DIM, Embedding, Engine, OFF_TOPIC_K, Precision, coverage_order, models_dir, outliers,
    preprocess, recommended_threads,
};
use sortypix_app::decode::decode;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::sync_channel;
use std::time::Instant;

const FIXTURES: usize = 200;
const DIR: &str = "target/ai-speed";
const DECODE_EDGE: u32 = 448;
const BATCH: usize = 8;
const DECODE_THREADS: usize = 2;
const SAMPLE: usize = 40;
/// Synthetic embeddings for the analysis timings: the largest set, and its prefix for the O(n^2)
/// off-topic step (`coverage_order` computes it up to 20,000 items).
const ANALYSIS_LARGE: usize = 100_000;
const ANALYSIS_SMALL: usize = 20_000;
/// Scenes the synthetic embeddings are drawn around (about 250 photos each at 100,000).
const ANALYSIS_SCENES: usize = 400;
/// Budgets in seconds (times `SORTYPIX_BUDGET_SCALE`).
const COVERAGE_SMALL_BUDGET_S: f64 = 10.;
const COVERAGE_LARGE_BUDGET_S: f64 = 30.;

type Res<T> = Result<T, Box<dyn std::error::Error>>;

/// Landscape pattern of `examples/fixtures.rs` scaled up, plus noise for a realistic file size.
fn write_fixture(path: &Path, n: u32) -> Res<()> {
    let (w, h) = (4000u32, 3000u32);
    let (wf, hf) = (w as f32, h as f32);
    let horizon: Vec<f32> = (0..w)
        .map(|x| {
            let x = x as f32 * 0.3;
            hf * 0.54 + hf * 0.1 * ((x / 190.) + n as f32).sin() + hf * 0.075 * (x / 81.).sin()
        })
        .collect();
    let ridge: Vec<f32> = (0..w)
        .map(|x| hf * 0.7 + hf * 0.08 * ((x as f32 * 0.3 / 125.) + n as f32 * 0.3).sin())
        .collect();
    let (sx, sy) = (wf * (0.71 + (n % 4) as f32 * 0.02), hf * 0.225);
    let sun_r2 = (wf.min(hf) * 0.08).powi(2);
    let mut rng = 0x9E37_79B9_7F4A_7C15u64 ^ u64::from(n + 1);
    let mut image = RgbImage::new(w, h);
    for (x, y, p) in image.enumerate_pixels_mut() {
        rng ^= rng << 13;
        rng ^= rng >> 7;
        rng ^= rng << 17;
        let noise = (rng & 31) as i32 - 16;
        let (xf, yf) = (x as f32, y as f32);
        let sky = yf / hf;
        let base = if yf > ridge[x as usize] {
            [25 + (n * 5 % 50), 62 + (n * 3 % 35), 66 + (n * 4 % 25)]
        } else if yf > horizon[x as usize] {
            [55 + (n * 4 % 60), 94 + (n * 3 % 50), 108 + (n * 2 % 45)]
        } else if (xf - sx).powi(2) + (yf - sy).powi(2) < sun_r2 {
            [255, 220, 160]
        } else {
            [
                (115. + sky * 35.) as u32,
                (155. + sky * 25.) as u32,
                (176. + sky * 20.) as u32,
            ]
        };
        *p = Rgb(base.map(|c| (c as i32 + noise).clamp(0, 255) as u8));
    }
    // Written under a temporary name, then renamed: an interrupted run leaves no truncated file.
    let temp = path.with_extension("tmp");
    let mut file = std::io::BufWriter::new(std::fs::File::create(&temp)?);
    JpegEncoder::new_with_quality(&mut file, 90).encode_image(&image)?;
    std::io::Write::flush(&mut file)?;
    drop(file);
    std::fs::rename(&temp, path)?;
    Ok(())
}

/// The 200 fixtures, generated once and reused; written by several threads when missing.
fn fixtures() -> Res<Vec<PathBuf>> {
    std::fs::create_dir_all(DIR)?;
    let paths: Vec<PathBuf> = (0..FIXTURES)
        .map(|n| Path::new(DIR).join(format!("photo-{n:03}.jpg")))
        .collect();
    let next = AtomicUsize::new(0);
    std::thread::scope(|s| {
        for _ in 0..8 {
            s.spawn(|| {
                loop {
                    let n = next.fetch_add(1, Ordering::Relaxed);
                    let Some(path) = paths.get(n) else { break };
                    let n = n as u32;
                    let ok = std::fs::metadata(path)
                        .map(|m| m.len() > 0)
                        .unwrap_or(false);
                    if !ok {
                        write_fixture(path, n).expect("fixture written");
                    }
                }
            });
        }
    });
    Ok(paths)
}

fn p50(values: &mut [f64]) -> f64 {
    values.sort_by(|a, b| a.total_cmp(b));
    values[values.len() / 2]
}

fn ms(since: Instant) -> f64 {
    since.elapsed().as_secs_f64() * 1000.
}

/// Decodes at the reduced edge, then ramps down to the model input.
fn prepare(path: &Path) -> Res<Vec<f32>> {
    let d = decode(path, Some(DECODE_EDGE))?;
    Ok(preprocess(&d.rgba, d.width, d.height)?)
}

/// Process peak working set in MB.
#[cfg(windows)]
fn peak_mb() -> f64 {
    #[repr(C)]
    struct Counters {
        cb: u32,
        page_fault_count: u32,
        peak_working_set: usize,
        rest: [usize; 7],
    }
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetCurrentProcess() -> isize;
        fn K32GetProcessMemoryInfo(process: isize, counters: *mut Counters, cb: u32) -> i32;
    }
    let mut c = Counters {
        cb: std::mem::size_of::<Counters>() as u32,
        page_fault_count: 0,
        peak_working_set: 0,
        rest: [0; 7],
    };
    // SAFETY: `c` matches PROCESS_MEMORY_COUNTERS and `cb` is its size.
    let ok = unsafe { K32GetProcessMemoryInfo(GetCurrentProcess(), &mut c, c.cb) };
    assert!(ok != 0, "GetProcessMemoryInfo failed");
    c.peak_working_set as f64 / 1_048_576.
}

#[cfg(not(windows))]
fn peak_mb() -> f64 {
    let status = std::fs::read_to_string("/proc/self/status").unwrap_or_default();
    status
        .lines()
        .find_map(|l| l.strip_prefix("VmHWM:"))
        .and_then(|v| v.split_whitespace().next())
        .and_then(|kb| kb.parse::<f64>().ok())
        .map_or(0., |kb| kb / 1024.)
}

/// Full pipeline: decode threads feed `engine.visual_sessions()` inference workers, each
/// embedding one image at a time (so the pool runs sessions in parallel), then the aesthetic
/// head. Returns images per second.
fn end_to_end(engine: &Engine, paths: &[PathBuf]) -> Res<f64> {
    let head = engine.aesthetic();
    let workers = engine.visual_sessions();
    let next = AtomicUsize::new(0);
    let done = AtomicUsize::new(0);
    let (tx, rx) = sync_channel::<Vec<f32>>(workers * 2);
    let rx = Mutex::new(rx);
    let start = Instant::now();
    std::thread::scope(|s| {
        for _ in 0..DECODE_THREADS {
            let tx = tx.clone();
            let next = &next;
            s.spawn(move || {
                while let Some(path) = paths.get(next.fetch_add(1, Ordering::Relaxed)) {
                    let input = prepare(path).expect("decode and preprocess");
                    if tx.send(input).is_err() {
                        break;
                    }
                }
            });
        }
        drop(tx);
        for _ in 0..workers {
            s.spawn(|| {
                // The receiver lock is only held while waiting for the next image.
                loop {
                    // A temporary in a `while let` scrutinee would stay locked for the whole body.
                    let next = rx.lock().expect("receiver").recv();
                    let Ok(input) = next else { break };
                    let e = engine
                        .embed_images(std::slice::from_ref(&input))
                        .expect("embedding");
                    assert!(head.score(&e[0]).is_finite());
                    done.fetch_add(1, Ordering::Relaxed);
                }
            });
        }
    });
    assert_eq!(
        done.load(Ordering::Relaxed),
        paths.len(),
        "every image went through the pipeline"
    );
    Ok(paths.len() as f64 / start.elapsed().as_secs_f64())
}

struct Measured {
    decode_ms: f64,
    preprocess_ms: f64,
    infer_ms: f64,
    ips: f64,
}

fn projection(report: &mut String, tag: &str, ips: f64) {
    for n in [1_000f64, 30_000., 300_000.] {
        let _ = writeln!(
            report,
            "{tag}_projection_{}_minutes={:.1}",
            n as u64,
            n / ips / 60.
        );
    }
}

/// One configuration: `threads` is the total intra-op budget given to `Engine::load`.
fn measure(
    dir: &Path,
    precision: Precision,
    threads: usize,
    paths: &[PathBuf],
    inputs: &[Vec<f32>],
) -> Res<(Measured, usize)> {
    let engine = Engine::load(dir, precision, threads)?;
    // Warm-up: first runs allocate and optimize.
    engine.embed_images(&inputs[..BATCH])?;
    let mut decode_t = Vec::new();
    let mut pre_t = Vec::new();
    for path in paths.iter().take(SAMPLE) {
        let t = Instant::now();
        let d = decode(path, Some(DECODE_EDGE))?;
        decode_t.push(ms(t));
        let t = Instant::now();
        preprocess(&d.rgba, d.width, d.height)?;
        pre_t.push(ms(t));
    }
    // Sequential, one session at a time: the latency of one image.
    let mut infer_t = Vec::new();
    for batch in inputs.chunks(BATCH).filter(|b| b.len() == BATCH) {
        let t = Instant::now();
        let out: Vec<Embedding> = engine.embed_images(batch)?;
        assert_eq!(out.len(), BATCH);
        infer_t.push(ms(t) / BATCH as f64);
    }
    let ips = end_to_end(&engine, paths)?;
    Ok((
        Measured {
            decode_ms: p50(&mut decode_t),
            preprocess_ms: p50(&mut pre_t),
            infer_ms: p50(&mut infer_t),
            ips,
        },
        engine.visual_sessions(),
    ))
}

/// `count` L2-normalized embeddings drawn around [`ANALYSIS_SCENES`] random scenes, as a folder of
/// many photo series would give (xorshift, fixed seed: the same set on every run).
fn synthetic_embeddings(count: usize) -> Vec<Embedding> {
    let mut state = 0x2545_F491_4F6C_DD1Du64;
    let mut uniform = move || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        (state >> 40) as f32 / (1u64 << 24) as f32 * 2. - 1.
    };
    let normalize = |v: &mut Embedding| {
        let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt();
        v.iter_mut().for_each(|x| *x /= norm);
    };
    let mut scenes = vec![[0f32; EMBED_DIM]; ANALYSIS_SCENES];
    for scene in &mut scenes {
        scene.iter_mut().for_each(|x| *x = uniform());
        normalize(scene);
    }
    (0..count)
        .map(|i| {
            let scene = &scenes[i % ANALYSIS_SCENES];
            let mut v = [0f32; EMBED_DIM];
            // A perturbation of norm about 0.5 around the scene (cosine about 0.9 to it).
            for (x, c) in v.iter_mut().zip(scene) {
                *x = c + uniform() * 0.03;
            }
            normalize(&mut v);
            v
        })
        .collect()
}

/// Times the analyses and returns their budget failures.
fn analyses(report: &mut String, scale: f64) -> Vec<String> {
    let items = synthetic_embeddings(ANALYSIS_LARGE);
    let small = &items[..ANALYSIS_SMALL];
    let mut failures = Vec::new();
    let t = Instant::now();
    let z = outliers(small, OFF_TOPIC_K);
    let outliers_s = t.elapsed().as_secs_f64();
    assert_eq!(z.len(), ANALYSIS_SMALL);
    let _ = writeln!(report, "outliers_{ANALYSIS_SMALL}_seconds={outliers_s:.2}");
    for (set, budget) in [
        (small, COVERAGE_SMALL_BUDGET_S),
        (&items[..], COVERAGE_LARGE_BUDGET_S),
    ] {
        let n = set.len();
        let t = Instant::now();
        let order = coverage_order(set, 1);
        let seconds = t.elapsed().as_secs_f64();
        assert_eq!(order.len(), n, "coverage_order is a permutation");
        let _ = writeln!(report, "coverage_order_{n}_seconds={seconds:.2}");
        if seconds > budget * scale {
            failures.push(format!(
                "coverage_order on {n} embeddings took {seconds:.2} s (limit {:.1})",
                budget * scale
            ));
        }
    }
    failures
}

fn main() -> Res<()> {
    let dir = models_dir();
    let cores = std::thread::available_parallelism().map_or(4, |n| n.get());
    // Three 8-thread sessions on a 32-thread machine; fewer on smaller ones.
    let main_threads = recommended_threads(cores);
    let paths = fixtures()?;
    let inputs: Vec<Vec<f32>> = paths
        .iter()
        .take(32)
        .map(|p| prepare(p))
        .collect::<Res<_>>()?;
    let mut report = String::new();
    let _ = writeln!(report, "cores={cores}");
    let _ = writeln!(report, "recommended_threads={main_threads}");
    let _ = writeln!(
        report,
        "fixtures={FIXTURES} size=4000x3000 quality=90 decode_edge={DECODE_EDGE}"
    );
    let _ = writeln!(
        report,
        "decode_threads={DECODE_THREADS} infer_batch={BATCH} intra_op_spinning=off inter_op_threads=1"
    );
    let mut int8_peak = 0.;
    let mut int8_ips = 0.;
    let mut int8_config = String::new();
    for (name, precision) in [
        ("int8", Precision::Int8),
        ("fp16", Precision::Fp16),
        ("fp32", Precision::Fp32),
    ] {
        // The main config, then (int8 only, to stay cheap) one 8-thread session for comparison.
        let configs: &[usize] = if name == "int8" {
            &[main_threads, 8]
        } else {
            &[main_threads]
        };
        for &threads in configs {
            let (r, sessions) = measure(&dir, precision, threads, &paths, &inputs)?;
            let per_session = threads.min(8);
            let tag = format!("{name}_threads_{threads}");
            let _ = writeln!(report, "{tag}_sessions={sessions}");
            let _ = writeln!(report, "{tag}_threads_per_session={per_session}");
            let _ = writeln!(report, "{tag}_decode_ms_p50={:.2}", r.decode_ms);
            let _ = writeln!(report, "{tag}_preprocess_ms_p50={:.2}", r.preprocess_ms);
            let _ = writeln!(report, "{tag}_infer_ms_p50={:.2}", r.infer_ms);
            let _ = writeln!(report, "{tag}_images_per_second={:.2}", r.ips);
            let peak = peak_mb();
            let _ = writeln!(report, "{tag}_peak_mb_so_far={peak:.0}");
            projection(&mut report, &tag, r.ips);
            if name == "int8" && threads == main_threads {
                int8_peak = peak;
                int8_ips = r.ips;
                int8_config = format!("{sessions} sessions x {per_session} threads");
            }
        }
    }
    let _ = writeln!(
        report,
        "budget_int8_config={int8_config} (threads={main_threads})"
    );
    let _ = writeln!(report, "peak_mb={:.0}", peak_mb());
    let scale = std::env::var("SORTYPIX_BUDGET_SCALE")
        .ok()
        .and_then(|s| s.parse::<f64>().ok())
        .unwrap_or(1.);
    // After the models: the 300 MB of synthetic embeddings must not count in the int8 peak.
    let mut failures = analyses(&mut report, scale);
    println!("{report}");
    std::fs::create_dir_all("target/reports")?;
    std::fs::write("target/reports/ai-speed.txt", &report)?;
    // Peak is the int8 main run's own (it runs first); every budget is checked before panicking.
    if int8_ips < 10. / scale {
        failures.push(format!(
            "End-to-end int8 budget exceeded: {int8_ips:.2} images/s with {int8_config} (needs {:.2})",
            10. / scale
        ));
    }
    if int8_peak >= 1500. * scale {
        failures.push(format!(
            "Peak memory budget exceeded after int8: {int8_peak:.0} MB (limit {:.0})",
            1500. * scale
        ));
    }
    assert!(failures.is_empty(), "{}", failures.join("; "));
    Ok(())
}
