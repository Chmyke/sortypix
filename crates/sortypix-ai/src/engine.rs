//! Loading of ONNX Runtime, the two CLIP sessions and the aesthetic head.

use crate::aesthetic::Aesthetic;
use crate::error::AiError;
use crate::tokenizer::{CONTEXT, ClipTokenizer};
use ort::environment::Environment;
use ort::session::Session;
use ort::session::builder::GraphOptimizationLevel;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, MutexGuard, OnceLock, TryLockError};

/// Numeric precision of the visual model.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Precision {
    Fp32,
    Fp16,
    Int8,
}

/// Precision loaded by default.
///
/// Mickaël, 2026-10-23: int8 accepted, fp16/fp32 too slow on CPU. Quality probe, 120 corpus
/// photos against fp32: int8 mean embedding cosine 0.966, score Spearman 0.943 (the plan asked
/// 0.99 and 0.95; the probe now guards 0.96 and 0.93); fp16 1.000 and 1.000 but 4.17 images/s,
/// fp32 7.16, int8 16.01 in the speed probe (budget 10).
pub const DEFAULT_PRECISION: Precision = Precision::Int8;

/// The only ONNX Runtime line the models were validated against.
const RUNTIME_LINE: &str = "1.31.";

#[cfg(windows)]
const RUNTIME_FILE: &str = "onnxruntime.dll";
#[cfg(not(windows))]
const RUNTIME_FILE: &str = "libonnxruntime.so.1.31.0";

impl Precision {
    fn visual_file(self) -> &'static str {
        match self {
            Precision::Fp32 => "visual-fp32.onnx",
            Precision::Fp16 => "visual-fp16.onnx",
            Precision::Int8 => "visual-int8.onnx",
        }
    }

    /// There is no fp16 text model: fp16 pairs with the fp32 one.
    fn textual_file(self) -> &'static str {
        match self {
            Precision::Int8 => "textual-int8.onnx",
            Precision::Fp16 | Precision::Fp32 => "textual-fp32.onnx",
        }
    }
}

/// Every file `Engine::load` needs, runtime library first.
fn expected_files(dir: &Path, precision: Precision) -> Vec<PathBuf> {
    [
        RUNTIME_FILE,
        precision.visual_file(),
        precision.textual_file(),
        "tokenizer.json",
        "aesthetic-l14.bin",
    ]
    .iter()
    .map(|name| dir.join(name))
    .collect()
}

/// Accepts only the ONNX Runtime 1.31 line.
fn check_runtime_version(found: &str) -> Result<(), AiError> {
    if found.starts_with(RUNTIME_LINE) {
        Ok(())
    } else {
        Err(AiError::RuntimeLoad(format!(
            "ONNX Runtime {RUNTIME_LINE}x is required, found '{found}'"
        )))
    }
}

/// Asks the library for its own version (`OrtGetApiBase().GetVersionString()`) without
/// initialising anything, so a wrong runtime is refused before `ort` touches it.
fn read_runtime_version(library: &Path) -> Result<String, String> {
    type ApiBaseGetter = unsafe extern "C" fn() -> *const ort::sys::OrtApiBase;
    // SAFETY: loading a shared library runs its initialisers; the file is the runtime the user
    // installed. `OrtGetApiBase` takes no argument and returns a static table, and
    // `GetVersionString` returns a static NUL-terminated string that is copied before the
    // library handle is dropped.
    unsafe {
        let lib =
            libloading::Library::new(library).map_err(|e| format!("{}: {e}", library.display()))?;
        let getter: libloading::Symbol<ApiBaseGetter> = lib
            .get(b"OrtGetApiBase\0")
            .map_err(|_| format!("{} does not export OrtGetApiBase", library.display()))?;
        let base = getter();
        if base.is_null() {
            return Err("OrtGetApiBase returned null".to_string());
        }
        let text = ((*base).GetVersionString)();
        if text.is_null() {
            return Err("GetVersionString returned null".to_string());
        }
        Ok(std::ffi::CStr::from_ptr(text)
            .to_string_lossy()
            .into_owned())
    }
}

