//! ROLE: Sortypix AI only. Stages of `--diagnostic` that follow Sortypix's own (plan 4, task
//! 4.5): wait for the analysis of the fixture folder, check the marks, "Reject suggested" then
//! Ctrl+Z with the original bytes back, a search and its clearing, every entry of the order menu;
//! then (task 4.6) the photo card by its shortcut and by a badge, the legend, each "show only"
//! filter and a hidden badge kind, and the taskbar progress cleared at the end.
//! OWNER: `Gallery::run_diagnostic` hands over here once Sortypix's stages passed.
//! INVARIANTS: Acts only through injected input (pointer, text, keys), like a person; checks
//! the grid against the analysis results; writes PASS.txt or FAIL.txt in the probe folder.
use super::filters::{Kind, Kinds};
use super::taskbar::Shown;
use super::view::{Arrangement, Order};
use super::{State, prefs};
use crate::gallery::Gallery;
use eframe::egui;
use sortypix_ai::SEARCH_MIN;
use sortypix_core::PhotoId;
use std::hash::{Hash, Hasher};
use std::path::PathBuf;
use std::time::{Duration, Instant};

/// Pause between two steps (a frame or two always runs in between).
const PACE: Duration = Duration::from_millis(300);
/// Words searched among the fixture landscapes (generated hills, sky and sun).
const WORDS: &str = "mountains";
/// Stage numbers of the AI steps start here (screenshots are `screen-<stage>.png`).
pub(crate) const FIRST_STAGE: usize = 100;

#[derive(Clone, Copy, Debug)]
enum Target {
    Reject,
    Search,
    Clear,
    Order,
    Entry(Option<Order>),
    /// "Show" entry of the menu (`None`: all photos), and a badge toggle.
    Show(Option<Kind>),
    Toggle(Kind),
    /// The badge of the photo chosen for the card, the card's cross, the shortcuts button.
    CardBadge,
    CardClose,
    Help,
}

#[derive(Clone, Copy, Debug)]
enum Step {
    /// Sortypix's stages leave a small window: the AI controls need a wider bar.
    Widen,
    WaitAnalysis,
    CheckMarks,
    HoverBadge,
    Pause,
    Shot,
    Remember,
    Press(Target),
    Release,
    CheckRejected,
    Undo,
    CheckRestored,
    Type,
    Enter,
    WaitSearch,
    CheckMenu,
    /// The grid shows the search (`Some(best match)`) or everything (`None`), in `Order`.
    Check(Option<bool>, Order),
    /// Points at a marked photo of a burst (in the lower half when there is one).
    PickCard,
    /// Presses the "Photo details" shortcut (default I).
    CardKey,
    /// The card is open on the picked photo (`true`) or closed.
    CheckCard(bool),
    CheckLegend,
    /// Turns the wheel over the shortcuts dialog, down to the legend.
    ScrollDialog,
    EscapeKey,
    /// The grid shows only the photos of that kind (`None`: all), marks unchanged.
    CheckShow(Option<Kind>),
    /// Badges of these kinds are not drawn, the others are.
    CheckHidden(Kinds),
}

fn click(target: Target) -> [Step; 3] {
    [Step::Press(target), Step::Release, Step::Pause]
}

