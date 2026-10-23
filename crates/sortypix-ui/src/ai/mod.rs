//! ROLE: Sortypix AI only. Starts the background analysis, feeds it the open folder, and turns
//! its results into what the grid shows: marks, suggestions, search and the AI orders.
//! OWNER: Gallery holds one `Ai`; the analysis threads live in `sortypix_ai::service`.
//! INVARIANTS: Nothing here blocks the interface: every call is a message or a short lock,
//! and a missing or broken model only leaves the analysis unavailable. The model loads only
//! once the first preview is on screen, so opening a folder is as fast as in Sortypix. The
//! grid's `photos` and `turns` are always one view of the worker's last snapshot (`base`),
//! permuted or filtered together; the results are looked up by the paths of that snapshot.
mod card;
pub(crate) mod diagnostic;
mod filters;
mod marks;
mod prefs;
mod search;
mod suggest;
mod taskbar;
mod toolbar;
mod view;

use crate::gallery::Gallery;
use filters::{Kind, Kinds};
use search::Search;
use sortypix_ai::cache::{MODEL_LABEL, cache_root};
use sortypix_ai::service::{Analyzer, DECODE_EDGE, Decoder, Status};
use sortypix_ai::{BURST_SUGGEST, cosine, search_prompt};
use sortypix_app::catalog::PhysicalPhoto;
use sortypix_core::PhotoId;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};
use suggest::{Marks, suggest};
use view::{Arrangement, Burst, Facts, Order};

/// Without any preview to wait for (empty screen, empty folder), the model loads after this.
const START_DELAY: Duration = Duration::from_secs(2);
/// Shortest pause between two readings of the analysis results.
const REFRESH_INTERVAL: Duration = Duration::from_millis(500);
/// The pause is also at least this many times the last reading, so a huge folder costs the
/// interface at most a tenth of its frames' time.
const REFRESH_PAUSE_FACTOR: u32 = 10;
/// The progress line shows only once the analysis has run this long: a folder read from the
/// cache is usually done before, and shows nothing.
const LINE_DELAY: Duration = Duration::from_secs(1);
/// Keyboard focus of the search field (the sorting shortcuts are off while it has it).
pub(crate) const SEARCH_ID: &str = "sortypix-ai-search";

/// What the interface can offer.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum State {
    /// The model is not loaded yet.
    Starting,
    Ready,
    /// The model could not load; the message says why. Sorting works as usual.
    Off(String),
}

/// Where the diagnostic clicks (centres of what was drawn last).
#[derive(Default)]
pub(crate) struct Targets {
    pub order: Option<egui::Pos2>,
    /// Entries of the open order menu: `None` is "Best match".
    pub entries: Vec<(Option<Order>, egui::Pos2)>,
    pub search: Option<egui::Pos2>,
    pub clear: Option<egui::Pos2>,
    pub reject: Option<egui::Pos2>,
    /// The first badge drawn on the grid this frame.
    pub badge: Option<egui::Pos2>,
    /// Every badge drawn this frame, with its photo.
    pub badges: Vec<(PhotoId, egui::Pos2)>,
    /// "Show" entries of the open menu (`None`: all photos), and the badge toggles.
    pub shows: Vec<(Option<Kind>, egui::Pos2)>,
    pub toggles: Vec<(Kind, egui::Pos2)>,
    /// The cross of the open photo card.
    pub card_close: Option<egui::Pos2>,
}

use eframe::egui;

/// What the facts were last computed from.
#[derive(Clone, Debug, PartialEq)]
struct Stamp {
    generation: u64,
    records: usize,
    analysis: Option<usize>,
    query: Option<String>,
}

/// Which toolbar controls fit (decided right to left, the button first).
#[derive(Clone, Copy, Default)]
pub(crate) struct Room {
    pub slider: bool,
    pub search: bool,
}

