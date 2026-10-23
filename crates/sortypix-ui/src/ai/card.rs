//! ROLE: Sortypix AI only. The photo card (feedback of 2026-10-23, A1): score and standing in
//! the folder, off-topic, burst with its photos outlined in the grid, the likely reason of a
//! low score; and the legend of the marks in the shortcuts dialog.
//! OWNER: `Ai.card` holds the photo shown; the "Photo details" shortcut (I) and a click on a
//! badge open it, the same again, Esc or the cross close it.
//! INVARIANTS: [`content`] is pure (tested); the card only reads the analysis. The reason is
//! worded as an estimate ("Likely reason") and only for a photo in the lower half of the
//! folder; a probe `ai_quality` did not recognise is never named.
use super::marks::{BADGE_FILL, badge_text};
use super::suggest::Badge;
use super::view::Facts;
use crate::gallery::Gallery;
use crate::shortcuts::{self, Action};
use crate::theme;
use eframe::egui::{self, Color32, RichText, Sense, Stroke};
use sortypix_ai::{Likely, OFF_TOPIC_Z};
use sortypix_core::PhotoId;

/// Width of the card.
const WIDTH: f32 = 280.;
/// Gap between the photo and its card.
const GAP: f32 = 8.;

/// Where a score stands among the analysed photos of the folder.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Standing {
    /// In the lowest `n` % (the photo is in the lower half).
    Lowest(u32),
    /// In the highest `n` %.
    Highest(u32),
}

/// The standing of `score` among `scores` (the folder's, the photo's own included). `None`
/// with fewer than two scores. Ties count in the photo's favour on both sides.
pub(crate) fn standing(score: f32, scores: &[f32]) -> Option<Standing> {
    let n = scores.len();
    if n < 2 {
        return None;
    }
    let below = scores.iter().filter(|&&s| s < score).count();
    let above = scores.iter().filter(|&&s| s > score).count();
    let share = |rank: usize| ((100 * (rank + 1)).div_ceil(n)).clamp(1, 100) as u32;
    Some(if below <= above {
        Standing::Lowest(share(below))
    } else {
        Standing::Highest(share(above))
    })
}

/// Tone of a card line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Tone {
    Text,
    Dim,
    Red,
    Amber,
}

