//! Background analysis of the open folder, with no interface code.
//!
//! A pilot thread loads the [`Engine`], then starts one worker per visual session. Workers take
//! photos in grid order: the cached record when there is one, else decode (through the caller's
//! closure), [`preprocess`], embed, score and store. The pilot gathers the results of the
//! current generation and, when new photos are analysed, recomputes the folder analyses
//! (off-topic scores, bursts, coverage order). Every thread here runs below the interface (see
//! [`crate::priority`]). Nothing panics on a missing model, an unreadable file or a bad image:
//! the analysis becomes unavailable or skips the photo, and sorting carries on.

use crate::analysis::{
    BURST_MAX_GAP_S, BURST_THRESHOLD, BURST_WINDOW, OFF_TOPIC_K, OUTLIER_LIMIT, bursts,
    coverage_order_with_outliers, outliers,
};
use crate::cache::{ContentKey, DEFAULT_BUDGET, EmbeddingCache, MODEL_LABEL, Record};
use crate::engine::{DEFAULT_PRECISION, Embedding, Engine, recommended_threads};
use crate::error::AiError;
use crate::preprocess::preprocess;
use crate::reasons::{Likely, ReasonProbes, folder_reasons};
use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, OnceLock, RwLock, RwLockReadGuard};
use std::time::{Duration, Instant};

/// Decodes a photo to RGBA pixels, `(pixels, width, height)`, its long edge at most
/// [`DECODE_EDGE`]; `None` when the file cannot be read.
pub type Decoder = Arc<dyn Fn(&Path) -> Option<(Vec<u8>, u32, u32)> + Send + Sync>;

/// Long edge of the image the model sees before [`preprocess`]: the quality probe calibrated
/// every threshold on this decode, never on the full file.
pub const DECODE_EDGE: u32 = 448;

/// Shortest pause between two recomputations of the folder analyses.
const ANALYSIS_INTERVAL: Duration = Duration::from_secs(2);
/// The pause is also at least this many times the last computation, so the analyses of a big
/// folder take at most a fifth of the pilot's time and leave the cores to the embeddings.
const ANALYSIS_PAUSE_FACTOR: u32 = 4;
/// Seed of the coverage order: the same folder always gives the same order.
const COVERAGE_SEED: u64 = 1;

/// Where the analysis stands.
#[derive(Clone, Debug, PartialEq)]
pub enum Status {
    /// The model is loading.
    Loading,
    /// The model is loaded; photos are analysed as folders arrive.
    Ready,
    /// The model could not load: sorting works, the help is off. The message says why.
    Unavailable(String),
}

/// Analyses of the whole folder, over the photos analysed so far.
#[derive(Debug, Default)]
pub struct FolderAnalysis {
    pub generation: u64,
    /// Analysed photos in grid order: the index space of the fields below.
    pub paths: Vec<PathBuf>,
    /// Off-topic score of each photo (`outliers` with `OFF_TOPIC_K`); empty above
    /// `OUTLIER_LIMIT` photos, where it is not computed.
    pub outliers: Vec<f32>,
    /// Bursts in shooting order (capture time, else file name), two photos or more each.
    pub bursts: Vec<Vec<usize>>,
    /// Coverage order, off-topic photos last (`coverage_order_with_outliers`).
    pub coverage: Vec<usize>,
    /// Likely reason of a low score for each photo (`reasons::folder_reasons`); empty when
    /// the text probes could not be embedded.
    pub reasons: Vec<Likely>,
    /// Every photo of the folder was analysed (or skipped as unreadable).
    pub complete: bool,
}

/// What the analysis knows of the current folder.
#[derive(Default)]
pub struct Results {
    /// Generation given to [`Analyzer::set_folder`] these results belong to.
    pub generation: u64,
    /// Embedding and aesthetic score by path, for the photos analysed in this generation
    /// (photos sorted out of the folder since may remain: look up the paths you show).
    pub records: HashMap<PathBuf, Record>,
    /// Photos that could not be read or analysed.
    pub failed: usize,
    /// Latest folder analyses, if any were computed in this generation.
    pub analysis: Option<Arc<FolderAnalysis>>,
}