pub struct Ai {
    /// Started by [`Ai::poll`]; `None` until then.
    pub(crate) analyzer: Option<Analyzer>,
    cache_root: PathBuf,
    /// Where the models are read from, resolved once (reported by the benchmark).
    models: PathBuf,
    created: Instant,
    /// One per folder opened: results of a previous folder never mix with this one.
    generation: u64,
    /// Paths and capture times last given to the analyzer, in grid order.
    sent: Vec<(PathBuf, Option<i64>)>,
    /// A folder was opened and its list not sent yet (even an empty one must be).
    opened: bool,
    /// The worker's last snapshot, capture order, with each photo's quarter turns.
    base: Vec<PhysicalPhoto>,
    base_turns: Vec<u8>,
    /// What the analysis knows of each photo of `base`.
    facts: Vec<Facts>,
    /// Indices of `base` the grid shows, in display order.
    view: Vec<usize>,
    /// The same before the "show only" filter: the marks are computed on it.
    arranged_view: Vec<usize>,
    /// Index in `base` by `PhotoId.0`.
    index: HashMap<u64, usize>,
    pub(crate) marks: Marks,
    pub(crate) search: Search,
    stamp: Option<Stamp>,
    next_refresh: Instant,
    /// Counts the readings of the results, and the reading the view was arranged with.
    revision: u64,
    arranged: u64,
    /// Since when the open folder has had photos left to analyse.
    incomplete_since: Option<Instant>,
    /// The progress line was drawn at least once (checked by the diagnostic).
    pub(crate) line_shown: bool,
    pub(crate) room: Room,
    pub(crate) targets: Targets,
    /// The photo whose card is open.
    pub(crate) card: Option<PhotoId>,
    /// The photo under the pointer on the grid, and the card photo's cell, this frame.
    pub(crate) hovered: Option<PhotoId>,
    card_cell: Option<egui::Rect>,
    /// Photos outlined as the card's burst this frame, and the card's lines (diagnostic).
    pub(crate) outlined: usize,
    pub(crate) card_lines: Vec<String>,
    /// The legend of the marks was drawn (diagnostic).
    pub(crate) legend_drawn: bool,
    /// Badges hidden by choice, read from the settings each frame.
    hidden: Kinds,
    /// Photos analysed and in the folder while the progress line shows (this frame).
    pub(crate) line: Option<(usize, usize)>,
    taskbar: taskbar::Taskbar,
    /// The taskbar button took a progress value at least once (diagnostic).
    pub(crate) taskbar_progressed: bool,
}

/// The models: `models/` beside the executable (installed copy), else the ai-dev folder
/// (`SORTYPIX_AI_DIR`, else the cache root), as `sortypix_ai::models_dir` resolves it.
fn models_dir() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|exe| Some(exe.parent()?.join("models")))
        .filter(|dir| dir.is_dir())
        .unwrap_or_else(sortypix_ai::models_dir)
}

/// Sends `photos` (grid order first) to the analyzer when the list changed (photos, order, or
/// capture times once the metadata is read), or when a folder was just opened.
fn observe(
    sent: &mut Vec<(PathBuf, Option<i64>)>,
    opened: &mut bool,
    analyzer: Option<&Analyzer>,
    generation: u64,
    photos: &[&PhysicalPhoto],
) {
    let changed = sent.len() != photos.len()
        || sent.iter().zip(photos).any(|((path, time), photo)| {
            *time != photo.capture_time || path != photo.display_source(false)
        });
    if !changed && !*opened {
        return;
    }
    *sent = photos
        .iter()
        .map(|p| (p.display_source(false).to_path_buf(), p.capture_time))
        .collect();
    if let Some(analyzer) = analyzer {
        *opened = false;
        analyzer.set_folder(generation, sent.clone());
    }
}