impl Tone {
    fn color(self) -> Color32 {
        match self {
            Tone::Text => theme::TEXT,
            Tone::Dim => theme::DIM,
            Tone::Red => theme::RED,
            Tone::Amber => theme::AMBER,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Line {
    pub text: String,
    pub tone: Tone,
    /// Hover text, when the line needs one.
    pub tip: Option<&'static str>,
}

fn line(text: impl Into<String>, tone: Tone) -> Line {
    Line {
        text: text.into(),
        tone,
        tip: None,
    }
}

/// Hover text of the reason line.
pub(crate) const REASON_TIP: &str = "An estimate from what the photo looks like next to the rest of the folder, not a certainty. Only blur is recognized reliably for now.";

/// What the card says of a photo with these `facts`, among the folder's analysed `scores`;
/// `suggested` when the grid suggests it for rejection.
pub(crate) fn content(facts: &Facts, scores: &[f32], suggested: bool) -> Vec<Line> {
    let mut lines = Vec::new();
    let Some(score) = facts.score else {
        lines.push(line("Not analysed yet", Tone::Dim));
        return lines;
    };
    let standing = standing(score, scores);
    lines.push(line(
        match standing {
            Some(Standing::Lowest(p)) => format!("Score {score:.1}, lowest {p}% of this folder"),
            Some(Standing::Highest(p)) => {
                format!("Score {score:.1}, highest {p}% of this folder")
            }
            None => format!("Score {score:.1}"),
        },
        Tone::Text,
    ));
    if matches!(standing, Some(Standing::Lowest(_))) {
        let reason = match facts.reason {
            Some(Likely::Clear(reason)) => Some(line(
                format!("Likely reason: {}", reason.label()),
                Tone::Text,
            )),
            Some(Likely::Unclear) => Some(line("No clear reason", Tone::Dim)),
            None => None,
        };
        lines.extend(reason.map(|l| Line {
            tip: Some(REASON_TIP),
            ..l
        }));
    }
    match facts.off_topic {
        Some(z) if z > OFF_TOPIC_Z => {
            lines.push(line(
                "Off-topic: unlike the rest of the folder",
                Tone::Amber,
            ));
        }
        Some(_) => lines.push(line("Fits with the rest of the folder", Tone::Dim)),
        None => {}
    }
    if let Some(burst) = facts.burst {
        lines.push(line(
            format!(
                "Burst: one of {} similar shots taken together, outlined in the grid",
                burst.size
            ),
            Tone::Text,
        ));
    }
    if suggested {
        lines.push(line("Suggested for rejection", Tone::Red));
    }
    lines
}

impl Gallery {
    /// The "Photo details" shortcut: the card of the photo under the pointer, else of the
    /// selected one; the same photo again closes it.
    pub(crate) fn ai_card_shortcut(&mut self) {
        if self.viewer {
            return;
        }
        if let Some(id) = self.ai.hovered.or_else(|| self.current_id()) {
            self.ai_toggle_card(id);
        }
    }

    pub(crate) fn ai_toggle_card(&mut self, id: PhotoId) {
        self.ai.card = if self.ai.card == Some(id) {
            None
        } else {
            Some(id)
        };
    }

    /// Each frame, after the grid: the card of `Ai.card`, beside its photo when it is on screen.
    pub(crate) fn ai_card(&mut self, ctx: &egui::Context) {
        self.ai.targets.card_close = None;
        self.ai.card_lines.clear();
        let Some(id) = self.ai.card else {
            return;
        };
        let Some(photo) = self.photos.iter().find(|p| p.id == id) else {
            // Sorted out, or hidden by a search or a filter.
            self.ai.card = None;
            return;
        };
        if self.viewer {
            self.ai.card = None;
            return;
        }
        let name = photo
            .display_source(false)
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        let facts = self.ai.facts_of(id).unwrap_or_default();
        let scores: Vec<f32> = self.ai.facts.iter().filter_map(|f| f.score).collect();
        let suggested = self.ai.marks.by_id.get(&id.0).is_some_and(|m| m.suggested);
        let lines = content(&facts, &scores, suggested);
        let grid = self.grid_rect;
        let height = 40. + 20. * lines.len() as f32;
        let mut pos = match self.ai.card_cell {
            Some(cell) if cell.right() + GAP + WIDTH <= grid.right() => {
                cell.right_top() + egui::vec2(GAP, 0.)
            }
            Some(cell) if cell.left() - GAP - WIDTH >= grid.left() => {
                egui::pos2(cell.left() - GAP - WIDTH, cell.top())
            }
            _ => egui::pos2(grid.right() - WIDTH - 20., grid.top() + GAP),
        };
        pos.y = pos
            .y
            .min(grid.bottom() - height - GAP)
            .max(grid.top() + GAP);
        let close_key = shortcuts::binding(&self.settings, Action::PhotoCard);
        let hint = if close_key.key.is_empty() {
            "Esc closes".to_string()
        } else {
            format!("{} or Esc closes", shortcuts::label(&close_key))
        };
        let mut close = false;
        let mut close_at = None;
        egui::Area::new(egui::Id::new("sortypix-ai-card"))
            .order(egui::Order::Foreground)
            .fixed_pos(pos)
            .show(ctx, |ui| {
                egui::Frame::new()
                    .fill(theme::CARD)
                    .stroke(Stroke::new(1., theme::LINE))
                    .corner_radius(6)
                    .inner_margin(egui::Margin::symmetric(12, 9))
                    .shadow(egui::Shadow {
                        offset: [0, 6],
                        blur: 18,
                        spread: 0,
                        color: Color32::from_black_alpha(128),
                    })
                    .show(ui, |ui| {
                        ui.set_width(WIDTH - 24.);
                        ui.spacing_mut().item_spacing.y = 4.;
                        ui.horizontal(|ui| {
                            let (cross, response) =
                                ui.allocate_exact_size(egui::vec2(16., 16.), Sense::click());
                            ui.add(
                                egui::Label::new(RichText::new(&name).strong().color(theme::TEXT))
                                    .truncate(),
                            );
                            let color = if response.hovered() {
                                theme::BLUE
                            } else {
                                theme::DIM
                            };
                            let r =
                                egui::Rect::from_center_size(cross.center(), egui::vec2(8., 8.));
                            let stroke = Stroke::new(1.5, color);
                            ui.painter()
                                .line_segment([r.left_top(), r.right_bottom()], stroke);
                            ui.painter()
                                .line_segment([r.right_top(), r.left_bottom()], stroke);
                            close_at = Some(cross.center());
                            close = response.on_hover_text("Close").clicked();
                        });
                        for l in &lines {
                            let label = ui.add(
                                egui::Label::new(RichText::new(&l.text).color(l.tone.color()))
                                    .wrap(),
                            );
                            if let Some(tip) = l.tip {
                                label.on_hover_text(tip);
                            }
                        }
                        ui.add_space(2.);
                        ui.label(RichText::new(&hint).small().color(theme::DIM));
                    });
            });
        self.ai.targets.card_close = close_at;
        self.ai.card_lines = lines.into_iter().map(|l| l.text).collect();
        if close {
            self.ai.card = None;
        }
    }

    /// The legend of the marks, in the shortcuts dialog.
    pub(crate) fn ai_legend(&mut self, ui: &mut egui::Ui) {
        self.ai.legend_drawn = true;
        ui.separator();
        ui.strong("Marks");
        let entries = [
            (
                Badge::Score(3.1),
                "Low score: among the lowest share set by Suggest, outlined in red",
            ),
            (
                Badge::OffTopic,
                "Off-topic: unlike the rest of the folder, outlined in red",
            ),
            (
                Badge::Burst,
                "Burst: similar shots taken together, only marked",
            ),
        ];
        egui::Grid::new("sortypix-ai-legend")
            .spacing(egui::vec2(10., 6.))
            .show(ui, |ui| {
                for (badge, text) in entries {
                    let (label, color) = badge_text(badge);
                    ui.label(
                        RichText::new(format!(" {label} "))
                            .small()
                            .color(color)
                            .background_color(BADGE_FILL),
                    );
                    ui.label(text);
                    ui.end_row();
                }
            });
        let key = shortcuts::binding(&self.settings, Action::PhotoCard);
        ui.weak(if key.key.is_empty() {
            "Click a badge for the photo's details. Suggestions only: nothing moves until you sort."
                .to_string()
        } else {
            format!(
                "Click a badge or press {} for the photo's details. Suggestions only: nothing moves until you sort.",
                shortcuts::label(&key)
            )
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::view::Burst;
    use sortypix_ai::Reason;

    fn facts(score: f32) -> Facts {
        Facts {
            score: Some(score),
            ..Facts::default()
        }
    }

    #[test]
    fn standing_counts_from_the_nearest_end() {
        let scores: Vec<f32> = (1..=100).map(|s| s as f32 / 10.).collect();
        assert_eq!(standing(0.1, &scores), Some(Standing::Lowest(1)));
        assert_eq!(standing(1.2, &scores), Some(Standing::Lowest(12)));
        assert_eq!(standing(5.0, &scores), Some(Standing::Lowest(50)));
        assert_eq!(standing(5.1, &scores), Some(Standing::Highest(50)));
        assert_eq!(standing(9.3, &scores), Some(Standing::Highest(8)));
        assert_eq!(standing(10.0, &scores), Some(Standing::Highest(1)));
        // Small folders round up: the lowest of 3 is in the lowest 34 %.
        assert_eq!(standing(1., &[1., 2., 3.]), Some(Standing::Lowest(34)));
        // Equal scores: no photo is "below" another.
        assert_eq!(standing(4., &[4., 4., 4.]), Some(Standing::Lowest(34)));
        assert_eq!(standing(4., &[4.]), None);
        assert_eq!(standing(4., &[]), None);
    }

    #[test]
    fn the_card_names_a_reason_only_for_a_low_photo_and_as_an_estimate() {
        let scores = [1., 2., 3., 4., 5., 6., 7., 8., 9., 10.];
        let mut low = facts(1.);
        low.reason = Some(Likely::Clear(Reason::Blurry));
        let lines = content(&low, &scores, true);
        assert_eq!(lines[0].text, "Score 1.0, lowest 10% of this folder");
        assert_eq!(lines[1].text, "Likely reason: blurry or out of focus");
        assert_eq!(lines[1].tip, Some(REASON_TIP));
        assert_eq!(lines.last().unwrap().text, "Suggested for rejection");
        low.reason = Some(Likely::Unclear);
        assert_eq!(content(&low, &scores, false)[1].text, "No clear reason");
        // A high photo gets no reason line, even when a probe leans its way.
        let mut high = facts(9.);
        high.reason = Some(Likely::Clear(Reason::Blurry));
        let lines = content(&high, &scores, false);
        assert_eq!(lines[0].text, "Score 9.0, highest 20% of this folder");
        assert!(lines.iter().all(|l| !l.text.contains("reason")));
        // Reasons not computed (probes unavailable): no line rather than a guess.
        low.reason = None;
        assert!(
            content(&low, &scores, false)
                .iter()
                .all(|l| !l.text.contains("reason"))
        );
    }

    #[test]
    fn off_topic_burst_and_not_analysed_in_plain_words() {
        let scores = [5., 6.];
        let mut f = facts(6.);
        f.off_topic = Some(OFF_TOPIC_Z + 0.5);
        f.burst = Some(Burst { group: 2, size: 4 });
        let texts: Vec<String> = content(&f, &scores, true)
            .into_iter()
            .map(|l| l.text)
            .collect();
        assert_eq!(
            texts,
            [
                "Score 6.0, highest 50% of this folder",
                "Off-topic: unlike the rest of the folder",
                "Burst: one of 4 similar shots taken together, outlined in the grid",
                "Suggested for rejection",
            ]
        );
        f.off_topic = Some(1.);
        assert_eq!(
            content(&f, &scores, false)[1].text,
            "Fits with the rest of the folder"
        );
        assert_eq!(
            content(&Facts::default(), &scores, false)[0].text,
            "Not analysed yet"
        );
        for l in content(&f, &scores, true) {
            assert!(!l.text.contains('\u{2014}') && !l.text.contains(" \u{2013} ")); // dash-ok
        }
        assert!(!REASON_TIP.contains('\u{2014}')); // dash-ok
    }
}