/// Process-wide runtime, initialised once by `ort`. Its outcome, success or failure (as the
/// message), is kept for the life of the process: `ort` cannot load a second library or retry.
static RUNTIME: OnceLock<Result<Environment, String>> = OnceLock::new();

/// The process runtime. Once `ort` has been initialised, its outcome is returned whatever
/// `library` is (the first library path wins). Before that, the version pre-check runs on each
/// call and its failure is not remembered: replacing a wrong library and retrying works.
fn runtime(library: &Path) -> Result<Environment, AiError> {
    if let Some(outcome) = RUNTIME.get() {
        return outcome.clone().map_err(AiError::RuntimeLoad);
    }
    let found = read_runtime_version(library).map_err(AiError::RuntimeLoad)?;
    check_runtime_version(&found)?;
    RUNTIME
        .get_or_init(|| init_runtime(library))
        .clone()
        .map_err(AiError::RuntimeLoad)
}

fn init_runtime(library: &Path) -> Result<Environment, String> {
    ort::init_from(library)
        .map_err(|e| e.to_string())?
        .with_name("sortypix")
        .with_telemetry(false)
        .build()
        .map_err(|e| e.to_string())
}

/// Length of a CLIP ViT-L/14 embedding.
pub const EMBED_DIM: usize = 768;

/// An L2-normalized image embedding.
pub type Embedding = [f32; EMBED_DIM];

/// Floats in one preprocessed image (3 x 224 x 224).
const IMAGE_LEN: usize = 3 * (crate::INPUT as usize) * (crate::INPUT as usize);

/// Intra-op threads given to each visual session (measured sweet spot on a 16-core part).
const THREADS_PER_SESSION: usize = 8;

/// Largest budget [`recommended_threads`] gives: the best measured configuration, three
/// sessions of eight threads on a 16-core, 32-thread desktop (Ryzen 9 7950X3D, `ai_speed`).
const MAX_RECOMMENDED_THREADS: usize = 3 * THREADS_PER_SESSION;

/// Intra-op budget to pass to [`Engine::load`] on a machine with `logical_cores` logical cores.
///
/// It leaves room for the interface and for decoding: three quarters of the cores, never more
/// than `logical_cores - 2`, at least 1. From 8 up it is rounded down to whole 8-thread
/// sessions (the engine builds `S = max(1, threads / 8)` of them) and capped at 24 (3 x 8), the
/// best configuration measured, on 32 logical cores; more was never measured. Examples: 32
/// cores give 24 (3 sessions), 16 give 8 (1 session), 8 give 6, 4 give 2, 2 or fewer give 1.
/// The 10 images/s budget is validated only on a 16-core desktop (Ryzen 9 7950X3D, 32 logical
/// cores).
pub fn recommended_threads(logical_cores: usize) -> usize {
    let budget = (logical_cores * 3 / 4)
        .min(logical_cores.saturating_sub(2))
        .max(1);
    if budget >= THREADS_PER_SESSION {
        (budget / THREADS_PER_SESSION * THREADS_PER_SESSION).min(MAX_RECOMMENDED_THREADS)
    } else {
        budget
    }
}

/// The visual and textual CLIP sessions and the aesthetic head, ready to run.
///
/// `Session::run` needs `&mut self` in this `ort` release, so each session sits behind a
/// `Mutex`: the public API stays `&self` and the engine can be shared between threads.
/// The visual model is a pool of `max(1, threads / 8)` sessions: one session caps near
/// 10 images/s whatever its thread count, so `embed_images` may run on several threads at once,
/// each call taking a free session. No lock is held across more than one `run` call.
pub struct Engine {
    visual: Vec<Mutex<Session>>,
    next: AtomicUsize,
    textual: Mutex<Session>,
    tokenizer: ClipTokenizer,
    aesthetic: Aesthetic,
}

fn lock(session: &Mutex<Session>) -> MutexGuard<'_, Session> {
    // A panic elsewhere cannot leave a session half-updated: keep using it.
    session.lock().unwrap_or_else(|e| e.into_inner())
}