impl Ai {
    /// Call once the interface, preview and file threads exist: they keep the normal priority,
    /// the analysis threads started later run below them.
    pub fn new(cache_dir: &Path, cc: &eframe::CreationContext<'_>) -> Ai {
        sortypix_ai::priority::favour_interface();
        Ai {
            analyzer: None,
            cache_root: cache_root(cache_dir, MODEL_LABEL),
            models: models_dir(),
            created: Instant::now(),
            generation: 0,
            sent: Vec::new(),
            opened: false,
            base: Vec::new(),
            base_turns: Vec::new(),
            facts: Vec::new(),
            view: Vec::new(),
            arranged_view: Vec::new(),
            index: HashMap::new(),
            marks: Marks::default(),
            search: Search::default(),
            stamp: None,
            next_refresh: Instant::now(),
            revision: 0,
            arranged: 0,
            incomplete_since: None,
            line_shown: false,
            room: Room::default(),
            targets: Targets::default(),
            card: None,
            hovered: None,
            card_cell: None,
            outlined: 0,
            card_lines: Vec::new(),
            legend_drawn: false,
            hidden: Kinds::default(),
            line: None,
            taskbar: taskbar::Taskbar::new(cc),
            taskbar_progressed: false,
        }
    }

    /// Each frame: starts the analysis once the first preview is shown (or after
    /// [`START_DELAY`]), then hands it the photos already seen.
    pub fn poll(&mut self, first_preview_shown: bool) {
        let due = first_preview_shown || self.created.elapsed() >= START_DELAY;
        if self.analyzer.is_some() || !due {
            return;
        }
        // The application's own decoder (EXIF orientation, reduced JPEG decode), without the
        // preview cache: a 448 px image is used once, its embedding is what is kept.
        let decode: Decoder = Arc::new(|path: &Path| {
            let image = sortypix_app::decode::decode(path, Some(DECODE_EDGE)).ok()?;
            Some((image.rgba, image.width, image.height))
        });
        let analyzer = Analyzer::start(self.models.clone(), self.cache_root.clone(), decode);
        if self.generation > 0 {
            analyzer.set_folder(self.generation, self.sent.clone());
            self.opened = false;
        }
        self.analyzer = Some(analyzer);
    }

    /// A folder is being opened: the next list starts a new generation, the search is cleared.
    pub fn folder_opened(&mut self) {
        self.generation += 1;
        self.sent.clear();
        self.opened = true;
        self.search = Search::default();
        self.stamp = None;
    }

    pub(crate) fn state(&self) -> State {
        match self.analyzer.as_ref().map(Analyzer::status) {
            None | Some(Status::Loading) => State::Starting,
            Some(Status::Ready) => State::Ready,
            Some(Status::Unavailable(why)) => State::Off(why),
        }
    }

    /// A new snapshot from the worker (capture order).
    fn set_base(&mut self, photos: Vec<PhysicalPhoto>, turns: Vec<u8>) {
        self.facts = vec![Facts::default(); photos.len()];
        self.index = photos
            .iter()
            .enumerate()
            .map(|(i, p)| (p.id.0, i))
            .collect();
        self.base = photos;
        self.base_turns = turns;
        self.stamp = None;
    }

    /// Photos in the folder (shown or not).
    pub(crate) fn total(&self) -> usize {
        self.base.len()
    }

