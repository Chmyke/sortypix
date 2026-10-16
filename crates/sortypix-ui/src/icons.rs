//! Consistent vector controls, independent of fonts and window scale.
use crate::theme;
use eframe::egui::{self, Color32, Rect, Response, Sense, Stroke, StrokeKind};
#[derive(Clone, Copy)]
pub enum Icon {
    Folder,
    SourceFolder,
    Undo,
    Rotate,
    Zoom,
    ThumbSmall,
    ThumbLarge,
    Ascending,
    Descending,
    Keyboard,
    KeepAll,
    RejectAll,
    KeptFolder,
    RejectedFolder,
    Fit,
    Expand,
    Contract,
}
pub fn button(ui: &mut egui::Ui, icon: Icon, tip: &str, enabled: bool) -> Response {
    lit_button(ui, icon, tip, enabled, 0.)
}
/// Icon button whose own color glows behind it; `glow` fades from 1 to 0.
pub fn lit_button(ui: &mut egui::Ui, icon: Icon, tip: &str, enabled: bool, glow: f32) -> Response {
    ui.add_enabled_ui(enabled, |ui| {
        let (rect, response) = ui.allocate_exact_size(egui::vec2(32., 32.), Sense::click());
        if response.hovered() || response.has_focus() {
            ui.painter().rect_filled(rect, 6, theme::CARD2);
        }
        let color = if !enabled {
            theme::DIM.linear_multiply(0.4)
        } else if matches!(icon, Icon::KeepAll | Icon::KeptFolder) {
            theme::GREEN
        } else if matches!(icon, Icon::RejectAll | Icon::RejectedFolder) {
            theme::RED
        } else if response.hovered() {
            theme::BLUE
        } else {
            theme::TEXT
        };
        if glow > 0. {
            ui.painter()
                .rect_filled(rect, 6, color.gamma_multiply(0.35 * glow));
            ui.painter().rect_stroke(
                rect,
                6,
                Stroke::new(1., color.gamma_multiply(glow)),
                StrokeKind::Inside,
            );
        }
        paint(ui.painter(), rect, icon, color);
        response
            .on_hover_cursor(egui::CursorIcon::PointingHand)
            .on_hover_text(tip)
    })
    .inner
}
pub fn corner(bounds: Rect) -> Rect {
    let size = bounds.size().min(egui::vec2(28., 28.));
    let inset = egui::vec2(6., 6.).min((bounds.size() - size) * 0.5);
    Rect::from_min_size(bounds.right_bottom() - inset - size, size)
}
/// Bottom-left mirror of `corner`, only when both corner controls fit side by side.
pub fn left_corner(bounds: Rect) -> Option<Rect> {
    let right = corner(bounds);
    let rect = Rect::from_min_size(
        egui::pos2(
            bounds.left() + (bounds.right() - right.right()),
            right.top(),
        ),
        right.size(),
    );
    (rect.right() + 6. <= right.left()).then_some(rect)
}
pub fn paint(p: &egui::Painter, rect: Rect, icon: Icon, color: Color32) {
    let c = rect.center();
    let stroke = Stroke::new(1.5, color);
    let line = |points: &[(f32, f32)]| {
        p.add(egui::Shape::line(
            points.iter().map(|&(x, y)| c + egui::vec2(x, y)).collect(),
            stroke,
        ))
    };
    match icon {
        Icon::Folder | Icon::SourceFolder | Icon::KeptFolder | Icon::RejectedFolder => {
            line(&[
                (-9., -7.),
                (-3., -7.),
                (0., -4.),
                (9., -4.),
                (9., 7.),
                (-9., 7.),
                (-9., -7.),
            ]);
            if matches!(icon, Icon::SourceFolder) {
                line(&[(-4., 4.), (4., -1.)]);
                line(&[(0., -1.), (4., -1.), (4., 3.)]);
            } else if matches!(icon, Icon::KeptFolder) {
                line(&[(-4., 2.), (-1., 5.), (5., -1.)]);
            } else if matches!(icon, Icon::RejectedFolder) {
                line(&[(-3., 0.), (3., 5.)]);
                line(&[(-3., 5.), (3., 0.)]);
            } else {
                line(&[(-8., -1.), (8., -1.)]);
            }
        }
        Icon::Undo => {
            line(&[(-4., -7.), (-9., -2.), (-4., 3.)]);
            line(&[
                (-9., -2.),
                (3., -2.),
                (7., 1.),
                (7., 5.),
                (4., 8.),
                (-1., 8.),
            ]);
        }
        Icon::Rotate => {
            // Clockwise arc around a small photo frame, so it never reads as redo.
            let points = (0..40)
                .map(|i| {
                    let a = -0.55 - i as f32 / 39. * 5.0;
                    c + egui::vec2(a.cos() * 9., a.sin() * 9.)
                })
                .collect();
            p.add(egui::Shape::line(points, Stroke::new(1.8, color)));
            line(&[(7.7, -10.), (7.7, -4.7), (2.4, -4.7)]);
            p.rect_stroke(
                Rect::from_center_size(c, egui::vec2(9., 7.)),
                1,
                Stroke::new(1.3, color),
                StrokeKind::Middle,
            );
            p.add(egui::Shape::line(
                [(-3., 2.), (-0.5, -0.5), (1., 1.), (3., -1.)]
                    .iter()
                    .map(|&(x, y)| c + egui::vec2(x, y))
                    .collect(),
                Stroke::new(1.1, color),
            ));
        }
        Icon::Zoom | Icon::Fit => {
            let glass = c + egui::vec2(-3., -3.);
            p.circle_stroke(glass, 7., Stroke::new(2., color));
            p.line_segment(
                [glass + egui::vec2(5., 5.), c + egui::vec2(10., 10.)],
                Stroke::new(3., color),
            );
            line(&[(-6.5, -3.), (0.5, -3.)]);
            if matches!(icon, Icon::Zoom) {
                line(&[(-3., -6.5), (-3., 0.5)]);
            }
        }
        Icon::ThumbSmall | Icon::ThumbLarge => {
            let side = if matches!(icon, Icon::ThumbSmall) {
                7.
            } else {
                14.
            };
            p.rect_stroke(
                Rect::from_center_size(c, egui::vec2(side, side)),
                1,
                stroke,
                StrokeKind::Inside,
            );
        }
        Icon::Ascending | Icon::Descending => {
            let s = if matches!(icon, Icon::Ascending) {
                1.
            } else {
                -1.
            };
            line(&[(-6., -8. * s), (-6., 8. * s)]);
            line(&[(-9., 4. * s), (-6., 8. * s), (-3., 4. * s)]);
            for (y, w) in [(-6., 4.), (0., 6.), (6., 8.)] {
                line(&[(0., y), (w, y)]);
            }
        }
        Icon::Keyboard => {
            p.rect_stroke(
                Rect::from_center_size(c, egui::vec2(22., 16.)),
                3,
                stroke,
                StrokeKind::Inside,
            );
            for x in [-6., -2., 2., 6.] {
                for y in [-3., 1.] {
                    p.circle_filled(c + egui::vec2(x, y), 0.8, color);
                }
            }
            line(&[(-5., 5.), (5., 5.)]);
        }
        Icon::KeepAll | Icon::RejectAll => {
            for y in [-7., 0., 7.] {
                line(&[(0., y), (10., y)]);
                if matches!(icon, Icon::KeepAll) {
                    line(&[(-10., y), (-7., y + 2.), (-3., y - 2.)]);
                } else {
                    line(&[(-9., y - 2.), (-5., y + 2.)]);
                    line(&[(-9., y + 2.), (-5., y - 2.)]);
                }
            }
        }
        Icon::Expand | Icon::Contract => {
            let (a, b) = if matches!(icon, Icon::Expand) {
                (7., 2.)
            } else {
                (2., 7.)
            };
            for (x, y) in [(-1., -1.), (1., -1.), (1., 1.), (-1., 1.)] {
                line(&[(x * b, y * a), (x * a, y * a), (x * a, y * b)]);
            }
        }
    }
}
