//! ROLE: Transient feedback that never moves the photos: decision glow, notices, errors.
//! OWNER: Gallery stores the timestamps; this module only decides timing and paints overlays.
use eframe::egui;
use std::time::{Duration, Instant};

/// Short file moves stay invisible; only a slow drive shows the busy state.
pub const BUSY_DELAY: Duration = Duration::from_millis(300);
pub const GLOW: Duration = Duration::from_millis(700);
pub const NOTICE: Duration = Duration::from_millis(2500);

pub fn busy_shown(since: Option<Instant>, now: Instant) -> bool {
    since.is_some_and(|since| now.saturating_duration_since(since) >= BUSY_DELAY)
}

/// Full strength briefly, then a linear fade to zero.
pub fn glow(since: Instant, now: Instant) -> f32 {
    let elapsed = now.saturating_duration_since(since).as_secs_f32();
    let hold = 0.15;
    if elapsed <= hold {
        1.
    } else {
        (1. - (elapsed - hold) / (GLOW.as_secs_f32() - hold)).clamp(0., 1.)
    }
}

pub fn notice_shown(since: Instant, now: Instant) -> bool {
    now.saturating_duration_since(since) < NOTICE
}

pub fn position_label(selected: usize, count: usize) -> String {
    if count == 0 {
        "0 / 0".into()
    } else {
        format!("{} / {count}", selected.min(count - 1) + 1)
    }
}

pub fn undo_badge(count: usize) -> Option<String> {
    match count {
        0 => None,
        1..=99 => Some(count.to_string()),
        _ => Some("99+".into()),
    }
}

/// Bottom-centered message floating above the photos, so the grid never jumps.
pub fn overlay(ctx: &egui::Context, error: &mut Option<String>, notice: Option<&str>) {
    let (text, color) = match (error.as_deref(), notice) {
        (Some(error), _) => (error.to_owned(), crate::theme::RED),
        (None, Some(notice)) => (notice.to_owned(), crate::theme::BLUE),
        (None, None) => return,
    };
    egui::Area::new(egui::Id::new("status-overlay"))
        .order(egui::Order::Foreground)
        .anchor(egui::Align2::CENTER_BOTTOM, egui::vec2(0., -16.))
        .show(ctx, |ui| {
            egui::Frame::new()
                .fill(crate::theme::CARD)
                .stroke(egui::Stroke::new(1., color))
                .corner_radius(8)
                .inner_margin(egui::Margin::symmetric(12, 8))
                .show(ui, |ui| {
                    ui.set_max_width((ctx.content_rect().width() - 64.).max(160.));
                    ui.horizontal_wrapped(|ui| {
                        ui.colored_label(color, text);
                        if error.is_some() && ui.small_button("Dismiss").clicked() {
                            *error = None;
                        }
                    });
                });
        });
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn quick_moves_do_not_flash_the_busy_state() {
        let start = Instant::now();
        assert!(!busy_shown(None, start));
        assert!(!busy_shown(Some(start), start + Duration::from_millis(120)));
        assert!(busy_shown(Some(start), start + BUSY_DELAY));
    }
    #[test]
    fn decision_glow_holds_then_fades_out() {
        let start = Instant::now();
        assert_eq!(glow(start, start), 1.);
        assert_eq!(glow(start, start + Duration::from_millis(150)), 1.);
        let middle = glow(start, start + Duration::from_millis(400));
        assert!(middle > 0. && middle < 1.);
        assert_eq!(glow(start, start + GLOW), 0.);
        assert_eq!(glow(start, start + Duration::from_secs(5)), 0.);
    }
    #[test]
    fn notices_expire() {
        let start = Instant::now();
        assert!(notice_shown(start, start + Duration::from_secs(1)));
        assert!(!notice_shown(start, start + NOTICE));
    }
    #[test]
    fn full_screen_position_is_one_based_and_bounded() {
        assert_eq!(position_label(0, 28), "1 / 28");
        assert_eq!(position_label(27, 28), "28 / 28");
        assert_eq!(position_label(40, 28), "28 / 28");
        assert_eq!(position_label(0, 0), "0 / 0");
    }
    #[test]
    fn undo_badge_stays_compact() {
        assert_eq!(undo_badge(0), None);
        assert_eq!(undo_badge(7).as_deref(), Some("7"));
        assert_eq!(undo_badge(99).as_deref(), Some("99"));
        assert_eq!(undo_badge(1234).as_deref(), Some("99+"));
    }
    #[test]
    fn status_overlay_never_moves_the_layout() {
        let ctx = egui::Context::default();
        let mut error = Some("Could not open folder".to_owned());
        let output = ctx.run_ui(egui::RawInput::default(), |ui| {
            let before = ui.cursor();
            overlay(ui.ctx(), &mut error, Some("Left click: keep"));
            assert_eq!(ui.cursor(), before, "Feedback must float above the photos");
        });
        output.drop_without_applying_deltas();
        assert!(error.is_some(), "Errors stay until dismissed");
    }
}
