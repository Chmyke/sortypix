//! ROLE: Sortypix AI only. Paints a photo's mark on the grid (design board, point 1 A1).
//! OWNER: `Gallery::grid` calls [`paint`] for each visible cell, behind `cfg(feature = "ai")`.
//! INVARIANTS: Paints over the photo only: dashed red outline for a suggested photo, a badge in
//! the top right corner (the corner controls sit at the bottom). A click on the badge opens the
//! photo card; any other click on the photo stays the grid's.
use super::filters::Kinds;
use super::suggest::{Badge, Mark};
use crate::theme;
use eframe::egui::{self, Color32, Rect, Sense, Shape, Stroke};

/// Badge background: black at 78 %, as on the board.
pub(crate) const BADGE_FILL: Color32 = Color32::from_black_alpha(199);
/// Outline of the photos of the burst shown in the open card.
const SIBLING: Stroke = Stroke {
    width: 2.,
    color: theme::TEXT,
};

/// Text and colour of a badge.
pub(crate) fn badge_text(badge: Badge) -> (String, Color32) {
    match badge {
        Badge::Score(score) => (format!("{score:.1}"), theme::RED),
        Badge::OffTopic => ("Off-topic".to_string(), theme::AMBER),
        Badge::Burst => ("Burst".to_string(), theme::DIM),
    }
}

/// Outlines a photo of the burst the card shows (inside the selection outline).
pub(crate) fn sibling(ui: &egui::Ui, rect: Rect) {
    ui.painter()
        .rect_stroke(rect.shrink(3.), 6, SIBLING, egui::StrokeKind::Inside);
}

/// Paints `mark` over the cell `rect` (badges of `hidden` kinds left out). Returns the badge's
/// response when one was drawn: it carries the mark's hover text (unless `tip` is false: the
/// card of the photo is open and says more) and opens the card on click.
pub(crate) fn paint(
    ui: &egui::Ui,
    rect: Rect,
    mark: Option<&Mark>,
    selected: bool,
    hidden: Kinds,
    salt: u64,
    tip: bool,
) -> Option<egui::Response> {
    let mark = mark?;
    let painter = ui.painter();
    if mark.suggested {
        // Inside the blue selection outline when both apply, so both stay visible.
        let r = rect.shrink(if selected { 4. } else { 1. });
        let points = [
            r.left_top(),
            r.right_top(),
            r.right_bottom(),
            r.left_bottom(),
            r.left_top(),
        ];
        painter.extend(Shape::dashed_line(
            &points,
            Stroke::new(2., theme::RED),
            6.,
            4.,
        ));
    }
    let (text, color) = badge_text(mark.badge(hidden)?);
    let galley = painter.layout_no_wrap(text, egui::FontId::proportional(11.), color);
    let size = galley.size() + egui::vec2(14., 2.);
    let inset = if selected { 7. } else { 5. };
    if size.x + 2. * inset > rect.width() || size.y + 2. * inset > rect.height() {
        return None;
    }
    let badge = Rect::from_min_size(
        egui::pos2(rect.right() - inset - size.x, rect.top() + inset),
        size,
    );
    // Above the cell: the click on the badge is the badge's, not a keep.
    let response = ui.interact(badge, ui.id().with(("ai-badge", salt)), Sense::click());
    painter.rect_filled(badge, size.y / 2., BADGE_FILL);
    if response.hovered() {
        painter.rect_stroke(
            badge,
            size.y / 2.,
            Stroke::new(1., color),
            egui::StrokeKind::Inside,
        );
    }
    painter.galley(badge.center() - galley.size() / 2., galley, color);
    let response = response.on_hover_cursor(egui::CursorIcon::PointingHand);
    Some(if tip {
        response.on_hover_text(format!("{}\nClick for details", mark.tip))
    } else {
        response
    })
}