    /// Reads the results into `facts` when they changed (or `force`), at most every
    /// [`REFRESH_INTERVAL`]. True when the facts were recomputed.
    fn refresh(&mut self, force: bool, now: Instant) -> bool {
        let Some(analyzer) = &self.analyzer else {
            return false;
        };
        if !force && now < self.next_refresh {
            return false;
        }
        let started = Instant::now();
        let results = analyzer.results();
        let stamp = Stamp {
            generation: results.generation,
            records: results.records.len(),
            analysis: results.analysis.as_ref().map(|a| Arc::as_ptr(a) as usize),
            query: self.search.applied().map(str::to_owned),
        };
        if !force && self.stamp.as_ref() == Some(&stamp) {
            return false;
        }
        let current = results.generation == self.generation;
        let analysis = results
            .analysis
            .as_deref()
            .filter(|a| current && a.generation == self.generation);
        let mut index: HashMap<&Path, usize> = HashMap::new();
        let mut bursts: Vec<Option<Burst>> = Vec::new();
        let mut ranks: Vec<Option<usize>> = Vec::new();
        if let Some(a) = analysis {
            index = a
                .paths
                .iter()
                .enumerate()
                .map(|(i, p)| (p.as_path(), i))
                .collect();
            bursts = vec![None; a.paths.len()];
            for (group, members) in a.bursts.iter().enumerate() {
                for &m in members {
                    if let Some(slot) = bursts.get_mut(m) {
                        *slot = Some(Burst {
                            group,
                            size: members.len(),
                        });
                    }
                }
            }
            ranks = vec![None; a.paths.len()];
            for (rank, &i) in a.coverage.iter().enumerate() {
                if let Some(slot) = ranks.get_mut(i) {
                    *slot = Some(rank);
                }
            }
        }
        let query = self.search.query();
        self.facts = self
            .base
            .iter()
            .map(|photo| {
                let path = photo.display_source(false);
                let record = if current {
                    results.records.get(path)
                } else {
                    None
                };
                let at = analysis.and_then(|_| index.get(path).copied());
                Facts {
                    score: record.map(|r| r.score),
                    off_topic: analysis
                        .zip(at)
                        .and_then(|(a, i)| a.outliers.get(i).copied()),
                    burst: at.and_then(|i| bursts.get(i).copied().flatten()),
                    coverage: at.and_then(|i| ranks.get(i).copied().flatten()),
                    similarity: record.zip(query).map(|(r, q)| cosine(q, &r.embedding)),
                    reason: analysis
                        .zip(at)
                        .and_then(|(a, i)| a.reasons.get(i).copied()),
                }
            })
            .collect();
        drop(results);
        self.stamp = Some(stamp);
        self.revision += 1;
        let pause = started.elapsed() * REFRESH_PAUSE_FACTOR;
        self.next_refresh = now + pause.max(REFRESH_INTERVAL);
        true
    }

    /// Computes the view of `base` and its marks, then keeps the photos `show` asks for;
    /// returns the photos and turns to show.
    fn arrange(
        &mut self,
        how: Arrangement,
        percent: u32,
        show: Option<Kind>,
    ) -> (Vec<PhysicalPhoto>, Vec<u8>) {
        self.arranged_view = view::arrange(&self.facts, how);
        self.arranged = self.revision;
        self.remark(percent);
        let base = &self.base;
        self.view = filters::apply(&self.arranged_view, |i| base[i].id.0, &self.marks, show);
        let mut shown = vec![false; self.base.len()];
        for &i in &self.view {
            shown[i] = true;
        }
        // The analysis follows the grid: shown photos first, then those the search hides.
        let order: Vec<&PhysicalPhoto> = self
            .view
            .iter()
            .map(|&i| &self.base[i])
            .chain(
                (0..self.base.len())
                    .filter(|&i| !shown[i])
                    .map(|i| &self.base[i]),
            )
            .collect();
        observe(
            &mut self.sent,
            &mut self.opened,
            self.analyzer.as_ref(),
            self.generation,
            &order,
        );
        (
            self.view.iter().map(|&i| self.base[i].clone()).collect(),
            self.view.iter().map(|&i| self.base_turns[i]).collect(),
        )
    }

    /// Recomputes the marks of the arranged photos (before the "show only" filter).
    fn remark(&mut self, percent: u32) {
        let shown: Vec<(PhotoId, Facts)> = self
            .arranged_view
            .iter()
            .map(|&i| (self.base[i].id, self.facts[i]))
            .collect();
        self.marks = suggest(&shown, percent, BURST_SUGGEST);
    }

    /// Advances the search; true when the applied query changed.
    fn poll_search(&mut self, now: Instant) -> bool {
        let Ai {
            search, analyzer, ..
        } = self;
        search.poll(now, |words| {
            let analyzer = analyzer.as_ref()?;
            (analyzer.status() == Status::Ready).then(|| analyzer.embed_text(&search_prompt(words)))
        })
    }

    /// Photos analysed and photos in the folder, while the progress line should show.
    fn progress_line(&mut self, now: Instant) -> Option<(usize, usize)> {
        let analyzer = self.analyzer.as_ref()?;
        let (done, total) = analyzer.progress();
        if analyzer.status() != Status::Ready || total == 0 || done >= total {
            self.incomplete_since = None;
            return None;
        }
        let since = *self.incomplete_since.get_or_insert(now);
        (now.saturating_duration_since(since) >= LINE_DELAY).then_some((done, total))
    }