impl Engine {
    /// Loads the runtime, the sessions of `precision`, the tokenizer and the aesthetic head
    /// from `dir`: when it succeeds, everything the engine offers is usable.
    ///
    /// Every expected file is checked before anything is loaded, and every failure is a typed
    /// error. `threads` is the total intra-op budget (see [`recommended_threads`]): it builds
    /// `max(1, threads / 8)` visual sessions of `min(threads, 8)` threads each (0 counts as 1),
    /// the text session getting the same share. Intra-op spinning is off so sessions and callers
    /// do not fight for cores.
    ///
    /// The runtime library is initialised once per process and its outcome is memoized: after
    /// the first initialisation by `ort` (success or failure), later calls get that same outcome,
    /// with the library of the first call even if `dir` differs (the first library path wins).
    /// Among failures, only those of that initialisation are memoized: missing files and a
    /// runtime of the wrong version are checked again on every call, so fixing them and retrying
    /// works.
    pub fn load(dir: &Path, precision: Precision, threads: usize) -> Result<Engine, AiError> {
        let dir = std::path::absolute(dir).unwrap_or_else(|_| dir.to_path_buf());
        let files = expected_files(&dir, precision);
        let missing: Vec<PathBuf> = files.iter().filter(|p| !p.is_file()).cloned().collect();
        if !missing.is_empty() {
            return Err(AiError::MissingFiles(missing));
        }
        let env = runtime(&files[0])?;
        let tokenizer = ClipTokenizer::load(&files[3])?;
        let aesthetic = Aesthetic::load(&files[4])?;
        let threads = threads.max(1);
        let per_session = threads.min(THREADS_PER_SESSION);
        let count = (threads / THREADS_PER_SESSION).max(1);
        let visual = (0..count)
            .map(|_| session(&env, &files[1], per_session).map(Mutex::new))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Engine {
            visual,
            next: AtomicUsize::new(0),
            textual: Mutex::new(session(&env, &files[2], per_session)?),
            tokenizer,
            aesthetic,
        })
    }

    /// Number of visual sessions, i.e. how many `embed_images` calls can run at once.
    pub fn visual_sessions(&self) -> usize {
        self.visual.len()
    }

    /// The aesthetic head loaded with the models (`aesthetic-l14.bin`).
    pub fn aesthetic(&self) -> &Aesthetic {
        &self.aesthetic
    }

    /// A free visual session: the first unlocked one from a rotating start, else wait on one.
    fn acquire_visual(&self) -> MutexGuard<'_, Session> {
        let count = self.visual.len();
        let start = self.next.fetch_add(1, Ordering::Relaxed) % count;
        for i in 0..count {
            match self.visual[(start + i) % count].try_lock() {
                Ok(guard) => return guard,
                Err(TryLockError::Poisoned(p)) => return p.into_inner(),
                Err(TryLockError::WouldBlock) => {}
            }
        }
        lock(&self.visual[start])
    }

    /// Embeds a batch of images, each the output of `preprocess` (3 x 224 x 224, NCHW).
    ///
    /// Returns one L2-normalized embedding per input, in order. An empty batch gives an
    /// empty result; a wrongly sized input or a model output that is not 768 columns is a
    /// typed error.
    pub fn embed_images(&self, inputs: &[Vec<f32>]) -> Result<Vec<Embedding>, AiError> {
        if inputs.is_empty() {
            return Ok(Vec::new());
        }
        if let Some(bad) = inputs.iter().find(|v| v.len() != IMAGE_LEN) {
            return Err(AiError::Image(format!(
                "preprocessed image has {} values, expected {IMAGE_LEN}",
                bad.len()
            )));
        }
        // The batch size is read under a short acquisition, released before any run.
        let session = self.acquire_visual();
        // The shipped models declare a fixed batch of 1; a dynamic axis (negative) takes it all.
        let chunk = session
            .inputs()
            .first()
            .and_then(|o| o.dtype().tensor_shape())
            .and_then(|shape| shape.first().copied())
            .and_then(|n| usize::try_from(n).ok())
            .filter(|&n| n > 0)
            .unwrap_or(inputs.len());
        drop(session);
        let mut result = Vec::with_capacity(inputs.len());
        for group in inputs.chunks(chunk) {
            // One session per run: a big batch never pins a session for its whole length.
            result.extend(run_visual(&mut self.acquire_visual(), group)?);
        }
        Ok(result)
    }

    /// Runs every visual session once on a blank image, one after the other. A session's first
    /// run allocates its working memory; several sessions doing it at once stalled the whole
    /// machine for a moment (UI benchmark during analysis, 2026-10-23).
    pub fn warm_up(&self) -> Result<(), AiError> {
        let blank = vec![0f32; IMAGE_LEN];
        for session in &self.visual {
            run_visual(&mut lock(session), std::slice::from_ref(&blank))?;
        }
        Ok(())
    }

    /// Embeds a text prompt: tokenizes it (truncated to 77 tokens) and returns the
    /// L2-normalized 768-value embedding. Empty text is valid.
    pub fn embed_text(&self, text: &str) -> Result<Embedding, AiError> {
        let fail = |e: &dyn std::fmt::Display| AiError::Model(format!("textual model: {e}"));
        let ids = self.tokenizer.encode(text);
        let mut session = lock(&self.textual);
        let input_name = session
            .inputs()
            .first()
            .map(|o| o.name().to_string())
            .ok_or_else(|| AiError::Model("textual model has no input".into()))?;
        let output_name = session
            .outputs()
            .first()
            .map(|o| o.name().to_string())
            .ok_or_else(|| AiError::Model("textual model has no output".into()))?;
        let tensor =
            ort::value::Tensor::from_array(([1, CONTEXT], ids.to_vec())).map_err(|e| fail(&e))?;
        let outputs = session
            .run(ort::inputs![input_name.as_str() => tensor])
            .map_err(|e| fail(&e))?;
        let (shape, data) = outputs[output_name.as_str()]
            .try_extract_tensor::<f32>()
            .map_err(|e| fail(&e))?;
        if shape.len() != 2 || shape[0] != 1 || shape[1] != EMBED_DIM as i64 {
            return Err(AiError::Model(format!(
                "textual output has shape {shape:?}, expected [1, {EMBED_DIM}]"
            )));
        }
        let row = data
            .as_chunks::<EMBED_DIM>()
            .0
            .first()
            .ok_or_else(|| AiError::Model("textual output is empty".into()))?;
        normalize(row, "textual")
    }
}