/// The model as the service uses it; a fake stands in for it in tests.
pub(crate) trait Model: Send + Sync {
    /// How many photos can be embedded at once.
    fn sessions(&self) -> usize;
    /// Embedding and aesthetic score of one preprocessed image.
    fn embed(&self, input: &[f32]) -> Result<Record, AiError>;
    fn embed_text(&self, text: &str) -> Result<Embedding, AiError>;
    /// Runs each session once, one after the other, before the analysis starts.
    fn warm_up(&self) {}
}

impl Model for Engine {
    fn sessions(&self) -> usize {
        self.visual_sessions()
    }

    fn embed(&self, input: &[f32]) -> Result<Record, AiError> {
        let embedding = self
            .embed_images(&[input.to_vec()])?
            .pop()
            .ok_or_else(|| AiError::Model("visual model returned nothing".into()))?;
        let score = self.aesthetic().score(&embedding);
        Ok(Record { embedding, score })
    }

    fn embed_text(&self, text: &str) -> Result<Embedding, AiError> {
        Engine::embed_text(self, text)
    }

    fn warm_up(&self) {
        // A failure here shows again, photo by photo, in the analysis itself.
        let _ = Engine::warm_up(self);
    }
}

type Loader = Box<dyn FnOnce() -> Result<Arc<dyn Model>, AiError> + Send>;

enum Message {
    SetFolder(u64, Vec<(PathBuf, Option<i64>)>),
    Done {
        generation: u64,
        path: PathBuf,
        /// Boxed: a record is 3 KB, the other messages a few words.
        record: Option<Box<Record>>,
    },
    Quit,
}

#[derive(Default)]
struct Queue {
    generation: u64,
    todo: VecDeque<PathBuf>,
    /// Taken by a worker and not reported yet, with their generation.
    in_flight: HashSet<(u64, PathBuf)>,
    closed: bool,
}

struct Shared {
    status: Mutex<Status>,
    model: OnceLock<Arc<dyn Model>>,
    results: RwLock<Results>,
    progress: Mutex<(usize, usize)>,
    queue: Mutex<Queue>,
    work: Condvar,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    // Nothing here is left half-updated by a panic: keep going with the data.
    mutex.lock().unwrap_or_else(|e| e.into_inner())
}

/// Handle on the background analysis; dropping it stops the threads after their current photo.
pub struct Analyzer {
    shared: Arc<Shared>,
    messages: Sender<Message>,
}

impl Analyzer {
    /// Starts the pilot thread, which loads the model of `models_dir` (int8,
    /// [`recommended_threads`]) and keeps records under `cache_root` (see
    /// [`crate::cache::cache_root`]). Returns at once; [`Analyzer::status`] tells how it went.
    pub fn start(models_dir: PathBuf, cache_root: PathBuf, decode: Decoder) -> Analyzer {
        let loader: Loader = Box::new(move || {
            let cores = std::thread::available_parallelism().map_or(1, |n| n.get());
            let engine = Engine::load(&models_dir, DEFAULT_PRECISION, recommended_threads(cores))?;
            Ok(Arc::new(engine) as Arc<dyn Model>)
        });
        Self::start_with(loader, cache_root, decode)
    }