    /// What the analysis knows of the photo `id` of the snapshot.
    pub(crate) fn facts_of(&self, id: PhotoId) -> Option<Facts> {
        self.index
            .get(&id.0)
            .and_then(|&i| self.facts.get(i))
            .copied()
    }

    /// The photo `id` belongs to the burst of the open card (the card's photo included).
    fn in_card_burst(&self, id: PhotoId) -> bool {
        let group = |id: PhotoId| self.facts_of(id).and_then(|f| f.burst).map(|b| b.group);
        self.card
            .and_then(group)
            .is_some_and(|g| group(id) == Some(g))
    }

    /// Photos of the arranged view before the "show only" filter, in display order.
    pub(crate) fn arranged_ids(&self) -> Vec<PhotoId> {
        self.arranged_view
            .iter()
            .map(|&i| self.base[i].id)
            .collect()
    }

    /// The grid starts drawing: forget what the last frame drew.
    pub(crate) fn grid_started(&mut self) {
        self.targets.badge = None;
        self.targets.badges.clear();
        self.hovered = None;
        self.card_cell = None;
        self.outlined = 0;
    }

    /// What the taskbar button shows now (diagnostic).
    pub(crate) fn taskbar_shown(&self) -> taskbar::Shown {
        self.taskbar.shown()
    }

    /// The facts reflect the latest results (nothing new since the last reading).
    pub(crate) fn facts_current(&self) -> bool {
        self.analyzer.as_ref().is_some_and(|analyzer| {
            let results = analyzer.results();
            self.stamp.as_ref().is_some_and(|stamp| {
                stamp.generation == results.generation
                    && stamp.records == results.records.len()
                    && stamp.analysis == results.analysis.as_ref().map(|a| Arc::as_ptr(a) as usize)
                    && stamp.query.as_deref() == self.search.applied()
            })
        })
    }

    /// The folder analyses cover every photo of the open folder (checked by the diagnostic).
    pub(crate) fn complete(&self) -> bool {
        self.analyzer.as_ref().is_some_and(|analyzer| {
            let (done, total) = analyzer.progress();
            let results = analyzer.results();
            done == total
                && results.generation == self.generation
                && results
                    .analysis
                    .as_ref()
                    .is_some_and(|a| a.complete && a.generation == self.generation)
        })
    }

    /// Analysis status for probes: `status done/total`.
    pub(crate) fn describe(&self) -> String {
        let Some(analyzer) = &self.analyzer else {
            return "not started".to_string();
        };
        let (done, total) = analyzer.progress();
        let status = match analyzer.status() {
            Status::Loading => "loading".to_string(),
            Status::Ready => "ready".to_string(),
            Status::Unavailable(why) => format!("unavailable ({why})"),
        };
        format!("{status} {done}/{total}")
    }

    /// The models folder in use: the installed copy beside the executable, or ai-dev.
    pub(crate) fn models(&self) -> &Path {
        &self.models
    }

    /// Photos analysed or skipped so far in the open folder.
    pub(crate) fn done(&self) -> usize {
        self.analyzer.as_ref().map_or(0, |a| a.progress().0)
    }

    /// The model is loaded and photos of the open folder are being analysed.
    pub(crate) fn analysing(&self) -> bool {
        self.analyzer.as_ref().is_some_and(|analyzer| {
            let (done, total) = analyzer.progress();
            analyzer.status() == Status::Ready && done >= 1 && done < total
        })
    }
}

impl Gallery {
    /// How the grid is arranged now (an AI order falls back to capture order without a model).
    fn ai_arrangement(&self) -> Arrangement {
        let mut order = prefs::order(&self.settings);
        if order.needs_model() && matches!(self.ai.state(), State::Off(_)) {
            order = if self.settings.newest_first {
                Order::Newest
            } else {
                Order::Oldest
            };
        }
        let searching = self.ai.search.query().is_some();
        Arrangement {
            order,
            searching,
            best_match: searching && self.ai.search.best_match,
        }
    }