fn script() -> Vec<Step> {
    use Step::*;
    let mut steps = vec![
        Widen,
        Pause,
        WaitAnalysis,
        CheckMarks,
        Shot,
        HoverBadge,
        // egui shows a tooltip once the pointer has been still for about a second.
        Pause,
        Pause,
        Pause,
        Pause,
        Pause,
        Shot,
    ];
    steps.push(Remember);
    steps.extend(click(Target::Reject));
    steps.extend([CheckRejected, Undo, Pause, CheckRestored]);
    steps.extend(click(Target::Search));
    steps.extend([
        Type,
        Enter,
        WaitSearch,
        Check(Some(true), Order::Oldest),
        Shot,
    ]);
    steps.extend(click(Target::Order));
    steps.extend([CheckMenu, Shot]);
    steps.extend(click(Target::Entry(Some(Order::Oldest))));
    steps.push(Check(Some(false), Order::Oldest));
    steps.extend(click(Target::Order));
    steps.extend(click(Target::Entry(None)));
    steps.push(Check(Some(true), Order::Oldest));
    steps.extend(click(Target::Clear));
    steps.push(Check(None, Order::Oldest));
    for order in [
        Order::Newest,
        Order::LowestScore,
        Order::HighestScore,
        Order::MostVaried,
        Order::Oldest,
    ] {
        steps.extend(click(Target::Order));
        steps.extend(click(Target::Entry(Some(order))));
        steps.push(Check(None, order));
        if order.needs_model() {
            steps.push(Shot);
        }
    }
    // The photo card: by its shortcut on the hovered photo, then by a click on its badge.
    steps.extend([
        PickCard,
        Pause,
        Pause,
        CardKey,
        Pause,
        CheckCard(true),
        Shot,
    ]);
    steps.extend([CardKey, Pause, CheckCard(false)]);
    steps.extend(click(Target::CardBadge));
    steps.push(CheckCard(true));
    steps.extend(click(Target::CardClose));
    steps.push(CheckCard(false));
    // The legend of the marks in the shortcuts dialog.
    steps.extend(click(Target::Help));
    steps.extend([
        CheckLegend,
        ScrollDialog,
        Pause,
        Pause,
        Shot,
        EscapeKey,
        Pause,
    ]);
    // "Show only" filters, then a hidden badge kind.
    for show in [Some(Kind::Low), Some(Kind::OffTopic), Some(Kind::Burst)] {
        steps.extend(click(Target::Order));
        steps.extend(click(Target::Show(show)));
        steps.push(CheckShow(show));
        if show != Some(Kind::OffTopic) {
            steps.push(Shot);
        }
    }
    let burst_hidden = Kinds {
        burst: true,
        ..Kinds::default()
    };
    steps.extend(click(Target::Order));
    steps.extend(click(Target::Toggle(Kind::Burst)));
    steps.extend([CheckHidden(burst_hidden), Shot]);
    steps.extend(click(Target::Order));
    steps.extend(click(Target::Toggle(Kind::Burst)));
    steps.push(CheckHidden(Kinds::default()));
    steps.extend(click(Target::Order));
    steps.extend(click(Target::Show(None)));
    steps.push(CheckShow(None));
    steps
}

/// A photo's files with a hash of their bytes (`None`: unreadable).
type Files = Vec<(PathBuf, Option<u64>)>;

/// State of the AI stages.
pub(crate) struct Run {
    steps: Vec<Step>,
    step: usize,
    last: Instant,
    count: usize,
    held: Option<egui::Pos2>,
    /// Suggested photos sent to rejected/: id, files and a hash of their bytes.
    rejected: Vec<(PhotoId, Files)>,
    notes: Vec<String>,
    /// The verdict was written.
    done: bool,
    failed: bool,
    /// Rounds a check waited for current results.
    step_waits: usize,
    /// The photo picked for the card.
    card: Option<PhotoId>,
}

impl Default for Run {
    fn default() -> Self {
        Run {
            steps: script(),
            step: 0,
            last: Instant::now(),
            count: 0,
            held: None,
            rejected: Vec::new(),
            notes: Vec::new(),
            done: false,
            failed: false,
            step_waits: 0,
            card: None,
        }
    }
}

fn digest(path: &std::path::Path) -> Option<u64> {
    let bytes = std::fs::read(path).ok()?;
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    bytes.hash(&mut hasher);
    Some(hasher.finish())
}

impl Gallery {
    fn ai_target(&self, target: Target, card: Option<PhotoId>) -> Option<egui::Pos2> {
        let t = &self.ai.targets;
        match target {
            Target::Show(show) => t.shows.iter().find(|(s, _)| *s == show).map(|(_, at)| *at),
            Target::Toggle(kind) => t
                .toggles
                .iter()
                .find(|(k, _)| *k == kind)
                .map(|(_, at)| *at),
            Target::CardBadge => t
                .badges
                .iter()
                .find(|(id, _)| Some(*id) == card)
                .map(|(_, at)| *at),
            Target::CardClose => t.card_close,
            Target::Help => self.diagnostic.as_ref().and_then(|d| d.help_target),
            Target::Reject => t.reject,
            Target::Search => t.search,
            Target::Clear => t.clear,
            Target::Order => t.order,
            Target::Entry(entry) => t
                .entries
                .iter()
                .find(|(e, _)| *e == entry)
                .map(|(_, at)| *at),
        }
    }