    pub(crate) fn start_with(loader: Loader, cache_root: PathBuf, decode: Decoder) -> Analyzer {
        let shared = Arc::new(Shared {
            status: Mutex::new(Status::Loading),
            model: OnceLock::new(),
            results: RwLock::new(Results::default()),
            progress: Mutex::new((0, 0)),
            queue: Mutex::new(Queue::default()),
            work: Condvar::new(),
        });
        let (messages, inbox) = mpsc::channel();
        let pilot = Pilot {
            shared: shared.clone(),
            inbox,
            outbox: messages.clone(),
            cache: Arc::new(EmbeddingCache::new(cache_root, DEFAULT_BUDGET)),
            decode,
            generation: 0,
            photos: Vec::new(),
            members: HashSet::new(),
            failed: HashSet::new(),
            dirty: false,
            completed: false,
            next_analysis: Instant::now(),
            probes: None,
        };
        let spawned = std::thread::Builder::new()
            .name("sortypix-ai-pilot".into())
            .spawn(move || pilot.run(loader));
        if let Err(e) = spawned {
            *lock(&shared.status) = Status::Unavailable(format!("could not start: {e}"));
        }
        Analyzer { shared, messages }
    }

    /// Replaces the photos to analyse, in grid order, with their EXIF capture times in
    /// seconds. A new `generation` (a folder opened) drops everything known of the previous
    /// one, results still on their way included; the same generation (photos sorted out,
    /// order changed) keeps what was analysed and queues only the rest.
    pub fn set_folder(&self, generation: u64, photos: Vec<(PathBuf, Option<i64>)>) {
        let _ = self.messages.send(Message::SetFolder(generation, photos));
    }

    pub fn status(&self) -> Status {
        lock(&self.shared.status).clone()
    }

    /// Photos of the current folder analysed or skipped, and photos in it.
    pub fn progress(&self) -> (usize, usize) {
        *lock(&self.shared.progress)
    }

    /// The results of the current generation. Hold the guard briefly: the pilot waits for it
    /// to store new results.
    pub fn results(&self) -> RwLockReadGuard<'_, Results> {
        self.shared
            .results
            .read()
            .unwrap_or_else(|e| e.into_inner())
    }

    /// Embeds `text` on a thread of its own; the answer arrives on the returned channel. An
    /// error when the model is not loaded (yet, or at all).
    pub fn embed_text(&self, text: &str) -> Receiver<Result<Embedding, AiError>> {
        let (answer, receiver) = mpsc::channel();
        match self.shared.model.get() {
            Some(model) => {
                let model = model.clone();
                let text = text.to_owned();
                let spawned = std::thread::Builder::new()
                    .name("sortypix-ai-text".into())
                    .spawn({
                        let answer = answer.clone();
                        move || {
                            let _ = answer.send(model.embed_text(&text));
                        }
                    });
                if let Err(e) = spawned {
                    let _ = answer.send(Err(AiError::Model(format!("could not start: {e}"))));
                }
            }
            None => {
                let _ = answer.send(Err(AiError::Model("the model is not loaded".into())));
            }
        }
        receiver
    }
}

impl Drop for Analyzer {
    fn drop(&mut self) {
        lock(&self.shared.queue).closed = true;
        self.shared.work.notify_all();
        let _ = self.messages.send(Message::Quit);
    }
}

/// State of the pilot thread.
struct Pilot {
    shared: Arc<Shared>,
    inbox: Receiver<Message>,
    outbox: Sender<Message>,
    cache: Arc<EmbeddingCache>,
    decode: Decoder,
    generation: u64,
    /// Photos of the current folder, grid order, with capture times.
    photos: Vec<(PathBuf, Option<i64>)>,
    members: HashSet<PathBuf>,
    failed: HashSet<PathBuf>,
    /// New results since the last folder analyses.
    dirty: bool,
    /// The last photo of the folder was just analysed.
    completed: bool,
    next_analysis: Instant,
    /// Text embeddings of the reason probes, once the model is loaded.
    probes: Option<ReasonProbes>,
}