    /// The "show only" filter in force (none without a model, like the AI orders).
    pub(crate) fn ai_show(&self) -> Option<Kind> {
        prefs::show(&self.settings).filter(|_| !matches!(self.ai.state(), State::Off(_)))
    }

    /// Shows the current view of the snapshot, keeping the selected photo when it stays.
    pub(crate) fn ai_arrange(&mut self) {
        let previous = self.current_id();
        let how = self.ai_arrangement();
        let percent = prefs::suggest_percent(&self.settings);
        let show = self.ai_show();
        let (photos, turns) = self.ai.arrange(how, percent, show);
        self.photos = photos;
        self.turns = turns;
        self.layout.invalidate();
        self.selected = previous
            .and_then(|id| self.photos.iter().position(|p| p.id == id))
            .unwrap_or(self.selected)
            .min(self.photos.len().saturating_sub(1));
    }

    /// The worker sent a snapshot: `photos` and `turns` hold it in capture order.
    pub(crate) fn ai_snapshot(&mut self) {
        let photos = std::mem::take(&mut self.photos);
        let turns = std::mem::take(&mut self.turns);
        self.ai.set_base(photos, turns);
        self.ai.refresh(true, Instant::now());
        self.ai_arrange();
    }

    /// The marks changed (share, new results): a "show only" filter follows them.
    pub(crate) fn ai_marks_changed(&mut self) {
        if self.ai.search.query().is_some() || self.ai_show().is_some() {
            self.ai_arrange();
        } else {
            self.ai.remark(prefs::suggest_percent(&self.settings));
        }
    }

    /// Each frame: analysis start, search answers, new results, the taskbar progress.
    pub(crate) fn ai_frame(&mut self) {
        self.ai.poll(self.textures.loaded > 0);
        let now = Instant::now();
        self.ai.hidden = prefs::hidden(&self.settings);
        self.ai.line = self.ai.progress_line(now);
        let shown = taskbar::Shown::of(self.ai.line);
        self.ai.taskbar.show(shown);
        if self.ai.taskbar.shown() != taskbar::Shown::Nothing {
            self.ai.taskbar_progressed = true;
        }
        if self.ai.poll_search(now) {
            self.ai.refresh(true, now);
            self.ai_arrange();
            self.selected = 0;
            self.grid_scroll_to_focus = true;
        } else if self.ai.refresh(false, now) {
            // A search or a "show only" filter fills in as photos are analysed; other orders
            // change on the next snapshot or menu choice, so photos do not move under the
            // pointer while sorting.
            self.ai_marks_changed();
        }
    }

    /// Paints the mark of the photo `id` over its cell, outlines it when it belongs to the
    /// card's burst; a click on its badge opens its card.
    pub(crate) fn ai_mark(
        &mut self,
        ui: &egui::Ui,
        response: egui::Response,
        rect: egui::Rect,
        id: PhotoId,
        selected: bool,
    ) -> egui::Response {
        if ui.rect_contains_pointer(rect) {
            self.ai.hovered = Some(id);
        }
        if self.ai.card == Some(id) {
            self.ai.card_cell = Some(rect);
        }
        if self.ai.in_card_burst(id) {
            marks::sibling(ui, rect);
            self.ai.outlined += 1;
        }
        let mark = self.ai.marks.by_id.get(&id.0);
        let tip = self.ai.card != Some(id);
        if let Some(badge) = marks::paint(ui, rect, mark, selected, self.ai.hidden, id.0, tip) {
            let at = badge.rect.center();
            self.ai.targets.badge.get_or_insert(at);
            self.ai.targets.badges.push((id, at));
            if badge.clicked() && !self.shortcut_editor.open {
                self.ai_toggle_card(id);
            }
        }
        response
    }

    /// The search field has the keyboard: typing must not sort photos.
    pub(crate) fn ai_typing(&self, ctx: &egui::Context) -> bool {
        ctx.memory(|m| m.has_focus(egui::Id::new(SEARCH_ID)))
    }
}