/// One run of a visual session on `group` (inputs of the right size, at most its batch size).
fn run_visual(session: &mut Session, group: &[Vec<f32>]) -> Result<Vec<Embedding>, AiError> {
    let fail = |e: &dyn std::fmt::Display| AiError::Model(format!("visual model: {e}"));
    let input_name = session
        .inputs()
        .first()
        .map(|o| o.name().to_string())
        .ok_or_else(|| AiError::Model("visual model has no input".into()))?;
    let output_name = session
        .outputs()
        .first()
        .map(|o| o.name().to_string())
        .ok_or_else(|| AiError::Model("visual model has no output".into()))?;
    let side = crate::INPUT as usize;
    let mut flat = Vec::with_capacity(group.len() * IMAGE_LEN);
    for v in group {
        flat.extend_from_slice(v);
    }
    let tensor = ort::value::Tensor::from_array(([group.len(), 3, side, side], flat))
        .map_err(|e| fail(&e))?;
    let outputs = session
        .run(ort::inputs![input_name.as_str() => tensor])
        .map_err(|e| fail(&e))?;
    let (shape, data) = outputs[output_name.as_str()]
        .try_extract_tensor::<f32>()
        .map_err(|e| fail(&e))?;
    if shape.len() != 2 || shape[0] != group.len() as i64 || shape[1] != EMBED_DIM as i64 {
        return Err(AiError::Model(format!(
            "visual output has shape {shape:?}, expected [{}, {EMBED_DIM}]",
            group.len()
        )));
    }
    data.as_chunks::<EMBED_DIM>()
        .0
        .iter()
        .map(|row| normalize(row, "visual"))
        .collect()
}