impl Pilot {
    fn run(mut self, loader: Loader) {
        crate::priority::lower_current_thread();
        let model = match loader() {
            Ok(model) => model,
            Err(e) => {
                *lock(&self.shared.status) = Status::Unavailable(e.to_string());
                return;
            }
        };
        model.warm_up();
        let _ = self.shared.model.set(model.clone());
        for index in 0..model.sessions().max(1) {
            let worker = Worker {
                shared: self.shared.clone(),
                model: model.clone(),
                cache: self.cache.clone(),
                decode: self.decode.clone(),
                outbox: self.outbox.clone(),
            };
            let spawned = std::thread::Builder::new()
                .name(format!("sortypix-ai-worker-{index}"))
                .spawn(move || worker.run());
            if spawned.is_err() && index == 0 {
                *lock(&self.shared.status) =
                    Status::Unavailable("could not start the analysis".into());
                return;
            }
        }
        *lock(&self.shared.status) = Status::Ready;
        // The reason probes: a few text embeddings, once, while the workers start on photos.
        self.probes = ReasonProbes::embed(|text| model.embed_text(text)).ok();
        loop {
            let message = if self.dirty {
                let wait = self.next_analysis.saturating_duration_since(Instant::now());
                match self.inbox.recv_timeout(wait) {
                    Ok(message) => message,
                    Err(RecvTimeoutError::Timeout) => {
                        self.analyse();
                        continue;
                    }
                    Err(RecvTimeoutError::Disconnected) => return,
                }
            } else {
                match self.inbox.recv() {
                    Ok(message) => message,
                    Err(_) => return,
                }
            };
            match message {
                Message::SetFolder(generation, photos) => self.set_folder(generation, photos),
                Message::Done {
                    generation,
                    path,
                    record,
                } => self.done(generation, path, record),
                Message::Quit => return,
            }
            // The last photo of the folder: analyse at once, without waiting for the pause. Later
            // changes of the list (photos sorted out) wait for the pause.
            if std::mem::take(&mut self.completed) {
                self.analyse();
            }
        }
    }

    fn set_folder(&mut self, generation: u64, photos: Vec<(PathBuf, Option<i64>)>) {
        let fresh = generation != self.generation;
        if fresh {
            self.generation = generation;
            self.failed.clear();
            let mut results = self.results_mut();
            *results = Results {
                generation,
                ..Results::default()
            };
        }
        self.members = photos.iter().map(|(path, _)| path.clone()).collect();
        // A photo that left the folder (kept, rejected) may come back (undo): what failed while
        // it was moving is tried again then.
        let members = &self.members;
        self.failed.retain(|path| members.contains(path));
        let failed = self.failed.len();
        if !fresh {
            self.results_mut().failed = failed;
        }
        self.photos = photos;
        let done = {
            let results = self
                .shared
                .results
                .read()
                .unwrap_or_else(|e| e.into_inner());
            let analysed =
                |path: &PathBuf| results.records.contains_key(path) || self.failed.contains(path);
            let mut queue = lock(&self.shared.queue);
            queue.generation = generation;
            let in_flight = &queue.in_flight;
            let todo: VecDeque<PathBuf> = self
                .photos
                .iter()
                .map(|(path, _)| path)
                .filter(|path| {
                    !analysed(path) && !in_flight.contains(&(generation, (*path).clone()))
                })
                .cloned()
                .collect();
            queue.todo = todo;
            self.photos
                .iter()
                .filter(|(path, _)| analysed(path))
                .count()
        };
        self.shared.work.notify_all();
        *lock(&self.shared.progress) = (done, self.photos.len());
        // The list changed: the analyses follow, within the usual pause.
        self.dirty = true;
    }

    fn done(&mut self, generation: u64, path: PathBuf, record: Option<Box<Record>>) {
        lock(&self.shared.queue)
            .in_flight
            .remove(&(generation, path.clone()));
        if generation != self.generation {
            return;
        }
        let counted = self.members.contains(&path);
        match record {
            Some(record) => {
                self.results_mut().records.insert(path, *record);
            }
            // A photo no longer in the folder failed because it was being moved: not a failure.
            None if counted => {
                self.failed.insert(path);
                let failed = self.failed.len();
                self.results_mut().failed = failed;
            }
            None => {}
        }
        if counted {
            let mut progress = lock(&self.shared.progress);
            progress.0 = (progress.0 + 1).min(progress.1);
            self.completed = progress.0 == progress.1;
        }
        self.dirty = true;
    }