    /// The grid against the analysis, for `how`: same photos as the view, in the expected order.
    fn ai_check_view(&self, how: Arrangement) -> Result<(), String> {
        let ai = &self.ai;
        let ids: Vec<PhotoId> = self.photos.iter().map(|p| p.id).collect();
        let view_ids: Vec<PhotoId> = ai.view.iter().map(|&i| ai.base[i].id).collect();
        if ids != view_ids {
            return Err("the grid is not the view of the snapshot".into());
        }
        if self.photos.len() != self.turns.len() {
            return Err("photos and turns differ in length".into());
        }
        let facts: Vec<_> = ai.view.iter().map(|&i| ai.facts[i]).collect();
        if how.searching {
            let matching = ai
                .facts
                .iter()
                .filter(|f| f.similarity.is_some_and(|s| s >= SEARCH_MIN))
                .count();
            if matching != ai.view.len() {
                return Err(format!(
                    "search shows {} photos, {matching} match",
                    ai.view.len()
                ));
            }
        } else if ai.view.len() != ai.base.len() {
            return Err("photos hidden without a search".into());
        }
        let pairs = ai.view.windows(2).zip(facts.windows(2));
        let ordered =
            |ok: &dyn Fn(usize, usize, &super::view::Facts, &super::view::Facts) -> bool| {
                pairs.clone().all(|(i, f)| ok(i[0], i[1], &f[0], &f[1]))
            };
        let fine = if how.searching && how.best_match {
            ordered(&|_, _, a, b| a.similarity >= b.similarity)
        } else {
            match how.order {
                Order::Oldest => ordered(&|a, b, _, _| a < b),
                Order::Newest => ordered(&|a, b, _, _| a > b),
                Order::LowestScore => ordered(&|a, b, fa, fb| match (fa.score, fb.score) {
                    (Some(x), Some(y)) => x <= y,
                    (Some(_), None) => true,
                    (None, Some(_)) => false,
                    (None, None) => a < b,
                }),
                Order::HighestScore => ordered(&|a, b, fa, fb| match (fa.score, fb.score) {
                    (Some(x), Some(y)) => x >= y,
                    (Some(_), None) => true,
                    (None, Some(_)) => false,
                    (None, None) => a < b,
                }),
                Order::MostVaried => ordered(&|a, b, fa, fb| match (fa.coverage, fb.coverage) {
                    (Some(x), Some(y)) => x < y,
                    (Some(_), None) => true,
                    (None, Some(_)) => false,
                    (None, None) => a < b,
                }),
            }
        };
        if !fine {
            return Err(format!("grid not in the expected order for {how:?}"));
        }
        Ok(())
    }

    fn probe(&mut self) -> Option<&mut crate::diagnostic::Diagnostic> {
        self.diagnostic.as_mut()
    }

    pub(crate) fn ai_diagnostic(&mut self, ctx: &egui::Context) {
        let Some(mut run) = self.probe().and_then(|d| d.ai.take()) else {
            return;
        };
        self.ai_step(ctx, &mut run);
        // Kept even after a failure: the closing window must not start the stages again.
        if let Some(d) = self.probe() {
            d.ai = Some(run);
        }
    }