/// L2-normalizes one embedding row; a zero or non-finite row is a model error.
fn normalize(row: &[f32; EMBED_DIM], model: &str) -> Result<Embedding, AiError> {
    let norm = row.iter().map(|x| x * x).sum::<f32>().sqrt();
    if !norm.is_finite() || norm == 0.0 {
        return Err(AiError::Model(format!(
            "{model} output is not a usable embedding"
        )));
    }
    let mut out = [0f32; EMBED_DIM];
    for (o, x) in out.iter_mut().zip(row) {
        *o = x / norm;
    }
    Ok(out)
}

fn session(env: &Environment, model: &Path, threads: usize) -> Result<Session, AiError> {
    let fail = |e: &dyn std::fmt::Display| AiError::Model(format!("{}: {e}", model.display()));
    Session::builder(env)
        .map_err(|e| fail(&e))?
        .with_optimization_level(GraphOptimizationLevel::Level3)
        .map_err(|e| fail(&e))?
        .with_intra_threads(threads)
        .map_err(|e| fail(&e))?
        .with_inter_threads(1)
        .map_err(|e| fail(&e))?
        .with_intra_op_spinning(false)
        .map_err(|e| fail(&e))?
        .with_memory_pattern(true)
        .map_err(|e| fail(&e))?
        .commit_from_file(model)
        .map_err(|e| fail(&e))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file_names(paths: &[PathBuf]) -> Vec<String> {
        paths
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
            .collect()
    }

    #[test]
    fn missing_dir_lists_every_expected_file() {
        let dir = tempfile::tempdir().unwrap();
        match Engine::load(dir.path(), Precision::Int8, 1) {
            Err(AiError::MissingFiles(v)) => assert_eq!(
                file_names(&v),
                [
                    RUNTIME_FILE,
                    "visual-int8.onnx",
                    "textual-int8.onnx",
                    "tokenizer.json",
                    "aesthetic-l14.bin"
                ]
            ),
            other => panic!("expected MissingFiles, got {:?}", other.map(|_| ())),
        }
    }

    #[test]
    fn fp16_pairs_with_the_fp32_textual_model() {
        let names = file_names(&expected_files(Path::new("d"), Precision::Fp16));
        assert_eq!(names[1], "visual-fp16.onnx");
        assert_eq!(names[2], "textual-fp32.onnx");
    }

    #[test]
    fn garbage_runtime_is_an_error_not_a_crash() {
        let dir = tempfile::tempdir().unwrap();
        for p in expected_files(dir.path(), Precision::Fp32) {
            std::fs::write(p, vec![0u8; 4096]).unwrap();
        }
        let first = Engine::load(dir.path(), Precision::Fp32, 1);
        assert!(matches!(
            first,
            Err(AiError::RuntimeLoad(_)) | Err(AiError::Model(_))
        ));
        // The second call answers the same way (the pre-check refuses the file again).
        let second = Engine::load(dir.path(), Precision::Fp32, 1);
        assert!(matches!(second, Err(AiError::RuntimeLoad(_))));
    }

    #[test]
    fn recommended_threads_leave_room_and_fill_whole_sessions() {
        let cases = [
            (0, 1),
            (1, 1),
            (2, 1),
            (3, 1),
            (4, 2),
            (6, 4),
            (8, 6),
            (12, 8),
            (16, 8),
            (24, 16),
            (32, 24),
            (64, 24),
        ];
        for (cores, want) in cases {
            assert_eq!(recommended_threads(cores), want, "{cores} cores");
        }
        for cores in 3..=128 {
            let t = recommended_threads(cores);
            assert!(t >= 1 && t <= cores - 2, "{cores} cores: {t}");
        }
    }

    #[test]
    fn only_runtime_1_31_is_accepted() {
        assert!(check_runtime_version("1.31.0").is_ok());
        for bad in ["1.30.2", "2.0.0", ""] {
            assert!(matches!(
                check_runtime_version(bad),
                Err(AiError::RuntimeLoad(_))
            ));
        }
    }
}