    fn results_mut(&self) -> std::sync::RwLockWriteGuard<'_, Results> {
        self.shared
            .results
            .write()
            .unwrap_or_else(|e| e.into_inner())
    }

    /// Recomputes the folder analyses over the photos analysed so far, then sets the pause.
    fn analyse(&mut self) {
        let started = Instant::now();
        self.dirty = false;
        let (paths, items, times) = {
            let results = self
                .shared
                .results
                .read()
                .unwrap_or_else(|e| e.into_inner());
            let mut paths = Vec::new();
            let mut items = Vec::new();
            let mut times = Vec::new();
            for (path, time) in &self.photos {
                if let Some(record) = results.records.get(path) {
                    paths.push(path.clone());
                    items.push(record.embedding);
                    times.push(*time);
                }
            }
            (paths, items, times)
        };
        let (done, total) = *lock(&self.shared.progress);
        let analysis = folder_analysis(
            self.generation,
            paths,
            &items,
            &times,
            done == total,
            self.probes.as_ref(),
        );
        self.results_mut().analysis = Some(Arc::new(analysis));
        let pause = started.elapsed() * ANALYSIS_PAUSE_FACTOR;
        self.next_analysis = Instant::now() + pause.max(ANALYSIS_INTERVAL);
    }
}

/// Off-topic scores, bursts and coverage order of `items` (grid order, capture times beside).
fn folder_analysis(
    generation: u64,
    paths: Vec<PathBuf>,
    items: &[Embedding],
    times: &[Option<i64>],
    complete: bool,
    probes: Option<&ReasonProbes>,
) -> FolderAnalysis {
    let z = if items.len() <= OUTLIER_LIMIT {
        outliers(items, OFF_TOPIC_K)
    } else {
        Vec::new()
    };
    // Shooting order: photos with a capture time by that time, then the others by file name.
    let mut order: Vec<usize> = (0..items.len()).collect();
    order.sort_by(|&a, &b| {
        (times[a].is_none(), times[a], &paths[a]).cmp(&(times[b].is_none(), times[b], &paths[b]))
    });
    let seconds: Vec<Option<f64>> = times.iter().map(|t| t.map(|s| s as f64)).collect();
    FolderAnalysis {
        generation,
        bursts: bursts(
            items,
            &order,
            &seconds,
            BURST_THRESHOLD,
            BURST_MAX_GAP_S,
            BURST_WINDOW,
        ),
        coverage: coverage_order_with_outliers(items, &z, COVERAGE_SEED),
        outliers: z,
        reasons: probes.map_or_else(Vec::new, |p| folder_reasons(p, items)),
        paths,
        complete,
    }
}

/// One analysis thread: one visual session's worth of photos at a time.
struct Worker {
    shared: Arc<Shared>,
    model: Arc<dyn Model>,
    cache: Arc<EmbeddingCache>,
    decode: Decoder,
    outbox: Sender<Message>,
}