    fn ai_step(&mut self, ctx: &egui::Context, run: &mut Run) {
        let Some(folder) = self.probe().map(|d| d.folder.clone()) else {
            return;
        };
        let fail = |ctx: &egui::Context, message: String| {
            // Every failure closes the window; `run.failed` keeps the stages from going on.
            let _ = std::fs::write(folder.join("FAIL.txt"), format!("AI: {message}"));
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        };
        if run.failed || self.busy || run.last.elapsed() < PACE {
            return;
        }
        let Some(&step) = run.steps.get(run.step) else {
            if run.done {
                return;
            }
            run.done = true;
            let percent = prefs::suggest_percent(&self.settings);
            if prefs::order(&self.settings) != Order::Oldest
                || percent != 20
                || prefs::show(&self.settings).is_some()
                || prefs::hidden(&self.settings) != Kinds::default()
            {
                return {
                    run.failed = true;
                    fail(
                        ctx,
                        "order, share, filter or badges not kept in the settings".into(),
                    )
                };
            }
            let _ = std::fs::write(
                folder.join("PASS.txt"),
                format!(
                    "Keyboard, batch click, undo, full screen, zoom, rotation, resize/UI scale, collapse, shortcut editing, batch icons, Ctrl+wheel, rotate on photo, mouse swap verified. AI: {}",
                    run.notes.join("; ")
                ),
            );
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        };
        run.last = Instant::now();
        let stage = FIRST_STAGE + run.step;
        if let Some(d) = self.probe() {
            d.stage = stage;
        }
        let mut advance = true;
        match step {
            Step::Widen => {
                ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(egui::vec2(1440., 860.)))
            }
            Step::WaitAnalysis => {
                if let State::Off(why) = self.ai.state() {
                    return {
                        run.failed = true;
                        fail(ctx, format!("model unavailable: {why}"))
                    };
                }
                advance = self.ai.complete() && self.ai.facts_current();
                run.count = self.photos.len();
            }
            Step::CheckMarks => {
                let suggested = self.ai.marks.suggested.len();
                let analysed = self.ai.facts.iter().filter(|f| f.score.is_some()).count();
                let off_topic = self
                    .ai
                    .marks
                    .by_id
                    .values()
                    .filter(|m| m.badge(Kinds::default()) == Some(super::suggest::Badge::OffTopic))
                    .count();
                let bursts = self
                    .ai
                    .marks
                    .by_id
                    .values()
                    .filter(|m| m.badge(Kinds::default()) == Some(super::suggest::Badge::Burst))
                    .count();
                if analysed != run.count || suggested < run.count * 20 / 100 {
                    return {
                        run.failed = true;
                        fail(
                            ctx,
                            format!(
                                "{suggested} suggested, {analysed} analysed of {} (failed {}, not analysed {:?}, {})",
                                run.count,
                                self.ai.analyzer.as_ref().map_or(0, |a| a.results().failed),
                                self.photos
                                    .iter()
                                    .filter(|p| self
                                        .ai
                                        .facts_of(p.id)
                                        .is_none_or(|f| f.score.is_none()))
                                    .map(|p| p.display_source(false).display().to_string())
                                    .collect::<Vec<_>>(),
                                self.ai.describe()
                            ),
                        )
                    };
                }
                if !self.ai.line_shown || self.ai.analysing() {
                    return {
                        run.failed = true;
                        fail(
                            ctx,
                            "progress line not shown during the analysis, or still running".into(),
                        )
                    };
                }
                // The taskbar button is cleared once the folder is analysed.
                if self.ai.taskbar_shown() != Shown::Nothing {
                    return {
                        run.failed = true;
                        fail(
                            ctx,
                            format!(
                                "taskbar still shows {:?} after the analysis",
                                self.ai.taskbar_shown()
                            ),
                        )
                    };
                }
                run.notes.push(format!(
                    "analysed {analysed}/{}, suggested {suggested} (20 %, off-topic {off_topic}, burst badges {bursts}), progress line shown then gone, taskbar progress {} then cleared",
                    run.count,
                    if self.ai.taskbar_progressed {
                        "shown"
                    } else {
                        "refused by the shell (hidden window)"
                    }
                ));
            }
            Step::HoverBadge => match self.ai.targets.badge {
                Some(at) => {
                    if let Some(d) = self.probe() {
                        d.hover = Some(at);
                    }
                }
                None => {
                    return {
                        run.failed = true;
                        fail(ctx, "no badge drawn on screen".into())
                    };
                }
            },
            Step::Pause => {}
            Step::Shot => {
                ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));
                run.notes.push(format!("screen-{stage}.png"));
            }
            Step::Remember => {
                let away = self.grid_rect.left_bottom() + egui::vec2(4., -4.);
                if let Some(d) = self.probe() {
                    d.hover = Some(away);
                }
                let shown: Vec<PhotoId> = self.photos.iter().map(|p| p.id).collect();
                let ids = super::suggest::reject_ids(&self.ai.marks.suggested, &shown);
                run.rejected = ids
                    .iter()
                    .filter_map(|id| self.photos.iter().find(|p| p.id == *id))
                    .map(|p| {
                        let files = p.members.iter().map(|f| (f.clone(), digest(f))).collect();
                        (p.id, files)
                    })
                    .collect();
            }
            Step::Press(target) => match self.ai_target(target, run.card) {
                Some(at) => {
                    run.held = Some(at);
                    if let Some(d) = self.probe() {
                        d.pointer = Some((at, true));
                    }
                }
                None => {
                    return {
                        run.failed = true;
                        fail(ctx, format!("{target:?} not on screen"))
                    };
                }
            },
            Step::Release => {
                if let Some(at) = run.held.take()
                    && let Some(d) = self.probe()
                {
                    d.pointer = Some((at, false));
                }
            }
            Step::CheckRejected => {
                let gone = run
                    .rejected
                    .iter()
                    .all(|(id, _)| self.photos.iter().all(|p| p.id != *id));
                if self.photos.len() + run.rejected.len() != run.count
                    || self.undo_count != 1
                    || !gone
                {
                    return {
                        run.failed = true;
                        fail(
                            ctx,
                            format!(
                                "Reject suggested left {} of {} photos ({} suggested), undo {}",
                                self.photos.len(),
                                run.count,
                                run.rejected.len(),
                                self.undo_count
                            ),
                        )
                    };
                }
            }
            Step::Undo => {
                if let Some(d) = self.probe() {
                    d.keys.push((egui::Key::Z, true));
                }
            }
            Step::CheckRestored => {
                let bytes = run.rejected.iter().all(|(_, files)| {
                    files
                        .iter()
                        .all(|(path, hash)| hash.is_some() && digest(path) == *hash)
                });
                if self.photos.len() != run.count || self.undo_count != 0 || !bytes {
                    return {
                        run.failed = true;
                        fail(
                            ctx,
                            format!(
                                "Ctrl+Z after Reject suggested: {} of {} photos, undo {}, bytes back {bytes}",
                                self.photos.len(),
                                run.count,
                                self.undo_count
                            ),
                        )
                    };
                }
                run.notes.push(format!(
                    "Reject suggested moved {} photos in one gesture, one Ctrl+Z put them back byte for byte",
                    run.rejected.len()
                ));
            }
            Step::Type => {
                if let Some(d) = self.probe() {
                    d.text = Some(WORDS.to_string());
                }
            }
            Step::Enter => {
                if let Some(d) = self.probe() {
                    d.keys.push((egui::Key::Enter, false));
                }
            }
            Step::WaitSearch => {
                advance = self.ai.search.applied() == Some(WORDS)
                    && !self.ai.search.waiting()
                    && self.ai.facts_current();
                if advance {
                    let sims: Vec<f32> =
                        self.ai.facts.iter().filter_map(|f| f.similarity).collect();
                    let best = sims.iter().copied().fold(f32::MIN, f32::max);
                    let worst = sims.iter().copied().fold(f32::MAX, f32::min);
                    if self.photos.is_empty() || self.photos.len() == run.count {
                        return {
                            run.failed = true;
                            fail(
                                ctx,
                                format!(
                                    "search '{WORDS}' kept {} of {} (cosine {worst:.3} to {best:.3}, threshold {SEARCH_MIN})",
                                    self.photos.len(),
                                    run.count
                                ),
                            )
                        };
                    }
                    let label = self.ai_count_label().unwrap_or_default();
                    run.notes.push(format!(
                        "search '{WORDS}' shows {label} (cosine {worst:.3} to {best:.3}, threshold {SEARCH_MIN})"
                    ));
                }
            }
            Step::CheckMenu => {
                if self.ai.targets.entries.len() != 1 + Order::ALL.len() {
                    return {
                        run.failed = true;
                        fail(
                            ctx,
                            format!(
                                "order menu did not open with its {} entries",
                                1 + Order::ALL.len()
                            ),
                        )
                    };
                }
            }
            Step::PickCard => {
                // A badge of a burst photo in the lower half of the folder, else any badge.
                let scores: Vec<f32> = self.ai.facts.iter().filter_map(|f| f.score).collect();
                let low = |id: PhotoId| {
                    self.ai.facts_of(id).and_then(|f| f.score).is_some_and(|s| {
                        matches!(
                            super::card::standing(s, &scores),
                            Some(super::card::Standing::Lowest(_))
                        )
                    })
                };
                let burst = |id: PhotoId| self.ai.facts_of(id).is_some_and(|f| f.burst.is_some());
                let badges = &self.ai.targets.badges;
                let pick = badges
                    .iter()
                    .find(|(id, _)| burst(*id) && low(*id))
                    .or_else(|| badges.iter().find(|(id, _)| burst(*id)))
                    .or_else(|| badges.first())
                    .copied();
                let Some((id, at)) = pick else {
                    return {
                        run.failed = true;
                        fail(ctx, "no badge on screen for the card".into())
                    };
                };
                run.card = Some(id);
                if let Some(d) = self.probe() {
                    d.hover = Some(at);
                }
            }
            Step::CardKey => {
                if let Some(d) = self.probe() {
                    d.keys.push((egui::Key::I, false));
                }
            }
            Step::ScrollDialog => {
                let at = self.grid_rect.center();
                if let Some(d) = self.probe() {
                    d.scroll = Some(at);
                }
            }
            Step::EscapeKey => {
                if let Some(d) = self.probe() {
                    d.keys.push((egui::Key::Escape, false));
                }
            }
            Step::CheckCard(open) => {
                let expected = if open { run.card } else { None };
                if self.ai.card != expected {
                    return {
                        run.failed = true;
                        fail(
                            ctx,
                            format!("card {:?}, expected {expected:?}", self.ai.card),
                        )
                    };
                }
                if open {
                    let Some(id) = run.card else {
                        return;
                    };
                    let lines = self.ai.card_lines.clone();
                    let facts = self.ai.facts_of(id).unwrap_or_default();
                    let scores: Vec<f32> = self.ai.facts.iter().filter_map(|f| f.score).collect();
                    let lower = facts.score.is_some_and(|s| {
                        matches!(
                            super::card::standing(s, &scores),
                            Some(super::card::Standing::Lowest(_))
                        )
                    });
                    let reason = lines
                        .iter()
                        .any(|l| l.starts_with("Likely reason: ") || l == "No clear reason");
                    let burst_line = lines.iter().any(|l| l.starts_with("Burst: one of "));
                    let problem = if lines.first().is_none_or(|l| !l.contains("of this folder")) {
                        Some("no score and standing line")
                    } else if lower != reason {
                        Some("reason line where it does not belong, or missing")
                    } else if facts.burst.is_some() && (!burst_line || self.ai.outlined < 2) {
                        Some("burst not told or its photos not outlined")
                    } else {
                        None
                    };
                    if let Some(problem) = problem {
                        return {
                            run.failed = true;
                            fail(ctx, format!("card: {problem}: {lines:?}"))
                        };
                    }
                    run.notes.push(format!(
                        "card [{}], {} burst photos outlined",
                        lines.join(" | "),
                        self.ai.outlined
                    ));
                }
            }
            Step::CheckLegend => {
                if !self.shortcut_editor.open || !self.ai.legend_drawn {
                    return {
                        run.failed = true;
                        fail(
                            ctx,
                            "shortcuts dialog without the legend of the marks".into(),
                        )
                    };
                }
                run.notes
                    .push("legend of the marks in the shortcuts dialog".into());
            }
            Step::CheckShow(show) => {
                if !self.ai.facts_current() || self.ai.revision != self.ai.arranged {
                    run.step_waits += 1;
                    if run.step_waits > 100 {
                        return {
                            run.failed = true;
                            fail(ctx, "the results kept changing".into())
                        };
                    }
                    if self.ai.facts_current() {
                        self.ai_arrange();
                    }
                    return;
                }
                run.step_waits = 0;
                let ids: Vec<PhotoId> = self.photos.iter().map(|p| p.id).collect();
                let expected: Vec<PhotoId> = self
                    .ai
                    .arranged_ids()
                    .into_iter()
                    .filter(|id| {
                        show.is_none_or(|k| {
                            self.ai
                                .marks
                                .by_id
                                .get(&id.0)
                                .is_some_and(|m| m.kinds.has(k))
                        })
                    })
                    .collect();
                if self.ai_show() != show || ids != expected {
                    return {
                        run.failed = true;
                        fail(
                            ctx,
                            format!(
                                "show {show:?}: grid has {} photos, {} expected (filter {:?})",
                                ids.len(),
                                expected.len(),
                                self.ai_show()
                            ),
                        )
                    };
                }
                if show.is_some() && self.ai_count_label().is_none() {
                    return {
                        run.failed = true;
                        fail(ctx, "no \"n of m\" count while filtering".into())
                    };
                }
                run.notes.push(format!(
                    "{} shows {} of {}",
                    super::filters::show_label(show),
                    ids.len(),
                    self.ai.total()
                ));
            }
            Step::CheckHidden(hidden) => {
                if prefs::hidden(&self.settings) != hidden {
                    return {
                        run.failed = true;
                        fail(
                            ctx,
                            format!(
                                "hidden badges {:?}, expected {hidden:?}",
                                prefs::hidden(&self.settings)
                            ),
                        )
                    };
                }
                // Every drawn badge is of a kind not hidden; with bursts hidden, a photo marked
                // only as a burst shows no badge.
                let drawn = &self.ai.targets.badges;
                let wrong = drawn.iter().any(|(id, _)| {
                    self.ai
                        .marks
                        .by_id
                        .get(&id.0)
                        .and_then(|m| m.badge(hidden))
                        .is_none_or(|b| hidden.has(b.kind()))
                });
                let marked = self
                    .photos
                    .iter()
                    .filter(|p| {
                        self.ai
                            .marks
                            .by_id
                            .get(&p.id.0)
                            .is_some_and(|m| m.badge(hidden).is_some())
                    })
                    .count();
                if wrong || (marked > 0 && drawn.is_empty()) {
                    return {
                        run.failed = true;
                        fail(
                            ctx,
                            format!("badges drawn for hidden kinds ({} drawn)", drawn.len()),
                        )
                    };
                }
                run.notes.push(format!(
                    "hidden badges {:?}: {} badges drawn",
                    Kind::ALL
                        .iter()
                        .filter(|k| hidden.has(**k))
                        .collect::<Vec<_>>(),
                    drawn.len()
                ));
            }
            Step::Check(search, order) => {
                // The AI orders are applied when chosen and on each snapshot, not as results
                // arrive: when the analysis changed since, apply it again (as a snapshot would).
                let stale = !self.ai.facts_current() || self.ai.revision != self.ai.arranged;
                if stale {
                    run.step_waits += 1;
                    if run.step_waits > 100 {
                        return {
                            run.failed = true;
                            fail(ctx, "the results kept changing".into())
                        };
                    }
                    if self.ai.facts_current() {
                        self.ai_arrange();
                    }
                    return;
                }
                run.step_waits = 0;
                let how = Arrangement {
                    order,
                    searching: search.is_some(),
                    best_match: search.unwrap_or(false),
                };
                if self.ai_arrangement() != how {
                    return {
                        run.failed = true;
                        fail(
                            ctx,
                            format!("arrangement {:?}, expected {how:?}", self.ai_arrangement()),
                        )
                    };
                }
                if let Err(message) = self.ai_check_view(how) {
                    return {
                        run.failed = true;
                        fail(ctx, message)
                    };
                }
                let label = match search {
                    Some(true) => "Best match".to_string(),
                    Some(false) => format!("{} while searching", order.label()),
                    None => order.label().to_string(),
                };
                run.notes.push(format!("{label} ok"));
            }
        }
        if advance {
            run.step += 1;
        }
    }
}