impl Worker {
    fn run(self) {
        while let Some((generation, path)) = self.next() {
            // A bug in a decoder must cost one photo, not the analysis.
            let record =
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| self.analyse(&path)))
                    .unwrap_or(None);
            let done = Message::Done {
                generation,
                path,
                record: record.map(Box::new),
            };
            if self.outbox.send(done).is_err() {
                return;
            }
        }
    }

    /// The next photo to analyse, waiting for one; `None` once the analyzer is dropped.
    fn next(&self) -> Option<(u64, PathBuf)> {
        let mut queue = lock(&self.shared.queue);
        loop {
            if queue.closed {
                return None;
            }
            if let Some(path) = queue.todo.pop_front() {
                let generation = queue.generation;
                queue.in_flight.insert((generation, path.clone()));
                return Some((generation, path));
            }
            queue = self
                .shared
                .work
                .wait(queue)
                .unwrap_or_else(|e| e.into_inner());
        }
    }

    fn analyse(&self, path: &Path) -> Option<Record> {
        let key = ContentKey::of(path, MODEL_LABEL)?;
        if let Some(record) = self.cache.get(&key) {
            return Some(record);
        }
        let (rgba, width, height) = (self.decode)(path)?;
        let input = preprocess(&rgba, width, height).ok()?;
        drop(rgba);
        let record = self.model.embed(&input).ok()?;
        // A file changed while it was read gets no record: the next analysis redoes it.
        if ContentKey::of(path, MODEL_LABEL) == Some(key) {
            let _ = self.cache.put(&key, &record);
        }
        Some(record)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::EMBED_DIM;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// Embeds the first pixel's colour: photos of one colour share an embedding.
    struct Fake {
        calls: AtomicUsize,
        order: Mutex<Vec<f32>>,
        /// When set, the first call reports itself then waits to be released.
        gate: Mutex<Option<(Sender<()>, Receiver<()>)>>,
    }

    impl Fake {
        fn new() -> Arc<Fake> {
            Arc::new(Fake {
                calls: AtomicUsize::new(0),
                order: Mutex::new(Vec::new()),
                gate: Mutex::new(None),
            })
        }
    }

    impl Model for Fake {
        fn sessions(&self) -> usize {
            1
        }

        fn embed(&self, input: &[f32]) -> Result<Record, AiError> {
            if let Some((entered, release)) = lock(&self.gate).take() {
                let _ = entered.send(());
                let _ = release.recv();
            }
            self.calls.fetch_add(1, Ordering::SeqCst);
            let plane = input.len() / 3;
            let rgb = [input[0], input[plane], input[2 * plane]];
            lock(&self.order).push(rgb[0]);
            let mut embedding = [0f32; EMBED_DIM];
            embedding[..3].copy_from_slice(&rgb);
            embedding[3] = 4.0;
            let norm = embedding.iter().map(|v| v * v).sum::<f32>().sqrt();
            embedding.iter_mut().for_each(|v| *v /= norm);
            Ok(Record {
                embedding,
                score: rgb[0],
            })
        }

        fn embed_text(&self, _text: &str) -> Result<Embedding, AiError> {
            Ok([0.0; EMBED_DIM])
        }
    }

    /// Files hold three bytes, the colour of their 8 x 8 image; anything else is unreadable.
    fn decoder() -> Decoder {
        Arc::new(|path: &Path| {
            let bytes = std::fs::read(path).ok()?;
            let rgb: [u8; 3] = bytes.as_slice().try_into().ok()?;
            let pixels = (0..64)
                .flat_map(|_| [rgb[0], rgb[1], rgb[2], 255])
                .collect();
            Some((pixels, 8, 8))
        })
    }

    fn start(fake: &Arc<Fake>, cache: &Path) -> Analyzer {
        let model = fake.clone();
        Analyzer::start_with(
            Box::new(move || Ok(model as Arc<dyn Model>)),
            cache.to_path_buf(),
            decoder(),
        )
    }

    fn photo(dir: &Path, name: &str, rgb: [u8; 3]) -> PathBuf {
        let path = dir.join(name);
        std::fs::write(&path, rgb).unwrap();
        path
    }

    fn wait(what: &str, mut condition: impl FnMut() -> bool) {
        let until = Instant::now() + Duration::from_secs(20);
        while !condition() {
            assert!(Instant::now() < until, "timed out waiting for {what}");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    fn complete(analyzer: &Analyzer, generation: u64, photos: usize) -> Arc<FolderAnalysis> {
        wait("a complete analysis", || {
            analyzer.results().analysis.as_ref().is_some_and(|a| {
                a.generation == generation && a.complete && a.paths.len() == photos
            })
        });
        analyzer.results().analysis.clone().unwrap()
    }

    #[test]
    fn missing_models_leave_the_analysis_unavailable() {
        let dir = tempfile::tempdir().unwrap();
        let analyzer = Analyzer::start(
            dir.path().join("no-models"),
            dir.path().join("cache"),
            decoder(),
        );
        wait("the model load", || analyzer.status() != Status::Loading);
        match analyzer.status() {
            Status::Unavailable(message) => assert!(message.contains("missing"), "{message}"),
            other => panic!("expected Unavailable, got {other:?}"),
        }
        analyzer.set_folder(1, vec![(dir.path().join("a.jpg"), None)]);
        assert!(analyzer.embed_text("a dog").recv().unwrap().is_err());
        assert_eq!(analyzer.progress(), (0, 0));
    }

    #[test]
    fn photos_are_analysed_in_grid_order_and_unreadable_ones_skipped() {
        let dir = tempfile::tempdir().unwrap();
        let fake = Fake::new();
        let analyzer = start(&fake, &dir.path().join("cache"));
        let c = photo(dir.path(), "c.jpg", [30, 0, 0]);
        let a = photo(dir.path(), "a.jpg", [10, 0, 0]);
        let b = photo(dir.path(), "b.jpg", [20, 0, 0]);
        let bad = dir.path().join("bad.jpg");
        std::fs::write(&bad, b"not three bytes").unwrap();
        let gone = dir.path().join("gone.jpg");
        let list = [&c, &a, &b, &bad, &gone];
        analyzer.set_folder(1, list.iter().map(|p| ((*p).clone(), None)).collect());
        let analysis = complete(&analyzer, 1, 3);
        assert_eq!(analyzer.progress(), (5, 5));
        assert_eq!(analysis.paths, [c.clone(), a.clone(), b.clone()]);
        assert_eq!(analysis.coverage.len(), 3);
        assert_eq!(analysis.outliers.len(), 3);
        let results = analyzer.results();
        assert_eq!(results.failed, 2);
        assert_eq!(results.records.len(), 3);
        drop(results);
        // One worker: the model saw the photos in grid order (first pixel, red channel).
        let order = lock(&fake.order).clone();
        assert!(order[0] > order[2] && order[2] > order[1], "{order:?}");
        assert_eq!(analyzer.status(), Status::Ready);
        assert!(analyzer.embed_text("a dog").recv().unwrap().is_ok());
    }

    #[test]
    fn a_photo_that_failed_while_moving_is_analysed_when_it_comes_back() {
        let dir = tempfile::tempdir().unwrap();
        let fake = Fake::new();
        let analyzer = start(&fake, &dir.path().join("cache"));
        let a = photo(dir.path(), "a.jpg", [10, 0, 0]);
        // Sorted out while the analysis starts: the file is gone when it is read.
        let away = dir.path().join("away.jpg");
        std::fs::rename(&a, &away).unwrap();
        analyzer.set_folder(1, vec![(a.clone(), None)]);
        wait("the failure", || analyzer.results().failed == 1);
        // The snapshot without it, then the undo brings it back.
        analyzer.set_folder(1, Vec::new());
        wait("the failure forgotten", || analyzer.results().failed == 0);
        std::fs::rename(&away, &a).unwrap();
        analyzer.set_folder(1, vec![(a.clone(), None)]);
        complete(&analyzer, 1, 1);
        assert!(analyzer.results().records.contains_key(&a));
        assert_eq!(analyzer.results().failed, 0);
        assert_eq!(analyzer.progress(), (1, 1));
    }

    #[test]
    fn a_second_run_reads_the_cache_and_a_move_keeps_the_record() {
        let dir = tempfile::tempdir().unwrap();
        let cache = dir.path().join("cache");
        let a = photo(dir.path(), "a.jpg", [10, 0, 0]);
        let first = Fake::new();
        let analyzer = start(&first, &cache);
        analyzer.set_folder(1, vec![(a.clone(), None)]);
        complete(&analyzer, 1, 1);
        let record = analyzer.results().records[&a];
        drop(analyzer);
        let keep = dir.path().join("keep");
        std::fs::create_dir(&keep).unwrap();
        let moved = keep.join("a.jpg");
        std::fs::rename(&a, &moved).unwrap();
        let second = Fake::new();
        let analyzer = start(&second, &cache);
        analyzer.set_folder(1, vec![(moved.clone(), None)]);
        complete(&analyzer, 1, 1);
        assert_eq!(analyzer.results().records[&moved], record);
        assert_eq!(second.calls.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn results_of_a_previous_folder_are_dropped() {
        let dir = tempfile::tempdir().unwrap();
        let fake = Fake::new();
        let (entered, wait_entered) = mpsc::channel();
        let (release, gate) = mpsc::channel();
        *lock(&fake.gate) = Some((entered, gate));
        let analyzer = start(&fake, &dir.path().join("cache"));
        let old = photo(dir.path(), "old.jpg", [10, 0, 0]);
        let new = photo(dir.path(), "new.jpg", [20, 0, 0]);
        analyzer.set_folder(1, vec![(old.clone(), None)]);
        wait_entered.recv().unwrap();
        // The old photo is being embedded when the other folder opens.
        analyzer.set_folder(2, vec![(new.clone(), None)]);
        release.send(()).unwrap();
        let analysis = complete(&analyzer, 2, 1);
        assert_eq!(analysis.paths, std::slice::from_ref(&new));
        let results = analyzer.results();
        assert_eq!(results.generation, 2);
        assert!(!results.records.contains_key(&old));
        assert!(results.records.contains_key(&new));
        drop(results);
        assert_eq!(analyzer.progress(), (1, 1));
    }

    #[test]
    fn the_same_generation_keeps_what_was_analysed() {
        let dir = tempfile::tempdir().unwrap();
        let fake = Fake::new();
        let analyzer = start(&fake, &dir.path().join("cache"));
        let a = photo(dir.path(), "a.jpg", [10, 0, 0]);
        let b = photo(dir.path(), "b.jpg", [20, 0, 0]);
        analyzer.set_folder(7, vec![(a.clone(), None), (b.clone(), None)]);
        complete(&analyzer, 7, 2);
        // `a` was sorted out of the folder.
        analyzer.set_folder(7, vec![(b.clone(), None)]);
        let analysis = complete(&analyzer, 7, 1);
        assert_eq!(analysis.paths, std::slice::from_ref(&b));
        assert_eq!(analyzer.progress(), (1, 1));
        assert_eq!(fake.calls.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn bursts_need_close_capture_times() {
        let dir = tempfile::tempdir().unwrap();
        let fake = Fake::new();
        let analyzer = start(&fake, &dir.path().join("cache"));
        // Same colour, same embedding: only the capture times tell the bursts apart.
        let names = ["a.jpg", "b.jpg", "c.jpg", "d.jpg", "e.jpg"];
        let paths: Vec<PathBuf> = names
            .iter()
            .map(|n| photo(dir.path(), n, [10, 20, 30]))
            .collect();
        // Grid order differs from shooting order; e has no capture time.
        let times = [Some(1000), Some(500), Some(1004), Some(503), None];
        analyzer.set_folder(1, paths.iter().cloned().zip(times).collect::<Vec<_>>());
        let analysis = complete(&analyzer, 1, 5);
        // Shooting order: b (500), d (503), a (1000), c (1004), then e (no time, which joins
        // on similarity alone).
        assert_eq!(analysis.bursts, [vec![1, 3], vec![0, 2, 4]]);
    }
}
